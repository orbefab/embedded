//! Straight runs. On a bus with a code version the core runs the decodes it keeps back to back,
//! and before each op only marks the step and checks the cycle, while a full step could find
//! nothing. Each test runs a program on a plain bus, which keeps no version, so every op is a
//! full step, and on the same bus kept. Both must stop alike, leave the core alike and make the
//! same loads and stores with the same values on the same cycles. The kept run must also make
//! fewer full steps, so its straight runs ran.

use std::ops::Range;

use orbe_xtensa::{
    Bus, Cause, Config, Core, Cycle, Decoded, Esp32s3, Exit, Missing, Op, PagedRam, Sr, Stop, Width,
};

use crate::common::elf::Elf;
use crate::common::{Pieces, Straight, board, read_fixture, run_in_pieces, s3board};
use crate::core::{CODE, Flat, RAM, core_at};
use crate::interrupts::sleeper;

const WOE: u32 = 1 << 18;
const EXCM: u32 = 1 << 4;
const SOFT1: u32 = 1 << 7;
const LINE2: u32 = 1 << 19;
const TIMER0: u32 = 1 << 6;
const KERNEL: u32 = 0x4020_0300;
const LEVEL2: u32 = 0x4020_0180;
const DEBUG: u32 = 0x4020_0280;
const OVERFLOW12: u32 = 0x4020_0100;
const DEVICE: u32 = 0x5000_0000;

struct Ran {
    stop: Stop,
    core: Core<Esp32s3>,
    trace: u64,
    steps: u64,
}

fn run<B: Bus>(mut core: Core<Esp32s3>, mut bus: Straight<B>, limit: Cycle) -> Ran {
    let stop = core.run_until(&mut bus, limit);
    Ran {
        stop,
        core,
        trace: bus.trace,
        steps: bus.steps.get(),
    }
}

/// Runs `core` to `limit` on `make()` plain and kept, requires both runs to match, and returns
/// the kept one. A still bus says no access moves its lines.
fn same<B: Bus>(core: &Core<Esp32s3>, make: impl Fn() -> B, code: Range<u32>, still: bool) -> Ran {
    let limit = Cycle(100_000);
    let plain = run(core.clone(), Straight::plain(make()), limit);
    let kept = run(core.clone(), Straight::kept(make(), code, still), limit);
    assert_eq!(kept.stop, plain.stop, "the stop");
    assert_eq!(kept.core, plain.core, "the core");
    assert_eq!(kept.trace, plain.trace, "the loads and stores");
    // A check build asks whether to yield before each op of a straight run too, as a step does.
    if cfg!(feature = "poll-skip-check") {
        assert_eq!(kept.steps, plain.steps, "the asks of a check build");
    } else {
        assert!(
            kept.steps < plain.steps,
            "{} full steps kept, {} plain",
            kept.steps,
            plain.steps
        );
    }
    kept
}

fn flat(hex: &str) -> impl Fn() -> Flat {
    let bus = Flat::new(hex);
    move || bus.clone()
}

/// `movi a2, 0`, then a loop of `addi a2, a2, 1`, `addi a3, a3, 1`, `l32i a4, a5, 0`,
/// `s32i a4, a5, 4` and `j` back to the first `addi`.
const LOOP: &str = "22a00022c20132c30142250042650106fcff";

fn looping(ps: u32, enabled: u32) -> Core<Esp32s3> {
    let mut core = core_at(CODE);
    core.set_special(Sr::Ps, ps);
    core.set_special(Sr::IntEnable, enabled);
    core.set_a(5, RAM);
    core
}

/// A `CCOMPARE0` match falls inside a straight run, which ends at the match's cycle, so the
/// interrupt is taken before the same op on the same cycle.
#[test]
fn a_timer_match_inside_a_run_is_taken_on_its_cycle() {
    for at in [700, 701, 702, 703, 5_000] {
        let mut core = looping(WOE, TIMER0);
        core.set_special(Sr::Ccompare0, at);
        let ran = same(&core, flat(LOOP), 0..0, true);
        assert_eq!(ran.stop, Stop::Bus, "the vector lies outside the code");
        assert_eq!(ran.core.pc(), KERNEL);
        let start = ran.core.cycle().0 - 4;
        assert!(
            (u64::from(at)..u64::from(at) + 4).contains(&start),
            "taken at {start}"
        );
    }
}

