//! The host's pulse engine. It watches one line, times each level, and calls [`PulseTarget`]
//! when a level ends. It owns the idle timer, the undo for a record that starts before an idle,
//! and the cut when a run ends inside a record. On an open-drain line it also plays the trains
//! the device queues: it pulls the line low for each low level and releases it for each high
//! one, each edge at its exact time. With a relay, it sends the edges a device passes from a
//! record on as a record of its own. The device owns what the levels mean, and what it shows.
//!
//! One [`Hosted::take`](super::native::Hosted::take) or [`Hosted::cut`](super::native::Hosted::cut)
//! enters per landing. The walk below then calls the device on the concrete type.

use std::collections::VecDeque;
use std::sync::Arc;

use orbe_model::{Device, Drive, PulseTarget, Pulses, Time};

use super::native::{Hosting, Port};
use crate::link::{Answers, LinkRow, Record, RecordKind};
use crate::netlist::NetId;
use crate::world::World;

/// Where the line is, against the level it rests at. `Fresh` is before the first level, so a
/// line that rests from power-on arms no idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Fresh,
    /// Away from rest since `since`.
    Away {
        since: Time,
    },
    /// Away from rest, from power-on or from a record's return to its own idle level. The hold
    /// is no pulse, so its end calls nothing, and the next record times its first level from
    /// its own start.
    Held,
    /// At rest since `since`.
    Rest {
        since: Time,
    },
}

/// The device and the line before an idle a stop made. A record lands only at its end, so a
/// record that started before that idle shows the rest was shorter, and both go back.
struct Spare<D> {
    dev: Hosting<D>,
    line: Line,
    idle_at: Option<Time>,
}

impl<D: Device> Clone for Spare<D> {
    fn clone(&self) -> Self {
        Spare {
            dev: self.dev.clone(),
            line: self.line,
            idle_at: self.idle_at,
        }
    }
}

/// A train the device queued, as the port plays it.
#[derive(Clone, Debug)]
struct Playing {
    levels: Arc<[(bool, u64)]>,
    /// The level that starts at `next`. Past the last level, `next` is the release.
    k: usize,
    next: Time,
}

/// The edges a device passes from the records it takes, sent on as `Pulses` records on the
/// net of its output. A record that lands passes its edges at once, and the record made of them
/// ends at a stop after that landing, where the parts on that net take it.
#[derive(Clone, Debug)]
pub(crate) struct Relay {
    pin: &'static str,
    net: NetId,
    /// The edges the record being walked has passed so far, in time order.
    edges: Vec<(Time, bool)>,
    /// The records made from landed records that have not ended yet, in order of end.
    due: VecDeque<Record>,
    /// The records that ended at a stop and have not been drained, in order of end.
    sent: Vec<Record>,
    /// How many records it made.
    seq: u64,
}

impl Relay {
    /// The relay of the output `pin` on `net`.
    pub(crate) fn new(pin: &'static str, net: NetId) -> Relay {
        Relay {
            pin,
            net,
            edges: Vec::new(),
            due: VecDeque::new(),
            sent: Vec::new(),
            seq: 0,
        }
    }

    /// The edges passed so far as a record that ends at `end`, or at the last edge when that is
    /// later. The record starts at the first edge, holds each level to the next edge, and rests
    /// at the last level. `None` when no edge was passed.
    fn record(&mut self, end: Time) -> Option<Record> {
        let &(start, _) = self.edges.first()?;
        let &(last, idle) = self.edges.last()?;
        let end = end.max(last);
        let pulses: Vec<(bool, u64)> = self
            .edges
            .iter()
            .zip(self.edges.iter().skip(1).map(|&(at, _)| at).chain([end]))
            .map(|(&(at, level), next)| (level, next.as_ps() - at.as_ps()))
            .filter(|&(_, ps)| ps > 0)
            .collect();
        self.edges.clear();
        let record = Record {
            seq: self.seq,
            start,
            end,
            peripheral: self.pin,
            pin: self.pin,
            kind: RecordKind::Pulses {
                pulses: Pulses {
                    idle,
                    pulses: Arc::from(pulses),
                },
                net: self.net,
            },
        };
        Some(record)
    }
}

