//! The external flash and the MMU that maps it into the address space.
//!
//! Flash bytes start from a shared read-only base image, with copy-on-write 4 KB sectors layered
//! on top whenever SPI1 programs or erases flash. Cloning `Flash` for engine checkpoints or rewind
//! copies only `Arc` pointers and the 512-entry MMU table, never the 16 MB image.
//!
//! The MMU's 512 entries, at `0x600c_5000`, each map one 64 KB page of the 32 MB linear space
//! that the instruction bus sees at `0x4200_0000` and the data bus at `0x3c00_0000`. An entry
//! holds a flash page number in its low 14 bits, and bit 14 set marks it invalid. Bit 15 picks
//! PSRAM, which this chip lacks, so such an entry maps nothing. The cache is functional: every
//! access reads the flash through the MMU at once. IDF v5.5.5,
//! `components/soc/esp32s3/include/soc/ext_mem_defs.h`.

use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) const IROM: std::ops::Range<u32> = 0x4200_0000..0x4400_0000;
pub(crate) const DROM: std::ops::Range<u32> = 0x3c00_0000..0x3e00_0000;
pub(crate) const MMU_TABLE: u32 = 0x600c_5000;
pub(crate) const ENTRIES: usize = 512;
pub(crate) const PAGE: u32 = 0x1_0000;
/// 4 KB erase sector and copy-on-write page size.
pub(crate) const SECTOR: usize = 4096;
/// 256-byte page program boundary of the BY25Q128ES flash.
pub(crate) const PAGE_PROGRAM: usize = 256;
pub(crate) const BLOCK_32K: usize = 32 * 1024;
pub(crate) const BLOCK_64K: usize = 64 * 1024;

const INVALID: u32 = 1 << 14;
const PSRAM: u32 = 1 << 15;
const PAGE_NUMBER: u32 = 0x3fff;
/// Number of 4 KB sectors covered by the O(1) presence bitmap (16 MB).
const BITMAP_SECTORS: usize = 4096;

#[derive(Clone, Debug)]
enum Sector {
    /// Every byte in this 4 KB sector is `0xFF`.
    Erased,
    /// A modified 4 KB sector behind `Arc` for copy-on-write across clones.
    Data(Arc<[u8; SECTOR]>),
}

#[derive(Clone, Debug)]
struct Overlay {
    /// Set by chip erase: any sector absent from `sectors` reads `0xFF` instead of `base`.
    erased_base: bool,
    /// Bit `s` is set when sector `s` (`s < BITMAP_SECTORS`) has an entry in `sectors`.
    present: [u64; BITMAP_SECTORS / 64],
    sectors: BTreeMap<u32, Sector>,
    #[cfg(test)]
    copies: u64,
}

impl Overlay {
    fn new(erased_base: bool) -> Overlay {
        Overlay {
            erased_base,
            present: [0; BITMAP_SECTORS / 64],
            sectors: BTreeMap::new(),
            #[cfg(test)]
            copies: 0,
        }
    }

    #[inline]
    fn has_sector(&self, sector: u32) -> bool {
        let index = sector as usize;
        if index < BITMAP_SECTORS {
            (self.present[index / 64] >> (index % 64)) & 1 != 0
        } else {
            self.sectors.contains_key(&sector)
        }
    }

