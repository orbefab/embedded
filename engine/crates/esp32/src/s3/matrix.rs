//! The interrupt matrix, which routes each peripheral interrupt source to a CPU interrupt of
//! each core, and the four `FROM_CPU` sources that firmware raises by writing a register.
//!
//! Each core has a map. Source `n`'s map register, at `4 * n` for the PRO CPU and `0x800` above
//! that for the APP CPU, names the CPU interrupt it drives on that core, and a CPU interrupt
//! line is high while any source mapped to it is. A source mapped to one of the core's own
//! interrupts drives nothing, as the core takes no external line there. The APP CPU's map
//! starts at 16, a core timer, as the hardware resets it. The status registers show each
//! source's level, the same in both halves. The sources modeled here are the GPIO interrupt,
//! SPI2's, I2C0's, each timer group's watchdog, the three SYSTIMER alarms and the four
//! `FROM_CPU` registers. The USB Serial/JTAG source is added by the caller. Every other source
//! stays low. IDF v5.5.5, `components/soc/esp32s3/include/soc/interrupts.h`,
//! `interrupt_core0_reg.h` and `interrupt_core1_reg.h`.

use sim_xtensa::CoreId;

/// The sources, `ETS_MAX_INTR_SOURCE`.
const SOURCES: u32 = 99;
/// `ETS_GPIO_INTR_SOURCE`.
const GPIO: u32 = 16;
const SPI2: u32 = 21;
/// `ETS_I2C_EXT0_INTR_SOURCE`.
const I2C_EXT0: u32 = 42;
/// `ETS_TG0_WDT_LEVEL_INTR_SOURCE` and `ETS_TG1_WDT_LEVEL_INTR_SOURCE`.
const TG_WDT: [u32; 2] = [52, 55];
pub(crate) const SYSTIMER_TARGET0: u32 = 57;
const FROM_CPU_INTR0: u32 = 79;
/// `INTERRUPT_CORE0_INTR_STATUS_0_REG` to `3`.
const STATUS: std::ops::RangeInclusive<u32> = 0x18c..=0x198;
/// `SYSTEM_CPU_INTR_FROM_CPU_0_REG` to `3`, in the SYSTEM block.
pub(crate) const FROM_CPU: std::ops::RangeInclusive<u32> = 0x30..=0x3c;

/// The CPU line of the non-maskable interrupt.
pub(crate) const NMI: u8 = 14;
/// Where the APP CPU's half starts in the block.
const CORE1: u32 = 0x800;

#[derive(Clone, Debug)]
pub(crate) struct Matrix {
    /// The PRO CPU's map, then the APP CPU's.
    maps: [[u8; SOURCES as usize]; 2],
    /// Bit `n` is `SYSTEM_CPU_INTR_FROM_CPU_n_REG`.
    from_cpu: u32,
}

impl Default for Matrix {
    fn default() -> Matrix {
        Matrix {
            maps: [[0; SOURCES as usize], [16; SOURCES as usize]],
            from_cpu: 0,
        }
    }
}

/// The core whose half `offset` in the block lies in, and the offset in that half.
fn half(offset: u32) -> (usize, u32) {
    if offset >= CORE1 {
        (1, offset - CORE1)
    } else {
        (0, offset)
    }
}

impl Matrix {
    /// The map or status register at `offset` in the block, with `sources` the source levels.
    pub(crate) fn read(&self, offset: u32, sources: u128) -> Option<u32> {
        let (core, offset) = half(offset);
        if offset < 4 * SOURCES {
            return Some(u32::from(self.maps[core][(offset / 4) as usize]));
        }
        if STATUS.contains(&offset) {
            return Some((sources >> (8 * (offset - STATUS.start()))) as u32);
        }
        None
    }

    pub(crate) fn write(&mut self, offset: u32, value: u32) -> bool {
        let (core, offset) = half(offset);
        if offset < 4 * SOURCES {
            self.maps[core][(offset / 4) as usize] = (value & 0x1f) as u8;
            return true;
        }
        STATUS.contains(&offset)
    }

    /// Whether a source is routed to `core`'s CPU line `line`.
    pub(crate) fn routes_to(&self, core: CoreId, line: u8) -> bool {
        self.maps[core as usize].contains(&line)
    }

    pub(crate) fn read_from_cpu(&self, offset: u32) -> u32 {
        (self.from_cpu >> ((offset - FROM_CPU.start()) / 4)) & 1
    }

    pub(crate) fn write_from_cpu(&mut self, offset: u32, value: u32) {
        let bit = 1 << ((offset - FROM_CPU.start()) / 4);
        self.from_cpu = (self.from_cpu & !bit) | if value & 1 != 0 { bit } else { 0 };
    }

