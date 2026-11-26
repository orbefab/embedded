//! The ops a straight run runs in line. Each program in `fixtures/inline` runs on a plain bus,
//! which keeps no code version, so every op is a full step through the core's `run`, and on the
//! same bus kept, where its straight runs take the commonest ops in line. Both must stop alike,
//! leave the core alike and make the same loads and stores with the same values on the same
//! cycles, and the kept run must make fewer full steps. A timer interrupt whose first match
//! moves a cycle at a time lands on each op of the programs in turn, and its handler sets the
//! next match 61 cycles on. `fixtures/inline/build.sh` assembles the programs.

use std::ops::Range;

use orbe_xtensa::{Bus, Cause, Core, Cycle, Esp32s3, Exit, PagedRam, Sr, Stop, Width};

use crate::common::{Pieces, Straight, run_in_pieces};
use crate::core::core_at;

/// `VECBASE`, where each program's image starts. The reset PC is 0x400 on.
const BASE: u32 = 0x4020_0000;
/// The data the programs write, past their code.
const DATA: u32 = 0x4020_8000;
/// What the programs' stores can make the core run again.
const CODE: Range<u32> = BASE..DATA;
const WOE: u32 = 1 << 18;
const TIMER0: u32 = 1 << 6;

const OPS: &[u8] = include_bytes!("fixtures/inline/ops.bin");
const WINDOWS: &[u8] = include_bytes!("fixtures/inline/windows.bin");
const LOOPS: &[u8] = include_bytes!("fixtures/inline/loops.bin");
const SMC: &[u8] = include_bytes!("fixtures/inline/smc.bin");

/// 64 KB of memory from [`BASE`] that fetches, loads and stores, with no interrupt line.
#[derive(Clone)]
struct Board {
    mem: PagedRam,
}

impl Board {
    fn new(image: &[u8]) -> Board {
        let mut mem = PagedRam::new(BASE, 64 * 1024);
        mem.write_bytes(BASE, image);
        Board { mem }
    }
}

impl Bus for Board {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        if self.mem.contains(addr) {
            Ok(self.mem.read(addr, Width::Word))
        } else {
            Err(Exit::Stop)
        }
    }

    fn load(&mut self, _now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        if self.mem.contains(addr) {
            Ok(self.mem.read(addr, width))
        } else {
            Err(Exit::Fault(Cause::LoadProhibited))
        }
    }

    fn store(&mut self, _now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        if !self.mem.contains(addr) {
            return Err(Exit::Fault(Cause::StoreProhibited));
        }
        self.mem.write(addr, width, value);
        Ok(())
    }

    fn interrupts(&mut self, _now: Cycle) -> u32 {
        0
    }

    fn next_event(&mut self, _now: Cycle) -> Option<Cycle> {
        None
    }
}

/// The core at the reset PC with `PS.WOE` set, at level 0, and the timer's first match at
/// `timer`, or no timer.
fn core(timer: Option<u32>) -> Core<Esp32s3> {
    let mut core = core_at(BASE + 0x400);
    core.set_special(Sr::Ps, WOE);
    if let Some(at) = timer {
        core.set_special(Sr::IntEnable, TIMER0);
        core.set_special(Sr::Ccompare0, at);
    }
    core
}

struct Ran {
    stop: Stop,
    core: Core<Esp32s3>,
    bus: Straight<Board>,
}

fn run(mut core: Core<Esp32s3>, mut bus: Straight<Board>, limit: Cycle) -> Ran {
    let stop = core.run_until(&mut bus, limit);
    Ran { stop, core, bus }
}

/// Runs `image` plain and kept to `limit`, requires both runs to match, and returns the kept
/// one.
fn same(image: &[u8], core: &Core<Esp32s3>, limit: Cycle) -> Ran {
    let plain = run(core.clone(), Straight::plain(Board::new(image)), limit);
    let kept = run(
        core.clone(),
        Straight::kept(Board::new(image), CODE, true),
        limit,
    );
    matches(&kept, &plain);
    kept
}

fn matches(kept: &Ran, plain: &Ran) {
    assert_eq!(kept.stop, plain.stop, "the stop");
    assert_eq!(kept.core, plain.core, "the core");
    assert_eq!(kept.bus.trace, plain.bus.trace, "the loads and stores");
    if cfg!(feature = "poll-skip-check") {
        assert_eq!(kept.bus.steps, plain.bus.steps, "the asks of a check build");
    } else {
        assert!(
            kept.bus.steps < plain.bus.steps,
            "{} full steps kept, {} plain",
            kept.bus.steps.get(),
            plain.bus.steps.get()
        );
    }
}

fn word(ran: &Ran, addr: u32) -> u32 {
    ran.bus.bus.mem.read(addr, Width::Word)
}

/// Every op `run_clear` takes, with data that moves each pass: the arithmetic, logic, shift,
/// move, `sar`, multiply and `rsr.ccount` ops, the loads, `l32r` and the stores, `memw` after
/// a store and after a load, each branch taken on some passes and not on others, `j`, `jx`,
/// `call0`, `callx0`, `ret` and `ret.n`.
#[test]
fn every_op_run_in_line_runs_as_a_full_step_runs_it() {
    let limit = Cycle(60_000);
    let ran = same(OPS, &core(None), limit);
    assert_eq!(ran.stop, Stop::Spin);
    assert_eq!(ran.core.a(2), 0, "every pass ran");
    for at in 9_000..9_200 {
        same(OPS, &core(Some(at)), limit);
    }
}

