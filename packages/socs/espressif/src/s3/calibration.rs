//! The RTC clock calibration of a timer group, which counts crystal cycles over a number of
//! cycles of a slow clock.
//!
//! A calibration finishes the moment `TIMG_RTC_CALI_START` rises, with the count the slow clock
//! would give at its nominal frequency: the 136 kHz RC slow clock, the 17.5 MHz RC fast clock
//! over 256, or the 32.768 kHz crystal, as `TIMG_RTC_CALI_CLK_SEL` 0, 1 or 2 picks. A cycling
//! calibration, `TIMG_RTC_CALI_START_CYCLING`, always reads ready. None times out. IDF v5.5.5, `components/soc/esp32s3/include/soc/clk_tree_defs.h` and
//! `components/esp_hw_support/port/esp32s3/rtc_time.c`.

/// `TIMG_RTCCALICFG_REG`, `TIMG_RTCCALICFG1_REG` and `TIMG_RTCCALICFG2_REG`.
const CFG: u32 = 0x68;
const CFG1: u32 = 0x6c;
const CFG2: u32 = 0x80;
pub(crate) const OFFSETS: [u32; 3] = [CFG, CFG1, CFG2];

const START: u32 = 1 << 31;
const START_CYCLING: u32 = 1 << 12;
const RDY: u32 = 1 << 15;
const TIMEOUT: u32 = 1 << 0;
const XTAL_HZ: u64 = 40_000_000;
/// The nominal frequency of each slow clock, by `TIMG_RTC_CALI_CLK_SEL`.
const SLOW_HZ: [u64; 4] = [136_000, 17_500_000 / 256, 32_768, 136_000];

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Calibration {
    cfg: u32,
    cfg1: u32,
    cfg2: u32,
}

impl Calibration {
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        match offset {
            CFG if self.cfg & START_CYCLING != 0 => Some(self.cfg | RDY),
            CFG => Some(self.cfg),
            CFG1 => Some(self.cfg1),
            CFG2 => Some(self.cfg2),
            _ => None,
        }
    }

    pub(crate) fn write(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            CFG => {
                let rose = value & START != 0 && self.cfg & START == 0;
                let ready = value & START != 0 && (rose || self.cfg & RDY != 0);
                self.cfg = (value & !RDY) | if ready { RDY } else { 0 };
                if rose {
                    let slow = SLOW_HZ[((value >> 13) & 3) as usize];
                    let cycles = u64::from((value >> 16) & 0x7fff);
                    self.cfg1 = ((cycles * XTAL_HZ / slow) as u32) << 7;
                }
            }
            CFG1 => {}
            CFG2 => self.cfg2 = value & !TIMEOUT,
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_finishes_at_once_with_the_nominal_count() {
        let mut calibration = Calibration::default();
        calibration.write(CFG, 1024 << 16);
        assert_eq!(calibration.read(CFG).unwrap() & RDY, 0);
        calibration.write(CFG, START | 1024 << 16);
        assert_ne!(calibration.read(CFG).unwrap() & RDY, 0);
        assert_eq!(
            calibration.read(CFG1),
            Some(301_176 << 7),
            "1024 cycles of 136 kHz"
        );
        calibration.write(CFG, 1024 << 16);
        assert_eq!(
            calibration.read(CFG).unwrap() & RDY,
            0,
            "a cleared start clears ready"
        );
        calibration.write(CFG, START_CYCLING);
        assert_ne!(
            calibration.read(CFG).unwrap() & RDY,
            0,
            "a cycling calibration"
        );
    }
}
