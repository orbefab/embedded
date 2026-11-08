//! The `potentiometer` kind: pins `a`, `wiper` and `b`, a resistance in `ohm`, and a position in `1`.

use std::collections::BTreeMap;

use super::waveform::{WirePosition, WireSteps};
use super::{FRACTION_UNIT, PinMap, WireValue, resolve_pins, resolve_value};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::name::{Name, Ref};
use crate::netlist::{
    Model, NetId, Potentiometer, Schedule, Source, Value, broken_fraction, position_rule,
};

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
    let number = si.get();
    let field = field.into();
    let resolved = resolve_value(part, field.clone(), FRACTION_UNIT, false, wire, out);
    let mut ok = resolved.is_some();
    let value = resolved.unwrap_or(Value {
        si,
        source: Source::User,
    });
    if let Some(resistance) = resistance {
        let Some(legs) = position_rule(number, resistance.si.get()).legs() else {
            refuse_fraction(part, field, number, out);
            return (false, value);
        };
        if !ok {
            return (false, value);
        }
        for (ohms, rule) in legs {
            let Some(rule) = rule else {
                continue;
            };
            ok = false;
            out.push(Diagnostic::NetlistValue {
                part: part.clone(),
                field: "resistance".into(),
                value: ohms,
                rule,
            });
        }
    } else if broken_fraction(number).is_some() {
        refuse_fraction(part, field, number, out);
        return (false, value);
    }
    (ok, value)
}

fn refuse_fraction(
    part: &Ref,
    field: std::borrow::Cow<'static, str>,
    number: f64,
    out: &mut Vec<Diagnostic>,
) {
    out.push(Diagnostic::NetlistValue {
        part: part.clone(),
        field,
        value: number,
        rule: ValueRule::Fraction,
    });
}
