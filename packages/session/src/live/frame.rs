//! What a viewer sees of a session: one `scene` message per `open`, and the circuit at `now`,
//! which a `frame` message shows. A sample reads the probe, the levels and the switch edges only,
//! so it never lands a step, and a run gives the same result with a viewer or without one.

use std::collections::BTreeMap;

use orbe_circuit::Status;
use serde::Serialize;
use serde_json::Value;

use super::{Live, Open, Progress};
use crate::reply::{Code, Diagnostic, Level};
use crate::target::{QUANTITIES, one_engine};
use crate::viewer;

/// The refs each frame array follows, in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Channels {
    #[cfg_attr(test, schemars(with = "Vec<crate::Ref>"))]
    pub pins: Vec<String>,
    #[cfg_attr(test, schemars(with = "Vec<crate::Ref>"))]
    pub leds: Vec<String>,
    #[cfg_attr(test, schemars(with = "Vec<crate::Ref>"))]
    pub switches: Vec<String>,
}

/// The channels with what reads each: a logic signal, a probe column, a switch's engine name.
#[derive(Default)]
pub(super) struct Bound {
    names: Channels,
    pins: Vec<usize>,
    leds: Vec<usize>,
    switches: Vec<String>,
}

/// A frame's `state`: the session's, or `stepping` for a frame read while a `step` runs, which
/// no reply says. A frame needs an open scene, so it is never `closed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum FrameState {
    Paused,
    Playing,
    Stopped,
    Faulted,
    Stepping,
}

/// The circuit at one time, as a frame shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub t_ps: u64,
    pub state: FrameState,
    pub pins: Vec<u8>,
    /// Each LED's current in amperes, and its integral from t = 0 in ampere picoseconds.
    pub leds: Vec<(f64, f64)>,
    pub switches: Vec<u8>,
    pub diagnostics: usize,
    /// The fault's diagnostic as `orbe snapshot` prints it, once the session faulted.
    pub fault: Option<String>,
}

/// `play`'s pace for a frame: the speed it asked for, and simulated time over wall time as the
/// session achieved it.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    pub speed: f64,
    pub ratio: f64,
}

impl Live {
    /// The `scene` message for the open scene.
    pub fn scene(&self) -> Option<Value> {
        let open = self.open.as_ref()?;
        Some(viewer::scene_message(
            &self.name,
            &open.channels.names,
            &open.view,
        ))
    }

    pub fn sample(&self) -> Option<Sample> {
        self.open.as_ref().map(Open::sample)
    }
}

impl Progress<'_> {
    /// A sample taken inside a step. It reads `stepping` unless the slice it follows stopped or
    /// faulted the session, which it reports as is.
    pub fn sample(&self) -> Sample {
        let sample = self.open.sample();
        match sample.state {
            FrameState::Stopped | FrameState::Faulted => sample,
            _ => Sample {
                state: FrameState::Stepping,
                ..sample
            },
        }
    }
}

/// Each switch's initial contact and its scheduled events, by engine name, from the netlist.
pub(super) fn contacts(json: &str) -> BTreeMap<String, (bool, Vec<(u64, bool)>)> {
    let netlist: Value = serde_json::from_str(json).expect("the reader writes JSON");
    let closed = |state: &Value| state.as_str() == Some("closed");
    netlist["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["kind"] == "switch")
        .filter_map(|part| {
            let events = part["events"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| Some((e["t_ps"].as_u64()?, closed(&e["state"]))))
                .collect();
            Some((
                part["name"].as_str()?.to_string(),
                (closed(&part["initial"]), events),
            ))
        })
        .collect()
}

impl Open {
    /// The scene's board pins that `snapshot` shows and that a chip pin drives, every LED the
    /// part data says glows, and every pressable switch, each with what reads it.
    pub(super) fn bind_channels(&self) -> Bound {
        let mut bound = Bound::default();
        let wired = self.wired_nets();
        for top in &self.refs.top {
            let part = &self.refs.parts[top];
            if !part.engine.is_empty() || self.holed(top) {
                continue;
            }
            for (pin, net) in &part.pins {
                let signal = net
                    .as_deref()
                    .filter(|net| wired.contains(*net))
                    .and_then(|net| self.signals.get(net));
                if let Some(&signal) = signal {
                    bound.names.pins.push(format!("@{top}.{pin}"));
                    bound.pins.push(signal);
                }
            }
        }
        let probe = self.engine.probe();
        for (path, part) in &self.refs.parts {
            let Some(data) = self.view.parts.get(&part.ty) else {
                continue;
            };
            let column = data
                .readings
                .brightness
                .as_ref()
                .and_then(|_| part.readings.get("brightness"))
                .and_then(|reading| QUANTITIES.into_iter().find(|q| q.as_str() == reading.of))
                .and_then(|q| {
                    let engine = one_engine(path, part, Code::NoReading).ok()?;
                    let column = self.column(path, &engine, q).ok()?;
                    probe.refs().iter().position(|r| *r == column)
                });
            if let Some(column) = column {
                bound.names.leds.push(format!("@{path}"));
                bound.leds.push(column);
            }
            let switch = data
                .controls
                .press
                .as_ref()
                .and_then(|_| one_engine(path, part, Code::NotSettable).ok())
                .filter(|engine| self.contacts.contains_key(engine));
            if let Some(engine) = switch {
                bound.names.switches.push(format!("@{path}"));
                bound.switches.push(engine);
            }
        }
        bound
    }

    /// A switch's contact at `now`: the state after every edge strictly before it.
    fn closed(&self, engine: &str) -> bool {
        let now = self.engine.now().as_ps();
        let (initial, edges) = &self.contacts[engine];
        edges
            .iter()
            .rev()
            .find(|(at, _)| *at < now)
            .map_or(*initial, |(_, closed)| *closed)
    }

    fn sample(&self) -> Sample {
        let probe = self.engine.probe();
        let bound = &self.channels;
        Sample {
            t_ps: self.engine.now().as_ps(),
            state: match self.engine.status() {
                Status::Running if self.playing => FrameState::Playing,
                Status::Running => FrameState::Paused,
                Status::Stopped => FrameState::Stopped,
                Status::Faulted => FrameState::Faulted,
            },
            pins: bound
                .pins
                .iter()
                .map(|&k| u8::from(self.level_before_now(k) == Level::High))
                .collect(),
            leds: bound
                .leds
                .iter()
                .map(|&k| (probe.values()[k].get(), probe.integrals()[k]))
                .collect(),
            switches: bound
                .switches
                .iter()
                .map(|engine| u8::from(self.closed(engine)))
                .collect(),
            diagnostics: self.findings(&probe).len(),
            fault: self
                .engine
                .fault()
                .map(|fault| Diagnostic::new(fault).text()),
        }
    }
}
