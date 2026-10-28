//! RAM in 4 KB pages behind `Arc`, copied on write.
//!
//! A landing copies each committed core with `clone_from`. A SoC that keeps its RAM here copies
//! one pointer per page there, and a page's bytes only when one of two copies first writes it
//! after that. So keeping a committed copy costs the pages written since, not the whole RAM.

use std::sync::Arc;

use crate::bus::Width;

pub const PAGE: usize = 4096;

type Page = [u8; PAGE];

#[derive(Clone, Debug)]
pub struct PagedRam {
    base: u32,
    pages: Vec<Arc<Page>>,
    copies: u64,
}

impl PagedRam {
    /// `len` zero bytes from `base`. Both are multiples of [`PAGE`]. Every page starts as the
    /// same shared zero page.
    pub fn new(base: u32, len: usize) -> Self {
        assert!(
            (base as usize).is_multiple_of(PAGE) && len.is_multiple_of(PAGE),
            "RAM base {base:#x} and length {len:#x} are whole pages"
        );
        let zero = Arc::new([0u8; PAGE]);
        PagedRam {
            base,
            pages: vec![zero; len / PAGE],
            copies: 0,
        }
    }

    pub fn base(&self) -> u32 {
        self.base
    }

    pub fn len(&self) -> usize {
        self.pages.len() * PAGE
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    pub fn contains(&self, addr: u32) -> bool {
        addr.wrapping_sub(self.base) < self.len() as u32
    }

    /// Pages this RAM and the copies it was cloned from have copied on a write since
    /// [`PagedRam::new`].
    pub fn page_copies(&self) -> u64 {
        self.copies
    }

    /// Reads at `addr`, a multiple of `width`'s size inside this RAM, little-endian.
    pub fn read(&self, addr: u32, width: Width) -> u32 {
        let (page, at) = self.locate(addr, width);
        let bytes = &self.pages[page][at..at + width.bytes() as usize];
        bytes
            .iter()
            .rev()
            .fold(0, |value, byte| (value << 8) | u32::from(*byte))
    }

    pub fn write(&mut self, addr: u32, width: Width, value: u32) {
        let (page, at) = self.locate(addr, width);
        let bytes = self.page_mut(page);
        let len = width.bytes() as usize;
        bytes[at..at + len].copy_from_slice(&value.to_le_bytes()[..len]);
    }

    /// Copies `data` in from `addr`, for loading an image. The range lies inside this RAM.
    pub fn write_bytes(&mut self, addr: u32, data: &[u8]) {
        let offset = addr.wrapping_sub(self.base) as usize;
        assert!(
            self.contains(addr) && offset + data.len() <= self.len(),
            "{} bytes at {addr:#x} fit in RAM at {:#x}",
            data.len(),
            self.base
        );
        let mut done = 0;
        while done < data.len() {
            let at = offset + done;
            let page = self.page_mut(at / PAGE);
            let start = at % PAGE;
            let take = (PAGE - start).min(data.len() - done);
            page[start..start + take].copy_from_slice(&data[done..done + take]);
            done += take;
        }
    }

    fn locate(&self, addr: u32, width: Width) -> (usize, usize) {
        let offset = addr.wrapping_sub(self.base);
        debug_assert!(
            self.contains(addr) && offset.is_multiple_of(width.bytes()),
            "{width:?} access at {addr:#x} is aligned and inside RAM at {:#x}",
            self.base
        );
        let offset = offset as usize;
        (offset / PAGE, offset % PAGE)
    }

    fn page_mut(&mut self, page: usize) -> &mut Page {
        let slot = &mut self.pages[page];
        if Arc::get_mut(slot).is_none() {
            self.copies += 1;
        }
        Arc::make_mut(slot)
    }
}
