//! Intel HEX and ELF32 images for an AVR die.
//!
//! HEX records 00, 01, 02, 03, 04, and 05 are accepted. The checksum is the
//! two's-complement byte of the record, and a bad record names its line.
//! ELF is 32-bit little-endian with `e_machine` `EM_AVR` (83). A `PT_LOAD`
//! whose `p_paddr` is below `0x800000` is flash. A segment at `0x800000`
//! (data) or `0x810000` (EEPROM) is not placed in flash; [`Image::notes`]
//! records it.

use std::fmt;

use object::LittleEndian as Le;
use object::elf::{FileHeader32, PN_XNUM};
use object::read::elf::{FileHeader, ProgramHeader};

/// Flash image. Gaps between placed bytes are `0xFF` (erased flash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    bytes: Vec<u8>,
    notes: Vec<IgnoredSegment>,
}

/// A `PT_LOAD` that is not flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgnoredSegment {
    pub region: IgnoredRegion,
    pub paddr: u32,
    pub filesz: u32,
}

/// AVR ELF region above flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoredRegion {
    /// `p_paddr` in `0x00800000`–`0x0080FFFF`. SRAM, not flash.
    Data,
    /// `p_paddr` in `0x00810000`–`0x0081FFFF`. EEPROM, not flash.
    Eeprom,
}

impl IgnoredSegment {
    /// One sentence a caller can print.
    pub fn note(&self) -> String {
        let name = match self.region {
            IgnoredRegion::Data => "data",
            IgnoredRegion::Eeprom => "EEPROM",
        };
        format!(
            "ignored {name} segment at p_paddr {:#08x} ({} bytes); not placed in flash",
            self.paddr, self.filesz
        )
    }
}

impl Image {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn notes(&self) -> &[IgnoredSegment] {
        &self.notes
    }
}

/// A HEX or ELF file this loader will not accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// `line` is 1-based.
    Hex {
        line: usize,
        detail: String,
    },
    Elf(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hex { line, detail } => write!(f, "Intel HEX line {line}: {detail}"),
            Self::Elf(detail) => write!(f, "ELF: {detail}"),
        }
    }
}

impl std::error::Error for ImageError {}

const DATA_BASE: u32 = 0x0080_0000;
const EEPROM_BASE: u32 = 0x0081_0000;
const REGION: u32 = 0x1_0000;
const EM_AVR: u16 = 83;
const PT_LOAD: u32 = 1;
const ELF32_HEADER: usize = 52;
const PHDR32: usize = 32;

/// Parse an Intel HEX file. `text` may use `\n` or `\r\n`.
pub fn parse_hex(text: &str) -> Result<Image, ImageError> {
    let mut bytes = Vec::new();
    let mut base: u32 = 0;
    let mut saw_eof = false;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if saw_eof {
            return Err(hex(line_no, "record after end of file"));
        }
        let record = parse_hex_line(line_no, line)?;
        match record.kind {
            0x00 => {
                let addr = base
                    .checked_add(u32::from(record.address))
                    .ok_or_else(|| hex(line_no, "address overflow"))?;
                place(&mut bytes, addr, &record.data).map_err(|detail| ImageError::Hex {
                    line: line_no,
                    detail,
                })?;
            }
            0x01 => saw_eof = true,
            0x02 => {
                if record.data.len() != 2 {
                    return Err(hex(line_no, "extended segment address is not 2 bytes"));
                }
                let segment = u32::from(u16::from_be_bytes([record.data[0], record.data[1]]));
                base = segment << 4;
            }
            0x03 => {
                if record.data.len() != 4 {
                    return Err(hex(line_no, "start segment address is not 4 bytes"));
                }
            }
            0x04 => {
                if record.data.len() != 2 {
                    return Err(hex(line_no, "extended linear address is not 2 bytes"));
                }
                let upper = u32::from(u16::from_be_bytes([record.data[0], record.data[1]]));
                base = upper << 16;
                if base >= DATA_BASE {
                    return Err(hex(
                        line_no,
                        format!("address {base:#x} is at or above {DATA_BASE:#x}"),
                    ));
                }
            }
            0x05 => {
                if record.data.len() != 4 {
                    return Err(hex(line_no, "start linear address is not 4 bytes"));
                }
            }
            other => {
                return Err(hex(line_no, format!("unknown record type {other}")));
            }
        }
    }
    if !saw_eof {
        return Err(hex(
            text.lines().count().max(1),
            "missing end-of-file record",
        ));
    }
    Ok(Image {
        bytes,
        notes: Vec::new(),
    })
}

