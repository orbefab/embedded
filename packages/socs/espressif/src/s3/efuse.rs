//! The eFuse controller, with one eFuse set in its read registers.
//!
//! An [`EfuseSet`] gives blocks 1 and 2 by field, from `efuse.txt`, and block 0 is all 0. The
//! default set, `synthetic`, is a revision 0.2 chip in the board's package, with 8 MB of PSRAM in
//! the package, no flash in it, block version 1.3, the MAC `02:00:00:00:00:01`, and every other
//! bit 0, the calibration and the unique ID among them. The read registers hold the set from the
//! start. A read command finishes at once. A program command would burn eFuses, so its write
//! ends the run. Every other register is a plain word that starts at 0. IDF v5.5.5,
//! `components/efuse/esp32s3/esp_efuse_table.csv` and
//! `components/soc/esp32s3/register/soc/efuse_reg.h`.

/// `EFUSE_RD_WR_DIS_REG`, the first read register.
const READ: u32 = 0x2c;
/// The words of blocks 0, 1 and 2 from `READ` on. Every read register after them reads 0.
const WORDS: usize = 20;
/// The first word of each block in the read registers: `EFUSE_RD_WR_DIS_REG`,
/// `EFUSE_RD_MAC_SPI_SYS_0_REG` and `EFUSE_RD_SYS_PART1_DATA0_REG`.
const BLOCK_WORD: [usize; 3] = [0, 6, 12];
/// The bits of each block.
#[cfg(test)]
const BLOCK_BITS: [u32; 3] = [192, 192, 256];

/// Every field a set can name, each as its pieces from the lowest bits of its value up: a block,
/// the first bit in it, and the piece's length. The MAC's first byte is its value's lowest.
type Pieces = &'static [(usize, u32, u32)];

const FIELDS: &[(&str, Pieces)] = &[
    (
        "MAC",
        &[
            (1, 40, 8),
            (1, 32, 8),
            (1, 24, 8),
            (1, 16, 8),
            (1, 8, 8),
            (1, 0, 8),
        ],
    ),
    ("SPI_PAD_CONFIG_CLK", &[(1, 48, 6)]),
    ("SPI_PAD_CONFIG_Q", &[(1, 54, 6)]),
    ("SPI_PAD_CONFIG_D", &[(1, 60, 6)]),
    ("SPI_PAD_CONFIG_CS", &[(1, 66, 6)]),
    ("SPI_PAD_CONFIG_HD", &[(1, 72, 6)]),
    ("SPI_PAD_CONFIG_WP", &[(1, 78, 6)]),
    ("SPI_PAD_CONFIG_DQS", &[(1, 84, 6)]),
    ("SPI_PAD_CONFIG_D4", &[(1, 90, 6)]),
    ("SPI_PAD_CONFIG_D5", &[(1, 96, 6)]),
    ("SPI_PAD_CONFIG_D6", &[(1, 102, 6)]),
    ("SPI_PAD_CONFIG_D7", &[(1, 108, 6)]),
    ("WAFER_VERSION_MINOR_LO", &[(1, 114, 3)]),
    ("PKG_VERSION", &[(1, 117, 3)]),
    ("BLK_VERSION_MINOR", &[(1, 120, 3)]),
    ("FLASH_CAP", &[(1, 123, 3)]),
    ("FLASH_TEMP", &[(1, 126, 2)]),
    ("FLASH_VENDOR", &[(1, 128, 3)]),
    // Two fields in the csv, and two lines of `espefuse summary`. The 2-bit piece is
    // `PSRAM_CAP`; bit 179 is `PSRAM_CAP_3`. Summary's `PSRAM_CAPACITY` is calculated
    // from them and is not a field.
    ("PSRAM_CAP", &[(1, 131, 2)]),
    ("PSRAM_CAP_3", &[(1, 179, 1)]),
    ("PSRAM_TEMP", &[(1, 133, 2)]),
    ("PSRAM_VENDOR", &[(1, 135, 2)]),
    ("K_RTC_LDO", &[(1, 141, 7)]),
    ("K_DIG_LDO", &[(1, 148, 7)]),
    ("V_RTC_DBIAS20", &[(1, 155, 8)]),
    ("V_DIG_DBIAS20", &[(1, 163, 8)]),
    ("DIG_DBIAS_HVT", &[(1, 171, 5)]),
    ("WAFER_VERSION_MINOR_HI", &[(1, 183, 1)]),
    ("WAFER_VERSION_MAJOR", &[(1, 184, 2)]),
    ("ADC2_CAL_VOL_ATTEN3", &[(1, 186, 6)]),
    ("OPTIONAL_UNIQUE_ID", &[(2, 0, 128)]),
    ("BLK_VERSION_MAJOR", &[(2, 128, 2)]),
    ("TEMP_CALIB", &[(2, 132, 9)]),
    ("OCODE", &[(2, 141, 8)]),
    ("ADC1_INIT_CODE_ATTEN0", &[(2, 149, 8)]),
    ("ADC1_INIT_CODE_ATTEN1", &[(2, 157, 6)]),
    ("ADC1_INIT_CODE_ATTEN2", &[(2, 163, 6)]),
    ("ADC1_INIT_CODE_ATTEN3", &[(2, 169, 6)]),
    ("ADC2_INIT_CODE_ATTEN0", &[(2, 175, 8)]),
    ("ADC2_INIT_CODE_ATTEN1", &[(2, 183, 6)]),
    ("ADC2_INIT_CODE_ATTEN2", &[(2, 189, 6)]),
    ("ADC2_INIT_CODE_ATTEN3", &[(2, 195, 6)]),
    ("ADC1_CAL_VOL_ATTEN0", &[(2, 201, 8)]),
    ("ADC1_CAL_VOL_ATTEN1", &[(2, 209, 8)]),
    ("ADC1_CAL_VOL_ATTEN2", &[(2, 217, 8)]),
    ("ADC1_CAL_VOL_ATTEN3", &[(2, 225, 8)]),
    ("ADC2_CAL_VOL_ATTEN0", &[(2, 233, 8)]),
    ("ADC2_CAL_VOL_ATTEN1", &[(2, 241, 7)]),
    ("ADC2_CAL_VOL_ATTEN2", &[(2, 248, 7)]),
];

