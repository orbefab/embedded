//! The host's pulse engine. It watches one line, times each level, and calls [`PulseTarget`]
//! when a level ends. It owns the idle timer, the undo for a record that starts before an idle,
//! and the cut when a run ends inside a record. On an open-drain line it also plays the trains
//! the device queues: it pulls the line low for each low level and releases it for each high
//! one, each edge at its exact time. The device owns what the levels mean, and what it shows.
//!
//! One [`Hosted::take`](super::native::Hosted::take) or [`Hosted::cut`](super::native::Hosted::cut)
//! enters per landing. The walk below then calls the device on the concrete type.

use std::sync::Arc;

use orbe_model::{Device, Drive, PulseTarget, Time};

use super::native::{Hosting, Port};
use crate::link::{Answers, LinkRow, Record, RecordKind};
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
}

impl<D: Device> Clone for Pulse<D> {
    fn clone(&self) -> Self {
        Pulse {
            line: self.line,
            idle_at: self.idle_at,
            undo: self.undo.clone(),
            playing: self.playing.clone(),
            low: self.low,
        }
    }

    /// Reuses the spare's box, so a landing's commit copies only what the spare holds.
    fn clone_from(&mut self, source: &Self) {
        self.line = source.line;
        self.idle_at = source.idle_at;
        self.undo.clone_from(&source.undo);
        self.playing.clone_from(&source.playing);
        self.low = source.low;
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
        self.walk(dev, world, start, idle, pulses, end);
        self.rest(dev, world, idle, end);
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
        match (
            self.idle_at,
            self.playing.as_ref().map(|playing| playing.next),
        ) {
            (Some(idle), Some(edge)) => Some(idle.min(edge)),
            (idle, edge) => idle.or(edge),
        }
    }

    fn take_due(&mut self, dev: &mut Hosting<D>, world: &World, at: Time) -> bool {
        self.play(at);
        self.fire(dev, world, at, true);
        false
    }

    fn input(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, levels: &[bool]) {
        assert_eq!(levels.len(), 1, "a pulse target watches one line");
        self.undo = None;
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

    fn cut_record(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, record: &Record) {
        let RecordKind::Pulses { pulses, .. } = &record.kind else {
            return;
        };
        debug_assert!(record.start <= at && at < record.end);
        self.walk(dev, world, record.start, pulses.idle, &pulses.pulses, at);
    }
}
