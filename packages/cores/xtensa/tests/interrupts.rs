//! Interrupts, the timers and `WAITI` on the flat test bus, with the interrupt table both chips'
//! `core-isa.h` give.

use orbe_xtensa::{Bus, Cause, Core, Cycle, Esp32, Esp32s3, Exit, Sr, Stop, Width};

use crate::common::{Pieces, run_in_pieces};
use crate::core::{CODE, Flat, core_at};

const WOE: u32 = 1 << 18;
const EXCM: u32 = 1 << 4;
const UM: u32 = 1 << 5;
const KERNEL: u32 = 0x4000_0300;
const USER: u32 = 0x4000_0340;

/// Software interrupt 7 at level 1, software interrupt 29 at level 3, external level interrupts
/// 19 at level 2 and 24 at level 4, external edge interrupt 10 at level 1, the NMI 14, and timer
/// 0's interrupt 6 at level 1 and timer 1's 15 at level 3.
const SOFT1: u32 = 1 << 7;
const SOFT3: u32 = 1 << 29;
const LINE2: u32 = 1 << 19;
const LINE4: u32 = 1 << 24;
const EDGE1: u32 = 1 << 10;
const NMI: u32 = 1 << 14;
const TIMER0: u32 = 1 << 6;
const TIMER1: u32 = 1 << 15;

/// `nop`s from `CODE`, one cycle each.
fn nops() -> Flat {
    Flat::new(&"f02000".repeat(400))
}

fn level_vector(level: u32) -> u32 {
    0x4000_0100 + 0x40 * level
}

fn setup(ps: u32, enabled: u32) -> Core<Esp32> {
    let mut core = core_at(CODE);
    core.set_special(Sr::Ps, ps);
    core.set_special(Sr::IntEnable, enabled);
    core
}

#[test]
fn a_level_1_interrupt_raises_the_level_1_exception() {
    let mut bus = nops();
    for (ps, vector) in [(WOE, KERNEL), (WOE | UM, USER)] {
        let mut core = setup(ps, SOFT1);
        core.set_special(Sr::Interrupt, SOFT1);
        assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit);
        assert_eq!(core.pc(), vector);
        assert_eq!(core.special(Sr::ExcCause), Cause::Level1Interrupt as u32);
        assert_eq!(core.special(Sr::Epc1), CODE);
        assert_eq!(core.special(Sr::Ps), ps | EXCM);
        assert_eq!(core.cycle(), Cycle(4));
    }
}

/// `PS.INTLEVEL` masks its level and below. A higher level saves `PC` and `PS` in its own
/// `EPC` and `EPS` (4.4.6.4, pp 160-161).
#[test]
fn intlevel_masks_its_level_and_below() {
    let mut bus = nops();
    let mut core = setup(WOE | 1, SOFT1 | SOFT3);
    core.set_special(Sr::Interrupt, SOFT1);
    assert_eq!(core.run_until(&mut bus, Cycle(10)), Stop::Limit);
    assert_eq!(core.pc(), CODE + 30);

    core.set_special(Sr::Interrupt, SOFT1 | SOFT3);
    assert_eq!(core.run_until(&mut bus, Cycle(11)), Stop::Limit);
    assert_eq!(core.pc(), level_vector(3));
    assert_eq!(core.special(Sr::Epc3), CODE + 30);
    assert_eq!(core.special(Sr::Eps3), WOE | 1);
    assert_eq!(core.special(Sr::Ps), WOE | EXCM | 3);
}

/// `PS.EXCM` masks up to `EXCM_LEVEL`, 3, and the highest pending level goes first.
#[test]
fn exception_mode_masks_up_to_level_3() {
    let mut bus = nops();
    let mut core = setup(WOE | EXCM, SOFT1 | SOFT3 | LINE2);
    core.set_special(Sr::Interrupt, SOFT1 | SOFT3);
    bus.lines = LINE2;
    assert_eq!(core.run_until(&mut bus, Cycle(10)), Stop::Limit);
    assert_eq!(core.pc(), CODE + 30);

    core.set_special(Sr::IntEnable, SOFT1 | SOFT3 | LINE2 | LINE4);
    bus.lines = LINE2 | LINE4;
    assert_eq!(core.run_until(&mut bus, Cycle(11)), Stop::Limit);
    assert_eq!(core.pc(), level_vector(4));
    assert_eq!(core.special(Sr::Epc4), CODE + 30);
    assert_eq!(core.special(Sr::Eps4), WOE | EXCM);
    assert_eq!(core.special(Sr::Ps), WOE | EXCM | 4);
}