/// A run cut at random limits matches one run, plain and kept.
#[test]
fn every_op_run_in_line_runs_the_same_cut_into_pieces() {
    let end = Cycle(40_000);
    let core = core(Some(700));
    let mut plain = Straight::plain(Board::new(OPS));
    let mut whole = core.clone();
    assert_eq!(
        run_in_pieces(&mut whole, &mut plain, end, &mut None),
        Stop::Limit
    );
    for seed in [1, 2, 3, 4] {
        let mut kept = Straight::kept(Board::new(OPS), CODE, true);
        let mut split = core.clone();
        let stop = run_in_pieces(&mut split, &mut kept, end, &mut Some(Pieces::new(seed)));
        assert_eq!(stop, Stop::Limit, "seed {seed}");
        assert_eq!(split, whole, "seed {seed}");
        assert_eq!(kept.trace, plain.trace, "seed {seed}");
    }
}

/// Fibonacci by `call4`, by `call8` and `callx8`, and by `call12`, each deep enough that its
/// `entry` overflows the window and its `retw` underflows it, through the spill and fill
/// handlers at `VECBASE`. The ops of the bodies run in line between them.
#[test]
fn window_overflows_and_underflows_land_on_the_same_ops() {
    let limit = Cycle(120_000);
    let ran = same(WINDOWS, &core(None), limit);
    assert_eq!(ran.stop, Stop::Spin);
    let stack = 0x4020_c000;
    assert_eq!(word(&ran, stack), 89, "fib(11) by call4");
    assert_eq!(word(&ran, stack + 4), 233, "fib(13) by call8");
    assert_eq!(word(&ran, stack + 8), 34, "fib(9) by call12");
    for at in 2_000..2_120 {
        same(WINDOWS, &core(Some(at)), limit);
    }
}

/// `loopnez` over a body of ops run in line, a loop of one op, a loop that ends in a branch
/// out of it, and a `loopgtz` that a negative count skips. Each loop end falls inside a run
/// of ops that runs in line, and the op that loops back runs through `run`.
#[test]
fn a_loop_end_inside_a_run_loops_back_on_the_same_op() {
    let limit = Cycle(40_000);
    let ran = same(LOOPS, &core(None), limit);
    assert_eq!(ran.stop, Stop::Spin);
    assert_eq!(ran.core.a(2), 0, "every pass ran");
    assert_eq!(ran.core.a(9), 30 * 4, "the one-op loop");
    for at in 1_000..1_150 {
        same(LOOPS, &core(Some(at)), limit);
    }
}

/// Every fourth pass stores one of two words over the two ops right after the store, from
/// inside the straight run the rest of the pass runs in. The store moves the code version, so
/// the run ends at the new ops, and a full step fetches them.
#[test]
fn a_store_over_the_next_op_runs_the_new_op() {
    let limit = Cycle(20_000);
    let ran = same(SMC, &core(None), limit);
    assert_eq!(ran.stop, Stop::Spin);
    // Passes 24 to 21, 16 to 13 and 8 to 5 run `addi.n a6, a6, 2; addi.n a7, a7, 3`, the
    // others the ones.
    assert_eq!((ran.core.a(6), ran.core.a(7)), (36, 48));
    for at in 300..420 {
        same(SMC, &core(Some(at)), limit);
    }
}

/// A kept run commits, runs on past stores over its code, and rewinds to the commit as the
/// engine does, with `clone_from` over the core and the bus. The control word then flips, so
/// the stores after the rewind write the other words at the code versions the run ahead wrote
/// first. The rewound run must run the words written after the rewind: it matches a plain run
/// and a kept run that flip the word at the commit without running ahead.
#[test]
fn a_rewind_past_a_code_change_runs_the_code_written_after_it() {
    let end = Cycle(20_000);
    let flip = |bus: &mut Straight<Board>| bus.bus.mem.write(DATA + 64, Width::Word, 1);
    for commit in [150, 400, 777] {
        let reference = |mut bus: Straight<Board>| {
            let mut core = core(Some(600));
            assert_eq!(core.run_until(&mut bus, Cycle(commit)), Stop::Limit);
            flip(&mut bus);
            run(core, bus, end)
        };
        let plain = reference(Straight::plain(Board::new(SMC)));
        let kept = reference(Straight::kept(Board::new(SMC), CODE, true));
        matches(&kept, &plain);

        let mut ahead = core(Some(600));
        let mut bus = Straight::kept(Board::new(SMC), CODE, true);
        assert_eq!(ahead.run_until(&mut bus, Cycle(commit)), Stop::Limit);
        let committed = (ahead.clone(), bus.clone());
        assert_eq!(ahead.run_until(&mut bus, Cycle(commit + 600)), Stop::Limit);
        ahead.clone_from(&committed.0);
        bus.clone_from(&committed.1);
        flip(&mut bus);
        let rewound = run(ahead, bus, end);
        assert_eq!(rewound.stop, plain.stop, "commit {commit}");
        assert_eq!(rewound.core, plain.core, "commit {commit}");
        assert_eq!(rewound.bus.trace, plain.bus.trace, "commit {commit}");
    }
}
