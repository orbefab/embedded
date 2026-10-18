use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::{
    Analysis, Bounce, Capacitor, Chip, Contact, Discharge, Edge, IntValue, Junction, Model, Ne555,
    NetId, Netlist, OutputStage, Part, Resistor, Schedule, Source, Switch, Transient, Value,
    VoltageSource, bounce, junction_scale,
};
use crate::diagnostic::{AnalysisRule, Diagnostic, ScheduleRule, ValueRule};
use crate::finite::Finite;
use crate::name::{Name, NameError, Ref};
use crate::time::Time;

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
    Transient(WireTransient),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireOp {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTransient {
    stop_ps: u64,
    max_step_ps: u64,
}

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
    Capacitor {
        name: String,
        pins: PinMap,
        capacitance: WireValue,
        // Required but nullable, like the ratings below.
        #[serde(deserialize_with = "Option::deserialize")]
        initial: Option<WireValue>,
    },
    Switch(WireSwitch),
    Ne555(Box<WireNe555>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNe555 {
    name: String,
    pins: PinMap,
    ladder_resistance: WireValue,
    reset_threshold: WireValue,
    discharge: WireDischarge,
    supply_resistance: WireValue,
    output: WireOutputStage,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDischarge {
    on_resistance: WireValue,
    off_resistance: WireValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireOutputStage {
    high_drop: WireValue,
    high_resistance: WireValue,
    low_drop: WireValue,
    low_resistance: WireValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSwitch {
    name: String,
    pins: PinMap,
    on_resistance: WireValue,
    off_resistance: WireValue,
    initial: Contact,
    events: Vec<WireEvent>,
    #[serde(deserialize_with = "Option::deserialize")]
    bounce: Option<WireBounce>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    t_ps: u64,
    state: Contact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBounce {
    window: WireInt<u64>,
    max_bounces: WireInt<u8>,
}

/// `value` must be a JSON integer in `T`'s range. Anything else is `netlist.syntax`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireInt<T> {
    value: T,
    unit: String,
    source: WireSource,
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

    let analysis = resolve_analysis(&wire.analysis, &mut out);
    let mut seen_parts = BTreeSet::new();
    let mut parts = Vec::new();
    for (index, part) in wire.parts.into_iter().enumerate() {
        if let Some(part) =
            resolve_part(wire.seed, index, part, &net_ids, &mut seen_parts, &mut out)
        {
            parts.push(part);
        }
    }

    if !out.is_empty() {
        return Err(out);
    }
    debug_assert_eq!(wire.version, super::FORMAT_VERSION);
    parts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Netlist {
        seed: wire.seed,
        analysis: analysis.expect("an analysis without diagnostics resolved"),
        nets,
        parts,
    })
}

fn resolve_analysis(wire: &WireAnalysis, out: &mut Vec<Diagnostic>) -> Option<Analysis> {
    let transient = match wire {
        WireAnalysis::Op(_) => return Some(Analysis::Op),
        WireAnalysis::Transient(transient) => transient,
    };
    let mut broken = Vec::new();
    if transient.stop_ps == 0 {
        broken.push(("stop_ps", transient.stop_ps, AnalysisRule::Positive));
    }
    if transient.max_step_ps < Transient::MIN_MAX_STEP_PS {
        broken.push((
            "max_step_ps",
            transient.max_step_ps,
            AnalysisRule::StepUnder1ns,
        ));
    }
    if transient.max_step_ps > transient.stop_ps {
        broken.push((
            "max_step_ps",
            transient.max_step_ps,
            AnalysisRule::StepOverStop,
        ));
    }
    let ok = broken.is_empty();
    out.extend(
        broken
            .into_iter()
            .map(|(field, value, rule)| Diagnostic::NetlistAnalysis { field, value, rule }),
    );
    ok.then_some(Analysis::Transient(Transient {
        stop: Time::from_ps(transient.stop_ps),
        max_step: transient.max_step_ps,
    }))
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
    seed: u64,
    index: usize,
    part: WirePart,
    nets: &BTreeMap<String, NetId>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Diagnostic>,
) -> Option<Part> {
    let raw_name = match &part {
        WirePart::Resistor { name, .. }
        | WirePart::VoltageSource { name, .. }
        | WirePart::Capacitor { name, .. } => name,
        WirePart::Diode(junction) | WirePart::Led(junction) => &junction.name,
        WirePart::Switch(switch) => &switch.name,
        WirePart::Ne555(chip) => &chip.name,
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
        WirePart::Capacitor {
            pins,
            capacitance,
            initial,
            ..
        } => {
            let pins = resolve_pins(&name, Capacitor::PINS, &pins.0, nets, out);
            let capacitance = resolve_value(&part_ref, "capacitance", "F", true, capacitance, out);
            let initial = match initial {
                None => Some(None),
                Some(initial) => {
                    resolve_value(&part_ref, "initial", "V", false, initial, out).map(Some)
                }
            };
            match (pins, capacitance, initial) {
                (Some(pins), Some(capacitance), Some(initial)) => {
                    Some(Model::Capacitor(Capacitor {
                        pins,
                        capacitance,
                        initial,
                    }))
                }
                _ => None,
            }
        }
        WirePart::Switch(switch) => {
            resolve_switch(seed, &name, &part_ref, switch, nets, out).map(Model::Switch)
        }
        WirePart::Ne555(chip) => resolve_ne555(&name, &part_ref, *chip, nets, out)
            .map(|chip| Model::Chip(Chip::Ne555(Box::new(chip)))),
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

fn resolve_ne555(
    name: &Name,
    part_ref: &Ref,
    wire: WireNe555,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Ne555> {
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    let drop = |field, value: WireValue, out: &mut Vec<Diagnostic>| {
        let volts = value.value.get();
        let value = resolve_value(part_ref, field, "V", false, value, out)?;
        if volts < 0.0 {
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field,
                value: volts,
                rule: ValueRule::NonNegative,
            });
            return None;
        }
        Some(value)
    };
    let pins = resolve_pins(name, Ne555::PINS, &wire.pins.0, nets, out);
    let ladder = ohm("ladder_resistance", wire.ladder_resistance, out);
    let reset = resolve_value(
        part_ref,
        "reset_threshold",
        "V",
        true,
        wire.reset_threshold,
        out,
    );
    let on = ohm("discharge.on_resistance", wire.discharge.on_resistance, out);
    let off = ohm(
        "discharge.off_resistance",
        wire.discharge.off_resistance,
        out,
    );
    let supply = ohm("supply_resistance", wire.supply_resistance, out);
    let output = wire.output;
    let high_drop = drop("output.high_drop", output.high_drop, out);
    let high_resistance = ohm("output.high_resistance", output.high_resistance, out);
    let low_drop = drop("output.low_drop", output.low_drop, out);
    let low_resistance = ohm("output.low_resistance", output.low_resistance, out);
    Some(Ne555 {
        pins: pins?,
        ladder_resistance: ladder?,
        reset_threshold: reset?,
        discharge: Discharge {
            on_resistance: on?,
            off_resistance: off?,
        },
        supply_resistance: supply?,
        output: OutputStage {
            high_drop: high_drop?,
            high_resistance: high_resistance?,
            low_drop: low_drop?,
            low_resistance: low_resistance?,
        },
    })
}

fn resolve_switch(
    seed: u64,
    name: &Name,
    part_ref: &Ref,
    wire: WireSwitch,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Switch> {
    let pins = resolve_pins(name, Switch::PINS, &wire.pins.0, nets, out);
    let on = resolve_value(
        part_ref,
        "on_resistance",
        "ohm",
        true,
        wire.on_resistance,
        out,
    );
    let off = resolve_value(
        part_ref,
        "off_resistance",
        "ohm",
        true,
        wire.off_resistance,
        out,
    );
    let bounce = match wire.bounce {
        None => Some(None),
        Some(bounce) => resolve_bounce(part_ref, bounce, out).map(Some),
    };
    let window = match &bounce {
        Some(Some(bounce)) => bounce.window.value,
        _ => 0,
    };
    let events: Vec<Edge> = wire
        .events
        .iter()
        .map(|event| Edge {
            at: Time::from_ps(event.t_ps),
            to: event.state,
        })
        .collect();
    let events_ok = check_events(part_ref, wire.initial, &events, window, out);
    let bounce = bounce?;
    if !events_ok {
        return None;
    }
    let edges = bounce::expand(seed, name, &events, bounce.as_ref());
    Some(Switch {
        pins: pins?,
        on_resistance: on?,
        off_resistance: off?,
        events,
        bounce,
        schedule: Schedule {
            initial: wire.initial,
            edges,
        },
    })
}

fn resolve_bounce(part: &Ref, wire: WireBounce, out: &mut Vec<Diagnostic>) -> Option<Bounce> {
    let window = resolve_int(part, "bounce.window", "ps", wire.window, out);
    let max_bounces = resolve_int(part, "bounce.max_bounces", "1", wire.max_bounces, out);
    let (window, max_bounces) = (window?, max_bounces?);
    if window.value < 2 * max_bounces.value * Schedule::MIN_GAP_PS {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "bounce.window",
            value: window.value as f64,
            rule: ValueRule::BounceRoom,
        });
        return None;
    }
    Some(Bounce {
        window,
        max_bounces,
    })
}

/// One diagnostic per event that breaks a rule. True when none did.
fn check_events(
    part: &Ref,
    initial: Contact,
    events: &[Edge],
    window: u64,
    out: &mut Vec<Diagnostic>,
) -> bool {
    let mut ok = true;
    let mut contact = initial;
    let mut previous: Option<u64> = None;
    // Earliest legal `t_ps` for the next event.
    // `None` means this event's window plus the gap does not fit in `u64`, so no later event is legal.
    let mut earliest = Some(0);
    for (event, edge) in events.iter().enumerate() {
        let at = edge.at.as_ps();
        let end = at.checked_add(window);
        let rule = if previous.is_some_and(|previous| at <= previous) {
            Some(ScheduleRule::NotIncreasing)
        } else if at == 0 {
            Some(ScheduleRule::AtZero)
        } else if edge.to == contact {
            Some(ScheduleRule::SameState)
        } else if earliest.is_none_or(|earliest| at < earliest) {
            Some(ScheduleRule::TooClose)
        } else if end.is_none() {
            Some(ScheduleRule::Overflow)
        } else {
            None
        };
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                event,
                rule,
            });
            // The window itself ran past the largest time. Later events are not checked.
            if rule == ScheduleRule::Overflow {
                break;
            }
        }
        contact = edge.to;
        previous = Some(at);
        earliest = match end {
            Some(end) => end.checked_add(Schedule::MIN_GAP_PS),
            None => None,
        };
    }
    ok
}

fn resolve_int<T: Into<u64>>(
    part: &Ref,
    field: &'static str,
    expected: &'static str,
    value: WireInt<T>,
    out: &mut Vec<Diagnostic>,
) -> Option<IntValue> {
    let number: u64 = value.value.into();
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
    if number == 0 {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field,
            value: 0.0,
            rule: ValueRule::Positive,
        });
        ok = false;
    }
    ok.then(|| IntValue {
        value: number,
        source: source(value.source),
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
        source: source(value.source),
    })
}

fn source(wire: WireSource) -> Source {
    match wire {
        WireSource::User => Source::User,
        WireSource::ClosedForm { note } => Source::ClosedForm { note },
        WireSource::Datasheet { doc, at, typical } => Source::Datasheet { doc, at, typical },
    }
}
