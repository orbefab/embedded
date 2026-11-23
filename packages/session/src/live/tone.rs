//! The frequency of the square wave across a part whose reading is `"of": "hz"`.
//!
//! `tone()` toggles a chip pin. The part's `hz` is that pin's frequency when the pin
//! reaches the part through resistors or a transistor, and the voltage across the part
//! is actually moving. A quiet coil, including DC across it, reads 0. Edges at `now`
//! stay out until time moves past them, the same rule as a pin read.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use orbe_engine::Probe;
use orbe_engine::logic::{Change, Level as EngineLevel};
use orbe_engine::name::{Quantity, Ref as EngineRef};
use orbe_scene::RefMap;

use super::quantity;
use crate::duration::Duration;
use crate::reply::Code;

/// How far back `hz` looks. A 40 ms window holds several cycles of a starter-kit note
/// and still clears a 200 ms rest.
const WINDOW: Duration = Duration::from_ms(40);
/// Below this peak-to-peak voltage the coil is not being driven, so `hz` is 0
/// even when a nearby pin is toggling.
const SWING_V: f64 = 0.2;
/// A missing next edge longer than this many periods means the wave has stopped.
const STOPPED_PERIODS: f64 = 1.5;

#[derive(Clone, Debug)]
struct Meter {
    path: String,
    /// The name the part file gave the reading, such as `hz`.
    reading: String,
    column: EngineRef,
    /// Chip signals whose nets reach the part without crossing a supply rail.
    signals: Vec<usize>,
    /// Rising edges, `(at_ps, signal)`, inside the window.
    rises: VecDeque<(u64, usize)>,
    /// Voltage samples, `(at_ps, volts)`.
    volts: VecDeque<(u64, f64)>,
}

pub(super) struct Log {
    meters: Vec<Meter>,
}

fn rail(net: &str) -> bool {
    let tail = net.rsplit("__").next().unwrap_or(net);
    matches!(tail, "GND" | "5V" | "3V3" | "VCC" | "VIN")
}

/// Nets joined by a resistor or a transistor, not through a supply rail.
fn bridges(refs: &RefMap) -> BTreeMap<String, Vec<String>> {
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for part in refs.parts.values() {
        for engine in &part.engine {
            if !matches!(
                engine.kind.as_str(),
                "resistor" | "npn" | "pnp" | "nmos" | "pmos"
            ) {
                continue;
            }
            let nets: Vec<&str> = engine.pins.values().map(String::as_str).collect();
            for &a in &nets {
                for &b in &nets {
                    if a != b && !rail(a) && !rail(b) {
                        adj.entry(a.to_string()).or_default().push(b.to_string());
                    }
                }
            }
        }
    }
    adj
}

fn reach(
    start: impl IntoIterator<Item = String>,
    adj: &BTreeMap<String, Vec<String>>,
) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for net in start {
        if rail(&net) {
            continue;
        }
        if seen.insert(net.clone()) {
            queue.push_back(net);
        }
    }
    while let Some(net) = queue.pop_front() {
        for next in adj.get(&net).into_iter().flatten() {
            if seen.insert(next.clone()) {
                queue.push_back(next.clone());
            }
        }
    }
    seen
}

impl Log {
    pub(super) fn build(
        refs: &RefMap,
        reports: &BTreeMap<String, Vec<Quantity>>,
        signals: &BTreeMap<String, usize>,
    ) -> Log {
        let adj = bridges(refs);
        let mut meters = Vec::new();
        for (path, part) in &refs.parts {
            let Some(reading) = part
                .readings
                .iter()
                .find(|(_, reading)| reading.of == "hz")
                .map(|(name, _)| name.clone())
            else {
                continue;
            };
            let Ok(engine) = crate::target::one_engine(path, part, Code::NoReading) else {
                continue;
            };
            let reported = reports.get(&engine).map(Vec::as_slice).unwrap_or(&[]);
            if !reported.contains(&Quantity::V) {
                continue;
            }
            let start = part.pins.values().filter_map(|net| net.clone());
            let mut watched: Vec<usize> = reach(start, &adj)
                .iter()
                .filter_map(|net| signals.get(net).copied())
                .collect();
            watched.sort_unstable();
            watched.dedup();
            meters.push(Meter {
                path: path.clone(),
                reading,
                column: quantity(&engine, Quantity::V),
                signals: watched,
                rises: VecDeque::new(),
                volts: VecDeque::new(),
            });
        }
        Log { meters }
    }

    pub(super) fn position(&self, path: &str) -> Option<usize> {
        self.meters.iter().position(|meter| meter.path == path)
    }

    pub(super) fn reading(&self, index: usize) -> &str {
        &self.meters[index].reading
    }

    pub(super) fn note(&mut self, probe: &Probe) {
        let now = probe.at().as_ps();
        let horizon = now.saturating_sub(WINDOW.as_ps());
        for meter in &mut self.meters {
            let volts = probe.get(&meter.column).expect("a checked column").get();
            if let Some(last) = meter.volts.back_mut()
                && last.0 == now
            {
                last.1 = volts;
            } else {
                meter.volts.push_back((now, volts));
            }
            while meter.volts.len() >= 2 && meter.volts[1].0 <= horizon {
                meter.volts.pop_front();
            }
        }
    }

    /// Rising edges drained with this step. An edge at `now` is left out, as a pin read is.
    pub(super) fn note_edges(&mut self, edges: &[Change], now_ps: u64) {
        let horizon = now_ps.saturating_sub(WINDOW.as_ps());
        for meter in &mut self.meters {
            for edge in edges {
                if edge.level != EngineLevel::High || edge.at.as_ps() >= now_ps {
                    continue;
                }
                if !meter.signals.contains(&edge.signal) {
                    continue;
                }
                let at = edge.at.as_ps();
                if at <= horizon {
                    continue;
                }
                meter.rises.push_back((at, edge.signal));
            }
            while meter.rises.front().is_some_and(|edge| edge.0 <= horizon) {
                meter.rises.pop_front();
            }
        }
    }

    /// Hertz at `now_ps`, or 0 when the coil is quiet.
    pub(super) fn hz(&self, index: usize, now_ps: u64, probe: &Probe) -> f64 {
        let meter = &self.meters[index];
        let horizon = now_ps.saturating_sub(WINDOW.as_ps());
        let mut best: f64 = 0.0;
        for &signal in &meter.signals {
            let rises: Vec<u64> = meter
                .rises
                .iter()
                .filter(|edge| edge.1 == signal && edge.0 > horizon && edge.0 < now_ps)
                .map(|edge| edge.0)
                .collect();
            if rises.len() < 2 {
                continue;
            }
            let first = rises[0];
            let last = *rises.last().expect("at least two");
            let periods = (rises.len() - 1) as f64;
            let span = (last - first) as f64;
            if span <= 0.0 {
                continue;
            }
            if (now_ps - last) as f64 > STOPPED_PERIODS * span / periods {
                continue;
            }
            best = best.max(periods / (span / 1e12));
        }
        if best == 0.0 || self.swing(index, now_ps, probe) < SWING_V {
            0.0
        } else {
            best
        }
    }

    fn swing(&self, index: usize, now_ps: u64, probe: &Probe) -> f64 {
        let meter = &self.meters[index];
        let horizon = now_ps.saturating_sub(WINDOW.as_ps());
        let now = probe.get(&meter.column).expect("a checked column").get();
        let mut low = now;
        let mut high = now;
        for &(at, volts) in &meter.volts {
            if at > horizon && at <= now_ps {
                low = low.min(volts);
                high = high.max(volts);
            }
        }
        high - low
    }
}