#[test]
fn the_highest_pending_level_goes_first() {
    let mut bus = nops();
    let mut core = setup(WOE, SOFT1 | SOFT3 | LINE2);
    core.set_special(Sr::Interrupt, SOFT1 | SOFT3);
    bus.lines = LINE2;
    assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit);
    assert_eq!(core.pc(), level_vector(3));
}

/// The NMI is taken on the line's rise, whatever `PS` and `INTENABLE` hold, and not again while
/// the line stays high.
#[test]
fn the_nmi_is_taken_once_per_rise() {
    let mut bus = nops();
    bus.rise = Some((Cycle(10), NMI));
    let ps = WOE | EXCM | 15;
    let mut core = setup(ps, 0);
    assert_eq!(core.run_until(&mut bus, Cycle(11)), Stop::Limit);
    assert_eq!(core.pc(), level_vector(7));
    assert_eq!(core.special(Sr::Epc7), CODE + 30);
    assert_eq!(core.special(Sr::Eps7), ps);
    assert_eq!(core.special(Sr::Ps), WOE | EXCM | 7);
    assert_eq!(core.special(Sr::Interrupt), 0);

    core.set_special(Sr::Ps, ps);
    core.set_pc(CODE);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Limit);
    assert_eq!(core.pc(), CODE + 3 * (100 - 14));
}

/// An edge latches in `INTERRUPT` until `INTCLEAR` clears it. A level line shows as it is.
#[test]
fn an_edge_latches_and_a_level_line_does_not() {
    // wsr.intclear a2; nop ...
    let mut bus = Flat::new(&format!("20e313{}", "f02000".repeat(20)));
    bus.lines = EDGE1 | 1;
    let mut core = setup(WOE, 0);
    core.set_a(2, u32::MAX);
    assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit);
    assert_eq!(core.special(Sr::Interrupt), 1);
    assert_eq!(core.run_until(&mut bus, Cycle(10)), Stop::Limit);
    assert_eq!(core.special(Sr::Interrupt), 1);

    bus.lines = 0;
    bus.rise = Some((Cycle(12), EDGE1));
    assert_eq!(core.run_until(&mut bus, Cycle(20)), Stop::Limit);
    assert_eq!(core.special(Sr::Interrupt), EDGE1);
}

/// `CCOMPAREn` raises its interrupt at the cycle `CCOUNT` equals it (4.4.7.2, p 162), and
/// writing `CCOUNT` moves that cycle.
#[test]
fn a_timer_fires_when_ccount_reaches_ccompare() {
    let mut bus = nops();
    let mut core = setup(WOE, TIMER0);
    core.set_special(Sr::Ccompare0, 50);
    assert_eq!(core.run_until(&mut bus, Cycle(51)), Stop::Limit);
    assert_eq!(core.pc(), KERNEL);
    assert_eq!(core.special(Sr::Epc1), CODE + 3 * 50);
    assert_eq!(core.special(Sr::Interrupt), TIMER0);
    assert_eq!(core.cycle(), Cycle(54));

    let mut core = setup(WOE, TIMER1);
    core.set_special(Sr::Ccompare1, 50);
    core.set_special(Sr::Ccount, 40);
    assert_eq!(core.run_until(&mut bus, Cycle(11)), Stop::Limit);
    assert_eq!(core.pc(), level_vector(3));
    assert_eq!(core.special(Sr::Epc3), CODE + 3 * 10);
}

/// A timer whose match is now fires a whole count later, not at once.
#[test]
fn a_timer_set_to_ccount_waits_a_whole_count() {
    let mut bus = nops();
    let mut core = setup(WOE, TIMER0);
    core.set_special(Sr::Ccount, 7);
    core.set_special(Sr::Ccompare0, 7);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Limit);
    assert_eq!(core.special(Sr::Interrupt), 0);
}

/// `waiti 0; nop`
const WAITI: &str = "007000f02000";

