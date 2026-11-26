//! The passes of a loop whose only input that moves from pass to pass is what its loads return,
//! such as an Arduino `loop()` that polls `millis()` until enough time has passed.
//!
//! When a straight run jumps or branches back, the core may record the next pass as it runs
//! it: each op, the registers it reads and writes, and its data accesses with their cycles. The
//! pass must come back to the same head through the same branch, with the same special
//! registers, pace and live-in registers, the registers it reads before it writes them. The
//! bus then says, for the next `n` passes, the range each load can return ([`Bus::sweep`]).
//! Over those ranges the core evaluates the recorded ops with intervals. When, for every value
//! in range, every branch goes the way it went, every op costs what it cost and every live-in
//! the pass writes ends as it started, each of the `n` passes runs the recorded path in the
//! recorded cycles from the same live-ins. The core then moves the cycle over the first `n - 1`
//! of them and runs the last, which writes every register a pass writes before anything reads
//! it, and makes the accesses that leave the bus as the `n` passes would.
//!
//! An op takes part only when this module knows the registers it reads and writes and, unless
//! its inputs are the recorded ones, how its result moves with them. A load or a store must be
//! one aligned access whose address is the recorded one. Anything else leaves the loop to run
//! op by op, and a loop that never skipped stops being looked at after a few tries. A check
//! build runs each pass it would skip and requires it to take the recorded path, with each
//! load in its range, and the core to match the skip.

use super::decodes::{Kept, Taken};
use super::{Core, PACE_DIVIDE, Stop};
use crate::bus::{Bus, Cycle, Exit, PassAccess, Swept, Width};
use crate::config::Config;
use crate::decode::{Insn, Op};
use crate::special::Sr;

/// The most ops a pass holds.
const MOST_OPS: usize = 512;
/// The fewest cycles before the straight run's horizon at which the core looks for a pass. With
/// less room a skip saves less than the look costs.
const ROOM: u64 = 8192;
/// The first and the longest wait, as powers of two cycles, before a head whose pass did not
/// skip is looked at again.
const FIRST_WAIT: u8 = 12;
const LONGEST_WAIT: u8 = 22;
/// Looks after which a head that never skipped is not looked at again at its code version.
const MOST_REFUSALS: u8 = 4;
const WAITS: usize = 256;

/// Whether `insn` at `pc` jumps or branches back, so it may close a loop's pass.
pub(super) fn loops(insn: &Insn, pc: u32) -> bool {
    target(insn).is_some_and(|to| to <= pc)
}

