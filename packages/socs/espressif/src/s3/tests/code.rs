//! The decode tables the cores keep, which run an instruction again without a fetch while the
//! chip's code version holds. Each test changes code a core has already run, in one of the ways
//! the version watches, and checks that the core runs what memory holds now. A table that kept
//! the old decode would run the old op, and each test names the value that op leaves.

use orbe_xtensa::Config;

use super::dual::{boot_rom, chip as dual_chip};
use super::*;

/// SRAM from `0x4037_8000` on the instruction bus, which the data bus sees at `0x3fc8_8000`.
const IRAM: u32 = 0x4037_8000;
const DRAM: u32 = 0x3fc8_8000;
const RTC_FAST: u32 = 0x600f_e000;
const MMU_TABLE: u32 = 0x600c_5000;
const IROM: u32 = 0x4200_0000;

/// `movi.n a5, n` and `nop.n`, one word.
fn movi_n_a5(n: u8) -> [u8; 4] {
    [0x0c, n << 4 | 5, 0x3d, 0xf0]
}

/// `movi.n a7, 7` and `nop.n`, one word.
const A7_IS_7: [u8; 4] = [0x0c, 0x77, 0x3d, 0xf0];
/// `nop.n` twice.
const NOPS: [u8; 4] = [0x3d, 0xf0, 0x3d, 0xf0];

fn says(bytes: &[u8], text: &str) {
    let decoded = orbe_xtensa::Esp32s3::decode(bytes, IRAM).expect("bytes");
    assert_eq!(orbe_xtensa::Esp32s3::render(&decoded), text);
}

#[test]
fn the_words_say_what_the_tests_name() {
    says(&movi_n_a5(1)[..2], "movi.n\ta5, 1");
    says(&movi_n_a5(2)[2..], "nop.n");
    says(&A7_IS_7[..2], "movi.n\ta7, 7");
    says(&[0xc0, 0x04, 0x00], "callx0\ta4");
    says(&[0x0d, 0xf0], "ret.n");
}

impl Program {
    /// A `j` to the next word boundary, then nothing up to it, so the next op starts a word.
    fn align(&mut self) -> &mut Program {
        if self.pc().is_multiple_of(4) {
            return self;
        }
        let to = (self.pc() + 3).next_multiple_of(4);
        self.j(to);
        while self.pc() < to {
            self.raw(&[0]);
        }
        self
    }
}

/// From `base`: runs the word `movi.n a5, 1; nop.n`, stores the word `movi.n a5, 2; nop.n` over
/// it at `store_at(word)`, runs it again and spins. `a5` ends at 2, and at 1 if the second pass
/// ran the decode of the first.
fn rewrites_itself(base: u32, store_at: impl Fn(u32) -> u32) -> Program {
    let mut program = Program::at(base, &[0, u32::from_le_bytes(movi_n_a5(2))]);
    program.l32r(2, 0).l32r(3, 1).movi(6, 0).align();
    let site = program.pc();
    program.literals[0] = store_at(site);
    let done = site + 4 + 12;
    program
        .raw(&movi_n_a5(1))
        .bnez(6, done)
        .movi(6, 1)
        .s32i(3, 2, 0)
        .j(site);
    assert_eq!(program.pc(), done);
    program.raw(&LOOP);
    program
}

fn a5_after(chip: &mut Esp32s3, through: u64) -> u32 {
    while chip.run_until(c(through)).is_some() {}
    chip.cores.core(CoreId::Pro).a(5)
}

#[test]
fn a_store_through_the_data_bus_over_a_run_op_runs_the_new_op() {
    let program = rewrites_itself(IRAM, |site| site - IRAM + DRAM);
    let mut chip = Esp32s3::new(&[], program.entry(), [(IRAM, &program.image()[..])], None);
    assert_eq!(a5_after(&mut chip, 200), 2);
}

#[test]
fn a_store_through_the_instruction_bus_over_a_run_op_runs_the_new_op() {
    let program = rewrites_itself(IRAM, |site| site);
    let mut chip = Esp32s3::new(&[], program.entry(), [(IRAM, &program.image()[..])], None);
    assert_eq!(a5_after(&mut chip, 200), 2);
}

#[test]
fn a_store_over_a_run_op_in_rtc_fast_memory_runs_the_new_op() {
    let program = rewrites_itself(RTC_FAST, |site| site);
    let no_segments: [(u32, &[u8]); 0] = [];
    let mut chip = Esp32s3::new(&[], program.entry(), no_segments, None);
    chip.soc.poke(RTC_FAST, &program.image());
    assert_eq!(a5_after(&mut chip, 200), 2);
}