    /// The source levels, bit `n` for source `n`, with `systimer` the SYSTIMER alarm lines,
    /// `spi2` SPI2's line, `watchdogs` each timer group's watchdog line, `gpio` the GPIO line and
    /// `i2c` I2C0's.
    pub(crate) fn sources(
        &self,
        systimer: u32,
        spi2: bool,
        watchdogs: [bool; 2],
        gpio: bool,
        i2c: bool,
    ) -> u128 {
        u128::from(systimer) << SYSTIMER_TARGET0
            | u128::from(gpio) << GPIO
            | u128::from(spi2) << SPI2
            | u128::from(i2c) << I2C_EXT0
            | u128::from(watchdogs[0]) << TG_WDT[0]
            | u128::from(watchdogs[1]) << TG_WDT[1]
            | u128::from(self.from_cpu) << FROM_CPU_INTR0
    }

    /// The CPU interrupt lines `sources` drive on `core`.
    pub(crate) fn lines(&self, core: CoreId, mut sources: u128) -> u32 {
        let map = &self.maps[core as usize];
        let mut lines = 0;
        while sources != 0 {
            let source = sources.trailing_zeros();
            sources &= sources - 1;
            lines |= 1 << map[source as usize];
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use CoreId::{App, Pro};

    #[test]
    fn a_source_drives_the_cpu_interrupt_its_map_names() {
        let mut matrix = Matrix::default();
        matrix.write(4 * FROM_CPU_INTR0, 9);
        matrix.write(4 * SYSTIMER_TARGET0, 19);
        assert_eq!(
            matrix.lines(Pro, matrix.sources(0, false, [false; 2], false, false)),
            0
        );
        matrix.write_from_cpu(*FROM_CPU.start(), 1);
        assert_eq!(
            matrix.lines(Pro, matrix.sources(0, false, [false; 2], false, false)),
            1 << 9
        );
        assert_eq!(
            matrix.lines(Pro, matrix.sources(0b1, false, [false; 2], false, false)),
            1 << 9 | 1 << 19
        );
        let status = matrix
            .read(
                STATUS.start() + 8,
                matrix.sources(0, false, [false; 2], false, false),
            )
            .unwrap();
        assert_eq!(status, 1 << (FROM_CPU_INTR0 - 64), "the third status word");
        matrix.write_from_cpu(*FROM_CPU.start(), 0);
        assert_eq!(
            matrix.lines(Pro, matrix.sources(0, false, [false; 2], false, false)),
            0
        );
        matrix.write(4 * SPI2, 5);
        assert_eq!(
            matrix.lines(Pro, matrix.sources(0, true, [false; 2], false, false)),
            1 << 5
        );
        let status = matrix
            .read(
                *STATUS.start(),
                matrix.sources(0, true, [false; 2], false, false),
            )
            .unwrap();
        assert_eq!(status, 1 << SPI2, "the first status word");
        matrix.write(4 * TG_WDT[1], 24);
        assert_eq!(
            matrix.lines(Pro, matrix.sources(0, false, [false, true], false, false)),
            1 << 24
        );
        let status = matrix
            .read(
                STATUS.start() + 4,
                matrix.sources(0, false, [true, false], false, false),
            )
            .unwrap();
        assert_eq!(status, 1 << (TG_WDT[0] - 32), "the second status word");
    }

    #[test]
    fn each_core_takes_the_sources_its_own_map_names() {
        let mut matrix = Matrix::default();
        let from_cpu_1 = FROM_CPU_INTR0 + 1;
        matrix.write_from_cpu(FROM_CPU.start() + 4, 1);
        assert_eq!(matrix.read(CORE1 + 4 * from_cpu_1, 0), Some(16));
        assert_eq!(
            matrix.lines(App, matrix.sources(0, false, [false; 2], false, false)),
            1 << 16
        );
        matrix.write(CORE1 + 4 * from_cpu_1, 2);
        assert_eq!(matrix.read(4 * from_cpu_1, 0), Some(0), "the PRO CPU's map");
        assert_eq!(
            matrix.lines(App, matrix.sources(0, false, [false; 2], false, false)),
            1 << 2
        );
        assert_eq!(
            matrix.lines(Pro, matrix.sources(0, false, [false; 2], false, false)),
            1 << 0
        );
        assert!(!matrix.routes_to(Pro, NMI));
        matrix.write(CORE1 + 4 * SPI2, u32::from(NMI));
        assert!(matrix.routes_to(App, NMI) && !matrix.routes_to(Pro, NMI));
        let status = matrix.read(
            CORE1 + STATUS.start() + 8,
            matrix.sources(0, false, [false; 2], false, false),
        );
        assert_eq!(
            status,
            Some(1 << (from_cpu_1 - 64)),
            "the APP CPU's third status word"
        );
    }
}
