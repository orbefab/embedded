//! Loops whose only moving input is a counter the bus latches, as Arduino's `millis()` reads
//! SYSTIMER. Inside a straight run the core may skip the passes such a loop makes while it
//! waits, as `Bus::sweep` proves them alike. Each test runs a program on a bus that keeps no
//! code version, so every op is a full step and nothing is skipped, and on the same bus kept.
//! Both must stop alike and leave the core, RAM, the counter and every store that is not a
//! latch alike. A skip shows as far fewer op starts.

use std::cell::Cell;

use orbe_xtensa::{Bus, Config, Core, Cycle, Esp32s3, Exit, PassAccess, Sr, Stop, Swept, Width};

use crate::common::{Pieces, run_in_pieces};
use crate::core::{CODE, Flat, RAM, core_at};

/// The counter's block: a store of `UPDATE` to `OP` latches the count, `OP` then reads
/// `VALID`, and `HI` and `LO` read the latched count.
const TIMER: u32 = 0x6002_3000;
const OP: u32 = TIMER + 0x04;
const HI: u32 = TIMER + 0x40;
const LO: u32 = TIMER + 0x44;
const UPDATE: u32 = 1 << 30;
const VALID: u32 = 1 << 29;
/// Cycles per count.
const DIV: u64 = 15;

const LINE2: u32 = 1 << 19;
const TIMER0: u32 = 1 << 6;
const LEVEL2: u32 = 0x4020_0180;
const KERNEL: u32 = 0x4020_0300;
const WOE: u32 = 1 << 18;

/// The flat bus with the counter.
#[derive(Clone)]
struct Timed {
    flat: Flat,
    /// Whether it keeps a code version, so the core runs straight runs and may skip.
    kept: bool,
    /// The count at cycle 0.
    base: u64,
    latched: u64,
    valid: bool,
    /// Each store that is not a latch: its cycle, address and value.
    stores: Vec<(u64, u32, u32)>,
    /// The asks whether to yield, one for each full step.
    asks: Cell<u64>,
    /// The starts the core marks without a poll.
    begins: u64,
}

impl Timed {
    fn new(code: &str, kept: bool) -> Timed {
        Timed {
            flat: Flat::new(code),
            kept,
            base: 0,
            latched: 0,
            valid: false,
            stores: Vec::new(),
            asks: Cell::new(0),
            begins: 0,
        }
    }

    fn count(&self, at: u64) -> u64 {
        self.base + at / DIV
    }

    /// What a load of the counter's word `word` reads after a latch of `latched`.
    fn word(word: u32, latched: u64) -> u32 {
        if word == HI {
            (latched >> 32) as u32
        } else {
            latched as u32
        }
    }
}

