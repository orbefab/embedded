//! The cache controller's operations, which finish at once in a functional cache.
//!
//! Each of lock, unlock, sync, occupy and preload starts on a write of its enable bit, which
//! the hardware clears when done, and sets its done bit. Autoload is a mode whose done bit
//! reads 1, and a freeze reads done while it is on. Every other EXTMEM register is a plain word.
//! IDF v5.5.5, `components/soc/esp32s3/register/soc/extmem_reg.h`.

/// Each operation register: its offset, its start bits and its done bit.
const OPERATIONS: [(u32, u32, u32); 7] = [
    (0x1c, 0b011, 1 << 2),
    (0x28, 0b111, 1 << 3),
    (0x34, 0b001, 1 << 1),
    (0x40, 0b001, 1 << 1),
    (0x7c, 0b011, 1 << 2),
    (0x88, 0b001, 1 << 1),
    (0x94, 0b001, 1 << 1),
];

const AUTOLOADS: [u32; 2] = [0x4c, 0xa0];
const AUTOLOAD_DONE: u32 = 1 << 3;
const FREEZES: [u32; 2] = [0x150, 0x154];
const FREEZE_DONE: u32 = 1 << 2;

/// The word the register at `offset` holds after a write of `value`.
pub(crate) fn written(offset: u32, value: u32) -> u32 {
    if let Some(&(_, start, done)) = OPERATIONS.iter().find(|op| op.0 == offset) {
        if value & start != 0 {
            return (value & !start) | done;
        }
        return value;
    }
    if AUTOLOADS.contains(&offset) {
        value | AUTOLOAD_DONE
    } else if FREEZES.contains(&offset) {
        (value & !FREEZE_DONE) | (value & 1) << 2
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_clears_its_start_and_sets_done_and_a_freeze_reads_done_while_on() {
        assert_eq!(written(0x28, 0b001), 1 << 3, "a data cache invalidate");
        assert_eq!(
            written(0x88, 0b001),
            1 << 1,
            "an instruction cache invalidate"
        );
        assert_eq!(
            written(0x150, 0b001),
            0b101,
            "a freeze stays on and reads done"
        );
        assert_eq!(written(0x150, 0b100), 0, "an ended freeze");
        assert_eq!(written(0xa0, 0), 1 << 3, "a suspended autoload");
        assert_eq!(written(0x0, 0x1234), 0x1234);
    }
}
