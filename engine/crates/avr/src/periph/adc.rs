//! ADC. DS40002061B §24.
//!
//! The prescaler runs while `ADEN` is set and is held in reset while `ADEN`
//! is clear (§24.4). A conversion is one scheduled completion, not a step per
//! ADC clock. The input is sampled at the Table 24-1 sample-and-hold point
//! and ignored after that.
//!
//! Auto-trigger sources are rising edges of interrupt flags (Table 24-6,
//! §24.9.4), delivered by the bus. [`Adc::noise_reduction`] is the §24.6
//! start from ADC-noise-reduction sleep. `PRADC` gates the ADC clock (§10.11.3).

use crate::analog::Sense;
use crate::mux::PinMux;
use crate::periph::{Ctx, Gate, Notice, Touch};
use crate::regs::RegBlock;

pub(crate) const REG_ADCL: u8 = 0;
pub(crate) const REG_ADCH: u8 = 1;
pub(crate) const REG_ADCSRA: u8 = 2;
pub(crate) const REG_ADCSRB: u8 = 3;
pub(crate) const REG_ADMUX: u8 = 4;
pub(crate) const REG_DIDR0: u8 = 5;

pub(crate) const TRIG_ACI: u8 = 1 << 1;
pub(crate) const TRIG_INT0: u8 = 1 << 2;

const ADEN: u8 = 1 << 7;
const ADSC: u8 = 1 << 6;
const ADATE: u8 = 1 << 5;
const ADIF: u8 = 1 << 4;
const ADIE: u8 = 1 << 3;
const ACME: u8 = 1 << 6;
const ADLAR: u8 = 1 << 5;

/// Table 24-5. `ADPS` 0 and 1 are both divide-by-2.
const DIV: [u16; 8] = [2, 2, 4, 8, 16, 32, 64, 128];

/// §24.4. Auto-trigger synchronization, in CPU clocks.
const AUTO_SYNC: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdcCfg {
    pub vec: u8,
    /// Pin id of ADC0..ADC7. `0xFF` has no digital pad.
    pub pin: [u8; 8],
    /// `PRADC`. Zero means this row has no power-reduction bit.
    pub prr_bit: u8,
    pub regs: RegBlock,
}

#[derive(Debug, Clone)]
pub(crate) struct Adc {
    vec: u8,
    pin: [u8; 8],
    admux: u8,
    /// `ADCSRA` without the busy and flag bits, which are derived.
    csra: u8,
    adcsrb: u8,
    didr0: u8,
    adif: bool,
    busy: bool,
    /// Next conversion after `ADEN` rose is the 25-clock initialization.
    init_pending: bool,
    /// Prescaler phase. Figure 24-3: the counter is 0 at `origin`, and the
    /// selected bit rises at `origin + div/2`, then every `div`.
    /// `u64::MAX` while `ADEN` is clear (the counter is held reset).
    origin: u64,
    div: u16,
    /// Cycle of the conversion start edge. `ADMUX` is copied there (§24.5).
    start_at: u64,
    /// `ADMUX` latched at the start edge (§24.5).
    latched: u8,
    /// Code captured at sample-and-hold. Published at completion.
    held: u16,
    sampled: bool,
    sample_at: u64,
    done_at: u64,
    /// Result visible to `ADCL`/`ADCH`.
    result: u16,
    /// Set by a read of `ADCL` until `ADCH` is read (§24.9.3).
    locked: bool,
    locked_value: u16,
    next: u64,
    notice: Notice,
    /// Table 10-1 `clkADC`, and `PRADC` (§10.11.3).
    clock_on: bool,
    prr_off: bool,
    frozen_at: u64,
}

impl Adc {
    pub(crate) fn new(cfg: &AdcCfg) -> Self {
        Self {
            vec: cfg.vec,
            pin: cfg.pin,
            admux: 0,
            csra: 0,
            adcsrb: 0,
            didr0: 0,
            adif: false,
            busy: false,
            init_pending: true,
            origin: u64::MAX,
            div: DIV[0],
            start_at: u64::MAX,
            latched: 0,
            held: 0,
            sampled: false,
            sample_at: u64::MAX,
            done_at: u64::MAX,
            result: 0,
            locked: false,
            locked_value: 0,
            next: u64::MAX,
            notice: Notice::default(),
            clock_on: true,
            prr_off: false,
            frozen_at: 0,
        }
    }

    fn running(&self) -> bool {
        self.clock_on && !self.prr_off
    }

    fn aden(&self) -> bool {
        self.csra & ADEN != 0
    }

    fn adate(&self) -> bool {
        self.csra & ADATE != 0
    }

    fn adie(&self) -> bool {
        self.csra & ADIE != 0
    }