fn target(insn: &Insn) -> Option<u32> {
    let f = &insn.fields;
    match insn.op {
        Op::J => Some(f[0]),
        Op::BEQZ | Op::BEQZ_N | Op::BNEZ | Op::BNEZ_N | Op::BGEZ | Op::BLTZ => Some(f[1]),
        Op::BEQ
        | Op::BNE
        | Op::BGE
        | Op::BLT
        | Op::BGEU
        | Op::BLTU
        | Op::BANY
        | Op::BNONE
        | Op::BALL
        | Op::BNALL
        | Op::BBC
        | Op::BBS
        | Op::BBCI
        | Op::BBSI
        | Op::BEQI
        | Op::BNEI
        | Op::BGEI
        | Op::BLTI
        | Op::BGEUI
        | Op::BLTUI => Some(f[2]),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Computes its write from its reads.
    Plain,
    Load,
    Store,
    Branch,
    /// A call, `entry`, return, jump or barrier: its reads must be the recorded ones.
    Fixed,
}

/// The address registers an op names, by number in its window, and whether it reads or writes
/// `SAR`.
#[derive(Clone, Copy, Debug)]
struct Shape {
    reads: [Option<u32>; 2],
    write: Option<u32>,
    /// The write lands in the window after the op, as `entry`'s does.
    write_after: bool,
    sar_read: bool,
    sar_write: bool,
    kind: Kind,
}

fn shape(insn: &Insn) -> Option<Shape> {
    let f = &insn.fields;
    let mut s = Shape {
        reads: [None; 2],
        write: None,
        write_after: false,
        sar_read: false,
        sar_write: false,
        kind: Kind::Plain,
    };
    match insn.op {
        Op::MOVI | Op::MOVI_N => s.write = Some(f[0]),
        Op::MOV_N
        | Op::ADDI
        | Op::ADDI_N
        | Op::ADDMI
        | Op::NEG
        | Op::SLLI
        | Op::SRLI
        | Op::EXTUI
        | Op::NSAU => {
            s.reads[0] = Some(f[1]);
            s.write = Some(f[0]);
        }
        Op::OR
        | Op::AND
        | Op::XOR
        | Op::ADD
        | Op::ADD_N
        | Op::SUB
        | Op::ADDX2
        | Op::ADDX4
        | Op::ADDX8
        | Op::MULL
        | Op::MULUH
        | Op::SALTU
        | Op::SALT
        | Op::QUOU
        | Op::REMU
        | Op::MINU
        | Op::MAXU => {
            s.reads = [Some(f[1]), Some(f[2])];
            s.write = Some(f[0]);
        }
        Op::SLL | Op::SRL => {
            s.reads[0] = Some(f[1]);
            s.sar_read = true;
            s.write = Some(f[0]);
        }
        Op::SRC => {
            s.reads = [Some(f[1]), Some(f[2])];
            s.sar_read = true;
            s.write = Some(f[0]);
        }
        Op::SSAI => s.sar_write = true,
        Op::SSL | Op::SSR => {
            s.reads[0] = Some(f[0]);
            s.sar_write = true;
        }
        Op::L32R => {
            s.write = Some(f[0]);
            s.kind = Kind::Load;
        }
        Op::L32I | Op::L32I_N | Op::L8UI | Op::L16UI => {
            s.reads[0] = Some(f[1]);
            s.write = Some(f[0]);
            s.kind = Kind::Load;
        }
        Op::S32I | Op::S32I_N | Op::S8I | Op::S16I => {
            s.reads = [Some(f[0]), Some(f[1])];
            s.kind = Kind::Store;
        }
        Op::MEMW | Op::NOP | Op::NOP_N | Op::EXTW | Op::J => s.kind = Kind::Fixed,
        Op::BEQZ | Op::BEQZ_N | Op::BNEZ | Op::BNEZ_N | Op::BGEZ | Op::BLTZ => {
            s.reads[0] = Some(f[0]);
            s.kind = Kind::Branch;
        }
        Op::BEQ | Op::BNE | Op::BGEU | Op::BLTU | Op::BGE | Op::BLT => {
            s.reads = [Some(f[0]), Some(f[1])];
            s.kind = Kind::Branch;
        }
        Op::BBCI | Op::BBSI | Op::BEQI | Op::BNEI | Op::BGEUI | Op::BLTUI | Op::BGEI | Op::BLTI => {
            s.reads[0] = Some(f[0]);
            s.kind = Kind::Branch;
        }
        Op::JX => {
            s.reads[0] = Some(f[0]);
            s.kind = Kind::Fixed;
        }
        Op::CALL4 | Op::CALL8 | Op::CALL12 => {
            s.write = Some(call_write(insn.op));
            s.kind = Kind::Fixed;
        }
        Op::CALLX4 | Op::CALLX8 | Op::CALLX12 => {
            s.reads[0] = Some(f[0]);
            s.write = Some(call_write(insn.op));
            s.kind = Kind::Fixed;
        }
        Op::ENTRY => {
            s.reads[0] = Some(f[0]);
            s.write = Some(f[0]);
            s.write_after = true;
            s.kind = Kind::Fixed;
        }
        Op::RETW | Op::RETW_N => {
            s.reads[0] = Some(0);
            s.kind = Kind::Fixed;
        }
        _ => return None,
    }
    Some(s)
}

/// The register a windowed call writes its return address to.
fn call_write(op: Op) -> u32 {
    match op {
        Op::CALL4 | Op::CALLX4 => 4,
        Op::CALL8 | Op::CALLX8 => 8,
        _ => 12,
    }
}

/// One recorded op.
struct Step {
    slot: usize,
    shape: Shape,
    /// The physical registers it read and their values.
    reads: [(u8, u32); 2],
    /// `SAR` as the op found it.
    sar: u32,
    /// The physical register it wrote and the value.
    write: Option<(u8, u32)>,
    /// `SAR` as the op left it.
    sar_after: u32,
    taken: bool,
    /// The op's start, from the pass's start, as the straight run checks it.
    start: u64,
    /// The op before it was a divide.
    follows: bool,
    /// Its one data access, an index into the pass's accesses.
    access: Option<usize>,
}

/// A recorded pass.
struct Pass {
    steps: Vec<Step>,
    accesses: Vec<PassAccess>,
    period: u64,
    /// The physical registers the pass reads before it writes them.
    live: u64,
    /// The physical registers the pass writes.
    written: u64,
    sar_live: bool,
}

/// A bus that records each load and store a pass makes, with its cycle from `start` and the
/// value it carried. Every other call goes to the bus it wraps as it is, so a pass runs on it
/// exactly as on that bus.
struct Recording<'a, B> {
    bus: &'a mut B,
    start: u64,
    accesses: Vec<PassAccess>,
    /// What each load returned, or what each store wrote.
    values: Vec<u32>,
}

impl<'a, B: Bus> Recording<'a, B> {
    fn new(bus: &'a mut B, start: Cycle) -> Self {
        Recording {
            bus,
            start: start.0,
            accesses: Vec::new(),
            values: Vec::new(),
        }
    }
}

