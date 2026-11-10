//! `step`: time advanced in slices, with each slice's changes and a caller's hook between them.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use orbe_engine::name::Ref as EngineRef;
use orbe_engine::time::Time;
use orbe_engine::{EdgeCursor, Probe, Status};

use super::bound::Bound;
use super::serial::Arrived;
use super::{Key, Open, Outcome, fail, failed, key, level, t};
use crate::command::Step;
use crate::duration::Duration;
use crate::reply::{Change, Check, Code, Data, Diagnostic, Error, Got, Level, Met, Stepped};
use crate::target::Ref;

/// `step` advances in slices this long, so `--until` and a caller see each one.
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
    /// Advances to `to`. For a pin condition it goes one landing at a time and stops at the
    /// first edge from `from` on that meets it, then 1 ps past the edge when `end` allows, so
    /// reads show it. Returns that edge's time.
    fn advance(
        &mut self,
        to: u64,
        end: u64,
        from: u64,
        pin: Option<(usize, Level)>,
    ) -> (Status, Option<u64>) {
        let Some((signal, want)) = pin else {
            return (self.engine.advance_to(Time::from_ps(to)), None);
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
                }
                return (self.engine.status(), Some(at));
            }
            let now = self.engine.now().as_ps();
            if now >= to || self.engine.status() != Status::Running {
                return (self.engine.status(), None);
            }
            let landing = self.engine.next_at().map_or(to, |at| at.as_ps().min(to));
            self.engine.advance_to(Time::from_ps(landing));
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

    fn lit(&self, leds: &[(String, EngineRef, f64)], probe: &Probe) -> Vec<bool> {
        leds.iter()
            .map(|(_, i, above)| probe.get(i).expect("a checked column").get() > *above)
            .collect()
    }

    pub(super) fn stop_name(&self) -> String {
        if self.stop_ps == orbe_scene::STOP_PS {
            "2-hour".into()
        } else {
            t(self.stop_ps).to_string()
        }
    }

    /// Steps as `step` asks. A step that runs ends `play`, and one refused before it runs
    /// leaves it running. It lists what `play` ran since the last `step` first, and a step
    /// refused at a fault or the stop lists those in its refusal, since no later step runs.
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
            (Status::Faulted, _) => return Err(failed(self.faulted(), self.unlisted())),
            (Status::Stopped, Step::For(_)) => {
                let error = Error::new(
                    Code::Ended,
                    format!("the session reached its {} stop", self.stop_name()),
                    "run `open` to start again",
                );
                return Err(failed(error, self.unlisted()));
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
        let leds: Vec<(String, EngineRef, f64)> = self
            .refs
            .top
            .iter()
            .filter_map(|p| {
                let (i, above) = self.on_threshold(p).ok()?;
                Some((p.clone(), i, above))
            })
            .collect();
        let probe = self.engine.probe();
        let mut was = self.lit(&leds, &probe);
        let mut known: BTreeSet<Key> = self.findings(&probe).iter().map(key).collect();
        drop(probe);
        let (listed_from, mut changes, mut more) = match self.unlisted.take() {
            Some(played) => (played.from_ps, played.changes, played.more),
            None => (from, Vec::new(), 0),
        };
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
            let edge;
            (status, edge) = self.advance(to, end, from, pin);
            now = self.engine.now().as_ps();
            let slice = self.drain(status != Status::Running);
            if let Some((_, _, bound)) = &until {
                met = edge.map(Met::Edge).or_else(|| self.met_in(bound, &slice));
            }
            let mut found = slice.changes;
            let probe = self.engine.probe();
            let lit = self.lit(&leds, &probe);
            for (((part, ..), before), after) in leds.iter().zip(&was).zip(&lit) {
                if before != after {
                    found.push(Change::Led {
                        by_ps: now,
                        part: part.clone(),
                        on: *after,
                    });
                }
            }
            was = lit;
            for d in self.findings(&probe) {
                if d.at().is_some() && known.insert(key(&d)) {
                    found.push(Change::Diagnostic {
                        by_ps: d.at().map_or(now, Time::as_ps),
                        diagnostic: Diagnostic::new(&d),
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
        let data = Data::Stepped(Stepped {
            from_ps: listed_from,
            met,
            changes,
            more,
            check,
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
                        "step longer with `--timeout`, or check the circuit with `snapshot`",
                    )
                };
                Err(failed(error, data))
            }
            _ => Ok(data),
        }
    }

    fn unlisted(&mut self) -> Data {
        self.unlisted.take().map_or(Data::None, Data::Stepped)
    }

    /// Takes the edges and bytes the engine logged since the last drain. Every edge through
    /// `now` is listed once. The cursor stays before an edge at `now`, which reads still
    /// leave out until time moves past it. Serial text is dated by `now`, when the session
    /// read it.
    fn drain(&mut self, ended: bool) -> Slice {
        let now = self.engine.now();
        let mut slice = Slice::default();
        if let Some((edges, _)) = self.engine.edges_since(self.edges) {
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
