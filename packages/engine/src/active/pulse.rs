//! The host's pulse engine. It watches one line, times each level, and calls [`PulseTarget`]
//! when a level ends. It owns the reset, the undo for a record that starts before a latch, and
//! the cut when a run ends inside a record. The device owns the bits and the colours.
//!
//! One [`Hosted::take`](super::native::Hosted::take) or [`Hosted::cut`](super::native::Hosted::cut)
//! enters per landing. The walk below then calls the device on the concrete type.

use std::sync::Arc;

use orbe_model::{Cx, PulseTarget, Time};

use super::native::Port;
use crate::diagnostic::ProtocolRule;
use crate::leds::Chain;
use crate::link::{LinkRow, Record, RecordKind};

/// Where the line is. `Fresh` is before the first level, so a power-on low does not arm a reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Fresh,
    High {
        at: Time,
    },
    /// High at the idle level, from power-on or from a record's return to idle. The hold is no
    /// bit, so a fall ends none, and the next record times its first high from its own start.
    Held,
    Low {
        since: Time,
    },
}

/// The chain before a latch a stop made. A record lands only at its end, so a record that
/// started before that latch shows the low was shorter, and the chain goes back.
struct Spare<D> {
    device: D,
    line: Line,
    reset_at: Option<Time>,
    warned: Option<(&'static str, Time)>,
    pixels: Vec<[u8; 3]>,
    rows: Arc<Vec<Row>>,
}

impl<D: Clone> Clone for Spare<D> {
    fn clone(&self) -> Self {
        Spare {
            device: self.device.clone(),
            line: self.line,
            reset_at: self.reset_at,
            warned: self.warned,
            pixels: self.pixels.clone(),
            rows: Arc::clone(&self.rows),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.device.clone_from(&source.device);
        self.line = source.line;
        self.reset_at = source.reset_at;
        self.warned = source.warned;
        self.pixels.clone_from(&source.pixels);
        self.rows.clone_from(&source.rows);
    }
}

#[derive(Clone)]
struct Row {
    at: Time,
    pixels: Arc<[[u8; 3]]>,
}

/// What a session reads off a pulse target: its colours, and each latch.
pub(crate) struct Seen {
    pub(crate) observed: Chain,
    pub(crate) latches: Vec<(Time, Vec<[u8; 3]>)>,
}

/// The line of a [`PulseTarget`]. `D` is the device. The labels and the idle time are the part's.
pub(crate) struct Pulse<D> {
    kind: &'static str,
    order: &'static str,
    count: u16,
    /// A low held this long, in picoseconds, is idle. For a WS2812 that is one past 280 µs.
    after_ps: u64,
    line: Line,
    /// One past the idle of the low the line is on. Armed at a fall, never at power-on.
    reset_at: Option<Time>,
    warned: Option<(&'static str, Time)>,
    pixels: Vec<[u8; 3]>,
    /// Shared with every copy, so a landing copies a pointer.
    rows: Arc<Vec<Row>>,
    undo: Option<Box<Spare<D>>>,
}

impl<D: Clone> Clone for Pulse<D> {
    fn clone(&self) -> Self {
        Pulse {
            kind: self.kind,
            order: self.order,
            count: self.count,
            after_ps: self.after_ps,
            line: self.line,
            reset_at: self.reset_at,
            warned: self.warned,
            pixels: self.pixels.clone(),
            rows: Arc::clone(&self.rows),
            undo: self.undo.clone(),
        }
    }

