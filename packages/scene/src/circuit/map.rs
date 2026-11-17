//! How the scene's dotted names reach the engine's flat ones, for a live session to resolve a
//! ref such as `@nano.D13` without guessing at names.

use std::collections::BTreeMap;

use super::Circuit;
use crate::diagnostic::Findings;
use crate::syntax;

/// Every instance the reader built, by its dotted path.
#[derive(Clone, Debug, Default)]
pub struct RefMap {
    pub parts: BTreeMap<String, MapPart>,
    /// The scene's own instances, in the scene's order.
    pub top: Vec<String>,
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
