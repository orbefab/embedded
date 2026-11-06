//! `step`: time advanced in slices, with each slice's changes and a caller's hook between them.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use sim_circuit::{EdgeCursor, Probe, Status};
use sim_core::logic::Change as EngineEdge;
use sim_core::name::Ref as EngineRef;
use sim_core::time::Time;

use super::bound::Bound;
use super::{Key, Open, Outcome, fail, failed, find, key, level, t};
use crate::command::Step;
use crate::duration::Duration;
use crate::reply::{Change, Check, Code, Data, Diagnostic, Error, Got, Stepped};
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
    bytes_from: usize,
    byte_at: Vec<u64>,
}

impl Open {
    /// When the condition first held inside the slice that just ran: a pin's edge from `from`
    /// on, the start bit of the byte that completed a serial match, else the slice's end if it
    /// holds there.
    fn met_in(&self, bound: &Bound, slice: &Slice, from: u64) -> Option<u64> {
        match bound {
            Bound::Level { signal, want, .. } => slice
                .edges
                .iter()
                .find(|e| e.signal == *signal && level(e.level) == *want && e.at.as_ps() >= from)
                .map(|e| e.at.as_ps()),
            Bound::SerialContains(text) => {
                let needle = text.as_bytes();
                let from = slice
                    .bytes_from
                    .saturating_sub(needle.len().saturating_sub(1));
                let start = from + find(&self.serial.bytes[from..], needle)?;
                let last = start + needle.len().max(1) - 1;
                slice
                    .byte_at
                    .get(last.checked_sub(slice.bytes_from)?)
                    .copied()
            }
            _ => self.observe(bound).0.then(|| self.engine.now().as_ps()),
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
        let (length, until) = match step {
            Step::For(length) => (*length, None),
            Step::Until { condition, timeout } => (
                *timeout,
                Some((condition, self.bind(condition).map_err(fail)?)),
            ),
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
        let end = from
            .checked_add(length.as_ps())
            .filter(|&end| end <= self.stop_ps)
            .ok_or_else(|| {
                let left = t(self.stop_ps - from);
                let hint = match until {
                    None => format!("the session has {left} left; step at most that"),
                    Some(_) => {
                        format!("the session has {left} left; give `--timeout` at most that")
                    }
                };
                fail(Error::new(
                    Code::PastStop,
                    "the step would end past the session's 2-hour stop",
                    hint,
                ))
            })?;
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
        let mut met_at_ps = until
            .as_ref()
            .filter(|(_, bound)| self.observe(bound).0)
            .map(|_| from);
        let mut status = Status::Running;
        let mut now = from;
        let mut cancelled = false;
        while met_at_ps.is_none() && now < end && status == Status::Running {
            let to = now.saturating_add(SLICE.as_ps()).min(end);
            status = self.engine.advance_to(Time::from_ps(to));
            now = self.engine.now().as_ps();
            let slice = self.drain(status != Status::Running);
            if let Some((_, bound)) = &until {
                met_at_ps = self.met_in(bound, &slice, from);
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
            for change in found {
                if changes.len() < CHANGES {
                    changes.push(change);
                } else {
                    more += 1;
                }
            }
            let progress = Progress {
                open: self,
                from_ps: from,
                end_ps: end,
            };
            let stop = during(&progress).is_break();
            if stop && met_at_ps.is_none() && now < end && status == Status::Running {
                cancelled = true;
                break;
            }
        }
        let check = until.as_ref().map(|(condition, bound)| {
            let (passed, actual) = match (met_at_ps, bound) {
                (Some(at), Bound::Level { pin, want, .. }) => {
                    (true, format!("{pin} {} at {}", want.as_str(), t(at)))
                }
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
            met_at_ps,
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
        match status {
            Status::Faulted => Err(failed(self.faulted(), data)),
            _ if until.is_some() && met_at_ps.is_none() => Err(failed(
                Error::new(
                    Code::UntilTimeout,
                    format!("the condition did not hold within {length}"),
                    "step longer with `--timeout`, or check the circuit with `snapshot`",
                ),
                data,
            )),
            _ => Ok(data),
        }
    }

    fn drain(&mut self, ended: bool) -> Slice {
        let now = self.engine.now();
        let mut slice = Slice::default();
        if let Some((edges, _)) = self.engine.edges_since(self.edges) {
            let read = if ended {
                edges.len()
            } else {
                edges
                    .iter()
                    .position(|e| e.at >= now)
                    .unwrap_or(edges.len())
            };
            slice.edges = edges[..read].to_vec();
            let next = EdgeCursor::from(u64::from(self.edges) + read as u64);
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
        slice.bytes_from = self.serial.bytes.len();
        if let Some((bytes, next)) = self.engine.serial_since(self.serial.cursor) {
            slice.byte_at = bytes.iter().map(|b| b.at.as_ps()).collect();
            self.serial.bytes.extend(bytes.iter().map(|b| b.byte));
            self.serial.unshown_at.extend(&slice.byte_at);
            self.serial.cursor = next;
            self.engine.forget_serial(next);
        }
        slice.changes.extend(self.serial.show(ended));
        slice
    }
}