    #[inline]
    fn mark_present(&mut self, sector: u32) {
        let index = sector as usize;
        if index < BITMAP_SECTORS {
            self.present[index / 64] |= 1 << (index % 64);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Flash {
    base: Arc<[u8]>,
    overlay: Option<Arc<Overlay>>,
    mmu: [u32; ENTRIES],
}

impl Flash {
    /// `bytes` as the whole flash, with every MMU entry invalid.
    pub(crate) fn new(bytes: Arc<[u8]>) -> Flash {
        Flash {
            base: bytes,
            overlay: None,
            mmu: [INVALID; ENTRIES],
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn page_copies(&self) -> u64 {
        self.overlay.as_ref().map_or(0, |ov| ov.copies)
    }

    /// Reads `len` bytes from flash offset `start`, or `None` when the range passes the end.
    pub(crate) fn read(&self, start: usize, len: usize) -> Option<Vec<u8>> {
        let end = start.checked_add(len)?;
        if end > self.base.len() {
            return None;
        }
        let Some(ov) = self.overlay.as_deref() else {
            return Some(self.base[start..end].to_vec());
        };
        let mut out = Vec::with_capacity(len);
        let mut at = start;
        while at < end {
            let sector = (at / SECTOR) as u32;
            let off = at % SECTOR;
            let take = (SECTOR - off).min(end - at);
            if ov.has_sector(sector) {
                match ov.sectors.get(&sector).expect("marked sector") {
                    Sector::Erased => out.resize(out.len() + take, 0xff),
                    Sector::Data(page) => out.extend_from_slice(&page[off..off + take]),
                }
            } else if ov.erased_base {
                out.resize(out.len() + take, 0xff);
            } else {
                out.extend_from_slice(&self.base[at..at + take]);
            }
            at += take;
        }
        Some(out)
    }

    /// Whether a page program of `len` bytes at `addr` stays inside the flash after 256-byte page
    /// wrapping.
    pub(crate) fn can_program_page(&self, addr: usize, len: usize) -> bool {
        if len == 0 || addr >= self.base.len() {
            return false;
        }
        let page_base = addr & !(PAGE_PROGRAM - 1);
        if page_base + PAGE_PROGRAM <= self.base.len() {
            return true;
        }
        let page_off = addr & (PAGE_PROGRAM - 1);
        (0..len.min(PAGE_PROGRAM))
            .all(|i| page_base + ((page_off + i) & (PAGE_PROGRAM - 1)) < self.base.len())
    }

    /// Programs `data` at `addr`: clears bits only (`byte &= new`), wrapping inside the 256-byte
    /// page that holds `addr`. Returns `Some(true)` when any flash byte changed, `Some(false)`
    /// when every byte stayed equal, and `None` when out of bounds.
    pub(crate) fn program_page(&mut self, addr: usize, data: &[u8]) -> Option<bool> {
        if !self.can_program_page(addr, data.len()) {
            return None;
        }
        let page_base = addr & !(PAGE_PROGRAM - 1);
        let page_off = addr & (PAGE_PROGRAM - 1);
        let sector = (page_base / SECTOR) as u32;
        let sector_base = page_base % SECTOR;
        let page = self.sector_mut(sector);
        let mut changed = false;
        for (i, &byte) in data.iter().enumerate() {
            let slot = &mut page[sector_base + ((page_off + i) & (PAGE_PROGRAM - 1))];
            let after = *slot & byte;
            if after != *slot {
                *slot = after;
                changed = true;
            }
        }
        Some(changed)
    }

    /// Whether an erase of `size` bytes (a power-of-two multiple of 4 KB) containing `addr` fits
    /// inside the flash.
    pub(crate) fn can_erase_range(&self, addr: usize, size: usize) -> bool {
        debug_assert!(size.is_power_of_two() && size >= SECTOR);
        if addr >= self.base.len() {
            return false;
        }
        let start = addr & !(size - 1);
        start
            .checked_add(size)
            .is_some_and(|end| end <= self.base.len())
    }

    /// Erases the `size`-byte sector or block containing `addr` to `0xFF`. Returns `None` when
    /// out of bounds.
    pub(crate) fn erase_range(&mut self, addr: usize, size: usize) -> Option<bool> {
        if !self.can_erase_range(addr, size) {
            return None;
        }
        let start = addr & !(size - 1);
        let first = (start / SECTOR) as u32;
        let count = (size / SECTOR) as u32;
        let ov = self.overlay_mut();
        for sector in first..first + count {
            ov.sectors.insert(sector, Sector::Erased);
            ov.mark_present(sector);
        }
        Some(true)
    }

    /// Erases the whole flash to `0xFF`. Returns `None` on an empty flash.
    pub(crate) fn erase_chip(&mut self) -> Option<bool> {
        if self.base.is_empty() {
            return None;
        }
        self.overlay = Some(Arc::new(Overlay::new(true)));
        Some(true)
    }

    fn overlay_mut(&mut self) -> &mut Overlay {
        let ov = self
            .overlay
            .get_or_insert_with(|| Arc::new(Overlay::new(false)));
        Arc::make_mut(ov)
    }

    fn sector_mut(&mut self, sector: u32) -> &mut [u8; SECTOR] {
        let base = &self.base;
        let ov = self
            .overlay
            .get_or_insert_with(|| Arc::new(Overlay::new(false)));
        let ov = Arc::make_mut(ov);
        ov.mark_present(sector);
        let erased_base = ov.erased_base;
        let entry = ov.sectors.entry(sector).or_insert_with(|| {
            if erased_base {
                Sector::Erased
            } else {
                let mut buf = [0xff; SECTOR];
                let start = sector as usize * SECTOR;
                let end = (start + SECTOR).min(base.len());
                if start < end {
                    buf[..end - start].copy_from_slice(&base[start..end]);
                }
                Sector::Data(Arc::new(buf))
            }
        });
        match entry {
            Sector::Erased => {
                *entry = Sector::Data(Arc::new([0xff; SECTOR]));
                match entry {
                    Sector::Data(arc) => Arc::make_mut(arc),
                    Sector::Erased => unreachable!(),
                }
            }
            Sector::Data(arc) => {
                #[cfg(test)]
                if Arc::get_mut(arc).is_none() {
                    ov.copies += 1;
                }
                Arc::make_mut(arc)
            }
        }
    }

    /// The flash offset the bus address `addr` reaches, inside one of the two windows.
    fn translate(&self, addr: u32) -> Option<usize> {
        let entry = self.mmu[((addr >> 16) & (ENTRIES as u32 - 1)) as usize];
        if entry & (INVALID | PSRAM) != 0 {
            return None;
        }
        let offset = ((entry & PAGE_NUMBER) << 16 | (addr & (PAGE - 1))) as usize;
        (offset + 4 <= self.base.len()).then_some(offset)
    }

    /// The little-endian word at `addr`, a multiple of 4 in a window, through the MMU.
    pub(crate) fn word(&self, addr: u32) -> Option<u32> {
        let at = self.translate(addr)?;
        if let Some(ov) = self.overlay.as_deref() {
            let sector = (at / SECTOR) as u32;
            if ov.has_sector(sector) {
                return Some(match ov.sectors.get(&sector).expect("marked sector") {
                    Sector::Erased => 0xffff_ffff,
                    Sector::Data(page) => {
                        let b = &page[at % SECTOR..at % SECTOR + 4];
                        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                    }
                });
            }
            if ov.erased_base {
                return Some(0xffff_ffff);
            }
        }
        let b = &self.base[at..at + 4];
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

    #[test]
    fn program_clears_bits_wraps_within_256_bytes_and_erase_sets_ff() {
        let mut flash = Flash::new(vec![0xff; BLOCK_64K * 2].into());
        flash.map(0x3c00_0000, 0, BLOCK_64K as u32);
        assert_eq!(
            flash.program_page(0x00fc, &[0xaa, 0x55, 0xf0, 0x0f, 0x12, 0x34]),
            Some(true)
        );
        assert_eq!(flash.read(0x00fc, 4), Some(vec![0xaa, 0x55, 0xf0, 0x0f]));
        assert_eq!(
            flash.read(0x0000, 2),
            Some(vec![0x12, 0x34]),
            "wrapped to the start of the 256-byte page"
        );
        assert_eq!(
            flash.read(0x0100, 2),
            Some(vec![0xff, 0xff]),
            "next page untouched"
        );
        // Programming again only clears bits (1 -> 0).
        assert_eq!(flash.program_page(0x00fc, &[0xff, 0x0f]), Some(true));
        assert_eq!(flash.read(0x00fc, 2), Some(vec![0xaa, 0x05]));
        assert_eq!(flash.word(0x3c00_00fc), Some(0x0ff0_05aa));

        // Sector erase aligns down to 4 KB.
        assert_eq!(flash.erase_range(0x00fc, SECTOR), Some(true));
        assert_eq!(flash.read(0x0000, 2), Some(vec![0xff, 0xff]));
        assert_eq!(flash.word(0x3c00_00fc), Some(0xffff_ffff));

        // 32 KB block erase and 64 KB block erase.
        flash.program_page(0x0000, &[0x11]).unwrap();
        flash.program_page(0x7000, &[0x22]).unwrap();
        flash.program_page(0x8000, &[0x33]).unwrap();
        flash.erase_range(0x5432, BLOCK_32K).unwrap();
        assert_eq!(flash.read(0x0000, 1), Some(vec![0xff]));
        assert_eq!(flash.read(0x7000, 1), Some(vec![0xff]));
        assert_eq!(flash.read(0x8000, 1), Some(vec![0x33]));

        flash.erase_range(0xf123, BLOCK_64K).unwrap();
        assert_eq!(flash.read(0x8000, 1), Some(vec![0xff]));

        // Chip erase.
        flash.program_page(BLOCK_64K + 16, &[0x42]).unwrap();
        flash.erase_chip().unwrap();
        assert_eq!(flash.read(BLOCK_64K + 16, 1), Some(vec![0xff]));
        flash.program_page(BLOCK_64K + 16, &[0x77]).unwrap();
        assert_eq!(flash.read(BLOCK_64K + 16, 1), Some(vec![0x77]));
    }

    #[test]
    fn cloning_flash_copies_only_written_sectors_on_write() {
        let mut flash = Flash::new(vec![0xff; 16 << 20].into());
        flash.program_page(0x9000, &[0x11, 0x22]).unwrap();
        flash.program_page(0xa000, &[0x33, 0x44]).unwrap();
        assert_eq!(flash.page_copies(), 0);

        let checkpoint = flash.clone();
        // Writing to sector 9 after a clone copies only sector 9, not sector 10 or 16 MB.
        flash.program_page(0x9002, &[0x55]).unwrap();
        assert_eq!(flash.page_copies(), 1);
        assert_eq!(checkpoint.read(0x9000, 3), Some(vec![0x11, 0x22, 0xff]));
        assert_eq!(flash.read(0x9000, 3), Some(vec![0x11, 0x22, 0x55]));

        // Rewinding restores checkpoint bytes without copying the 16 MB image.
        flash.clone_from(&checkpoint);
        assert_eq!(flash.read(0x9000, 3), Some(vec![0x11, 0x22, 0xff]));
        assert_eq!(flash.read(0xa000, 2), Some(vec![0x33, 0x44]));
    }
}