impl Bus for Timed {
    fn fetch(&mut self, now: Cycle, addr: u32) -> Result<u32, Exit> {
        self.flat.fetch(now, addr)
    }

    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        match addr {
            OP => Ok(if self.valid { VALID } else { 0 }),
            HI | LO => Ok(Timed::word(addr, self.latched)),
            _ => self.flat.load(now, addr, width),
        }
    }

    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        if addr == OP && value & UPDATE != 0 {
            self.latched = self.count(now.0);
            self.valid = true;
            return Ok(());
        }
        self.stores.push((now.0, addr, value));
        if addr & !0xfff == TIMER {
            return Ok(());
        }
        self.flat.store(now, addr, width, value)
    }

    fn interrupts(&mut self, now: Cycle) -> u32 {
        self.flat.interrupts(now)
    }

    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        self.flat.next_event(now)
    }

    fn yields(&self) -> bool {
        self.asks.set(self.asks.get() + 1);
        false
    }

    fn begin_step(&mut self, _now: Cycle) {
        self.begins += 1;
    }

    fn access_moves_lines(&self, _addr: u32) -> bool {
        false
    }

    fn code_version(&self) -> Option<u64> {
        self.kept.then_some(0)
    }

    /// A pass may load RAM, latch the counter and load what it latched, and store nothing
    /// else. A load of the counter before the pass's first latch is refused.
    fn sweep(
        &self,
        accesses: &[PassAccess],
        start: Cycle,
        period: u64,
        passes: u64,
        out: &mut [Swept],
    ) -> bool {
        let last = start.0 + (passes - 1) * period;
        let mut latch: Option<(u64, u64)> = None;
        let mut latches = 0;
        let mut loaded: Vec<(u32, u32, u16)> = Vec::new();
        for (i, a) in accesses.iter().enumerate() {
            if let Some(value) = a.store {
                if a.addr != OP || value & UPDATE == 0 || a.width != Width::Word {
                    return false;
                }
                latch = Some((self.count(start.0 + a.offset), self.count(last + a.offset)));
                latches += 1;
                continue;
            }
            out[i] = match a.addr {
                OP => {
                    let Some(_) = latch else { return false };
                    Swept {
                        lo: VALID,
                        hi: VALID,
                        same_as: None,
                    }
                }
                HI | LO => {
                    let Some((first, last)) = latch else {
                        return false;
                    };
                    let (lo, hi) = if a.addr == HI || first >> 32 == last >> 32 {
                        (Timed::word(a.addr, first), Timed::word(a.addr, last))
                    } else {
                        (0, u32::MAX)
                    };
                    let same_as = loaded
                        .iter()
                        .find(|&&(w, n, _)| w == a.addr && n == latches)
                        .map(|&(.., j)| j);
                    if same_as.is_none() {
                        loaded.push((a.addr, latches, i as u16));
                    }
                    Swept { lo, hi, same_as }
                }
                addr if self.flat.ram.contains(addr) => {
                    let v = self.flat.ram.read(addr, a.width);
                    Swept {
                        lo: v,
                        hi: v,
                        same_as: None,
                    }
                }
                _ => return false,
            };
        }
        true
    }
}

fn s32i(t: u8, s: u8, offset: u32) -> [u8; 3] {
    [t << 4 | 2, 0x60 | s, (offset / 4) as u8]
}

fn l32i(t: u8, s: u8, offset: u32) -> [u8; 3] {
    [t << 4 | 2, 0x20 | s, (offset / 4) as u8]
}

fn addi(t: u8, s: u8, imm: i8) -> [u8; 3] {
    [t << 4 | 2, 0xc0 | s, imm as u8]
}

fn rrr(op2: u8, op1: u8, r: u8, s: u8, t: u8) -> [u8; 3] {
    [t << 4, r << 4 | s, op2 << 4 | op1]
}

fn add(r: u8, s: u8, t: u8) -> [u8; 3] {
    rrr(8, 0, r, s, t)
}

fn sub(r: u8, s: u8, t: u8) -> [u8; 3] {
    rrr(0xc, 0, r, s, t)
}

fn xor(r: u8, s: u8, t: u8) -> [u8; 3] {
    rrr(3, 0, r, s, t)
}

fn src(r: u8, s: u8, t: u8) -> [u8; 3] {
    rrr(8, 1, r, s, t)
}

fn quou(r: u8, s: u8, t: u8) -> [u8; 3] {
    rrr(0xc, 2, r, s, t)
}

fn ssai(sa: u8) -> [u8; 3] {
    rrr(4, 0, 4, sa & 15, sa >> 4)
}

const NOP: [u8; 3] = [0xf0, 0x20, 0x00];

const BLTU: u8 = 0x3;

/// A branch on `as` and `at` at op index `at` of a program of 3-byte ops, to op index `to`.
fn branch(cond: u8, s: u8, t: u8, at: usize, to: usize) -> [u8; 3] {
    let imm = 3 * to as i32 - 3 * at as i32 - 4;
    [t << 4 | 7, cond << 4 | s, i8::try_from(imm).unwrap() as u8]
}

/// `bbci as, bit` at op index `at`, to op index `to`.
fn bbci(s: u8, bit: u8, at: usize, to: usize) -> [u8; 3] {
    let imm = 3 * to as i32 - 3 * at as i32 - 4;
    [
        (bit & 15) << 4 | 7,
        (6 | bit >> 4) << 4 | s,
        i8::try_from(imm).unwrap() as u8,
    ]
}