/// `WAITI` with no interrupt to take moves the cycle to the bus's next event and takes the
/// interrupt there, as `rfi` would return past the `WAITI`. The core reads the lines before the
/// `WAITI` and at the event. The wait's own step and the fetch of the vector keep those lines.
#[test]
fn waiti_skips_to_the_next_event_and_wakes_there() {
    let mut bus = Flat::new(WAITI);
    bus.rise = Some((Cycle(5000), LINE2));
    let mut core = setup(WOE | 5, LINE2);
    assert_eq!(core.run_until(&mut bus, Cycle(1_000_000)), Stop::Bus);
    assert_eq!(core.pc(), level_vector(2));
    assert_eq!(core.special(Sr::Epc2), CODE + 3);
    assert_eq!(core.special(Sr::Eps2), WOE);
    assert_eq!(core.special(Sr::Ps), WOE | EXCM | 2);
    assert_eq!(core.cycle(), Cycle(5000 + 4));
    #[cfg(not(feature = "poll-skip-check"))]
    assert_eq!(bus.polls, 2);
}

/// A limit inside the wait stops it there, and the next run waits on to the same event.
#[test]
fn a_limit_inside_waiti_splits_the_wait() {
    let mut bus = Flat::new(WAITI);
    bus.rise = Some((Cycle(5000), LINE2));
    let mut whole = setup(WOE, LINE2);
    let mut split = whole.clone();
    assert_eq!(whole.run_until(&mut bus.clone(), Cycle(6000)), Stop::Bus);

    assert_eq!(split.run_until(&mut bus, Cycle(3000)), Stop::Limit);
    assert_eq!(split.cycle(), Cycle(3000));
    assert_eq!(split.pc(), CODE + 3);
    assert_eq!(split.run_until(&mut bus, Cycle(6000)), Stop::Bus);
    assert_eq!(split, whole);
    assert_eq!(split.special(Sr::Epc2), CODE + 3);
}

/// A core in `WAITI` has no wake until it has idled, so a run whose limit falls on the end of
/// the `WAITI` itself leaves it to poll there. Each idle then gives the earlier of the bus's
/// next event and a timer match, whether or not the match is enabled. An interrupt taken ends
/// the wait and the wake.
#[test]
fn a_waiting_core_has_a_wake_once_it_has_idled() {
    let mut bus = Flat::new(WAITI);
    bus.rise = Some((Cycle(5000), LINE2));
    let mut core = setup(WOE, LINE2);
    core.set_special(Sr::Ccompare0, 4000);
    assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit);
    assert_eq!((core.waiting(), core.wake()), (true, None));
    assert_eq!(core.run_until(&mut bus, Cycle(3000)), Stop::Limit);
    assert_eq!(core.wake(), Some(Cycle(4000)));
    assert_eq!(core.run_until(&mut bus, Cycle(4500)), Stop::Limit);
    assert_eq!(core.wake(), Some(Cycle(5000)));
    assert_eq!(core.run_until(&mut bus, Cycle(6000)), Stop::Bus);
    assert_eq!((core.waiting(), core.wake()), (false, None));
}

/// The flat bus with an event due on every cycle it is asked about, which [`Bus::next_event`]
/// rules out.
struct EventNow(Flat);

impl Bus for EventNow {
    fn instruction_bus(&self, addr: u32) -> bool {
        self.0.instruction_bus(addr)
    }

    fn fetch(&mut self, now: Cycle, addr: u32) -> Result<u32, Exit> {
        self.0.fetch(now, addr)
    }

    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        self.0.load(now, addr, width)
    }

    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        self.0.store(now, addr, width, value)
    }

    fn interrupts(&mut self, now: Cycle) -> u32 {
        self.0.interrupts(now)
    }

    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        Some(now)
    }
}

/// An idle whose bus gives an event at the idle's own cycle fails a debug build. A release build
/// records the wake as the cycle the core moved to, so a core that stops at its limit has its
/// wake there.
#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(expected = "a wake at Cycle(1), not after Cycle(1)")
)]
fn a_wake_not_after_the_idles_cycle_is_the_cycle_it_moved_to() {
    let mut core = setup(WOE, LINE2);
    let mut bus = EventNow(Flat::new(WAITI));
    assert_eq!(core.run_until(&mut bus, Cycle(3000)), Stop::Limit);
    assert_eq!(core.wake(), Some(Cycle(3000)));
}

