//! Nets are ground first, then the rest by name. Parts are by name.

use std::f64::consts::SQRT_2;

pub mod bounce;
mod json;
mod waveform;

use serde::{Deserialize, Serialize};

use crate::diagnostic::Diagnostic;
use crate::finite::Finite;
use crate::name::{Name, Quantity};
use crate::time::Time;

pub use waveform::{Pulse, Shape, Step, Waveform};

/// The netlist `version` this engine reads.
pub const FORMAT_VERSION: u64 = 0;

/// Parse and validate netlist.json bytes.
///
/// Schema errors stop early. Name, pin, unit, and value errors are collected
/// together.
pub fn parse(json: &[u8]) -> Result<Netlist, Vec<Diagnostic>> {
    json::parse(json)
}

#[derive(Debug)]
pub struct Netlist {
    seed: u64,
    analysis: Analysis,
    nets: Vec<Name>,
    parts: Vec<Part>,
}

impl Netlist {
    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }

    /// All nets in canonical order, ground first.
    pub fn nets(&self) -> impl ExactSizeIterator<Item = (NetId, &Name)> {
        self.nets
            .iter()
            .enumerate()
            .map(|(i, n)| (NetId(i as u32), n))
    }

    pub fn net_name(&self, id: NetId) -> &Name {
        &self.nets[id.index()]
    }

    /// All parts in canonical order.
    pub fn parts(&self) -> impl ExactSizeIterator<Item = (PartId, &Part)> {
        self.parts
            .iter()
            .enumerate()
            .map(|(i, p)| (PartId(i as u32), p))
    }

    pub fn part(&self, id: PartId) -> &Part {
        &self.parts[id.index()]
    }
}

/// A net's position in the netlist. Only a `Netlist` hands these out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetId(u32);

impl NetId {
    pub const GROUND: NetId = NetId(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn is_ground(self) -> bool {
        self == NetId::GROUND
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartId(u32);

impl PartId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What the run computes.
#[derive(Clone, Debug, PartialEq)]
pub enum Analysis {
    /// DC operating point.
    Op,
    /// The operating point at t = 0, then time steps to `stop`.
    Transient(Transient),
}

/// Parse enforces `stop >= max_step >= MIN_MAX_STEP_PS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transient {
    pub stop: Time,
    /// Picoseconds. No step is longer.
    pub max_step: u64,
}

impl Transient {
    /// The engine's first step after a contact edge or a source jump, which `max_step` may
    /// not undercut.
    pub const MIN_MAX_STEP_PS: u64 = 1_000;
}

/// One package, modelled at its pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: Name,
    pub model: Model,
}

/// The closed set of part kinds.
/// Pin order is that kind's `PINS` const.
#[derive(Clone, Debug, PartialEq)]
pub enum Model {
    Resistor(Resistor),
    VoltageSource(VoltageSource),
    CurrentSource(CurrentSource),
    Diode(Junction),
    /// The same device equation as `Diode`. Its own kind, so an LED-only quantity needs no flag.
    Led(Junction),
    Capacitor(Capacitor),
    Switch(Switch),
    /// A package modelled as a behavioural black box. A new chip kind is a `Chip` variant,
    /// not a `Model` variant.
    Chip(Chip),
}

impl Model {
    /// Pin names in the order of `pins()`.
    pub fn pin_names(&self) -> &'static [&'static str] {
        match self {
            Model::Resistor(_) => &Resistor::PINS,
            Model::VoltageSource(_) => &VoltageSource::PINS,
            Model::CurrentSource(_) => &CurrentSource::PINS,
            Model::Diode(_) | Model::Led(_) => &Junction::PINS,
            Model::Capacitor(_) => &Capacitor::PINS,
            Model::Switch(_) => &Switch::PINS,
            Model::Chip(Chip::Ne555(_)) => &Ne555::PINS,
        }
    }

    pub fn pins(&self) -> &[NetId] {
        match self {
            Model::Resistor(r) => &r.pins,
            Model::VoltageSource(v) => &v.pins,
            Model::CurrentSource(c) => &c.pins,
            Model::Diode(j) | Model::Led(j) => &j.pins,
            Model::Capacitor(c) => &c.pins,
            Model::Switch(s) => &s.pins,
            Model::Chip(Chip::Ne555(n)) => &n.pins,
        }
    }