/// A line that rises at the bus's next event inside a straight run is read on the first step at
/// or after that cycle.
#[test]
fn a_line_rising_inside_a_run_is_taken_on_its_cycle() {
    for at in [1000, 1001, 1002, 1003] {
        let make = || {
            let mut bus = Flat::new(LOOP);
            bus.rise = Some((Cycle(at), LINE2));
            bus
        };
        let ran = same(&looping(WOE, LINE2), make, 0..0, true);
        assert_eq!(ran.core.pc(), LEVEL2);
        let start = ran.core.cycle().0 - 4;
        assert!((at..at + 4).contains(&start), "taken at {start}");
    }
}

/// A loop of a masking op, twelve `addi`, an unmasking op, `addi` and `j` back. `CCOMPARE0`
/// matches on one of a span of cycles. Every match that lands while the loop masks it is taken
/// right after the unmasking op, which ends the straight run it ran in.
#[test]
fn an_op_that_lets_an_interrupt_through_ends_the_run() {
    let cases = [
        ("rsil", "706100", "606000", 0, 0),
        ("wsr.ps", "70e613", "60e613", WOE | 1, WOE),
        ("wsr.intenable", "70e413", "60e413", 0, TIMER0),
    ];
    for (op, mask, unmask, a7, a6) in cases {
        let code = format!("{mask}{}{unmask}22c201c6f3ff", "22c201".repeat(12));
        let mut after = 0;
        for at in 300..330 {
            let mut core = looping(WOE, TIMER0);
            core.set_special(Sr::Ccompare0, at);
            core.set_a(7, a7);
            core.set_a(6, a6);
            let ran = same(&core, flat(&code), 0..0, true);
            assert_eq!(ran.core.pc(), KERNEL, "{op} at {at}");
            if ran.core.special(Sr::Epc1) == CODE + 42 {
                after += 1;
            }
        }
        assert!(after > 15, "{op}: {after} of 30 matches taken after it");
    }
}

/// `addi` twice, `wsr.intset a5`, `slli a5, a5, 1`, `addi` and `j` back. `a5` starts at bit 3,
/// so the fifth pass sets software interrupt 7, which is taken right after the `wsr.intset`.
#[test]
fn a_kept_wsr_intset_ends_the_run() {
    let mut core = looping(WOE, SOFT1);
    core.set_a(5, 1 << 3);
    let ran = same(
        &core,
        flat("22c20122c20150e213f0551122c20146fbff"),
        0..0,
        true,
    );
    assert_eq!(ran.core.pc(), KERNEL);
    assert_eq!(ran.core.special(Sr::Epc1), CODE + 9);
    assert_eq!(ran.core.a(2), 4 * 3 + 2);
}

/// Each pass sets `CCOMPARE0` to `CCOUNT` plus `a8`: 1000 while `a7` counts down to 0, then 6.
/// The write ends the run and moves its horizon, so the match is taken six cycles on.
///
/// ```text
/// 1: rsr.ccount a6; movi a8, 1000; moveqz a8, a10, a7; add a6, a6, a8; wsr.ccompare0 a6
///    addi a2, a2, 1; addi a2, a2, 1; addi a2, a2, 1; addi a7, a7, -1; j 1b
/// ```
#[test]
fn a_kept_ccompare_write_moves_the_horizon() {
    let code = "60ea0382a3e8708a8380668060f01322c20122c20122c20172c7ff46f8ff";
    let mut core = looping(WOE, TIMER0);
    core.set_a(7, 3);
    core.set_a(10, 6);
    let ran = same(&core, flat(code), 0..0, true);
    assert_eq!(ran.core.pc(), KERNEL);
    assert_eq!(ran.core.special(Sr::Epc1), CODE + 0x12);
    assert_eq!(ran.core.a(7), 0);
}

