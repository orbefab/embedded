//! JSON for a shared input threshold. Several kinds read one.

use crate::netlist::Threshold;

use serde::Deserialize;

use super::{WireValue, resolve_fraction};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::name::Ref;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireThreshold {
    low: WireValue,
    high: WireValue,
}

pub(crate) fn resolve_threshold(
    part: &Ref,
    [low_field, high_field]: [&'static str; 2],
    wire: WireThreshold,
    out: &mut Vec<Diagnostic>,
) -> Option<Threshold> {
    let low = resolve_fraction(part, low_field, wire.low, out);
    let high = resolve_fraction(part, high_field, wire.high, out);
    let (low, high) = (low?, high?);
    if low.si.get() > high.si.get() {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: low_field.into(),
            value: low.si.get(),
            rule: ValueRule::AboveHigh,
        });
        return None;
    }
    Some(Threshold { low, high })
}