/// The line of a [`PulseTarget`] `D`, which declares the level the line rests at and when that
/// rest is idle.
pub(crate) struct Pulse<D> {
    line: Line,
    /// When the rest the line is on becomes idle. Armed when the line comes to rest, never at
    /// power-on.
    idle_at: Option<Time>,
    undo: Option<Box<Spare<D>>>,
    playing: Option<Playing>,
    /// Whether the open drain pulls the line low now.
    low: bool,
    /// Where the edges the device passes go. `None` on a device with no output to pass them to.
    relay: Option<Relay>,
    /// Watch index of the supply comparator, when the device has a minimum supply. Below it the
    /// port ignores edges and records and starts fresh when power returns.
    supply: Option<usize>,
    /// Whether the supply watch reads high. Always true when `supply` is `None`.
    powered: bool,
    /// When the supply last rose above its minimum.
    powered_since: Time,
}

impl<D: Device> Clone for Pulse<D> {
    fn clone(&self) -> Self {
        Pulse {
            line: self.line,
            idle_at: self.idle_at,
            undo: self.undo.clone(),
            playing: self.playing.clone(),
            low: self.low,
            relay: self.relay.clone(),
            supply: self.supply,
            powered: self.powered,
            powered_since: self.powered_since,
        }
    }

    /// Reuses the spare's box, so a landing's commit copies only what the spare holds.
    fn clone_from(&mut self, source: &Self) {
        self.line = source.line;
        self.idle_at = source.idle_at;
        self.undo.clone_from(&source.undo);
        self.playing.clone_from(&source.playing);
        self.low = source.low;
        self.relay.clone_from(&source.relay);
        self.supply = source.supply;
        self.powered = source.powered;
        self.powered_since = source.powered_since;
    }
}

impl<D: PulseTarget> Pulse<D> {
    pub(crate) fn new() -> Pulse<D> {
        Pulse {
            line: Line::Fresh,
            idle_at: None,
            undo: None,
            playing: None,
            low: false,
            relay: None,
            supply: None,
            powered: true,
            powered_since: Time::ZERO,
        }
    }

    /// The line of a device that passes the edges of the records it takes on through `relay`.
    pub(crate) fn relaying(relay: Relay) -> Pulse<D> {
        Pulse {
            relay: Some(relay),
            ..Pulse::new()
        }
    }

    /// Gates edges and records on the supply comparator at watch index `watch`.
    pub(crate) fn with_supply(mut self, watch: usize) -> Pulse<D> {
        self.supply = Some(watch);
        self
    }

    /// Keeps the edge the last call on the device passed.
    fn passed(&mut self, dev: &mut Hosting<D>) {
        if let Some(edge) = dev.take_passed() {
            self.relay
                .as_mut()
                .expect("only a device with a relay passes an edge")
                .edges
                .push(edge);
        }
    }

    /// Makes the record of the edges a landed record passed, which ends at the later of `end`
    /// and the last edge.
    fn relay(&mut self, end: Time) {
        let Some(relay) = &mut self.relay else {
            return;
        };
        if let Some(record) = relay.record(end) {
            debug_assert!(
                relay.due.back().is_none_or(|last| last.end < record.end),
                "a relay's records end in order"
            );
            relay.seq += 1;
            relay.due.push_back(record);
        }
    }

