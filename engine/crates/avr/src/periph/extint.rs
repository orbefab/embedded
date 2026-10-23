//! External interrupts and pin-change interrupts.
//!
//! `INT0`/`INT1` (`EICRA`, `EIMSK`, `EIFR`) and `PCINT0`–`PCINT2`
//! (`PCICR`, `PCIFR`, `PCMSK0`–`PCMSK2`). DS40002061B §13.
//!
//! An edge is taken from the same synchronizer as `PINx` ([`PIN_SYNC_DELAY`]).
//! The flag is set when that sample changes. A low-level request has no flag
//! and stays pending while the sampled pin is low. `EIFR` and `PCIFR` are
//! write-one-to-clear and clear again on vector entry.
//!
//! Sleep modes other than Idle stop the I/O clock (DS40002061B §13, the
//! paragraph above §13.1). While that clock is stopped this device does not
//! set edge flags. Low level stays a level request.

use crate::Level;
use crate::Port;
use crate::mux::{PIN_SYNC_DELAY, PinMux};
use crate::periph::adc::TRIG_INT0;
use crate::periph::{Ctx, Notice, Touch};
use crate::regs::RegBlock;

pub(crate) const REG_MCUCR: u8 = 0;
pub(crate) const REG_EICRA: u8 = 1;
pub(crate) const REG_EICRB: u8 = 2;
pub(crate) const REG_EIMSK: u8 = 3;
pub(crate) const REG_EIFR: u8 = 4;
pub(crate) const REG_PCICR: u8 = 5;
pub(crate) const REG_PCIFR: u8 = 6;
pub(crate) const REG_PCMSK0: u8 = 7;
pub(crate) const REG_PCMSK1: u8 = 8;
pub(crate) const REG_PCMSK2: u8 = 9;

const PUD: u8 = 1 << 4;
const DUE_CAP: usize = 40;

/// One external interrupt line. `bit` is the `EIMSK`/`EIFR` bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IntLine {
    pub pin: u8,
    pub bit: u8,
    /// Shift of `ISCn1:0` inside `EICRA`, or inside `EICRB` when `in_b`.
    pub isc_shift: u8,
    pub in_b: bool,
    pub vector: u8,
}

/// One pin-change group. `bit` is the `PCICR`/`PCIFR` bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PcGroup {
    pub port: Port,
    pub index: u8,
    pub bit: u8,
    pub vector: u8,
    /// Mask bits that name a real pin. The rest read as zero.
    pub valid: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExtIntCfg {
    pub lines: &'static [IntLine],
    pub groups: &'static [PcGroup],
    pub mcucr_mask: u8,
    pub eifr_mask: u8,
    pub pcifr_mask: u8,
    pub eimsk_mask: u8,
    pub pcicr_mask: u8,
    pub eicra_mask: u8,
    pub eicrb_mask: u8,
    pub regs: RegBlock,
}

#[derive(Debug, Clone, Copy)]
struct Due {
    pin: u8,
    high: bool,
    at: u64,
}

#[derive(Debug)]
pub(crate) struct ExtInt {
    cfg: ExtIntCfg,
    mcucr: u8,
    eicra: u8,
    eicrb: u8,
    eimsk: u8,
    eifr: u8,
    pcicr: u8,
    pcifr: u8,
    pcmsk: [u8; 3],
    /// Sampled digital level, one bit per pin id. High is 1.
    seen: u64,
    /// Waiting edges. Allocated once in [`ExtInt::new`]; the tick path only
    /// follows the pointer while `due_len` is non-zero.
    due: Box<[Due; DUE_CAP]>,
    due_len: u8,
    next: u64,
    /// False while a sleep mode has stopped the I/O clock.
    io_clock: bool,
    rose: u8,
    fell: u8,
    /// `BODSE` window closes at this cycle. `0` when closed. §10.11.2.
    bodse_until: u64,
    arm_bodse: bool,
    /// `BODS` reads as 1 until this cycle, three cycles after the second write.
    bods_until: u64,
    arm_bods: bool,
}

