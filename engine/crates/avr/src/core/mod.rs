//! Architectural state and the AVRe+ instruction set of this part.
//!
//! No `ELPM`, `EIJMP`, or `EICALL`. Cycle counts are the AVRe column in DS40002198,
//! which is the same timing as AVRe+. Call and return add the extra cycle the chip
//! table names. `SPM` does not write flash: that operation is selected by a control
//! register, and its cycle count varies with the programming time, so the logged
//! operation retires in one cycle. The core reads and writes data space only through
//! the bus. Interrupt entry is not an instruction; the facade runs it between
//! instructions.

mod execute;

use std::sync::Arc;

use crate::bus::Bus;

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

#[derive(Debug, Clone)]
pub(crate) struct Core {
    r: [u8; 32],
    sreg: u8,
    pc: u16,
    flash: Arc<[u16]>,
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

impl Core {
    pub(crate) fn new(sp: u16, pc_mask: u16) -> Self {
        Self {
            r: [0; 32],
            sreg: 0,
            pc: 0,
            flash: Arc::from([]),
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
        self.flash = flash;
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

    pub(crate) fn flash(&self) -> Arc<[u16]> {
        Arc::clone(&self.flash)
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
        let word = self.fetch(pc);
        self.irq_delay = false;
        self.execute(bus, pc, word);
        // The store that clears a halt is rare. Skipping it keeps the bus line
        // clean on the ordinary instruction.
        let halt = bus.cpu_halt();
        if halt != 0 {
            bus.clear_cpu_halt();
            self.cycles = self.cycles.wrapping_add(u64::from(halt));
        }
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