#[test]
fn waiti_wakes_at_a_timer_match() {
    let mut bus = Flat::new(WAITI);
    let mut core = setup(WOE, TIMER0);
    core.set_special(Sr::Ccompare0, 700);
    assert_eq!(core.run_until(&mut bus, Cycle(1_000_000)), Stop::Bus);
    assert_eq!(core.pc(), KERNEL);
    assert_eq!(core.special(Sr::Epc1), CODE + 3);
    assert_eq!(core.cycle(), Cycle(700 + 4));
}

/// `waiti 3` masks a level-2 line, so the core waits to the limit and stops there exactly. It
/// reads the lines before the `WAITI` and at the line's rise. The start of the wait keeps the
/// lines from the first read.
#[test]
fn a_masked_wait_runs_to_the_limit() {
    // waiti 3; nop
    let mut bus = Flat::new("007300f02000");
    bus.rise = Some((Cycle(5000), LINE2));
    let mut core = setup(WOE, LINE2);
    assert_eq!(core.run_until(&mut bus, Cycle(1_000_000)), Stop::Limit);
    assert_eq!(core.cycle(), Cycle(1_000_000));
    assert_eq!(core.pc(), CODE + 3);
    assert_eq!(core.special(Sr::Ps), WOE | 3);
    #[cfg(not(feature = "poll-skip-check"))]
    assert_eq!(bus.polls, 2);
}

/// A loop, then `WAITI` in a loop, woken by timer 0 every 100 cycles and once by edge
/// interrupt 22 at level 3 at cycle 5050, with `VECBASE` at `CODE + 0x400`:
///
/// ```text
///     movi a3, 0; movi a2, 50; loop a2, 2f; addi a8, a8, 1
/// 2:  waiti 0; addi a3, a3, 1; j 2b
/// level 3:  rsr.ccount a5; movi a6, 1; slli a6, a6, 22; wsr.intclear a6; addi a7, a7, 1; rfi 3
/// kernel:   rsr.ccompare0 a4; addi a4, a4, 100; wsr.ccompare0 a4; addi a9, a9, 1; rfe
/// ```
pub fn sleeper() -> (Core<Esp32s3>, Flat) {
    const MAIN: &str = "32a00022a03276820282c80100700032c30186fdff";
    const LEVEL3: &str = "50ea0362a001a0660160e31372c701103300";
    const KERNEL_HANDLER: &str = "40f00342c46440f01392c901003000";
    let pad = |from: usize, to: usize| "00".repeat(to - from);
    let hex = format!(
        "{MAIN}{}{LEVEL3}{}{KERNEL_HANDLER}",
        pad(MAIN.len() / 2, 0x5c0),
        pad(0x5c0 + LEVEL3.len() / 2, 0x700),
    );
    let mut bus = Flat::new(&hex);
    bus.rise = Some((Cycle(5050), 1 << 22));
    let mut core = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::Vecbase, CODE + 0x400);
    core.set_special(Sr::IntEnable, TIMER0 | 1 << 22);
    core.set_special(Sr::Ccompare0, 100);
    (core, bus)
}

/// Runs cut at random limits end in the state of one run.
#[test]
fn random_limit_splits_equal_one_run() {
    let end = Cycle(20_000);
    let (mut whole, mut bus) = sleeper();
    assert_eq!(
        run_in_pieces(&mut whole, &mut bus, end, &mut None),
        Stop::Limit
    );
    assert_eq!(whole.a(8), 50, "the loop ran");
    assert_eq!(whole.a(7), 1, "the edge woke the core once");
    assert!(whole.a(9) >= 199, "the timer woke it {} times", whole.a(9));
    assert_eq!(whole.a(3), whole.a(7) + whole.a(9));

    for seed in 1..=20 {
        let (mut split, mut bus) = sleeper();
        let stop = run_in_pieces(&mut split, &mut bus, end, &mut Some(Pieces::new(seed)));
        assert_eq!(stop, Stop::Limit, "seed {seed}");
        assert_eq!(split, whole, "seed {seed}");
    }
    println!(
        "split: 20 seeds equal one run to cycle {}, with {} timer and {} edge wakes",
        end.0,
        whole.a(9),
        whole.a(7)
    );
}
