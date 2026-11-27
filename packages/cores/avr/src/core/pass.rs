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
//! such as an interrupt entry or an event, through [`PassProbe::spoil`].

use std::cell::Cell;

use super::Core;
use crate::bus::Bus;

/// Bytes one pass may write and put back. A pass that writes more is not skipped.
const TOUCHED: usize = 16;

/// Visits a pass may take before the anchor moves to the op being visited. A loop that never
/// comes back to its anchor in that many steps is not one this skips.
const STEPS: u32 = 4096;

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
    spoiled: Cell<bool>,
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
            spoiled: Cell::new(false),
        }
    }

    /// The pass did something a later pass would not repeat, or something the probe cannot see.
    /// It is not skipped, and the next visit anchors a new one.
    #[inline(always)]
    pub fn spoil(&self) {
        self.spoiled.set(true);
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
            && self.steps < STEPS
        {
            self.steps += 1;
            return 0;
        }
        self.revisit(core, bus, bound)
    }

    #[inline(never)]
    fn revisit<B: Bus>(&mut self, core: &mut Core, bus: &B, bound: u64) -> u64 {
        let mut skip = 0;
        if let Some(anchor) = &self.anchor
            && !self.spoiled.get()
            && anchor.mark.pc == core.pc
            && anchor.bound == bound
            && core.cycles < bound
            && anchor.mark == core.mark()
            && self.restored(core, bus)
        {
            let pass = core.cycles - anchor.cycles;
            skip = (bound - 1 - core.cycles) / pass * pass;
            core.cycles += skip;
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
        if !self.bus.steady(addr) {
            self.probe.spoil();
        }
        self.bus.load(core, addr)
    }

    /// An idle store changes nothing. A plain store is noted, and the pass is skipped only when
    /// the byte is back by its end. A store that writes only `SREG` or the stack pointer leaves
    /// what the visit compares. Any other store reaches a device.
    #[inline(always)]
    fn store(&mut self, core: &mut Core, addr: u16, value: u8, cycles: u64) {
        if !self.bus.store_idle(core, addr, value) {
            if self.bus.plain(addr) {
                self.probe.touch(addr, self.bus.peek(core, addr));
            } else if !self.bus.store_writes_sreg(addr) && !self.bus.store_writes_sp(addr) {
                self.probe.spoil();
            }
        }
        self.bus.store(core, addr, value, cycles);
    }

    /// A stepped `SBIC` or `SBIS` reads through here, as a load.
    #[inline(always)]
    fn peek(&self, core: &Core, addr: u16) -> u8 {
        if !self.bus.steady(addr) {
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
    fn take_stall(&mut self) -> u8 {
        let stall = self.bus.take_stall();
        if stall != 0 {
            self.probe.spoil();
        }
        stall
    }
}
