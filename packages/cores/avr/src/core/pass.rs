//! Whole passes of a loop that calls, pushes, or writes SRAM, and puts every byte back.
//!
//! A chip's run loop steps each op a free run stops at through a [`Probed`] bus, and asks
//! [`PassProbe::visit`] before each one. The first op it visits is the anchor. When the program
//! counter is back at the anchor with the same registers, `SREG`, stack pointer and interrupt
//! delay, and the pass between read only steady bytes, wrote no device, and left every byte it
//! wrote as it found it, the next pass starts from the same state and runs the same way. The
//! passes that end before the bound are then skipped by their cycles.
//!
//! The probe changes nothing a step does. Each op still runs through the chip's bus, and the
//! probe only watches what it loads and stores. The chip reports what the probe cannot see,
//! such as an interrupt entry or an event, through [`PassProbe::spoil`]. While passes keep ending
//! unskipped, [`PassProbe::resting`] tells the chip to step ops without the probe.

use std::cell::Cell;

use super::Core;
use crate::bus::Bus;

/// Bytes one pass may write and put back. A pass that writes more is not skipped.
const TOUCHED: usize = 16;

/// Ops a pass may step before the probe drops its anchor, at the start of a run. An anchor the
/// loop never comes back to, such as an op of an interrupt handler, then costs a few dozen steps.
/// Each pass that steps past it doubles it up to [`STEPS`], so a long pass is still found.
const FIRST_STEPS: u32 = 32;

/// The most ops a pass may step. A loop that never comes back to its anchor in that many steps is
/// not one this skips.
const STEPS: u32 = 4096;

/// The most passes in a row that end unskipped, or that do not come back to the anchor, before
/// the chip rests the probe for the most ops. Each such pass doubles the rest, from one op up to
/// 1023.
const MISSES: u32 = 10;

/// What a pass has to leave as it found it, besides data space.
#[derive(Clone, PartialEq, Eq)]
struct Mark {
    pc: u16,
    r: [u8; 32],
    sreg: u8,
    sp: u16,
    irq_delay: bool,
    spm: usize,
    wdr: u32,
    unknown: Option<u16>,
    first_unknown: Option<(u16, u64)>,
    fetching: bool,
    stopped: bool,
}

impl Core {
    fn mark(&self) -> Mark {
        Mark {
            pc: self.pc,
            r: self.r,
            sreg: self.sreg,
            sp: self.sp,
            irq_delay: self.irq_delay,
            spm: self.spm.len(),
            wdr: self.wdr_executions,
            unknown: self.unknown,
            first_unknown: self.first_unknown,
            fetching: self.fetching,
            stopped: self.stopped,
        }
    }
}

#[derive(Clone)]
struct Anchor {
    mark: Mark,
    cycles: u64,
    bound: u64,
}

/// The anchor of the pass a run is in, and what that pass has done so far. A chip makes one per
/// run, so nothing carries across a host call.
pub struct PassProbe {
    anchor: Option<Anchor>,
    /// Each byte the pass wrote, with the value it held before the pass's first write to it.
    touched: [(u16, u8); TOUCHED],
    n: usize,
    steps: u32,
    /// Steps before the anchor moves.
    window: u32,
    spoiled: Cell<bool>,
    /// Passes in a row that ended without a skip.
    misses: u32,
    /// Ops the chip steps without the probe before it anchors again.
    rest: u32,
}

impl Default for PassProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl PassProbe {
    pub fn new() -> Self {
        Self {
            anchor: None,
            touched: [(0, 0); TOUCHED],
            n: 0,
            steps: 0,
            window: FIRST_STEPS,
            spoiled: Cell::new(false),
            misses: 0,
            rest: 0,
        }
    }

    /// The pass did something a later pass would not repeat, or something the probe cannot see.
    /// It is not skipped, and the next visit anchors a new one.
    #[inline(always)]
    pub fn spoil(&self) {
        self.spoiled.set(true);
    }

    /// The chip steps the next op without the probe. A loop whose passes keep ending unskipped,
    /// such as one that writes a device on every pass, then pays for the probe on few of its ops.
    /// The probe anchors afresh after a rest.
    #[inline(always)]
    pub fn resting(&mut self) -> bool {
        if self.rest == 0 {
            return false;
        }
        self.rest -= 1;
        true
    }

    /// At the boundary before the op at the program counter, which the chip steps next through
    /// [`Self::bus`]. `bound` is the cycle of the next event or the run's limit, the same bound a
    /// free run takes. Returns the cycles skipped, which the core has already added and the chip
    /// still moves its bus by.
    ///
    /// The chip visits only where the bus is quiet and no interrupt waits, and spoils the pass
    /// otherwise.
    #[inline(always)]
    pub fn visit<B: Bus>(&mut self, core: &mut Core, bus: &B, bound: u64) -> u64 {
        if let Some(anchor) = &self.anchor
            && anchor.mark.pc != core.pc
            && !self.spoiled.get()
            && self.steps < self.window
        {
            self.steps += 1;
            return 0;
        }
        self.revisit(core, bus, bound)
    }