    /// The train's edges due at `at`: each level that starts there, or the release after the
    /// last. A level of no width gives way to the next at once.
    fn play(&mut self, at: Time) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        while playing.next == at {
            let Some(&(level, ps)) = playing.levels.get(playing.k) else {
                self.low = false;
                self.playing = None;
                return;
            };
            self.low = !level;
            playing.k += 1;
            playing.next = Time::from_ps(at.as_ps().saturating_add(ps));
        }
    }

    /// `undo` keeps the device and the line from before this idle. A record that lands later,
    /// and that started during the rest, restores them. An idle inside a record does not: that
    /// record is the one that made it.
    fn fire(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, undo: bool) {
        if self.idle_at != Some(at) {
            return;
        }
        debug_assert!(matches!(self.line, Line::Rest { .. }), "only a rest idles");
        let spare = undo.then(|| {
            Box::new(Spare {
                dev: dev.clone(),
                line: self.line,
                idle_at: self.idle_at,
            })
        });
        self.idle_at = None;
        let level = D::IDLE.level;
        dev.call(world, at, |device, cx| device.idle(cx, at, level));
        if let Some(spare) = spare {
            self.undo = Some(spare);
        }
    }

    /// Drops the spare, or restores it when `start` is before the idle it undoes.
    fn restore(&mut self, dev: &mut Hosting<D>, start: Time) {
        let Some(before) = self.undo.take() else {
            return;
        };
        if before.idle_at.is_some_and(|idle| start < idle) {
            dev.clone_from(&before.dev);
            self.line = before.line;
            self.idle_at = before.idle_at;
        }
    }

    fn settle(&mut self, at: Time) {
        self.line = Line::Rest { since: at };
        self.idle_at = at.as_ps().checked_add(D::IDLE.after).map(Time::from_ps);
    }

    /// The line at `level` from `at`. `replay` is a level a record carried, and its call says so.
    fn feed(&mut self, dev: &mut Hosting<D>, world: &World, level: bool, at: Time, replay: bool) {
        let rest = D::IDLE.level;
        let call = |dev: &mut Hosting<D>, level: bool, width: u64| {
            let pulse =
                |device: &mut D, cx: &mut orbe_model::Cx| device.pulse(cx, at, level, width);
            if replay {
                dev.replay(world, at, pulse);
            } else {
                dev.call(world, at, pulse);
            }
        };
        match self.line {
            Line::Fresh => {
                self.line = if level == rest {
                    Line::Rest { since: at }
                } else {
                    Line::Held
                };
            }
            Line::Held if level == rest => self.settle(at),
            Line::Away { since } if level == rest => {
                let width = at.as_ps().saturating_sub(since.as_ps());
                call(dev, !rest, width);
                self.settle(at);
            }
            Line::Rest { since } if level != rest => {
                debug_assert!(
                    self.idle_at.is_none_or(|idle| at < idle),
                    "the idle lands before the line leaves rest"
                );
                let width = at.as_ps().saturating_sub(since.as_ps());
                call(dev, rest, width);
                self.idle_at = None;
                self.line = Line::Away { since: at };
            }
            Line::Away { .. } | Line::Held | Line::Rest { .. } => {}
        }
        if replay {
            self.passed(dev);
        }
    }

    /// The pulses from `start` up to `until`, and a level that starts at `until`, as a landing
    /// there would feed it. A held line leaves rest at `start`, so the train's first level is
    /// timed from the record, and a train that opens at rest comes to rest with no pulse. A rest
    /// that passes its idle time idles there, inside the pulse.
    fn walk(
        &mut self,
        dev: &mut Hosting<D>,
        world: &World,
        start: Time,
        idle: bool,
        pulses: &[(bool, u64)],
        until: Time,
    ) {
        self.restore(dev, start);
        if self.line == Line::Fresh {
            self.feed(dev, world, idle, start, true);
        }
        if self.line == Line::Held {
            match pulses.first() {
                Some(&(level, _)) if level == D::IDLE.level => self.settle(start),
                _ => self.line = Line::Away { since: start },
            }
        }
        let until_ps = until.as_ps();
        let mut t = start.as_ps();
        for &(level, ps) in pulses {
            if t > until_ps {
                break;
            }
            self.feed(dev, world, level, Time::from_ps(t), true);
            t = t.saturating_add(ps);
            if let Some(idle_at) = self.idle_at
                && idle_at.as_ps() <= t.min(until_ps)
            {
                self.fire(dev, world, idle_at, false);
            }
        }
    }

    /// The line back at `idle`, the record's own idle level, at its end. A level a pulse began
    /// keeps its start.
    fn rest(&mut self, dev: &mut Hosting<D>, world: &World, idle: bool, at: Time) {
        let leaves = idle != D::IDLE.level && matches!(self.line, Line::Rest { .. });
        self.feed(dev, world, idle, at, true);
        if leaves {
            self.line = Line::Held;
        }
    }

    fn land(
        &mut self,
        dev: &mut Hosting<D>,
        world: &World,
        start: Time,
        idle: bool,
        pulses: &[(bool, u64)],
        end: Time,
    ) {
        if start < self.powered_since {
            self.line = Line::Fresh;
            self.idle_at = None;
            self.undo = None;
            return;
        }
        self.walk(dev, world, start, idle, pulses, end);
        self.rest(dev, world, idle, end);
        self.relay(Time::from_ps(end.as_ps() + 1));
    }
}

impl<D: PulseTarget> Port<D> for Pulse<D> {
    const PINS: usize = D::OPEN_DRAIN as usize;