    /// §11.1 and §24.4. A prescaler that was already running restarts at the
    /// release. `ADEN` clear holds the counter reset, so a fresh device leaves
    /// `origin` at `u64::MAX` until the program sets `ADEN`.
    pub(crate) fn release_epoch(&mut self, now: u64) {
        if self.origin != u64::MAX {
            self.origin = now;
        }
    }

    pub(crate) fn set_clock_gate(&mut self, io_adc: Option<bool>, prr: Option<bool>, cycle: u64) {
        let delta = Gate {
            clock_on: &mut self.clock_on,
            prr_off: &mut self.prr_off,
            frozen_at: &mut self.frozen_at,
        }
        .set(io_adc, prr, cycle);
        if let Some(delta) = delta {
            self.origin = super::shift_deadline(self.origin, delta);
            self.start_at = super::shift_deadline(self.start_at, delta);
            self.sample_at = super::shift_deadline(self.sample_at, delta);
            self.done_at = super::shift_deadline(self.done_at, delta);
            self.recompute_next();
        } else if !self.running() {
            self.next = u64::MAX;
        }
    }

    pub(crate) fn channel_pin(&self, channel: u8) -> Option<u8> {
        let pin = self.pin.get(usize::from(channel)).copied()?;
        if pin == 0xFF { None } else { Some(pin) }
    }

    pub(crate) fn sample_due(&self, at: u64) -> bool {
        self.busy && !self.sampled && self.sample_at == at
    }

    /// Channel and `REFS` locked at the start of this conversion.
    pub(crate) fn latched_sense(&self) -> (Sense, u8) {
        (sense_of(self.latched & 0x0F, &self.pin), self.latched >> 6)
    }

    pub(crate) fn hold(&mut self, code: u16) {
        self.held = code;
        self.sampled = true;
        if self.sample_at <= self.done_at {
            self.sample_at = u64::MAX;
        }
        self.recompute_next();
    }

    /// §24.6. Called once the CPU has halted in ADC Noise
    /// Reduction. Single-conversion mode only; auto-trigger is left alone.
    pub(crate) fn noise_reduction(&mut self, cycle: u64) -> bool {
        if !self.running() || !self.aden() || self.busy || self.adate() {
            return false;
        }
        self.arm_single(cycle);
        true
    }

    pub(crate) fn read(&mut self, reg: u8) -> (u8, bool) {
        if !self.running() {
            return (self.peek(reg), false);
        }
        match reg {
            REG_ADCL => (self.read_adcl(), false),
            REG_ADCH => (self.read_adch(), false),
            _ => (self.peek(reg), false),
        }
    }

