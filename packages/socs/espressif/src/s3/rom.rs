//! The mask ROM, from the ROM ELF Espressif publishes for revision 0, loaded by section.
//!
//! The ROM is 384 KB at `0x4000_0000` on the instruction bus. Its last 128 KB also show at
//! `0x3ff0_0000` on the data bus. A section at an address in either window lands in the ROM at
//! that address. A section the boot ROM copies into SRAM, its `.data`, lands in SRAM. The ROM's
//! program headers place some data twice and some nowhere, so sections are the reliable map.

use std::sync::Arc;

use object::Endianness;
use object::elf::FileHeader32;
use object::read::elf::{FileHeader, SectionHeader};

/// The instruction-bus window.
pub(crate) const ROM: std::ops::Range<u32> = 0x4000_0000..0x4006_0000;
/// The data-bus window and the ROM address of its first byte.
pub(crate) const DROM: std::ops::Range<u32> = 0x3ff0_0000..0x3ff2_0000;
pub(crate) const DROM_AT: u32 = 0x4004_0000;

const SHT_PROGBITS: u32 = 1;

/// The ROM's bytes, and the SRAM bytes its `.data` sections hold, each as an address and its
/// bytes.
pub(crate) struct Image {
    pub(crate) rom: Arc<[u8]>,
    pub(crate) data: Vec<(u32, Vec<u8>)>,
}

/// Reads the ROM ELF, or names what it lacks. The netlist pins the ELF by its SHA-256, so a
/// failure here means the pinned file itself changed shape.
pub(crate) fn load(elf: &[u8], in_sram: impl Fn(u32, usize) -> bool) -> Result<Image, String> {
    if elf.get(..4) != Some(b"\x7fELF".as_slice()) {
        return Err("the ROM is not an ELF file".into());
    }
    // A class, endian, or version this loader does not read still has the ELF magic, so it is
    // the early ending rather than "not an ELF file".
    if elf.len() < 52 || elf[4] != 1 || elf[5] != 1 || elf[6] != 1 {
        return Err("the ROM ELF ends early".into());
    }
    let header =
        FileHeader32::<Endianness>::parse(elf).map_err(|_| "the ROM ELF ends early".to_string())?;
    let endian = header
        .endian()
        .map_err(|_| "the ROM ELF ends early".to_string())?;
    let shoff = u64::from(header.e_shoff(endian));
    let shentsize = header.e_shentsize(endian);
    let shnum = header.e_shnum(endian);
    // `e_shnum` 0 is no table. The extended count in section 0 is not consulted, and offset 0
    // is not a table either. A header size other than 40 does not place bytes.
    let sections = if shnum == 0 || shoff == 0 {
        &[][..]
    } else if shentsize != 40 {
        return Err("the ROM ELF ends early".into());
    } else {
        header
            .section_headers(endian, elf)
            .map_err(|_| "the ROM ELF ends early".to_string())?
    };
    let mut rom = vec![0u8; (ROM.end - ROM.start) as usize];
    let mut data = Vec::new();
    for section in sections {
        if section.sh_type(endian).0 != SHT_PROGBITS {
            continue;
        }
        let size = usize::try_from(u64::from(section.sh_size(endian)))
            .map_err(|_| "a ROM section lies past the file's end".to_string())?;
        if size == 0 {
            continue;
        }
        let offset = usize::try_from(u64::from(section.sh_offset(endian)))
            .map_err(|_| "a ROM section lies past the file's end".to_string())?;
        let bytes = elf
            .get(offset..offset + size)
            .ok_or("a ROM section lies past the file's end")?;
        let addr = u32::try_from(u64::from(section.sh_addr(endian)))
            .map_err(|_| "a ROM section runs past the ROM's end".to_string())?;
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