/// `j` at op index `at`, to op index `to`.
fn j(at: usize, to: usize) -> [u8; 3] {
    let offset = (3 * to as i32 - 3 * at as i32 - 4) as u32 & 0x3_ffff;
    let word = offset << 6 | 6;
    [word as u8, (word >> 8) as u8, (word >> 16) as u8]
}

/// `loop as` at op index `at`, whose body ends before op index `end`.
fn lp(s: u8, at: usize, end: usize) -> [u8; 3] {
    let imm = 3 * end as i32 - 3 * at as i32 - 4;
    [0x76, 0x80 | s, u8::try_from(imm).unwrap()]
}

fn hex(ops: &[[u8; 3]]) -> String {
    ops.iter().flatten().map(|b| format!("{b:02x}")).collect()
}

fn listing(ops: &[[u8; 3]]) -> Vec<String> {
    ops.iter()
        .enumerate()
        .map(|(i, op)| {
            let pc = CODE + 3 * i as u32;
            Esp32s3::render(&Esp32s3::decode(op, pc).expect("an op"))
        })
        .collect()
}

/// Registers the program reads and never writes: `a3` the counter's block, `a4` RAM, `a5` the
/// counts between toggles, `a6` `UPDATE`, `a13` a divisor, `a15` a loop count.
fn poller() -> Core<Esp32s3> {
    let mut core = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_a(3, TIMER);
    core.set_a(4, RAM);
    core.set_a(5, 20);
    core.set_a(6, UPDATE);
    core.set_a(13, 3);
    core.set_a(15, 3);
    core
}

/// What reads the counter: `millis()`'s latch, the `VALID` poll, `LO` and `HI`, and the count
/// shifted down by 4.
fn read_count() -> Vec<[u8; 3]> {
    vec![
        s32i(6, 3, 4),
        l32i(7, 3, 4),
        bbci(7, 29, 2, 1),
        l32i(8, 3, 0x44),
        l32i(9, 3, 0x40),
        ssai(4),
        src(8, 9, 8),
    ]
}

/// `loop()`: read the count, and while it is less than `a5` past `last`, go back. Otherwise
/// move `last` on by `a5`, count a toggle and go back. `extra` sits between the read and the
/// test.
fn program(extra: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let mut ops = read_count();
    ops.extend_from_slice(extra);
    let test = ops.len() + 2;
    ops.extend([l32i(10, 4, 0), sub(11, 8, 10), branch(BLTU, 11, 5, test, 0)]);
    let back = ops.len() + 5;
    ops.extend([
        add(10, 10, 5),
        s32i(10, 4, 0),
        l32i(12, 4, 4),
        addi(12, 12, 1),
        s32i(12, 4, 4),
        j(back, 0),
    ]);
    ops
}

struct Ran {
    stop: Stop,
    core: Core<Esp32s3>,
    bus: Timed,
}

fn run(core: &Core<Esp32s3>, bus: &Timed, limit: Cycle, seed: Option<u64>) -> Ran {
    let (mut core, mut bus) = (core.clone(), bus.clone());
    let mut pieces = seed.map(Pieces::new);
    let stop = run_in_pieces(&mut core, &mut bus, limit, &mut pieces);
    Ran { stop, core, bus }
}

/// Runs `core` on `ops` to `limit` stepped and kept, requires both runs to match, and returns
/// both, stepped first.
fn both(core: &Core<Esp32s3>, bus: &Timed, limit: Cycle, seed: Option<u64>) -> (Ran, Ran) {
    let plain = run(
        core,
        &Timed {
            kept: false,
            ..bus.clone()
        },
        limit,
        seed,
    );
    let kept = run(
        core,
        &Timed {
            kept: true,
            ..bus.clone()
        },
        limit,
        seed,
    );
    let at = format!("limit {limit:?} seed {seed:?}");
    assert_eq!(kept.stop, plain.stop, "the stop, {at}");
    assert_eq!(kept.core, plain.core, "the core, {at}");
    assert_eq!(kept.bus.stores, plain.bus.stores, "the stores, {at}");
    assert_eq!(
        (kept.bus.latched, kept.bus.valid),
        (plain.bus.latched, plain.bus.valid),
        "the latch, {at}"
    );
    for offset in (0..16).step_by(4) {
        let addr = RAM + offset;
        assert_eq!(
            kept.bus.flat.ram.read(addr, Width::Word),
            plain.bus.flat.ram.read(addr, Width::Word),
            "RAM at {addr:#x}, {at}"
        );
    }
    (plain, kept)
}

