//! `step`: time advanced in slices, with each slice's changes and a caller's hook between them.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use orbe_model::Time;

use orbe_engine::diagnostic::Diagnostic as EngineDiagnostic;
use orbe_engine::{EdgeCursor, Status, SupplyMark};

use super::bound::Bound;
use super::light;
use super::serial::Arrived;
use super::{Key, Open, Outcome, fail, failed, key, level, t};
use crate::command::Step;
use crate::duration::Duration;
use crate::reply::{Change, Check, Code, Data, Error, Got, LedRead, Level, Met, Stepped};
use crate::target::Ref;

/// What `play` held, then what the step returned. A step that did not run has no list of
/// its own, so the held stretch is the whole reply. `from_ps` stays the start of that stretch.
fn held_then(mut held: Stepped, data: Data) -> Data {
    let Data::Stepped(mut own) = data else {
        return Data::Stepped(held);
    };
    held.more += own.more;
    held.changes.append(&mut own.changes);
    held.leds = own.leds;
    held.met = own.met;
    held.check = own.check;
    Data::Stepped(held)
}

/// `step` advances in slices this long, so `--until` and a caller see each one.
/// LED samples and state holds run on [`light::SAMPLE_GRID`], independent of slice boundaries.
const SLICE: Duration = Duration::from_ms(1);
/// The most changes one `step` lists.
pub const CHANGES: usize = 20;

/// Where a running step stands after a slice, for a caller that shows frames or stops it.
pub struct Progress<'a> {
    pub(super) open: &'a Open,
    pub from_ps: u64,
    pub end_ps: u64,
}

impl Progress<'_> {
    pub fn t_ps(&self) -> u64 {
        self.open.engine.now().as_ps()
    }

    /// Reads a ref as `get` does.
    pub fn get(&self, r: &Ref) -> Result<Got, Error> {
        self.open.get(r)
    }
}

#[derive(Default)]
struct Slice {
    changes: Vec<Change>,
    arrived: Arrived,
}

impl Open {
    /// The coil voltage at `now`. The pitch comes from the pin edges, so this only has to
    /// show whether the coil is actually moving.
    fn note_tone(&mut self) {
        if self.tone.is_empty() {
            return;
        }
        let probe = self.engine.probe();
        self.tone.note(&probe, self.engine.now().as_ps());
    }

    /// Advances to `to`. A part that reads `hz` is probed often enough to see a narrow pulse,
    /// not once per solved landing: a stiff edge is a few hundred picosecond steps, and the
    /// pitch comes from the pin edges.
    fn advance_for(&mut self, to: u64) -> Status {
        if self.tone.is_empty() {
            return self.engine.advance_to(Time::from_ps(to));
        }
        // Shorter than the high time of `analogWrite` duty 10 at about 490 Hz, so that
        // pulse still moves the coil watermark.
        const PROBE_PS: u64 = 20_000_000;
        loop {
            let now = self.engine.now().as_ps();
            if now >= to || self.engine.status() != Status::Running {
                break;
            }
            let dest = now.saturating_add(PROBE_PS).min(to);
            self.engine.advance_to(Time::from_ps(dest));
            self.note_tone();
            if dest >= to || self.engine.status() != Status::Running {
                break;
            }
        }
        self.engine.status()
    }

    fn advance(
        &mut self,
        to: u64,
        end: u64,
        from: u64,
        pin: Option<(usize, Level)>,
    ) -> (Status, Option<u64>) {
        let Some((signal, want)) = pin else {
            return (self.advance_for(to), None);
        };
        let mut seen = 0;
        loop {
            let edge = self.engine.edges_since(self.edges).and_then(|(edges, _)| {
                let found = edges[seen..]
                    .iter()
                    .find(|e| e.signal == signal && level(e.level) == want && e.at.as_ps() >= from);
                seen = edges.len();
                found.map(|e| e.at.as_ps())
            });
            if let Some(at) = edge {
                let after = at + 1;
                if self.engine.now().as_ps() < after && after <= end {
                    self.engine.advance_to(Time::from_ps(after));
                    self.note_tone();
                }
                return (self.engine.status(), Some(at));
            }
            let now = self.engine.now().as_ps();
            if now >= to || self.engine.status() != Status::Running {
                return (self.engine.status(), None);
            }
            let landing = self.engine.next_at().map_or(to, |at| at.as_ps().min(to));
            self.engine.advance_to(Time::from_ps(landing));
            self.note_tone();
        }
    }

