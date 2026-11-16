//! The mask ROM, from the ROM ELF Espressif publishes for revision 0, loaded by section.
//!
//! The ROM is 384 KB at `0x4000_0000` on the instruction bus. Its last 128 KB also show at
//! `0x3ff0_0000` on the data bus. A section at an address in either window lands in the ROM at
//! that address. A section the boot ROM copies into SRAM, its `.data`, lands in SRAM. The ROM's
//! program headers place some data twice and some nowhere, so sections are the reliable map.

use std::sync::Arc;

use object::LittleEndian as Le;
use object::elf::{FileHeader32, SHT_PROGBITS};
use object::read::elf::{FileHeader, SectionHeader};

/// The instruction-bus window.
pub(crate) const ROM: std::ops::Range<u32> = 0x4000_0000..0x4006_0000;
/// The data-bus window and the ROM address of its first byte.
pub(crate) const DROM: std::ops::Range<u32> = 0x3ff0_0000..0x3ff2_0000;
pub(crate) const DROM_AT: u32 = 0x4004_0000;

/// The ROM's bytes, and the SRAM bytes its `.data` sections hold, each as an address and its
/// bytes.
pub(crate) struct Image {
    pub(crate) rom: Arc<[u8]>,
    pub(crate) data: Vec<(u32, Vec<u8>)>,
}

/// Reads the ROM ELF, or names what it lacks. The netlist pins the ELF by its SHA-256, so a
/// failure here means the pinned file itself changed shape. `e_shnum` 0 is no section table. A
/// table at offset 0 or of headers other than 40 bytes is refused.
pub(crate) fn load(elf: &[u8], in_sram: impl Fn(u32, usize) -> bool) -> Result<Image, String> {
    if elf.get(..4) != Some(b"\x7fELF".as_slice()) {
        return Err("the ROM is not an ELF file".into());
    }
    let header = FileHeader32::<Le>::parse(elf)
        .ok()
        .filter(|header| header.endian().is_ok())
        .ok_or(if elf.len() < 52 {
            "the ROM ELF ends early"
        } else {
            "the ROM is not a 32-bit little-endian ELF version 1"
        })?;
    let sections = match (
        header.e_shoff(Le),
        header.e_shnum(Le),
        header.e_shentsize(Le),
    ) {
        (_, 0, _) => &[][..],
        (0, _, _) => return Err("the ROM's section table is at offset 0".into()),
        (_, _, 40) => header
            .section_headers(Le, elf)
            .map_err(|_| "the ROM ELF ends early")?,
        (_, _, size) => {
            return Err(format!(
                "the ROM's section headers are {size} bytes, not 40"
            ));
        }
    };
    let mut rom = vec![0u8; (ROM.end - ROM.start) as usize];
    let mut data = Vec::new();
    for section in sections {
        let (addr, size) = (section.sh_addr(Le), section.sh_size(Le) as usize);
        if section.sh_type(Le) != SHT_PROGBITS || size == 0 {
            continue;
        }
        let bytes = section
            .data(Le, elf)
            .map_err(|_| "a ROM section lies past the file's end")?;
        let at = if ROM.contains(&addr) {
            Some(addr)
        } else if DROM.contains(&addr) {
            Some(addr - DROM.start + DROM_AT)
        } else {
            None
        };
        match at {
            Some(at) => {
                let start = (at - ROM.start) as usize;
                rom.get_mut(start..start + size)
                    .ok_or("a ROM section runs past the ROM's end")?
                    .copy_from_slice(bytes);
            }
            None if in_sram(addr, size) => data.push((addr, bytes.to_vec())),
            None => {}
        }
    }
    Ok(Image {
        rom: rom.into(),
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM ELF with one `PROGBITS` section of `bytes` at `addr`.
    fn rom_elf(addr: u32, bytes: &[u8]) -> Vec<u8> {
        let mut file = vec![0u8; 52];
        file[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
        file.extend(bytes);
        let shoff = file.len() as u32;
        file[32..36].copy_from_slice(&shoff.to_le_bytes());
        file[46..48].copy_from_slice(&40u16.to_le_bytes());
        file[48..50].copy_from_slice(&2u16.to_le_bytes());
        file.extend([0; 40]);
        let mut section = [0u8; 40];
        for (at, word) in [
            (4, SHT_PROGBITS.0),
            (12, addr),
            (16, 52),
            (20, bytes.len() as u32),
        ] {
            section[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        file.extend(section);
        file
    }

    fn refusal(file: &[u8]) -> String {
        match load(file, |_, _| false) {
            Ok(_) => panic!("the ROM loads"),
            Err(text) => text,
        }
    }

    #[test]
    fn a_section_lands_in_the_rom_at_its_address() {
        let Ok(image) = load(&rom_elf(DROM.start + 4, &[1, 2, 3]), |_, _| false) else {
            panic!("the ROM loads");
        };
        let at = (DROM_AT - ROM.start) as usize + 4;
        assert_eq!(&image.rom[at..at + 3], &[1, 2, 3]);
    }

    #[test]
    fn a_header_or_table_the_loader_does_not_read_is_named() {
        let good = rom_elf(ROM.start, &[1]);
        let edits: [(usize, &[u8], &str); 5] = [
            (
                4,
                &[2],
                "the ROM is not a 32-bit little-endian ELF version 1",
            ),
            (
                5,
                &[2],
                "the ROM is not a 32-bit little-endian ELF version 1",
            ),
            (
                6,
                &[0],
                "the ROM is not a 32-bit little-endian ELF version 1",
            ),
            (32, &[0; 4], "the ROM's section table is at offset 0"),
            (
                46,
                &[64, 0],
                "the ROM's section headers are 64 bytes, not 40",
            ),
        ];
        for (at, value, text) in edits {
            let mut file = good.clone();
            file[at..at + value.len()].copy_from_slice(value);
            assert_eq!(refusal(&file), text);
        }
        assert_eq!(refusal(&good[..good.len() - 20]), "the ROM ELF ends early");
        assert_eq!(refusal(&good[..40]), "the ROM ELF ends early");
        assert_eq!(refusal(b"MZ"), "the ROM is not an ELF file");
    }
}