    /// Reuses the pixel buffer and the spare, so a landing's commit copies the chain it holds.
    fn clone_from(&mut self, source: &Self) {
        self.kind = source.kind;
        self.order = source.order;
        self.count = source.count;
        self.after_ps = source.after_ps;
        self.line = source.line;
        self.reset_at = source.reset_at;
        self.warned = source.warned;
        self.pixels.clone_from(&source.pixels);
        self.rows.clone_from(&source.rows);
        match (&mut self.undo, &source.undo) {
            (Some(dest), Some(src)) => dest.as_mut().clone_from(src.as_ref()),
            (dest, src) => *dest = src.clone(),
        }
    }
}

impl<D> Pulse<D> {
    /// A WS2812 chain of `count` LEDs. Idle is a low one picosecond past 280 µs.
    pub(crate) fn ws2812(count: u16) -> Pulse<D> {
        Pulse {
            kind: "ws2812",
            order: "grb",
            count,
            after_ps: 280_000_001,
            line: Line::Fresh,
            reset_at: None,
            warned: None,
            pixels: vec![[0, 0, 0]; usize::from(count)],
            rows: Arc::new(Vec::new()),
            undo: None,
        }
    }
}

impl<D: PulseTarget> Pulse<D> {
    fn note(&mut self, cx: &Cx) {
        if self.warned.is_none()
            && let Some(broken) = cx.broken()
        {
            self.warned = Some(broken);
        }
    }

    fn call_pulse(&mut self, device: &mut D, at: Time, level: bool, width: u64) {
        let mut cx = Cx::default();
        device.pulse(&mut cx, at, level, width);
        self.note(&cx);
    }

    /// `undo` keeps the chain from before this latch. A record that lands later, and that
    /// started during the low, restores it. A latch inside a record does not: that record is
    /// the one that made it.
    fn fire(&mut self, device: &mut D, at: Time, undo: bool) {
        if self.reset_at != Some(at) {
            return;
        }
        let spare = undo.then(|| {
            Box::new(Spare {
                device: *device,
                line: self.line,
                reset_at: self.reset_at,
                warned: self.warned,
                pixels: self.pixels.clone(),
                rows: Arc::clone(&self.rows),
            })
        });
        self.reset_at = None;
        if !matches!(self.line, Line::Low { .. }) {
            self.line = Line::Low { since: at };
        }
        let mut cx = Cx::default();
        device.idle(&mut cx, at, false);
        self.note(&cx);
        if cx.viewed() {
            self.keep_view(at, cx.view_bytes());
        }
        if let Some(spare) = spare {
            self.undo = Some(spare);
        }
    }

    fn keep_view(&mut self, at: Time, bytes: &[u8]) {
        let count = usize::from(self.count);
        let (chunks, rest) = bytes.as_chunks::<3>();
        debug_assert!(
            rest.is_empty() && chunks.len() == count,
            "a chain's view is three bytes per led"
        );
        for (led, px) in self.pixels.iter_mut().zip(chunks) {
            *led = *px;
        }
        Arc::make_mut(&mut self.rows).push(Row {
            at,
            pixels: Arc::from(self.pixels.as_slice()),
        });
    }

    fn seen(&self) -> Seen {
        Seen {
            observed: Chain {
                kind: self.kind,
                count: self.count,
                order: self.order,
                pixels: self.pixels.clone(),
            },
            latches: self
                .rows
                .iter()
                .map(|row| (row.at, row.pixels.to_vec()))
                .collect(),
        }
    }

    /// Drops the spare, or restores it when `start` is before the latch it undoes.
    fn restore(&mut self, device: &mut D, start: Time) {
        let Some(before) = self.undo.take() else {
            return;
        };
        if before.reset_at.is_some_and(|latch| start < latch) {
            device.clone_from(&before.device);
            self.line = before.line;
            self.reset_at = before.reset_at;
            self.warned = before.warned;
            self.pixels.clone_from(&before.pixels);
            self.rows.clone_from(&before.rows);
        }
    }

    fn fall(&mut self, at: Time) {
        self.line = Line::Low { since: at };
        self.reset_at = at.as_ps().checked_add(self.after_ps).map(Time::from_ps);
    }

