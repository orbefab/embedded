//! The mask ROM, from the ROM ELF Espressif publishes for revision 0, loaded by section.
//!
//! The ROM is 384 KB at `0x4000_0000` on the instruction bus. Its last 128 KB also show at
//! `0x3ff0_0000` on the data bus. A section at an address in either window lands in the ROM at
//! that address. A section the boot ROM copies into SRAM, its `.data`, lands in SRAM. The ROM's
//! program headers place some data twice and some nowhere, so sections are the reliable map.

use std::sync::Arc;

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
    let u16_at = |at: usize| -> Result<u16, String> {
        let bytes = elf.get(at..at + 2).ok_or("the ROM ELF ends early")?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    };
    let u32_at = |at: usize| -> Result<u32, String> {
        let bytes = elf.get(at..at + 4).ok_or("the ROM ELF ends early")?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    if elf.get(..4) != Some(b"\x7fELF".as_slice()) {
        return Err("the ROM is not an ELF file".into());
    }
    let shoff = u32_at(32)? as usize;
    let (shentsize, shnum) = (usize::from(u16_at(46)?), usize::from(u16_at(48)?));
    let mut rom = vec![0u8; (ROM.end - ROM.start) as usize];
    let mut data = Vec::new();
    for k in 0..shnum {
        let header = shoff + k * shentsize;
        let (kind, addr) = (u32_at(header + 4)?, u32_at(header + 12)?);
        let (offset, size) = (u32_at(header + 16)? as usize, u32_at(header + 20)? as usize);
        if kind != SHT_PROGBITS || size == 0 {
            continue;
        }
        let bytes = elf
            .get(offset..offset + size)
            .ok_or("a ROM section lies past the file's end")?;
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
