//! The frequency a listener would hear from a part whose reading is `"of": "hz"`.
//!
//! The rule: at every sample, `hz` is 0, the note that was playing, or the note now playing,
//! within 0.5% of the timer frequency, and never any other pitch. It shows the note now
//! playing no later than the end of that note's first full cycle (rise to rise) after its
//! start. A `tone()` start pulse is not a cycle. A drive whose high and low differ, such as
//! narrow PWM, shows from the end of its second full cycle, since its first looks the same as
//! a start pulse and the half-cycle after it. It reads 0 within 17 ms after the drive stops or
//! the coil loses its swing. A steady PWM at any duty that sounds reads the PWM rate.
//!
//! How: a full cycle of a pin that can drive the coil counts when its high and low agree
//! within 0.5% of the cycle, as each cycle of `tone()` does, or when its high and low repeat
//! the cycle before, as each cycle of PWM does. The reading is the median of the cycles that
//! count and are within 0.6% of the latest one, on the pin whose cycle that counts ended
//! last. A cycle that spans a note change has one half of the old note or the new one and one
//! of neither, so it counts only within 0.5% of one of them. Until the new note's first full
//! cycle ends, the note before still reads. A start pulse is shorter than the half-cycle after
//! it, so it never counts. An interrupt that delays an edge by a few microseconds can move a
//! cycle that still counts by 0.5%, and the median leaves it out.
//!
//! A pin rests when it has not moved for 17 ms, longer than the half-cycle of 31 Hz, the
//! lowest note `tone()` plays on an Uno or Nano. When the pin with the latest cycle has
//! rested, the reading is 0, never an older note on another pin. So `noTone()` or the end of
//! `tone(pin, f, ms)` reads 0 once 17 ms have passed with no edge.
//!
//! The coil must swing by 0.2 V from the start of that latest cycle to now, so a coil whose
//! supply is cut or whose circuit is open reads 0 within one cycle. Play probes the coil every
//! 20 us and keeps its average between probes too, so a pulse narrower than that still moves
//! it. A pitch above 20 kHz is not heard and reads 0.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use orbe_engine::Probe;
use orbe_engine::logic::{Change, Level as EngineLevel};
use orbe_engine::name::Quantity;
use orbe_scene::RefMap;

use crate::reply::Code;