/// The sets, as `efuse.txt` gives them.
const SETS: &str = include_str!("efuse.txt");

/// Names `espefuse summary` prints that are not fields of the csv. They are an error, not a
/// skipped line: the calculated number can disagree with the pieces IDF reads.
const CALCULATED: &[(&str, &str)] = &[
    (
        "PSRAM_CAPACITY",
        "PSRAM_CAP and PSRAM_CAP_3 (PSRAM_CAP_3 << 2 | PSRAM_CAP)",
    ),
    (
        "WAFER_VERSION_MINOR",
        "WAFER_VERSION_MINOR_LO and WAFER_VERSION_MINOR_HI (HI << 3 | LO)",
    ),
];

/// Why a section of `efuse.txt` could not be read. A netlist that names the set reports this
/// as `netlist.syntax` and does not panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EfuseParseError {
    message: String,
}

impl EfuseParseError {
    fn new(message: impl Into<String>) -> EfuseParseError {
        EfuseParseError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EfuseParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EfuseParseError {}

/// The eFuses of one chip: blocks 0, 1 and 2, block 0 all 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EfuseSet {
    name: &'static str,
    words: [u32; WORDS],
}

impl EfuseSet {
    /// The set a part that names none has.
    pub const DEFAULT: &'static str = "synthetic";

    /// The name of every set, in `efuse.txt` order.
    pub fn names() -> impl Iterator<Item = &'static str> {
        sections()
            .expect("efuse.txt starts each set with its name")
            .into_iter()
            .map(|(name, _)| name)
    }

    /// The set called `name`, if there is one. `Err` when that section does not parse, which a
    /// netlist reports instead of panicking.
    pub fn named(name: &str) -> Result<Option<EfuseSet>, EfuseParseError> {
        let sections = sections()?;
        let Some((name, lines)) = sections.into_iter().find(|(found, _)| *found == name) else {
            return Ok(None);
        };
        Ok(Some(fill(name, &lines)?))
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The pieces of `field`, when it is one csv field.
    fn pieces(field: &str) -> Option<&'static [(usize, u32, u32)]> {
        FIELDS
            .iter()
            .find(|&&(name, _)| name == field)
            .map(|&(_, pieces)| pieces)
    }

    fn set(&mut self, field: &str, mut value: u128) -> Result<(), EfuseParseError> {
        if let Some(from) = CALCULATED
            .iter()
            .find(|&&(name, _)| name == field)
            .map(|&(_, from)| from)
        {
            return Err(EfuseParseError::new(format!(
                "{field} in [{}] is calculated by espefuse from {from}, and it is not a field IDF reads",
                self.name
            )));
        }
        let Some(pieces) = Self::pieces(field) else {
            return Err(EfuseParseError::new(format!(
                "{field} in [{}] is not a field of block 1 or block 2",
                self.name
            )));
        };
        for &(block, start, len) in pieces {
            for bit in start..start + len {
                let at = BLOCK_WORD[block] + (bit / 32) as usize;
                self.words[at] &= !(1 << (bit % 32));
                self.words[at] |= ((value & 1) as u32) << (bit % 32);
                value >>= 1;
            }
        }
        if value != 0 {
            return Err(EfuseParseError::new(format!(
                "{field} in [{}] is wider than its bits",
                self.name
            )));
        }
        Ok(())
    }

