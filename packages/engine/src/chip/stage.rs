use crate::diagnostic::Diagnostic;
use crate::diagnostic::ValueRule;
use crate::name::Ref;
use crate::netlist::Value;
use crate::netlist::json::WireValue;
use crate::netlist::json::resolve_value;
use serde::Deserialize;

/// A push-pull output to the chip's own rails, linear on each side. The current into OUT is
/// `(V(vcc) - high_drop - V(out)) / high_resistance` while high and
/// `(V(gnd) + low_drop - V(out)) / low_resistance` while low.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputStage {
    /// Volt, at least 0.
    pub high_drop: Value,
    /// Ohm.
    pub high_resistance: Value,
    /// Volt, at least 0.
    pub low_drop: Value,
    /// Ohm.
    pub low_resistance: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireOutputStage {
    high_drop: WireValue,
    high_resistance: WireValue,
    low_drop: WireValue,
    low_resistance: WireValue,
}

pub(crate) fn resolve_output_stage(
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