/// Below this swing across the coil since the latest cycle began, the coil is not driven.
const EDGE_V: f64 = 0.2;
/// The pin has rested when it has not moved for this long.
const REST_PS: u64 = orbe_engine::passive::DRIVE_REST_PS;
/// A cycle's high and low, or two cycles' highs and lows, agree within this fraction of the
/// cycle.
const AGREE: f64 = 0.005;
/// A cycle that counts is the latest one's note within this fraction of its period. An
/// interrupt that delays one edge can move a cycle that still counts by a little over `AGREE`.
/// A semitone is 6%.
const SAME_NOTE: f64 = 0.006;
/// Above this no one hears a pitch.
const AUDIBLE_HZ: f64 = 20_000.0;
/// Edges kept per pin, 16 ms of a 4 kHz note, where the latest cycle that counts is found.
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
    /// The coil voltage at each probe, and its average since the probe before, back to the
    /// longest cycle a note can have.
    coil: VecDeque<(u64, f64, f64)>,
    /// The time and the coil voltage's integral at the last probe.
    area: Option<(u64, f64)>,
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
                area: None,
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

    /// The coil voltage at `now_ps`, and its average since the last probe, from its integral.
    /// The average holds a pulse narrower than the time between probes.
    pub(super) fn note(&mut self, probe: &Probe, now_ps: u64) {
        for meter in &mut self.meters {
            let volts = probe.get(&meter.column).expect("a checked column").get();
            let slot = probe
                .refs()
                .iter()
                .position(|r| *r == meter.column)
                .expect("a checked column");
            let integral = probe.integrals()[slot];
            let mean = match meter.area {
                Some((at, before)) if at < now_ps => (integral - before) / (now_ps - at) as f64,
                _ => volts,
            };
            meter.area = Some((now_ps, integral));
            if meter.coil.back().is_some_and(|&(at, _, _)| at >= now_ps) {
                meter.coil.pop_back();
            }
            meter.coil.push_back((now_ps, volts, mean));
            // One sample older than the longest cycle stays, as the level that cycle began at.
            let oldest = now_ps.saturating_sub(2 * REST_PS);
            while meter.coil.get(1).is_some_and(|&(at, _, _)| at <= oldest) {
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
        let latest = meter
            .tracks
            .iter()
            .filter_map(|track| Some((track, track.latest()?)))
            .max_by_key(|(_, (cycle, _))| cycle.to);
        let Some((track, (cycle, period))) = latest else {
            return 0.0;
        };
        let hz = 1e12 / period as f64;
        let heard =
            !track.rested(now_ps) && meter.swing(cycle.from, now_ps) >= EDGE_V && hz <= AUDIBLE_HZ;
        if heard { hz } else { 0.0 }
    }
}

/// One full cycle of a pin: a rise, the fall after it, and the next rise.
#[derive(Clone, Copy, Debug)]
struct Cycle {
    from: u64,
    fall: u64,
    to: u64,
}

impl Cycle {
    fn high(&self) -> u64 {
        self.fall - self.from
    }

    fn low(&self) -> u64 {
        self.to - self.fall
    }

    fn period(&self) -> u64 {
        self.to - self.from
    }

    fn agree(&self, a: u64, b: u64) -> bool {
        a.abs_diff(b) as f64 <= AGREE * (self.to - self.from) as f64
    }

    /// Whether the cycle counts, given the cycle just before it.
    fn counts(&self, before: Option<&Cycle>) -> bool {
        self.agree(self.high(), self.low())
            || before.is_some_and(|before| {
                self.agree(self.high(), before.high()) && self.agree(self.low(), before.low())
            })
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

    /// Whether the pin has not moved for longer than a rest.
    fn rested(&self, now_ps: u64) -> bool {
        self.flips
            .back()
            .is_none_or(|last| now_ps.saturating_sub(last.at) > REST_PS)
    }

    /// The latest full cycle that counts since the pin last rested, and the median period of
    /// the cycles that count and are its note.
    fn latest(&self) -> Option<(Cycle, u64)> {
        let mut cycles = Vec::new();
        let mut rise: Option<u64> = None;
        let mut fall: Option<u64> = None;
        for flip in &self.flips {
            match (flip.high, rise, fall) {
                (true, Some(from), Some(down)) => {
                    cycles.push(Cycle {
                        from,
                        fall: down,
                        to: flip.at,
                    });
                    rise = Some(flip.at);
                    fall = None;
                }
                (true, _, _) => {
                    rise = Some(flip.at);
                    fall = None;
                }
                (false, Some(_), None) => fall = Some(flip.at),
                (false, _, _) => {}
            }
        }
        let counts: Vec<Cycle> = (0..cycles.len())
            .filter(|&k| cycles[k].counts(k.checked_sub(1).map(|b| &cycles[b])))
            .map(|k| cycles[k])
            .collect();
        let last = *counts.last()?;
        // The cycles that count and are the last one's note. An interrupt that delays an edge
        // can leave a cycle that counts a little long or short, and the median drops it.
        let mut note: Vec<u64> = counts
            .iter()
            .map(Cycle::period)
            .filter(|&period| {
                period.abs_diff(last.period()) as f64 <= SAME_NOTE * last.period() as f64
            })
            .collect();
        note.sort_unstable();
        Some((last, note[(note.len() - 1) / 2]))
    }
}

impl Meter {
    /// The coil's swing from `from_ps` to `to_ps`, counting the level it held at `from_ps` and
    /// the average between probes.
    fn swing(&self, from_ps: u64, to_ps: u64) -> f64 {
        let start = self.coil.partition_point(|&(at, _, _)| at <= from_ps);
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(at, volts, mean) in self.coil.range(start.saturating_sub(1)..) {
            if at > to_ps {
                break;
            }
            low = low.min(volts).min(mean);
            high = high.max(volts).max(mean);
        }
        if low > high { 0.0 } else { high - low }
    }
}