    /// This set with `field` at `value`.
    #[cfg(test)]
    pub(crate) fn with(mut self, field: &str, value: u128) -> EfuseSet {
        self.set(field, value)
            .unwrap_or_else(|error| panic!("{error}"));
        self
    }

    /// The raw value of `field`.
    pub(crate) fn field(&self, field: &str) -> u128 {
        let mut value = 0;
        let mut shift = 0;
        let pieces = Self::pieces(field)
            .unwrap_or_else(|| panic!("{field} is not a field of block 1 or block 2"));
        for &(block, start, len) in pieces {
            for bit in start..start + len {
                let at = BLOCK_WORD[block] + (bit / 32) as usize;
                value |= u128::from(self.words[at] >> (bit % 32) & 1) << shift;
                shift += 1;
            }
        }
        value
    }
}

impl Default for EfuseSet {
    fn default() -> EfuseSet {
        match EfuseSet::named(EfuseSet::DEFAULT) {
            Ok(Some(set)) => set,
            Ok(None) => panic!("efuse.txt has the default set"),
            Err(error) => panic!("the default eFuse set did not parse: {error}"),
        }
    }
}

/// One set from its lines. `name` is stored on the set and named in an error.
fn fill(name: &'static str, lines: &[&str]) -> Result<EfuseSet, EfuseParseError> {
    let mut set = EfuseSet {
        name,
        words: [0; WORDS],
    };
    for line in lines {
        let Some((field, value)) = line.split_once(char::is_whitespace) else {
            return Err(EfuseParseError::new(format!(
                "`{line}` in [{name}] is a field and a value"
            )));
        };
        let value = value.trim();
        let Some(value) = (if field == "MAC" {
            mac(value)
        } else {
            number(value)
        }) else {
            return Err(EfuseParseError::new(format!(
                "`{value}` of {field} in [{name}] is not a value"
            )));
        };
        set.set(field, value)?;
    }
    Ok(set)
}

/// Each `[name]` section of `efuse.txt`, with its lines that are not blank or a comment.
fn sections() -> Result<Vec<(&'static str, Vec<&'static str>)>, EfuseParseError> {
    let lines = SETS
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    let mut sections: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for line in lines {
        match line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            Some(name) => sections.push((name, Vec::new())),
            None => {
                let Some(section) = sections.last_mut() else {
                    return Err(EfuseParseError::new(
                        "efuse.txt names a field before a set".to_string(),
                    ));
                };
                section.1.push(line);
            }
        }
    }
    Ok(sections)
}

fn number(text: &str) -> Option<u128> {
    if let Some(hex) = text.strip_prefix("0x") {
        u128::from_str_radix(hex, 16).ok()
    } else if let Some(binary) = text.strip_prefix("0b") {
        u128::from_str_radix(binary, 2).ok()
    } else {
        text.parse().ok()
    }
}

/// A MAC as `aa:bb:cc:dd:ee:ff`, its first byte lowest.
fn mac(text: &str) -> Option<u128> {
    let bytes: Vec<u8> = text
        .split(':')
        .map(|byte| u8::from_str_radix(byte, 16).ok())
        .collect::<Option<_>>()?;
    (bytes.len() == 6).then(|| {
        bytes
            .iter()
            .rev()
            .fold(0, |value, &byte| value << 8 | u128::from(byte))
    })
}

/// The last read register, `EFUSE_RD_RS_ERR1_REG`.
const READ_END: u32 = 0x1c4;
const CMD: u32 = 0x1d4;
const READ_CMD: u32 = 1 << 0;
const PGM_CMD: u32 = 1 << 1;
const SIZE: u32 = 0x200;

#[derive(Clone, Debug)]
pub(crate) struct Efuse {
    pub(crate) set: EfuseSet,
    /// The writable registers, by word.
    plain: Vec<u32>,
}

impl Default for Efuse {
    fn default() -> Efuse {
        Efuse {
            set: EfuseSet::default(),
            plain: vec![0; (SIZE / 4) as usize],
        }
    }
}