impl<B: Bus> Bus for Recording<'_, B> {
    fn fetch(&mut self, now: Cycle, addr: u32) -> Result<u32, Exit> {
        self.bus.fetch(now, addr)
    }

    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        let value = self.bus.load(now, addr, width)?;
        self.accesses.push(PassAccess {
            addr,
            width,
            store: None,
            offset: now.0 - self.start,
        });
        self.values.push(value);
        Ok(value)
    }

    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        self.bus.store(now, addr, width, value)?;
        self.accesses.push(PassAccess {
            addr,
            width,
            store: Some(value),
            offset: now.0 - self.start,
        });
        self.values.push(value);
        Ok(())
    }

    fn check_store(&mut self, now: Cycle, addr: u32) -> Result<(), Exit> {
        self.bus.check_store(now, addr)
    }

    fn instruction_bus(&self, addr: u32) -> bool {
        self.bus.instruction_bus(addr)
    }

    fn interrupts(&mut self, now: Cycle) -> u32 {
        self.bus.interrupts(now)
    }

    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        self.bus.next_event(now)
    }

    fn yields(&self) -> bool {
        self.bus.yields()
    }

    fn begin_step(&mut self, now: Cycle) {
        self.bus.begin_step(now);
    }

    fn access_moves_lines(&self, addr: u32) -> bool {
        self.bus.access_moves_lines(addr)
    }

    fn load_moves_lines(&self, addr: u32) -> bool {
        self.bus.load_moves_lines(addr)
    }

    fn code_version(&self) -> Option<u64> {
        self.bus.code_version()
    }

    fn fetch_held(&self) -> bool {
        self.bus.fetch_held()
    }

    fn sweep(
        &self,
        accesses: &[PassAccess],
        start: Cycle,
        period: u64,
        passes: u64,
        out: &mut [Swept],
    ) -> bool {
        self.bus.sweep(accesses, start, period, passes, out)
    }
}

/// The values a register holds across the passes: each lies in `lo..=hi` and has at least `tz`
/// low zero bits, and when `load` is set, it is what that load of the pass returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Val {
    lo: u32,
    hi: u32,
    tz: u32,
    load: Option<u16>,
}

const ANY: Val = Val {
    lo: 0,
    hi: u32::MAX,
    tz: 0,
    load: None,
};

impl Val {
    fn known(x: u32) -> Val {
        Val {
            lo: x,
            hi: x,
            tz: x.trailing_zeros(),
            load: None,
        }
    }

    /// `lo..=hi`, every value with at least `tz` low zero bits.
    fn range(lo: u32, hi: u32, tz: u32) -> Val {
        debug_assert!(lo <= hi, "{lo:#x} > {hi:#x}");
        if lo == hi {
            return Val::known(lo);
        }
        Val {
            lo,
            hi,
            tz: tz.min(lo.trailing_zeros()),
            load: None,
        }
    }

    fn single(self) -> Option<u32> {
        (self.lo == self.hi).then_some(self.lo)
    }

    fn shl(self, k: u32) -> Val {
        if k == 0 {
            return self;
        }
        if k >= 32 {
            return Val::known(0);
        }
        if self.lo >> (32 - k) == self.hi >> (32 - k) {
            // The bits shifted out are the same for every value, so the shift keeps order.
            Val::range(self.lo << k, self.hi << k, self.tz + k)
        } else {
            Val::range(0, u32::MAX << k, k)
        }
    }

    fn shr(self, k: u32) -> Val {
        if k == 0 {
            return self;
        }
        if k >= 32 {
            return Val::known(0);
        }
        Val::range(self.lo >> k, self.hi >> k, self.tz.saturating_sub(k))
    }

    fn add(self, other: Val) -> Val {
        if let (Some(a), Some(b)) = (self.single(), other.single()) {
            return Val::known(a.wrapping_add(b));
        }
        let lo = u64::from(self.lo) + u64::from(other.lo);
        let hi = u64::from(self.hi) + u64::from(other.hi);
        if lo >> 32 == hi >> 32 {
            // Every sum wraps alike, so the sums keep order.
            Val::range(lo as u32, hi as u32, self.tz.min(other.tz))
        } else {
            ANY
        }
    }

    fn or(self, other: Val) -> Val {
        if let (Some(a), Some(b)) = (self.single(), other.single()) {
            return Val::known(a | b);
        }
        let bits = |v: Val| 32 - v.hi.leading_zeros();
        if self.tz >= bits(other) || other.tz >= bits(self) {
            // No bit is set in both, so the or is the sum.
            return self.add(other);
        }
        ANY
    }
}

