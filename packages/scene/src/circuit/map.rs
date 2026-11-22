//! How the scene's dotted names reach the engine's flat ones, for a live session to resolve a
//! ref such as `@nano.D13` without guessing at names.

use std::collections::{BTreeMap, BTreeSet};

use super::Circuit;
use crate::diagnostic::Findings;
use crate::model::ParamKind;
use crate::syntax;

/// Every instance the reader built, by its dotted path.
#[derive(Clone, Debug, Default)]
pub struct RefMap {
    pub parts: BTreeMap<String, MapPart>,
    /// The scene's own instances, in the scene's order.
    pub top: Vec<String>,
    /// Nets `net.floating` leaves out of lint and of a live session. The engine still reports
    /// them. A part lists those pins in `open`, and the net stays out while no scene wire
    /// reaches it and every other pin on it, other than a hole, belongs to that part or a part
    /// inside it. A hole that only seats that pin, with nothing else in its strip, does not
    /// reach the net. A scene wire, another part's pin, or a wire to a hole on the net still
    /// warns.
    pub quiet_floating: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MapPart {
    /// The catalog id, or the path the file names it by.
    pub ty: String,
    pub children: Vec<String>,
    /// Each pin with the engine net it sits on. `None` for a pin on no simulated net.
    pub pins: BTreeMap<String, Option<String>>,
    /// The engine parts this instance's own model writes.
    pub engine: Vec<EnginePart>,
    /// The part file's `readings`, such as an LED's `lit`.
    pub readings: BTreeMap<String, Reading>,
    /// Hole pin names, when this part is a board other parts plug into.
    pub holes: Vec<String>,
    /// Each param its part file declares, by name. A part with children hands its own params
    /// to them, so it lists none.
    pub params: BTreeMap<String, MapParam>,
}

/// A param as a live session sees it: whether `set` changes it while the session runs, and
/// the range a value must fall in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapParam {
    /// A number param's unit, as the part file writes it, such as `1` or `ohm`. Empty for a
    /// choice or a file.
    pub unit: String,
    /// The value is above this, from `min_exclusive`.
    pub above: Option<f64>,
    /// The value is below this, from `max_exclusive`.
    pub below: Option<f64>,
    /// `"live": true`: `set` changes it while the session runs, as a person turns a knob.
    pub live: bool,
}

#[derive(Clone, Debug)]
pub struct EnginePart {
    /// The flat name, such as `nano__mcu`.
    pub name: String,
    pub kind: String,
    pub pins: BTreeMap<String, String>,
}

/// A reading a part states, such as `lit` while `i` is above 2 mA.
#[derive(Clone, Debug)]
pub struct Reading {
    /// The engine quantity it reads, such as `i`.
    pub of: String,
    pub above: Option<Amount>,
}

#[derive(Clone, Debug)]
pub struct Amount {
    pub value: f64,
    pub unit: String,
}

impl Circuit<'_> {
    pub(super) fn map(&mut self, nets: &BTreeMap<usize, String>) -> RefMap {
        let mut map = RefMap::default();
        for inst in 1..self.insts.len() {
            let path = self.insts[inst].dotted();
            if self.insts[inst].path.len() == 1 {
                map.top.push(path.clone());
            }
            let mut part = MapPart {
                children: self.insts[inst].children.keys().cloned().collect(),
                ..MapPart::default()
            };
            if let Some(file) = self.part(inst) {
                part.ty.clone_from(&file.label);
                part.readings = self.readings(file.file);
                if let Some(board) = &file.holes {
                    part.holes = board.strips.iter().flatten().cloned().collect();
                }
                for decl in &file.params {
                    let param = match &decl.kind {
                        ParamKind::Number { unit, range } => MapParam {
                            unit: unit.as_str().to_string(),
                            above: range.above,
                            below: range.below,
                            live: decl.live.is_some(),
                        },
                        ParamKind::Choice(_) | ParamKind::File(_) => MapParam::default(),
                        ParamKind::Through => continue,
                    };
                    part.params.insert(decl.name.clone(), param);
                }
                for pin in &file.pins {
                    let net = self.pin_net(inst, &pin.name, nets);
                    part.pins.insert(pin.name.clone(), net);
                }
                if self.insts[inst].dropped.is_none() && self.built.contains_key(&inst) {
                    for body in &file.bodies {
                        let mut name = self.insts[inst].flat();
                        if let Some(body) = &body.name {
                            name = format!("{name}__{body}");
                        }
                        let pins = body
                            .pins
                            .iter()
                            .filter_map(|(engine, pin)| {
                                Some((engine.to_string(), self.pin_net(inst, pin, nets)?))
                            })
                            .collect();
                        part.engine.push(EnginePart {
                            name,
                            kind: body.kind.clone(),
                            pins,
                        });
                    }
                }
            }
            map.parts.insert(path, part);
        }
        map
    }

    fn pin_net(
        &mut self,
        inst: usize,
        pin: &str,
        nets: &BTreeMap<usize, String>,
    ) -> Option<String> {
        if self.insts[inst].dropped.is_some() {
            return None;
        }
        let key = *self.keys.get(&(inst, pin.to_string()))?;
        let root = self.find(key);
        nets.get(&root).cloned()
    }

    /// The part file's `readings`. The file already read once, so a second parse finds nothing
    /// new to report.
    fn readings(&self, file: crate::source::FileId) -> BTreeMap<String, Reading> {
        let mut quiet = Findings::default();
        let Some(tree) = syntax::parse(&self.project.sources, file, &mut quiet) else {
            return BTreeMap::new();
        };
        let Some(props) = tree.get("readings").and_then(|r| r.props()) else {
            return BTreeMap::new();
        };
        props
            .iter()
            .filter_map(|prop| {
                let of = prop.value.get("of")?.str()?.to_string();
                let above = prop.value.get("above").and_then(|a| {
                    Some(Amount {
                        value: a.get("value")?.number()?,
                        unit: a.get("unit")?.str()?.to_string(),
                    })
                });
                Some((prop.key.clone(), Reading { of, above }))
            })
            .collect()
    }
}
