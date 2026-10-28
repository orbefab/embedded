use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

mod atmega328p;
mod potentiometer;
mod spi_replay;
mod waveform;

use super::{
    Analysis, Basis, Bjt, Bounce, BreakdownFault, Capacitor, Chip, Contact, CurrentSource,
    Discharge, Edge, Event, Inductor, IntValue, Junction, LinearRegulator, MIN_GAP_PS, Model,
    Mosfet, Ne555, NetId, Netlist, OutputStage, Part, Relay, Resistor, Schedule, Source, Switch,
    Transient, Value, VoltageSource, Zener, bounce, breakdown_scale, junction_scale,
};
use crate::diagnostic::{AnalysisRule, Diagnostic, ScheduleRule, ValueRule};
use crate::finite::Finite;
use crate::name::{Name, NameError, Ref};
use crate::time::Time;
use atmega328p::{WireAtmega328p, resolve_atmega328p};
use spi_replay::{WireSpiReplay, resolve_spi_replay};
use waveform::{CURRENT, VOLTAGE, WireWaveform, resolve_waveform};

pub(crate) use waveform::{AMPERE, VOLT};
pub(crate) const FRACTION_UNIT: &str = "1";

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
    grid_ps: u64,
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
    Potentiometer {
        name: String,
        pins: PinMap,
        resistance: WireValue,
        position: waveform::WirePosition,
    },
    VoltageSource {
        name: String,
        pins: PinMap,
        voltage: WireWaveform,
    },
    CurrentSource {
        name: String,
        pins: PinMap,
        current: WireWaveform,
    },
    Diode(WireJunction),
    Led(WireJunction),
    Zener(Box<WireZener>),
    LinearRegulator(Box<WireLinearRegulator>),
    Npn(Box<WireBjt>),
    Pnp(Box<WireBjt>),
    Nmos(Box<WireMosfet>),
    Capacitor {
        name: String,
        pins: PinMap,
        capacitance: WireValue,
        // Required but nullable, like the ratings below.
        #[serde(deserialize_with = "Option::deserialize")]
        initial: Option<WireValue>,
    },
    Inductor {
        name: String,
        pins: PinMap,
        inductance: WireValue,
        series_resistance: WireValue,
    },
    Relay(Box<WireRelay>),
    Switch(WireSwitch),
    Ne555(Box<WireNe555>),
    Atmega328p(Box<WireAtmega328p>),
    SpiReplay(Box<WireSpiReplay>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRelay {
    name: String,
    pins: PinMap,
    inductance: WireValue,
    coil_resistance: WireValue,
    pull_in_current: WireValue,
    drop_out_current: WireValue,
    on_resistance: WireValue,
    off_resistance: WireValue,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLinearRegulator {
    name: String,
    pins: PinMap,
    output_voltage: WireValue,
    line_regulation: WireValue,
    output_resistance: WireValue,
    dropout_voltage: WireValue,
    dropout_resistance: WireValue,
    current_limit: WireValue,
    quiescent_current: WireValue,
    ground_current_ratio: WireValue,
    ratings: WireJunctionRatings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireZener {
    name: String,
    pins: PinMap,
    saturation_current: WireValue,
    emission_coefficient: WireValue,
    series_resistance: WireValue,
    zener_voltage: WireValue,
    zener_current: WireValue,
    zener_impedance: WireValue,
    breakdown_resistance: WireValue,
    ratings: WireJunctionRatings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBjt {
    name: String,
    pins: PinMap,
    saturation_current: WireValue,
    forward_beta: WireValue,
    reverse_beta: WireValue,
    forward_emission: WireValue,
    reverse_emission: WireValue,
    early_voltage: WireValue,
    base_resistance: WireValue,
    collector_resistance: WireValue,
    emitter_resistance: WireValue,
    base_emitter_capacitance: WireValue,
    base_collector_capacitance: WireValue,
    ratings: WireJunctionRatings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireMosfet {
    name: String,
    pins: PinMap,
    transconductance_parameter: WireValue,
    threshold_voltage: WireValue,
    channel_length_modulation: WireValue,
    gate_source_capacitance: WireValue,
    gate_drain_capacitance: WireValue,
    drain_source_capacitance: WireValue,
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
        basis: Basis,
    },
}

/// A later version adds its own wire types here, and a lift into the domain types. Nothing
/// past this function sees a version.
pub(super) fn parse(json: &[u8]) -> Result<Netlist, Vec<Diagnostic>> {
    if let Some(found) = probe_version(json)
        && !super::VERSIONS.contains(&found)
    {
        return Err(vec![Diagnostic::NetlistVersion {
            found,
            supported: super::VERSIONS,
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

pub(super) fn probe_version(json: &[u8]) -> Option<u64> {
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
    debug_assert!(super::VERSIONS.contains(&wire.version));
    parts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Netlist {
        version: wire.version,
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
    if transient.grid_ps < Transient::MIN_GRID_PS {
        broken.push(("grid_ps", transient.grid_ps, AnalysisRule::GridUnder1ns));
    }
    if transient.grid_ps > transient.stop_ps {
        broken.push(("grid_ps", transient.grid_ps, AnalysisRule::GridOverStop));
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
        grid: transient.grid_ps,
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
        | WirePart::Potentiometer { name, .. }
        | WirePart::VoltageSource { name, .. }
        | WirePart::CurrentSource { name, .. }
        | WirePart::Capacitor { name, .. }
        | WirePart::Inductor { name, .. } => name,
        WirePart::Diode(junction) | WirePart::Led(junction) => &junction.name,
        WirePart::Zener(zener) => &zener.name,
        WirePart::LinearRegulator(regulator) => &regulator.name,
        WirePart::Npn(bjt) | WirePart::Pnp(bjt) => &bjt.name,
        WirePart::Nmos(mosfet) => &mosfet.name,
        WirePart::Relay(relay) => &relay.name,
        WirePart::Switch(switch) => &switch.name,
        WirePart::Ne555(chip) => &chip.name,
        WirePart::Atmega328p(chip) => &chip.name,
        WirePart::SpiReplay(chip) => &chip.name,
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
        WirePart::Potentiometer {
            pins,
            resistance,
            position,
            ..
        } => potentiometer::resolve_potentiometer(
            &name, &part_ref, pins, resistance, position, nets, out,
        ),
        WirePart::VoltageSource { pins, voltage, .. } => {
            let pins = resolve_pins(&name, VoltageSource::PINS, &pins.0, nets, out);
            let voltage = resolve_waveform(&part_ref, &VOLTAGE, voltage, out);
            match (pins, voltage) {
                (Some(pins), Some(voltage)) => {
                    Some(Model::VoltageSource(VoltageSource { pins, voltage }))
                }
                _ => None,
            }
        }
        WirePart::CurrentSource { pins, current, .. } => {
            let pins = resolve_pins(&name, CurrentSource::PINS, &pins.0, nets, out);
            let current = resolve_waveform(&part_ref, &CURRENT, current, out);
            match (pins, current) {
                (Some(pins), Some(current)) => {
                    Some(Model::CurrentSource(CurrentSource { pins, current }))
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
        WirePart::Zener(zener) => {
            resolve_zener(&name, &part_ref, *zener, nets, out).map(Model::Zener)
        }
        WirePart::LinearRegulator(regulator) => {
            resolve_regulator(&name, &part_ref, *regulator, nets, out).map(Model::LinearRegulator)
        }
        WirePart::Npn(bjt) => resolve_bjt(&name, &part_ref, *bjt, nets, out).map(Model::Npn),
        WirePart::Pnp(bjt) => resolve_bjt(&name, &part_ref, *bjt, nets, out).map(Model::Pnp),
        WirePart::Nmos(mosfet) => {
            resolve_mosfet(&name, &part_ref, *mosfet, nets, out).map(Model::Nmos)
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
        WirePart::Inductor {
            pins,
            inductance,
            series_resistance,
            ..
        } => {
            let pins = resolve_pins(&name, Inductor::PINS, &pins.0, nets, out);
            let inductance = resolve_value(&part_ref, "inductance", "H", true, inductance, out);
            let series_resistance = resolve_value(
                &part_ref,
                "series_resistance",
                "ohm",
                true,
                series_resistance,
                out,
            );
            match (pins, inductance, series_resistance) {
                (Some(pins), Some(inductance), Some(series_resistance)) => {
                    Some(Model::Inductor(Inductor {
                        pins,
                        inductance,
                        series_resistance,
                    }))
                }
                _ => None,
            }
        }
        WirePart::Relay(relay) => {
            resolve_relay(&name, &part_ref, *relay, nets, out).map(Model::Relay)
        }
        WirePart::Switch(switch) => {
            resolve_switch(seed, &name, &part_ref, switch, nets, out).map(Model::Switch)
        }
        WirePart::Ne555(chip) => resolve_ne555(&name, &part_ref, *chip, nets, out)
            .map(|chip| Model::Chip(Chip::Ne555(Box::new(chip)))),
        WirePart::Atmega328p(chip) => resolve_atmega328p(&name, &part_ref, *chip, nets, out)
            .map(|chip| Model::Chip(Chip::Atmega328p(Box::new(chip)))),
        WirePart::SpiReplay(chip) => resolve_spi_replay(&name, &part_ref, *chip, nets, out)
            .map(|chip| Model::Chip(Chip::SpiReplay(Box::new(chip)))),
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
            field: fault.field("emission_coefficient").into(),
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

fn resolve_zener(
    name: &Name,
    part_ref: &Ref,
    wire: WireZener,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Zener> {
    let pins = resolve_pins(name, Junction::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let is = positive("saturation_current", "A", wire.saturation_current);
    let n = positive("emission_coefficient", "1", wire.emission_coefficient);
    let rs = positive("series_resistance", "ohm", wire.series_resistance);
    let vz = positive("zener_voltage", "V", wire.zener_voltage);
    let izt = positive("zener_current", "A", wire.zener_current);
    let zzt = positive("zener_impedance", "ohm", wire.zener_impedance);
    let rb = positive("breakdown_resistance", "ohm", wire.breakdown_resistance);
    let max_current = resolve_rating(part_ref, "ratings.current", "A", wire.ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", wire.ratings.power, out);
    let mut rejected = false;
    let mut breakdown = None;
    if let (Some(is), Some(n), Some(rs), Some(vz), Some(izt), Some(zzt), Some(rb)) =
        (&is, &n, &rs, &vz, &izt, &zzt, &rb)
    {
        if let Err(fault) = junction_scale(n.si.get(), is.si.get()) {
            rejected = true;
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: fault.field("emission_coefficient").into(),
                value: fault.value(n.si.get(), is.si.get()),
                rule: ValueRule::FiniteJunction,
            });
        }
        let series = rs.si.get() + rb.si.get();
        let resistance_ok = zzt.si.get() > series;
        let drop_ok = vz.si.get() > izt.si.get() * series;
        if !resistance_ok {
            rejected = true;
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "zener_impedance".into(),
                value: zzt.si.get(),
                rule: ValueRule::AboveResistance,
            });
        }
        if !drop_ok {
            rejected = true;
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "zener_voltage".into(),
                value: vz.si.get(),
                rule: ValueRule::AboveDrop,
            });
        }
        if resistance_ok && drop_ok {
            match breakdown_scale(
                rs.si.get(),
                rb.si.get(),
                vz.si.get(),
                izt.si.get(),
                zzt.si.get(),
            ) {
                Ok(found) => breakdown = Some(found),
                Err(fault) => {
                    rejected = true;
                    let value = match fault {
                        BreakdownFault::Impedance => zzt.si.get(),
                        BreakdownFault::Voltage => vz.si.get(),
                    };
                    out.push(Diagnostic::NetlistValue {
                        part: part_ref.clone(),
                        field: fault.field().into(),
                        value,
                        rule: ValueRule::FiniteJunction,
                    });
                }
            }
        }
    }
    if rejected {
        return None;
    }
    Some(Zener {
        pins: pins?,
        saturation_current: is?,
        emission_coefficient: n?,
        series_resistance: rs?,
        zener_voltage: vz?,
        zener_current: izt?,
        zener_impedance: zzt?,
        breakdown_resistance: rb?,
        breakdown: breakdown?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

fn resolve_regulator(
    name: &Name,
    part_ref: &Ref,
    wire: WireLinearRegulator,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<LinearRegulator> {
    let pins = resolve_pins(name, LinearRegulator::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let output_voltage = positive("output_voltage", "V", wire.output_voltage);
    let output_resistance = positive("output_resistance", "ohm", wire.output_resistance);
    let dropout_voltage = positive("dropout_voltage", "V", wire.dropout_voltage);
    let dropout_resistance = positive("dropout_resistance", "ohm", wire.dropout_resistance);
    let current_limit = positive("current_limit", "A", wire.current_limit);
    let line_regulation = bounded(
        part_ref,
        "line_regulation",
        "1",
        wire.line_regulation,
        |number| (number < 0.0 || number >= 1.0).then_some(ValueRule::BelowOne),
        out,
    );
    let quiescent_current = bounded(
        part_ref,
        "quiescent_current",
        "A",
        wire.quiescent_current,
        |number| (number < 0.0).then_some(ValueRule::NonNegative),
        out,
    );
    let ground_current_ratio = bounded(
        part_ref,
        "ground_current_ratio",
        "1",
        wire.ground_current_ratio,
        |number| (number < 0.0).then_some(ValueRule::NonNegative),
        out,
    );
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    Some(LinearRegulator {
        pins: pins?,
        output_voltage: output_voltage?,
        line_regulation: line_regulation?,
        output_resistance: output_resistance?,
        dropout_voltage: dropout_voltage?,
        dropout_resistance: dropout_resistance?,
        current_limit: current_limit?,
        quiescent_current: quiescent_current?,
        ground_current_ratio: ground_current_ratio?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

/// A field that is not a positive reciprocal, checked by `rule` after its unit. Zero can be legal.
fn bounded(
    part: &Ref,
    field: &'static str,
    unit: &'static str,
    value: WireValue,
    rule: impl FnOnce(f64) -> Option<ValueRule>,
    out: &mut Vec<Diagnostic>,
) -> Option<Value> {
    let number = value.value.get();
    resolve_value(part, field, unit, false, value, out).and_then(|value| match rule(number) {
        Some(rule) => {
            out.push(Diagnostic::NetlistValue {
                part: part.clone(),
                field: field.into(),
                value: number,
                rule,
            });
            None
        }
        None => Some(value),
    })
}

fn resolve_bjt(
    name: &Name,
    part_ref: &Ref,
    wire: WireBjt,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Bjt> {
    let pins = resolve_pins(name, Bjt::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let is = positive("saturation_current", "A", wire.saturation_current);
    let bf = positive("forward_beta", "1", wire.forward_beta);
    let br = positive("reverse_beta", "1", wire.reverse_beta);
    let nf = positive("forward_emission", "1", wire.forward_emission);
    let nr = positive("reverse_emission", "1", wire.reverse_emission);
    let vaf = positive("early_voltage", "V", wire.early_voltage);
    let rb = positive("base_resistance", "ohm", wire.base_resistance);
    let rc = positive("collector_resistance", "ohm", wire.collector_resistance);
    let re = positive("emitter_resistance", "ohm", wire.emitter_resistance);
    let cje = positive(
        "base_emitter_capacitance",
        "F",
        wire.base_emitter_capacitance,
    );
    let cjc = positive(
        "base_collector_capacitance",
        "F",
        wire.base_collector_capacitance,
    );
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    let mut faults = Vec::new();
    if let Some(is) = &is {
        for (field, n) in [("forward_emission", &nf), ("reverse_emission", &nr)] {
            if let Some(n) = n
                && let Err(fault) = junction_scale(n.si.get(), is.si.get())
            {
                let at = (fault.field(field), fault.value(n.si.get(), is.si.get()));
                // Both junctions share `saturation_current`, which breaks the rule once.
                if !faults.contains(&at) {
                    faults.push(at);
                }
            }
        }
    }
    let junctions = faults.is_empty();
    for (field, value) in faults {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: field.into(),
            value,
            rule: ValueRule::FiniteJunction,
        });
    }
    if !junctions {
        return None;
    }
    Some(Bjt {
        pins: pins?,
        saturation_current: is?,
        forward_beta: bf?,
        reverse_beta: br?,
        forward_emission: nf?,
        reverse_emission: nr?,
        early_voltage: vaf?,
        base_resistance: rb?,
        collector_resistance: rc?,
        emitter_resistance: re?,
        base_emitter_capacitance: cje?,
        base_collector_capacitance: cjc?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

fn resolve_mosfet(
    name: &Name,
    part_ref: &Ref,
    wire: WireMosfet,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Mosfet> {
    let pins = resolve_pins(name, Mosfet::PINS, &wire.pins.0, nets, out);
    let lambda_number = wire.channel_length_modulation.value.get();
    let (kp, cgs, cgd, cds) = {
        let mut positive = |field: &'static str, unit, value| {
            resolve_value(part_ref, field, unit, true, value, out)
        };
        (
            positive(
                "transconductance_parameter",
                "A/V^2",
                wire.transconductance_parameter,
            ),
            positive("gate_source_capacitance", "F", wire.gate_source_capacitance),
            positive("gate_drain_capacitance", "F", wire.gate_drain_capacitance),
            positive(
                "drain_source_capacitance",
                "F",
                wire.drain_source_capacitance,
            ),
        )
    };
    let vto = resolve_value(
        part_ref,
        "threshold_voltage",
        "V",
        false,
        wire.threshold_voltage,
        out,
    );
    let lambda = resolve_value(
        part_ref,
        "channel_length_modulation",
        "1/V",
        false,
        wire.channel_length_modulation,
        out,
    )
    .and_then(|value| {
        if lambda_number < 0.0 {
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "channel_length_modulation".into(),
                value: lambda_number,
                rule: ValueRule::NonNegative,
            });
            None
        } else {
            Some(value)
        }
    });
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    Some(Mosfet {
        pins: pins?,
        transconductance_parameter: kp?,
        threshold_voltage: vto?,
        channel_length_modulation: lambda?,
        gate_source_capacitance: cgs?,
        gate_drain_capacitance: cgd?,
        drain_source_capacitance: cds?,
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
    let output = resolve_output_stage(part_ref, wire.output, out);
    Some(Ne555 {
        pins: pins?,
        ladder_resistance: ladder?,
        reset_threshold: reset?,
        discharge: Discharge {
            on_resistance: on?,
            off_resistance: off?,
        },
        supply_resistance: supply?,
        output: output?,
    })
}

fn resolve_output_stage(
    part_ref: &Ref,
    wire: WireOutputStage,
    out: &mut Vec<Diagnostic>,
) -> Option<OutputStage> {
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    let drop = |field: &'static str, value: WireValue, out: &mut Vec<Diagnostic>| {
        let volts = value.value.get();
        let value = resolve_value(part_ref, field, "V", false, value, out)?;
        if volts < 0.0 {
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: field.into(),
                value: volts,
                rule: ValueRule::NonNegative,
            });
            return None;
        }
        Some(value)
    };
    let high_drop = drop("output.high_drop", wire.high_drop, out);
    let high_resistance = ohm("output.high_resistance", wire.high_resistance, out);
    let low_drop = drop("output.low_drop", wire.low_drop, out);
    let low_resistance = ohm("output.low_resistance", wire.low_resistance, out);
    Some(OutputStage {
        high_drop: high_drop?,
        high_resistance: high_resistance?,
        low_drop: low_drop?,
        low_resistance: low_resistance?,
    })
}

fn resolve_relay(
    name: &Name,
    part_ref: &Ref,
    wire: WireRelay,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Relay> {
    let pins = resolve_pins(name, Relay::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let inductance = positive("inductance", "H", wire.inductance);
    let coil_resistance = positive("coil_resistance", "ohm", wire.coil_resistance);
    let pull_in_current = positive("pull_in_current", AMPERE, wire.pull_in_current);
    let drop_out_current = positive("drop_out_current", AMPERE, wire.drop_out_current);
    let on_resistance = positive("on_resistance", "ohm", wire.on_resistance);
    let off_resistance = positive("off_resistance", "ohm", wire.off_resistance);
    if let (Some(pull), Some(drop)) = (&pull_in_current, &drop_out_current)
        && drop.si.get() >= pull.si.get()
    {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: "drop_out_current".into(),
            value: drop.si.get(),
            rule: ValueRule::Hysteresis,
        });
        return None;
    }
    Some(Relay {
        pins: pins?,
        inductance: inductance?,
        coil_resistance: coil_resistance?,
        pull_in_current: pull_in_current?,
        drop_out_current: drop_out_current?,
        on_resistance: on_resistance?,
        off_resistance: off_resistance?,
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
    let events_ok = check_events(
        part_ref,
        "events",
        wire.initial,
        &events,
        window,
        |left, right| left == right,
        out,
    );
    let bounce = bounce?;
    if !events_ok {
        return None;
    }
    let edges = bounce::expand(seed, name, &events, bounce.as_ref());
    Some(Switch {
        pins: pins?,
        on_resistance: on?,
        off_resistance: off?,
        events: Schedule {
            initial: wire.initial,
            events,
        },
        bounce,
        schedule: Schedule {
            initial: wire.initial,
            events: edges,
        },
    })
}

fn resolve_bounce(part: &Ref, wire: WireBounce, out: &mut Vec<Diagnostic>) -> Option<Bounce> {
    let window = resolve_int(part, "bounce.window", "ps", wire.window, out);
    let max_bounces = resolve_int(part, "bounce.max_bounces", "1", wire.max_bounces, out);
    let (window, max_bounces) = (window?, max_bounces?);
    if window.value < 2 * max_bounces.value * MIN_GAP_PS {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "bounce.window".into(),
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

fn resolve_schedule(
    part: &Ref,
    events_field: &'static str,
    initial_field: &'static str,
    initial_wire: WireValue,
    events: Vec<waveform::WireLevel>,
    mut level: impl FnMut(Cow<'static, str>, WireValue, &mut Vec<Diagnostic>) -> (bool, Value),
    out: &mut Vec<Diagnostic>,
) -> Option<(Value, Vec<Event<Value>>)> {
    let (mut ok, initial) = level(Cow::Borrowed(initial_field), initial_wire, out);
    let mut resolved = Vec::with_capacity(events.len());
    for (index, event) in events.into_iter().enumerate() {
        let field = format!("{events_field}[{index}].level");
        let (level_ok, value) = level(Cow::Owned(field), event.level, out);
        ok &= level_ok;
        resolved.push(Event {
            at: Time::from_ps(event.t_ps),
            to: value,
        });
    }
    let events_ok = check_events(
        part,
        events_field,
        initial.clone(),
        &resolved,
        0,
        |left, right| left.si == right.si,
        out,
    );
    (ok && events_ok).then_some((initial, resolved))
}

/// One diagnostic per event that breaks a rule. True when none did.
/// `same` says the event does not change the value in force. A switch compares contacts.
/// A level compares the SI value.
fn check_events<T: Clone>(
    part: &Ref,
    field: &'static str,
    initial: T,
    events: &[Event<T>],
    window: u64,
    same: impl Fn(&T, &T) -> bool,
    out: &mut Vec<Diagnostic>,
) -> bool {
    let mut ok = true;
    let mut current = initial;
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
        } else if same(&edge.to, &current) {
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
                field,
                event,
                rule,
                end_ps: 0,
            });
            // The window itself ran past the largest time. Later events are not checked.
            if rule == ScheduleRule::Overflow {
                break;
            }
        }
        current = edge.to.clone();
        previous = Some(at);
        earliest = match end {
            Some(end) => end.checked_add(MIN_GAP_PS),
            None => None,
        };
    }
    ok
}

fn resolve_int<T: Into<u64>>(
    part: &Ref,
    field: impl Into<Cow<'static, str>>,
    expected: &'static str,
    value: WireInt<T>,
    out: &mut Vec<Diagnostic>,
) -> Option<IntValue> {
    let field = field.into();
    let number: u64 = value.value.into();
    let mut ok = true;
    if value.unit != expected {
        out.push(Diagnostic::NetlistUnit {
            part: part.clone(),
            field: field.clone(),
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
    field: impl Into<Cow<'static, str>>,
    expected: &'static str,
    positive: bool,
    value: WireValue,
    out: &mut Vec<Diagnostic>,
) -> Option<Value> {
    let field = field.into();
    let mut ok = true;
    if value.unit != expected {
        out.push(Diagnostic::NetlistUnit {
            part: part.clone(),
            field: field.clone(),
            expected,
            found: value.unit,
        });
        ok = false;
    }
    let number = value.value.get();
    if positive && let Some(broken) = super::broken_positive(number) {
        ok = false;
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: field.clone(),
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

fn resolve_fraction(
    part: &Ref,
    field: impl Into<Cow<'static, str>>,
    value: WireValue,
    out: &mut Vec<Diagnostic>,
) -> Option<Value> {
    let field = field.into();
    let number = value.value.get();
    let resolved = resolve_value(part, field.clone(), FRACTION_UNIT, false, value, out);
    if super::broken_fraction(number).is_some() {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field,
            value: number,
            rule: ValueRule::Fraction,
        });
        return None;
    }
    resolved
}

fn source(wire: WireSource) -> Source {
    match wire {
        WireSource::User => Source::User,
        WireSource::ClosedForm { note } => Source::ClosedForm { note },
        WireSource::Datasheet { doc, at, basis } => Source::Datasheet { doc, at, basis },
    }
}