/// Whether the branch `op` with fields `f` goes for every pair of values in `a` and `b`, or
/// `None` when the ranges decide nothing.
fn decide(op: Op, f: &[u32; 8], a: Val, b: Val) -> Option<bool> {
    let ge = |a: Val, b: Val| {
        if a.lo >= b.hi {
            Some(true)
        } else if a.hi < b.lo {
            Some(false)
        } else {
            None
        }
    };
    let eq = |a: Val, b: Val| {
        if a.load.is_some() && a.load == b.load {
            return Some(true);
        }
        if let (Some(x), Some(y)) = (a.single(), b.single()) {
            return Some(x == y);
        }
        if a.hi < b.lo || b.hi < a.lo {
            Some(false)
        } else {
            None
        }
    };
    // With the sign bit flipped, a signed compare is an unsigned one. A range across the sign
    // boundary does not stay a range.
    let flip = |v: Val| -> Option<Val> {
        const SIGN: u32 = 1 << 31;
        if v.lo < SIGN && v.hi >= SIGN {
            return None;
        }
        Some(Val::range(v.lo ^ SIGN, v.hi ^ SIGN, 0))
    };
    let signed_ge = |a: Val, b: Val| ge(flip(a)?, flip(b)?);
    let k = Val::known(f[1]);
    match op {
        Op::BEQZ | Op::BEQZ_N => eq(a, Val::known(0)),
        Op::BNEZ | Op::BNEZ_N => eq(a, Val::known(0)).map(|x| !x),
        Op::BEQ => eq(a, b),
        Op::BNE => eq(a, b).map(|x| !x),
        Op::BEQI => eq(a, k),
        Op::BNEI => eq(a, k).map(|x| !x),
        Op::BGEU => ge(a, b),
        Op::BLTU => ge(a, b).map(|x| !x),
        Op::BGEUI => ge(a, k),
        Op::BLTUI => ge(a, k).map(|x| !x),
        Op::BGE => signed_ge(a, b),
        Op::BLT => signed_ge(a, b).map(|x| !x),
        Op::BGEI => signed_ge(a, k),
        Op::BLTI => signed_ge(a, k).map(|x| !x),
        Op::BGEZ => signed_ge(a, Val::known(0)),
        Op::BLTZ => signed_ge(a, Val::known(0)).map(|x| !x),
        Op::BBCI | Op::BBSI => {
            let bit = f[1];
            let clear = if a.tz > bit {
                true
            } else if a.lo >> bit == a.hi >> bit {
                // Every value shares the bits from `bit` up.
                a.lo >> bit & 1 == 0
            } else {
                return None;
            };
            Some(if op == Op::BBCI { clear } else { !clear })
        }
        _ => None,
    }
}

/// What the plain op `insn` writes when its inputs lie in `a`, `b` and `sar`, or `None` when it
/// has no rule for them.
fn compute(insn: &Insn, a: Val, b: Val, sar: Val) -> Option<Val> {
    let f = &insn.fields;
    Some(match insn.op {
        Op::MOV_N => a,
        // `mov` is `or` of a register with itself.
        Op::OR if f[1] == f[2] => a,
        Op::OR => a.or(b),
        Op::ADD | Op::ADD_N => a.add(b),
        Op::ADDI | Op::ADDI_N | Op::ADDMI => a.add(Val::known(f[2])),
        Op::SUB => {
            if a.load.is_some() && a.load == b.load {
                Val::known(0)
            } else if let Some(c) = b.single() {
                a.add(Val::known(c.wrapping_neg()))
            } else if let Some(c) = a.single() {
                if c >= b.hi {
                    Val::range(c - b.hi, c - b.lo, 0)
                } else if c < b.lo {
                    Val::range(c.wrapping_sub(b.hi), c.wrapping_sub(b.lo), 0)
                } else {
                    ANY
                }
            } else {
                ANY
            }
        }
        Op::SLLI => a.shl(f[2]),
        Op::SRLI => a.shr(f[2]),
        // The 64-bit pair `a:0` shifted right by `SAR`.
        Op::SLL => {
            let s = sar.single()?;
            if s <= 32 {
                a.shl(32 - s)
            } else {
                a.shr(s - 32)
            }
        }
        Op::SRL => a.shr(sar.single()?),
        // The 64-bit pair `a:b` shifted right by `SAR`.
        Op::SRC => {
            let s = sar.single()?;
            let high = a.single()?;
            if s == 0 {
                b
            } else if s >= 32 {
                Val::known(high >> (s - 32))
            } else {
                let top = high << (32 - s);
                let low = b.shr(s);
                Val::range(top | low.lo, top | low.hi, 0)
            }
        }
        Op::EXTUI => {
            let (s, w) = (f[2], f[3]);
            if a.tz >= s + w {
                return Some(Val::known(0));
            }
            let y = a.shr(s);
            if w >= 32 - s {
                y
            } else if y.lo >> w == y.hi >> w {
                let mask = (1u32 << w) - 1;
                Val::range(y.lo & mask, y.hi & mask, y.tz)
            } else {
                Val::range(0, (1u32 << w) - 1, 0)
            }
        }
        Op::MULL => {
            if a.single() == Some(0) || b.single() == Some(0) {
                Val::known(0)
            } else if u64::from(a.hi) * u64::from(b.hi) <= u64::from(u32::MAX) {
                Val::range(a.lo * b.lo, a.hi * b.hi, 0)
            } else {
                ANY
            }
        }
        Op::MULUH => {
            let high = |x: u32, y: u32| ((u64::from(x) * u64::from(y)) >> 32) as u32;
            Val::range(high(a.lo, b.lo), high(a.hi, b.hi), 0)
        }
        Op::SALTU | Op::SALT => {
            let less = if insn.op == Op::SALTU {
                Op::BLTU
            } else {
                Op::BLT
            };
            match decide(less, f, a, b) {
                Some(x) => Val::known(u32::from(x)),
                None => Val::range(0, 1, 0),
            }
        }
        Op::QUOU => {
            let d = b.single().filter(|&d| d != 0)?;
            Val::range(a.lo / d, a.hi / d, 0)
        }
        Op::REMU => {
            let d = b.single().filter(|&d| d != 0)?;
            if a.lo / d == a.hi / d {
                Val::range(a.lo % d, a.hi % d, 0)
            } else {
                Val::range(0, d - 1, 0)
            }
        }
        Op::NSAU => {
            let z = a.lo.leading_zeros();
            if z != a.hi.leading_zeros() {
                return None;
            }
            Val::known(z)
        }
        Op::NEG => Val::known(a.single()?.wrapping_neg()),
        _ => return None,
    })
}

