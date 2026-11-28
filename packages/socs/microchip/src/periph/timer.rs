//! Timer/Counter 0, 1, and 2, one row per instance.
//!
//! The counter is a function of the prescaler. [`Timer::next_event`] is the
//! cycle of the next compare, TOP, or BOTTOM. A read of `TCNT` projects the
//! count; clocks with no flag and no pin change are not stepped one by one.
//! An event inside an instruction still runs on that cycle.
//!
//! In a dual-slope mode, a count above `TOP` keeps its direction. It wraps
//! at the register width (`0xFF` or `0xFFFF` becomes 0) and only then uses
//! the triangle inside `0..=TOP`. That wrap is not BOTTOM: §15.7.4 and
//! §16.9.4 set `TOV` at BOTTOM, after the count comes down to 0, and neither
//! section names a flag for the width wrap. §15.8 draws the `MAX`→0 overflow for
//! every mode except phase correct. The `OCnx` register changes on a compare
//! match, at BOTTOM in the fast-PWM tables, at TOP and at BOTTOM in the
//! phase-correct tables, and on `FOCnx` in a non-PWM mode. A missed up-count
//! takes the up-count result on the clock that leaves TOP (Figure 15-7,
//! §15.7.4, Figure 16-12).
//! The pin is not recomputed from `TCNT < OCR`.
//!
//! A 16-bit timer is the same row as an 8-bit one. `ICP1` and the external
//! `T0`/`T1` clock count from pad edges. The synchronizer and edge detector
//! delay that count by 4 cycles, the first boundary past the 2.5-to-3.5-cycle
//! window in §17.3. The noise canceler adds another 4 (§16.6.2).

use crate::Level;
use crate::chip::{TimerCfg, TimerClock, TimerWidth};
use crate::mux::{DriveSource, PinMux};
use crate::regs::{
    ASSR, ICR_H, ICR_L, OCR_A, OCR_AH, OCR_B, OCR_BH, TCCR_A, TCCR_B, TCCR_C, TCNT_H, TCNT_L, TIFR,
    TIMSK,
};

use super::prescaler::ClockView;
use super::{Ctx, Notice, StoreAt, Touch};

/// Queues that are empty on the fast path. One allocation per timer.
#[derive(Debug, Clone)]
struct EdgeQs {
    cap: [u64; 4],
    ext: [u64; 4],
}

