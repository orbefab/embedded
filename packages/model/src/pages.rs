/// A large memory in `PAGES` pages of `SIZE` bytes, as plain data, that a copy copies by the
/// page. Each page keeps the generation of its last write, so [`copy_from`](Self::copy_from)
/// copies only the pages whose generations differ.
///
/// Equal generations mean equal pages between a memory and a copy made before or after it,
/// which is how a host copies a state: it runs a copy ahead of its checkpoint and copies one
/// over the other. Two copies that both wrote since they split can share a generation with
/// different bytes, so a host never copies one over the other.
#[derive(Clone, Copy, Debug)]
pub struct Pages<const PAGES: usize, const SIZE: usize> {
    bytes: [[u8; SIZE]; PAGES],
    generations: [u64; PAGES],
    /// The newest generation of any page.
    newest: u64,
}

impl<const PAGES: usize, const SIZE: usize> Pages<PAGES, SIZE> {
    /// Every byte zero.
    pub const fn new() -> Self {
        Pages {
            bytes: [[0; SIZE]; PAGES],
            generations: [0; PAGES],
            newest: 0,
        }
    }

    /// Every byte, page after page.
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_flattened()
    }

    /// Page `page`, to write.
    #[inline]
    pub fn page_mut(&mut self, page: usize) -> &mut [u8; SIZE] {
        self.newest += 1;
        self.generations[page] = self.newest;
        &mut self.bytes[page]
    }

    /// Becomes `source`, a copy made before or after it. Returns how many pages it copied.
    pub fn copy_from(&mut self, source: &Self) -> usize {
        let mut copied = 0;
        for page in 0..PAGES {
            if self.generations[page] != source.generations[page] {
                self.bytes[page] = source.bytes[page];
                self.generations[page] = source.generations[page];
                copied += 1;
            }
        }
        self.newest = source.newest;
        copied
    }
}

impl<const PAGES: usize, const SIZE: usize> Default for Pages<PAGES, SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_copies_only_the_pages_either_side_wrote() {
        let mut kept = Pages::<4, 2>::new();
        kept.page_mut(0)[0] = 1;
        let mut ahead = kept;
        ahead.page_mut(2)[1] = 7;
        ahead.page_mut(2)[0] = 5;
        assert_eq!(kept.copy_from(&ahead), 1, "the page ahead wrote");
        assert_eq!(kept.bytes(), [1, 0, 0, 0, 5, 7, 0, 0]);
        assert_eq!(ahead.copy_from(&kept), 0, "a copy of a twin");
        ahead.page_mut(3)[0] = 9;
        assert_eq!(ahead.copy_from(&kept), 1, "back to the checkpoint");
        assert_eq!(ahead.bytes(), kept.bytes());
        ahead.page_mut(1)[0] = 4;
        assert_eq!(kept.copy_from(&ahead), 1, "a write after going back");
        assert_eq!(kept.bytes(), [1, 0, 4, 0, 5, 7, 0, 0]);
    }
}