impl Ran {
    /// The ops a stepped run ran: one ask whether to yield before each.
    fn ops(&self) -> u64 {
        self.bus.asks.get()
    }

    /// The op starts a kept run marked: a full step asks whether to yield, an op of a straight
    /// run marks its start, or in a `poll-skip-check` build asks whether to yield. A skipped
    /// pass marks none.
    fn starts(&self) -> u64 {
        self.bus.asks.get() + self.bus.begins
    }
}

/// A kept run that skips marks a third of the stepped run's op starts or fewer. A check build
/// runs each pass it would skip, so it skips nothing.
fn skipped(plain: &Ran, kept: &Ran) -> bool {
    kept.starts() * 3 < plain.ops() || cfg!(feature = "pass-skip-check")
}

fn ran_each_op(plain: &Ran, kept: &Ran) -> bool {
    kept.starts() * 10 >= plain.ops() * 9
}

/// Sets `last` to what the loop reads at the start when the count starts at `base` and the
/// loop divides it by `divisor` after shifting it down by 4.
fn start_at(bus: &mut Timed, base: u64, divisor: u32) {
    bus.base = base;
    let now = (base >> 4) as u32 / divisor;
    bus.flat.ram.write(RAM, Width::Word, now);
}

/// The toggles the program counted.
fn toggles(ran: &Ran) -> u32 {
    ran.bus.flat.ram.read(RAM + 4, Width::Word)
}

#[test]
fn the_programs_are_the_ops_they_name() {
    assert_eq!(
        listing(&program(&[])),
        [
            "s32i\ta6, a3, 4",
            "l32i\ta7, a3, 4",
            "bbci\ta7, 29, 0x40100003",
            "l32i\ta8, a3, 68",
            "l32i\ta9, a3, 64",
            "ssai\t4",
            "src\ta8, a9, a8",
            "l32i\ta10, a4, 0",
            "sub\ta11, a8, a10",
            "bltu\ta11, a5, 0x40100000",
            "add\ta10, a10, a5",
            "s32i\ta10, a4, 0",
            "l32i\ta12, a4, 4",
            "addi\ta12, a12, 1",
            "s32i\ta12, a4, 4",
            "j\t0x40100000",
        ]
    );
    let looped = program(&[lp(15, 7, 9), NOP]);
    assert_eq!(listing(&looped)[7], "loop\ta15, 0x4010001b");
    assert_eq!(listing(&[quou(8, 8, 13)]), ["quou\ta8, a8, a13"]);
    assert_eq!(listing(&[xor(8, 8, 13)]), ["xor\ta8, a8, a13"]);
}

/// The loop toggles on the counts single steps toggle on, and between toggles it skips.
#[test]
fn a_polling_loop_skips_and_toggles_as_single_steps_do() {
    let bus = Timed::new(&hex(&program(&[])), false);
    let (plain, kept) = both(&poller(), &bus, Cycle(200_000), None);
    assert!(toggles(&kept) > 30, "{}", toggles(&kept));
    assert!(
        skipped(&plain, &kept),
        "{} of {}",
        kept.starts(),
        plain.ops()
    );
}

/// A run stops at every limit around a toggle, and on every split into pieces, where single
/// steps stop.
#[test]
fn a_polling_loop_stops_at_every_limit_as_single_steps_do() {
    let bus = Timed::new(&hex(&program(&[])), false);
    let (plain, _) = both(&poller(), &bus, Cycle(30_000), None);
    let toggles: Vec<u64> = plain
        .bus
        .stores
        .iter()
        .filter(|&&(_, addr, _)| addr == RAM)
        .map(|&(at, ..)| at)
        .collect();
    assert!(toggles.len() >= 3, "{toggles:?}");
    for &at in &toggles[1..3] {
        for limit in at - 300..at + 60 {
            both(&poller(), &bus, Cycle(limit), None);
        }
    }
    for seed in 1..40 {
        both(&poller(), &bus, Cycle(60_000), Some(seed));
    }
}