/// Whether a `quou` or `remu` with dividends in `a` costs the same for each, when the op
/// before it was a divide or not as `follows` says. A longer dividend never costs less, so the
/// ends decide.
fn divide_costs_alike<C: Config>(op: Op, follows: bool, a: Val) -> bool {
    let remainder = op == Op::REMU;
    C::divide_cycles(follows, remainder, a.lo) == C::divide_cycles(follows, remainder, a.hi)
}

/// By the slot of the branch that closes a loop's pass, when the core looks at the pass again,
/// allocated at the first look. It decides only which passes are looked at, never what a run
/// computes, so a copy starts empty and two compare equal. It sits in the core beside the
/// decodes, not in them, so the decodes stay as the straight run reads them.
#[derive(Default)]
pub(super) struct Waits(Option<Box<[Wait; WAITS]>>);

#[derive(Clone, Copy, Default)]
pub(super) struct Wait {
    /// The branch's slot, plus one, or 0 for an entry no branch holds.
    branch: usize,
    /// The cycle before which the core does not look again.
    at: u64,
    /// The next wait, as a power of two cycles.
    shift: u8,
    refusals: u8,
    skipped: bool,
    /// The last look skipped. The pass after a skip that a flip ended is often the one that
    /// flips, so the look after it is refused, and the next pass is worth a look at once.
    fresh: bool,
}

impl Waits {
    fn at(&mut self, branch: usize) -> &mut Wait {
        let waits = self
            .0
            .get_or_insert_with(|| Box::new([Wait::default(); WAITS]));
        let wait = &mut waits[branch % WAITS];
        if wait.branch != branch + 1 {
            *wait = Wait {
                branch: branch + 1,
                shift: FIRST_WAIT,
                ..Wait::default()
            };
        }
        wait
    }
}

impl Clone for Waits {
    fn clone(&self) -> Self {
        Waits(None)
    }
}

impl PartialEq for Waits {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Waits {}

/// The physical register `n` of the window at `base`.
fn phys(base: u32, n: u32) -> u8 {
    ((base * 4 + n) & 63) as u8
}

impl<C: Config> Core<C> {
    /// One op of a straight run with the step before it, as [`Core::straight`] runs it. `Err`
    /// holds what the run ends with.
    fn straight_one<B: Bus>(&mut self, bus: &mut B, kept: &Kept) -> Result<(), Result<(), Stop>> {
        self.keep_step(bus);
        if self.refetch && (self.pc & 3) + u32::from(kept.insn.len) > 4 {
            self.cycle.0 += 1;
        }
        self.refetch = false;
        let ran = if self.clear(kept) {
            self.run_clear(bus, kept)
        } else {
            self.run(bus, kept).map(drop)
        };
        match ran {
            Ok(()) if !kept.rechecks => Ok(()),
            Ok(()) => Err(Ok(())),
            Err(trap) => Err(self.trap(trap)),
        }
    }

