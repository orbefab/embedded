//! EEPROM, one row. DS40002061B §8.4 and §8.6.
//!
//! The array lives on the heap so [`super::Device`] stays the size of a timer.
//! Programming time is the calibrated RC oscillator, not `clkCPU`, so `CLKPR`
//! does not change the wall time (§8.6.3, Table 8-2). The model uses 8.0 MHz,
//! the nominal RC: Table 8-2's 26,368 cycles are the 3.3 ms typical write.
//! Erase-only and write-only use Table 8-1's 1.8 ms, which is 14,400 of those
//! RC cycles. A reset while `VCC` is still sufficient finishes the byte in
//! progress (§8.4.2). A brown-out, or a reset below `VBOT−`, leaves the old
//! byte: the datasheet calls that case undefined, and this model keeps it.
//! `OSCCAL` does not retune this RC; the time stays the table's typical.

use crate::periph::Touch;

pub(crate) const REG_EEARH: u8 = 0;
pub(crate) const REG_EEARL: u8 = 1;
pub(crate) const REG_EEDR: u8 = 2;
pub(crate) const REG_EECR: u8 = 3;

const EEPM1: u8 = 1 << 5;
const EEPM0: u8 = 1 << 4;
const EERIE: u8 = 1 << 3;
const EEMPE: u8 = 1 << 2;
const EEPE: u8 = 1 << 1;
const EERE: u8 = 1 << 0;
const EEPM_MASK: u8 = EEPM1 | EEPM0;

/// Table 8-2. Atomic CPU write, in cycles of the 8.0 MHz RC.
const ATOMIC_RC: u64 = 26_368;
/// Table 8-1. 1.8 ms at that same 8.0 MHz is 14,400 cycles.
const HALF_RC: u64 = 14_400;
const RC_HZ: u64 = 8_000_000;
/// `EEAR9:8` are the only high bits on a 1 KiB EEPROM (§8.6.1).
const ADDR_MASK: u16 = 0x03FF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EepromCfg {
    pub bytes: u16,
    /// Table 12-6. `EE READY` is 23 on the 328P.
    pub vec: u8,
    pub regs: crate::regs::RegBlock,
}

#[derive(Debug, Clone)]
pub(crate) struct Eeprom {
    vec: u8,
    data: Box<[u8]>,
    addr: u16,
    edr: u8,
    eecr: u8,
    /// CPU clock the in-flight program was scheduled against.
    cpu_hz: u32,
    /// Retire cycle plus 4. `0` means the window is closed.
    eempe_until: u64,
    arm_window: bool,
    /// RC-derived length waiting for the retire cycle. `None` once scheduled.
    arm_len: Option<u64>,
    /// CPU cycle the program finishes. `u64::MAX` when idle.
    done_at: u64,
    pending: Option<u8>,
}

impl Eeprom {
    pub(crate) fn new(cfg: EepromCfg) -> Self {
        let len = usize::from(cfg.bytes.max(1));
        Self {
            vec: cfg.vec,
            data: vec![0xFF; len].into_boxed_slice(),
            addr: 0,
            edr: 0,
            eecr: 0,
            cpu_hz: 16_000_000,
            eempe_until: 0,
            arm_window: false,
            arm_len: None,
            done_at: u64::MAX,
            pending: None,
        }
    }

    pub(crate) fn set_cpu_hz(&mut self, hz: u32) {
        self.cpu_hz = hz.max(1);
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn load_image(&mut self, image: &[u8]) {
        for (slot, byte) in self.data.iter_mut().enumerate() {
            *byte = image.get(slot).copied().unwrap_or(0xFF);
        }
    }

    /// Wall time of a program, in CPU cycles at `cpu_hz`. `None` for EEPM 11.
    pub(crate) fn program_cycles(mode: u8, cpu_hz: u32) -> Option<u64> {
        let rc = match mode & 0x03 {
            0b00 => ATOMIC_RC,
            0b01 | 0b10 => HALF_RC,
            _ => return None,
        };
        Some(rc.saturating_mul(u64::from(cpu_hz.max(1))) / RC_HZ)
    }

    pub(crate) fn rescale(&mut self, now: u64, old_div: u32, new_div: u32) {
        self.done_at = scale(self.done_at, now, old_div, new_div);
        if self.eempe_until != 0 {
            self.eempe_until = scale(self.eempe_until, now, old_div, new_div);
        }
    }

    pub(crate) fn read(&mut self, reg: u8) -> (u8, bool) {
        (self.peek(reg), false)
    }

    /// `EEPE` falls when the program's event fires, and `EEMPE` when its window's does.
    pub(crate) fn steady(&self, reg: u8) -> bool {
        matches!(reg, REG_EEARH | REG_EEARL | REG_EEDR | REG_EECR)
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            REG_EEARH => (self.addr >> 8) as u8 & 0x03,
            REG_EEARL => self.addr as u8,
            REG_EEDR => self.edr,
            REG_EECR => self.eecr & !EERE,
            _ => 0,
        }
    }

