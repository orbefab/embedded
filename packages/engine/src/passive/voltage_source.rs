use orbe_model::Time;
use serde::Deserialize;

use crate::diagnostic::Diagnostic;
use crate::element::{Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::name::{Name, Ref};
use crate::netlist::json::waveform::{VOLT, VOLTAGE, WireWaveform, resolve_waveform};
use crate::netlist::json::{ParseCtx, PinMap, WireValue, resolve_pins, resolve_rating};
use crate::netlist::{
    Conduction, Kind, LevelField, NetId, Schedule, Serial, SerialField, Value, Waveform,
};
use crate::newton::Side;

/// Ideal voltage source. V(p) - V(n) = voltage. Only parse builds one, so `p` and `n` are on two
/// nets.
#[derive(Clone, Debug, PartialEq)]
pub struct VoltageSource {
    /// Nets of pins `p`, `n`.
    pub pins: [NetId; 2],
    /// Volt. Any finite value.
    pub voltage: Waveform,
    /// Ampere, greater than 0: the most the supply delivers, which bounds `i`. `None` when the
    /// part has no `ratings` object or its `current` is `null`.
    pub max_current: Option<Value>,
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
    /// Optional, so a source written before sources had ratings keeps its meaning.
    #[serde(default)]
    ratings: Option<WireVoltageSourceRatings>,
}

/// The key is required, and `null` leaves the source unrated.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireVoltageSourceRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    current: Option<WireValue>,
}

pub(crate) fn resolve_voltage_source(
    ctx: ParseCtx<'_>,
    wire: WireVoltageSource,
) -> Option<VoltageSource> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let WireVoltageSource {
        pins,
        voltage,
        ratings,
        ..
    } = wire;

    let pins = resolve_pins(name, VoltageSource::PINS, &pins.0, nets, out).and_then(|pins| {
        if pins[0] != pins[1] {
            return Some(pins);
        }
        // Both pins on one net make the source's branch row zero, which no solve can take.
        let (net, _) = nets
            .iter()
            .find(|&(_, &id)| id == pins[0])
            .expect("a resolved pin's net is named");
        out.push(Diagnostic::NetShort {
            part: part_ref.clone(),
            net: Ref::Net(Name::new(net).expect("a resolved net name")),
        });
        None
    });
    let voltage = resolve_waveform(part_ref, &VOLTAGE, voltage, out);
    let max_current = match ratings {
        None => Some(None),
        Some(ratings) => resolve_rating(part_ref, "ratings.current", "A", ratings.current, out),
    };
    match (pins, voltage, max_current) {
        (Some(pins), Some(voltage), Some(max_current)) => Some(VoltageSource {
            pins,
            voltage,
            max_current,
        }),
        _ => None,
    }
}

impl Kind for VoltageSource {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let volts = stamp.conditions.level(&self.voltage);
        stamp.voltage_source(self.pins[0], self.pins[1], 0, volts);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        (self.max_current.as_ref().map(|v| (Quantity::I, v)), None)
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(Observed::two_pin(self.pins, probe.branch(0), probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Fixes
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.voltage.next_change_after(at)
    }
    fn level(&self) -> Option<LevelField<'_>> {
        let schedule = self.voltage.as_steps()?;
        Some(LevelField {
            schedule,
            field: "voltage",
            unit: VOLT,
        })
    }
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        self.voltage.steps_mut()
    }
    fn serial(&self) -> Option<SerialField<'_>> {
        let waveform = self.voltage.as_serial()?;
        Some(SerialField {
            waveform,
            field: "voltage",
        })
    }
    fn serial_mut(&mut self) -> Option<&mut Serial> {
        self.voltage.serial_mut()
    }
}