clone_fields!(ExtInt {
    cfg,
    mcucr,
    eicra,
    eicrb,
    eimsk,
    eifr,
    pcicr,
    pcifr,
    pcmsk,
    seen,
    due,
    due_len,
    next,
    io_clock,
    rose,
    fell,
    bodse_until,
    arm_bodse,
    bods_until,
    arm_bods
});

impl ExtInt {
    pub(crate) fn new(cfg: ExtIntCfg) -> Self {
        Self {
            cfg,
            mcucr: 0,
            eicra: 0,
            eicrb: 0,
            eimsk: 0,
            eifr: 0,
            pcicr: 0,
            pcifr: 0,
            pcmsk: [0; 3],
            seen: 0,
            due: Box::new(
                [Due {
                    pin: 0,
                    high: false,
                    at: 0,
                }; DUE_CAP],
            ),
            due_len: 0,
            next: u64::MAX,
            io_clock: true,
            rose: 0,
            fell: 0,
            bodse_until: 0,
            arm_bodse: false,
            bods_until: 0,
            arm_bods: false,
        }
    }

    /// §10.11.2. True while `BODS` is inside its three-cycle active window.
    pub(crate) fn bods_active(&self, cycle: u64) -> bool {
        self.bods_until != 0 && cycle < self.bods_until
    }

    pub(crate) fn commit_windows(&mut self, retire: u64) {
        if self.arm_bodse {
            self.arm_bodse = false;
            self.bodse_until = retire.saturating_add(4);
        }
        if self.arm_bods {
            self.arm_bods = false;
            self.bods_until = retire.saturating_add(3);
        }
        self.recompute_next();
    }

    pub(crate) fn notice_waits(&self) -> bool {
        self.rose != 0 || self.fell != 0
    }

    pub(crate) fn take_notice(&mut self) -> Notice {
        let notice = Notice {
            rose: self.rose,
            fell: self.fell,
            ..Notice::default()
        };
        self.rose = 0;
        self.fell = 0;
        notice
    }

    /// Edge detectors that need `clkI/O` stay idle while `running` is
    /// false. Low-level `INT` keeps tracking the pad, because that request is
    /// asynchronous (DS40002061B §13).
    pub(crate) fn set_io_clock_running(&mut self, running: bool) {
        self.io_clock = running;
    }

    pub(crate) fn on_pad(&mut self, pin: u8, level: Level, at: u64) -> Touch {
        if !self.watches(pin) {
            return Touch::default();
        }
        let high = level == Level::High;
        if !self.io_clock {
            // The pad moved without clkI/O. A sample still waiting in the
            // synchronizer is an older level and must not overwrite this one.
            self.drop_due(pin);
            let was = self.seen_high(pin);
            self.set_seen(pin, high);
            self.hold_level_flags();
            // Pin change is asynchronous (§13). An edge INT needs clkI/O and
            // is not flagged here. Low level follows `seen`.
            if was != high {
                self.note_pcint(pin);
            }
            return Touch {
                irq: true,
                schedule: true,
                ..Touch::default()
            };
        }
        self.push_due(Due {
            pin,
            high,
            at: at.saturating_add(PIN_SYNC_DELAY),
        });
        Touch {
            schedule: true,
            ..Touch::default()
        }
    }

    pub(crate) fn read(&mut self, reg: u8) -> u8 {
        self.peek(reg)
    }

