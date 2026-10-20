//! The `potentiometer` kind: pins `a`, `wiper` and `b`, a resistance in `ohm`, and a position in `1`.

use std::collections::BTreeMap;

use super::{PinMap, WireValue, check_positive, resolve_fraction, resolve_pins, resolve_value};
use crate::diagnostic::Diagnostic;
use crate::name::{Name, Ref};
use crate::netlist::{Model, NetId, Potentiometer};

pub(super) fn resolve_potentiometer(
    name: &Name,
    part_ref: &Ref,
    pins: PinMap,
    resistance: WireValue,
    position: WireValue,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Model> {
    let pins = resolve_pins(name, Potentiometer::PINS, &pins.0, nets, out);
    let resistance = resolve_value(part_ref, "resistance", "ohm", true, resistance, out);
    let position = resolve_fraction(part_ref, "position", position, out);
    let legs = match (&resistance, &position) {
        (Some(resistance), Some(position)) => {
            leg_reciprocals(part_ref, position.si.get(), resistance.si.get(), out)
        }
        _ => false,
    };
    match (pins, resistance, position) {
        (Some(pins), Some(resistance), Some(position)) if legs => {
            Some(Model::Potentiometer(Potentiometer {
                pins,
                resistance,
                position,
            }))
        }
        _ => None,
    }
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
            field: "resistance",
            value: leg,
            rule,
        });
    }
    ok
}
