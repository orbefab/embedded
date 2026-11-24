//! The frequency a listener would hear from a part whose reading is `"of": "hz"`.
//!
//! The pitch is the frequency of the drive's full cycles, rise to rise of the driving pin, as
//! they happen. Those rises are the chip's own edges, so a 1 kHz tone is not read off a 1 ms
//! sample. The reading is the median cycle of the note now playing and never mixes two notes.
//! A cycle within 3% of that median is the same note. Two cycles in a row that agree with each
//! other and not with it start a new note, so a new note reads its own frequency from its
//! second full cycle, or its first after a rest. Until then the note before still reads. The
//! cycle that spans the change, which no one played, never reads.
//!
//! A rest is the pin not moving for 17 ms, longer than the half-cycle of 31 Hz, the lowest
//! note `tone()` plays on an Uno or Nano. A rest reads 0 and clears every cycle before it, so
//! the next note starts from nothing. Any falling step between notes moves the pin before
//! then and does not read 0. The cost is that `noTone()` or the end of `tone(pin, f, ms)`
//! reads 0 only once 17 ms have passed with no edge. A pulse train slower than 31 Hz rests
//! between pulses and reads 0, as separate clicks with no pitch.
//!
//! A cycle counts only while the coil swings by at least 0.2 V within the last cycle, so a
//! coil whose supply is cut or whose circuit is open reads 0 within one cycle. Play probes the
//! coil every 20 us, which is enough to see a narrow PWM pulse.
//!
//! A pulse of either level more than 4 times shorter than both half-cycles after it is a
//! glitch and not a cycle. `tone()` starts each note from silence with a pulse of 7 us to
//! 60 us, and a 31 Hz half-cycle is 16 ms. A narrow PWM pulse repeats at its own width, so it
//! still counts. A pitch above 20 kHz is not heard and reads 0.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use orbe_engine::Probe;
use orbe_engine::logic::{Change, Level as EngineLevel};
use orbe_engine::name::Quantity;
use orbe_scene::RefMap;

use crate::reply::Code;

/// Below this swing across the coil within the last cycle, the coil is not being driven.
const EDGE_V: f64 = 0.2;
/// The pin has rested when it has not moved for this long.
const REST_PS: u64 = orbe_engine::passive::DRIVE_REST_PS;
/// A pulse this many times shorter than both half-cycles after it is a glitch.
const GLITCH_RATIO: u64 = 4;
/// Two cycles within this fraction of the longer are the same note. A semitone is 6%.
const SAME_PITCH: f64 = 0.03;
/// Above this no one hears a pitch.
const AUDIBLE_HZ: f64 = 20_000.0;
/// Edges kept per pin, 16 ms of a 4 kHz note.
const HISTORY: usize = 128;

#[derive(Clone, Copy, Debug)]
struct Flip {
    at: u64,
    high: bool,
}

/// One pin that can drive the coil, and its edges since it last rested.
#[derive(Clone, Debug)]
struct Track {
    signal: usize,
    flips: VecDeque<Flip>,
}

#[derive(Clone, Debug)]
struct Meter {
    path: String,
    reading: String,
    column: orbe_engine::name::Ref,
    tracks: Vec<Track>,
    /// The coil voltage at each probe, back to the longest cycle a note can have.
    coil: VecDeque<(u64, f64)>,
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
                tracks: watched
                    .into_iter()
                    .map(|signal| Track {
                        signal,
                        flips: VecDeque::new(),
                    })
                    .collect(),
                coil: VecDeque::new(),
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

    /// The coil voltage at `now_ps`.
    pub(super) fn note(&mut self, probe: &Probe, now_ps: u64) {
        for meter in &mut self.meters {
            let volts = probe.get(&meter.column).expect("a checked column").get();
            if meter.coil.back().is_some_and(|&(at, _)| at >= now_ps) {
                meter.coil.pop_back();
            }
            meter.coil.push_back((now_ps, volts));
            // One sample older than the longest cycle stays, as the level that cycle began at.
            let oldest = now_ps.saturating_sub(2 * REST_PS);
            while meter.coil.get(1).is_some_and(|&(at, _)| at <= oldest) {
                meter.coil.pop_front();
            }
        }
    }

    /// Edges through `now_ps`. An edge at `now_ps` comes again in the next batch.
    pub(super) fn note_edges(&mut self, edges: &[Change], now_ps: u64) {
        for meter in &mut self.meters {
            for edge in edges {
                let at = edge.at.as_ps();
                if at > now_ps {
                    continue;
                }
                let Some(track) = meter.tracks.iter_mut().find(|t| t.signal == edge.signal) else {
                    continue;
                };
                track.push(Flip {
                    at,
                    high: edge.level == EngineLevel::High,
                });
            }
        }
    }

