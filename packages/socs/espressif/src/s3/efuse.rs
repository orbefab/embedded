//! The eFuse controller, with a fixed synthetic set of eFuses.
//!
//! The set is a revision 0.2 chip in the board's package, with 8 MB of PSRAM in the package, no
//! flash in it, block version 1.3, the MAC `02:00:00:00:00:01`, and every other bit 0, the
//! calibration and the unique ID among them. The read registers hold it from the start. A read
//! command finishes at once. A program command would burn eFuses, so its write ends the run.
//! Every other register is a plain word that starts at 0. IDF v5.5.5, `components/efuse/esp32s3/esp_efuse_table.csv`
//! and `components/soc/esp32s3/register/soc/efuse_reg.h`.

/// `EFUSE_RD_WR_DIS_REG`, the first read register.
const READ: u32 = 0x2c;
/// The words of blocks 0, 1 and 2 from `READ` on, and every read register after them reads 0.
const BLOCKS: [u32; 20] = [
    // Block 0: `EFUSE_RD_WR_DIS_REG` and `EFUSE_RD_REPEAT_DATA0_REG` to `4`.
    0,
    0,
    0,
    0,
    0,
    0,
    // Block 1: the MAC in bits 0 to 47 with its first byte at bit 40, `WAFER_VERSION_MINOR_LO`
    // 2 at bit 114, `BLK_VERSION_MINOR` 3 at bit 120, `PSRAM_CAP` 1 at bit 131, `PSRAM_TEMP` 2
    // at bit 133 and `PSRAM_VENDOR` 1 at bit 135.
    0x0000_0001,
    0x0000_0200,
    0,
    0x0308_0000,
    0x0000_00c8,
    0,
    // Block 2: `BLK_VERSION_MAJOR` 1 at bit 128.
    0,
    0,
    0,
    0,
    1,
    0,
    0,
    0,
];
/// The last read register, `EFUSE_RD_RS_ERR1_REG`.
const READ_END: u32 = 0x1c4;
const CMD: u32 = 0x1d4;
const READ_CMD: u32 = 1 << 0;
const PGM_CMD: u32 = 1 << 1;
const SIZE: u32 = 0x200;

#[derive(Clone, Debug)]
pub(crate) struct Efuse {
    /// The writable registers, by word.
    plain: Vec<u32>,
}

impl Default for Efuse {
    fn default() -> Efuse {
        Efuse {
            plain: vec![0; (SIZE / 4) as usize],
        }
    }
}

impl Efuse {
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        if (READ..=READ_END).contains(&offset) {
            return Some(
                BLOCKS
                    .get(((offset - READ) / 4) as usize)
                    .copied()
                    .unwrap_or(0),
            );
        }
        self.plain.get((offset / 4) as usize).copied()
    }

    /// Writes a register. `Err` when the write starts a program command.
    pub(crate) fn write(&mut self, offset: u32, value: u32) -> Result<bool, ()> {
        if offset == CMD && value & PGM_CMD != 0 {
            return Err(());
        }
        if (READ..=READ_END).contains(&offset) {
            return Ok(true);
        }
        let value = if offset == CMD {
            value & !READ_CMD
        } else {
            value
        };
        Ok(self
            .plain
            .get_mut((offset / 4) as usize)
            .map(|word| *word = value)
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blocks_read_the_synthetic_set_and_a_read_command_finishes_at_once() {
        let mut efuse = Efuse::default();
        assert_eq!(efuse.read(0x44), Some(0x0000_0001), "the MAC's last byte");
        assert_eq!(efuse.read(0x48), Some(0x0000_0200), "the MAC's first byte");
        assert_eq!(efuse.read(0x50), Some(0x0308_0000));
        assert_eq!(efuse.read(0x6c), Some(1), "BLK_VERSION_MAJOR");
        assert_eq!(efuse.read(0x7c), Some(0), "block 3 on");
        assert_eq!(efuse.write(0x1cc, 0x5aa5), Ok(true));
        assert_eq!(efuse.write(CMD, READ_CMD), Ok(true));
        assert_eq!(efuse.read(CMD), Some(0));
        assert_eq!(efuse.write(0x44, 0xffff_ffff), Ok(true));
        assert_eq!(
            efuse.read(0x44),
            Some(0x0000_0001),
            "the read registers keep their set"
        );
    }

    #[test]
    fn a_program_command_is_refused() {
        let mut efuse = Efuse::default();
        assert_eq!(efuse.write(0x1cc, 0x5a5a), Ok(true));
        assert_eq!(efuse.write(CMD, PGM_CMD), Err(()));
        assert_eq!(efuse.write(CMD, PGM_CMD | READ_CMD), Err(()));
    }
}
