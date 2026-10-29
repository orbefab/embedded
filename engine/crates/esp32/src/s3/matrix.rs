//! The interrupt matrix, which routes each peripheral interrupt source to a CPU interrupt, and
//! the four `FROM_CPU` sources that firmware raises by writing a register.
//!
//! Source `n`'s map register, at `4 * n`, names the CPU interrupt it drives, and a CPU interrupt
//! line is high while any source mapped to it is. A source mapped to one of the core's own
//! interrupts drives nothing, as the core takes no external line there. The status registers
//! show each source's level. The sources modeled are SPI2's, the three SYSTIMER alarms and the
//! four `FROM_CPU` registers, and every other source stays low. IDF v5.5.5,
//! `components/soc/esp32s3/include/soc/interrupts.h` and `interrupt_core0_reg.h`.

/// The sources, `ETS_MAX_INTR_SOURCE`.
const SOURCES: u32 = 99;
const SPI2: u32 = 21;
pub(crate) const SYSTIMER_TARGET0: u32 = 57;
const FROM_CPU_INTR0: u32 = 79;
/// `INTERRUPT_CORE0_INTR_STATUS_0_REG` to `3`.
const STATUS: std::ops::RangeInclusive<u32> = 0x18c..=0x198;
/// `SYSTEM_CPU_INTR_FROM_CPU_0_REG` to `3`, in the SYSTEM block.
pub(crate) const FROM_CPU: std::ops::RangeInclusive<u32> = 0x30..=0x3c;

/// The CPU line of the non-maskable interrupt.
pub(crate) const NMI: u8 = 14;

#[derive(Clone, Debug)]
pub(crate) struct Matrix {
    map: [u8; SOURCES as usize],
    /// Bit `n` is `SYSTEM_CPU_INTR_FROM_CPU_n_REG`.
    from_cpu: u32,
}

impl Default for Matrix {
    fn default() -> Matrix {
        Matrix {
            map: [0; SOURCES as usize],
            from_cpu: 0,
        }
    }
}

impl Matrix {
    /// The map or status register at `offset`, with `sources` the source levels.
    pub(crate) fn read(&self, offset: u32, sources: u128) -> Option<u32> {
        if offset < 4 * SOURCES {
            return Some(u32::from(self.map[(offset / 4) as usize]));
        }
        if STATUS.contains(&offset) {
            return Some((sources >> (8 * (offset - STATUS.start()))) as u32);
        }
        None
    }

    pub(crate) fn write(&mut self, offset: u32, value: u32) -> bool {
        if offset < 4 * SOURCES {
            self.map[(offset / 4) as usize] = (value & 0x1f) as u8;
            return true;
        }
        STATUS.contains(&offset)
    }

    /// Whether a source is routed to CPU line `line`.
    pub(crate) fn routes_to(&self, line: u8) -> bool {
        self.map.contains(&line)
    }

    pub(crate) fn read_from_cpu(&self, offset: u32) -> u32 {
        (self.from_cpu >> ((offset - FROM_CPU.start()) / 4)) & 1
    }

    pub(crate) fn write_from_cpu(&mut self, offset: u32, value: u32) {
        let bit = 1 << ((offset - FROM_CPU.start()) / 4);
        self.from_cpu = (self.from_cpu & !bit) | if value & 1 != 0 { bit } else { 0 };
    }

    /// The source levels, bit `n` for source `n`, with `systimer` the SYSTIMER alarm lines and
    /// `spi2` SPI2's line.
    pub(crate) fn sources(&self, systimer: u32, spi2: bool) -> u128 {
        u128::from(systimer) << SYSTIMER_TARGET0
            | u128::from(spi2) << SPI2
            | u128::from(self.from_cpu) << FROM_CPU_INTR0
    }

    /// The CPU interrupt lines `sources` drive.
    pub(crate) fn lines(&self, mut sources: u128) -> u32 {
        let mut lines = 0;
        while sources != 0 {
            let source = sources.trailing_zeros();
            sources &= sources - 1;
            lines |= 1 << self.map[source as usize];
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_drives_the_cpu_interrupt_its_map_names() {
        let mut matrix = Matrix::default();
        matrix.write(4 * FROM_CPU_INTR0, 9);
        matrix.write(4 * SYSTIMER_TARGET0, 19);
        assert_eq!(matrix.lines(matrix.sources(0, false)), 0);
        matrix.write_from_cpu(*FROM_CPU.start(), 1);
        assert_eq!(matrix.lines(matrix.sources(0, false)), 1 << 9);
        assert_eq!(matrix.lines(matrix.sources(0b1, false)), 1 << 9 | 1 << 19);
        let status = matrix
            .read(STATUS.start() + 8, matrix.sources(0, false))
            .unwrap();
        assert_eq!(status, 1 << (FROM_CPU_INTR0 - 64), "the third status word");
        matrix.write_from_cpu(*FROM_CPU.start(), 0);
        assert_eq!(matrix.lines(matrix.sources(0, false)), 0);
        matrix.write(4 * SPI2, 5);
        assert_eq!(matrix.lines(matrix.sources(0, true)), 1 << 5);
        let status = matrix
            .read(*STATUS.start(), matrix.sources(0, true))
            .unwrap();
        assert_eq!(status, 1 << SPI2, "the first status word");
    }
}