    pub(super) fn hz(&self, index: usize, now_ps: u64) -> f64 {
        let meter = &self.meters[index];
        let mut best = 0.0_f64;
        for track in &meter.tracks {
            let flips: Vec<Flip> = track.flips.iter().copied().collect();
            let Some((period, cycle_from)) = period(&flips, now_ps) else {
                continue;
            };
            if meter.swing(cycle_from, now_ps) < EDGE_V {
                continue;
            }
            let hz = 1e12 / period as f64;
            if hz <= AUDIBLE_HZ {
                best = best.max(hz);
            }
        }
        best
    }
}

impl Track {
    fn push(&mut self, flip: Flip) {
        if let Some(last) = self.flips.back() {
            if flip.at < last.at || (flip.at == last.at && flip.high == last.high) {
                return;
            }
            if flip.at == last.at {
                // A pulse of no width is no edge.
                self.flips.pop_back();
                return;
            }
            if flip.at - last.at > REST_PS {
                self.flips.clear();
            }
        }
        self.flips.push_back(flip);
        while self.flips.len() > HISTORY {
            self.flips.pop_front();
        }
    }
}

impl Meter {
    /// The coil's swing from `from_ps` to `to_ps`, counting the level it held at `from_ps`.
    fn swing(&self, from_ps: u64, to_ps: u64) -> f64 {
        let start = self.coil.partition_point(|&(at, _)| at <= from_ps);
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(at, volts) in self.coil.range(start.saturating_sub(1)..) {
            if at > to_ps {
                break;
            }
            low = low.min(volts);
            high = high.max(volts);
        }
        if low > high { 0.0 } else { high - low }
    }
}

/// The period of the note `flips` play at `now_ps`, in picoseconds, and the rise that began
/// the latest full cycle. `None` when the pin has rested or no full cycle has passed.
fn period(flips: &[Flip], now_ps: u64) -> Option<(u64, u64)> {
    let last = flips.last()?;
    if now_ps.saturating_sub(last.at) > REST_PS {
        return None;
    }
    let rises = rises(flips, now_ps);
    let intervals: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
    Some((note(&intervals)?, rises[rises.len() - 2]))
}

/// The rises that start a cycle: those of `flips` left once each glitch is taken out.
fn rises(flips: &[Flip], now_ps: u64) -> Vec<u64> {
    let length = |k: usize| -> Option<(u64, bool)> {
        let flip = flips.get(k)?;
        Some(match flips.get(k + 1) {
            Some(next) => (next.at - flip.at, true),
            None => (now_ps.saturating_sub(flip.at), false),
        })
    };
    let mut keep = vec![true; flips.len()];
    for k in 0..flips.len().saturating_sub(1) {
        if !keep[k] {
            continue;
        }
        let width = flips[k + 1].at - flips[k].at;
        let limit = width.saturating_mul(GLITCH_RATIO);
        let Some((after, _)) = length(k + 1) else {
            continue;
        };
        if after <= limit {
            continue;
        }
        // Until the second half-cycle after it is long, or ends short, the pulse is held out.
        let glitch = match length(k + 2) {
            None => true,
            Some((then, done)) => then > limit || !done,
        };
        if glitch {
            keep[k] = false;
            keep[k + 1] = false;
        }
    }
    let mut out = Vec::new();
    let mut level = None;
    for (flip, kept) in flips.iter().zip(keep) {
        if !kept || level == Some(flip.high) {
            continue;
        }
        level = Some(flip.high);
        if flip.high {
            out.push(flip.at);
        }
    }
    out
}

fn same(a: u64, b: u64) -> bool {
    a.abs_diff(b) as f64 <= SAME_PITCH * a.max(b) as f64
}

/// The period of the note the last of `intervals` belong to: the median of its cycles.
fn note(intervals: &[u64]) -> Option<u64> {
    let mut run: Vec<u64> = Vec::new();
    let mut held: Option<u64> = None;
    for &cycle in intervals {
        let Some(center) = median(&run) else {
            run.push(cycle);
            continue;
        };
        if same(cycle, center) {
            run.push(cycle);
            held = None;
            continue;
        }
        match held.take() {
            // A late edge and the early one after it.
            Some(before) if same((before + cycle) / 2, center) => run.extend([before, cycle]),
            Some(before) if same(before, cycle) => run = vec![before, cycle],
            _ => held = Some(cycle),
        }
    }
    median(&run)
}

fn median(run: &[u64]) -> Option<u64> {
    let mut sorted = run.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    match sorted.len() {
        0 => None,
        n if n % 2 == 1 => Some(sorted[mid]),
        _ => Some((sorted[mid - 1] + sorted[mid]) / 2),
    }
}
