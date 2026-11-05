//! The external flash and the MMU that maps it into the address space.
//!
//! The flash is read-only here. The MMU's 512 entries, at `0x600c_5000`, each map one 64 KB
//! page of the 32 MB linear space that the instruction bus sees at `0x4200_0000` and the data
//! bus at `0x3c00_0000`. An entry holds a flash page number in its low 14 bits, and bit 14 set
//! marks it invalid. Bit 15 picks PSRAM, which this chip lacks, so such an entry maps nothing.
//! The cache is functional: every access reads the flash through the MMU at once. IDF v5.5.5,
//! `components/soc/esp32s3/include/soc/ext_mem_defs.h`.

use std::sync::Arc;

pub(crate) const IROM: std::ops::Range<u32> = 0x4200_0000..0x4400_0000;
pub(crate) const DROM: std::ops::Range<u32> = 0x3c00_0000..0x3e00_0000;
pub(crate) const MMU_TABLE: u32 = 0x600c_5000;
pub(crate) const ENTRIES: usize = 512;
pub(crate) const PAGE: u32 = 0x1_0000;
const INVALID: u32 = 1 << 14;
const PSRAM: u32 = 1 << 15;
const PAGE_NUMBER: u32 = 0x3fff;

#[derive(Clone, Debug)]
pub(crate) struct Flash {
    bytes: Arc<[u8]>,
    mmu: [u32; ENTRIES],
}

impl Flash {
    /// `bytes` as the whole flash, with every MMU entry invalid.
    pub(crate) fn new(bytes: Arc<[u8]>) -> Flash {
        Flash {
            bytes,
            mmu: [INVALID; ENTRIES],
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The flash offset the bus address `addr` reaches, inside one of the two windows.
    fn translate(&self, addr: u32) -> Option<usize> {
        let entry = self.mmu[((addr >> 16) & (ENTRIES as u32 - 1)) as usize];
        if entry & (INVALID | PSRAM) != 0 {
            return None;
        }
        let offset = ((entry & PAGE_NUMBER) << 16 | (addr & (PAGE - 1))) as usize;
        (offset + 4 <= self.bytes.len()).then_some(offset)
    }

    /// The little-endian word at `addr`, a multiple of 4 in a window, through the MMU.
    pub(crate) fn word(&self, addr: u32) -> Option<u32> {
        let at = self.translate(addr)?;
        let b = &self.bytes[at..at + 4];
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// The MMU table's word at `offset` from its base.
    pub(crate) fn read_mmu(&self, offset: u32) -> Option<u32> {
        self.mmu.get((offset / 4) as usize).copied()
    }

    pub(crate) fn write_mmu(&mut self, offset: u32, value: u32) -> bool {
        match self.mmu.get_mut((offset / 4) as usize) {
            Some(entry) => {
                *entry = value & (INVALID | PSRAM | PAGE_NUMBER);
                true
            }
            None => false,
        }
    }

    /// Maps the pages under `size` bytes from bus address `vaddr` to flash from `offset`, as
    /// the bootloader maps an app's DROM or IROM segment: both rounded down to a page.
    pub(crate) fn map(&mut self, vaddr: u32, offset: u32, size: u32) {
        let pages = (size + (vaddr & (PAGE - 1))).div_ceil(PAGE);
        let first = ((vaddr >> 16) & (ENTRIES as u32 - 1)) as usize;
        for k in 0..pages as usize {
            self.mmu[(first + k) % ENTRIES] = (offset >> 16) + k as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mapped_page_reads_through_either_window_and_an_unmapped_one_reads_nothing() {
        let mut bytes = vec![0u8; 0x3_0000];
        bytes[0x2_0010..0x2_0014].copy_from_slice(&[1, 2, 3, 4]);
        let mut flash = Flash::new(bytes.into());
        assert_eq!(flash.word(0x4201_0010), None, "every entry starts invalid");
        flash.map(0x4201_0020, 0x2_0020, 8);
        assert_eq!(flash.read_mmu(4), Some(2), "page 1 maps flash page 2");
        assert_eq!(flash.word(0x4201_0010), Some(0x0403_0201));
        assert_eq!(
            flash.word(0x3c01_0010),
            Some(0x0403_0201),
            "one table for both windows"
        );
        flash.write_mmu(4, INVALID | 2);
        assert_eq!(flash.word(0x4201_0010), None);
        flash.write_mmu(4, PSRAM | 2);
        assert_eq!(
            flash.word(0x4201_0010),
            None,
            "PSRAM, which this chip lacks"
        );
        flash.write_mmu(4, 3);
        assert_eq!(flash.word(0x4201_0010), None, "a page past the flash's end");
    }
}