    /// Advances one slice up to `to`, stopping at any fixed light-grid tick strictly before `to`
    /// to record LED samples and emit any completed light-state hold. Returns the status, any
    /// pin edge that stopped the slice early, and whether `engine.now()` landed on a light-grid
    /// tick still to be sampled with the slice's probe.
    fn advance_slice(
        &mut self,
        to: u64,
        end: u64,
        from: u64,
        pin: Option<(usize, Level)>,
        found: &mut Vec<Change>,
    ) -> (Status, Option<u64>, bool) {
        if self.light.lamps().is_empty() {
            let (status, edge) = self.advance(to, end, from, pin);
            return (status, edge, false);
        }
        loop {
            let now = self.engine.now().as_ps();
            let grid = light::Log::next_sample_after(now);
            let target = grid.map_or(to, |g| g.min(to));
            let (status, edge) = self.advance(target, end, from, pin);
            let reached = self.engine.now().as_ps();
            let on_grid = grid == Some(reached);
            if edge.is_some() || status != Status::Running || reached >= to {
                return (status, edge, on_grid);
            }
            if on_grid {
                let probe = self.engine.probe();
                found.extend(self.light.sample(&probe));
            }
        }
    }

    /// When a condition read from the circuit or the serial first held in the slice that just
    /// ran: by the slice's end. A pin's edge is found by `advance`.
    fn met_in(&self, bound: &Bound, slice: &Slice) -> Option<Met> {
        let by_ps = self.engine.now().as_ps();
        match bound {
            Bound::Level { .. } => None,
            Bound::SerialContains(text) => self
                .serial
                .met(&slice.arrived, text)
                .map(|sent_at_ps| Met::Serial { by_ps, sent_at_ps }),
            _ => self.observe(bound).0.then_some(Met::By(by_ps)),
        }
    }

    pub(super) fn stop_name(&self) -> String {
        if self.stop_ps == orbe_scene::STOP_PS {
            "2-hour".into()
        } else {
            t(self.stop_ps).to_string()
        }
    }

    /// A CLI `step`. What `play` held is listed first, under its own cap of [`CHANGES`], then
    /// this step's own changes under the same cap, and the hold is cleared. `from_ps` stays
    /// the start of the stretch that was held.
    pub(super) fn step_and_list(
        &mut self,
        step: &Step,
        during: &mut dyn FnMut(&Progress) -> ControlFlow<()>,
    ) -> Outcome {
        match self.step(step, during) {
            Ok(data) => Ok(self.after_held(data)),
            Err(mut failed) => {
                failed.data = self.after_held(failed.data);
                Err(failed)
            }
        }
    }

    /// What `play` held, then `data`, with the supply episodes the stretch opened or closed.
    fn after_held(&mut self, data: Data) -> Data {
        let mut data = match self.unlisted.take() {
            Some(held) => held_then(held, data),
            None => data,
        };
        if let Data::Stepped(stepped) = &mut data {
            self.list_supply(stepped);
        }
        data
    }

    /// Lists, at the end of `data`'s changes, each `rating.supply_low` episode that opened or
    /// closed since the last listing, as the snapshot shows it now. An episode lists once on the
    /// first listing that ends inside it and once more on the one where it closes with a warning.
    /// One that opens and closes between listings lists once. A power cycle that closes before
    /// its first listing never lists, and the snapshot leaves it out too.
    pub(super) fn list_supply(&mut self, stepped: &mut Stepped) {
        let probe = self.engine.probe();
        let marks = self.engine.supply_marks(&probe);
        let fresh = |mark: &SupplyMark| {
            let listed = self.listed_supply.iter().find(|m| m.pin == mark.pin);
            let (began, closed) = listed.map_or((0, 0), |m| (m.began, m.closed));
            mark.closed > closed || (mark.open && mark.began > began)
        };
        if marks.iter().any(fresh) {
            let found = self.findings(&probe);
            for mark in marks.iter().filter(|m| fresh(m)) {
                let Some(d) = found.iter().find(
                    |d| matches!(d, EngineDiagnostic::RatingSupplyLow(e) if e.pin == mark.pin),
                ) else {
                    continue;
                };
                if stepped.changes.len() < CHANGES {
                    stepped.changes.push(Change::Diagnostic {
                        by_ps: d.at().map_or(probe.at().as_ps(), Time::as_ps),
                        diagnostic: self.diagnostic(d),
                    });
                } else {
                    stepped.more += 1;
                }
            }
        }
        drop(probe);
        self.listed_supply = marks;
    }