/// An exception inside a straight run is taken at its op, on a later pass of a loop: a divide
/// by zero, a window overflow and a load the bus refuses.
#[test]
fn an_exception_inside_a_run_is_taken_at_its_op() {
    // 1: addi a2, a2, 1; addi a2, a2, 1; quou a3, a3, a4; addi a4, a4, -1; j 1b
    let mut core = looping(WOE, 0);
    core.set_a(3, 100);
    core.set_a(4, 3);
    let ran = same(&core, flat("22c20122c2014033c242c4ff06fcff"), 0..0, true);
    assert_eq!(ran.core.pc(), KERNEL);
    assert_eq!(ran.core.special(Sr::Epc1), CODE + 6);
    assert_eq!(
        ran.core.special(Sr::ExcCause),
        Cause::IntegerDivideByZero as u32
    );
    assert_eq!(ran.core.a(2), 8);

    // 1: wsr.windowstart a3; addi a5, a5, 1; addi a12, a12, 1; addi a2, a2, -1;
    //    moveqz a3, a4, a2; j 1b
    // The fourth pass makes the frame three quarters up live, and `a12` spills it.
    let mut core = looping(WOE, 0);
    core.set_a(2, 3);
    core.set_a(3, 1);
    core.set_a(4, 0b1001);
    let ran = same(
        &core,
        flat("30491352c501c2cc0122c2ff20348346fbff"),
        0..0,
        true,
    );
    assert_eq!(ran.core.pc(), OVERFLOW12);
    assert_eq!(ran.core.special(Sr::Epc1), CODE + 6);
    assert_eq!(ran.core.special(Sr::WindowBase), 3);
    assert_eq!(ran.core.special(Sr::Ps) & EXCM, EXCM);

    // 1: addi a2, a2, -1; l32i a8, a5, 0; moveqz a5, a6, a2; addi a9, a9, 1; j 1b
    // The fourth pass loads where the bus refuses.
    let mut core = looping(WOE, 0);
    core.set_a(2, 3);
    core.set_a(6, 0x2000_0000);
    let ran = same(&core, flat("22c2ff82250020568392c90106fcff"), 0..0, true);
    assert_eq!(ran.core.pc(), KERNEL);
    assert_eq!(ran.core.special(Sr::Epc1), CODE + 3);
    assert_eq!(ran.core.special(Sr::ExcCause), Cause::LoadProhibited as u32);
    assert_eq!(ran.core.a(9), 3);
}

/// What a store to [`DEVICE`] does, beside ending a straight run.
#[derive(Clone, Copy)]
enum Effect {
    Line,
    Yield,
    Hold,
}

/// Code and RAM in pages, and a device register whose store can move the lines. A store to it
/// raises line 2, turns `yields` on or holds every fetch.
#[derive(Clone)]
struct Gate {
    code: PagedRam,
    ram: PagedRam,
    effect: Effect,
    high: bool,
    yielding: bool,
    held: bool,
}

impl Gate {
    fn new(hex: &str, effect: Effect) -> Gate {
        let mut code = PagedRam::new(CODE, 4096);
        code.write_bytes(CODE, &crate::common::board::bytes_of(hex));
        Gate {
            code,
            ram: PagedRam::new(RAM, 4096),
            effect,
            high: false,
            yielding: false,
            held: false,
        }
    }

    fn memory(&mut self, addr: u32) -> Option<&mut PagedRam> {
        [&mut self.code, &mut self.ram]
            .into_iter()
            .find(|ram| ram.contains(addr))
    }
}

impl Bus for Gate {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        if self.held || !self.code.contains(addr) {
            return Err(Exit::Stop);
        }
        Ok(self.code.read(addr, Width::Word))
    }

    fn load(&mut self, _now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        match self.memory(addr) {
            Some(ram) => Ok(ram.read(addr, width)),
            None if addr == DEVICE => Ok(0),
            None => Err(Exit::Fault(Cause::LoadProhibited)),
        }
    }

    fn store(&mut self, _now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        if let Some(ram) = self.memory(addr) {
            ram.write(addr, width, value);
            return Ok(());
        }
        if addr != DEVICE {
            return Err(Exit::Fault(Cause::StoreProhibited));
        }
        match self.effect {
            Effect::Line => self.high = true,
            Effect::Yield => self.yielding = true,
            Effect::Hold => self.held = true,
        }
        Ok(())
    }

    fn interrupts(&mut self, _now: Cycle) -> u32 {
        if self.high { LINE2 } else { 0 }
    }

    fn next_event(&mut self, _now: Cycle) -> Option<Cycle> {
        None
    }

    fn yields(&self) -> bool {
        self.yielding
    }

    fn access_moves_lines(&self, addr: u32) -> bool {
        addr == DEVICE
    }

    fn fetch_held(&self) -> bool {
        self.held
    }
}

