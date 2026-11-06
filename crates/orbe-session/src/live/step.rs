//! `step`: time advanced in slices, with each slice's changes and a caller's hook between them.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use sim_circuit::{EdgeCursor, Probe, Status};
use sim_core::logic::Change as EngineEdge;
use sim_core::name::Ref as EngineRef;
use sim_core::time::Time;

use super::bound::Bound;
use super::serial::Arrived;
use super::{Key, Open, Outcome, fail, failed, key, level, t};
use crate::command::Step;
use crate::duration::Duration;
use crate::reply::{Change, Check, Code, Data, Diagnostic, Error, Got, Met, Stepped};
use crate::target::Ref;

/// `step` advances in slices this long, so `--until` and a caller see each one.
const SLICE: Duration = Duration::from_ms(1);
const CHANGES: usize = 20;

/// Where a running step stands after a slice, for a caller that shows frames or stops it.
pub struct Progress<'a> {
    open: &'a Open,
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
    edges: Vec<EngineEdge>,
    arrived: Arrived,
}

impl Open {
    /// Whether the condition holds as the step starts. A pin's level counts every edge at
    /// `now`, since a step from `now` counts them.
    fn holds_at_start(&self, bound: &Bound) -> bool {
        match bound {
            Bound::Level { signal, want, .. } => level(self.engine.levels()[*signal]) == *want,
            _ => self.observe(bound).0,
        }
    }

    /// When the condition first held inside the slice that just ran: a pin's edge, the start
    /// bit of the byte that completed a serial match, else by the slice's end if it holds there.
    fn met_in(&self, bound: &Bound, slice: &Slice, from: u64) -> Option<Met> {
        match bound {
            Bound::Level { signal, want, .. } => slice
                .edges
                .iter()
                .find(|e| e.signal == *signal && level(e.level) == *want && e.at.as_ps() >= from)
                .map(|e| Met::At(e.at.as_ps())),
            Bound::SerialContains(text) => self
                .serial
                .met(&slice.arrived, text)
                .map(|at| Met::At(at.max(from))),
            _ => self
                .observe(bound)
                .0
                .then(|| Met::By(self.engine.now().as_ps())),
        }
    }

    fn lit(&self, leds: &[(String, EngineRef, f64)], probe: &Probe) -> Vec<bool> {
        leds.iter()
            .map(|(_, i, above)| probe.get(i).expect("a checked column").get() > *above)
            .collect()
    }

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
        match self.engine.status() {
            Status::Running => {}
            Status::Stopped => {
                return Err(fail(Error::new(
                    Code::Ended,
                    "the session reached its 2-hour stop",
                    "run `open` to start again",
                )));
            }
            Status::Faulted => return Err(fail(self.faulted())),
        }
        let from = self.engine.now().as_ps();
        let end = match step {
            Step::For(length) => from
                .checked_add(length.as_ps())
                .filter(|&end| end <= self.stop_ps)
                .ok_or_else(|| {
                    fail(Error::new(
                        Code::PastStop,
                        "the step would end past the session's 2-hour stop",
                        format!(
                            "the session has {} left; step at most that",
                            t(self.stop_ps - from)
                        ),
                    ))
                })?,
            Step::Until { timeout, .. } => from.saturating_add(timeout.as_ps()).min(self.stop_ps),
        };
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
        list(self.drain(from, false).changes);
        let mut met = until
            .as_ref()
            .filter(|(_, _, bound)| self.holds_at_start(bound))
            .map(|_| Met::At(from));
        let mut status = Status::Running;
        let mut now = from;
        let mut cancelled = false;
        while met.is_none() && now < end && status == Status::Running {
            let to = now.saturating_add(SLICE.as_ps()).min(end);
            status = self.engine.advance_to(Time::from_ps(to));
            now = self.engine.now().as_ps();
            let slice = self.drain(from, status != Status::Running);
            if let Some((_, _, bound)) = &until {
                met = self.met_in(bound, &slice, from);
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
                (Some(Met::At(at)), Bound::Level { pin, want, .. }) => {
                    (true, format!("{pin} {} at {}", want.as_str(), t(at)))
                }
                (None, Bound::Level { pin, signal, .. }) => (
                    false,
                    format!("{pin} {}", level(self.engine.levels()[*signal]).as_str()),
                ),
                (Some(_), _) => (true, self.observe(bound).1),
                (None, _) => self.observe(bound),
            };
            Check {
                passed,
                expected: condition.to_string(),
                actual,
            }
        });
        let data = Data::Stepped(Stepped {
            from_ps: from,
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

    /// Takes the edges and bytes the engine logged since the last drain. Every edge through
    /// `now` is listed once. The cursor stays before an edge at `now`, which reads still
    /// leave out until time moves past it.
    fn drain(&mut self, from: u64, ended: bool) -> Slice {
        let now = self.engine.now();
        let mut slice = Slice::default();
        if let Some((edges, _)) = self.engine.edges_since(self.edges) {
            let base = u64::from(self.edges);
            let listed = (u64::from(self.listed) - base) as usize;
            slice.edges = edges[listed..].to_vec();
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
        for edge in &slice.edges {
            slice.changes.push(Change::Edge {
                at_ps: edge.at.as_ps(),
                pin: self.signal_pins[edge.signal].clone(),
                level: level(edge.level),
            });
        }
        if let Some((bytes, next)) = self.engine.serial_since(self.serial.cursor) {
            let timed: Vec<(u8, u64)> = bytes.iter().map(|b| (b.byte, b.at.as_ps())).collect();
            self.serial.cursor = next;
            self.engine.forget_serial(next);
            slice.arrived = self.serial.arrive(timed.into_iter(), ended);
        }
        slice
            .changes
            .extend(self.serial.change(&slice.arrived, from));
        slice
    }
}
