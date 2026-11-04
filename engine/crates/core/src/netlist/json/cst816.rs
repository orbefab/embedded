//! The `cst816` kind: its pins, its open-drain low side, its input threshold and its touches.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::atmega328p::{WireThreshold, resolve_threshold};
use super::{PinMap, WireValue, resolve_pins, resolve_value};
use crate::diagnostic::{Diagnostic, ScheduleRule, ValueRule};
use crate::name::{Name, Ref};
use crate::netlist::{Cst816, Event, MIN_GAP_PS, NetId, Touch};
use crate::time::Time;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireCst816 {
    pub(super) name: String,
    pins: PinMap,
    low_drop: WireValue,
    low_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default)]
    touches: Vec<WireTouch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTouch {
    t_ps: u64,
    x: u16,
    y: u16,
    down: bool,
}

pub(super) fn resolve_cst816(
    name: &Name,
    part_ref: &Ref,
    wire: WireCst816,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Cst816> {
    let pins = resolve_pins(name, Cst816::PINS, &wire.pins.0, nets, out);
    let low_drop = resolve_drop(part_ref, wire.low_drop, out);
    let low_resistance = resolve_value(
        part_ref,
        "low_resistance",
        "ohm",
        true,
        wire.low_resistance,
        out,
    );
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let touches = resolve_touches(part_ref, wire.touches, out);
    Some(Cst816 {
        pins: pins?,
        low_drop: low_drop?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
        touches: touches?,
    })
}

fn resolve_drop(
    part: &Ref,
    wire: WireValue,
    out: &mut Vec<Diagnostic>,
) -> Option<crate::netlist::Value> {
    let volts = wire.value.get();
    let value = resolve_value(part, "low_drop", "V", false, wire, out)?;
    if volts < 0.0 {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "low_drop".into(),
            value: volts,
            rule: ValueRule::NonNegative,
        });
        return None;
    }
    Some(value)
}

fn resolve_touches(
    part: &Ref,
    wires: Vec<WireTouch>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<Event<Touch>>> {
    let mut ok = true;
    let mut previous: Option<u64> = None;
    let mut earliest = Some(0u64);
    let mut events = Vec::with_capacity(wires.len());
    for (index, wire) in wires.into_iter().enumerate() {
        let at = wire.t_ps;
        let rule = if previous.is_some_and(|previous| at <= previous) {
            Some(ScheduleRule::NotIncreasing)
        } else if at == 0 {
            Some(ScheduleRule::AtZero)
        } else if earliest.is_none_or(|earliest| at < earliest) {
            Some(ScheduleRule::TooClose)
        } else {
            None
        };
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: "touches",
                event: index,
                rule,
                end_ps: 0,
            });
        }
        for (axis, value) in [("x", wire.x), ("y", wire.y)] {
            if value > Cst816::COORD_MAX {
                ok = false;
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: format!("touches[{index}].{axis}").into(),
                    value: f64::from(value),
                    rule: ValueRule::Span,
                });
            }
        }
        events.push(Event {
            at: Time::from_ps(at),
            to: Touch {
                x: wire.x,
                y: wire.y,
                down: wire.down,
            },
        });
        previous = Some(at);
        earliest = at.checked_add(MIN_GAP_PS);
    }
    ok.then_some(events)
}