    pub(crate) fn write(&mut self, reg: u8, value: u8, cycle: u64) -> Touch {
        let mut touch = Touch {
            irq: true,
            schedule: true,
            windows: true,
            ..Touch::default()
        };
        let busy = self.eecr & EEPE != 0;
        match reg {
            REG_EEARH | REG_EEARL if busy => {}
            REG_EEARH => {
                let hi = u16::from(value & 0x03) << 8;
                self.addr = (self.addr & 0x00FF) | hi;
                self.addr &= ADDR_MASK;
            }
            REG_EEARL => {
                self.addr = (self.addr & 0xFF00) | u16::from(value);
                self.addr &= ADDR_MASK;
            }
            REG_EEDR if !busy => self.edr = value,
            REG_EECR => self.write_cr(value, cycle, &mut touch),
            _ => {
                touch.irq = false;
                touch.schedule = false;
            }
        }
        touch
    }

    fn write_cr(&mut self, value: u8, cycle: u64, touch: &mut Touch) {
        let busy = self.eecr & EEPE != 0;
        if !busy {
            self.eecr = (self.eecr & !EEPM_MASK) | (value & EEPM_MASK);
        }
        if value & EERIE != 0 {
            self.eecr |= EERIE;
        } else {
            self.eecr &= !EERIE;
        }
        if !busy && value & EEMPE != 0 {
            self.eecr |= EEMPE;
            self.arm_window = true;
        }
        if value & EERE != 0 && !busy {
            self.edr = self.byte_at(self.addr);
            touch.halt = touch.halt.saturating_add(4);
        }
        if value & EEPE != 0 {
            self.try_program(value, cycle, touch);
        }
    }

    fn try_program(&mut self, _value: u8, cycle: u64, touch: &mut Touch) {
        if self.eecr & EEPE != 0 {
            return;
        }
        // §8.6.3. The store that sets EEPE does not arm itself. EEMPE has to
        // have been written by an earlier store, and this cycle is inside the
        // four-cycle window that store opened.
        let armed = self.eempe_until != 0 && cycle < self.eempe_until;
        if !armed {
            return;
        }
        let mode = (self.eecr & EEPM_MASK) >> 4;
        let Some(len) = Self::program_cycles(mode, self.cpu_hz) else {
            self.eecr &= !EEMPE;
            self.eempe_until = 0;
            self.arm_window = false;
            return;
        };
        let next = match mode {
            0b00 => self.edr,
            0b01 => 0xFF,
            0b10 => self.byte_at(self.addr) & self.edr,
            _ => return,
        };
        self.eecr = (self.eecr | EEPE) & !EEMPE;
        self.eempe_until = 0;
        self.arm_window = false;
        self.pending = Some(next);
        // Measured from the retire cycle, which includes this instruction's
        // two-cycle halt. `commit_windows` adds `len` there.
        self.arm_len = Some(len);
        touch.halt = touch.halt.saturating_add(2);
    }

    /// The instruction that armed a window or a program has retired at `retire`.
    pub(crate) fn commit_windows(&mut self, retire: u64) {
        if self.arm_window {
            self.arm_window = false;
            self.eempe_until = retire.saturating_add(4);
        }
        if let Some(len) = self.arm_len.take() {
            self.done_at = retire.saturating_add(len);
        }
    }

    /// §8.4.2. The in-progress byte lands when a reset arrives with `VCC`
    /// still sufficient. The caller skips this on a brown-out.
    pub(crate) fn commit_pending(&mut self) {
        if let Some(byte) = self.pending.take() {
            self.store_byte(self.addr, byte);
        }
        self.arm_len = None;
        self.done_at = u64::MAX;
        self.eecr &= !EEPE;
    }

    pub(crate) fn poke(&mut self, reg: u8, value: u8, cycle: u64) -> Touch {
        let touch = self.write(reg, value, cycle);
        // A poke is already at its cycle. The window starts there.
        self.commit_windows(cycle);
        touch
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        if self.eecr & EERIE != 0 && self.eecr & EEPE == 0 {
            1u64 << self.vec
        } else {
            0
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        let window = if self.eempe_until == 0 {
            u64::MAX
        } else {
            self.eempe_until
        };
        window.min(self.done_at)
    }

    pub(crate) fn fire(&mut self, at: u64) {
        if self.eempe_until != 0 && self.eempe_until == at {
            self.eecr &= !EEMPE;
            self.eempe_until = 0;
        }
        if self.done_at == at {
            if let Some(byte) = self.pending.take() {
                self.store_byte(self.addr, byte);
            }
            self.eecr &= !EEPE;
            self.done_at = u64::MAX;
        }
    }

    fn byte_at(&self, addr: u16) -> u8 {
        self.data.get(usize::from(addr)).copied().unwrap_or(0xFF)
    }

    fn store_byte(&mut self, addr: u16, value: u8) {
        if let Some(slot) = self.data.get_mut(usize::from(addr)) {
            *slot = value;
        }
    }
}

pub(crate) fn scale(deadline: u64, now: u64, old_div: u32, new_div: u32) -> u64 {
    if deadline == u64::MAX || deadline <= now || new_div == 0 {
        return deadline;
    }
    let left = deadline - now;
    now + left.saturating_mul(u64::from(old_div)) / u64::from(new_div)
}
