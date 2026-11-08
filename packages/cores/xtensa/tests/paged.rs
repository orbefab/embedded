//! A SoC-shaped copy of a core and its RAM, as a landing makes one.

use orbe_xtensa::{Config, Core, Cycle, Esp32s3, PAGE, Stop, Width};

use crate::core::{CODE, Flat, core_at};

/// Stores `a4` at `a2`, moves `a2` up one page, and loops `a5` times.
const DIRTY_PAGES: &str = "426200\
                           22d210\
                           52c5ff\
                           5635ff";

#[derive(Clone)]
struct Soc {
    core: Core<Esp32s3>,
    bus: Flat,
}

impl Soc {
    /// Runs the program from the RAM base, writing `value` into `pages` pages.
    fn dirty(&mut self, pages: u32, value: u32) {
        let base = self.bus.ram.base();
        self.core.set_pc(CODE);
        self.core.set_a(2, base);
        self.core.set_a(4, value);
        self.core.set_a(5, pages);
        let limit = Cycle(self.core.cycle().0 + 10_000);
        assert_eq!(self.core.run_until(&mut self.bus, limit), Stop::Bus);
        assert_eq!(self.core.pc(), CODE + 12);
    }

    fn word(&self, page: u32) -> u32 {
        let ram = &self.bus.ram;
        ram.read(ram.base() + page * PAGE as u32, Width::Word)
    }
}

#[test]
fn the_program_is_the_one_described() {
    let mut text = Vec::new();
    let mut pc = CODE;
    let bytes: Vec<u8> = (0..DIRTY_PAGES.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&DIRTY_PAGES[i..i + 2], 16).unwrap())
        .collect();
    let mut off = 0;
    while off < bytes.len() {
        let decoded = Esp32s3::decode(&bytes[off..], pc).unwrap();
        text.push(Esp32s3::render(&decoded));
        off += decoded.size() as usize;
        pc += u32::from(decoded.size());
    }
    assert_eq!(
        text,
        [
            "s32i\ta4, a2, 0",
            "addmi\ta2, a2, 0x1000",
            "addi\ta5, a5, -1",
            "bnez\ta5, 0x40370000",
        ]
    );
}

/// The ahead copy writes `N` pages, the committed copy takes it with `clone_from`, and the ahead
/// copy writes the same pages again. Each round copies `N` pages, and the committed copy keeps
/// the first round's bytes.
#[test]
fn a_copy_costs_only_its_dirty_pages() {
    const N: u32 = 5;
    let mut committed = Soc {
        core: core_at(CODE),
        bus: Flat::new(DIRTY_PAGES),
    };
    let total = committed.bus.ram.len() / PAGE;
    assert_eq!(total, 128);

    let mut ahead = committed.clone();
    ahead.dirty(N, 0x11);
    assert_eq!(ahead.bus.ram.page_copies(), u64::from(N));

    committed.clone_from(&ahead);
    assert_eq!(committed.core.pc(), ahead.core.pc());
    assert_eq!(committed.bus.ram.page_copies(), u64::from(N));

    ahead.dirty(N, 0x22);
    assert_eq!(ahead.bus.ram.page_copies(), u64::from(2 * N));
    for page in 0..N {
        assert_eq!(committed.word(page), 0x11);
        assert_eq!(ahead.word(page), 0x22);
    }
    assert_eq!(committed.word(N), 0);
}