    #[inline(never)]
    fn revisit<B: Bus>(&mut self, core: &mut Core, bus: &B, bound: u64) -> u64 {
        let mut skip = 0;
        if let Some(anchor) = &self.anchor {
            if !self.spoiled.get()
                && anchor.mark.pc == core.pc
                && anchor.bound == bound
                && core.cycles < bound
                && anchor.mark == core.mark()
                && self.restored(core, bus)
            {
                let pass = core.cycles - anchor.cycles;
                skip = (bound - 1 - core.cycles) / pass * pass;
                core.cycles += skip;
                self.misses = 0;
            } else {
                if !self.spoiled.get() && anchor.mark.pc != core.pc {
                    self.window = (self.window * 2).min(STEPS);
                }
                self.misses = (self.misses + 1).min(MISSES);
                self.rest = (1 << self.misses) - 1;
                self.anchor = None;
                self.spoiled.set(false);
                return 0;
            }
        }
        self.anchor = Some(Anchor {
            mark: core.mark(),
            cycles: core.cycles,
            bound,
        });
        self.n = 0;
        self.steps = 0;
        self.spoiled.set(false);
        skip
    }

    /// Every byte the pass wrote holds what it held before the pass.
    fn restored<B: Bus>(&self, core: &Core, bus: &B) -> bool {
        self.touched[..self.n]
            .iter()
            .all(|&(addr, was)| bus.peek(core, addr) == was)
    }

    /// Notes the byte at `addr` before the pass's first write to it.
    fn touch(&mut self, addr: u16, was: u8) {
        if self.touched[..self.n].iter().any(|&(at, _)| at == addr) {
            return;
        }
        if self.n == TOUCHED {
            self.spoil();
            return;
        }
        self.touched[self.n] = (addr, was);
        self.n += 1;
    }

    /// `bus`, watched for this pass.
    #[inline(always)]
    pub fn bus<'a, B: Bus>(&'a mut self, bus: &'a mut B) -> Probed<'a, B> {
        Probed { bus, probe: self }
    }
}

/// A chip's bus with a [`PassProbe`] watching what the core loads and stores through it.
pub struct Probed<'a, B> {
    bus: &'a mut B,
    probe: &'a mut PassProbe,
}

impl<B: Bus> Bus for Probed<'_, B> {
    /// A load of a byte that is not steady can read something else on the next pass.
    #[inline(always)]
    fn load(&mut self, core: &Core, addr: u16) -> u8 {
        if !self.probe.spoiled.get() && !self.bus.steady(addr) {
            self.probe.spoil();
        }
        self.bus.load(core, addr)
    }

    /// A plain store is noted when it changes the byte, and the pass is skipped only when the byte
    /// is back by its end. An idle store changes nothing. A store that writes only `SREG` or the
    /// stack pointer leaves what the visit compares. Any other store reaches a device.
    #[inline(always)]
    fn store(&mut self, core: &mut Core, addr: u16, value: u8, cycles: u64) {
        if !self.probe.spoiled.get() {
            if self.bus.plain(addr) {
                let was = self.bus.peek(core, addr);
                if was != value {
                    self.probe.touch(addr, was);
                }
            } else if !self.bus.store_idle(core, addr, value)
                && !self.bus.store_writes_sreg(addr)
                && !self.bus.store_writes_sp(addr)
            {
                self.probe.spoil();
            }
        }
        self.bus.store(core, addr, value, cycles);
    }

    /// A stepped `SBIC` or `SBIS` reads through here, as a load.
    #[inline(always)]
    fn peek(&self, core: &Core, addr: u16) -> u8 {
        if !self.probe.spoiled.get() && !self.bus.steady(addr) {
            self.probe.spoil();
        }
        self.bus.peek(core, addr)
    }

    #[inline(always)]
    fn steady(&self, addr: u16) -> bool {
        self.bus.steady(addr)
    }

    /// A projected byte moves with the cycle, so a later pass can read another.
    #[inline(always)]
    fn projected_at(&self, addr: u16, cycle: u64) -> Option<u8> {
        let byte = self.bus.projected_at(addr, cycle);
        if byte.is_some() {
            self.probe.spoil();
        }
        byte
    }

    #[inline(always)]
    fn store_idle(&self, core: &Core, addr: u16, value: u8) -> bool {
        self.bus.store_idle(core, addr, value)
    }

    #[inline(always)]
    fn store_writes_sreg(&self, addr: u16) -> bool {
        self.bus.store_writes_sreg(addr)
    }

    #[inline(always)]
    fn plain(&self, addr: u16) -> bool {
        self.bus.plain(addr)
    }

    #[inline(always)]
    fn store_writes_sp(&self, addr: u16) -> bool {
        self.bus.store_writes_sp(addr)
    }

    #[inline(always)]
    fn write_bit(&mut self, core: &mut Core, addr: u16, bit: u8, set: bool) {
        self.probe.spoil();
        self.bus.write_bit(core, addr, bit, set);
    }

    #[inline(always)]
    fn sleep(&mut self) -> bool {
        self.probe.spoil();
        self.bus.sleep()
    }

    /// The watchdog restarts at this cycle, which differs on every pass.
    #[inline(always)]
    fn watchdog_reset_timer(&mut self) {
        self.probe.spoil();
        self.bus.watchdog_reset_timer();
    }

    #[inline(always)]
    fn on_spm(&mut self, pc: u16, cycle: u64) {
        self.probe.spoil();
        self.bus.on_spm(pc, cycle);
    }

    #[inline(always)]
    fn on_break(&mut self, pc: u16, cycle: u64) -> bool {
        self.probe.spoil();
        self.bus.on_break(pc, cycle)
    }

    #[inline(always)]
    fn take_stall(&mut self) -> u8 {
        let stall = self.bus.take_stall();
        if stall != 0 {
            self.probe.spoil();
        }
        stall
    }
}
