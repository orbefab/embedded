//! The `potentiometer` kind: pins `a`, `wiper` and `b`, a resistance in `ohm`, and a position in `1`.

use std::collections::BTreeMap;

use super::waveform::{WirePosition, WireSteps};
use super::{PinMap, WireValue, check_positive, resolve_fraction, resolve_pins, resolve_value};
use crate::diagnostic::Diagnostic;
use crate::name::{Name, Ref};
use crate::netlist::{Model, NetId, Potentiometer, Schedule, Source, Value};

pub(super) fn resolve_potentiometer(
    name: &Name,
    part_ref: &Ref,
    pins: PinMap,
    resistance: WireValue,
    position: WirePosition,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Model> {
    let pins = resolve_pins(name, Potentiometer::PINS, &pins.0, nets, out);
    let resistance = resolve_value(part_ref, "resistance", "ohm", true, resistance, out);
    let position = resolve_position(part_ref, resistance.as_ref(), position, out);
    match (pins, resistance, position) {
        (Some(pins), Some(resistance), Some(position)) => {
            Some(Model::Potentiometer(Potentiometer {
                pins,
                resistance,
                position,
            }))
        }
        _ => None,
    }
}

fn resolve_position(
    part: &Ref,
    resistance: Option<&Value>,
    position: WirePosition,
    out: &mut Vec<Diagnostic>,
) -> Option<Schedule<Value>> {
    match position {
        WirePosition::Value(value) => {
            let (ok, position) = level_with_legs(part, "position", value, resistance, out);
            ok.then_some(Schedule {
                initial: position,
                events: Vec::new(),
            })
        }
        WirePosition::Steps(steps) => resolve_steps(part, resistance, steps, out),
    }
}

fn resolve_steps(
    part: &Ref,
    resistance: Option<&Value>,
    steps: WireSteps,
    out: &mut Vec<Diagnostic>,
) -> Option<Schedule<Value>> {
    super::resolve_schedule(
        part,
        "position.events",
        "position.initial",
        steps.initial,
        steps.events,
        |field, wire, out| level_with_legs(part, field, wire, resistance, out),
        out,
    )
    .map(|(initial, events)| Schedule { initial, events })
}

fn level_with_legs(
    part: &Ref,
    field: impl Into<std::borrow::Cow<'static, str>>,
    wire: WireValue,
    resistance: Option<&Value>,
    out: &mut Vec<Diagnostic>,
) -> (bool, Value) {
    let si = wire.value;
    let resolved = resolve_fraction(part, field, wire, out);
    let Some(position) = resolved else {
        return (
            false,
            Value {
                si,
                source: Source::User,
            },
        );
    };
    let legs = match resistance {
        Some(resistance) => leg_reciprocals(part, position.si.get(), resistance.si.get(), out),
        None => true,
    };
    (legs, position)
}

fn leg_reciprocals(part: &Ref, position: f64, resistance: f64, out: &mut Vec<Diagnostic>) -> bool {
    let mut ok = true;
    for leg in [position * resistance, (1.0 - position) * resistance] {
        let Some(rule) = check_positive(leg) else {
            continue;
        };
        ok = false;
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "resistance".into(),
            value: leg,
            rule,
        });
    }
    ok
}
