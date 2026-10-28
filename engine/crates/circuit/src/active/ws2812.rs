//! A `ws2812` chain behind the seam. It watches `DIN` and takes `Pulses` on that net. Each LED
//! keeps the first 24 bits and the rest go on down the chain. A low longer than the reset
//! latches. The drive never changes: the chain has no output pin.
//!
//! The times are the Worldsemi WS2812E-1313 datasheet's, V1.1, page 3, "Data Transfer Time":
//! T0H 220 ns to 380 ns, T1H 580 ns to 1 µs, and RES a low of more than 280 µs. A bit is a 0 or
//! a 1 by its high alone, and any low up to the reset ends it. The same page gives T0L and T1L
//! as 580 ns to 1 µs and a data period of at least 1.25 µs. The WS2812E 5050 datasheet, V1.0,
//! page 3, gives T1L as 220 ns to 420 ns. The chain checks neither low, so a train timed to
//! either sheet decodes. A high outside both windows is rule `bit`, dropped, and warned once.

use std::sync::Arc;

use sim_core::diagnostic::ProtocolRule;
use sim_core::leds::Chain as Observed;
use sim_core::link::{LinkRow, Record, RecordKind};
use sim_core::netlist::{PartId, Ws2812};
use sim_core::time::Time;

use super::{Chip, Drive, Stop, Takes};
use crate::mna::Node;
use crate::watch::{Signal, Watch};

const VCC: usize = 0;
const GND: usize = 1;
const DIN: usize = 2;

/// T0H, inclusive, picoseconds.
const T0H: std::ops::RangeInclusive<u64> = 220_000..=380_000;
/// T1H, inclusive.
const T1H: std::ops::RangeInclusive<u64> = 580_000..=1_000_000;
/// RES is a low longer than this. The table gives ">280µs", so exactly this long is no reset.
const RES: u64 = 280_000_000;

/// The pin pairs it joins at DC. It joins none: `DIN` draws no current.
pub(crate) const DC_PATHS: [(usize, usize); 0] = [];

pub(crate) fn chip(part: PartId, chain: &Ws2812) -> Chip {
    let pins = chain.pins;
    let against = |fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[DIN]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let threshold = &chain.input_threshold;
    Chip {
        part: Some(part),
        outputs: Vec::new(),
        watches: vec![Watch {
            rise: against(threshold.high.si.get()),
            fall: against(threshold.low.si.get()),
        }],
        signals: vec![(Ws2812::PINS[DIN], 0)],
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        takes: Takes {
            spi: None,
            duty: None,
            pulses: Some(pins[DIN]),
        },
        core: super::Core::Ws2812(Box::new(Core::new(chain.leds))),
    }
}

/// Where `DIN` is. `Fresh` is before the first level, so a power-on low does not arm a reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Fresh,
    High {
        at: Time,
    },
    /// High since `at` at the idle level, from power-on or from a record's return to idle.
    /// The hold is no bit, so the next record times its first high from its own start.
    Held {
        at: Time,
    },
    /// `open` is the bit of the high before, when that high was in a window. The rise or the
    /// reset that ends the low shifts it.
    Low {
        open: Option<bool>,
    },
}

/// The colours after each latch, and the bits of the frame still open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Core {
    count: u16,
    leds: Vec<[u8; 3]>,
    /// `Some` once that LED has its 24 bits. A latch copies those and leaves the rest.
    pending: Vec<Option<[u8; 3]>>,
    /// The LED the next bit belongs to. `count` once the chain is full.
    index: u16,
    acc: u32,
    got: u8,
    line: Line,
    /// One past the reset of the low `DIN` is on. Armed at each fall.
    reset_at: Option<Time>,
    /// The chain before the latch its last stop made. A record lands only at its end, so a
    /// record that started before that latch shows the low was shorter, and the chain goes back.
    undo: Option<Box<Core>>,
    warned: Option<Time>,
    /// Shared with every copy of the core, so a landing copies a pointer.
    rows: Arc<Vec<Row>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) at: Time,
    pub(crate) pixels: Arc<[[u8; 3]]>,
}