    /// Flags rise when a queued sample fires, and `BODS` falls at its window's event.
    pub(crate) fn steady(&self, reg: u8) -> bool {
        matches!(
            reg,
            REG_MCUCR
                | REG_EICRA
                | REG_EICRB
                | REG_EIMSK
                | REG_EIFR
                | REG_PCICR
                | REG_PCIFR
                | REG_PCMSK0
                | REG_PCMSK1
                | REG_PCMSK2
        )
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            REG_MCUCR => self.mcucr,
            REG_EICRA => self.eicra,
            REG_EICRB => self.eicrb,
            REG_EIMSK => self.eimsk,
            REG_EIFR => self.eifr,
            REG_PCICR => self.pcicr,
            REG_PCIFR => self.pcifr,
            REG_PCMSK0 => self.pcmsk[0],
            REG_PCMSK1 => self.pcmsk[1],
            REG_PCMSK2 => self.pcmsk[2],
            _ => 0,
        }
    }

    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        at: u64,
    ) -> Touch {
        let mut touch = Touch {
            irq: true,
            ..Touch::default()
        };
        match reg {
            REG_MCUCR => {
                let next = value & self.cfg.mcucr_mask;
                let pud = next & PUD != 0;
                if pud != (self.mcucr & PUD != 0) {
                    mux.set_pud(pud, ctx, at, true);
                    touch.schedule = true;
                }
                self.write_mcucr(next, at);
                touch.schedule = true;
                touch.windows = true;
            }
            REG_EICRA => {
                self.eicra = value & self.cfg.eicra_mask;
                self.hold_level_flags();
            }
            REG_EICRB => {
                self.eicrb = value & self.cfg.eicrb_mask;
                self.hold_level_flags();
            }
            REG_EIMSK => self.eimsk = value & self.cfg.eimsk_mask,
            REG_EIFR => {
                self.clear_written(value & self.cfg.eifr_mask);
                self.hold_level_flags();
            }
            REG_PCICR => self.pcicr = value & self.cfg.pcicr_mask,
            REG_PCIFR => self.pcifr &= !(value & self.cfg.pcifr_mask),
            REG_PCMSK0 => self.pcmsk[0] = value,
            REG_PCMSK1 => self.pcmsk[1] = value & self.group_valid(1),
            REG_PCMSK2 => self.pcmsk[2] = value & self.group_valid(2),
            _ => touch.irq = false,
        }
        touch
    }

    pub(crate) fn poke(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        at: u64,
    ) -> Touch {
        match reg {
            REG_MCUCR => {
                self.write_mcucr(value & self.cfg.mcucr_mask, at);
                self.commit_windows(at);
                mux.set_pud(self.mcucr & PUD != 0, ctx, at, false);
            }
            REG_EICRA => self.eicra = value & self.cfg.eicra_mask,
            REG_EICRB => self.eicrb = value & self.cfg.eicrb_mask,
            REG_EIMSK => self.eimsk = value & self.cfg.eimsk_mask,
            REG_EIFR => self.plant_eifr(value & self.cfg.eifr_mask),
            REG_PCICR => self.pcicr = value & self.cfg.pcicr_mask,
            REG_PCIFR => self.pcifr = value & self.cfg.pcifr_mask,
            REG_PCMSK0 => self.pcmsk[0] = value,
            REG_PCMSK1 => self.pcmsk[1] = value & self.group_valid(1),
            REG_PCMSK2 => self.pcmsk[2] = value & self.group_valid(2),
            _ => {}
        }
        self.hold_level_flags();
        Touch {
            irq: true,
            // §10.11.2. The BODS window expiry is an event. Other pokes
            // only plant bits.
            schedule: reg == REG_MCUCR,
            ..Touch::default()
        }
    }

    /// `ISC = 0` makes this `INT` line a level request. Anything else is an edge.
    pub(crate) fn int_is_level(&self, vector: u8) -> bool {
        self.cfg
            .lines
            .iter()
            .any(|line| line.vector == vector && self.isc(*line) == 0)
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        let mut bits = 0u64;
        for line in self.cfg.lines {
            if self.eimsk & (1 << line.bit) == 0 {
                continue;
            }
            if self.isc(*line) == 0 {
                if !self.seen_high(line.pin) {
                    bits |= 1u64 << line.vector;
                }
            } else if self.eifr & (1 << line.bit) != 0 {
                bits |= 1u64 << line.vector;
            }
        }
        for group in self.cfg.groups {
            let mask = 1u8 << group.bit;
            if self.pcicr & mask != 0 && self.pcifr & mask != 0 {
                bits |= 1u64 << group.vector;
            }
        }
        bits
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        let lines = self.cfg.lines;
        for line in lines {
            if line.vector == vector && self.isc(*line) != 0 {
                self.lower_line(*line);
            }
        }
        for group in self.cfg.groups {
            if group.vector == vector {
                self.pcifr &= !(1 << group.bit);
            }
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        self.next
    }

    pub(crate) fn fire(&mut self, at: u64) {
        let mut index = 0;
        while index < usize::from(self.due_len) {
            if self.due[index].at != at {
                index += 1;
                continue;
            }
            let due = self.due[index];
            self.due[index] = self.due[usize::from(self.due_len) - 1];
            self.due_len -= 1;
            self.commit(due);
        }
        self.expire_bod(at);
        self.recompute_next();
    }

    pub(crate) fn reschedule(&mut self) {
        self.recompute_next();
    }

    fn commit(&mut self, due: Due) -> bool {
        let was = self.seen_high(due.pin);
        if was == due.high {
            return false;
        }
        self.set_seen(due.pin, due.high);
        // The sample was queued while `clkI/O` ran. If the clock has stopped
        // by the time it comes due, the level is kept and the edge flag is not
        // set. A low-level request follows `seen`. Sleep decides the wake.
        if !self.io_clock {
            self.hold_level_flags();
            // The edge was sampled on clkI/O. The flag stays down (§13). Pin
            // change still wakes: the pad did toggle.
            self.note_pcint(due.pin);
            return true;
        }
        let mut irq = false;
        for line in self.cfg.lines {
            if line.pin != due.pin {
                continue;
            }
            irq |= self.sense(*line, was, due.high);
        }
        for group in self.cfg.groups {
            if !self.group_has(*group, due.pin) {
                continue;
            }
            let bit = port_bit(due.pin);
            if self.pcmsk[usize::from(group.index)] & (1 << bit) == 0 {
                continue;
            }
            self.pcifr |= 1 << group.bit;
            irq = true;
        }
        irq
    }

    fn sense(&mut self, line: IntLine, was: bool, now: bool) -> bool {
        match self.isc(line) {
            0 => {
                self.lower_line(line);
                true
            }
            1 => {
                self.raise_line(line);
                true
            }
            2 if was && !now => {
                self.raise_line(line);
                true
            }
            3 if !was && now => {
                self.raise_line(line);
                true
            }
            _ => false,
        }
    }

    fn hold_level_flags(&mut self) {
        let lines = self.cfg.lines;
        for line in lines {
            if self.isc(*line) == 0 {
                self.lower_line(*line);
            }
        }
    }

    /// `INT0` (vector 2) is Table 24-6 trigger source 2. Other lines are not.
    fn raise_line(&mut self, line: IntLine) {
        let bit = 1u8 << line.bit;
        if self.eifr & bit == 0 && line.vector == 2 {
            self.rose |= TRIG_INT0;
        }
        self.eifr |= bit;
    }

    fn lower_line(&mut self, line: IntLine) {
        let bit = 1u8 << line.bit;
        if self.eifr & bit != 0 && line.vector == 2 {
            self.fell |= TRIG_INT0;
        }
        self.eifr &= !bit;
    }

    fn clear_written(&mut self, mask: u8) {
        let lines = self.cfg.lines;
        for line in lines {
            if mask & (1 << line.bit) != 0 {
                self.lower_line(*line);
            }
        }
    }

    fn plant_eifr(&mut self, next: u8) {
        let lines = self.cfg.lines;
        for line in lines {
            let bit = 1u8 << line.bit;
            let was = self.eifr & bit != 0;
            let now = next & bit != 0;
            if !was && now && line.vector == 2 {
                self.rose |= TRIG_INT0;
            }
            if was && !now && line.vector == 2 {
                self.fell |= TRIG_INT0;
            }
        }
        self.eifr = next;
    }

    fn isc(&self, line: IntLine) -> u8 {
        let word = if line.in_b { self.eicrb } else { self.eicra };
        (word >> line.isc_shift) & 0x03
    }

    fn watches(&self, pin: u8) -> bool {
        if self.cfg.lines.iter().any(|line| line.pin == pin) {
            return true;
        }
        self.cfg
            .groups
            .iter()
            .any(|group| self.group_has(*group, pin))
    }

    fn group_has(&self, group: PcGroup, pin: u8) -> bool {
        let Some((port, bit)) = Port::from_pin(u32::from(pin)) else {
            return false;
        };
        port == group.port && group.valid & (1 << bit) != 0
    }

    fn group_valid(&self, index: u8) -> u8 {
        self.cfg
            .groups
            .iter()
            .find(|group| group.index == index)
            .map(|group| group.valid)
            .unwrap_or(0)
    }

    fn seen_high(&self, pin: u8) -> bool {
        self.seen & (1u64 << pin) != 0
    }

    fn set_seen(&mut self, pin: u8, high: bool) {
        if high {
            self.seen |= 1u64 << pin;
        } else {
            self.seen &= !(1u64 << pin);
        }
    }

    fn drop_due(&mut self, pin: u8) {
        let mut index = 0;
        while index < usize::from(self.due_len) {
            if self.due[index].pin == pin {
                self.due[index] = self.due[usize::from(self.due_len) - 1];
                self.due_len -= 1;
            } else {
                index += 1;
            }
        }
        self.recompute_next();
    }

    fn push_due(&mut self, due: Due) {
        if usize::from(self.due_len) >= DUE_CAP {
            return;
        }
        self.due[usize::from(self.due_len)] = due;
        self.due_len += 1;
        self.recompute_next();
    }

    fn recompute_next(&mut self) {
        let mut next = u64::MAX;
        for due in &self.due[..usize::from(self.due_len)] {
            next = next.min(due.at);
        }
        if self.bodse_until != 0 {
            next = next.min(self.bodse_until);
        }
        if self.bods_until != 0 {
            next = next.min(self.bods_until);
        }
        self.next = next;
    }

    fn write_mcucr(&mut self, next: u8, at: u64) {
        const BODS: u8 = 1 << 6;
        const BODSE: u8 = 1 << 5;
        const KEPT: u8 = PUD | (1 << 1) | (1 << 0);
        let bods = next & BODS != 0;
        let bodse = next & BODSE != 0;
        let mut bod = self.mcucr & (BODS | BODSE);
        if bods && bodse {
            bod = BODS | BODSE;
            self.arm_bodse = true;
        } else if self.bodse_until != 0 && at < self.bodse_until && bods && !bodse {
            bod = BODS;
            self.arm_bods = true;
            self.bodse_until = 0;
            self.arm_bodse = false;
        }
        self.mcucr = (next & KEPT) | bod;
    }

    fn expire_bod(&mut self, at: u64) {
        const BODS: u8 = 1 << 6;
        const BODSE: u8 = 1 << 5;
        if self.bodse_until != 0 && self.bodse_until == at {
            self.bodse_until = 0;
            self.mcucr &= !(BODS | BODSE);
        }
        if self.bods_until != 0 && self.bods_until == at {
            self.bods_until = 0;
            self.mcucr &= !BODS;
        }
    }

    /// Asynchronous pin change. No flag when the pin is not in a mask.
    fn note_pcint(&mut self, pin: u8) {
        for group in self.cfg.groups {
            if !self.group_has(*group, pin) {
                continue;
            }
            let bit = port_bit(pin);
            if self.pcmsk[usize::from(group.index)] & (1 << bit) == 0 {
                continue;
            }
            self.pcifr |= 1 << group.bit;
        }
    }
}

fn port_bit(pin: u8) -> u8 {
    pin & 7
}