/// A level-2 line that rises at each cycle of several passes while the loop waits is taken on
/// the step single steps take it at, with the registers of the pass it cut.
#[test]
fn a_line_rising_at_each_cycle_of_a_pass_is_taken_alike() {
    let mut core = poller();
    core.set_special(Sr::IntEnable, LINE2);
    let mut bus = Timed::new(&hex(&program(&[])), false);
    for at in 20_000..20_040 {
        bus.flat.rise = Some((Cycle(at), LINE2));
        let (plain, kept) = both(&core, &bus, Cycle(100_000), None);
        assert_eq!(kept.core.pc(), LEVEL2, "at {at}");
        assert!(
            skipped(&plain, &kept),
            "at {at}: {} of {}",
            kept.starts(),
            plain.ops()
        );
    }
}

/// A `CCOMPARE0` match at each cycle of several passes while the loop waits is taken alike.
#[test]
fn a_timer_match_at_each_cycle_of_a_pass_is_taken_alike() {
    let bus = Timed::new(&hex(&program(&[])), false);
    for at in 30_000..30_040 {
        let mut core = poller();
        core.set_special(Sr::IntEnable, TIMER0);
        core.set_special(Sr::Ccompare0, at);
        let (plain, kept) = both(&core, &bus, Cycle(100_000), None);
        assert_eq!(kept.core.pc(), KERNEL, "at {at}");
        assert!(
            skipped(&plain, &kept),
            "at {at}: {} of {}",
            kept.starts(),
            plain.ops()
        );
    }
}

/// The count's low word turns over while the loop waits, and `HI` moves on: the loop reads the
/// pair as single steps do.
#[test]
fn a_count_across_a_low_word_carry() {
    for before in [1, 100, 1_000, 5_000] {
        let mut bus = Timed::new(&hex(&program(&[])), false);
        start_at(&mut bus, (1 << 32) - before, 1);
        let (plain, kept) = both(&poller(), &bus, Cycle(200_000), None);
        assert!(kept.bus.latched >> 32 == 1, "{:#x}", kept.bus.latched);
        assert!(toggles(&kept) > 30, "{before}: {}", toggles(&kept));
        assert!(
            skipped(&plain, &kept),
            "{before} before the carry: {} of {}",
            kept.starts(),
            plain.ops()
        );
    }
}

/// Two divides back to back, the second at the cost its dividend's bits give: a skip stops
/// before the dividend gains a bit. The second dividend crosses 2^17, a step of that cost, in
/// the second run.
#[test]
fn a_divide_after_a_divide_keeps_its_cost() {
    let program = program(&[quou(8, 8, 13), quou(8, 8, 13)]);
    for base in [0, 48 * (1 << 17) - 6_000, (1 << 32) - 3_000] {
        let mut bus = Timed::new(&hex(&program), false);
        start_at(&mut bus, base, 9);
        let mut core = poller();
        core.set_a(5, 1);
        let (plain, kept) = both(&core, &bus, Cycle(200_000), None);
        assert!(
            skipped(&plain, &kept),
            "base {base:#x}: {} of {}",
            kept.starts(),
            plain.ops()
        );
    }
}

/// A pass that stores, that holds an op with no rule for a moving input, that counts in a
/// register or that holds a zero-overhead loop runs op by op, as single steps run it.
#[test]
fn a_pass_the_skip_refuses_runs_each_op() {
    let refused = [
        ("a store", vec![s32i(8, 4, 8)]),
        (
            "an op with no rule",
            vec![l32i(14, 3, 0x44), xor(14, 14, 13)],
        ),
        ("a count in a register", vec![addi(14, 14, 1)]),
        ("a loop", vec![lp(15, 7, 9), NOP]),
    ];
    for (name, extra) in refused {
        let bus = Timed::new(&hex(&program(&extra)), false);
        let (plain, kept) = both(&poller(), &bus, Cycle(100_000), None);
        assert!(
            ran_each_op(&plain, &kept),
            "{name}: {} of {}",
            kept.starts(),
            plain.ops()
        );
        for seed in 1..6 {
            both(&poller(), &bus, Cycle(40_000), Some(seed));
        }
    }
}