/// `1: addi a2, a2, -1; s32i a2, a3, 0; moveqz a3, a6, a2; addi a9, a9, 1; addi a9, a9, 1;
/// j 1b`. The fourth pass stores to [`DEVICE`], which can move the lines, so the run ends after
/// the store, and the next step sees what it did.
#[test]
fn a_store_that_can_move_the_lines_ends_the_run() {
    let code = "22c2ff22630020368392c90192c90146fbff";
    let mut core = looping(WOE, LINE2);
    core.set_a(2, 3);
    core.set_a(3, RAM);
    core.set_a(6, DEVICE);
    let at = CODE + 6;

    let ran = same(&core, || Gate::new(code, Effect::Line), 0..0, false);
    assert_eq!(ran.core.pc(), LEVEL2);
    assert_eq!(ran.core.special(Sr::Epc2), at);
    assert_eq!(ran.core.a(9), 6);

    let ran = same(&core, || Gate::new(code, Effect::Yield), 0..0, false);
    assert_eq!(ran.stop, Stop::Yield);
    assert_eq!(ran.core.pc(), at);

    let ran = same(&core, || Gate::new(code, Effect::Hold), 0..0, false);
    assert_eq!(ran.stop, Stop::Bus);
    assert_eq!(ran.core.pc(), at);
}

/// `addi a3, a3, 1` sits word-aligned at `CODE + 8`. Each of six passes stores `a5` to RAM,
/// except the third, which stores it over that word from inside a straight run, so the op
/// there becomes `addi a3, a3, 16`. The store moves the code version, so the run stops finding
/// kept ops. The fourth pass runs the new op from a fetch, and the last two from its decode.
///
/// ```text
///     movi a7, 0; movi a2, 0; mov.n a2, a2
/// 2:  addi a3, a3, 1; addi a9, a9, -1; mov.n a6, a11; moveqz a6, a10, a9; s32i a5, a6, 0
///     addi a7, a7, 1; blti a7, 6, 2b
/// 1:  j 1b
/// ```
#[test]
fn a_store_over_a_kept_op_runs_the_new_op() {
    let code = "72a00022a0002d0232c30192c9ff6d0b906a8352660072c701a667eb06ffff";
    let mut core = looping(WOE, 0);
    core.set_a(5, 0x9210_c332);
    core.set_a(9, 3);
    core.set_a(10, CODE + 8);
    core.set_a(11, RAM);
    let make = || Gate::new(code, Effect::Line);
    let ran = same(&core, make, CODE..CODE + 4096, true);
    assert_eq!(ran.core.a(3), 3 + 3 * 16);
    assert_eq!(ran.core.pc(), CODE + 0x1c);
}

/// While `ICOUNT` counts, every op is a full step, and the debug exception lands on the same op.
/// In the loop below the fourth pass's `wsr.icountlevel` starts the count from inside a straight
/// run, so the run ends after it.
///
/// ```text
/// 1: wsr.icountlevel a6; addi a2, a2, 1; addi a2, a2, 1; addi a3, a3, -1; moveqz a6, a7, a3
///    j 1b
/// ```
#[test]
fn icount_is_counted_on_the_same_ops() {
    // 1: addi a2, a2, 1; j 1b
    let mut core = looping(WOE, 0);
    core.set_special(Sr::IcountLevel, 1);
    core.set_special(Sr::Icount, -10i32 as u32);
    let limit = Cycle(1000);
    let plain = run(
        core.clone(),
        Straight::plain(Flat::new("22c20146feff")),
        limit,
    );
    let kept = Straight::kept(Flat::new("22c20146feff"), 0..0, true);
    let kept = run(core, kept, limit);
    assert_eq!(kept.stop, plain.stop);
    assert_eq!(kept.core, plain.core);
    assert_eq!(kept.steps, plain.steps, "no straight run while it counts");
    assert_eq!(kept.core.pc(), DEBUG);

    let code = "60ed1322c20122c20132c3ff30678346fbff";
    let mut core = looping(WOE, 0);
    core.set_special(Sr::Icount, -3i32 as u32);
    core.set_a(3, 3);
    core.set_a(7, 1);
    let ran = same(&core, flat(code), 0..0, true);
    assert_eq!(ran.core.pc(), DEBUG);
    assert_eq!(ran.core.special(Sr::Epc6), CODE + 9);
    assert_eq!(ran.core.a(2), 3 * 2 + 2);
}