struct HexRecord {
    address: u16,
    kind: u8,
    data: Vec<u8>,
}

fn parse_hex_line(line_no: usize, line: &str) -> Result<HexRecord, ImageError> {
    let Some(rest) = line.strip_prefix(':') else {
        return Err(hex(line_no, "line does not start with ':'"));
    };
    if rest.len() % 2 != 0 || rest.len() < 10 {
        return Err(hex(line_no, "truncated record"));
    }
    let raw = decode_hex(line_no, rest)?;
    let count = usize::from(raw[0]);
    if raw.len() != count + 5 {
        return Err(hex(line_no, "length does not match the byte count"));
    }
    let sum = raw.iter().fold(0u8, |acc, byte| acc.wrapping_add(*byte));
    if sum != 0 {
        return Err(hex(line_no, "bad checksum"));
    }
    let address = u16::from_be_bytes([raw[1], raw[2]]);
    let kind = raw[3];
    let data = raw[4..4 + count].to_vec();
    Ok(HexRecord {
        address,
        kind,
        data,
    })
}

fn decode_hex(line_no: usize, text: &str) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::with_capacity(text.len() / 2);
    let digits: Vec<u8> = text.bytes().collect();
    for pair in digits.chunks(2) {
        let hi = hex_nibble(pair[0]).ok_or_else(|| hex(line_no, "not hexadecimal"))?;
        let lo = hex_nibble(pair[1]).ok_or_else(|| hex(line_no, "not hexadecimal"))?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn place(bytes: &mut Vec<u8>, addr: u32, data: &[u8]) -> Result<(), String> {
    if addr >= DATA_BASE {
        return Err(format!("address {addr:#x} is at or above {DATA_BASE:#x}"));
    }
    let len = u32::try_from(data.len()).map_err(|_| "record does not fit".to_string())?;
    let end = addr
        .checked_add(len)
        .ok_or_else(|| "address overflow".to_string())?;
    if end > DATA_BASE {
        return Err(format!("address {addr:#x} crosses {DATA_BASE:#x}"));
    }
    let start = usize::try_from(addr).map_err(|_| "address does not fit".to_string())?;
    let end = start
        .checked_add(data.len())
        .ok_or_else(|| "address overflow".to_string())?;
    if bytes.len() < end {
        bytes.resize(end, 0xFF);
    }
    bytes[start..end].copy_from_slice(data);
    Ok(())
}

fn hex(line: usize, detail: impl Into<String>) -> ImageError {
    ImageError::Hex {
        line,
        detail: detail.into(),
    }
}

/// Parse an ELF32 little-endian AVR file. `e_phnum` 0 is no program header table. A table at
/// offset 0 or with the extended count `PN_XNUM` is refused.
pub fn parse_elf(file: &[u8]) -> Result<Image, ImageError> {
    let header = FileHeader32::<Le>::parse(file)
        .ok()
        .filter(|header| header.endian().is_ok())
        .ok_or_else(|| {
            ImageError::Elf(match file {
                _ if file.len() < ELF32_HEADER => "truncated ELF header".into(),
                [0x7F, b'E', b'L', b'F', 1, 1, version, ..] => {
                    format!("ELF version {version} is not 1")
                }
                [0x7F, b'E', b'L', b'F', 1, ..] => "not little-endian".into(),
                [0x7F, b'E', b'L', b'F', ..] => "not ELF32".into(),
                _ => "not an ELF file".into(),
            })
        })?;
    let machine = header.e_machine(Le).0;
    if machine != EM_AVR {
        return Err(ImageError::Elf(format!(
            "e_machine {machine} is not EM_AVR ({EM_AVR})"
        )));
    }
    let phentsize = header.e_phentsize(Le);
    if usize::from(phentsize) != PHDR32 {
        return Err(ImageError::Elf(format!(
            "program header size {phentsize} is not {PHDR32}"
        )));
    }
    let programs = match (header.e_phoff(Le), header.e_phnum(Le)) {
        (_, 0) => &[][..],
        (0, phnum) => {
            return Err(ImageError::Elf(format!(
                "e_phoff is 0 with e_phnum {phnum}"
            )));
        }
        (_, PN_XNUM) => {
            return Err(ImageError::Elf(
                "extended e_phnum (PN_XNUM) is not supported".into(),
            ));
        }
        _ => header.program_headers(Le, file).map_err(|_| {
            ImageError::Elf("program header extends past the end of the file".into())
        })?,
    };
    let mut bytes = Vec::new();
    let mut notes = Vec::new();
    for program in programs {
        if program.p_type(Le).0 != PT_LOAD {
            continue;
        }
        let (offset, paddr) = (program.p_offset(Le), program.p_paddr(Le));
        let (filesz, memsz) = (program.p_filesz(Le), program.p_memsz(Le));
        if memsz < filesz {
            return Err(ImageError::Elf(format!(
                "PT_LOAD at {paddr:#x} has p_memsz {memsz} below p_filesz {filesz}"
            )));
        }
        if let Some(region) = ignored_region(paddr) {
            notes.push(IgnoredSegment {
                region,
                paddr,
                filesz,
            });
            continue;
        }
        if paddr >= DATA_BASE {
            return Err(ImageError::Elf(format!(
                "p_paddr {paddr:#x} is outside flash, data, and EEPROM"
            )));
        }
        let end = paddr
            .checked_add(memsz)
            .ok_or_else(|| ImageError::Elf("flash segment address overflows".to_string()))?;
        if end > DATA_BASE {
            return Err(ImageError::Elf(format!(
                "flash segment at {paddr:#x} crosses {DATA_BASE:#x}"
            )));
        }
        let data = program.data(Le, file).map_err(|()| {
            ImageError::Elf(format!(
                "PT_LOAD at file offset {offset:#x} extends past the end of the file"
            ))
        })?;
        place(&mut bytes, paddr, data).map_err(ImageError::Elf)?;
        if memsz > filesz {
            let zero_at = paddr + filesz;
            let zeros = vec![0u8; (memsz - filesz) as usize];
            place(&mut bytes, zero_at, &zeros).map_err(ImageError::Elf)?;
        }
    }
    Ok(Image { bytes, notes })
}

fn ignored_region(paddr: u32) -> Option<IgnoredRegion> {
    if (DATA_BASE..EEPROM_BASE).contains(&paddr) {
        Some(IgnoredRegion::Data)
    } else if (EEPROM_BASE..EEPROM_BASE + REGION).contains(&paddr) {
        Some(IgnoredRegion::Eeprom)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eof() -> &'static str {
        ":00000001FF\n"
    }

    #[test]
    fn hex_data_is_little_endian_flash() {
        let image = parse_hex(&format!(":02000000010CF1\n{}", eof())).unwrap();
        assert_eq!(image.bytes(), &[0x01, 0x0C]);
    }

    #[test]
    fn bad_checksum_names_the_line() {
        let err = parse_hex(":0100000012ED\n:010000001200\n").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("line 2"), "{text}");
        assert!(text.contains("bad checksum"), "{text}");
    }

    #[test]
    fn unknown_record_names_the_line() {
        let err = parse_hex(":00000006FA\n").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("line 1"), "{text}");
        assert!(text.contains("unknown record type 6"), "{text}");
    }

    #[test]
    fn extended_addresses_and_start_records() {
        // Type 04 sets bits 31:16. The data byte lands at 0x10000.
        let linear = parse_hex(":020000040001F9\n:01000000AB54\n:00000001FF\n").unwrap();
        assert_eq!(linear.bytes().len(), 0x10001);
        assert_eq!(linear.bytes()[0], 0xFF);
        assert_eq!(*linear.bytes().last().unwrap(), 0xAB);

        // Type 02: segment 0x1000 << 4 = 0x10000.
        let segment = parse_hex(":020000021000EC\n:01000000AB54\n:00000001FF\n").unwrap();
        assert_eq!(segment.bytes(), linear.bytes());

        // Types 03 and 05 carry a start address and do not write flash.
        let start = parse_hex(":0400000300000000F9\n:0400000500000000F7\n:00000001FF\n").unwrap();
        assert!(start.bytes().is_empty());
    }

    #[test]
    fn hex_address_at_or_above_the_data_base_is_rejected() {
        // Type 04 with upper word 0x0080 is base 0x00800000. That must fail
        // before the image buffer grows to the data-space base.
        let at_base = parse_hex(":0200000400807A\n:00000001FF\n").unwrap_err();
        assert!(at_base.to_string().contains("0x800000"), "{at_base}");
        // Upper word 0xFFFF would ask for a 4 GiB buffer if it were allocated.
        let huge = parse_hex(":02000004FFFFFC\n:01000000AA55\n:00000001FF\n").unwrap_err();
        assert!(
            huge.to_string().contains("0x800000") || huge.to_string().contains("ffff"),
            "{huge}"
        );
    }

    #[test]
    fn image_larger_than_the_die_is_an_error() {
        // One byte at address 32768 makes a 32769-byte image. Flash is 32768.
        let over = parse_hex(":01800000FF80\n:00000001FF\n").unwrap();
        assert_eq!(over.bytes().len(), 32 * 1024 + 1);
        let mut cpu = crate::Cpu::new();
        let err = cpu.load_image(&over).unwrap_err();
        assert!(err.to_string().contains("32769"), "{}", err);
        assert_eq!(err.flash_bytes, 32 * 1024);

        let exact = parse_hex(":017FFF000081\n:00000001FF\n").unwrap();
        assert_eq!(exact.bytes().len(), 32 * 1024);
        cpu.load_image(&exact).unwrap();
        // Byte 0x7FFE stays the erased 0xFF. Byte 0x7FFF is the record's 0x00.
        assert_eq!(cpu.flash_word(0x3FFF), 0x00FF);
    }

    #[test]
    fn elf_rejects_class_endian_and_a_short_header() {
        let class = elf_header(2, 1, 0, 0, 0);
        let err = parse_elf(&class).unwrap_err();
        assert!(err.to_string().contains("not ELF32"), "{err}");

        let endian = elf_header(1, 2, 0, 0, 0);
        let err = parse_elf(&endian).unwrap_err();
        assert!(err.to_string().contains("not little-endian"), "{err}");

        let err = parse_elf(&class[..16]).unwrap_err();
        assert!(err.to_string().contains("truncated ELF header"), "{err}");
    }

    #[test]
    fn elf_rejects_a_segment_and_a_header_that_do_not_fit() {
        let mut file = elf_with(EM_AVR, &[(0, &[0x11, 0x22])], &[]);
        file[52 + 20..52 + 24].copy_from_slice(&1u32.to_le_bytes());
        let err = parse_elf(&file).unwrap_err();
        assert!(err.to_string().contains("p_memsz"), "{err}");

        let mut file = elf_with(EM_AVR, &[(0, &[0x11, 0x22])], &[]);
        file[52 + 4..52 + 8].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        let err = parse_elf(&file).unwrap_err();
        assert!(
            err.to_string().contains("extends past the end of the file"),
            "{err}"
        );

        let header = elf_header(1, 1, 1_000, PHDR32 as u16, 1);
        let err = parse_elf(&header).unwrap_err();
        assert!(
            err.to_string()
                .contains("program header extends past the end"),
            "{err}"
        );
    }

    #[test]
    fn elf_names_a_version_and_a_program_header_table_it_does_not_read() {
        let good = elf_with(EM_AVR, &[(0, &[0x11, 0x22])], &[]);
        let edits: [(usize, &[u8], &str); 4] = [
            (6, &[0], "ELF version 0 is not 1"),
            (42, &[48, 0], "program header size 48 is not 32"),
            (28, &[0; 4], "e_phoff is 0 with e_phnum 1"),
            (
                44,
                &[0xff, 0xff],
                "extended e_phnum (PN_XNUM) is not supported",
            ),
        ];
        for (at, value, text) in edits {
            let mut file = good.clone();
            file[at..at + value.len()].copy_from_slice(value);
            assert_eq!(
                parse_elf(&file).unwrap_err().to_string(),
                format!("ELF: {text}")
            );
        }
    }

    #[test]
    fn hex_rejects_a_missing_eof_an_odd_length_and_a_bad_count() {
        let err = parse_hex(":0100000012ED\n").unwrap_err();
        assert!(
            err.to_string().contains("missing end-of-file record"),
            "{err}"
        );

        // Eleven hex digits: an odd count, and longer than the minimum record.
        let err = parse_hex(":00000001FF0\n").unwrap_err();
        assert!(err.to_string().contains("truncated record"), "{err}");

        // The count byte says 2, so the record should be 7 bytes. This line is 6.
        let err = parse_hex(":0200000000FF\n").unwrap_err();
        assert!(
            err.to_string()
                .contains("length does not match the byte count"),
            "{err}"
        );
    }

    #[test]
    fn elf_wrong_machine() {
        let err = parse_elf(&elf_with(62, &[(0, &[0x12, 0x34])], &[])).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("EM_AVR"), "{text}");
        assert!(text.contains("83"), "{text}");
    }

    #[test]
    fn elf_loads_flash_and_notes_data_and_eeprom() {
        let image = parse_elf(&elf_with(
            EM_AVR,
            &[(0, &[0x11, 0x22])],
            &[(0x0080_0060, &[0xAA]), (0x0081_0000, &[0xBB, 0xCC])],
        ))
        .unwrap();
        assert_eq!(image.bytes(), &[0x11, 0x22]);
        assert_eq!(image.notes().len(), 2);
        assert_eq!(image.notes()[0].region, IgnoredRegion::Data);
        assert_eq!(image.notes()[1].region, IgnoredRegion::Eeprom);
        let note = image.notes()[0].note();
        assert!(note.contains("ignored data segment"), "{note}");
        assert!(note.contains("not placed in flash"), "{note}");
    }

    fn elf_header(class: u8, data: u8, phoff: u32, phentsize: u16, phnum: u16) -> Vec<u8> {
        let mut file = vec![0u8; ELF32_HEADER];
        file[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        file[4] = class;
        file[5] = data;
        file[6] = 1;
        file[16] = 2;
        file[18..20].copy_from_slice(&EM_AVR.to_le_bytes());
        file[20..24].copy_from_slice(&1u32.to_le_bytes());
        file[28..32].copy_from_slice(&phoff.to_le_bytes());
        file[40..42].copy_from_slice(&(ELF32_HEADER as u16).to_le_bytes());
        file[42..44].copy_from_slice(&phentsize.to_le_bytes());
        file[44..46].copy_from_slice(&phnum.to_le_bytes());
        file
    }

    /// `loads` are `(p_paddr, bytes)` placed in flash. `ignored` are noted and not placed.
    fn elf_with(machine: u16, loads: &[(u32, &[u8])], ignored: &[(u32, &[u8])]) -> Vec<u8> {
        let mut segments: Vec<(u32, &[u8])> = Vec::new();
        segments.extend_from_slice(loads);
        segments.extend_from_slice(ignored);
        let phnum = segments.len();
        let phoff = ELF32_HEADER;
        let data_at = phoff + phnum * PHDR32;
        let mut file = vec![0u8; data_at];
        file[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        file[4] = 1;
        file[5] = 1;
        file[6] = 1;
        file[16] = 2; // ET_EXEC
        file[18..20].copy_from_slice(&machine.to_le_bytes());
        file[20..24].copy_from_slice(&1u32.to_le_bytes());
        file[28..32].copy_from_slice(&(phoff as u32).to_le_bytes());
        file[40..42].copy_from_slice(&(ELF32_HEADER as u16).to_le_bytes());
        file[42..44].copy_from_slice(&(PHDR32 as u16).to_le_bytes());
        file[44..46].copy_from_slice(&(phnum as u16).to_le_bytes());
        let mut cursor = data_at;
        for (index, (paddr, data)) in segments.iter().enumerate() {
            let at = phoff + index * PHDR32;
            file[at..at + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
            file[at + 4..at + 8].copy_from_slice(&(cursor as u32).to_le_bytes());
            file[at + 8..at + 12].copy_from_slice(&paddr.to_le_bytes());
            file[at + 12..at + 16].copy_from_slice(&paddr.to_le_bytes());
            let len = data.len() as u32;
            file[at + 16..at + 20].copy_from_slice(&len.to_le_bytes());
            file[at + 20..at + 24].copy_from_slice(&len.to_le_bytes());
            file[at + 24..at + 28].copy_from_slice(&5u32.to_le_bytes());
            file[at + 28..at + 32].copy_from_slice(&1u32.to_le_bytes());
            file.extend_from_slice(data);
            cursor += data.len();
        }
        file
    }
}
