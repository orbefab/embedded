//! Spins on `CCOUNT`. Inside a straight run the core skips the passes of a loop that reads
//! `CCOUNT` and branches back while too few cycles have passed. Each test runs a program on a
//! plain bus, which keeps no code version, so every op is a full step and nothing is skipped,
//! and on the same bus kept. Both must stop alike, leave the core alike, registers, cycle and
//! `CCOUNT` included, and make the same loads and stores. A skip shows as far fewer op starts.

use orbe_xtensa::{Core, Cycle, Esp32s3, Sr, Stop};

use crate::common::{Pieces, Straight, run_in_pieces};
use crate::core::{CODE, Flat, RAM, core_at};

const WOE: u32 = 1 << 18;
const LINE2: u32 = 1 << 19;
const TIMER0: u32 = 1 << 6;
const LEVEL2: u32 = 0x4020_0180;
const KERNEL: u32 = 0x4020_0300;

fn rsr_ccount(t: u8) -> [u8; 3] {
    [t << 4, 0xea, 0x03]
}

fn sub(r: u8, s: u8, t: u8) -> [u8; 3] {
    [t << 4, r << 4 | s, 0xc0]
}

fn add(r: u8, s: u8, t: u8) -> [u8; 3] {
    [t << 4, r << 4 | s, 0x80]
}

/// `mov ar, as`, which is `or ar, as, as`.
fn mov(r: u8, s: u8) -> [u8; 3] {
    [s << 4, r << 4 | s, 0x20]
}

fn neg(r: u8, t: u8) -> [u8; 3] {
    [t << 4, r << 4, 0x60]
}

fn addi(t: u8, s: u8, imm: i8) -> [u8; 3] {
    [t << 4 | 2, 0xc0 | s, imm as u8]
}

fn l32i(t: u8, s: u8) -> [u8; 3] {
    [t << 4 | 2, 0x20 | s, 0]
}

const NOP: [u8; 3] = [0xf0, 0x20, 0x00];

const BLT: u8 = 0x2;
const BLTU: u8 = 0x3;
const BGE: u8 = 0xa;
const BGEU: u8 = 0xb;

/// A branch on `as` and `at` at op index `at` of a program of 3-byte ops, to op index `to`.
fn branch(cond: u8, s: u8, t: u8, at: usize, to: usize) -> [u8; 3] {
    let imm = 3 * to as i32 - 3 * at as i32 - 4;
    [t << 4 | 7, cond << 4 | s, i8::try_from(imm).unwrap() as u8]
}

/// `bgez as` at op index `at`, to op index `to`.
fn bgez(s: u8, at: usize, to: usize) -> [u8; 3] {
    let imm = (3 * to as i32 - 3 * at as i32 - 4) as u32 & 0xfff;
    [0xd6, s | ((imm & 0xf) << 4) as u8, (imm >> 4) as u8]
}