    /// After the op in slot `branch` jumped or branched back inside a straight run whose horizon
    /// is `horizon`: skips the passes of the spin on `CCOUNT` it closes, as `spin` says, or else
    /// those of the loop it closes. `Err` holds what the straight run ends with. Out of line and
    /// cold, so the straight run that calls it after each jump or branch back stays as it was.
    #[cold]
    #[inline(never)]
    pub(super) fn after_back<B: Bus>(
        &mut self,
        bus: &mut B,
        table: &mut Taken,
        branch: usize,
        horizon: Cycle,
    ) -> Result<(), Result<(), Stop>> {
        let bound = horizon.min(self.lines_until);
        let kept = table.kept(branch);
        if kept.back && !kept.spinless {
            self.skip_spin(bus, table, branch, bound);
            Ok(())
        } else if bound.0.saturating_sub(self.cycle.0) >= ROOM {
            self.skip_pass(bus, table, branch, horizon)
        } else {
            Ok(())
        }
    }

    /// At the head of a loop that the op in slot `branch` just went back to, inside a straight
    /// run whose horizon is `horizon`: records the next pass as it runs it, and skips the
    /// passes after it that the bus's ranges prove run alike. `Err` holds what the straight run
    /// ends with.
    fn skip_pass<B: Bus>(
        &mut self,
        bus: &mut B,
        table: &mut Taken,
        branch: usize,
        horizon: Cycle,
    ) -> Result<(), Result<(), Stop>> {
        if self.cycle.0 < self.waits.at(branch).at {
            return Ok(());
        }
        let Some(version) = bus.code_version() else {
            return Ok(());
        };
        let pass = match self.record(bus, table, version, branch, horizon) {
            Ok(Some(pass)) => pass,
            Ok(None) => {
                self.refused(table, branch);
                return Ok(());
            }
            Err(end) => {
                self.refused(table, branch);
                return Err(end);
            }
        };

        // The passes whose every op starts before the bound, from this one on.
        let start = self.cycle;
        let bound = horizon.min(self.lines_until);
        let last = pass.steps.last().map_or(0, |step| step.start);
        let room = bound
            .0
            .checked_sub(start.0 + last + 1)
            .map_or(0, |room| room / pass.period + 1);
        if room < 2 {
            self.wait_more(branch);
            return Ok(());
        }
        let mut swept = vec![Swept::default(); pass.accesses.len()];
        let mut proves = |n: u64, core: &Self, table: &Taken| {
            bus.sweep(&pass.accesses, start, pass.period, n, &mut swept)
                && core.same_path(table, &pass, &swept)
        };
        // The most passes that prove. A test that flips inside the room, such as the one that
        // toggles a pin every 500 ms, still skips up to the flip.
        let n = if proves(room, self, table) {
            room
        } else if !proves(2, self, table) {
            0
        } else {
            let (mut lo, mut hi) = (2, room);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if proves(mid, self, table) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        if n < 2 {
            self.refused(table, branch);
            return Ok(());
        }
        // The check build compares each pass with the ranges of the passes skipped.
        #[cfg(feature = "pass-skip-check")]
        assert!(proves(n, self, table), "{n} passes proved, then did not");
        let wait = self.waits.at(branch);
        (wait.at, wait.shift, wait.refusals) = (0, FIRST_WAIT, 0);
        (wait.skipped, wait.fresh) = (true, true);
        #[cfg(feature = "pass-skip-check")]
        self.check_skip(bus, table, version, &pass, &swept, n);
        #[cfg(not(feature = "pass-skip-check"))]
        {
            self.cycle.0 += (n - 1) * pass.period;
        }
        Ok(())
    }

    /// Runs one pass from the head as the straight run would, and records it. `None` when it
    /// holds an op this module takes no part in, or does not come back to the head as it left
    /// it. `Err` holds what the straight run ends with.
    fn record<B: Bus>(
        &mut self,
        bus: &mut B,
        table: &Taken,
        version: u64,
        branch: usize,
        horizon: Cycle,
    ) -> Result<Option<Pass>, Result<(), Stop>> {
        let head = self.pc;
        let c0 = self.cycle.0;
        let (ar0, sr0, pace0) = (self.ar, self.sr, self.pace);
        let mut bus = Recording::new(bus, self.cycle);
        let mut steps: Vec<Step> = Vec::with_capacity(128);
        let (mut live, mut written) = (0u64, 0u64);
        let (mut sar_live, mut sar_written) = (false, false);
        loop {
            if steps.len() == MOST_OPS || self.cycle >= horizon || self.cycle >= self.lines_until {
                return Ok(None);
            }
            let Some(slot) = table.find(self.pc, version) else {
                return Ok(None);
            };
            let kept = table.kept(slot);
            // An op this module has no shape for is left to the straight run.
            let Some(shape) = shape(&kept.insn) else {
                return Ok(None);
            };
            let base = self.sr[Sr::WindowBase as usize];
            let mut reads = [(0u8, 0u32); 2];
            for (read, n) in reads.iter_mut().zip(shape.reads) {
                if let Some(n) = n {
                    let p = phys(base, n);
                    *read = (p, self.ar[usize::from(p)]);
                    live |= !written & 1 << p;
                }
            }
            sar_live |= shape.sar_read && !sar_written;
            let start = self.cycle.0 - c0;
            let follows = self.pace & PACE_DIVIDE != 0;
            let sar = self.sr[Sr::Sar as usize];
            let made = bus.accesses.len();
            let seq = self.pc.wrapping_add(u32::from(kept.insn.len));
            self.straight_one(&mut bus, kept)?;
            let write = shape.write.map(|n| {
                let at = if shape.write_after {
                    self.sr[Sr::WindowBase as usize]
                } else {
                    base
                };
                let p = phys(at, n);
                written |= 1 << p;
                (p, self.ar[usize::from(p)])
            });
            sar_written |= shape.sar_write;
            let access = match (shape.kind, bus.accesses.len() - made) {
                (Kind::Load | Kind::Store, 1) => Some(made),
                (Kind::Load | Kind::Store, _) => return Ok(None),
                (_, 0) => None,
                _ => return Ok(None),
            };
            steps.push(Step {
                slot,
                shape,
                reads,
                sar,
                write,
                sar_after: self.sr[Sr::Sar as usize],
                taken: self.pc != seq,
                start,
                follows,
                access,
            });
            if self.pc == head {
                // Back at the head through another branch, the pass this one closes ran a
                // path that the other closes.
                if slot != branch {
                    return Ok(None);
                }
                break;
            }
        }
        // The next pass starts as this one did when the special registers but `SAR`, the pace,
        // the fetch and every register the pass reads before it writes are as they were.
        let same_sr = (0..self.sr.len())
            .filter(|&i| i != Sr::Sar as usize)
            .all(|i| self.sr[i] == sr0[i]);
        let same_live = (0..64).all(|p| live >> p & 1 == 0 || self.ar[p] == ar0[p]);
        let same_sar = !sar_live || self.sr[Sr::Sar as usize] == sr0[Sr::Sar as usize];
        if !(same_sr && same_live && same_sar && self.pace == pace0 && self.refetch) {
            return Ok(None);
        }
        Ok(Some(Pass {
            steps,
            accesses: bus.accesses,
            period: self.cycle.0 - c0,
            live,
            written,
            sar_live,
        }))
    }

    /// A look after the branch in slot `branch` that did not skip. A branch whose pass never
    /// skipped is not looked at again at this code version after a few of them.
    fn refused(&mut self, table: &mut Taken, branch: usize) {
        let wait = self.waits.at(branch);
        if wait.fresh {
            wait.fresh = false;
            return;
        }
        wait.refusals = wait.refusals.saturating_add(1);
        if !wait.skipped && wait.refusals >= MOST_REFUSALS {
            table.set_passless(branch);
        }
        self.wait_more(branch);
    }

    fn wait_more(&mut self, branch: usize) {
        let now = self.cycle.0;
        let wait = self.waits.at(branch);
        wait.at = now + (1 << wait.shift);
        wait.shift = (wait.shift + 1).min(LONGEST_WAIT);
    }

    /// Whether, with each load in its swept range, every op of `pass` from the registers as
    /// they stand goes the way it went and costs what it cost, and every register the pass
    /// reads before it writes ends as it stands now.
    fn same_path(&self, table: &Taken, pass: &Pass, swept: &[Swept]) -> bool {
        let mut regs: [Val; 64] = std::array::from_fn(|p| Val::known(self.ar[p]));
        let mut sar = Val::known(self.sr[Sr::Sar as usize]);
        for step in &pass.steps {
            let insn = &table.kept(step.slot).insn;
            let shape = &step.shape;
            let mut vals = [Val::known(0); 2];
            // Whether the op reads what it read when recorded, so it does what it did.
            let mut same = true;
            for (i, n) in shape.reads.iter().enumerate() {
                if n.is_some() {
                    let (p, value) = step.reads[i];
                    vals[i] = regs[usize::from(p)];
                    same &= vals[i].single() == Some(value);
                }
            }
            if shape.sar_read {
                same &= sar.single() == Some(step.sar);
            }
            let wrote = match shape.kind {
                Kind::Load => {
                    let Some(i) = step.access.filter(|_| same) else {
                        return false;
                    };
                    let s = swept[i];
                    let most = match pass.accesses[i].width {
                        Width::Byte => 0xff,
                        Width::Half => 0xffff,
                        Width::Word => u32::MAX,
                    };
                    if s.lo > s.hi || s.hi > most || s.same_as.is_some_and(|j| usize::from(j) >= i)
                    {
                        return false;
                    }
                    let mut v = Val::range(s.lo, s.hi, 0);
                    v.load = Some(s.same_as.unwrap_or(i as u16));
                    Some(v)
                }
                Kind::Store | Kind::Fixed => {
                    if !same {
                        return false;
                    }
                    step.write.map(|(_, value)| Val::known(value))
                }
                Kind::Branch => {
                    if !same && decide(insn.op, &insn.fields, vals[0], vals[1]) != Some(step.taken)
                    {
                        return false;
                    }
                    None
                }
                Kind::Plain if same => step.write.map(|(_, value)| Val::known(value)),
                Kind::Plain => {
                    let Some(v) = compute(insn, vals[0], vals[1], sar) else {
                        return false;
                    };
                    if matches!(insn.op, Op::QUOU | Op::REMU)
                        && !divide_costs_alike::<C>(insn.op, step.follows, vals[0])
                    {
                        return false;
                    }
                    Some(v)
                }
            };
            if let (Some((p, _)), Some(v)) = (step.write, wrote) {
                regs[usize::from(p)] = v;
            }
            if shape.sar_write {
                if !same {
                    return false;
                }
                sar = Val::known(step.sar_after);
            }
        }
        // Each pass then starts from the live-ins this one starts from.
        let kept = (0..64)
            .filter(|&p| (pass.live & pass.written) >> p & 1 != 0)
            .all(|p| regs[p].single() == Some(self.ar[p]));
        let sar_kept = !pass.sar_live || sar.single() == Some(self.sr[Sr::Sar as usize]);
        kept && sar_kept
    }

    /// For a check build: runs the `n - 1` passes a skip leaves out and requires each to take
    /// the recorded path in the recorded cycles, with each load in its range, and the core then
    /// to match a skipped one but for registers the next pass writes before it reads them.
    #[cfg(feature = "pass-skip-check")]
    fn check_skip<B: Bus>(
        &mut self,
        bus: &mut B,
        table: &Taken,
        version: u64,
        pass: &Pass,
        swept: &[Swept],
        n: u64,
    ) {
        let head = self.pc;
        let start = self.cycle.0;
        let mut skipped = self.clone();
        skipped.cycle.0 += (n - 1) * pass.period;
        for k in 0..n - 1 {
            let s0 = self.cycle.0;
            assert_eq!(
                s0,
                start + k * pass.period,
                "pass {k} of {head:#x} starts off"
            );
            let mut rec = Recording::new(bus, self.cycle);
            for step in &pass.steps {
                let slot = table.find(self.pc, version).expect("a kept op");
                assert_eq!(slot, step.slot, "pass {k} of {head:#x} left the path");
                assert_eq!(self.cycle.0 - s0, step.start, "an op's start in pass {k}");
                assert!(
                    self.straight_one(&mut rec, table.kept(slot)).is_ok(),
                    "pass {k} of {head:#x} ended the run"
                );
            }
            assert_eq!(rec.accesses, pass.accesses, "the accesses of pass {k}");
            for (i, access) in rec.accesses.iter().enumerate() {
                if access.store.is_none() {
                    let value = rec.values[i];
                    let s = swept[i];
                    assert!(
                        (s.lo..=s.hi).contains(&value),
                        "load {i} of pass {k} read {value:#x}, out of {s:?}"
                    );
                    if let Some(j) = s.same_as {
                        assert_eq!(value, rec.values[usize::from(j)], "load {i} as load {j}");
                    }
                }
            }
            assert_eq!(self.pc, head, "pass {k} came back");
            assert_eq!(self.cycle.0 - s0, pass.period, "the period of pass {k}");
        }
        // How many passes this process checked, printed at each power of two, so a run shows
        // that it skipped.
        static CHECKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let before = CHECKED.fetch_add(n - 1, std::sync::atomic::Ordering::Relaxed);
        if (before + n - 1).ilog2() > before.checked_ilog2().unwrap_or(0) || before == 0 {
            eprintln!("pass-skip-check: {} passes checked", before + n - 1);
        }
        let apart: Vec<usize> = (0..64)
            .filter(|&p| {
                let next_writes_first = pass.written >> p & 1 != 0 && pass.live >> p & 1 == 0;
                self.ar[p] != skipped.ar[p] && !next_writes_first
            })
            .collect();
        let sar_apart = pass.sar_live && self.sr[Sr::Sar as usize] != skipped.sr[Sr::Sar as usize];
        let other_sr = (0..self.sr.len())
            .filter(|&i| i != Sr::Sar as usize)
            .all(|i| self.sr[i] == skipped.sr[i]);
        assert!(
            apart.is_empty()
                && !sar_apart
                && other_sr
                && self.pc == skipped.pc
                && self.cycle == skipped.cycle
                && self.pace == skipped.pace
                && self.refetch == skipped.refetch
                && self.wait == skipped.wait
                && self.lines == skipped.lines
                && self.lines_until == skipped.lines_until
                && self.timer_due == skipped.timer_due
                && self.timer_next == skipped.timer_next
                && self.fr == skipped.fr
                && self.q == skipped.q
                && self.spun == skipped.spun
                && self.disabled == skipped.disabled,
            "a skip of {n} passes at {head:#x} ran apart from stepping: registers {apart:?}"
        );
    }
}

#[cfg(test)]
mod tests;
