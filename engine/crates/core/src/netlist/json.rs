use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::{
    Analysis, Junction, Model, NetId, Netlist, Part, Resistor, Source, Value, VoltageSource,
    junction_scale,
};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::finite::Finite;
use crate::name::{Name, NameError, Ref};

#[derive(Deserialize)]
struct VersionProbe {
    version: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNetlist {
    version: u64,
    seed: u64,
    analysis: WireAnalysis,
    ground: String,
    nets: Vec<String>,
    parts: Vec<WirePart>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum WireAnalysis {
    Op(WireOp),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireOp {}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WirePart {
    Resistor {
        name: String,
        pins: PinMap,
        resistance: WireValue,
        ratings: WireResistorRatings,
    },
    VoltageSource {
        name: String,
        pins: PinMap,
        voltage: WireValue,
    },
    Diode(WireJunction),
    Led(WireJunction),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireJunction {
    name: String,
    pins: PinMap,
    saturation_current: WireValue,
    emission_coefficient: WireValue,
    series_resistance: WireValue,
    ratings: WireJunctionRatings,
}

// `Option::deserialize` makes each key required while its value may be `null`.
// Plain `Option` would read a missing key as `None`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResistorRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    power: Option<WireValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireJunctionRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    current: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    power: Option<WireValue>,
}

/// Pin objects reject a repeated key. `BTreeMap` would keep the last one.
struct PinMap(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for PinMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(PinMapVisitor)
    }
}

struct PinMapVisitor;

impl<'de> Visitor<'de> for PinMapVisitor {
    type Value = PinMap;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a map from pin name to net name")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut pins = BTreeMap::new();
        while let Some((key, net)) = map.next_entry::<String, String>()? {
            if pins.insert(key.clone(), net).is_some() {
                return Err(de::Error::custom(format!("duplicate pin `{key}`")));
            }
        }
        Ok(PinMap(pins))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireValue {
    value: Finite,
    unit: String,
    source: WireSource,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum WireSource {
    User,
    ClosedForm {
        note: String,
    },
    Datasheet {
        doc: String,
        at: String,
        typical: bool,
    },
}

pub(super) fn parse(json: &[u8]) -> Result<Netlist, Vec<Diagnostic>> {
    if let Some(found) = probe_version(json)
        && found != super::FORMAT_VERSION
    {
        return Err(vec![Diagnostic::NetlistVersion {
            found,
            supported: super::FORMAT_VERSION,
        }]);
    }
    let mut de = serde_json::Deserializer::from_slice(json);
    let wire = match serde_path_to_error::deserialize(&mut de) {
        Ok(wire) => wire,
        Err(err) => return Err(vec![syntax(err)]),
    };
    if let Err(err) = de.end() {
        return Err(vec![json_syntax(&err)]);
    }
    resolve(wire)
}

fn probe_version(json: &[u8]) -> Option<u64> {
    let mut de = serde_json::Deserializer::from_slice(json);
    serde_path_to_error::deserialize::<_, VersionProbe>(&mut de)
        .ok()
        .map(|probe| probe.version)
}

fn syntax(err: serde_path_to_error::Error<serde_json::Error>) -> Diagnostic {
    let path = err.path().to_string();
    let inner = err.into_inner();
    netlist_syntax(path, &inner)
}

fn json_syntax(err: &serde_json::Error) -> Diagnostic {
    netlist_syntax(String::new(), err)
}

fn netlist_syntax(path: String, err: &serde_json::Error) -> Diagnostic {
    Diagnostic::NetlistSyntax {
        path,
        line: err.line(),
        column: err.column(),
        detail: err.to_string(),
    }
}

fn resolve(wire: WireNetlist) -> Result<Netlist, Vec<Diagnostic>> {
    let mut out = Vec::new();
    let nets = resolve_nets(&wire.nets, &wire.ground, &mut out);
    let net_ids: BTreeMap<String, NetId> = nets
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str().to_string(), NetId(i as u32)))
        .collect();

    let mut seen_parts = BTreeSet::new();
    let mut parts = Vec::new();
    for (index, part) in wire.parts.into_iter().enumerate() {
        if let Some(part) = resolve_part(index, part, &net_ids, &mut seen_parts, &mut out) {
            parts.push(part);
        }
    }

    if !out.is_empty() {
        return Err(out);
    }
    debug_assert_eq!(wire.version, super::FORMAT_VERSION);
    parts.sort_by(|a, b| a.name.cmp(&b.name));
    let analysis = match wire.analysis {
        WireAnalysis::Op(_) => Analysis::Op,
    };
    Ok(Netlist {
        seed: wire.seed,
        analysis,
        nets,
        parts,
    })
}

fn resolve_nets(raw_nets: &[String], ground: &str, out: &mut Vec<Diagnostic>) -> Vec<Name> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    for (i, raw) in raw_nets.iter().enumerate() {
        match Name::new(raw) {
            Ok(name) => {
                if !seen.insert(name.as_str().to_string()) {
                    out.push(Diagnostic::NetlistDuplicate {
                        name: Ref::Net(name),
                    });
                } else {
                    valid.push(name);
                }
            }
            Err(problem) => out.push(Diagnostic::NetlistName {
                path: format!("nets[{i}]"),
                name: raw.clone(),
                problem,
            }),
        }
    }

    let ground = match Name::new(ground) {
        Ok(name) => {
            if !valid.iter().any(|net| net == &name) {
                out.push(Diagnostic::NetlistGround {
                    net: ground.to_string(),
                });
            }
            Some(name)
        }
        Err(problem) => {
            out.push(Diagnostic::NetlistName {
                path: "ground".to_string(),
                name: ground.to_string(),
                problem,
            });
            None
        }
    };

    canonical_nets(valid, ground.as_ref())
}

fn canonical_nets(mut valid: Vec<Name>, ground: Option<&Name>) -> Vec<Name> {
    if let Some(ground) = ground
        && let Some(pos) = valid.iter().position(|net| net == ground)
    {
        valid.remove(pos);
        valid.sort();
        let mut nets = vec![ground.clone()];
        nets.extend(valid);
        return nets;
    }
    valid.sort();
    valid
}

fn resolve_part(
    index: usize,
    part: WirePart,
    nets: &BTreeMap<String, NetId>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Diagnostic>,
) -> Option<Part> {
    let raw_name = match &part {
        WirePart::Resistor { name, .. } | WirePart::VoltageSource { name, .. } => name,
        WirePart::Diode(junction) | WirePart::Led(junction) => &junction.name,
    };
    let path = format!("parts[{index}].name");
    let name = match Name::new(raw_name) {
        Ok(name) => name,
        Err(problem) => {
            out.push(Diagnostic::NetlistName {
                path,
                name: raw_name.clone(),
                problem,
            });
            return None;
        }
    };
    let reserved = name.as_str() == "net";
    if reserved {
        out.push(Diagnostic::NetlistName {
            path,
            name: raw_name.clone(),
            problem: NameError::Reserved,
        });
    }
    let duplicate = !seen.insert(name.as_str().to_string());
    if duplicate {
        out.push(Diagnostic::NetlistDuplicate {
            name: Ref::Part(name.clone()),
        });
    }
    let part_ref = Ref::Part(name.clone());
    let model = match part {
        WirePart::Resistor {
            pins,
            resistance,
            ratings,
            ..
        } => {
            let pins = resolve_pins(&name, Resistor::PINS, &pins.0, nets, out);
            let resistance = resolve_value(&part_ref, "resistance", "ohm", true, resistance, out);
            let max_power = resolve_rating(&part_ref, "ratings.power", "W", ratings.power, out);
            match (pins, resistance, max_power) {
                (Some(pins), Some(resistance), Some(max_power)) => {
                    Some(Model::Resistor(Resistor {
                        pins,
                        resistance,
                        max_power,
                    }))
                }
                _ => None,
            }
        }
        WirePart::VoltageSource { pins, voltage, .. } => {
            let pins = resolve_pins(&name, VoltageSource::PINS, &pins.0, nets, out);
            let voltage = resolve_value(&part_ref, "voltage", "V", false, voltage, out);
            match (pins, voltage) {
                (Some(pins), Some(voltage)) => {
                    Some(Model::VoltageSource(VoltageSource { pins, voltage }))
                }
                _ => None,
            }
        }
        WirePart::Diode(junction) => {
            resolve_junction(&name, &part_ref, junction, nets, out).map(Model::Diode)
        }
        WirePart::Led(junction) => {
            resolve_junction(&name, &part_ref, junction, nets, out).map(Model::Led)
        }
    };
    if duplicate || reserved {
        None
    } else {
        model.map(|model| Part { name, model })
    }
}

fn resolve_junction(
    name: &Name,
    part_ref: &Ref,
    wire: WireJunction,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Junction> {
    let pins = resolve_pins(name, Junction::PINS, &wire.pins.0, nets, out);
    let is = resolve_value(
        part_ref,
        "saturation_current",
        "A",
        true,
        wire.saturation_current,
        out,
    );
    let n = resolve_value(
        part_ref,
        "emission_coefficient",
        "1",
        true,
        wire.emission_coefficient,
        out,
    );
    let rs = resolve_value(
        part_ref,
        "series_resistance",
        "ohm",
        true,
        wire.series_resistance,
        out,
    );
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    if let (Some(is), Some(n)) = (&is, &n)
        && let Err(fault) = junction_scale(n.si.get(), is.si.get())
    {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: fault.field(),
            value: fault.value(n.si.get(), is.si.get()),
            rule: ValueRule::FiniteJunction,
        });
        return None;
    }
    Some(Junction {
        pins: pins?,
        saturation_current: is?,
        emission_coefficient: n?,
        series_resistance: rs?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

fn check_positive(number: f64) -> Option<ValueRule> {
    if number <= 0.0 {
        Some(ValueRule::Positive)
    } else if !(1.0 / number).is_finite() {
        Some(ValueRule::FiniteReciprocal)
    } else {
        None
    }
}

fn resolve_pins<const N: usize>(
    part: &Name,
    names: [&'static str; N],
    pins: &BTreeMap<String, String>,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<[NetId; N]> {
    let mut ids = [NetId::GROUND; N];
    let mut missing = Vec::new();
    let mut resolved = true;
    for (i, pin) in names.iter().enumerate() {
        match pins.get(*pin) {
            None => missing.push(*pin),
            Some(net) => match nets.get(net) {
                Some(id) => ids[i] = *id,
                None => {
                    resolved = false;
                    out.push(Diagnostic::NetlistUnknownNet {
                        pin: Ref::Pin(part.clone(), Name::new(pin).expect("kind pin name")),
                        net: net.clone(),
                    });
                }
            },
        }
    }
    let expected: BTreeSet<&str> = names.iter().copied().collect();
    let mut unexpected = Vec::new();
    for (key, net) in pins {
        if expected.contains(key.as_str()) {
            continue;
        }
        unexpected.push(key.clone());
        if let Ok(pin_name) = Name::new(key)
            && !nets.contains_key(net)
        {
            out.push(Diagnostic::NetlistUnknownNet {
                pin: Ref::Pin(part.clone(), pin_name),
                net: net.clone(),
            });
        }
    }
    if !missing.is_empty() || !unexpected.is_empty() {
        out.push(Diagnostic::NetlistPins {
            part: Ref::Part(part.clone()),
            missing,
            unexpected,
        });
        resolved = false;
    }
    resolved.then_some(ids)
}

/// `None` when the rating is invalid. `Some(None)` is an explicit `null`.
fn resolve_rating(
    part: &Ref,
    field: &'static str,
    expected: &'static str,
    value: Option<WireValue>,
    out: &mut Vec<Diagnostic>,
) -> Option<Option<Value>> {
    match value {
        None => Some(None),
        Some(value) => resolve_value(part, field, expected, true, value, out).map(Some),
    }
}

fn resolve_value(
    part: &Ref,
    field: &'static str,
    expected: &'static str,
    positive: bool,
    value: WireValue,
    out: &mut Vec<Diagnostic>,
) -> Option<Value> {
    let mut ok = true;
    if value.unit != expected {
        out.push(Diagnostic::NetlistUnit {
            part: part.clone(),
            field,
            expected,
            found: value.unit,
        });
        ok = false;
    }
    let number = value.value.get();
    if positive && let Some(broken) = check_positive(number) {
        ok = false;
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field,
            value: number,
            rule: broken,
        });
    }
    if !ok {
        return None;
    }
    Some(Value {
        si: value.value,
        source: match value.source {
            WireSource::User => Source::User,
            WireSource::ClosedForm { note } => Source::ClosedForm { note },
            WireSource::Datasheet { doc, at, typical } => Source::Datasheet { doc, at, typical },
        },
    })
}