    /// Rated quantities in `RatingKind` order. A `null` rating is left out.
    pub fn ratings(&self) -> Vec<(RatingKind, &Value)> {
        let (current, power) = match self {
            Model::Resistor(r) => (None, r.max_power.as_ref()),
            Model::VoltageSource(_)
            | Model::CurrentSource(_)
            | Model::Capacitor(_)
            | Model::Switch(_)
            | Model::Chip(_) => (None, None),
            Model::Diode(j) | Model::Led(j) => (j.max_current.as_ref(), j.max_power.as_ref()),
        };
        let current = current.map(|v| (RatingKind::Current, v));
        let power = power.map(|v| (RatingKind::Power, v));
        current.into_iter().chain(power).collect()
    }

    /// The source's value over time. `None` for every kind that is not an independent source.
    pub fn waveform(&self) -> Option<&Waveform> {
        match self {
            Model::VoltageSource(source) => Some(&source.voltage),
            Model::CurrentSource(source) => Some(&source.current),
            Model::Resistor(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Capacitor(_)
            | Model::Switch(_)
            | Model::Chip(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resistor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Ohm, greater than 0.
    pub resistance: Value,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl Resistor {
    pub const PINS: [&'static str; 2] = ["a", "b"];
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

/// An ideal capacitor.
#[derive(Clone, Debug, PartialEq)]
pub struct Capacitor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Farad, greater than 0.
    pub capacitance: Value,
    /// Volt, any finite value. The operating point holds `V(a) - V(b)` here and solves the
    /// current through it. `None` is an explicit `"initial": null`, open at the operating point.
    pub initial: Option<Value>,
}

impl Capacitor {
    pub const PINS: [&'static str; 2] = ["a", "b"];
}

/// A two-terminal contact: `on_resistance` closed, `off_resistance` open.
#[derive(Clone, Debug, PartialEq)]
pub struct Switch {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Ohm, greater than 0.
    pub on_resistance: Value,
    /// Ohm, greater than 0.
    pub off_resistance: Value,
    /// The netlist's own events, without bounce.
    pub events: Vec<Edge>,
    pub bounce: Option<Bounce>,
    /// `events` with their bounce expanded.
    pub schedule: Schedule,
}

impl Switch {
    pub const PINS: [&'static str; 2] = ["a", "b"];

    pub fn resistance(&self, contact: Contact) -> &Value {
        match contact {
            Contact::Open => &self.off_resistance,
            Contact::Closed => &self.on_resistance,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contact {
    Open,
    Closed,
}

impl Contact {
    pub fn flipped(self) -> Contact {
        match self {
            Contact::Open => Contact::Closed,
            Contact::Closed => Contact::Open,
        }
    }
}

/// The contact becomes `to` at `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Edge {
    #[serde(rename = "t_ps")]
    pub at: Time,
    #[serde(rename = "state")]
    pub to: Contact,
}

/// A contact over time. Only parse builds one, so every edge is after t = 0,
/// edges are at least `MIN_GAP_PS` apart, and each flips the contact.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    initial: Contact,
    edges: Vec<Edge>,
}

impl Schedule {
    /// The least time between two edges of one switch.
    pub const MIN_GAP_PS: u64 = 2_000;

    pub fn initial(&self) -> Contact {
        self.initial
    }

    /// Nominal and bounce edges, ascending.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The contact on the interval that ends at `at`: after every edge strictly
    /// before `at`. `initial` at t = 0.
    pub fn before(&self, at: Time) -> Contact {
        let k = self.edges.partition_point(|edge| edge.at < at);
        if k == 0 {
            self.initial
        } else {
            self.edges[k - 1].to
        }
    }

    /// The contact on the interval that starts at `at`: after every edge at or before `at`.
    pub fn after(&self, at: Time) -> Contact {
        let k = self.edges.partition_point(|edge| edge.at <= at);
        if k == 0 {
            self.initial
        } else {
            self.edges[k - 1].to
        }
    }
}

/// The closed set of chip kinds. A chip's internals are model detail, with no refs and no
/// trace columns.
#[derive(Clone, Debug, PartialEq)]
pub enum Chip {
    Ne555(Box<Ne555>),
}

/// An NE555 timer, behavioural. Pins in package order, TI SLFS022K's pin functions.
///
/// Three equal resistors run VCC to CONT to a lower node to GND. THRES compares with CONT,
/// TRIG with the lower node, and RESET with GND plus `reset_threshold`. The latch follows
/// SLFS022K Table 6-1: RESET low resets it, then TRIG low sets it, then THRES high resets it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ne555 {
    pub pins: [NetId; 8],
    /// Ohm. Each of the three ladder resistors.
    pub ladder_resistance: Value,
    /// Volt, greater than 0. RESET more than this above GND lets the latch run.
    pub reset_threshold: Value,
    pub discharge: Discharge,
    /// Ohm. VCC to GND, the supply current the ladder does not draw.
    pub supply_resistance: Value,
    pub output: OutputStage,
}

impl Ne555 {
    pub const PINS: [&'static str; 8] = [
        "gnd", "trig", "out", "reset", "cont", "thres", "disch", "vcc",
    ];
    pub const GND: usize = 0;
    pub const TRIG: usize = 1;
    pub const OUT: usize = 2;
    pub const RESET: usize = 3;
    pub const CONT: usize = 4;
    pub const THRES: usize = 5;
    pub const DISCH: usize = 6;
    pub const VCC: usize = 7;
}

/// DISCH to GND: `on_resistance` while the latch is reset, `off_resistance` while it is set.
#[derive(Clone, Debug, PartialEq)]
pub struct Discharge {
    /// Ohm.
    pub on_resistance: Value,
    /// Ohm.
    pub off_resistance: Value,
}

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

/// Seeded contact bounce after each event. The expansion is `bounce::expand`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bounce {
    /// Picoseconds. Every bounce edge lands within this of its event.
    pub window: IntValue,
    /// Each event gets 1 to this many extra open and close pairs. 1 to 255.
    pub max_bounces: IntValue,
}

/// An integer quantity with where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct IntValue {
    pub value: u64,
    pub source: Source,
}

/// A Shockley junction in series with a resistance, at 298.15 K with no temperature scaling.
/// `i = Is * (exp(vj / (N * kT/q)) - 1)` and `V(anode) - V(cathode) = vj + i * Rs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Junction {
    /// Nets of pins `anode`, `cathode`.
    pub pins: [NetId; 2],
    /// Ampere, greater than 0.
    pub saturation_current: Value,
    /// Unit `1`, greater than 0.
    pub emission_coefficient: Value,
    /// Ohm, greater than 0.
    pub series_resistance: Value,
    /// Ampere, greater than 0. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl Junction {
    pub const PINS: [&'static str; 2] = ["anode", "cathode"];

    /// `k*T/q` at 298.15 K, from the exact SI values of `k` and `q`.
    pub const VT: f64 = 1.380649e-23 * 298.15 / 1.602176634e-19;
}

/// `n*VT` and `vcrit` for a junction whose parameters are inside the domain.
pub struct JunctionScale {
    pub nvt: f64,
    pub vcrit: f64,
}

/// Which junction field breaks `finite_junction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JunctionFault {
    EmissionCoefficient,
    SaturationCurrent,
}

impl JunctionFault {
    pub fn field(self) -> &'static str {
        match self {
            JunctionFault::EmissionCoefficient => "emission_coefficient",
            JunctionFault::SaturationCurrent => "saturation_current",
        }
    }

    fn value(self, emission: f64, saturation: f64) -> f64 {
        match self {
            JunctionFault::EmissionCoefficient => emission,
            JunctionFault::SaturationCurrent => saturation,
        }
    }
}

/// `n*VT` must be normal, and `vcrit = nvt*ln(nvt/(sqrt(2)*Is))` must be finite and positive.
///
/// `vcrit` is positive when `n*VT > sqrt(2)*Is`. A `vcrit` that fails the rule names
/// `saturation_current` when `Is` is not normal, and `emission_coefficient` otherwise.
/// `n*VT` itself names `emission_coefficient`.
pub fn junction_scale(emission: f64, saturation: f64) -> Result<JunctionScale, JunctionFault> {
    let nvt = emission * Junction::VT;
    if !nvt.is_normal() {
        return Err(JunctionFault::EmissionCoefficient);
    }
    let vcrit = nvt * (nvt / (SQRT_2 * saturation)).ln();
    if !(vcrit.is_finite() && vcrit > 0.0) {
        return Err(if saturation.is_normal() {
            JunctionFault::EmissionCoefficient
        } else {
            JunctionFault::SaturationCurrent
        });
    }
    Ok(JunctionScale { nvt, vcrit })
}

/// What a rating bounds. Each maps to one quantity and one diagnostic code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RatingKind {
    Current,
    Power,
}

impl RatingKind {
    pub fn quantity(self) -> Quantity {
        match self {
            RatingKind::Current => Quantity::I,
            RatingKind::Power => Quantity::P,
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            RatingKind::Current => "A",
            RatingKind::Power => "W",
        }
    }
}

/// A number in SI base units, with where it came from.
/// The JSON `unit` was checked against the field and is not kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    pub si: Finite,
    pub source: Source,
}

/// Provenance of a value. The solver does not compute with it.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// Chosen by whoever wrote the scenario.
    User,
    /// Derived by a formula. `note` says which.
    ClosedForm { note: String },
    /// Read from a datasheet. `at` locates the cell, row, or figure.
    /// `typical` marks a value taken from a typical-only curve.
    Datasheet {
        doc: String,
        at: String,
        typical: bool,
    },
}
