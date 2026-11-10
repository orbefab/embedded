use crate::diagnostic::Diagnostic;
use crate::name::Name;
use crate::name::Ref;
use crate::netlist::Model;
use crate::netlist::NetId;
use crate::netlist::Waveform;
use crate::netlist::json::PinMap;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::waveform::VOLTAGE;
use crate::netlist::json::waveform::WireWaveform;
use crate::netlist::json::waveform::resolve_waveform;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::mna::{Probe, Stamp};

pub(crate) fn stamp(source: &VoltageSource, stamp: &mut Stamp<'_>) {
    let volts = stamp.conditions.level(&source.voltage);
    stamp.voltage_source(source.pins[0], source.pins[1], 0, volts);
}

pub(crate) fn current(probe: &Probe<'_>) -> f64 {
    probe.branch(0)
}

/// Ideal voltage source. V(p) - V(n) = voltage.
#[derive(Clone, Debug, PartialEq)]
pub struct VoltageSource {
    /// Nets of pins `p`, `n`.
    pub pins: [NetId; 2],
    /// Volt. Any finite value.
    pub voltage: Waveform,
}

impl VoltageSource {
    pub const PINS: [&'static str; 2] = ["p", "n"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireVoltageSource {
    pub(crate) name: String,
    pins: PinMap,
    voltage: WireWaveform,
}

pub(crate) fn resolve_voltage_source(
    name: &Name,
    part_ref: &Ref,
    wire: WireVoltageSource,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Model> {
    let WireVoltageSource { pins, voltage, .. } = wire;

    let pins = resolve_pins(name, VoltageSource::PINS, &pins.0, nets, out);
    let voltage = resolve_waveform(part_ref, &VOLTAGE, voltage, out);
    match (pins, voltage) {
        (Some(pins), Some(voltage)) => Some(Model::VoltageSource(VoltageSource { pins, voltage })),
        _ => None,
    }
}
