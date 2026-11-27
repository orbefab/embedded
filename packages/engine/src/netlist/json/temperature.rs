//! JSON for an operating temperature range, `ratings.temperature`. Several kinds read one.

use serde::Deserialize;

use super::{WireValue, resolve_value};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::name::Ref;
use crate::netlist::TemperatureRange;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireTemperatureRange {
    min: WireValue,
    max: WireValue,
}

/// `Some(None)` when the part rates no temperature. `None` when the range is invalid: a unit
/// other than `degC`, or `min` above `max`.
pub(crate) fn resolve_temperature_range(
    part: &Ref,
    wire: Option<WireTemperatureRange>,
    out: &mut Vec<Diagnostic>,
) -> Option<Option<TemperatureRange>> {
    let Some(wire) = wire else {
        return Some(None);
    };
    let min = resolve_value(
        part,
        "ratings.temperature.min",
        "degC",
        false,
        wire.min,
        out,
    );
    let max = resolve_value(
        part,
        "ratings.temperature.max",
        "degC",
        false,
        wire.max,
        out,
    );
    let (min, max) = (min?, max?);
    if min.si.get() > max.si.get() {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "ratings.temperature.min".into(),
            value: min.si.get(),
            rule: ValueRule::AboveHigh,
        });
        return None;
    }
    Some(Some(TemperatureRange { min, max }))
}