impl Core {
    fn new(count: u16) -> Core {
        debug_assert!((1..=Ws2812::LED_MAX).contains(&count));
        Core {
            count,
            leds: vec![[0, 0, 0]; usize::from(count)],
            pending: vec![None; usize::from(count)],
            index: 0,
            acc: 0,
            got: 0,
            line: Line::Fresh,
            reset_at: None,
            undo: None,
            warned: None,
            rows: Arc::new(Vec::new()),
        }
    }

    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let at = self.reset_at?;
        if at > limit {
            return None;
        }
        let before = Box::new(self.clone());
        self.reset();
        self.undo = Some(before);
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        &[]
    }

    /// `DIN` from `at` on. A power-on low arms nothing: the reset starts at a fall.
    pub(crate) fn input(&mut self, at: Time, levels: &[bool]) {
        assert_eq!(levels.len(), 1, "a chain watches DIN");
        self.undo = None;
        self.feed(levels[0], at);
    }

    pub(crate) fn take(&mut self, at: Time, records: &[LinkRow]) {
        for row in records {
            let RecordKind::Pulses(pulses) = &row.record.kind else {
                continue;
            };
            debug_assert_eq!(row.record.end, at);
            self.land(row.record.start, pulses.idle, &pulses.pulses, at);
        }
    }

    /// The part of `record` sent by `at`, where the run ends inside it. A record lands only at
    /// its end, so without this the run would keep a latch the record undoes.
    pub(crate) fn cut(&mut self, at: Time, record: &Record) {
        let RecordKind::Pulses(pulses) = &record.kind else {
            return;
        };
        debug_assert!(record.start <= at && at < record.end);
        self.walk(record.start, pulses.idle, &pulses.pulses, at);
    }

    pub(crate) fn observed(&self) -> Observed {
        Observed {
            kind: "ws2812",
            count: self.count,
            order: "grb",
            pixels: self.leds.clone(),
        }
    }

    pub(crate) fn latches(&self) -> &[Row] {
        &self.rows
    }

    /// The first malformed bit, if one was dropped.
    pub(crate) fn broke(&self) -> Option<(ProtocolRule, Time)> {
        self.warned.map(|at| (ProtocolRule::Bit, at))
    }

    /// A record of `pulses` from `start` to `end`, then `idle`.
    fn land(&mut self, start: Time, idle: bool, pulses: &[(bool, u64)], end: Time) {
        self.walk(start, idle, pulses, end);
        self.rest(idle, end);
    }

    /// The pulses from `start` up to `until`, and a level that starts at `until`, as a landing
    /// there would feed it. A held line starts its high at `start`, so the train's first high is
    /// timed from the record, and a train that opens on a low falls with no bit. A low that
    /// passes the reset latches at the reset, inside the pulse.
    fn walk(&mut self, start: Time, idle: bool, pulses: &[(bool, u64)], until: Time) {
        if let Some(before) = self.undo.take()
            && before.reset_at.is_some_and(|latch| start < latch)
        {
            *self = *before;
        }
        match self.line {
            Line::Fresh => self.feed(idle, start),
            Line::Held { .. } => match pulses.first() {
                Some((false, _)) => self.fall(None, start),
                _ => self.line = Line::High { at: start },
            },
            Line::High { .. } | Line::Low { .. } => {}
        }
        let until = until.as_ps();
        let mut t = start.as_ps();
        for &(level, ps) in pulses {
            if t > until {
                break;
            }
            self.feed(level, Time::from_ps(t));
            t = t.saturating_add(ps);
            if self
                .reset_at
                .is_some_and(|reset| reset.as_ps() <= t.min(until))
            {
                self.reset();
            }
        }
    }

    /// The line back at `idle` at a record's end. A high a pulse began keeps its start.
    fn rest(&mut self, idle: bool, at: Time) {
        let rises = idle && matches!(self.line, Line::Low { .. });
        self.feed(idle, at);
        if rises {
            self.line = Line::Held { at };
        }
    }

    fn feed(&mut self, level: bool, at: Time) {
        match self.line {
            Line::Fresh => {
                self.line = if level {
                    Line::Held { at }
                } else {
                    Line::Low { open: None }
                };
            }
            Line::High { at: since } | Line::Held { at: since } if !level => {
                let high = at.as_ps().saturating_sub(since.as_ps());
                let open = if T0H.contains(&high) {
                    Some(false)
                } else if T1H.contains(&high) {
                    Some(true)
                } else {
                    self.warn(at);
                    None
                };
                self.fall(open, at);
            }
            Line::Low { open } if level => {
                debug_assert!(
                    self.reset_at.is_none_or(|reset| at < reset),
                    "the reset lands before the rise"
                );
                self.reset_at = None;
                if let Some(one) = open {
                    self.shift(one);
                }
                self.line = Line::High { at };
            }
            Line::High { .. } | Line::Held { .. } | Line::Low { .. } => {}
        }
    }

    fn fall(&mut self, open: Option<bool>, at: Time) {
        self.line = Line::Low { open };
        self.reset_at = at.as_ps().checked_add(RES + 1).map(Time::from_ps);
    }

    /// The low has passed the reset. Its open bit shifts, and the chain latches at the reset.
    fn reset(&mut self) {
        let Some(at) = self.reset_at.take() else {
            return;
        };
        if let Line::Low { open: Some(one) } = self.line {
            self.shift(one);
        }
        self.line = Line::Low { open: None };
        self.latch(at);
    }

    fn shift(&mut self, one: bool) {
        if self.index >= self.count {
            return;
        }
        self.acc = (self.acc << 1) | u32::from(one);
        self.got += 1;
        if self.got == 24 {
            let g = (self.acc >> 16) as u8;
            let r = (self.acc >> 8) as u8;
            let b = self.acc as u8;
            self.pending[usize::from(self.index)] = Some([g, r, b]);
            self.index += 1;
            self.acc = 0;
            self.got = 0;
        }
    }

    fn latch(&mut self, at: Time) {
        for (led, next) in self.leds.iter_mut().zip(self.pending.iter()) {
            if let Some(px) = next {
                *led = *px;
            }
        }
        Arc::make_mut(&mut self.rows).push(Row {
            at,
            pixels: Arc::from(self.leds.as_slice()),
        });
        self.pending.fill(None);
        self.index = 0;
        self.acc = 0;
        self.got = 0;
    }

    fn warn(&mut self, at: Time) {
        if self.warned.is_none() {
            self.warned = Some(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sketch's bits, high then low.
    const ZERO: (u64, u64) = (312_500, 937_500);
    const ONE: (u64, u64) = (625_000, 625_000);
    const RESET_LOW: u64 = 300_000_000;
    const START: u64 = 1_000_000_000;

    fn bits(bytes: &[u8]) -> Vec<bool> {
        bytes
            .iter()
            .flat_map(|byte| (0..8).rev().map(move |k| byte & (1 << k) != 0))
            .collect()
    }

    /// Each bit as a high and a low, the last low lengthened by the reset.
    fn train(bytes: &[u8], zero: (u64, u64), one: (u64, u64)) -> Vec<(bool, u64)> {
        let bits = bits(bytes);
        let mut out = Vec::with_capacity(bits.len() * 2);
        for (i, bit) in bits.iter().enumerate() {
            let (high, mut low) = if *bit { one } else { zero };
            if i + 1 == bits.len() {
                low += RESET_LOW;
            }
            out.push((true, high));
            out.push((false, low));
        }
        out
    }

    fn end(start: u64, pulses: &[(bool, u64)]) -> u64 {
        start + pulses.iter().map(|&(_, ps)| ps).sum::<u64>()
    }

    /// One past the reset of the train's last low.
    fn reset_after(start: u64, pulses: &[(bool, u64)]) -> u64 {
        end(start, pulses) - pulses.last().unwrap().1 + RES + 1
    }

    /// Every stop up to `t` lands, as the seam lands them before anything else at `t`.
    fn settle(core: &mut Core, t: u64) {
        while core.run_until(Time::from_ps(t)).is_some() {}
    }

    /// A power-on low, then `pulses` on the pin from `START`, then low.
    fn by_pin(count: u16, pulses: &[(bool, u64)]) -> Core {
        let mut core = Core::new(count);
        core.input(Time::ZERO, &[false]);
        let mut t = START;
        for &(level, ps) in pulses {
            settle(&mut core, t);
            core.input(Time::from_ps(t), &[level]);
            t += ps;
        }
        settle(&mut core, t);
        core.input(Time::from_ps(t), &[false]);
        settle(&mut core, t + 2 * RES);
        core
    }

    /// The pin at `idle` from power-on, then each record landing at its end. As at the seam,
    /// a landing leaves no stop at or before its time.
    fn by_records(count: u16, idle: bool, records: &[(u64, Vec<(bool, u64)>)]) -> Core {
        let mut core = Core::new(count);
        core.input(Time::ZERO, &[idle]);
        let mut last = 0;
        for (start, pulses) in records {
            last = end(*start, pulses);
            settle(&mut core, last);
            core.land(Time::from_ps(*start), idle, pulses, Time::from_ps(last));
            assert_eq!(core.clone().run_until(Time::from_ps(last)), None);
        }
        settle(&mut core, last + 2 * RES);
        core
    }

    fn latch_times(core: &Core) -> Vec<u64> {
        core.latches().iter().map(|row| row.at.as_ps()).collect()
    }

    #[test]
    fn a_bit_is_told_by_its_high_alone() {
        let bytes = [0xA5, 0x5A, 0xC3];
        let timings = [
            ("the sketch", ZERO, ONE),
            ("an RMT at 10 MHz", (300_000, 900_000), (900_000, 300_000)),
            (
                "T0H max over T0L min",
                (380_000, 580_000),
                (850_000, 400_000),
            ),
            (
                "T0H min over T0L max",
                (220_000, 1_000_000),
                (580_000, 220_000),
            ),
            ("T1H max", (220_000, 50_000), (1_000_000, 3_000_000)),
        ];
        for (name, zero, one) in timings {
            let pulses = train(&bytes, zero, one);
            let pin = by_pin(1, &pulses);
            let record = by_records(1, false, &[(START, pulses.clone())]);
            for (path, core) in [("pin", &pin), ("record", &record)] {
                assert_eq!(
                    core.observed().pixels,
                    vec![[0xA5, 0x5A, 0xC3]],
                    "{name} {path}"
                );
                assert_eq!(core.broke(), None, "{name} {path}");
                assert_eq!(
                    latch_times(core),
                    vec![reset_after(START, &pulses)],
                    "{name} {path}"
                );
            }
        }
    }

    #[test]
    fn a_low_of_up_to_280_us_between_bits_ends_the_bit_and_does_not_latch() {
        let mut pulses = train(&[0xFF, 0x00, 0x81], ZERO, ONE);
        pulses[1].1 = 1;
        pulses[3].1 = RES;
        pulses[5].1 = 100_000_000;
        for core in [
            by_pin(1, &pulses),
            by_records(1, false, &[(START, pulses.clone())]),
        ] {
            assert_eq!(core.observed().pixels, vec![[0xFF, 0x00, 0x81]]);
            assert_eq!(core.latches().len(), 1);
            assert_eq!(core.broke(), None);
        }
    }

    #[test]
    fn a_high_outside_both_windows_warns_once_and_drops_the_bit() {
        for high in [219_999, 380_001, 579_999, 1_000_001] {
            let mut pulses = vec![(true, high), (false, ZERO.1)];
            pulses.extend(train(&[0x0F, 0xF0, 0x3C], ZERO, ONE));
            pulses.extend([(true, high), (false, ZERO.1)]);
            for core in [
                by_pin(1, &pulses),
                by_records(1, false, &[(START, pulses.clone())]),
            ] {
                assert_eq!(core.observed().pixels, vec![[0x0F, 0xF0, 0x3C]], "{high}");
                assert_eq!(
                    core.broke(),
                    Some((ProtocolRule::Bit, Time::from_ps(START + high)))
                );
            }
        }
    }

    #[test]
    fn a_low_of_280_us_does_not_latch_and_one_past_it_does() {
        let mut core = Core::new(1);
        core.input(Time::ZERO, &[false]);
        let mut pulses = train(&[0x80, 0x00, 0x00], ZERO, ONE);
        pulses.pop();
        let fall = end(START, &pulses);
        core.land(Time::from_ps(START), false, &pulses, Time::from_ps(fall));
        assert_eq!(core.run_until(Time::from_ps(fall + RES)), None);
        assert_eq!(core.observed().pixels, vec![[0, 0, 0]]);
        assert!(core.latches().is_empty());
        let stop = core.run_until(Time::from_ps(fall + RES + 1));
        assert_eq!(
            stop.map(|stop| stop.at),
            Some(Time::from_ps(fall + RES + 1))
        );
        assert_eq!(core.observed().pixels, vec![[0x80, 0, 0]]);
        assert_eq!(latch_times(&core), vec![fall + RES + 1]);
    }

    #[test]
    fn a_frame_split_across_two_records_latches_as_one() {
        let bytes: Vec<u8> = (0..13u8).flat_map(|i| [i, i + 16, i + 32]).collect();
        let pulses = train(&bytes, ZERO, ONE);
        let whole = by_records(13, false, &[(START, pulses.clone())]);
        assert_eq!(whole.latches().len(), 1);
        for (idle, at) in [false, true].into_iter().flat_map(|idle| {
            [18 * 8 * 2, 18 * 8 * 2 + 1]
                .into_iter()
                .map(move |at| (idle, at))
        }) {
            let (first, second) = pulses.split_at(at);
            // On an idle-high line a long gap after a high is a long high.
            let gaps: &[u64] = if idle { &[0, 1] } else { &[0, 1, 100_000_000] };
            for &gap in gaps {
                let split = by_records(
                    13,
                    idle,
                    &[
                        (START, first.to_vec()),
                        (end(START, first) + gap, second.to_vec()),
                    ],
                );
                let case = format!("idle {idle}, split at pulse {at}, gap {gap}");
                assert_eq!(split.observed(), whole.observed(), "{case}");
                assert_eq!(
                    latch_times(&split),
                    vec![latch_times(&whole)[0] + gap],
                    "{case}"
                );
                assert_eq!(split.broke(), None, "{case}");
            }
        }
    }

    #[test]
    fn a_record_that_starts_at_the_reset_keeps_the_latch_and_one_before_it_does_not() {
        let mut pulses = train(&[1, 2, 3], ZERO, ONE);
        pulses.last_mut().unwrap().1 = ZERO.1;
        let fall = end(START, &pulses) - ZERO.1;
        let latch = fall + RES + 1;
        let next = train(&[4, 5, 6], ZERO, ONE);
        let two = by_records(1, false, &[(START, pulses.clone()), (latch, next.clone())]);
        assert_eq!(latch_times(&two), vec![latch, reset_after(latch, &next)]);
        assert_eq!(&*two.latches()[0].pixels, &[[1, 2, 3]]);
        assert_eq!(two.observed().pixels, vec![[4, 5, 6]]);
        let one = by_records(1, false, &[(START, pulses), (latch - 1, next)]);
        assert_eq!(one.latches().len(), 1);
        assert_eq!(one.observed().pixels, vec![[1, 2, 3]]);
    }

    #[test]
    fn a_high_a_record_ends_on_keeps_its_start_on_either_idle_level() {
        let pulses = train(&[0x80, 0, 0], ZERO, ONE);
        let (first, second) = pulses.split_at(1);
        for idle in [false, true] {
            for gap in [0, 1] {
                let split = by_records(
                    1,
                    idle,
                    &[
                        (START, first.to_vec()),
                        (end(START, first) + gap, second.to_vec()),
                    ],
                );
                let case = format!("idle {idle}, gap {gap}");
                assert_eq!(split.observed().pixels, vec![[0x80, 0, 0]], "{case}");
                assert_eq!(split.broke(), None, "{case}");
            }
        }
    }

    #[test]
    fn a_low_of_one_past_the_reset_latches_inside_the_record() {
        let mut pulses = train(&[1, 2, 3], ZERO, ONE);
        pulses.last_mut().unwrap().1 = RES + 1;
        let latch = end(START, &pulses);
        let ends = by_records(1, false, &[(START, pulses.clone())]);
        assert_eq!(latch_times(&ends), vec![latch]);
        assert_eq!(ends.observed().pixels, vec![[1, 2, 3]]);

        let next = train(&[4, 5, 6], ZERO, ONE);
        pulses.extend(next.iter().copied());
        let goes_on = by_records(1, false, &[(START, pulses)]);
        assert_eq!(
            latch_times(&goes_on),
            vec![latch, reset_after(latch, &next)]
        );
        assert_eq!(&*goes_on.latches()[0].pixels, &[[1, 2, 3]]);
        assert_eq!(goes_on.observed().pixels, vec![[4, 5, 6]]);
    }

    #[test]
    fn an_idle_high_line_times_the_first_high_from_the_record() {
        let pulses = train(&[0xC0, 0xFF, 0x01], ZERO, ONE);
        let next = train(&[0x01, 0x02, 0x03], ZERO, ONE);
        let records = [(START, pulses.clone()), (end(START, &pulses) + RES, next)];
        let core = by_records(1, true, &records);
        assert_eq!(core.observed().pixels, vec![[0x01, 0x02, 0x03]]);
        assert_eq!(&*core.latches()[0].pixels, &[[0xC0, 0xFF, 0x01]]);
        assert_eq!(core.broke(), None);
        assert_eq!(
            latch_times(&core),
            latch_times(&by_records(1, false, &records))
        );
    }

    #[test]
    fn a_held_line_falls_with_no_bit_when_a_record_opens_low() {
        let mut opens_low = vec![(false, 500_000)];
        opens_low.extend(train(&[0x80, 0, 0], ZERO, ONE));
        let one = by_records(1, true, &[(START, opens_low.clone())]);
        assert_eq!(one.observed().pixels, vec![[0x80, 0, 0]]);
        assert_eq!(one.broke(), None);
        assert_eq!(latch_times(&one), vec![reset_after(START, &opens_low)]);

        let first = train(&[0x11, 0x22, 0x33], ZERO, ONE);
        opens_low[0].1 = 400_000;
        let next = end(START, &first) + 1_000_000_000;
        let two = by_records(1, true, &[(START, first), (next, opens_low)]);
        assert_eq!(&*two.latches()[0].pixels, &[[0x11, 0x22, 0x33]]);
        assert_eq!(two.observed().pixels, vec![[0x80, 0, 0]]);
        assert_eq!(two.broke(), None);
    }

    #[test]
    fn the_line_returns_to_idle_after_a_record() {
        let mut ends_high = train(&[0x12, 0x34, 0x56], ZERO, ONE);
        ends_high.pop();
        let low = by_records(1, false, &[(START, ends_high.clone())]);
        assert_eq!(low.observed().pixels, vec![[0x12, 0x34, 0x56]]);
        assert_eq!(latch_times(&low), vec![end(START, &ends_high) + RES + 1]);

        let mut ends_low = train(&[0x12, 0x34, 0x56], ZERO, ONE);
        ends_low.last_mut().unwrap().1 = ZERO.1;
        let high = by_records(1, true, &[(START, ends_low)]);
        assert!(high.latches().is_empty());
        assert_eq!(high.observed().pixels, vec![[0, 0, 0]]);
        assert_eq!(high.broke(), None);
    }

    #[test]
    fn the_first_byte_is_green() {
        let core = by_pin(1, &train(&[0xA5, 0x00, 0x00], ZERO, ONE));
        assert_eq!(core.observed().pixels, vec![[0xA5, 0, 0]]);
    }

    #[test]
    fn two_leds_take_24_bits_each() {
        let core = by_pin(2, &train(&[1, 2, 3, 4, 5, 6], ZERO, ONE));
        assert_eq!(core.observed().pixels, vec![[1, 2, 3], [4, 5, 6]]);
    }

    #[test]
    fn bits_after_the_chain_are_discarded() {
        let core = by_pin(1, &train(&[1, 2, 3, 0xFF, 0xFF, 0xFF], ZERO, ONE));
        assert_eq!(core.observed().pixels, vec![[1, 2, 3]]);
    }

    #[test]
    fn a_partial_led_keeps_its_colour() {
        let mut core = by_pin(2, &train(&[1, 2, 3, 4, 5, 6], ZERO, ONE));
        let pulses = train(&[7, 8, 9, 10], ZERO, ONE);
        let start = START + 4 * RES;
        core.land(
            Time::from_ps(start),
            false,
            &pulses,
            Time::from_ps(end(start, &pulses)),
        );
        settle(&mut core, end(start, &pulses) + 2 * RES);
        assert_eq!(core.observed().pixels, vec![[7, 8, 9], [4, 5, 6]]);
    }
}