    fn feed(&mut self, device: &mut D, level: bool, at: Time) {
        match self.line {
            Line::Fresh => {
                self.line = if level {
                    Line::Held
                } else {
                    Line::Low { since: at }
                };
            }
            Line::Held if !level => self.fall(at),
            Line::High { at: since } if !level => {
                let high = at.as_ps().saturating_sub(since.as_ps());
                self.call_pulse(device, at, true, high);
                self.fall(at);
            }
            Line::Low { since } if level => {
                debug_assert!(
                    self.reset_at.is_none_or(|reset| at < reset),
                    "the reset lands before the rise"
                );
                let low = at.as_ps().saturating_sub(since.as_ps());
                self.call_pulse(device, at, false, low);
                self.reset_at = None;
                self.line = Line::High { at };
            }
            Line::High { .. } | Line::Held | Line::Low { .. } => {}
        }
    }

    /// The pulses from `start` up to `until`, and a level that starts at `until`, as a landing
    /// there would feed it. A held line starts its high at `start`, so the train's first high is
    /// timed from the record, and a train that opens on a low falls with no bit. A low that
    /// passes the reset latches at the reset, inside the pulse.
    fn walk(
        &mut self,
        device: &mut D,
        start: Time,
        idle: bool,
        pulses: &[(bool, u64)],
        until: Time,
    ) {
        self.restore(device, start);
        if self.line == Line::Fresh {
            self.feed(device, idle, start);
        }
        if self.line == Line::Held {
            match pulses.first() {
                Some((false, _)) => self.fall(start),
                _ => self.line = Line::High { at: start },
            }
        }
        let until_ps = until.as_ps();
        let mut t = start.as_ps();
        for &(level, ps) in pulses {
            if t > until_ps {
                break;
            }
            self.feed(device, level, Time::from_ps(t));
            t = t.saturating_add(ps);
            if let Some(reset) = self.reset_at
                && reset.as_ps() <= t.min(until_ps)
            {
                self.fire(device, reset, false);
            }
        }
    }

    /// The line back at `idle` at a record's end. A high a pulse began keeps its start.
    fn rest(&mut self, device: &mut D, idle: bool, at: Time) {
        let rises = idle && matches!(self.line, Line::Low { .. });
        self.feed(device, idle, at);
        if rises {
            self.line = Line::Held;
        }
    }

    fn land(&mut self, device: &mut D, start: Time, idle: bool, pulses: &[(bool, u64)], end: Time) {
        self.walk(device, start, idle, pulses, end);
        self.rest(device, idle, end);
    }
}

impl<D: PulseTarget> Port<D> for Pulse<D> {
    const PINS: usize = 0;

    fn drive(&self, _image: &mut [orbe_model::Drive]) {}

    fn due_at(&self) -> Option<Time> {
        self.reset_at
    }

    fn take_due(&mut self, device: &mut D, _cx: &mut Cx, at: Time) {
        self.fire(device, at, true);
    }

    fn input(&mut self, device: &mut D, _cx: &mut Cx, at: Time, levels: &[bool]) {
        debug_assert_eq!(levels.len(), 1, "a pulse target watches one line");
        self.undo = None;
        self.feed(device, levels[0], at);
    }

    fn broke(&self, found: &mut Vec<(ProtocolRule, Time)>) {
        let Some((rule, at)) = self.warned else {
            return;
        };
        debug_assert!(
            rule == "bit",
            "a pulse target names a rule the engine knows, not {rule}"
        );
        if rule == "bit" {
            found.push((ProtocolRule::Bit, at));
        }
    }

    fn take_records(&mut self, device: &mut D, at: Time, records: &[LinkRow]) {
        for row in records {
            let RecordKind::Pulses { pulses, .. } = &row.record.kind else {
                continue;
            };
            debug_assert_eq!(row.record.end, at);
            self.land(device, row.record.start, pulses.idle, &pulses.pulses, at);
        }
    }

    fn cut_record(&mut self, device: &mut D, at: Time, record: &Record) {
        let RecordKind::Pulses { pulses, .. } = &record.kind else {
            return;
        };
        debug_assert!(record.start <= at && at < record.end);
        self.walk(device, record.start, pulses.idle, &pulses.pulses, at);
    }

    fn chain(&self) -> Option<Seen> {
        Some(self.seen())
    }
}