/// `TIFRn.TOVn`, bit 0.
const TOV: u8 = 1 << 0;
/// `TIFRn.OCFnA` is bit 1. The next compare unit is the next bit.
const OCF_A: u8 = 1 << 1;
/// `TIFRn.ICFn`, bit 5.
const ICF: u8 = 1 << 5;
/// §17.3 and §16.6.1. The first cycle boundary past the 3.5-clock maximum.
const EXT_SAMPLE_DELAY: u64 = 4;
/// §16.6.2. Four equal samples, then the same edge detector as `T1`.
const NOISE_SAMPLES: u64 = 4;
/// `ASSR.EXCLK` and `ASSR.AS2`. The busy bits stay clear on the system clock.
const ASSR_CLOCK_SELECT: u8 = (1 << 6) | (1 << 5);
/// `TCCRnA` bits 3:2 are reserved and read as zero.
const TCCRA_MASK: u8 = 0xF3;
/// Compare units A and B.
const COMPARES: usize = 2;
/// `TCCRnB` for a 16-bit timer. Bit 5 is reserved. `FOC` lives in `TCCRnC`.
const TCCRB_16: u8 = 0xDF;
/// `TCCRnB` for an 8-bit timer: `WGMn2` and the clock select. `FOC` reads as zero.
const TCCRB_8: u8 = 0x0F;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Normal,
    Ctc,
    Fast,
    Phase,
    /// Phase and frequency correct. OCR updates at BOTTOM, like phase correct's TOP.
    PhaseFreq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopSrc {
    Max,
    Fixed(u16),
    Ocra,
    Icr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Update {
    Immediate,
    Top,
    Bottom,
}

/// A `TCCRnB` store and the edge that ends its instruction. The timer clock
/// on that edge counts with the old value (model parameter 1).
#[derive(Debug, Clone, Copy)]
struct Select {
    value: u8,
    at: u64,
}

#[derive(Debug, Clone, Copy)]
struct Wave {
    reserved: bool,
    kind: Kind,
    top: TopSrc,
    update: Update,
    /// `true` when `TOV` is the wrap from `MAX` to 0. Fast PWM uses `false` and
    /// sets `TOV` on the clock that reaches TOP.
    tov_on_max: bool,
    /// `COM=1` toggles channel A. Other channels stay disconnected.
    toggle_a: bool,
}

#[derive(Debug)]
pub(crate) struct Timer {
    cfg: TimerCfg,
    index: u8,
    tccra: u8,
    tccrb: u8,
    tcnt: u16,
    icr: u16,
    /// Compare value the waveform uses.
    ocr: [u16; COMPARES],
    /// Buffer the CPU writes in a PWM mode.
    buf: [u16; COMPARES],
    timsk: u8,
    tifr: u8,
    /// `ASSR` bits that select the asynchronous clock. Busy bits stay clear.
    assr: u8,
    temp: u8,
    up: bool,
    /// Divisor, or 0 when stopped or on the external pin.
    div: u16,
    external: bool,
    /// Last digital level on the `T0`/`T1` pin.
    t_prev: bool,
    /// Last digital level on `ICP1`, before the noise canceler.
    icp_prev: bool,
    /// `ACIC` selects the comparator instead of `ICP1` (§23.3.2).
    acic: bool,
    /// Last synchronized comparator level fed to the capture front-end.
    aco_level: bool,
    /// Noise-canceler output. The edge detector watches this when `ICNC1` is set.
    filtered: bool,
    filter_level: bool,
    filter_due: u64,
    /// Extra delay after the noise canceler. The pin adds §17.3; `ACO` does not.
    filter_extra: u64,
    /// Capture and external-clock times. Off the enum so `Device` keeps its
    /// size: the hot path only follows the pointer while a length is non-zero.
    edges: Box<EdgeQs>,
    cap_len: u8,
    ext_len: u8,
    /// Bit 0 is `clkI/O` running. Bit 1 is `PRR` stopped (§10.9, §10.11.3).
    gate: u8,
    /// Next prescaler tick. [`Timer::next`] is the soonest of every timer event.
    count_next: u64,
    select: Option<Select>,
    /// The next counted clock skips the compare match (§15.5.2).
    block: bool,
    /// Clocks inside the instruction that wrote `TCNT` do not count (§15.4).
    hold_write: bool,
    /// Internal `OCnx` register, one bit per compare unit.
    oc: u8,
    /// Compare units currently overriding the pin.
    linked: u8,
    /// CPU cycle at which `tcnt` and `up` were last exact.
    anchor: u64,
    /// Next CPU cycle with a compare, TOP, or BOTTOM event.
    next: u64,
    /// Table 24-6 flag edges since the bus last drained them.
    rose: u8,
    fell: u8,
    /// Test reference: every timer clock is an event, so nothing is projected.
    #[cfg(test)]
    stepped: bool,
}

clone_fields!(Timer {
    cfg,
    index,
    tccra,
    tccrb,
    tcnt,
    icr,
    ocr,
    buf,
    timsk,
    tifr,
    assr,
    temp,
    up,
    div,
    external,
    t_prev,
    icp_prev,
    acic,
    aco_level,
    filtered,
    filter_level,
    filter_due,
    filter_extra,
    edges,
    cap_len,
    ext_len,
    gate,
    count_next,
    select,
    block,
    hold_write,
    oc,
    linked,
    anchor,
    next,
    rose,
    fell,
    #[cfg(test)]
    stepped
});

impl Timer {
    fn io_clock(&self) -> bool {
        self.gate & 1 != 0
    }

    fn prr_off(&self) -> bool {
        self.gate & 2 != 0
    }

    pub(crate) fn new(cfg: TimerCfg, index: u8) -> Self {
        Self {
            cfg,
            index,
            tccra: 0,
            tccrb: 0,
            tcnt: 0,
            icr: 0,
            ocr: [0; COMPARES],
            buf: [0; COMPARES],
            timsk: 0,
            tifr: 0,
            assr: 0,
            temp: 0,
            up: true,
            div: 0,
            external: false,
            t_prev: false,
            icp_prev: false,
            acic: false,
            aco_level: false,
            filtered: false,
            filter_level: false,
            filter_due: u64::MAX,
            filter_extra: 0,
            edges: Box::new(EdgeQs {
                cap: [0; 4],
                ext: [0; 4],
            }),
            cap_len: 0,
            ext_len: 0,
            gate: 0x01,
            count_next: u64::MAX,
            select: None,
            block: false,
            hold_write: false,
            oc: 0,
            linked: 0,
            anchor: 0,
            next: u64::MAX,
            rose: 0,
            fell: 0,
            #[cfg(test)]
            stepped: false,
        }
    }

    /// Count by `step` on every timer clock. The projection tests compare this
    /// reference with the scheduler.
    #[cfg(test)]
    pub(crate) fn arm_stepped(&mut self, clock: &ClockView) {
        self.stepped = true;
        self.reschedule(clock);
    }

    /// Projected `TCNT` and the visible flag byte, for the stepped comparison.
    #[cfg(test)]
    pub(crate) fn observe(&self, cycle: u64, clock: &ClockView) -> (u16, u8) {
        (self.tcnt_at(cycle, clock), self.tifr & self.flag_mask())
    }

    pub(crate) fn next_event(&self) -> u64 {
        self.next
    }

    pub(crate) fn read(&mut self, reg: u8, cycle: u64, clock: &ClockView) -> u8 {
        // §10.9. A frozen module's read has no side effect, so `TEMP` stays.
        if self.prr_off() || !self.io_clock() {
            return self.peek(reg, cycle, clock);
        }
        if self.is16() && reg == TCNT_L {
            let value = self.tcnt_at(cycle, clock);
            self.temp = (value >> 8) as u8;
            return value as u8;
        }
        if self.is16() && reg == ICR_L {
            self.temp = (self.icr >> 8) as u8;
            return self.icr as u8;
        }
        self.peek(reg, cycle, clock)
    }

    /// A load that writes nothing, as the byte [`Self::peek`] returns at `cycle`.
    ///
    /// A running 16-bit timer's `TCNTnL` or `ICRnL` latches `TEMP`, so that load is not here.
    /// An 8-bit `TCNTnL` only projects the count.
    pub(crate) fn pure(&self, reg: u8, cycle: u64, clock: &ClockView) -> Option<u8> {
        let latches = self.is16() && (reg == TCNT_L || reg == ICR_L);
        if latches && !self.prr_off() && self.io_clock() {
            return None;
        }
        if reg == TCNT_L || latches {
            Some(self.peek(reg, cycle, clock))
        } else {
            None
        }
    }

    /// Flags rise only when the timer fires, and `TCNTnH` and `ICRnH` read `TEMP`. Not `TCNTnL`,
    /// which counts with the cycle, or `ICRnL`, whose 16-bit read latches `TEMP`.
    pub(crate) fn steady(&self, reg: u8) -> bool {
        matches!(
            reg,
            TCCR_A
                | TCCR_B
                | TCCR_C
                | TCNT_H
                | ICR_H
                | OCR_A
                | OCR_AH
                | OCR_B
                | OCR_BH
                | TIMSK
                | TIFR
                | ASSR
        )
    }

    pub(crate) fn peek(&self, reg: u8, cycle: u64, clock: &ClockView) -> u8 {
        match reg {
            TCCR_A => self.tccra_view(),
            // 8-bit bits 7:6 are `FOC` and read as zero (`TCCRB_8`). On a 16-bit
            // timer those bits are `ICNCn` and `ICESn` (§16.11.2) and read back.
            TCCR_B => self.tccrb & self.tccrb_mask(),
            TCCR_C => 0,
            TCNT_L => self.tcnt_at(cycle, clock) as u8,
            TCNT_H => self.temp,
            ICR_L => self.icr as u8,
            ICR_H => self.temp,
            TIMSK => self.timsk & self.flag_mask(),
            TIFR => self.tifr & self.flag_mask(),
            ASSR => self.assr,
            OCR_A => self.cpu_ocr(0) as u8,
            OCR_AH => (self.cpu_ocr(0) >> 8) as u8,
            OCR_B => self.cpu_ocr(1) as u8,
            OCR_BH => (self.cpu_ocr(1) >> 8) as u8,
            _ => 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        ctx: &mut Ctx<'_>,
        mux: &mut PinMux,
        StoreAt { cycle, end }: StoreAt,
        clock: &ClockView,
    ) -> Touch {
        // §10.9. The module is frozen. The store is discarded.
        if self.prr_off() {
            return Touch::default();
        }
        if reg != TCNT_L && reg != TCNT_H {
            self.materialize(cycle, clock);
        }
        let mut touch = Touch {
            schedule: true,
            ..Touch::default()
        };
        match reg {
            TCCR_A => {
                self.tccra = value & TCCRA_MASK;
                self.publish(mux, ctx, None, cycle);
            }
            TCCR_B => {
                self.select = Some(Select { value, at: end });
            }
            TCCR_C => {
                let wave = self.wave();
                self.force(value, wave);
                self.publish(mux, ctx, None, cycle);
            }
            TCNT_L => {
                let written = if self.is16() {
                    u16::from(self.temp) << 8 | u16::from(value)
                } else {
                    u16::from(value)
                };
                self.commit_tcnt(written);
                self.anchor = cycle;
                touch.hold = true;
            }
            TCNT_H | ICR_H | OCR_AH | OCR_BH => {
                self.temp = value;
                touch.schedule = false;
            }
            ICR_L => {
                self.icr = u16::from(self.temp) << 8 | u16::from(value);
            }
            TIMSK => {
                self.timsk = value & self.flag_mask();
                touch.schedule = false;
                touch.irq = true;
            }
            TIFR => {
                self.lower_flags(value & self.flag_mask());
                touch.schedule = false;
                touch.irq = true;
            }
            ASSR => {
                self.assr = value & ASSR_CLOCK_SELECT;
                touch.schedule = false;
            }
            OCR_A => self.write_ocr(0, self.assemble_ocr(value)),
            OCR_B => self.write_ocr(1, self.assemble_ocr(value)),
            _ => touch.schedule = false,
        }
        if touch.schedule {
            self.reschedule(clock);
        }
        touch
    }

    pub(crate) fn poke(&mut self, reg: u8, value: u8, cycle: u64, clock: &ClockView) -> Touch {
        if self.prr_off() {
            return Touch::default();
        }
        if !matches!(reg, TCNT_L | TCNT_H | OCR_AH | OCR_BH | ICR_H) {
            self.materialize(cycle, clock);
        }
        let mut schedule = true;
        match reg {
            TCCR_A => self.tccra = value & TCCRA_MASK,
            TCCR_B => {
                self.tccrb = value & self.tccrb_mask();
                self.apply_cs(value);
            }
            TCNT_L => {
                self.tcnt = if self.is16() {
                    u16::from(self.temp) << 8 | u16::from(value)
                } else {
                    u16::from(value)
                };
                self.anchor = cycle;
            }
            TCNT_H | ICR_H | OCR_AH | OCR_BH => {
                self.temp = value;
                schedule = false;
            }
            ICR_L => self.icr = u16::from(self.temp) << 8 | u16::from(value),
            TIMSK => {
                self.timsk = value & self.flag_mask();
                schedule = false;
            }
            TIFR => {
                self.plant_flags(value & self.flag_mask());
                schedule = false;
            }
            OCR_A => self.plant_ocr(0, self.assemble_ocr(value)),
            OCR_B => self.plant_ocr(1, self.assemble_ocr(value)),
            _ => schedule = false,
        }
        if schedule {
            self.reschedule(clock);
        }
        Touch {
            schedule,
            irq: matches!(reg, TIMSK | TIFR),
            ..Touch::default()
        }
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        let pending = self.tifr & self.timsk & self.flag_mask();
        if pending == 0 {
            return 0;
        }
        let mut bits = 0u64;
        if pending & TOV != 0 {
            bits |= 1u64 << self.cfg.ovf_vec;
        }
        for (unit, vector) in self.cfg.comp_vec.into_iter().enumerate() {
            if pending & (OCF_A << unit) != 0 {
                bits |= 1u64 << vector;
            }
        }
        if pending & ICF != 0 && self.cfg.capt_vec != 0 {
            bits |= 1u64 << self.cfg.capt_vec;
        }
        bits
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        if vector == self.cfg.ovf_vec {
            self.lower_flags(TOV);
        }
        for unit in 0..COMPARES {
            if self.cfg.comp_vec[unit] == vector {
                self.lower_flags(OCF_A << unit);
            }
        }
        if self.cfg.capt_vec != 0 && vector == self.cfg.capt_vec {
            self.lower_flags(ICF);
        }
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

    /// `ACIC` in `ACSR`. Enabling copies the current `ACO` and does not capture.
    /// Disabling does not capture either.
    pub(crate) fn set_acic(&mut self, on: bool, level: bool) {
        if on && !self.acic {
            self.aco_level = level;
        }
        self.acic = on;
    }

    /// Synchronized comparator level. Stored whether or not `ACIC` is set, so
    /// a later enable does not compare against a stale reset level. The pin
    /// synchronizer is not added again; the noise canceler and `ICES1` still
    /// apply (§23.3.2).
    pub(crate) fn on_aco(&mut self, high: bool, at: u64) -> bool {
        let prev = self.aco_level;
        self.aco_level = high;
        if !self.acic || self.icp_disconnected() || !self.io_clock() {
            return false;
        }
        self.detect_edge(prev, high, at, 0)
    }

    pub(crate) fn materialize(&mut self, cycle: u64, clock: &ClockView) {
        if self.hold_write || cycle <= self.anchor {
            return;
        }
        let n = self.ticks_between(clock, self.anchor, cycle);
        if n > 0 {
            let (count, up) = self.project(n);
            self.tcnt = count;
            self.up = up;
        }
        self.anchor = cycle;
    }

    pub(crate) fn release_hold(&mut self, end: u64, clock: &ClockView) {
        if !self.hold_write {
            return;
        }
        self.hold_write = false;
        self.anchor = end;
        self.reschedule(clock);
    }

    /// Run every event scheduled at `at`: a noise-canceler sample, a capture,
    /// an external-clock tick, or a prescaler tick.
    pub(crate) fn fire(&mut self, at: u64, clock: &ClockView, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if self.filter_due == at {
            self.apply_filter(at);
        }
        while self.cap_len > 0 && self.edges.cap[0] == at {
            pop_sorted(&mut self.edges.cap, &mut self.cap_len);
            self.apply_capture(at, clock);
        }
        let mut ext_ticks = 0u8;
        while self.ext_len > 0 && self.edges.ext[0] == at {
            pop_sorted(&mut self.edges.ext, &mut self.ext_len);
            ext_ticks += 1;
        }
        for _ in 0..ext_ticks {
            self.on_tick(mux, ctx, at);
            self.anchor = at;
        }
        if !self.hold_write && self.count_next == at {
            let n = self.ticks_between(clock, self.anchor, at);
            if n > 0 {
                if n > 1 {
                    let (count, up) = self.project(n - 1);
                    self.tcnt = count;
                    self.up = up;
                }
                self.on_tick(mux, ctx, at);
                self.anchor = at;
            }
        }
        if let Some(select) = self.select.filter(|select| select.at == at) {
            self.select = None;
            self.materialize(at, clock);
            self.apply_tccrb(select.value, mux, ctx, at);
        }
        self.reschedule(clock);
        if self.next <= at {
            self.next = u64::MAX;
        }
    }

    /// `io` is `clkI/O` running. `prr` is the power-reduction bit stopping the timer.
    /// `None` leaves that half unchanged. The count freezes across a gap (§10).
    pub(crate) fn apply_gate(
        &mut self,
        io: Option<bool>,
        prr: Option<bool>,
        cycle: u64,
        clock: &ClockView,
    ) {
        let was = self.io_clock() && !self.prr_off();
        if was {
            let n = self.ticks_between(clock, self.anchor, cycle);
            if n > 0 {
                let (count, up) = self.project(n);
                self.tcnt = count;
                self.up = up;
            }
            self.anchor = cycle;
        }
        if let Some(running) = io {
            self.gate = (self.gate & !1) | u8::from(running);
        }
        if let Some(stopped) = prr {
            self.gate = (self.gate & !2) | (u8::from(stopped) << 1);
        }
        if self.io_clock() && !self.prr_off() && !was {
            self.anchor = cycle;
        }
        self.reschedule(clock);
    }

    /// A pad edge. The delay is applied here; the counter moves in [`Timer::fire`].
    pub(crate) fn on_pad(&mut self, pin: u8, level: Level, at: u64) -> Touch {
        let high = level == Level::High;
        let mut schedule = false;
        if self.cfg.t_pin == Some(pin) {
            let rising = !self.t_prev && high;
            let falling = self.t_prev && !high;
            self.t_prev = high;
            let cs = self.tccrb & 0x07;
            let selected = (cs == 7 && rising) || (cs == 6 && falling);
            if self.external && self.io_clock() && selected {
                push_sorted(
                    &mut self.edges.ext,
                    &mut self.ext_len,
                    at.saturating_add(EXT_SAMPLE_DELAY),
                );
                schedule = true;
            }
        }
        if self.cfg.icp_pin == Some(pin) && self.io_clock() {
            schedule |= self.note_icp(high, at);
        }
        Touch {
            schedule,
            ..Touch::default()
        }
    }

    /// One timer clock at `cycle`.
    ///
    /// Each event acts on the clock that leaves the count it names, with the
    /// count, compare values and direction from before that clock (model
    /// parameter 4). A compare match is the clock that leaves `OCR`: it raises
    /// `OCFnx` and writes `OCnx` together (Figure 15-10, Figure 15-6). TOP is the
    /// clock that leaves TOP and BOTTOM the clock that leaves 0, so the buffer
    /// copy, `TOV` and the `ICF1` of `ICR1` as TOP land with it (Figure 16-12).
    fn on_tick(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, cycle: u64) {
        let wave = self.wave();
        if wave.reserved {
            return;
        }
        let top = self.top_value(wave);
        let prev = self.tcnt;
        let max = self.max();
        let dual = matches!(wave.kind, Kind::Phase | Kind::PhaseFreq);
        let (new, up) = predict_step(wave.kind, self.up, top, max, prev);
        let left_top = prev == top && wave.kind != Kind::Normal;
        // A dual-slope count turns up at 0. The width wrap lands on 0 still
        // counting up and is not BOTTOM.
        let left_bottom = dual && prev == 0 && !self.up;

        let blocked = self.block;
        self.block = false;
        let mut matched = 0u8;
        if !blocked {
            for unit in 0..COMPARES {
                if prev == self.ocr[unit] {
                    matched |= 1 << unit;
                }
            }
        }

        let copy = match wave.update {
            Update::Immediate => false,
            Update::Top => left_top,
            Update::Bottom => (left_top && !dual) || (wave.kind == Kind::PhaseFreq && left_bottom),
        };
        if copy {
            self.ocr = self.buf;
        }

        self.tcnt = new;
        self.up = up;

        let tov = if wave.tov_on_max {
            prev == max && new == 0
        } else if dual {
            left_bottom
        } else {
            left_top
        };
        if tov {
            self.raise_flags(TOV);
        }
        if self.is16() && matches!(wave.top, TopSrc::Icr) && left_top {
            self.raise_flags(ICF);
        }
        self.raise_flags(matched << 1);

        let pwm = matches!(wave.kind, Kind::Fast | Kind::Phase | Kind::PhaseFreq);
        for unit in 0..COMPARES {
            if matched & (1 << unit) != 0 && (!pwm || self.com(unit) == 1) {
                self.match_edge(unit, wave);
            }
        }

        // COM = 0 does not write OCnx (§15.6.1, §16.8.1, §18.6.1). A channel
        // that is still linked has to publish the release on this clock.
        if self.output_live() {
            if pwm {
                let edge = Edge {
                    left_top,
                    left_bottom,
                    up,
                };
                self.apply_pwm(wave.kind, top, matched, edge);
            }
            self.publish(mux, ctx, Some(cycle), cycle);
        }
    }

    fn tcnt_at(&self, cycle: u64, clock: &ClockView) -> u16 {
        if self.hold_write || cycle <= self.anchor {
            return self.tcnt;
        }
        let n = self.ticks_between(clock, self.anchor, cycle);
        if n == 0 { self.tcnt } else { self.project(n).0 }
    }

    fn ticking(&self, clock: &ClockView) -> bool {
        self.io_clock()
            && !self.prr_off()
            && self.div != 0
            && !self.external
            && !self.hold_write
            && !clock.held(self.cfg.clock)
            && !self.wave().reserved
    }

    fn ticks_between(&self, clock: &ClockView, from: u64, to: u64) -> u64 {
        if !self.ticking(clock) || to <= from {
            return 0;
        }
        let div = u64::from(self.div);
        let origin = clock.origin(self.cfg.clock);
        let from_count = from.wrapping_sub(origin);
        let to_count = to.wrapping_sub(origin);
        let first = from_count / div + 1;
        let last = to_count / div;
        if last < first { 0 } else { last - first + 1 }
    }

    pub(crate) fn reschedule(&mut self, clock: &ClockView) {
        if !self.ticking(clock) {
            self.count_next = u64::MAX;
        } else {
            let n = self.clocks_until_event();
            self.count_next = cycle_of_tick(
                clock.origin(self.cfg.clock),
                u64::from(self.div),
                self.anchor,
                n,
            );
        }
        let mut next = self.count_next;
        if self.ext_len > 0 {
            next = next.min(self.edges.ext[0]);
        }
        if self.cap_len > 0 {
            next = next.min(self.edges.cap[0]);
        }
        next = next.min(self.filter_due);
        if let Some(select) = self.select {
            next = next.min(select.at);
        }
        self.next = next;
    }

    /// Table 24-6 bits named by this chip-table row. Position in the device
    /// list does not matter.
    fn map_trigger(&self, flags: u8) -> u8 {
        let mut out = 0u8;
        for packed in self.cfg.adc_trigger.into_iter().flatten() {
            let bit = packed & 0x0F;
            let adts = packed >> 4;
            if flags & (1u8 << bit) != 0 {
                out |= 1u8 << adts;
            }
        }
        out
    }

    fn raise_flags(&mut self, mask: u8) {
        let up = mask & !self.tifr;
        self.tifr |= mask;
        if up != 0 {
            self.rose |= self.map_trigger(up);
        }
    }

    fn lower_flags(&mut self, mask: u8) {
        let down = self.tifr & mask;
        self.tifr &= !mask;
        if down != 0 {
            self.fell |= self.map_trigger(down);
        }
    }

    fn plant_flags(&mut self, next: u8) {
        let up = next & !self.tifr;
        let down = self.tifr & !next;
        self.tifr = next;
        if up != 0 {
            self.rose |= self.map_trigger(up);
        }
        if down != 0 {
            self.fell |= self.map_trigger(down);
        }
    }

    fn note_icp(&mut self, high: bool, at: u64) -> bool {
        if self.acic || self.icp_disconnected() {
            self.icp_prev = high;
            return false;
        }
        let prev = self.icp_prev;
        self.icp_prev = high;
        self.detect_edge(prev, high, at, EXT_SAMPLE_DELAY)
    }

    fn detect_edge(&mut self, prev: bool, high: bool, at: u64, extra: u64) -> bool {
        if self.tccrb & 0x80 != 0 {
            self.filter_level = high;
            self.filter_due = at.saturating_add(NOISE_SAMPLES);
            self.filter_extra = extra;
            return true;
        }
        let rising = self.tccrb & 0x40 != 0;
        let edge = if rising { !prev && high } else { prev && !high };
        if edge {
            push_sorted(
                &mut self.edges.cap,
                &mut self.cap_len,
                at.saturating_add(extra),
            );
            true
        } else {
            false
        }
    }

    fn apply_filter(&mut self, at: u64) {
        self.filter_due = u64::MAX;
        if self.icp_disconnected() || self.tccrb & 0x80 == 0 {
            return;
        }
        if self.filter_level == self.filtered {
            return;
        }
        let was = self.filtered;
        self.filtered = self.filter_level;
        let rising = self.tccrb & 0x40 != 0;
        let edge = if rising {
            !was && self.filtered
        } else {
            was && !self.filtered
        };
        if edge {
            push_sorted(
                &mut self.edges.cap,
                &mut self.cap_len,
                at.saturating_add(self.filter_extra),
            );
        }
    }

    /// §16.6. `ICR1` is overwritten when a new capture arrives before the
    /// previous one is read. That overwrite has no overflow flag.
    fn apply_capture(&mut self, at: u64, clock: &ClockView) {
        if self.icp_disconnected() {
            return;
        }
        self.materialize(at, clock);
        self.icr = self.tcnt;
        self.raise_flags(ICF);
    }

    fn icp_disconnected(&self) -> bool {
        self.is16() && matches!(self.wave().top, TopSrc::Icr)
    }

    fn clocks_until_event(&self) -> u64 {
        #[cfg(test)]
        if self.stepped {
            return 1;
        }
        if self.block || self.pin_changes_on_next_clock() {
            return 1;
        }
        let wave = self.wave();
        let top = self.top_value(wave);
        match wave.kind {
            Kind::Phase | Kind::PhaseFreq => self.clocks_until_phase(top),
            Kind::Normal | Kind::Ctc | Kind::Fast => self.clocks_until_up(top),
        }
    }

    /// The next timer clock has to run so `publish` sees it.
    ///
    /// A poke of `COM` does not publish. The link bit then disagrees with
    /// the register: the next clock connects the pin or releases it.
    ///
    /// Compare matches, BOTTOM, TOP, and `MAX` are stops in
    /// [`clocks_until_up`] and [`clocks_until_phase`]. `block` returns one
    /// clock before this function runs.
    fn pin_changes_on_next_clock(&self) -> bool {
        if !self.output_live() {
            return false;
        }
        let wave = self.wave();
        if wave.reserved {
            return false;
        }
        for unit in 0..COMPARES {
            let com = self.com(unit);
            let connected = com != 0 && !self.com_disconnected(unit);
            let linked = self.linked & (1 << unit) != 0;
            // A poke stores `COM` without `publish`. The next clock has to
            // connect a new override or release one the register dropped.
            if connected != linked {
                return true;
            }
        }
        false
    }

    /// A channel with `COM` ≠ 0 can still write `OCnx`, and a linked pin has
    /// to be released. Every `COM` = 0 and nothing linked means no output work.
    fn output_live(&self) -> bool {
        if self.linked != 0 {
            return true;
        }
        for unit in 0..COMPARES {
            if self.com(unit) != 0 {
                return true;
            }
        }
        false
    }

    /// Clocks to the next one that leaves an `OCR`, TOP, or 0. The count
    /// reaches the nearest of them ahead in its direction, then leaves it on
    /// the clock after.
    fn clocks_until_phase(&self, top: u16) -> u64 {
        let tcnt = self.tcnt;
        // Above TOP, `step` keeps the direction. A compare, TOP, BOTTOM, or
        // pin update in that span is one clock away, not a triangle modulus.
        if top == 0 || tcnt > top || tcnt == top || tcnt == 0 {
            return 1;
        }
        let mut reach = if self.up { top - tcnt } else { tcnt };
        for unit in 0..COMPARES {
            let ocr = self.ocr[unit];
            if ocr == tcnt {
                return 1;
            }
            if self.up && ocr > tcnt && ocr < top {
                reach = reach.min(ocr - tcnt);
            } else if !self.up && ocr < tcnt {
                reach = reach.min(tcnt - ocr);
            }
        }
        u64::from(reach) + 1
    }

    /// Clocks to the next one that leaves an `OCR`, TOP, or `MAX`. The count
    /// reaches the nearest of them, then leaves it on the clock after.
    fn clocks_until_up(&self, top: u16) -> u64 {
        let tcnt = self.tcnt;
        let max = self.max();
        if tcnt == top || tcnt == max {
            return 1;
        }
        // The clock that leaves TOP, or `MAX` above TOP, lands on 0.
        let wrap = if tcnt < top {
            u64::from(top - tcnt) + 1
        } else {
            u64::from(max - tcnt) + 1
        };
        let mut best = wrap;
        for unit in 0..COMPARES {
            let ocr = self.ocr[unit];
            let reach = if ocr == tcnt {
                Some(0)
            } else if ocr > tcnt && (tcnt > top || ocr <= top) {
                Some(u64::from(ocr - tcnt))
            } else if ocr <= top {
                Some(wrap + u64::from(ocr))
            } else {
                None
            };
            if let Some(reach) = reach {
                best = best.min(reach + 1);
            }
        }
        best
    }

    fn project(&self, clocks: u64) -> (u16, bool) {
        if clocks == 0 {
            return (self.tcnt, self.up);
        }
        let wave = self.wave();
        if wave.reserved {
            return (self.tcnt, self.up);
        }
        let top = self.top_value(wave);
        match wave.kind {
            Kind::Phase | Kind::PhaseFreq => {
                project_phase(self.tcnt, self.up, clocks, top, self.max())
            }
            Kind::Normal | Kind::Ctc | Kind::Fast => {
                (project_up(self.tcnt, clocks, top, self.max()), true)
            }
        }
    }

    fn apply_pwm(&mut self, kind: Kind, top: u16, matched: u8, edge: Edge) {
        for unit in 0..COMPARES {
            let hit = matched & (1 << unit) != 0;
            let Some(level) = pwm_oc_level(kind, top, self.ocr[unit], self.com(unit), hit, edge)
            else {
                continue;
            };
            self.set_oc(unit, level);
        }
    }

    fn match_edge(&mut self, unit: usize, wave: Wave) {
        let com = self.com(unit);
        if matches!(wave.kind, Kind::Fast | Kind::Phase | Kind::PhaseFreq) {
            if com == 1 && unit == 0 && wave.toggle_a {
                self.set_oc(0, self.oc & 1 == 0);
            }
            return;
        }
        match com {
            1 => self.set_oc(unit, self.oc & (1u8 << unit) == 0),
            2 => self.set_oc(unit, false),
            3 => self.set_oc(unit, true),
            _ => {}
        }
    }

    fn force(&mut self, value: u8, wave: Wave) {
        if matches!(wave.kind, Kind::Fast | Kind::Phase | Kind::PhaseFreq) {
            return;
        }
        for (unit, fired) in [value & 0x80 != 0, value & 0x40 != 0]
            .into_iter()
            .enumerate()
        {
            if fired {
                self.match_edge(unit, wave);
            }
        }
    }

    fn publish(&mut self, mux: &mut PinMux, ctx: &mut Ctx<'_>, report_at: Option<u64>, at: u64) {
        for unit in 0..COMPARES {
            let pin = self.cfg.oc[unit];
            let source = DriveSource::Compare {
                timer: self.index,
                unit: unit as u8,
            };
            let bit = 1u8 << unit;
            let com = self.com(unit);
            let connect = com != 0 && !self.com_disconnected(unit);
            if connect {
                let level = if self.oc & bit == 0 {
                    Level::Low
                } else {
                    Level::High
                };
                self.linked |= bit;
                mux.set_override(source, pin.port, pin.bit, Some(level), ctx, report_at, at);
            } else if self.linked & bit != 0 {
                self.linked &= !bit;
                mux.set_override(source, pin.port, pin.bit, None, ctx, report_at, at);
            }
        }
    }

    fn com_disconnected(&self, unit: usize) -> bool {
        let com = self.com(unit);
        if com != 1 {
            return false;
        }
        let wave = self.wave();
        if !matches!(wave.kind, Kind::Fast | Kind::Phase | Kind::PhaseFreq) {
            return false;
        }
        unit != 0 || !wave.toggle_a
    }

    fn set_oc(&mut self, unit: usize, high: bool) {
        let bit = 1u8 << unit;
        if high {
            self.oc |= bit;
        } else {
            self.oc &= !bit;
        }
    }

    fn commit_tcnt(&mut self, value: u16) {
        self.tcnt = if self.is16() { value } else { value & 0xFF };
        self.hold_write = true;
        self.block = true;
    }

    fn assemble_ocr(&self, value: u8) -> u16 {
        if self.is16() {
            u16::from(self.temp) << 8 | u16::from(value)
        } else {
            u16::from(value)
        }
    }

    fn write_ocr(&mut self, unit: usize, value: u16) {
        let masked = self.mask_ocr(value);
        if self.wave().update == Update::Immediate {
            self.ocr[unit] = masked;
        }
        self.buf[unit] = masked;
    }

    fn plant_ocr(&mut self, unit: usize, value: u16) {
        let masked = self.mask_ocr(value);
        self.ocr[unit] = masked;
        self.buf[unit] = masked;
    }

    fn mask_ocr(&self, value: u16) -> u16 {
        match self.wave().top {
            TopSrc::Fixed(top) => value & top,
            _ if !self.is16() => value & 0xFF,
            _ => value,
        }
    }

    fn cpu_ocr(&self, unit: usize) -> u16 {
        if self.wave().update == Update::Immediate {
            self.ocr[unit]
        } else {
            self.buf[unit]
        }
    }

    fn top_value(&self, wave: Wave) -> u16 {
        match wave.top {
            TopSrc::Max => self.max(),
            TopSrc::Fixed(top) => top,
            TopSrc::Ocra => self.ocr[0],
            TopSrc::Icr => self.icr,
        }
    }

    fn max(&self) -> u16 {
        if self.is16() { 0xFFFF } else { 0xFF }
    }

    fn wave(&self) -> Wave {
        decode(self.wgm(), self.is16())
    }

    fn wgm(&self) -> u8 {
        self.wgm_with(self.tccra, self.tccrb)
    }

    fn wgm_with(&self, tccra: u8, tccrb: u8) -> u8 {
        if self.is16() {
            (tccra & 0x03) | ((tccrb & 0x18) >> 1)
        } else {
            (tccra & 0x03) | ((tccrb & 0x08) >> 1)
        }
    }

    fn apply_tccrb(&mut self, value: u8, mux: &mut PinMux, ctx: &mut Ctx<'_>, at: u64) {
        let wave = decode(self.wgm_with(self.tccra, value), self.is16());
        self.tccrb = value & self.tccrb_mask();
        self.apply_cs(value);
        if !self.is16() {
            self.force(value, wave);
        }
        self.publish(mux, ctx, Some(at), at);
    }

    fn apply_cs(&mut self, value: u8) {
        let (div, external) = decode_cs(value, self.cfg.clock);
        self.div = div;
        self.external = external;
    }

    fn com(&self, unit: usize) -> u8 {
        if unit == 0 {
            self.tccra >> 6
        } else {
            (self.tccra >> 4) & 0x03
        }
    }

    fn is16(&self) -> bool {
        self.cfg.width == TimerWidth::Sixteen
    }

    fn tccrb_mask(&self) -> u8 {
        if self.is16() { TCCRB_16 } else { TCCRB_8 }
    }

    fn tccra_view(&self) -> u8 {
        self.tccra & TCCRA_MASK
    }

    /// Write-one-to-clear flag bits. The chip table copies this into `TIFRn`.
    pub(crate) const fn flag_mask_for(capture: bool) -> u8 {
        let mut mask = TOV | OCF_A | (OCF_A << 1);
        if capture {
            mask |= ICF;
        }
        mask
    }

    fn flag_mask(&self) -> u8 {
        Self::flag_mask_for(self.cfg.capt_vec != 0)
    }
}

fn push_sorted(queue: &mut [u64; 4], len: &mut u8, at: u64) {
    if usize::from(*len) >= queue.len() {
        return;
    }
    let mut index = usize::from(*len);
    queue[index] = at;
    *len += 1;
    while index > 0 && queue[index] < queue[index - 1] {
        queue.swap(index, index - 1);
        index -= 1;
    }
}

fn pop_sorted(queue: &mut [u64; 4], len: &mut u8) {
    if *len == 0 {
        return;
    }
    for index in 1..usize::from(*len) {
        queue[index - 1] = queue[index];
    }
    *len -= 1;
}

fn predict_step(kind: Kind, up: bool, top: u16, max: u16, prev: u16) -> (u16, bool) {
    match kind {
        Kind::Normal => {
            if prev >= max {
                (0, true)
            } else {
                (prev.wrapping_add(1), true)
            }
        }
        Kind::Ctc | Kind::Fast => {
            if prev == top || prev >= max {
                (0, true)
            } else {
                (prev.wrapping_add(1), true)
            }
        }
        Kind::Phase | Kind::PhaseFreq => phase_step(up, top, max, prev),
    }
}

/// One dual-slope clock. `TOP` turns the count around. The register width
/// wraps to 0 and leaves the direction alone (§15.2.2, Table 15-1 `MAX`).
fn phase_step(up: bool, top: u16, max: u16, prev: u16) -> (u16, bool) {
    if top == 0 {
        return (0, true);
    }
    if up {
        if prev == top {
            return (prev.wrapping_sub(1), false);
        }
        if prev >= max {
            return (0, true);
        }
        return (prev.wrapping_add(1), true);
    }
    if prev == 0 {
        return (1, true);
    }
    (prev.wrapping_sub(1), false)
}

/// What one timer clock was, for the PWM waveform.
#[derive(Debug, Clone, Copy)]
struct Edge {
    /// The clock left TOP: fast PWM clears to BOTTOM, dual slope turns down.
    left_top: bool,
    /// A dual-slope count left 0 and turned up.
    left_bottom: bool,
    /// The direction after the clock.
    up: bool,
}

/// `Some(level)` when this clock writes the `OCnx` register. `COM` 0 and 1 do
/// nothing here; toggle is [`Timer::match_edge`]. `COM` 2 is non-inverting
/// and `COM` 3 is the inverted table row (§15.6.1, §16.8.1, §18.6.1). `hit`
/// is a compare match on this clock, the clock that leaves `OCR`, and `ocr`
/// is the value after any buffer copy on it.
fn pwm_oc_level(kind: Kind, top: u16, ocr: u16, com: u8, hit: bool, edge: Edge) -> Option<bool> {
    if com < 2 {
        return None;
    }
    let level = |high: bool| if com == 2 { high } else { !high };
    match kind {
        Kind::Fast => {
            // Set at BOTTOM. With OCR = TOP the match on that clock is
            // ignored (Table 15-3 note 1), so the pin holds (§15.7.3).
            if edge.left_top {
                Some(level(true))
            } else if hit {
                Some(level(false))
            } else {
                None
            }
        }
        Kind::Phase | Kind::PhaseFreq => {
            // At TOP the pin takes the up-count result for the OCR now in
            // force, low below TOP and high at TOP: Figure 15-7's period 2,
            // and the set at TOP of Table 15-4 note 1 (§15.7.4, §16.9.4,
            // §18.7.4).
            if edge.left_top {
                return (ocr <= top).then(|| level(ocr == top));
            }
            // OCR = BOTTOM holds the pin low, including an OCR the copy at
            // BOTTOM has just set to 0 (§16.9.5).
            if hit || (edge.left_bottom && ocr == 0) {
                return Some(level(!edge.up));
            }
            None
        }
        Kind::Normal | Kind::Ctc => None,
    }
}

fn cycle_of_tick(origin: u64, div: u64, from: u64, n: u64) -> u64 {
    if div == 0 || n == 0 {
        return u64::MAX;
    }
    let rem = from.wrapping_sub(origin) % div;
    let first_delta = if rem == 0 { div } else { div - rem };
    from.saturating_add(first_delta)
        .saturating_add((n - 1).saturating_mul(div))
}

fn project_up(tcnt: u16, clocks: u64, top: u16, max: u16) -> u16 {
    if clocks == 0 {
        return tcnt;
    }
    let period = u64::from(top) + 1;
    if tcnt <= top {
        return ((u64::from(tcnt) + clocks) % period) as u16;
    }
    let to_zero = u64::from(max - tcnt) + 1;
    if clocks < to_zero {
        return tcnt.wrapping_add(clocks as u16);
    }
    ((clocks - to_zero) % period) as u16
}

fn project_phase(tcnt: u16, up: bool, clocks: u64, top: u16, max: u16) -> (u16, bool) {
    if top == 0 {
        return (0, true);
    }
    if clocks == 0 {
        return (tcnt, up);
    }
    // Above TOP the count keeps its direction. Counting up wraps at `max`
    // and re-enters the triangle at 0. Counting down enters `0..=top` first.
    if tcnt > top {
        if up {
            let to_zero = u64::from(max.wrapping_sub(tcnt)) + 1;
            if clocks < to_zero {
                return (tcnt.wrapping_add(clocks as u16), true);
            }
            let rest = clocks - to_zero;
            if rest == 0 {
                return (0, true);
            }
            return project_phase_triangle(0, true, rest, top);
        }
        let span = u64::from(tcnt.wrapping_sub(top));
        if clocks <= span {
            return (tcnt.wrapping_sub(clocks as u16), false);
        }
        return project_phase_triangle(top, false, clocks - span, top);
    }
    project_phase_triangle(tcnt, up, clocks, top)
}

fn project_phase_triangle(tcnt: u16, up: bool, clocks: u64, top: u16) -> (u16, bool) {
    if clocks == 0 {
        return (tcnt, up);
    }
    let period = u64::from(top) * 2;
    let index = if tcnt == 0 && up {
        (clocks - 1) % period
    } else {
        let base = if up {
            u64::from(tcnt) - 1
        } else {
            period - 1 - u64::from(tcnt)
        };
        (base + clocks) % period
    };
    if index < u64::from(top) {
        ((index + 1) as u16, true)
    } else {
        ((period - 1 - index) as u16, false)
    }
}

fn decode_cs(value: u8, clock: TimerClock) -> (u16, bool) {
    let cs = value & 0x07;
    if clock == TimerClock::Async {
        let div = match cs {
            0 => 0,
            1 => 1,
            2 => 8,
            3 => 32,
            4 => 64,
            5 => 128,
            6 => 256,
            _ => 1024,
        };
        return (div, false);
    }
    match cs {
        0 => (0, false),
        1 => (1, false),
        2 => (8, false),
        3 => (64, false),
        4 => (256, false),
        5 => (1024, false),
        _ => (0, true),
    }
}

fn decode(wgm: u8, sixteen: bool) -> Wave {
    if sixteen { decode16(wgm) } else { decode8(wgm) }
}

const fn row(kind: Kind, top: TopSrc, update: Update, toggle_a: bool) -> Wave {
    Wave {
        reserved: false,
        kind,
        top,
        update,
        tov_on_max: matches!(kind, Kind::Normal | Kind::Ctc),
        toggle_a,
    }
}

const RESERVED: Wave = Wave {
    reserved: true,
    kind: Kind::Normal,
    top: TopSrc::Max,
    update: Update::Immediate,
    tov_on_max: true,
    toggle_a: false,
};

fn decode8(wgm: u8) -> Wave {
    match wgm & 0x07 {
        0 => row(Kind::Normal, TopSrc::Max, Update::Immediate, false),
        1 => row(Kind::Phase, TopSrc::Fixed(0xFF), Update::Top, false),
        2 => row(Kind::Ctc, TopSrc::Ocra, Update::Immediate, false),
        3 => row(Kind::Fast, TopSrc::Fixed(0xFF), Update::Bottom, false),
        5 => row(Kind::Phase, TopSrc::Ocra, Update::Top, true),
        7 => row(Kind::Fast, TopSrc::Ocra, Update::Bottom, true),
        _ => RESERVED,
    }
}

fn decode16(wgm: u8) -> Wave {
    match wgm & 0x0F {
        0 => row(Kind::Normal, TopSrc::Max, Update::Immediate, false),
        1 => row(Kind::Phase, TopSrc::Fixed(0x00FF), Update::Top, false),
        2 => row(Kind::Phase, TopSrc::Fixed(0x01FF), Update::Top, false),
        3 => row(Kind::Phase, TopSrc::Fixed(0x03FF), Update::Top, false),
        4 => row(Kind::Ctc, TopSrc::Ocra, Update::Immediate, false),
        5 => row(Kind::Fast, TopSrc::Fixed(0x00FF), Update::Bottom, false),
        6 => row(Kind::Fast, TopSrc::Fixed(0x01FF), Update::Bottom, false),
        7 => row(Kind::Fast, TopSrc::Fixed(0x03FF), Update::Bottom, false),
        8 => row(Kind::PhaseFreq, TopSrc::Icr, Update::Bottom, false),
        9 => row(Kind::PhaseFreq, TopSrc::Ocra, Update::Bottom, true),
        10 => row(Kind::Phase, TopSrc::Icr, Update::Top, false),
        11 => row(Kind::Phase, TopSrc::Ocra, Update::Top, true),
        12 => row(Kind::Ctc, TopSrc::Icr, Update::Immediate, false),
        14 => row(Kind::Fast, TopSrc::Icr, Update::Bottom, true),
        15 => row(Kind::Fast, TopSrc::Ocra, Update::Bottom, true),
        _ => RESERVED,
    }
}

#[cfg(test)]
mod projection_tests {
    use super::{Kind, predict_step, project_phase};

    fn agrees(start: u16, up: bool, top: u16, max: u16, clocks: u64) {
        let mut tcnt = start;
        let mut dir = up;
        for n in 1..=clocks {
            let (next, next_up) = predict_step(Kind::Phase, dir, top, max, tcnt);
            tcnt = next;
            dir = next_up;
            assert_eq!(
                project_phase(start, up, n, top, max),
                (tcnt, dir),
                "after {n} clocks from {start:#x} up={up} top={top} max={max:#x}"
            );
        }
    }

    #[test]
    fn phase_projection_matches_step_above_and_inside_top() {
        agrees(0x29, true, 10, 0xFFFF, 30);
        agrees(0x40, false, 10, 0xFFFF, 100);
        agrees(5, true, 10, 0xFF, 50);
        agrees(10, true, 10, 0xFF, 40);
        agrees(0, true, 80, 0xFF, 20);
        // 8-bit width: 0x21 counts to 0xFF, wraps to 0 on clock 223, sits on
        // TOP at 233, and turns on 234.
        agrees(0x21, true, 10, 0xFF, 250);
        assert_eq!(project_phase(0x21, true, 222, 10, 0xFF), (0xFF, true));
        assert_eq!(project_phase(0x21, true, 223, 10, 0xFF), (0, true));
        assert_eq!(project_phase(0x21, true, 233, 10, 0xFF), (10, true));
        assert_eq!(project_phase(0x21, true, 234, 10, 0xFF), (9, false));
        agrees(0xFFF0, true, 10, 0xFFFF, 40);
        assert_eq!(project_phase(0xFFF0, true, 16, 10, 0xFFFF), (0, true));
        assert_eq!(project_phase(0xFFF0, true, 27, 10, 0xFFFF), (9, false));
    }
}
