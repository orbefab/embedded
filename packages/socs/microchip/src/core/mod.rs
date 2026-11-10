//! Architectural state and the AVRe+ instruction set of this part.
//!
//! No `ELPM`, `EIJMP`, or `EICALL`. Cycle counts are the AVRe column in DS40002198,
//! which is the same timing as AVRe+. Call and return add the extra cycle the chip
//! table names. `SPM` does not write flash: that operation is selected by a control
//! register, and its cycle count varies with the programming time, so the logged
//! operation retires in one cycle. The core reads and writes data space only through
//! the bus. Interrupt entry is not an instruction; the facade runs it between
//! instructions.

#[cfg(test)]
mod check;
mod decode;
mod execute;

use std::sync::Arc;

use crate::bus::Bus;
use decode::{Countdown, Op};

/// What the core does between instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Activity {
    Fetch,
    /// `SLEEP` with `SE` set. Time jumps to the next wake.
    Sleep,
    /// Start-up delay. `from_reset` fetches from address 0 when it ends.
    Startup {
        until: u64,
        from_reset: bool,
    },
    /// `RESET` is low, or `VCC` is under the brown-out threshold.
    Held,
}

/// An `SPM`. Flash is left unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpmNote {
    pub z: u16,
    pub word: u16,
}

/// An image and its op table, carried across a reset so the image is not decoded again.
pub(crate) struct Program {
    flash: Arc<[u16]>,
    ops: Arc<[Op]>,
}

#[derive(Debug, Clone)]
pub(crate) struct Core {
    r: [u8; 32],
    sreg: u8,
    pc: u16,
    flash: Arc<[u16]>,
    /// `flash` decoded, one op per word up to the program counter's reach.
    ops: Arc<[Op]>,
    /// Flash size in words, minus one.
    pc_mask: u16,
    /// Stack pointer. `SPL`/`SPH` on the bus read and write this word.
    /// DS40002198 §6.86 stores at SP and then post-decrements.
    sp: u16,
    spm: Vec<SpmNote>,
    /// `WDR` instructions executed. Not a count of watchdog resets.
    wdr_executions: u32,
    sleeping: bool,
    /// `true` only for [`Activity::Fetch`]. The instruction loop reads this
    /// byte instead of matching [`Activity`], which carries the start-up deadline.
    fetching: bool,
    /// What the core is doing besides fetching. `Fetch` is the usual path.
    activity: Activity,
    stopped: bool,
    unknown: Option<u16>,
    cycles: u64,
    /// `SEI` and `RETI` suppress interrupts at the boundary that follows them, until the next
    /// instruction starts.
    irq_delay: bool,
}

/// The program counter, registers, `SREG` and cycle count at a visit of a steady run's anchor.
#[derive(Clone, Copy)]
struct Visit {
    pc: u16,
    r: [u8; 32],
    sreg: u8,
    cycles: u64,
}

impl Core {
    pub(crate) fn new(sp: u16, pc_mask: u16) -> Self {
        Self {
            r: [0; 32],
            sreg: 0,
            pc: 0,
            flash: Arc::from([]),
            ops: Arc::from([]),
            pc_mask,
            sp,
            spm: Vec::new(),
            wdr_executions: 0,
            sleeping: false,
            fetching: true,
            activity: Activity::Fetch,
            stopped: false,
            unknown: None,
            cycles: 0,
            irq_delay: false,
        }
    }

    pub(crate) fn load_words(&mut self, words: &[u16]) {
        self.set_flash(Arc::from(words));
    }

    pub(crate) fn set_flash(&mut self, flash: Arc<[u16]>) {
        self.ops = Arc::from(decode::table(&flash, self.pc_mask));
        self.flash = flash;
        self.pc = 0;
    }

    pub(crate) fn program(&self) -> Program {
        Program {
            flash: Arc::clone(&self.flash),
            ops: Arc::clone(&self.ops),
        }
    }

    pub(crate) fn set_program(&mut self, program: Program) {
        self.flash = program.flash;
        self.ops = program.ops;
        self.pc = 0;
    }

    pub(crate) fn reg(&self, index: u8) -> u8 {
        self.r.get(usize::from(index)).copied().unwrap_or(0)
    }

    pub(crate) fn set_reg(&mut self, index: u8, value: u8) {
        if let Some(slot) = self.r.get_mut(usize::from(index)) {
            *slot = value;
        }
    }

    pub(crate) fn sreg(&self) -> u8 {
        self.sreg
    }

    pub(crate) fn set_sreg(&mut self, value: u8) {
        self.sreg = value;
    }

