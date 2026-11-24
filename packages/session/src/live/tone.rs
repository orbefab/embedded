//! The frequency a listener would hear from a part whose reading is `"of": "hz"`.
//!
//! The pitch is the time from one rise of the driving pin to the next. Those rises are the
//! chip's edges. The gap between landings is the real half-cycle, so a 1 kHz tone is not read
//! off a 1 ms sample. A new pitch replaces the old one when the next cycle of the new pitch
//! finishes, about one period after it starts. One interval that does not match the cycle
//! before it is the gap where the note changed, and it is not a pitch anyone played.
//!
//! The reading falls to 0 when the pin has not moved for 1.5 cycles of that pitch. A listener
//! hears the note stop once its next cycle does not arrive. A lower note within about an
//! octave still moves the pin before then, because the wait is measured from the last edge of
//! either direction, so a falling step does not sound like a rest. A 31 Hz square wave, the
//! lowest `tone()` on a Nano, changes every half-cycle and stays inside that wait.
//!
//! The coil voltage has to move by at least 0.2 V. Play probes it every 20 us, which is enough
//! to see a narrow PWM pulse and a whole-kHz tone, and not once per picosecond of a stiff edge.
//! A rise
//! followed by a fall in under 15 us is not a cycle. A half-cycle at 20 kHz is 25 us, and the
//! few-microsecond pulse at the start of `tone()` is shorter than that.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use orbe_engine::Probe;
use orbe_engine::logic::{Change, Level as EngineLevel};
use orbe_engine::name::Quantity;
use orbe_scene::RefMap;

use crate::reply::Code;

/// Below this peak-to-peak voltage the coil is not being driven.
const EDGE_V: f64 = 0.2;
/// Shorter than a half-cycle at the top of hearing, and longer than `tone()`'s startup pulse.
const GLITCH_PS: u64 = 15_000_000;
/// The pin has stopped when this many of the last cycles pass with no edge.
const STOP_CYCLES: f64 = 1.5;
/// Two periods within this fraction are the same pitch. It covers a late compare interrupt
/// and still treats an octave step as a new note.
const SAME_PITCH: f64 = 0.2;
/// How many recent cycles to average once a pitch has settled.
const AVERAGE: usize = 8;

#[derive(Clone, Copy, Debug)]
struct Flip {
    at: u64,
    signal: usize,
    high: bool,
}

#[derive(Clone, Debug)]
struct Meter {
    path: String,
    reading: String,
    column: orbe_engine::name::Ref,
    signals: Vec<usize>,
    flips: VecDeque<Flip>,
    /// The coil voltages seen so far. A stiff driver takes many tiny steps, so the swing is
    /// the extremes, not the last handful of samples.
    low: f64,
    high: f64,
    swung: bool,
}

pub(super) struct Log {
    meters: Vec<Meter>,
}

fn rail(net: &str) -> bool {
    let tail = net.rsplit("__").next().unwrap_or(net);
    matches!(tail, "GND" | "5V" | "3V3" | "VCC" | "VIN")
}

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
                column: super::quantity(&engine, Quantity::V),
                signals: watched,
                flips: VecDeque::new(),
                low: 0.0,
                high: 0.0,
                swung: false,
            });
        }
        Log { meters }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.meters.is_empty()
    }

    pub(super) fn position(&self, path: &str) -> Option<usize> {
        self.meters.iter().position(|meter| meter.path == path)
    }

    pub(super) fn reading(&self, index: usize) -> &str {
        &self.meters[index].reading
    }

    pub(super) fn note(&mut self, probe: &Probe) {
        for meter in &mut self.meters {
            let volts = probe
                .get(&meter.column)
                .expect("a checked column")
                .get()
                .abs();
            if !meter.swung {
                meter.low = volts;
                meter.high = volts;
                meter.swung = true;
            } else {
                meter.low = meter.low.min(volts);
                meter.high = meter.high.max(volts);
            }
        }
    }

    pub(super) fn note_edges(&mut self, edges: &[Change], now_ps: u64) {
        for meter in &mut self.meters {
            for edge in edges {
                let at = edge.at.as_ps();
                if at > now_ps || !meter.signals.contains(&edge.signal) {
                    continue;
                }
                let high = edge.level == EngineLevel::High;
                if meter.flips.back().is_some_and(|flip| {
                    flip.at == at && flip.signal == edge.signal && flip.high == high
                }) {
                    continue;
                }
                meter.flips.push_back(Flip {
                    at,
                    signal: edge.signal,
                    high,
                });
            }
            while meter.flips.len() > 48 {
                meter.flips.pop_front();
            }
        }
    }

    pub(super) fn hz(&self, index: usize, now_ps: u64) -> f64 {
        let meter = &self.meters[index];
        if meter.swing() < EDGE_V {
            return 0.0;
        }
        let mut best = 0.0_f64;
        for &signal in &meter.signals {
            let flips: Vec<Flip> = meter
                .flips
                .iter()
                .copied()
                .filter(|flip| flip.signal == signal)
                .collect();
            let Some(period) = pitch(&rises(&flips)) else {
                continue;
            };
            let last = flips.last().map(|flip| flip.at).unwrap_or(0);
            if (now_ps.saturating_sub(last)) as f64 > STOP_CYCLES * period as f64 {
                continue;
            }
            best = best.max(1e12 / period as f64);
        }
        best
    }
}

impl Meter {
    fn swing(&self) -> f64 {
        if self.swung {
            self.high - self.low
        } else {
            0.0
        }
    }
}

/// Rising edges that are not the start of a pulse shorter than [`GLITCH_PS`].
fn rises(flips: &[Flip]) -> Vec<u64> {
    let mut out = Vec::new();
    for (i, flip) in flips.iter().enumerate() {
        if !flip.high {
            continue;
        }
        let glitch = flips
            .get(i + 1)
            .is_some_and(|next| !next.high && next.at.saturating_sub(flip.at) < GLITCH_PS);
        if !glitch {
            out.push(flip.at);
        }
    }
    out
}

fn close(a: u64, b: u64) -> bool {
    let scale = a.max(b) as f64;
    scale > 0.0 && a.abs_diff(b) as f64 / scale <= SAME_PITCH
}

fn pitch(rises: &[u64]) -> Option<u64> {
    if rises.len() < 2 {
        return None;
    }
    let intervals: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let last = *intervals.last().expect("at least one");
    if intervals.len() >= 2 {
        let prev = intervals[intervals.len() - 2];
        if !close(last, prev) && intervals.len() >= 3 && close(prev, intervals[intervals.len() - 3])
        {
            return Some(mean_run(&intervals[..intervals.len() - 1]));
        }
    }
    Some(mean_run(&intervals))
}

fn mean_run(intervals: &[u64]) -> u64 {
    let last = *intervals.last().expect("at least one");
    let mut sum = 0u64;
    let mut count = 0u64;
    for &interval in intervals.iter().rev().take(AVERAGE) {
        if !close(interval, last) {
            break;
        }
        sum += interval;
        count += 1;
    }
    sum / count.max(1)
}
