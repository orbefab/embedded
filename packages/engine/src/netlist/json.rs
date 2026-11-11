use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

pub(crate) mod threshold;
pub(crate) mod waveform;

use orbe_model::Time;

use super::catalog::WirePart;
use super::{
    Analysis, Basis, Event, IntValue, MIN_GAP_PS, NetId, Netlist, Part, Source, Transient, Value,
};
use crate::blob::{Blobs, Sha256};
use crate::diagnostic::{AnalysisRule, Diagnostic, ScheduleRule, ValueRule};
use crate::finite::Finite;
use crate::name::{Name, NameError, Ref};
pub(crate) use waveform::AMPERE;
pub(crate) const FRACTION_UNIT: &str = "1";

/// What a kind's resolve function reads besides its wire payload.
pub(crate) struct ParseCtx<'a> {
    pub(crate) seed: u64,
    pub(crate) name: &'a Name,
    pub(crate) part: &'a Ref,
    pub(crate) nets: &'a BTreeMap<String, NetId>,
    pub(crate) blobs: &'a Blobs,
    pub(crate) out: &'a mut Vec<Diagnostic>,
}

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

/// `value` must be a JSON integer in `T`'s range. Anything else is `netlist.syntax`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireInt<T> {
    pub(crate) value: T,
    pub(crate) unit: String,
    pub(crate) source: WireSource,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireJunctionRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) current: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) power: Option<WireValue>,
}

/// Pin objects reject a repeated key. `BTreeMap` would keep the last one.
pub(crate) struct PinMap(pub(crate) BTreeMap<String, String>);

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
pub(crate) struct WireValue {
    pub(crate) value: Finite,
    pub(crate) unit: String,
    pub(crate) source: WireSource,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WireSource {
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
pub(crate) fn parse(json: &[u8], blobs: &Blobs) -> Result<Netlist, Vec<Diagnostic>> {
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
    resolve(wire, blobs)
}

pub(crate) fn blob_names(json: &[u8]) -> Vec<Sha256> {
    let Ok(wire) = serde_json::from_slice::<WireNetlist>(json) else {
        return Vec::new();
    };
    wire.parts
        .iter()
        .flat_map(|part| match part {
            WirePart::Esp32s3(chip) => chip.blob_names(),
            _ => Vec::new(),
        })
        .collect()
}

pub(crate) fn probe_version(json: &[u8]) -> Option<u64> {
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

fn resolve(wire: WireNetlist, blobs: &Blobs) -> Result<Netlist, Vec<Diagnostic>> {
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
        if let Some(part) = resolve_part(
            wire.seed,
            index,
            part,
            &net_ids,
            blobs,
            &mut seen_parts,
            &mut out,
        ) {
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
    blobs: &Blobs,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Diagnostic>,
) -> Option<Part> {
    let raw_name = part.name();
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
    let model = part.resolve(ParseCtx {
        seed,
        name: &name,
        part: &part_ref,
        nets,
        blobs,
        out,
    });
    if duplicate || reserved {
        None
    } else {
        model.map(|model| Part { name, model })
    }
}

/// A field that is not a positive reciprocal, checked by `rule` after its unit. Zero can be legal.
pub(crate) fn bounded(
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

pub(crate) fn resolve_schedule(
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
pub(crate) fn check_events<T: Clone>(
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

pub(crate) fn resolve_int<T: Into<u64>>(
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

pub(crate) fn resolve_pins<const N: usize>(
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
pub(crate) fn resolve_rating(
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

pub(crate) fn resolve_value(
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

pub(crate) fn resolve_fraction(
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