    pub(crate) fn sp(&self) -> u16 {
        self.sp
    }

    pub(crate) fn set_sp_lo(&mut self, value: u8) {
        self.sp = (self.sp & 0xFF00) | u16::from(value);
    }

    pub(crate) fn set_sp_hi(&mut self, value: u8) {
        self.sp = (self.sp & 0x00FF) | (u16::from(value) << 8);
    }

    pub(crate) fn pc(&self) -> u16 {
        self.pc
    }

    pub(crate) fn cycles(&self) -> u64 {
        self.cycles
    }

    pub(crate) fn advance_cycles(&mut self, n: u64) {
        self.cycles = self.cycles.wrapping_add(n);
    }

    pub(crate) fn set_cycles(&mut self, cycles: u64) {
        self.cycles = cycles;
    }

    #[cfg(test)]
    pub(crate) fn shares_image(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.flash, &other.flash) && Arc::ptr_eq(&self.ops, &other.ops)
    }

    pub(crate) fn spm_notes(&self) -> &[SpmNote] {
        &self.spm
    }

    pub(crate) fn wdr_executions(&self) -> u32 {
        self.wdr_executions
    }

    pub(crate) fn set_wdr_executions(&mut self, n: u32) {
        self.wdr_executions = n;
    }

    pub(crate) fn activity(&self) -> Activity {
        self.activity
    }

    pub(crate) fn set_activity(&mut self, activity: Activity) {
        self.fetching = matches!(activity, Activity::Fetch);
        self.sleeping = matches!(
            activity,
            Activity::Sleep
                | Activity::Startup {
                    from_reset: false,
                    ..
                }
        );
        self.activity = activity;
    }

    pub(crate) fn is_held(&self) -> bool {
        matches!(self.activity, Activity::Held)
    }

    #[inline]
    pub(crate) fn can_fetch(&self) -> bool {
        self.fetching
    }

    pub(crate) fn is_sleeping(&self) -> bool {
        self.sleeping
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped
    }

    pub(crate) fn interrupts_enabled(&self) -> bool {
        self.sreg & 0x80 != 0
    }

    pub(crate) fn irq_delay_pending(&self) -> bool {
        self.irq_delay
    }

    pub(crate) fn unknown(&self) -> Option<u16> {
        self.unknown
    }

    pub(crate) fn flash_word(&self, addr: u16) -> u16 {
        self.fetch(addr)
    }

    #[inline(always)]
    pub(crate) fn step(&mut self, bus: &mut Bus) {
        if self.sleeping || self.stopped {
            return;
        }
        let pc = self.pc;
        let op = self.op(pc);
        self.irq_delay = false;
        self.execute_op(bus, pc, op);
        // The store that clears a halt is rare. Skipping it keeps the bus line
        // clean on the ordinary instruction.
        let halt = bus.cpu_halt();
        if halt != 0 {
            bus.clear_cpu_halt();
            self.cycles = self.cycles.wrapping_add(u64::from(halt));
        }
    }

    /// Runs free ops, and the [`Self::execute_steady`] ops among them, while each ends before
    /// cycle `bound`. Returns the cycles of the op that reached `bound`, which the caller still
    /// retires through the bus, or `None` before any other op.
    #[inline(always)]
    pub(crate) fn run_free(&mut self, bus: &Bus, bound: u64) -> Option<u64> {
        // When `SEI` or `RETI` set the delay, the first op this runs is the one it delays, and
        // stepping clears it before that op too. A `None` goes to `step`, which clears it.
        self.irq_delay = false;
        let mut op = self.op(self.pc);
        // An `if` and a `loop`, not `while op.is_free()`. The `while` form measured about 50%
        // slower on a `delay()` loop.
        if op.is_free() {
            loop {
                let before = self.cycles;
                self.execute_in_run(self.pc, op, bound);
                if self.cycles >= bound {
                    return Some(self.cycles.wrapping_sub(before));
                }
                op = self.op(self.pc);
                if !op.is_free() {
                    break;
                }
            }
        }
        if !op.may_be_steady() {
            return None;
        }
        let last = self.visit();
        if self.execute_steady(bus, self.pc, op) {
            self.run_steady(bus, bound, last)
        } else {
            None
        }
    }

    fn visit(&self) -> Visit {
        Visit {
            pc: self.pc,
            r: self.r,
            sreg: self.sreg,
            cycles: self.cycles,
        }
    }

    /// The rest of [`Self::run_free`] after its first steady op, which ran from `last`. That op
    /// is the anchor. Each time the program counter is back at the anchor with the same registers
    /// and `SREG` as on the last visit, the stretch between the visits was a loop that changed
    /// nothing. Every later pass reads the same values and takes the same path, so the passes
    /// that end before `bound` are skipped by their cycles.
    #[inline(never)]
    fn run_steady(&mut self, bus: &Bus, bound: u64, mut last: Visit) -> Option<u64> {
        let mut before = last.cycles;
        loop {
            while self.cycles < bound {
                let op = self.op(self.pc);
                if !op.is_free() {
                    break;
                }
                before = self.cycles;
                self.execute_in_run(self.pc, op, bound);
            }
            if self.cycles >= bound {
                return Some(self.cycles.wrapping_sub(before));
            }
            if self.pc == last.pc {
                if (self.r, self.sreg) == (last.r, last.sreg) {
                    let pass = self.cycles.wrapping_sub(last.cycles);
                    let passes = (bound - 1 - self.cycles) / pass;
                    self.cycles = self.cycles.wrapping_add(passes * pass);
                }
                last = self.visit();
            }
            before = self.cycles;
            if !self.execute_steady(bus, self.pc, self.op(self.pc)) {
                return None;
            }
        }
    }

    /// Runs `op`, which [`Op::is_free`], in a run that ends at cycle `bound`. When a
    /// [`Countdown`]'s `BRNE` branches back to the head before `bound`, it jumps all but the last
    /// of the passes that end before `bound` and leave the counter above zero, and the last one
    /// runs. With fewer than two passes before `bound` it refuses before it reads the counter. The
    /// jump sits in this op's arm so that no other op pays for it.
    #[inline(always)]
    fn execute_in_run(&mut self, pc: u16, op: Op, bound: u64) {
        match op {
            Op::Countdown(countdown) => {
                self.execute_free(pc, op);
                let pass = u64::from(countdown.pass);
                if self.pc == countdown.head(pc)
                    && self.cycles < bound
                    && bound - 1 - self.cycles >= 2 * pass
                {
                    self.jump_passes(countdown, (bound - 1 - self.cycles) / pass);
                }
            }
            _ => self.execute_free(pc, op),
        }
    }

    /// [`Self::execute_in_run`] at the head of `countdown`, with `room` passes before the bound.
    #[inline(never)]
    fn jump_passes(&mut self, countdown: Countdown, room: u64) {
        let (lo, bytes) = (usize::from(countdown.lo), usize::from(countdown.bytes));
        let counter = &mut self.r[lo..lo + bytes];
        let n = counter
            .iter()
            .rev()
            .fold(0u64, |n, &byte| (n << 8) | u64::from(byte));
        let k = u64::from(countdown.k);
        let passes = room.min(n.saturating_sub(1) / k);
        if passes < 2 {
            return;
        }
        // A jumped pass leaves `SREG` as it was at the head. The pass that runs next rewrites
        // every flag the loop writes before any is read: `SUBI`, `SBIW` and `DEC` read no flag,
        // each `SBCI` reads only the `C` and `Z` the op before it wrote, and `BRNE` reads the `Z`
        // the decrement wrote. That pass ends before `bound` and before the exit, so no run
        // returns, takes an interrupt, or leaves the loop with the stale flags.
        let skip = passes - 1;
        let mut left = n - skip * k;
        for byte in counter {
            *byte = left as u8;
            left >>= 8;
        }
        self.cycles = self.cycles.wrapping_add(skip * u64::from(countdown.pass));
    }

    #[inline(always)]
    fn op(&self, pc: u16) -> Op {
        self.ops.get(usize::from(pc)).copied().unwrap_or(Op::Nop)
    }

    fn fetch(&self, pc: u16) -> u16 {
        self.flash.get(usize::from(pc)).copied().unwrap_or(0)
    }

    fn finish(&mut self, pc: u16, cycles: u64) {
        self.pc = pc & self.pc_mask;
        self.cycles = self.cycles.wrapping_add(cycles);
    }

    /// DS40002198 §6.86: `STACK ← Rr`, then `SP ← SP − 1`. `cycles` is the
    /// length of the instruction that pushes.
    fn push(&mut self, bus: &mut Bus, value: u8, cycles: u64) {
        let sp = self.sp;
        bus.store(self, sp, value, cycles);
        self.sp = sp.wrapping_sub(1);
    }

    /// DS40002198 §6.85: `SP ← SP + 1`, then `Rd ← STACK`.
    fn pop(&mut self, bus: &mut Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        let sp = self.sp;
        bus.load(self, sp)
    }
}