impl Efuse {
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        if (READ..=READ_END).contains(&offset) {
            return Some(
                self.set
                    .words
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

    /// The words the synthetic set had before sets were data.
    #[test]
    fn the_synthetic_set_is_the_fixed_one_it_replaced() {
        let words = [
            0,
            0,
            0,
            0,
            0,
            0,
            0x0000_0001,
            0x0000_0200,
            0,
            0x0308_0000,
            0x0000_00c8,
            0,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
        ];
        assert_eq!(EfuseSet::default().words, words);
        assert_eq!(EfuseSet::default().field("MAC"), 0x0100_0000_0002);
        assert_eq!(EfuseSet::default().field("BLK_VERSION_MAJOR"), 1);
        assert_eq!(EfuseSet::default().field("PSRAM_CAP"), 1);
    }

    #[test]
    fn every_set_parses_and_every_field_lies_in_its_block() {
        for &(name, pieces) in FIELDS {
            for &(block, start, len) in pieces {
                assert!(start + len <= BLOCK_BITS[block], "{name}");
            }
        }
        let names: Vec<&str> = EfuseSet::names().collect();
        assert!(names.contains(&EfuseSet::DEFAULT));
        for name in names {
            let set = EfuseSet::named(name).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(set.map(|set| set.name()), Some(name));
        }
        assert_eq!(EfuseSet::named("no such set").unwrap(), None);
    }

    /// `format-test` in `efuse.txt` is a made-up summary excerpt, not a board. `0x81` stays the
    /// unsigned 129 IDF reads.
    #[test]
    fn a_summary_excerpt_parses_to_the_raw_unsigned_words() {
        let set = EfuseSet::named("format-test")
            .unwrap_or_else(|error| panic!("{error}"))
            .expect("format-test");
        assert_eq!(set.field("MAC"), 0x5544_3322_1102);
        assert_eq!(set.field("PSRAM_CAP"), 0b01);
        assert_eq!(set.field("PSRAM_CAP_3"), 1);
        assert_eq!(set.field("ADC1_INIT_CODE_ATTEN0"), 129);
        assert_eq!(
            set.words,
            [
                0,
                0,
                0,
                0,
                0,
                0,
                0x2233_4455,
                0x0000_0211,
                0,
                0,
                0x8,
                0x0008_0000,
                0,
                0,
                0,
                0,
                0x1020_0000,
                0,
                0,
                0,
            ]
        );
    }

    #[test]
    fn a_calculated_or_unknown_field_is_an_error() {
        let calculated = fill("excerpt", &["PSRAM_CAPACITY 0b101"]).unwrap_err();
        assert!(
            calculated.to_string().contains("PSRAM_CAPACITY"),
            "{calculated}"
        );
        assert!(
            calculated.to_string().contains("PSRAM_CAP_3"),
            "{calculated}"
        );
        let wafer = fill("excerpt", &["WAFER_VERSION_MINOR 2"]).unwrap_err();
        assert!(
            wafer.to_string().contains("WAFER_VERSION_MINOR_LO"),
            "{wafer}"
        );
        let unknown = fill("excerpt", &["NOT_A_FIELD 1"]).unwrap_err();
        assert!(unknown.to_string().contains("NOT_A_FIELD"), "{unknown}");
        let wide = fill("excerpt", &["PSRAM_CAP 0b101"]).unwrap_err();
        assert!(wide.to_string().contains("wider than its bits"), "{wide}");
    }

    #[test]
    fn a_field_reads_back_what_was_set_across_its_pieces() {
        let mut set = EfuseSet {
            name: "test",
            words: [0; WORDS],
        };
        set.set("PSRAM_CAP", 0b01).unwrap();
        set.set("PSRAM_CAP_3", 0b1).unwrap();
        set.set("ADC2_CAL_VOL_ATTEN3", 0b10_0001).unwrap();
        set.set("ADC1_INIT_CODE_ATTEN0", 0xa5).unwrap();
        assert_eq!(set.field("PSRAM_CAP"), 0b01);
        assert_eq!(set.field("PSRAM_CAP_3"), 1);
        assert_eq!(set.words[6 + 131 / 32] >> (131 % 32) & 0b11, 0b01);
        assert_eq!(set.words[6 + 179 / 32] >> (179 % 32) & 1, 1);
        assert_eq!(set.field("ADC2_CAL_VOL_ATTEN3"), 0b10_0001);
        assert_eq!(set.field("ADC1_INIT_CODE_ATTEN0"), 0xa5);
        assert_eq!(set.field("ADC1_INIT_CODE_ATTEN1"), 0);
        assert_eq!(number("0b1010"), Some(10));
        assert_eq!(number("0x1f"), Some(31));
        assert_eq!(mac("7c:df:a1:00:00:ff"), Some(0xff00_00a1_df7c));
    }

    #[test]
    fn a_program_command_is_refused() {
        let mut efuse = Efuse::default();
        assert_eq!(efuse.write(0x1cc, 0x5a5a), Ok(true));
        assert_eq!(efuse.write(CMD, PGM_CMD), Err(()));
        assert_eq!(efuse.write(CMD, PGM_CMD | READ_CMD), Err(()));
    }
}
