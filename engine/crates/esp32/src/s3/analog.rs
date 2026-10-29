//! The analog I2C master, through which the firmware sets the PLL and other analog blocks.
//!
//! A write of a control register runs one transaction at once: bits 0 to 7 name the analog
//! block, bits 8 to 15 its register, bit 24 marks a write of the byte in bits 16 to 23, and a
//! read leaves that register's byte there. The busy bit, 25, reads 0. The analog blocks keep each
//! byte written and read 0 for one never written, since they have no effect here. The PLL's
//! calibration is done at once. IDF v5.5.5, `components/soc/esp32s3/include/soc/regi2c_defs.h`
//! and the ROM's `rom_i2c_*` functions.

use std::collections::BTreeMap;

/// The two control registers, one for each host.
const CONTROLS: u32 = 2;
/// `I2C_MST_ANA_CONF0_REG` to `ANA_CONFIG2_REG`.
const CONFIG: std::ops::RangeInclusive<u32> = 0x40..=0x48;
const CAL_DONE: u32 = 1 << 24;
const WRITE: u32 = 1 << 24;

#[derive(Clone, Debug, Default)]
pub(crate) struct Analog {
    controls: [u32; CONTROLS as usize],
    config: [u32; 3],
    /// Each byte written, by block and register.
    bytes: BTreeMap<(u8, u8), u8>,
}

impl Analog {
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        if offset < 4 * CONTROLS {
            return Some(self.controls[(offset / 4) as usize]);
        }
        if CONFIG.contains(&offset) {
            let word = self.config[((offset - CONFIG.start()) / 4) as usize];
            return Some(if offset == *CONFIG.start() {
                word | CAL_DONE
            } else {
                word
            });
        }
        None
    }

    pub(crate) fn write(&mut self, offset: u32, value: u32) -> bool {
        if offset < 4 * CONTROLS {
            let key = (value as u8, (value >> 8) as u8);
            let byte = if value & WRITE != 0 {
                let byte = (value >> 16) as u8;
                self.bytes.insert(key, byte);
                byte
            } else {
                self.bytes.get(&key).copied().unwrap_or(0)
            };
            self.controls[(offset / 4) as usize] = (value & 0x0100_ffff) | u32::from(byte) << 16;
            return true;
        }
        if CONFIG.contains(&offset) {
            self.config[((offset - CONFIG.start()) / 4) as usize] = value;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_returns_the_byte_written_to_that_block_and_register() {
        let mut analog = Analog::default();
        assert!(analog.write(0, 0x015a_ab66));
        assert!(analog.write(4, 0x0000_ab66));
        assert_eq!(analog.read(4), Some(0x005a_ab66));
        assert!(analog.write(0, 0x0000_ac66));
        assert_eq!(analog.read(0), Some(0x0000_ac66), "never written reads 0");
        assert_eq!(analog.read(0x40), Some(CAL_DONE));
    }
}
