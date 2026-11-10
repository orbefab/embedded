use orbe_model::Time;

use crate::diagnostic::Diagnostic;
use crate::name::Name;
use crate::name::Ref;
use crate::netlist::Model;
use crate::netlist::NetId;
use crate::netlist::Waveform;
use crate::netlist::json::PinMap;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::waveform::CURRENT;
use crate::netlist::json::waveform::WireWaveform;
use crate::netlist::json::waveform::resolve_waveform;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::mna::Stamp;
use crate::newton::Side;

/// The current leaves at `p`, so it flows through the part from `n` to `p`. `Stamp::current`
/// does not scale, so the stamp applies `source_scale` itself.
pub(crate) fn stamp(source: &CurrentSource, stamp: &mut Stamp<'_>) {
    let amps = stamp.conditions.level(&source.current) * stamp.conditions.source_scale;
    stamp.current(source.pins[1], source.pins[0], amps);
}

/// Into pin `p`.
pub(crate) fn current(source: &CurrentSource, at: Time, side: Side) -> f64 {
    -side.level(&source.current, at)
}

/// Ideal current source. `current` leaves at `p` into the circuit and returns at `n`, so a
/// source that delivers power has negative `i` and negative `p`, like a voltage source.
/// It has no DC path.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentSource {
    /// Nets of pins `p`, `n`.
    pub pins: [NetId; 2],
    /// Ampere. Any finite value.
    pub current: Waveform,
}

impl CurrentSource {
    pub const PINS: [&'static str; 2] = ["p", "n"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireCurrentSource {
    pub(crate) name: String,
    pins: PinMap,
    current: WireWaveform,
}

pub(crate) fn resolve_current_source(
    name: &Name,
    part_ref: &Ref,
    wire: WireCurrentSource,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Model> {
    let WireCurrentSource { pins, current, .. } = wire;

    let pins = resolve_pins(name, CurrentSource::PINS, &pins.0, nets, out);
    let current = resolve_waveform(part_ref, &CURRENT, current, out);
    match (pins, current) {
        (Some(pins), Some(current)) => Some(Model::CurrentSource(CurrentSource { pins, current })),
        _ => None,
    }
}
