use orbe_model::Time;
use serde::Deserialize;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::waveform::{AMPERE, CURRENT, WireWaveform, resolve_waveform};
use crate::netlist::json::{ParseCtx, PinMap, resolve_pins};
use crate::netlist::{
    Conduction, Kind, LevelField, NetId, Schedule, Serial, SerialField, Value, Waveform,
};
use crate::newton::Side;

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
    ctx: ParseCtx<'_>,
    wire: WireCurrentSource,
) -> Option<CurrentSource> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let WireCurrentSource { pins, current, .. } = wire;

    let pins = resolve_pins(name, CurrentSource::PINS, &pins.0, nets, out);
    let current = resolve_waveform(part_ref, &CURRENT, current, out);
    match (pins, current) {
        (Some(pins), Some(current)) => Some(CurrentSource { pins, current }),
        _ => None,
    }
}

impl Kind for CurrentSource {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns::default()
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    /// The current leaves at `p`, so it flows through the part from `n` to `p`.
    /// `Stamp::current` does not scale, so the stamp applies `source_scale` itself.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let amps = stamp.conditions.level(&self.current) * stamp.conditions.source_scale;
        stamp.current(self.pins[1], self.pins[0], amps);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[]
    }
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        (None, None)
    }
    /// The current into pin `p`.
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed> {
        let i = -side.level(&self.current, at);
        Some(Observed::two_pin(self.pins, i, probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Other
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.current.next_change_after(at)
    }
    fn level(&self) -> Option<LevelField<'_>> {
        let schedule = self.current.as_steps()?;
        Some(LevelField {
            schedule,
            field: "current",
            unit: AMPERE,
        })
    }
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        self.current.steps_mut()
    }
    fn serial(&self) -> Option<SerialField<'_>> {
        let waveform = self.current.as_serial()?;
        Some(SerialField {
            waveform,
            field: "current",
        })
    }
    fn serial_mut(&mut self) -> Option<&mut Serial> {
        self.current.serial_mut()
    }
}