fn hex(ops: &[[u8; 3]]) -> String {
    ops.iter()
        .flatten()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `rsr.ccount a9`, then `1: rsr.ccount a10; sub a10, a10, a9; bltu a10, a2, 1b`, then
/// `addi a3, a3, 1`, after `pad` `nop`s. So the loop's head is op `pad + 1`.
fn delay(pad: usize) -> Vec<[u8; 3]> {
    let mut ops = vec![NOP; pad];
    let head = pad + 1;
    ops.extend([
        rsr_ccount(9),
        rsr_ccount(10),
        sub(10, 10, 9),
        branch(BLTU, 10, 2, head + 2, head),
        addi(3, 3, 1),
    ]);
    ops
}

struct Ran {
    stop: Stop,
    core: Core<Esp32s3>,
    trace: u64,
    /// Op starts the core marked: full steps and the starts it marks without a poll.
    starts: u64,
}

fn run(mut core: Core<Esp32s3>, mut bus: Straight<Flat>, limit: Cycle, seed: Option<u64>) -> Ran {
    let mut pieces = seed.map(Pieces::new);
    let stop = run_in_pieces(&mut core, &mut bus, limit, &mut pieces);
    Ran {
        stop,
        core,
        trace: bus.trace,
        starts: bus.steps.get() + bus.begins,
    }
}

/// Runs `core` on `bus` to `limit` plain and kept, requires both runs to match, and returns
/// both, plain first.
fn both(core: &Core<Esp32s3>, bus: &Flat, limit: Cycle, seed: Option<u64>) -> (Ran, Ran) {
    let plain = run(core.clone(), Straight::plain(bus.clone()), limit, seed);
    let kept = run(
        core.clone(),
        Straight::kept(bus.clone(), 0..0, true),
        limit,
        seed,
    );
    assert_eq!(kept.stop, plain.stop, "the stop");
    assert_eq!(kept.core, plain.core, "the core");
    assert_eq!(
        kept.core.special(Sr::Ccount),
        plain.core.special(Sr::Ccount)
    );
    assert_eq!(kept.trace, plain.trace, "the loads and stores");
    (plain, kept)
}

/// A plain run marks each op's start twice, at its step's poll and at its ask whether to yield,
/// and a kept one at least once unless it skips the op. A check build runs each pass it would
/// skip, and requires the core to match the skip, so it skips nothing.
fn skipped(plain: &Ran, kept: &Ran) -> bool {
    kept.starts * 10 < plain.starts || cfg!(feature = "poll-skip-check")
}

fn ran_each_op(plain: &Ran, kept: &Ran) -> bool {
    kept.starts * 2 >= plain.starts
}

fn spinner(wait: u32) -> Core<Esp32s3> {
    let mut core = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_a(2, wait);
    core.set_a(5, RAM);
    core
}

/// A wait of each length, from a loop head at each word offset, ends on the cycle and with the
/// registers that single steps leave. A wait of a few passes or more is skipped.
#[test]
fn a_spin_ends_where_single_steps_end_it() {
    for pad in 0..4 {
        let bus = Flat::new(&hex(&delay(pad)));
        for wait in [0, 1, 2, 5, 6, 7, 8, 11, 12, 13, 100, 1_000, 12_345, 77_777] {
            let (plain, kept) = both(&spinner(wait), &bus, Cycle(100_000), None);
            assert_eq!(kept.stop, Stop::Bus, "pad {pad} wait {wait}");
            assert_eq!(kept.core.a(3), 1, "pad {pad} wait {wait}");
            assert!(
                kept.core.a(10) >= wait,
                "pad {pad} wait {wait}: {}",
                kept.core.a(10)
            );
            if wait >= 1_000 {
                assert!(skipped(&plain, &kept), "pad {pad} wait {wait}");
            }
        }
    }
}

/// A run stops at every limit around and inside a spin, and on every split into pieces, where
/// a run of single steps stops: before, on and after the cycle the loop falls out at.
#[test]
fn a_spin_stops_at_every_limit_as_single_steps_do() {
    for pad in 0..4 {
        let bus = Flat::new(&hex(&delay(pad)));
        for limit in 1..400 {
            both(&spinner(300), &bus, Cycle(limit), None);
        }
        for seed in 1..40 {
            both(&spinner(20_000), &bus, Cycle(40_000), Some(seed));
        }
    }
}

/// A level-2 line that rises at each cycle of several passes inside a long wait is taken on the
/// step single steps take it at, with the registers of the pass it cut.
#[test]
fn a_line_rising_at_each_cycle_of_a_pass_is_taken_alike() {
    let mut core = spinner(50_000);
    core.set_special(Sr::IntEnable, LINE2);
    for pad in 0..2 {
        let mut bus = Flat::new(&hex(&delay(pad)));
        for at in 2_000..2_020 {
            bus.rise = Some((Cycle(at), LINE2));
            let (plain, kept) = both(&core, &bus, Cycle(100_000), None);
            assert_eq!(kept.core.pc(), LEVEL2, "at {at}");
            assert!(skipped(&plain, &kept), "at {at}");
        }
    }
}

/// A `CCOMPARE0` match at each cycle of several passes inside a long wait is taken alike.
#[test]
fn a_timer_match_at_each_cycle_of_a_pass_is_taken_alike() {
    for pad in 0..2 {
        let bus = Flat::new(&hex(&delay(pad)));
        for at in 3_000..3_020 {
            let mut core = spinner(50_000);
            core.set_special(Sr::IntEnable, TIMER0);
            core.set_special(Sr::Ccompare0, at);
            let (plain, kept) = both(&core, &bus, Cycle(100_000), None);
            assert_eq!(kept.core.pc(), KERNEL, "at {at}");
            assert!(skipped(&plain, &kept), "at {at}");
        }
    }
}

/// A wait whose cycles have already passed makes one pass and falls through.
#[test]
fn a_spin_whose_wait_has_passed_falls_through() {
    let bus = Flat::new(&hex(&delay(0)));
    for passed in [5_000, 5_001, 1 << 31, u32::MAX] {
        let mut core = spinner(5_000);
        core.set_pc(CODE + 3);
        core.set_special(Sr::Ccount, 100_000);
        core.set_a(9, 100_000u32.wrapping_sub(passed));
        let (_, kept) = both(&core, &bus, Cycle(100_000), None);
        assert_eq!(kept.core.a(3), 1);
        assert_eq!(kept.core.a(10), passed, "one pass, read on its first cycle");
    }
}

/// `CCOUNT` wraps inside a wait: the difference the loop compares stays right, as on the chip.
#[test]
fn a_spin_across_a_ccount_wrap() {
    let bus = Flat::new(&hex(&delay(1)));
    for before in [1, 2, 3, 6, 100, 30_000] {
        let mut core = spinner(50_000);
        core.set_special(Sr::Ccount, 0u32.wrapping_sub(before));
        let (plain, kept) = both(&core, &bus, Cycle(100_000), None);
        assert_eq!(kept.core.a(3), 1);
        assert!(kept.core.special(Sr::Ccount) < 60_000);
        assert!(skipped(&plain, &kept), "{before} before the wrap");
    }
}

/// Other spellings of a wait: a signed compare, a bound on the left, a deadline counted down,
/// a copy and a negation on the way.
#[test]
fn other_spins_are_skipped_alike() {
    // 1: rsr.ccount a10; sub a10, a10, a9; blt a10, a2, 1b
    let signed = vec![
        rsr_ccount(9),
        rsr_ccount(10),
        sub(10, 10, 9),
        branch(BLT, 10, 2, 3, 1),
        addi(3, 3, 1),
    ];
    // 1: rsr.ccount a10; sub a10, a10, a9; bgeu a2, a10, 1b
    let left = vec![
        rsr_ccount(9),
        rsr_ccount(10),
        sub(10, 10, 9),
        branch(BGEU, 2, 10, 3, 1),
        addi(3, 3, 1),
    ];
    // a2 = CCOUNT + a2, then 1: rsr.ccount a10; sub a10, a2, a10; bgez a10, 1b
    let deadline = vec![
        rsr_ccount(9),
        add(2, 2, 9),
        rsr_ccount(10),
        sub(10, 2, 10),
        bgez(10, 4, 2),
        addi(3, 3, 1),
    ];
    // The same against a register: 1: rsr.ccount a10; sub a10, a2, a10; bge a10, a11, 1b, with
    // a11 zero.
    let deadline_reg = vec![
        rsr_ccount(9),
        add(2, 2, 9),
        rsr_ccount(10),
        sub(10, 2, 10),
        branch(BGE, 10, 11, 4, 2),
        addi(3, 3, 1),
    ];
    // 1: rsr.ccount a11; mov a10, a11; neg a12, a9; add a10, a10, a12; nop; blt a10, a2, 1b
    let copied = vec![
        rsr_ccount(9),
        rsr_ccount(11),
        mov(10, 11),
        neg(12, 9),
        add(10, 10, 12),
        NOP,
        branch(BLT, 10, 2, 6, 1),
        addi(3, 3, 1),
    ];
    for (name, ops, wait) in [
        ("signed", signed, 30_000),
        ("left", left, 30_000),
        ("deadline", deadline, 30_000),
        ("deadline against a register", deadline_reg, 30_000),
        ("copied", copied, 30_000),
    ] {
        let bus = Flat::new(&hex(&ops));
        let (plain, kept) = both(&spinner(wait), &bus, Cycle(100_000), None);
        assert_eq!(kept.stop, Stop::Bus, "{name}");
        assert_eq!(kept.core.a(3), 1, "{name}");
        assert!(skipped(&plain, &kept), "{name}");
        for limit in [1_000, 29_999, 30_000, 30_001, 30_005] {
            both(&spinner(wait), &bus, Cycle(limit), None);
        }
    }

    // A wait that runs while the difference is at least the bound, so until it wraps.
    // 1: rsr.ccount a10; sub a10, a10, a9; bgeu a10, a2, 1b, from a difference of 2^32 - 30000.
    let wrap = vec![
        rsr_ccount(10),
        sub(10, 10, 9),
        branch(BGEU, 10, 2, 2, 0),
        addi(3, 3, 1),
    ];
    let bus = Flat::new(&hex(&wrap));
    let mut core = spinner(1 << 31);
    core.set_a(9, 30_000);
    let (plain, kept) = both(&core, &bus, Cycle(100_000), None);
    assert_eq!(kept.core.a(3), 1);
    assert!(skipped(&plain, &kept));
}

/// A loop with an op beyond a wait's runs op by op: a count, a write of the start it reads, a
/// load, and `CCOUNT` read twice and added.
#[test]
fn a_loop_with_another_op_is_not_skipped() {
    let counted = vec![
        rsr_ccount(9),
        rsr_ccount(10),
        sub(10, 10, 9),
        addi(3, 3, 1),
        branch(BLTU, 10, 2, 4, 1),
    ];
    let moved_start = vec![
        rsr_ccount(9),
        rsr_ccount(10),
        sub(10, 10, 9),
        addi(9, 9, 1),
        branch(BLTU, 10, 2, 4, 1),
    ];
    let loaded = vec![
        rsr_ccount(9),
        rsr_ccount(10),
        l32i(4, 5),
        sub(10, 10, 9),
        branch(BLTU, 10, 2, 4, 1),
    ];
    let twice = vec![
        rsr_ccount(9),
        rsr_ccount(10),
        rsr_ccount(11),
        add(10, 10, 11),
        branch(BLTU, 10, 2, 4, 1),
    ];
    for (name, ops) in [
        ("counted", counted),
        ("moved start", moved_start),
        ("loaded", loaded),
        ("twice", twice),
    ] {
        let bus = Flat::new(&hex(&ops));
        let (plain, kept) = both(&spinner(3_000), &bus, Cycle(100_000), None);
        assert_eq!(kept.stop, Stop::Bus, "{name}");
        assert!(ran_each_op(&plain, &kept), "{name}: skipped");
    }
}