    /// Steps as `step` asks. A step that runs ends `play`, and one refused before it runs
    /// leaves it running. The changes it returns are this step's own. What `play` held stays
    /// where it is, for [`Self::step_and_list`] or `pause`.
    pub(super) fn step(
        &mut self,
        step: &Step,
        during: &mut dyn FnMut(&Progress) -> ControlFlow<()>,
    ) -> Outcome {
        let until = match step {
            Step::For(_) => None,
            Step::Until { condition, timeout } => {
                Some((condition, *timeout, self.bind(condition).map_err(fail)?))
            }
        };
        match (self.engine.status(), step) {
            (Status::Faulted, _) => return Err(fail(self.faulted())),
            (Status::Stopped, Step::For(_)) => {
                return Err(fail(Error::new(
                    Code::Ended,
                    format!("the session reached its {} stop", self.stop_name()),
                    "run `open` to start again",
                )));
            }
            _ => {}
        }
        let from = self.engine.now().as_ps();
        let end = match step {
            Step::For(length) => from
                .checked_add(length.as_ps())
                .filter(|&end| end <= self.stop_ps)
                .ok_or_else(|| {
                    fail(Error::new(
                        Code::PastStop,
                        format!(
                            "the step would end past the session's {} stop",
                            self.stop_name()
                        ),
                        format!(
                            "the session has {} left; step at most that",
                            t(self.stop_ps - from)
                        ),
                    ))
                })?,
            Step::Until { timeout, .. } => from.saturating_add(timeout.as_ps()).min(self.stop_ps),
        };
        self.playing = false;
        let probe = self.engine.probe();
        self.tone.note(&probe, from);
        let mut known: BTreeSet<Key> = self.findings(&probe).iter().map(key).collect();
        drop(probe);
        let mut changes = Vec::new();
        let mut more = 0;
        let mut list = |found: Vec<Change>| {
            for change in found {
                if changes.len() < CHANGES {
                    changes.push(change);
                } else {
                    more += 1;
                }
            }
        };
        let mut status = self.engine.status();
        list(self.drain(status != Status::Running).changes);
        let mut met = until
            .as_ref()
            .filter(|(_, _, bound)| self.observe(bound).0)
            .map(|_| Met::At(from));
        let pin = until.as_ref().and_then(|(_, _, bound)| match bound {
            Bound::Level { signal, want, .. } => Some((*signal, *want)),
            _ => None,
        });
        let mut now = from;
        let mut cancelled = false;
        while met.is_none() && now < end && status == Status::Running {
            let to = now.saturating_add(SLICE.as_ps()).min(end);
            let mut found = Vec::new();
            let edge;
            let sample_at_end;
            (status, edge, sample_at_end) = self.advance_slice(to, end, from, pin, &mut found);
            now = self.engine.now().as_ps();
            let slice = self.drain(status != Status::Running);
            if let Some((_, _, bound)) = &until {
                met = edge.map(Met::Edge).or_else(|| self.met_in(bound, &slice));
            }
            found.extend(slice.changes);
            let probe = self.engine.probe();
            if sample_at_end {
                found.extend(self.light.sample(&probe));
            }
            self.tone.note(&probe, now);
            if let Some((_, _, Bound::Light { index, want, .. })) = &until
                && met.is_some()
                && let Some(change) = self.light.list_asked(*index, *want, &probe)
            {
                found.push(change);
            }
            for d in self.findings(&probe) {
                // `list_supply` lists these once the step ends.
                if matches!(d, EngineDiagnostic::RatingSupplyLow(_)) {
                    continue;
                }
                if d.at().is_some() && known.insert(key(&d)) {
                    found.push(Change::Diagnostic {
                        by_ps: d.at().map_or(now, Time::as_ps),
                        diagnostic: self.diagnostic(&d),
                    });
                }
            }
            drop(probe);
            found.sort_by_key(Change::ps);
            list(found);
            let progress = Progress {
                open: self,
                from_ps: from,
                end_ps: end,
            };
            let stop = during(&progress).is_break();
            if stop && met.is_none() && now < end && status == Status::Running {
                cancelled = true;
                break;
            }
        }
        let check = until.as_ref().map(|(condition, _, bound)| {
            let (passed, actual) = match (met, bound) {
                (Some(Met::At(at) | Met::Edge(at)), Bound::Level { pin, want, .. }) => {
                    (true, format!("{pin} {} at {}", want.as_str(), t(at)))
                }
                (met, _) => (met.is_some(), self.observe(bound).1),
            };
            Check {
                passed,
                expected: condition.to_string(),
                actual,
            }
        });
        let glows = {
            let probe = self.engine.probe();
            self.light.glows(&probe)
        };
        let leds = glows
            .into_iter()
            .enumerate()
            .map(|(index, glow)| LedRead {
                part: self.light.lamps()[index].path.clone(),
                state: glow.state,
                brightness: glow.amperes,
                over_ps: glow.over_ps,
            })
            .collect();
        let steady = until.as_ref().and_then(|(_, _, bound)| {
            let Bound::Light { index, want, .. } = bound else {
                return None;
            };
            let probe = self.engine.probe();
            let end = self.light.glow(*index, &probe);
            self.light.steady_hint(*index, *want, end)
        });
        let data = Data::Stepped(Stepped {
            from_ps: from,
            met,
            changes,
            more,
            check,
            leds,
        });
        if cancelled {
            return Err(failed(
                Error::new(
                    Code::Cancelled,
                    format!("the step was cancelled at t={}", t(now)),
                    "the session stands there; step again to go on",
                ),
                data,
            ));
        }
        match (status, until) {
            (Status::Faulted, _) => Err(failed(self.faulted(), data)),
            (_, Some((_, timeout, _))) if met.is_none() => {
                let error = if from.saturating_add(timeout.as_ps()) > self.stop_ps {
                    Error::new(
                        Code::Ended,
                        "the session reached its stop before the condition held",
                        "run `open` to start again",
                    )
                } else {
                    Error::new(
                        Code::UntilTimeout,
                        format!("the condition did not hold within {timeout}"),
                        steady.unwrap_or_else(|| {
                            "step longer with `--timeout`, or check the circuit with `snapshot`"
                                .to_string()
                        }),
                    )
                };
                Err(failed(error, data))
            }
            _ => Ok(data),
        }
    }