    /// `ADSC` and `ADIF` change at the conversion event. Not `ADCL` or `ADCH`, whose reads lock and
    /// unlock the data pair (§24.9.3).
    pub(crate) fn steady(&self, reg: u8) -> bool {
        matches!(reg, REG_ADCSRA | REG_ADCSRB | REG_ADMUX | REG_DIDR0)
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            REG_ADCL => self.present(self.shown()).0,
            REG_ADCH => self.present(self.shown()).1,
            REG_ADCSRA => self.csra_byte(),
            REG_ADCSRB => self.adcsrb,
            REG_ADMUX => self.admux,
            REG_DIDR0 => self.didr0,
            _ => 0,
        }
    }

    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        cycle: u64,
    ) -> Touch {
        if !self.running() {
            return Touch::default();
        }
        self.apply(reg, value, mux, ctx, cycle)
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        if self.adif && self.adie() {
            1u64 << self.vec
        } else {
            0
        }
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        if vector == self.vec {
            self.adif = false;
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        self.next
    }

    pub(crate) fn fire(&mut self, at: u64) {
        // §24.5. The channel locks at the start edge, before sample-and-hold.
        if self.start_at == at {
            self.latched = self.admux;
            self.start_at = u64::MAX;
        }
        if self.sample_due(at) {
            // The bus latches the code before `fire` when it can. A sample
            // with no latch (aborted setup) holds 0.
            self.hold(0);
        }
        if self.done_at == at {
            self.complete(at);
        }
        self.recompute_next();
    }

    pub(crate) fn notice_waits(&self) -> bool {
        !self.notice.is_empty()
    }

    pub(crate) fn take_notice(&mut self) -> Notice {
        let notice = self.notice;
        self.notice = Notice::default();
        notice
    }

    /// Rising edges of Table 24-6 flags. A flag that is already 1 does not
    /// arrive here, so it does not start another conversion.
    pub(crate) fn on_edges(&mut self, rose: u8, cycle: u64) {
        if !self.adate() || self.adts() == 0 || !self.aden() {
            return;
        }
        let mask = 1u8 << self.adts();
        if rose & mask != 0 {
            self.start_auto(cycle);
        }
    }

    /// §24.9.4. An `ADTS` change from a cleared flag onto a set flag is an edge.
    /// Enabling `ADATE` while the flag is already 1 is not: that edge is in the
    /// past. Switching to free running is not an edge either.
    pub(crate) fn retarget(&mut self, level: u8, cycle: u64, synthesize: bool, old_adts: u8) {
        let old_high = self.adate() && old_adts != 0 && level & (1u8 << old_adts) != 0;
        let new_high = self.adate() && self.adts() != 0 && level & (1u8 << self.adts()) != 0;
        if synthesize && !old_high && new_high {
            self.start_auto(cycle);
        }
    }

    fn apply(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        cycle: u64,
    ) -> Touch {
        let mut touch = Touch {
            irq: true,
            schedule: true,
            ..Touch::default()
        };
        match reg {
            REG_ADCSRA => self.write_csra(value, cycle),
            REG_ADCSRB => self.write_csrb(value),
            REG_ADMUX => {
                // Bit 4 is reserved and logged from the chip-table mask.
                self.admux = value & !0x10;
                self.mark_mux();
            }
            REG_DIDR0 => {
                self.didr0 = value & 0x3F;
                self.apply_didr(mux, ctx, cycle);
            }
            REG_ADCL | REG_ADCH => touch.schedule = false,
            _ => {
                touch.schedule = false;
                touch.irq = false;
            }
        }
        touch
    }

    fn write_csra(&mut self, value: u8, cycle: u64) {
        if value & ADIF != 0 {
            self.adif = false;
        }
        let old = self.csra;
        self.div = DIV[usize::from(value & 0x07)];
        self.csra = value & (ADEN | ADATE | ADIE | 0x07);
        if self.csra & ADEN != old & ADEN {
            if self.aden() {
                self.origin = cycle;
                self.init_pending = true;
            } else {
                self.shutdown();
            }
        }
        if value & ADSC != 0 {
            self.arm_single(cycle);
        }
        if self.csra & ADATE != old & ADATE {
            self.notice.retarget = true;
        }
        self.mark_mux();
    }

    fn write_csrb(&mut self, value: u8) {
        let old_adts = self.adts();
        let next = value & !0xB8;
        let adts_changed = next & 0x07 != self.adcsrb & 0x07;
        let acme_changed = next & ACME != self.adcsrb & ACME;
        self.adcsrb = next;
        if adts_changed {
            self.notice.retarget = true;
            self.notice.synthesize = true;
            self.notice.old_adts = old_adts;
        }
        if acme_changed || adts_changed {
            self.mark_mux();
        }
    }

    fn mark_mux(&mut self) {
        self.notice.mux = true;
        self.notice.acme = self.adcsrb & ACME != 0;
        self.notice.aden = self.aden();
        self.notice.channel = self.admux & 0x0F;
    }

    fn arm_single(&mut self, cycle: u64) {
        if !self.aden() || self.busy {
            return;
        }
        // The following rising edge after `ADSC`. The edge on the write cycle
        // has already happened.
        let start = self.next_edge(cycle, false);
        self.arm_from_edge(start, cycle);
    }

    fn start_auto(&mut self, cycle: u64) {
        if !self.aden() || self.busy || !self.adate() || self.adts() == 0 {
            return;
        }
        // §24.4. The trigger resets the prescaler.
        self.origin = cycle;
        let div = self.divisor();
        let first = self.init_pending;
        self.init_pending = false;
        // First ADC clock the synchronized logic can use. For `div >= 8` this
        // is `trigger + div/2`, which is after the 3-clock sync.
        let start = self.next_edge(cycle.saturating_add(AUTO_SYNC), true);
        self.busy = true;
        self.sampled = false;
        self.held = 0;
        self.note_start(start, cycle);
        if first {
            // Table 24-1's first row still applies: 13.5 and 25 from that edge.
            // The prose below is the auto-triggered row, after initialization.
            self.sample_at = start.saturating_add(27 * div / 2);
            self.done_at = start.saturating_add(25 * div);
        } else {
            // §24.4 text, which Figure 24-6 agrees with. Counting 2 and 13.5
            // ADC clocks from `start` instead lands later whenever `div/2`
            // is not exactly the 3-clock sync (`div` above 2). The sentence
            // is the one scheduled here.
            self.sample_at = cycle.saturating_add(AUTO_SYNC + 2 * div);
            self.done_at = cycle.saturating_add(AUTO_SYNC + 27 * div / 2);
        }
        self.recompute_next();
    }

    /// Single or free-running. Offsets are Table 24-1 from the start edge.
    fn arm_from_edge(&mut self, start: u64, now: u64) {
        let first = self.init_pending;
        self.init_pending = false;
        let div = self.divisor();
        let (sample_half, total_half) = if first { (27_u64, 50) } else { (3, 26) };
        self.sample_at = start.saturating_add(sample_half * div / 2);
        self.done_at = start.saturating_add(total_half * div / 2);
        self.sampled = false;
        self.held = 0;
        self.busy = true;
        self.note_start(start, now);
        self.recompute_next();
    }

    /// §24.5. A start edge in the future waits. Free running starts on the
    /// completion cycle, which is already now, so the channel locks here.
    fn note_start(&mut self, start: u64, now: u64) {
        if start <= now {
            self.latched = self.admux;
            self.start_at = u64::MAX;
        } else {
            self.start_at = start;
        }
    }

    fn complete(&mut self, at: u64) {
        self.done_at = u64::MAX;
        self.sample_at = u64::MAX;
        if self.locked {
            // §24.9.3. The result is lost. `ADIF` still sets.
        } else {
            self.result = if self.sampled { self.held } else { 0 };
        }
        self.sampled = false;
        self.adif = true;
        let chain = self.aden() && self.adate() && self.adts() == 0;
        if chain {
            // Figure 24-7. The completion edge is the next conversion's start.
            // The prescaler is not reset. Timing is the normal 13, not 13.5.
            self.busy = false;
            self.arm_from_edge(at, at);
            self.busy = true;
        } else {
            self.busy = false;
        }
    }

    fn shutdown(&mut self) {
        self.busy = false;
        self.sampled = false;
        self.init_pending = true;
        self.origin = u64::MAX;
        self.start_at = u64::MAX;
        self.sample_at = u64::MAX;
        self.done_at = u64::MAX;
        self.csra &= !ADEN;
        self.next = u64::MAX;
    }

    fn divisor(&self) -> u64 {
        u64::from(self.div.max(2))
    }

    /// Smallest Figure 24-3 rising edge at `after`, or strictly after it.
    ///
    /// The counter is 0 at `origin`. Output bit `k` (`CK/2^(k+1)`) first
    /// rises `div/2` CPU clocks later, then once per `div`.
    fn next_edge(&self, after: u64, inclusive: bool) -> u64 {
        let div = self.divisor();
        let origin = self.origin;
        if origin == u64::MAX {
            return u64::MAX;
        }
        let first = origin.saturating_add(div / 2);
        let target = if inclusive {
            after
        } else {
            after.saturating_add(1)
        };
        if target <= first {
            return first;
        }
        let steps = (target - first).div_ceil(div);
        first + steps * div
    }

    fn recompute_next(&mut self) {
        if !self.running() {
            self.next = u64::MAX;
            return;
        }
        self.next = self.start_at.min(self.sample_at).min(self.done_at);
    }

    fn adts(&self) -> u8 {
        self.adcsrb & 0x07
    }

    fn csra_byte(&self) -> u8 {
        let mut value = self.csra & (ADATE | ADIE | 0x07);
        if self.aden() {
            value |= ADEN;
        }
        if self.busy {
            value |= ADSC;
        }
        if self.adif {
            value |= ADIF;
        }
        value
    }

    fn shown(&self) -> u16 {
        if self.locked {
            self.locked_value
        } else {
            self.result
        }
    }

    fn read_adcl(&mut self) -> u8 {
        if !self.locked {
            self.locked_value = self.result;
            self.locked = true;
        }
        self.present(self.locked_value).0
    }

    fn read_adch(&mut self) -> u8 {
        let value = self.shown();
        self.locked = false;
        self.present(value).1
    }

    /// `ADLAR` applies at the read (§24.9.1), including while the pair is locked.
    fn present(&self, value: u16) -> (u8, u8) {
        if self.admux & ADLAR != 0 {
            let shifted = value << 6;
            (shifted as u8, (shifted >> 8) as u8)
        } else {
            (value as u8, (value >> 8) as u8)
        }
    }

    fn apply_didr(&self, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        for channel in 0..6u8 {
            let Some(pin) = self.pin.get(usize::from(channel)).copied() else {
                continue;
            };
            if pin == 0xFF {
                continue;
            }
            let off = self.didr0 & (1 << channel) != 0;
            mux.set_buffer_disabled(pin, off, ctx, at);
        }
    }
}

pub(crate) fn sense_of(mux: u8, pin: &[u8; 8]) -> Sense {
    match mux & 0x0F {
        channel @ 0..=7 => {
            let pad = pin[usize::from(channel)];
            if pad == 0xFF {
                Sense::Only(channel)
            } else {
                Sense::Pin(pad)
            }
        }
        0x08 => Sense::Temp,
        0x0E => Sense::Bandgap,
        0x0F => Sense::Ground,
        _ => Sense::Ground,
    }
}
