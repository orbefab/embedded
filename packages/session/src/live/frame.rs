//! What a viewer sees of a session: one `scene` message per `open`, and the circuit at `now`,
//! which a `frame` message shows. A sample reads the probe, the levels and the switch edges only,
//! so it never lands a step, and a run gives the same result with a viewer or without one.

use std::collections::BTreeMap;

use orbe_engine::Status;
use orbe_engine::world::Quantity;
use serde::Serialize;
use serde_json::Value;

use super::{Live, Open, Progress};
use crate::reply::{Code, Diagnostic, Level};
use crate::target::{QUANTITIES, one_engine, rgb_reading, screen_reading};
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
    /// Each addressable LED part, by its reading `"of": "rgb"`.
    #[cfg_attr(test, schemars(with = "Vec<crate::Ref>"))]
    pub pixels: Vec<String>,
    /// Each screen part, by its reading `"of": "screen"`. Absent when the scene has none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, schemars(with = "Vec<crate::Ref>"))]
    pub screens: Vec<String>,
}

/// The channels with what reads each: a logic signal, a probe column, a switch's engine name,
/// an LED chain's engine name.
#[derive(Default)]
pub(super) struct Bound {
    names: Channels,
    pins: Vec<usize>,
    leds: Vec<usize>,
    switches: Vec<String>,
    pixels: Vec<String>,
    screens: Vec<String>,
    /// Each live param, as `@pot1.position`, with its engine part and the param.
    params: Vec<(String, String, orbe_scene::MapParam)>,
}

/// A world quantity's name: `temperature`, `humidity` or `light`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorldName {
    Temperature,
    Humidity,
    Light,
}

impl From<Quantity> for WorldName {
    fn from(quantity: Quantity) -> Self {
        match quantity {
            Quantity::Temperature => Self::Temperature,
            Quantity::Humidity => Self::Humidity,
            Quantity::Light => Self::Light,
        }
    }
}

/// How a page's thumb walks a quantity's range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Scale {
    /// Even steps of `step` from `min` to `max`.
    Linear { step: f64 },
    /// Round values: 0, then 1, 2, 3 and 5 times each power of ten, up to `max`.
    Log,
}

/// A world quantity the scene declares, as a page shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct WorldQuantity {
    pub(crate) name: WorldName,
    /// The unit of `min`, `max`, `start` and a frame's values, as the CLI writes it, such as `lx`.
    pub(crate) unit: &'static str,
    pub(crate) min: f64,
    pub(crate) max: f64,
    /// The value the scene gives it at t = 0.
    pub(crate) start: f64,
    pub(crate) scale: Scale,
}

impl WorldQuantity {
    fn new(quantity: Quantity, start: f64) -> Self {
        let row = quantity.row();
        let scale = match quantity {
            Quantity::Temperature => Scale::Linear { step: 0.5 },
            Quantity::Humidity => Scale::Linear { step: 1.0 },
            Quantity::Light => Scale::Log,
        };
        WorldQuantity {
            name: quantity.into(),
            unit: row.unit,
            min: row.min,
            max: row.max,
            start,
            scale,
        }
    }
}

/// A part's live param, as a page shows it: a slider, and the joint the param poses.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct LiveParam {
    /// The param, such as `@pot1.position`, as `orbe set` names it.
    #[cfg_attr(test, schemars(with = "crate::Ref"))]
    pub(crate) r#ref: String,
    /// The unit of `above`, `below`, `start` and a frame's values, as the part file writes it,
    /// such as `1`.
    pub(crate) unit: String,
    /// The value stays above this, the param's `min_exclusive`.
    pub(crate) above: f64,
    /// The value stays below this, the param's `max_exclusive`.
    pub(crate) below: f64,
    /// The value the scene gives it at t = 0.
    pub(crate) start: f64,
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
    /// Each addressable LED part's first LED, the red, green and blue it last latched.
    pub pixels: Vec<[u8; 3]>,
    /// What each screen's glass shows.
    pub screens: Vec<Glass>,
    /// Each declared world quantity's value from `t_ps` on, or from where its last `set` lands
    /// when that is later, in table order. Empty when the scene declares none.
    pub world: Vec<f64>,
    /// Each live param's value, as `world` holds a quantity's, in the scene message's order.
    pub params: Vec<f64>,
    pub diagnostics: usize,
    /// The fault's diagnostic as `orbe snapshot` prints it, once the session faulted.
    pub fault: Option<String>,
}

/// A screen's glass: `width` by `height` pixels of red, green, blue and alpha, a byte each, rows
/// top to bottom and each row left to right.
#[derive(Clone, Debug, PartialEq)]
pub struct Glass {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

/// `play`'s pace for a frame: simulated time over wall time as the session achieved it.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    pub ratio: f64,
}