/// A store to the MMU table moves the instruction-bus page under a PC the core has run. Flash
/// page 0 holds `movi.n a5, 1; ret.n` and page 1 holds `movi.n a5, 2; ret.n`. The code calls
/// `0x4200_0000` through page 0, maps page 1 there, and calls it again.
#[test]
fn a_remap_of_the_flash_under_a_run_op_runs_the_new_page() {
    let mut program = Program::new(&[MMU_TABLE, IROM]);
    let callx0_a4 = [0xc0, 0x04, 0x00];
    program
        .l32r(2, 0)
        .l32r(4, 1)
        .raw(&callx0_a4)
        .movi(3, 1)
        .s32i(3, 2, 0)
        .raw(&callx0_a4)
        .raw(&LOOP);
    let mut bytes = vec![0u8; 0x2_0000];
    bytes[..4].copy_from_slice(&[0x0c, 0x15, 0x0d, 0xf0]);
    bytes[0x1_0000..0x1_0004].copy_from_slice(&[0x0c, 0x25, 0x0d, 0xf0]);
    let mut flash = Flash::new(bytes.into());
    flash.write_mmu(0, 0);
    let mut chip = program.chip_on(&[]);
    chip.soc.attach_flash(flash);
    assert_eq!(a5_after(&mut chip, 200), 2);
    assert_eq!(chip.cores.core(CoreId::Pro).pc(), program.pc() - 3);
}

/// The PRO CPU starts the APP CPU and spins on two `nop.n` until `a7` is set. The APP CPU waits,
/// then stores `movi.n a7, 7; nop.n` over them through the data bus. The PRO CPU runs it on its
/// next pass and leaves the loop.
#[test]
fn a_store_by_the_other_core_over_a_run_op_runs_the_new_op() {
    let app_at = IRAM + 0x1000;
    let mut app = Program::at(app_at, &[0, u32::from_le_bytes(A7_IS_7)]);
    let mut pro = Program::at(IRAM, &[SYSTEM, app.entry()]);
    pro.l32r(2, 0).l32r(4, 1).release().movi(7, 0).align();
    let site = pro.pc();
    pro.raw(&NOPS).beqz(7, site).raw(&LOOP);
    app.literals[0] = site - IRAM + DRAM;
    app.l32r(2, 0)
        .l32r(3, 1)
        .delay(4, 40)
        .s32i(3, 2, 0)
        .raw(&LOOP);
    let mut chip = dual_chip(&pro, &boot_rom(), &[&app]);
    while chip.run_until(c(2000)).is_some() {}
    let core = chip.cores.core(CoreId::Pro);
    assert_eq!((core.a(7), core.pc()), (7, site + 7));
}

/// A rewind puts back the code a copy held, and the core runs that code, not what it decoded
/// after the copy was taken. The loop runs the word at `BASE`, `movi.n a5, n; nop.n`, and jumps
/// back to it.
#[test]
fn a_rewound_chip_runs_the_code_it_holds_and_not_what_it_ran_ahead() {
    let mut program = Program::new(&[]);
    program.raw(&movi_n_a5(1)).j(BASE);
    let mut chip = program.chip_on(&[]);
    assert_eq!(a5_after(&mut chip, 50), 1);
    let copy = chip.clone();
    chip.soc.poke(BASE, &movi_n_a5(2));
    assert_eq!(a5_after(&mut chip, 100), 2);
    chip.clone_from(&copy);
    chip.soc.poke(BASE, &movi_n_a5(3));
    assert_eq!(a5_after(&mut chip, 100), 3);
}

/// A loop raises and lowers GPIO21. Its first pass decodes every op, and each later pass runs
/// them from the table, where a store that changes the pad must stop the core before the next
/// op as the fetch does. Every pass shows the same two changes the same number of cycles apart.
#[test]
fn a_pad_change_stops_the_core_on_a_pass_it_runs_from_the_table() {
    let mut program = Program::new(&[GPIO, GPIO21]);
    program.l32r(2, 0).l32r(3, 1).s32i(3, 2, 0x24);
    let top = program.pc();
    program.s32i(3, 2, 0x08).s32i(3, 2, 0x0c).j(top);
    let mut chip = program.chip();
    let mut ticks = Vec::new();
    while let Some(halt) = chip.run_until(c(200)) {
        ticks.push(halt.at());
    }
    assert!(ticks.len() > 20, "{ticks:?}");
    // The output enable, then each pass's raise and lower. The first pass decodes its ops as
    // it runs them, and every later pass runs them from the table.
    let gaps: Vec<u64> = ticks.windows(2).map(|w| w[1].0 - w[0].0).collect();
    for k in 3..gaps.len() {
        assert_eq!(gaps[k], gaps[k - 2], "gap {k} of {gaps:?}");
    }
}