    /// Keeps what one `play` tick ran, after the changes already held, up to [`CHANGES`].
    pub(super) fn hold(&mut self, mut stepped: Stepped) {
        let Some(held) = self.unlisted.as_mut() else {
            self.unlisted = Some(stepped);
            return;
        };
        let room = CHANGES.saturating_sub(held.changes.len());
        if stepped.changes.len() > room {
            held.more += stepped.changes.len() - room;
            stepped.changes.truncate(room);
        }
        held.more += stepped.more;
        held.changes.append(&mut stepped.changes);
        held.leds = stepped.leds;
    }

    /// Takes the edges and bytes the engine logged since the last drain. Every edge through
    /// `now` is listed once. The cursor stays before an edge at `now`, which reads still
    /// leave out until time moves past it. Serial text is dated by `now`, when the session
    /// read it.
    fn drain(&mut self, ended: bool) -> Slice {
        let now = self.engine.now();
        let mut slice = Slice::default();
        if let Some((edges, _)) = self.engine.edges_since(self.edges) {
            self.tone.note_edges(edges, now.as_ps());
            let base = u64::from(self.edges);
            let listed = (u64::from(self.listed) - base) as usize;
            for edge in &edges[listed..] {
                slice.changes.push(Change::Edge {
                    at_ps: edge.at.as_ps(),
                    pin: self.signal_pins[edge.signal].clone(),
                    level: level(edge.level),
                });
            }
            let read = if ended {
                edges.len()
            } else {
                edges
                    .iter()
                    .position(|e| e.at >= now)
                    .unwrap_or(edges.len())
            };
            self.listed = EdgeCursor::from(base + edges.len() as u64);
            let next = EdgeCursor::from(base + read as u64);
            self.edges = next;
            self.engine.forget_edges(next);
        }
        if let Some((bytes, next)) = self.engine.serial_since(self.serial.cursor) {
            let timed: Vec<(u8, u64)> = bytes.iter().map(|b| (b.byte, b.at.as_ps())).collect();
            self.serial.cursor = next;
            self.engine.forget_serial(next);
            slice.arrived = self.serial.arrive(timed.into_iter(), ended);
        }
        slice
            .changes
            .extend(self.serial.change(&slice.arrived, now.as_ps()));
        slice
    }
}