impl Live {
    /// The session's serial text: every USART's bytes in the order their stop bits ended,
    /// decoded as UTF-8, the same text `serial contains` searches. Empty when no scene is
    /// open. The session does not record which USART a byte came from.
    pub fn serial(&self) -> &str {
        self.open
            .as_ref()
            .map(|open| open.serial.text())
            .unwrap_or("")
    }

    /// The `scene` message for the open scene.
    pub fn scene(&self) -> Option<Value> {
        let open = self.open.as_ref()?;
        let world: Vec<WorldQuantity> = open
            .engine
            .world()
            .declared()
            .map(|(quantity, schedule)| WorldQuantity::new(quantity, schedule.initial().si.get()))
            .collect();
        let params: Vec<LiveParam> = open
            .channels
            .params
            .iter()
            .map(|(r, engine, spec)| LiveParam {
                r#ref: r.clone(),
                unit: spec.unit.clone(),
                above: spec.above.expect("a live param has both bounds"),
                below: spec.below.expect("a live param has both bounds"),
                start: open
                    .engine
                    .part_level(&super::engine_name(engine))
                    .expect("a live param's part has a level")
                    .schedule
                    .initial()
                    .si
                    .get(),
            })
            .collect();
        Some(viewer::scene_message(
            &self.name,
            &open.channels.names,
            &world,
            &params,
            &open.view,
        ))
    }

    pub fn sample(&self) -> Option<Sample> {
        self.open.as_ref().map(Open::sample)
    }
}

impl Progress<'_> {
    /// The session's serial text, as [`Live::serial`] reads it.
    pub fn serial(&self) -> &str {
        self.open.serial.text()
    }

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
    /// part data says glows, every pressable switch, and every addressable LED, each with what
    /// reads it.
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
            let chain =
                rgb_reading(part).and_then(|_| one_engine(path, part, Code::NoReading).ok());
            if let Some(engine) = chain {
                bound.names.pixels.push(format!("@{path}"));
                bound.pixels.push(engine);
            }
            let screen =
                screen_reading(part).and_then(|_| one_engine(path, part, Code::NoReading).ok());
            if let Some(engine) = screen {
                bound.names.screens.push(format!("@{path}"));
                bound.screens.push(engine);
            }
        }
        for (path, part) in &self.refs.parts {
            let Ok(engine) = one_engine(path, part, Code::NotSettable) else {
                continue;
            };
            let settable = self
                .engine
                .part_level(&super::engine_name(&engine))
                .is_some();
            for (param, spec) in &part.params {
                if spec.live && settable && spec.above.is_some() && spec.below.is_some() {
                    bound
                        .params
                        .push((format!("@{path}.{param}"), engine.clone(), spec.clone()));
                }
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
            pixels: bound
                .pixels
                .iter()
                .map(|engine| {
                    self.colors(engine)
                        .leds
                        .first()
                        .copied()
                        .unwrap_or_default()
                })
                .collect(),
            screens: bound
                .screens
                .iter()
                .map(|engine| {
                    let display = self
                        .engine
                        .display(&super::engine_name(engine))
                        .expect("a part with a reading of screen is a display");
                    let panel = display.panel();
                    Glass {
                        width: panel.width,
                        height: panel.height,
                        rgba: display.pixels(),
                    }
                })
                .collect(),
            world: self
                .engine
                .world()
                .declared()
                .map(|(quantity, schedule)| {
                    let now = self.engine.now();
                    let from = self
                        .world_sets
                        .get(&quantity)
                        .map_or(now, |&at| at.max(now));
                    schedule.after(from).si.get()
                })
                .collect(),
            params: bound
                .params
                .iter()
                .map(|(_, engine, _)| {
                    let now = self.engine.now();
                    let from = self.param_sets.get(engine).map_or(now, |&at| at.max(now));
                    self.engine
                        .part_level(&super::engine_name(engine))
                        .expect("a live param's part has a level")
                        .schedule
                        .after(from)
                        .si
                        .get()
                })
                .collect(),
            diagnostics: self.findings(&probe).len(),
            fault: self
                .engine
                .fault()
                .map(|fault| Diagnostic::new(fault).text()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_names_are_the_table() {
        let wire: Vec<String> = Quantity::ALL
            .into_iter()
            .map(|quantity| {
                serde_json::to_value(WorldName::from(quantity))
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let names: Vec<&str> = Quantity::ALL
            .iter()
            .map(|quantity| quantity.name())
            .collect();
        assert_eq!(wire, names);
    }
}