/// The sleeper's loop, `WAITI`, timer and edge interrupts, run kept, equal one plain run, cut
/// at random limits or not.
#[test]
fn the_sleeper_runs_the_same_straight() {
    let end = Cycle(20_000);
    let (core, bus) = sleeper();
    let mut plain = Straight::plain(bus.clone());
    let mut whole = core.clone();
    assert_eq!(
        run_in_pieces(&mut whole, &mut plain, end, &mut None),
        Stop::Limit
    );
    for seed in [None, Some(1), Some(2), Some(3)] {
        let mut kept = Straight::kept(bus.clone(), 0..0, true);
        let mut split = core.clone();
        let stop = run_in_pieces(&mut split, &mut kept, end, &mut seed.map(Pieces::new));
        assert_eq!(stop, Stop::Limit, "seed {seed:?}");
        assert_eq!(split, whole, "seed {seed:?}");
        assert_eq!(kept.trace, plain.trace, "seed {seed:?}");
    }
}

/// Each classic probe prints the same text, stops alike, ends in the same state and makes the
/// same loads and stores on the same cycles, plain and kept.
#[test]
fn the_classic_probes_run_the_same_straight() {
    for probe in ["alu", "icount", "window", "loop", "fpu", "mac", "ccount"] {
        let elf = Elf::parse(&read_fixture(&format!("{probe}.elf")));
        let limit = Cycle(1_000_000_000);
        let load = || board::Board::load(&elf, &[]);
        let plain = board::run_probe_on(Straight::plain(load()), &elf, probe, limit, None);
        let kept = board::run_probe_on(load().kept(), &elf, probe, limit, None);
        assert_eq!(kept.uart, plain.uart, "{probe}");
        assert_eq!(kept.stop, plain.stop, "{probe}");
        assert_eq!(kept.core, plain.core, "{probe}");
        assert_eq!(kept.trace, plain.trace, "{probe}");
        #[cfg(not(feature = "poll-skip-check"))]
        assert!(kept.steps < plain.steps, "{probe}");
        println!(
            "{probe}: {} full steps kept, {} plain, to cycle {}",
            kept.steps,
            plain.steps,
            kept.core.cycle().0
        );
    }
}

/// The same for each S3 probe. `cop` carries out the `ee.zero.q` its handler reruns.
#[test]
fn the_s3_probes_run_the_same_straight() {
    let probes = [
        "ccount",
        "cop",
        "fpu",
        "mac",
        "salt",
        "unaligned",
        "icounthold",
    ];
    for probe in probes {
        let elf = Elf::parse(&read_fixture(&format!("s3-{probe}.elf")));
        let limit = Cycle(20_000_000);
        let answer = |core: &mut Core<Esp32s3>, stop: orbe_xtensa::Unimplemented| {
            if stop.missing != Missing::Op(Op::EE_ZERO_Q) {
                return false;
            }
            let Some(Decoded::Insn(insn)) = Esp32s3::decode(elf.bytes_at(stop.pc, 3), stop.pc)
            else {
                return false;
            };
            core.set_q(insn.fields[0], 0);
            core.set_pc(stop.pc + u32::from(insn.len));
            true
        };
        let load = || s3board::Board::load(&elf, &[]);
        let plain = s3board::run_answering_on(Straight::plain(load()), &elf, probe, limit, answer);
        let kept = s3board::run_answering_on(load().kept(), &elf, probe, limit, answer);
        assert_eq!(kept.uart, plain.uart, "{probe}");
        assert_eq!(kept.stop, plain.stop, "{probe}");
        assert_eq!(kept.core, plain.core, "{probe}");
        assert_eq!(kept.trace, plain.trace, "{probe}");
        #[cfg(not(feature = "poll-skip-check"))]
        assert!(kept.steps < plain.steps, "{probe}");
        println!(
            "s3 {probe}: {} full steps kept, {} plain, to cycle {}",
            kept.steps,
            plain.steps,
            kept.core.cycle().0
        );
    }
}