    fn drive(&self, image: &mut [Drive]) {
        if D::OPEN_DRAIN {
            image[0] = if self.low { Drive::Low } else { Drive::Off };
        }
    }

    fn due_at(&self) -> Option<Time> {
        let relay = self
            .relay
            .as_ref()
            .and_then(|relay| relay.due.front())
            .map(|record| record.end);
        [
            self.idle_at,
            self.playing.as_ref().map(|playing| playing.next),
            relay,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn take_due(&mut self, dev: &mut Hosting<D>, world: &World, at: Time) -> bool {
        self.play(at);
        self.fire(dev, world, at, true);
        let Some(relay) = &mut self.relay else {
            return false;
        };
        let mut ended = false;
        while relay.due.front().is_some_and(|record| record.end == at) {
            let record = relay.due.pop_front().expect("a front record");
            relay.sent.push(record);
            ended = true;
        }
        ended
    }

    fn input(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, levels: &[bool]) {
        let watches = if self.supply.is_some() { 2 } else { 1 };
        assert_eq!(levels.len(), watches, "a pulse target watches one line");
        self.undo = None;
        if let Some(watch) = self.supply {
            if !levels[watch] {
                self.powered = false;
                self.line = Line::Fresh;
                self.idle_at = None;
                dev.clear_latched();
                if let Some(relay) = &mut self.relay {
                    relay.edges.clear();
                    relay.due.clear();
                }
                return;
            }
            if !self.powered {
                self.powered = true;
                self.powered_since = at;
                self.line = Line::Fresh;
                self.idle_at = None;
            }
        }
        self.feed(dev, world, levels[0], at, false);
    }

    fn claim(&mut self, dev: &mut Hosting<D>, at: Time) {
        let Some((start, train)) = dev.take_queued() else {
            return;
        };
        assert!(
            D::OPEN_DRAIN,
            "only a pulse target on an open-drain line queues a train"
        );
        assert!(
            self.playing.is_none(),
            "a device queues a train after its last one ends"
        );
        assert!(start > at, "a train starts after the call that queues it");
        self.playing = Some(Playing {
            levels: train.pulses,
            k: 0,
            next: start,
        });
    }

    fn take_records(
        &mut self,
        dev: &mut Hosting<D>,
        world: &World,
        at: Time,
        records: &[LinkRow],
        _answers: &mut Answers,
    ) {
        debug_assert!(
            !D::OPEN_DRAIN,
            "an open-drain line is read from its watch, not from records"
        );
        if !self.powered {
            return;
        }
        for row in records {
            let RecordKind::Pulses { pulses, .. } = &row.record.kind else {
                continue;
            };
            debug_assert_eq!(row.record.end, at);
            self.land(
                dev,
                world,
                row.record.start,
                pulses.idle,
                &pulses.pulses,
                at,
            );
        }
    }

    fn cut_record(
        &mut self,
        dev: &mut Hosting<D>,
        world: &World,
        at: Time,
        record: &Record,
    ) -> Option<Record> {
        if !self.powered {
            return None;
        }
        let RecordKind::Pulses { pulses, .. } = &record.kind else {
            return None;
        };
        debug_assert!(record.start <= at && at < record.end);
        if record.start < self.powered_since {
            self.line = Line::Fresh;
            self.idle_at = None;
            self.undo = None;
            return None;
        }
        self.walk(dev, world, record.start, pulses.idle, &pulses.pulses, at);
        let end = Time::from_ps(at.as_ps() + 1);
        let relay = self.relay.as_mut()?;
        relay.record(end).filter(|passed| passed.start <= at)
    }

    fn records(&self) -> &[Record] {
        self.relay.as_ref().map_or(&[], |relay| &relay.sent)
    }

    fn drain_records(&mut self, through: Time, sent: &mut dyn FnMut(Record)) {
        let Some(relay) = &mut self.relay else {
            return;
        };
        let ended = relay
            .sent
            .iter()
            .take_while(|record| record.end <= through)
            .count();
        for record in relay.sent.drain(..ended) {
            sent(record);
        }
    }

    fn in_flight(&self, at: Time) -> Vec<Record> {
        let Some(relay) = &self.relay else {
            return Vec::new();
        };
        relay
            .due
            .iter()
            .filter(|record| record.start <= at)
            .inspect(|record| debug_assert!(record.end > at, "a record that ended was sent"))
            .cloned()
            .collect()
    }
}
