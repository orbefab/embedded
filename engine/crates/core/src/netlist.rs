//! Nets are ground first, then the rest by name. Parts are by name.

use std::f64::consts::SQRT_2;
use std::sync::Arc;

pub mod bounce;
mod document;
pub mod elf;
pub mod flash;
mod json;
mod waveform;

use serde::{Deserialize, Serialize, Serializer};

use crate::blob::{Blob, Blobs, Sha256};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::finite::Finite;
use crate::link::{Cs, Lanes, SpiPhase};
use crate::name::{Name, Quantity};
use crate::time::Time;

pub use document::Document;
pub use waveform::{Baud, Frame, Pulse, Serial, SerialEvent, SerialSend, Shape, Step, Waveform};

/// The netlist `version`s this engine reads, ascending. See docs/versions.md.
pub const VERSIONS: &[u64] = &[0];

/// Parse and validate netlist.json bytes.
///
/// Schema errors stop early. Name, pin, unit, and value errors are collected
/// together.
pub fn parse(json: &[u8]) -> Result<Netlist, Vec<Diagnostic>> {
    json::parse(json, &Blobs::new())
}

/// [`parse`], finding each blob the netlist names in `blobs`. A blob it names and `blobs` lacks
/// is a `netlist.blob` error, as is one whose bytes do not hash to its name or whose length
/// differs from the netlist's.
pub fn parse_with(json: &[u8], blobs: &Blobs) -> Result<Netlist, Vec<Diagnostic>> {
    json::parse(json, blobs)
}

/// The SHA-256 of each blob the document names, in part order, so a caller can look up only
/// those. Empty when the document does not parse, which [`parse_with`] then reports.
pub fn blob_names(json: &[u8]) -> Vec<Sha256> {
    json::blob_names(json)
}

/// The document's `version`, when it is readable as an unsigned integer, whether or not
/// the engine reads that version or the rest of the document parses.
pub fn probe_version(json: &[u8]) -> Option<u64> {
    json::probe_version(json)
}

#[derive(Clone, Debug)]
pub struct Netlist {
    version: u64,
    seed: u64,
    analysis: Analysis,
    nets: Vec<Name>,
    parts: Vec<Part>,
}

impl Netlist {
    /// The `version` the document resolved under.
    pub fn version(&self) -> u64 {
        self.version
    }

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

    /// Every blob the netlist names: parts in canonical order, and each part's in its field
    /// order.
    pub fn blobs(&self) -> impl Iterator<Item = &Blob> {
        self.parts.iter().flat_map(|part| part.model.blobs())
    }

    pub fn part_id(&self, name: &Name) -> Option<PartId> {
        let k = self
            .parts
            .binary_search_by(|part| part.name.cmp(name))
            .ok()?;
        Some(PartId(k as u32))
    }

    /// Appends one event to part `id` at `at`. A contact goes on a switch, with the bounce
    /// parse draws for an event at that index. A level goes on a `steps` source or a
    /// potentiometer. A serial send goes on a `serial` source. For a contact or a level,
    /// `at` is a time `Schedule::earliest` returned for that part, and the event changes
    /// the value in force. A serial send does not use that helper. Panics when the part
    /// cannot take that event.
    pub fn append(&mut self, id: PartId, at: Time, to: Appended) {
        match to {
            Appended::Contact(to) => self.append_contact(id, at, to),
            Appended::Level(level) => self.append_level(id, at, level),
            Appended::Serial(send) => self.append_serial(id, at, send),
            Appended::Touch(touch) => self.append_touch(id, at, touch),
        }
    }

    fn append_contact(&mut self, id: PartId, at: Time, to: Contact) {
        let seed = self.seed;
        let part = &mut self.parts[id.index()];
        let Model::Switch(switch) = &mut part.model else {
            panic!("{} is not a switch", part.name.as_str());
        };
        let edge = Edge { at, to };
        debug_assert_eq!(switch.earliest(edge.at), Some(edge.at));
        debug_assert_ne!(*switch.schedule.after(edge.at), edge.to);
        let index = switch.events.events().len() as u64;
        switch.events.events.push(edge);
        let bounce = switch.bounce.as_ref();
        let edges = bounce::expand_event(seed, &part.name, index, edge, bounce);
        switch.schedule.events.extend(edges);
    }

    fn append_level(&mut self, id: PartId, at: Time, level: Finite) {
        let part = &mut self.parts[id.index()];
        let Some(schedule) = part.model.level_mut() else {
            panic!("{} takes no level", part.name.as_str());
        };
        schedule.append_level(at, level);
    }

    fn append_serial(&mut self, id: PartId, at: Time, send: SerialSend) {
        let part = &mut self.parts[id.index()];
        let Some(serial) = part.model.serial_mut() else {
            panic!("{} takes no serial event", part.name.as_str());
        };
        serial.push(at, send);
    }

    fn append_touch(&mut self, id: PartId, at: Time, touch: Touch) {
        let part = &mut self.parts[id.index()];
        let Model::Chip(Chip::Cst816(dev)) = &mut part.model else {
            panic!("{} takes no touch", part.name.as_str());
        };
        debug_assert_eq!(dev.earliest(at), Some(at));
        debug_assert!(touch.x <= Cst816::COORD_MAX && touch.y <= Cst816::COORD_MAX);
        dev.touches.push(Event { at, to: touch });
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

/// Parse enforces `stop >= max_step >= MIN_MAX_STEP_PS` and `stop >= grid >= MIN_GRID_PS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transient {
    pub stop: Time,
    /// Picoseconds. No step is longer.
    pub max_step: u64,
    /// Picoseconds between trace rows. A row is a probe, so the grid never moves a step.
    pub grid: u64,
}

impl Transient {
    /// The engine's first step of a segment, which `max_step` may not undercut.
    pub const MIN_MAX_STEP_PS: u64 = 1_000;
    /// The finest grid. A smaller minimum later keeps the netlist version.
    pub const MIN_GRID_PS: u64 = 1_000;

    /// Rows a run that reaches `stop` writes: t = 0, every multiple of `grid` before `stop`,
    /// and `stop`.
    pub fn rows(&self) -> u64 {
        self.stop.as_ps().div_ceil(self.grid) + 1
    }

    /// Whether a run writes a row at `at`, which is at most `stop`.
    pub fn is_row(&self, at: Time) -> bool {
        at.as_ps().is_multiple_of(self.grid) || at == self.stop
    }

    /// The first row time after `now`, which is before `stop`.
    pub fn next_row_after(&self, now: Time) -> Time {
        let next = (now.as_ps() / self.grid + 1).saturating_mul(self.grid);
        Time::from_ps(next.min(self.stop.as_ps()))
    }
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
    Potentiometer(Potentiometer),
    VoltageSource(VoltageSource),
    CurrentSource(CurrentSource),
    Diode(Junction),
    /// The same device equation as `Diode`. Its own kind, so an LED-only quantity needs no flag.
    Led(Junction),
    /// A forward junction and a breakdown junction. See [`Zener`].
    Zener(Zener),
    /// A static linear regulator. See [`LinearRegulator`].
    LinearRegulator(LinearRegulator),
    Npn(Bjt),
    /// The same device equations as `Npn`, with every junction voltage and pin current negated.
    Pnp(Bjt),
    /// An N-channel MOSFET. Level 1, bulk tied to the source.
    Nmos(Mosfet),
    Capacitor(Capacitor),
    Inductor(Inductor),
    /// A coil and a contact that follows the coil current. Passive, with one state.
    Relay(Relay),
    Switch(Switch),
    /// A package modelled as a behavioural black box. A new chip kind is a `Chip` variant,
    /// not a `Model` variant.
    Chip(Chip),
}

impl Model {
    /// Pin names in the order of `pins()`. An `esp32s3` lists only the pins it has nets on.
    pub fn pin_names(&self) -> &[&'static str] {
        match self {
            Model::Resistor(_) => &Resistor::PINS,
            Model::Potentiometer(_) => &Potentiometer::PINS,
            Model::VoltageSource(_) => &VoltageSource::PINS,
            Model::CurrentSource(_) => &CurrentSource::PINS,
            Model::Diode(_) | Model::Led(_) | Model::Zener(_) => &Junction::PINS,
            Model::LinearRegulator(_) => &LinearRegulator::PINS,
            Model::Npn(_) | Model::Pnp(_) => &Bjt::PINS,
            Model::Nmos(_) => &Mosfet::PINS,
            Model::Capacitor(_) => &Capacitor::PINS,
            Model::Inductor(_) => &Inductor::PINS,
            Model::Relay(_) => &Relay::PINS,
            Model::Switch(_) => &Switch::PINS,
            Model::Chip(Chip::Ne555(_)) => &Ne555::PINS,
            Model::Chip(Chip::Atmega328p(_)) => &Atmega328p::PINS,
            Model::Chip(Chip::SpiReplay(_)) => &SpiReplay::PINS,
            Model::Chip(Chip::St77916(_)) => &St77916::PINS,
            Model::Chip(Chip::Cst816(_)) => &Cst816::PINS,
            Model::Chip(Chip::Esp32s3(mcu)) => &mcu.names,
            Model::Chip(Chip::Ws2812(_)) => &Ws2812::PINS,
            Model::Chip(Chip::PulsesReplay(_)) => &PulsesReplay::PINS,
        }
    }

    /// The quantities a run observes on this part, in `Quantity` order. A two-pin part has
    /// `i`, `v` and `p`, and so does a relay, of its coil. A BJT has `ib`, `ic` and `p`. An `nmos`
    /// has `id` and `p`. A linear regulator has `iin`, `iout` and `p`. A chip and a potentiometer
    /// have none.
    pub fn quantities(&self) -> &'static [Quantity] {
        match self {
            Model::Resistor(_)
            | Model::VoltageSource(_)
            | Model::CurrentSource(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Zener(_)
            | Model::Capacitor(_)
            | Model::Inductor(_)
            | Model::Relay(_)
            | Model::Switch(_) => &[Quantity::I, Quantity::V, Quantity::P],
            Model::LinearRegulator(_) => &[Quantity::Iin, Quantity::Iout, Quantity::P],
            Model::Npn(_) | Model::Pnp(_) => &[Quantity::Ib, Quantity::Ic, Quantity::P],
            Model::Nmos(_) => &[Quantity::Id, Quantity::P],
            Model::Potentiometer(_) | Model::Chip(_) => &[],
        }
    }

    /// The quantity a rating of `kind` bounds by magnitude. A current rating bounds `iout` on a
    /// linear regulator, `ic` on a BJT, `id` on an `nmos`, and `i` on every other part.
    pub fn rated(&self, kind: RatingKind) -> Quantity {
        match (kind, self) {
            (RatingKind::Current, Model::Npn(_) | Model::Pnp(_)) => Quantity::Ic,
            (RatingKind::Current, Model::Nmos(_)) => Quantity::Id,
            (RatingKind::Current, Model::LinearRegulator(_)) => Quantity::Iout,
            (RatingKind::Current, _) => Quantity::I,
            (RatingKind::Power, _) => Quantity::P,
        }
    }

    pub fn pins(&self) -> &[NetId] {
        match self {
            Model::Resistor(r) => &r.pins,
            Model::Potentiometer(p) => &p.pins,
            Model::VoltageSource(v) => &v.pins,
            Model::CurrentSource(c) => &c.pins,
            Model::Diode(j) | Model::Led(j) => &j.pins,
            Model::Zener(z) => &z.pins,
            Model::LinearRegulator(r) => &r.pins,
            Model::Npn(b) | Model::Pnp(b) => &b.pins,
            Model::Nmos(m) => &m.pins,
            Model::Capacitor(c) => &c.pins,
            Model::Inductor(l) => &l.pins,
            Model::Relay(k) => &k.pins,
            Model::Switch(s) => &s.pins,
            Model::Chip(Chip::Ne555(n)) => &n.pins,
            Model::Chip(Chip::Atmega328p(m)) => &m.pins,
            Model::Chip(Chip::SpiReplay(r)) => &r.pins,
            Model::Chip(Chip::St77916(p)) => &p.pins,
            Model::Chip(Chip::Cst816(dev)) => &dev.pins,
            Model::Chip(Chip::Ws2812(chain)) => &chain.pins,
            Model::Chip(Chip::PulsesReplay(replay)) => &replay.pins,
            Model::Chip(Chip::Esp32s3(mcu)) => &mcu.pins,
        }
    }

    /// The blobs it names, in its own field order.
    pub fn blobs(&self) -> Vec<&Blob> {
        match self {
            Model::Chip(Chip::Esp32s3(mcu)) => mcu.firmware.blobs(),
            _ => Vec::new(),
        }
    }

    /// Rated quantities in `RatingKind` order. A `null` rating is left out.
    pub fn ratings(&self) -> Vec<(RatingKind, &Value)> {
        let (current, power) = match self {
            Model::Resistor(r) => (None, r.max_power.as_ref()),
            Model::Potentiometer(_)
            | Model::VoltageSource(_)
            | Model::CurrentSource(_)
            | Model::Capacitor(_)
            | Model::Inductor(_)
            | Model::Relay(_)
            | Model::Switch(_)
            | Model::Chip(_) => (None, None),
            Model::Diode(j) | Model::Led(j) => (j.max_current.as_ref(), j.max_power.as_ref()),
            Model::Zener(z) => (z.max_current.as_ref(), z.max_power.as_ref()),
            Model::LinearRegulator(r) => (r.max_current.as_ref(), r.max_power.as_ref()),
            Model::Npn(b) | Model::Pnp(b) => (b.max_current.as_ref(), b.max_power.as_ref()),
            Model::Nmos(m) => (m.max_current.as_ref(), m.max_power.as_ref()),
        };
        let current = current.map(|v| (RatingKind::Current, v));
        let power = power.map(|v| (RatingKind::Power, v));
        current.into_iter().chain(power).collect()
    }

    /// The next contact edge, source jump, potentiometer move, or touch strictly after `at`.
    pub fn next_change_after(&self, at: Time) -> Option<Time> {
        match self {
            Model::VoltageSource(source) => source.voltage.next_change_after(at),
            Model::CurrentSource(source) => source.current.next_change_after(at),
            Model::Switch(switch) => switch.schedule.next_change_after(at),
            Model::Potentiometer(pot) => pot.position.next_change_after(at),
            Model::Chip(Chip::Cst816(dev)) => dev.next_touch_after(at),
            Model::Resistor(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Zener(_)
            | Model::LinearRegulator(_)
            | Model::Npn(_)
            | Model::Pnp(_)
            | Model::Nmos(_)
            | Model::Capacitor(_)
            | Model::Inductor(_)
            | Model::Relay(_)
            | Model::Chip(_) => None,
        }
    }

    /// The settable level schedule, with the document field that holds it and that field's unit.
    /// A `steps` voltage, a `steps` current, or a potentiometer position. A `step`, `pulse`, or
    /// `serial` source has none.
    pub fn level(&self) -> Option<LevelField<'_>> {
        match self {
            Model::VoltageSource(source) => {
                level_field(source.voltage.as_steps(), "voltage", json::VOLT)
            }
            Model::CurrentSource(source) => {
                level_field(source.current.as_steps(), "current", json::AMPERE)
            }
            Model::Potentiometer(pot) => {
                level_field(Some(&pot.position), "position", json::FRACTION_UNIT)
            }
            Model::Resistor(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Zener(_)
            | Model::LinearRegulator(_)
            | Model::Npn(_)
            | Model::Pnp(_)
            | Model::Nmos(_)
            | Model::Capacitor(_)
            | Model::Inductor(_)
            | Model::Relay(_)
            | Model::Switch(_)
            | Model::Chip(_) => None,
        }
    }

    /// [`level`](Self::level)'s schedule, for appending one event.
    pub fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        match self {
            Model::VoltageSource(source) => source.voltage.steps_mut(),
            Model::CurrentSource(source) => source.current.steps_mut(),
            Model::Potentiometer(pot) => Some(&mut pot.position),
            Model::Resistor(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Zener(_)
            | Model::LinearRegulator(_)
            | Model::Npn(_)
            | Model::Pnp(_)
            | Model::Nmos(_)
            | Model::Capacitor(_)
            | Model::Inductor(_)
            | Model::Relay(_)
            | Model::Switch(_)
            | Model::Chip(_) => None,
        }
    }

    /// The serial waveform, with the document field that holds it. A `serial` voltage or current.
    pub fn serial(&self) -> Option<SerialField<'_>> {
        match self {
            Model::VoltageSource(source) => serial_field(source.voltage.as_serial(), "voltage"),
            Model::CurrentSource(source) => serial_field(source.current.as_serial(), "current"),
            Model::Resistor(_)
            | Model::Potentiometer(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Zener(_)
            | Model::LinearRegulator(_)
            | Model::Npn(_)
            | Model::Pnp(_)
            | Model::Nmos(_)
            | Model::Capacitor(_)
            | Model::Inductor(_)
            | Model::Relay(_)
            | Model::Switch(_)
            | Model::Chip(_) => None,
        }
    }

    fn serial_mut(&mut self) -> Option<&mut waveform::Serial> {
        match self {
            Model::VoltageSource(source) => source.voltage.serial_mut(),
            Model::CurrentSource(source) => source.current.serial_mut(),
            Model::Resistor(_)
            | Model::Potentiometer(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Zener(_)
            | Model::LinearRegulator(_)
            | Model::Npn(_)
            | Model::Pnp(_)
            | Model::Nmos(_)
            | Model::Capacitor(_)
            | Model::Inductor(_)
            | Model::Relay(_)
            | Model::Switch(_)
            | Model::Chip(_) => None,
        }
    }
}

fn serial_field<'a>(waveform: Option<&'a Serial>, field: &'static str) -> Option<SerialField<'a>> {
    waveform.map(|waveform| SerialField { waveform, field })
}

fn level_field<'a>(
    schedule: Option<&'a Schedule<Value>>,
    field: &'static str,
    unit: &'static str,
) -> Option<LevelField<'a>> {
    schedule.map(|schedule| LevelField {
        schedule,
        field,
        unit,
    })
}

/// A part's settable level: its schedule, the document field, and the unit parse accepts there.
pub struct LevelField<'a> {
    pub schedule: &'a Schedule<Value>,
    pub field: &'static str,
    pub unit: &'static str,
}

/// A part's serial waveform and the document field that holds it.
pub struct SerialField<'a> {
    pub waveform: &'a Serial,
    pub field: &'static str,
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

/// `a` to `wiper` is `position * resistance`. `wiper` to `b` is the rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Potentiometer {
    /// Nets of pins `a`, `wiper`, `b`.
    pub pins: [NetId; 3],
    /// Ohm, greater than 0.
    pub resistance: Value,
    /// Fraction of `resistance` from `a` to the wiper, in `(0, 1)`.
    pub position: Schedule<Value>,
}

impl Potentiometer {
    pub const PINS: [&'static str; 3] = ["a", "wiper", "b"];
}

/// A potentiometer position that breaks a rule, in the order parse reports them.
/// The fraction is outside `(0, 1)`. Otherwise each leg is checked: `position * resistance`,
/// then `(1 - position) * resistance`. A leg that is not positive is [`ValueRule::Positive`].
/// A leg whose reciprocal is not finite is [`ValueRule::FiniteReciprocal`].
pub struct PositionRule {
    kind: PositionKind,
}

enum PositionKind {
    Fraction,
    Legs([(f64, Option<ValueRule>); 2]),
}

impl PositionRule {
    /// The first broken rule. `None` when the position holds.
    pub fn first(self) -> Option<ValueRule> {
        match self.kind {
            PositionKind::Fraction => Some(ValueRule::Fraction),
            PositionKind::Legs(legs) => legs.into_iter().find_map(|(_, rule)| rule),
        }
    }

    /// Both legs when the fraction holds. Each entry is the leg's ohms and the rule it breaks.
    /// `None` when the fraction breaks, because the legs are not checked then.
    pub fn legs(self) -> Option<[(f64, Option<ValueRule>); 2]> {
        match self.kind {
            PositionKind::Fraction => None,
            PositionKind::Legs(legs) => Some(legs),
        }
    }
}

pub fn position_rule(position: f64, resistance: f64) -> PositionRule {
    if broken_fraction(position).is_some() {
        return PositionRule {
            kind: PositionKind::Fraction,
        };
    }
    let legs = [position * resistance, (1.0 - position) * resistance]
        .map(|ohms| (ohms, broken_positive(ohms)));
    PositionRule {
        kind: PositionKind::Legs(legs),
    }
}

pub(crate) fn broken_fraction(number: f64) -> Option<ValueRule> {
    (!(number > 0.0 && number < 1.0)).then_some(ValueRule::Fraction)
}

pub(crate) fn broken_positive(number: f64) -> Option<ValueRule> {
    if number <= 0.0 {
        Some(ValueRule::Positive)
    } else if !(1.0 / number).is_finite() {
        Some(ValueRule::FiniteReciprocal)
    } else {
        None
    }
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

/// An ideal inductance in series with a resistance. `a` reaches the inductance through the
/// resistance. A short behind the resistance at the operating point.
#[derive(Clone, Debug, PartialEq)]
pub struct Inductor {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Henry, greater than 0.
    pub inductance: Value,
    /// Ohm, greater than 0.
    pub series_resistance: Value,
}

impl Inductor {
    pub const PINS: [&'static str; 2] = ["a", "b"];
}

/// A coil and an SPDT contact. The coil is `coil_a` through `coil_resistance`, then
/// `inductance` to `coil_b`, as an `Inductor`. `com` meets `no` through `on_resistance` while
/// the contact is closed, and `nc` while it is open. Each other pair takes `off_resistance`. The
/// contact closes when the coil current's magnitude rises above `pull_in_current` and opens when
/// it falls to `drop_out_current` or below.
#[derive(Clone, Debug, PartialEq)]
pub struct Relay {
    /// Nets of pins `coil_a`, `coil_b`, `com`, `no`, `nc`.
    pub pins: [NetId; 5],
    /// Henry, greater than 0.
    pub inductance: Value,
    /// Ohm, greater than 0.
    pub coil_resistance: Value,
    /// Ampere, greater than 0 and above `drop_out_current`.
    pub pull_in_current: Value,
    /// Ampere, greater than 0.
    pub drop_out_current: Value,
    /// Ohm, greater than 0.
    pub on_resistance: Value,
    /// Ohm, greater than 0.
    pub off_resistance: Value,
}

impl Relay {
    pub const PINS: [&'static str; 5] = ["coil_a", "coil_b", "com", "no", "nc"];
    pub const COIL_A: usize = 0;
    pub const COIL_B: usize = 1;
    pub const COM: usize = 2;
    pub const NO: usize = 3;
    pub const NC: usize = 4;
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
    pub events: Schedule<Contact>,
    pub bounce: Option<Bounce>,
    /// `events` with their bounce expanded.
    pub schedule: Schedule<Contact>,
}

impl Switch {
    pub const PINS: [&'static str; 2] = ["a", "b"];

    pub fn resistance(&self, contact: Contact) -> &Value {
        match contact {
            Contact::Open => &self.off_resistance,
            Contact::Closed => &self.on_resistance,
        }
    }

    /// The first time at or after `from` that one more event passes the schedule rules.
    /// The bounce window is this switch's addition to [`Schedule::earliest`].
    pub fn earliest(&self, from: Time) -> Option<Time> {
        let window = self.bounce.as_ref().map_or(0, |bounce| bounce.window.value);
        self.events.earliest(from, window)
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

/// The value becomes `to` at `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event<T> {
    pub at: Time,
    pub to: T,
}

/// A switch event, serialized as the netlist writes one: `t_ps` and `state`.
pub type Edge = Event<Contact>;

/// One event [`Netlist::append`] adds.
#[derive(Clone, Debug, PartialEq)]
pub enum Appended {
    /// A switch contact.
    Contact(Contact),
    /// A source level or a potentiometer position, in the field's SI unit.
    Level(Finite),
    /// Bytes a `serial` source sends from the appended time.
    Serial(SerialSend),
    /// A point on a `cst816`.
    Touch(Touch),
}

impl Serialize for Edge {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            #[serde(rename = "t_ps")]
            at: Time,
            state: &'a Contact,
        }
        Wire {
            at: self.at,
            state: &self.to,
        }
        .serialize(serializer)
    }
}

/// The least time between two events of one schedule.
pub const MIN_GAP_PS: u64 = 2_000;

/// `initial`, then each event's value from its time on. Only parse and `Netlist::append`
/// build one, so every event is after t = 0, changes the value, and is at least
/// [`MIN_GAP_PS`] after the previous event.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule<T> {
    initial: T,
    events: Vec<Event<T>>,
}

impl<T> Schedule<T> {
    pub fn initial(&self) -> &T {
        &self.initial
    }

    /// Events, ascending.
    pub fn events(&self) -> &[Event<T>] {
        &self.events
    }

    /// The value on the interval that ends at `at`: after every event strictly before `at`.
    /// `initial` at t = 0.
    pub fn before(&self, at: Time) -> &T {
        let k = self.events.partition_point(|event| event.at < at);
        if k == 0 {
            &self.initial
        } else {
            &self.events[k - 1].to
        }
    }

    /// The value after the last event. `initial` when the schedule has none.
    pub fn after_last(&self) -> &T {
        match self.events.last() {
            Some(event) => &event.to,
            None => &self.initial,
        }
    }

    /// The value on the interval that starts at `at`: after every event at or before `at`.
    pub fn after(&self, at: Time) -> &T {
        let k = self.events.partition_point(|event| event.at <= at);
        if k == 0 {
            &self.initial
        } else {
            &self.events[k - 1].to
        }
    }

    /// The next event strictly after `at`.
    pub fn next_change_after(&self, at: Time) -> Option<Time> {
        let k = self.events.partition_point(|event| event.at <= at);
        self.events.get(k).map(|event| event.at)
    }

    /// The first time at or after `from` that one more event passes the schedule rules.
    /// With no event that is 1 ps. After one it is that event plus `window` plus
    /// [`MIN_GAP_PS`]. `None` when that time, or `window` after it, does not fit in a time.
    pub fn earliest(&self, from: Time, window: u64) -> Option<Time> {
        let first = match self.events.last() {
            None => 1,
            Some(last) => last
                .at
                .as_ps()
                .checked_add(window)?
                .checked_add(MIN_GAP_PS)?,
        };
        let at = from.as_ps().max(first);
        at.checked_add(window)?;
        Some(Time::from_ps(at))
    }
}

impl Schedule<Value> {
    fn append_level(&mut self, at: Time, level: Finite) {
        debug_assert_eq!(self.earliest(at, 0), Some(at));
        debug_assert_ne!(self.after(at).si, level);
        self.events.push(Event {
            at,
            to: Value {
                si: level,
                source: Source::User,
            },
        });
    }
}

/// The closed set of chip kinds. A chip's internals are model detail, with no refs and no
/// trace columns.
#[derive(Clone, Debug, PartialEq)]
pub enum Chip {
    Ne555(Box<Ne555>),
    Atmega328p(Box<Atmega328p>),
    SpiReplay(Box<SpiReplay>),
    St77916(Box<St77916>),
    Cst816(Box<Cst816>),
    Ws2812(Box<Ws2812>),
    PulsesReplay(Box<PulsesReplay>),
    Esp32s3(Box<Esp32s3>),
}

/// An ESP32-S3 running `firmware`: a bare-metal program whose core starts at its entry at
/// power-on, with every segment loaded, or the app on its flash, from where the bootloader
/// leaves it.
///
/// `VCC` and `GND` need nets, and every GPIO is optional. A GPIO with no net is neither stamped
/// nor watched. One with a net drives through `output` from `VCC` and into `GND`, pulls up
/// through `pull_up_resistance`, or is off, as the firmware sets it, leaks to `VCC` and to `GND`
/// through `leakage_resistance`, and reads its level through `input_threshold`. Only parse
/// builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Esp32s3 {
    /// `VCC`, `GND`, then each GPIO with a net, in `PINS` order.
    pub pins: Vec<NetId>,
    /// The name of each of `pins`.
    pub names: Vec<&'static str>,
    pub firmware: Esp32s3Firmware,
    pub output: OutputStage,
    /// Ohm. Every GPIO's pull-up.
    pub pull_up_resistance: Value,
    /// Ohm. Every GPIO's to `VCC` and again to `GND`, in every drive state.
    pub leakage_resistance: Value,
    pub input_threshold: Threshold,
}

/// What an `esp32s3` runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Esp32s3Firmware {
    /// A bare-metal program in SRAM.
    Elf(elf::Elf),
    /// The app on the flash, booted with the mask ROM.
    Flash { rom: Blob, flash: flash::Flash },
}

impl Esp32s3Firmware {
    /// The blobs it names: the ELF, or the ROM and then each flash region.
    pub fn blobs(&self) -> Vec<&Blob> {
        match self {
            Esp32s3Firmware::Elf(elf) => vec![elf.blob()],
            Esp32s3Firmware::Flash { rom, flash } => {
                std::iter::once(rom).chain(flash.regions()).collect()
            }
        }
    }

    /// The symbol a diagnostic names `pc` by. A flash's app carries none.
    pub fn symbolize(&self, pc: u32) -> Option<String> {
        match self {
            Esp32s3Firmware::Elf(elf) => elf.symbolize(pc),
            Esp32s3Firmware::Flash { .. } => None,
        }
    }
}

impl Esp32s3 {
    /// The mask ROM the model runs, revision 0, as its SHA-256 and length.
    pub const ROM: (&'static str, u64) = (
        "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
        949_552,
    );

    /// The supply pins, then the GPIOs a board can wire. GPIO22 to GPIO25 do not exist, and
    /// GPIO33 to GPIO37 belong to the in-package PSRAM.
    pub const PINS: [&'static str; 42] = [
        "VCC", "GND", "GPIO0", "GPIO1", "GPIO2", "GPIO3", "GPIO4", "GPIO5", "GPIO6", "GPIO7",
        "GPIO8", "GPIO9", "GPIO10", "GPIO11", "GPIO12", "GPIO13", "GPIO14", "GPIO15", "GPIO16",
        "GPIO17", "GPIO18", "GPIO19", "GPIO20", "GPIO21", "GPIO26", "GPIO27", "GPIO28", "GPIO29",
        "GPIO30", "GPIO31", "GPIO32", "GPIO38", "GPIO39", "GPIO40", "GPIO41", "GPIO42", "GPIO43",
        "GPIO44", "GPIO45", "GPIO46", "GPIO47", "GPIO48",
    ];

    /// Where a bare-metal program may load: the 512 KB of internal SRAM, through its window on
    /// the instruction bus and through its window on the data bus. Each window's `offset` is
    /// its first byte's place in the SRAM, so code at `0x4037_8000` and data at `0x3fc8_8000`
    /// are one byte.
    pub const RAM: [elf::Window; 2] = [
        elf::Window {
            start: 0x4037_0000,
            end: 0x403e_0000,
            offset: 0,
            code: true,
        },
        elf::Window {
            start: 0x3fc8_8000,
            end: 0x3fd0_0000,
            offset: 0x8000,
            code: false,
        },
    ];

    /// The GPIO number of each of `pins` past `VCC` and `GND`.
    pub fn gpios(&self) -> impl Iterator<Item = u8> + '_ {
        self.names[2..].iter().map(|name| {
            name.strip_prefix("GPIO")
                .and_then(|number| number.parse().ok())
                .expect("a GPIO pin name")
        })
    }
}

/// An ATmega328P in its 28-pin DIP, running `firmware` from power-on. Microchip DS40002061B.
///
/// Each I/O pin drives through `output` from VCC and into `GND_8`, pulls up through
/// `pull_up_resistance`, or is off, as the firmware sets it, leaks to VCC and to `GND_8` through
/// `leakage_resistance`, and reads its level through `input_threshold`. `PC6` is `RESET` while
/// `RSTDISBL` is unprogrammed, with its own pull-up and thresholds. `PB6` and `PB7` hold the
/// crystal or resonator the fuses select, and `clock_hz` is its frequency. Only parse builds
/// one, so `GND_8` and `GND_22` name one net.
#[derive(Clone, Debug, PartialEq)]
pub struct Atmega328p {
    pub pins: [NetId; 28],
    pub firmware: Firmware,
    /// Hertz, greater than 0, at most `u32::MAX`.
    pub clock_hz: IntValue,
    pub fuses: Fuses,
    pub output: OutputStage,
    /// Ohm. Every I/O pin's pull-up.
    pub pull_up_resistance: Value,
    /// Ohm. `PC6`'s pull-up while it is `RESET`.
    pub reset_pull_up_resistance: Value,
    /// Ohm. Every port pin's to VCC and again to `GND_8`, in every drive state, `PC6` as
    /// `RESET` included.
    pub leakage_resistance: Value,
    /// Every I/O pin's.
    pub input_threshold: Threshold,
    /// `PC6`'s while it is `RESET`.
    pub reset_threshold: Threshold,
}

impl Atmega328p {
    /// DIP order, DS40002061B Figure 1-1. The two ground pins carry their pin numbers.
    pub const PINS: [&'static str; 28] = [
        "PC6", "PD0", "PD1", "PD2", "PD3", "PD4", "VCC", "GND_8", "PB6", "PB7", "PD5", "PD6",
        "PD7", "PB0", "PB1", "PB2", "PB3", "PB4", "PB5", "AVCC", "AREF", "GND_22", "PC0", "PC1",
        "PC2", "PC3", "PC4", "PC5",
    ];
    /// The flash, 32 KB, DS40002061B Table 2-1.
    pub const FLASH_BYTES: usize = 32 * 1024;
}

/// A replay of SPI transactions, an active chip with no clock of its own. It drives `CS` high
/// and `SCLK` and `D0` to `D3` low, through `output` from `VCC` and into `GND`, for the whole
/// run, and sends each of `events` as a `Spi` record at the event's end. Only parse builds one,
/// so the events start in order and none starts before the one before it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct SpiReplay {
    pub pins: [NetId; 8],
    /// Hertz, greater than 0, at most `u32::MAX`. Every event's SCLK.
    pub clock_hz: IntValue,
    pub output: OutputStage,
    pub events: Vec<SpiEvent>,
}

impl SpiReplay {
    pub const PINS: [&'static str; 8] = ["VCC", "GND", "CS", "SCLK", "D0", "D1", "D2", "D3"];
}

/// A 360 by 360 QSPI panel controller, an active chip with no clock of its own. It takes the
/// `Spi` records on its `CS` and `SCLK` nets and the `Duty` records on `BLK`. It reads `RST`
/// through `input_threshold`, fractions of its supply, and drives `TE` low through `output`
/// from `VCC` and into `GND` for the whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct St77916 {
    pub pins: [NetId; 11],
    pub output: OutputStage,
    pub input_threshold: Threshold,
}

impl St77916 {
    pub const PINS: [&'static str; 11] = [
        "VCC", "GND", "CS", "SCLK", "D0", "D1", "D2", "D3", "RST", "TE", "BLK",
    ];
}

/// A CST816 touch controller, an active chip with no clock of its own. It is an I2C target at
/// address 0x15. `SDA` and `INT` are open drain into `GND` through `low_resistance` above
/// `low_drop`. `SCL` is an input, and the part does not hold it. `RST` low clears it. It reads
/// each of those four pins through `input_threshold`, fractions of its supply.
///
/// `touches` is the finger. Each event is one point, after t = 0 and at least [`MIN_GAP_PS`]
/// after the previous event. `x` and `y` run from 0 to [`Cst816::COORD_MAX`].
#[derive(Clone, Debug, PartialEq)]
pub struct Cst816 {
    pub pins: [NetId; 6],
    /// Volt, at least 0. The open-drain low side.
    pub low_drop: Value,
    /// Ohm.
    pub low_resistance: Value,
    pub input_threshold: Threshold,
    pub touches: Vec<Event<Touch>>,
}

/// One point of a [`Cst816`]: the finger is down at `(x, y)`, or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub x: u16,
    pub y: u16,
    pub down: bool,
}

impl Cst816 {
    pub const PINS: [&'static str; 6] = ["VCC", "GND", "SCL", "SDA", "INT", "RST"];
    /// The largest `x` or `y`.
    pub const COORD_MAX: u16 = 4095;

    /// The next touch strictly after `at`.
    pub fn next_touch_after(&self, at: Time) -> Option<Time> {
        let k = self.touches.partition_point(|event| event.at <= at);
        self.touches.get(k).map(|event| event.at)
    }

    /// The first time at or after `from` that one more touch passes the schedule rules.
    /// With no touch that is 1 ps. After one it is that touch plus [`MIN_GAP_PS`].
    pub fn earliest(&self, from: Time) -> Option<Time> {
        let first = match self.touches.last() {
            None => 1,
            Some(last) => last.at.as_ps().checked_add(MIN_GAP_PS)?,
        };
        Some(Time::from_ps(from.as_ps().max(first)))
    }
}

/// A chain of WS2812E-1313 LEDs, an active chip with no clock of its own. It reads `DIN`
/// through `input_threshold`, fractions of its supply, and takes `Pulses` records on that net.
/// Each LED keeps the first 24 bits, green then red then blue, and passes the rest on. A low
/// longer than 280 µs latches. `leds` is from 1 to [`Ws2812::LED_MAX`]. Only parse builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Ws2812 {
    pub pins: [NetId; 3],
    pub leds: u16,
    pub input_threshold: Threshold,
}

impl Ws2812 {
    pub const PINS: [&'static str; 3] = ["VCC", "GND", "DIN"];
    /// The most LEDs one part holds.
    pub const LED_MAX: u16 = 1024;
    /// The count a part omits.
    pub const LEDS_DEFAULT: u16 = 13;
}

/// A replay of pulse trains, an active chip with no clock of its own. It drives `OUT` at
/// `idle` through `output` from `VCC` and into `GND` for the whole run, and sends each of
/// `events` as a `Pulses` record at the event's end. Only parse builds one, so the events
/// start in order and none starts before the one before it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct PulsesReplay {
    pub pins: [NetId; 3],
    pub idle: bool,
    pub output: OutputStage,
    pub events: Vec<PulseTrain>,
}

/// One train of a `pulses_replay`, from `at` to `end`. `end` is `at` plus the pulse durations.
#[derive(Clone, Debug, PartialEq)]
pub struct PulseTrain {
    pub at: Time,
    pub end: Time,
    pub pulses: Arc<[(bool, u64)]>,
}

impl PulsesReplay {
    pub const PINS: [&'static str; 3] = ["VCC", "GND", "OUT"];
}

/// One transaction of a `spi_replay`: its phases, then `data` on `lanes`, from `at` to `end`.
/// `end` is `at` plus the transaction's SCLK cycles at the part's clock, rounded down to a
/// picosecond, and is after `at`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpiEvent {
    pub at: Time,
    pub end: Time,
    pub phases: Vec<SpiPhase>,
    pub lanes: Lanes,
    pub data: Arc<[u8]>,
    pub cs: Cs,
}

/// A program image from flash address 0. Only parse builds one, so it fits the flash.
#[derive(Clone, Debug, PartialEq)]
pub struct Firmware {
    flash: Vec<u8>,
}

impl Firmware {
    /// At most `Atmega328p::FLASH_BYTES`. A gap between records is erased flash, `0xFF`.
    pub fn flash(&self) -> &[u8] {
        &self.flash
    }
}

/// The three fuse bytes as programmed, DS40002061B Tables 28-6, 28-8 and 28-9, where a
/// programmed bit is 0. Only parse builds one, so `CKSEL` selects a crystal or ceramic
/// resonator (Table 9-1, `0110` to `1111`), `CKOUT`, `DWEN` and `BOOTRST` are unprogrammed,
/// and `BODLEVEL` is a level of Table 29-12 or disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fuses {
    low: u8,
    high: u8,
    extended: u8,
}

impl Fuses {
    pub fn low(self) -> u8 {
        self.low
    }

    pub fn high(self) -> u8 {
        self.high
    }

    pub fn extended(self) -> u8 {
        self.extended
    }

    /// `RSTDISBL` unprogrammed: `PC6` is `RESET`.
    pub fn has_reset(self) -> bool {
        self.high & 0x80 != 0
    }
}

/// Where an input reads high and low, as fractions of `V(VCC) - V(GND_8)` over `GND_8`. A low
/// input reads high above `high`, and a high one reads low at or below `low`. Each is in
/// `(0, 1)`, and `low` is at most `high`.
#[derive(Clone, Debug, PartialEq)]
pub struct Threshold {
    /// Unit `1`.
    pub low: Value,
    /// Unit `1`.
    pub high: Value,
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
/// `i = Is * (exp(vj / (N * kT/q)) - 1)` down to `vj = -3 * N * kT/q`, ngspice-42's reverse law
/// `i = -Is * (1 + (3 * N * kT/q / (e * vj))^3)` below it, and `V(anode) - V(cathode) = vj + i * Rs`.
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

/// A bipolar transistor. `npn` and `pnp` share it, and the kind is the polarity.
///
/// SPICE's Gummel-Poon model at its defaults, which is the transport form of Ebers-Moll with a
/// forward Early voltage, at 298.15 K with no temperature scaling. `RB`, `RC` and `RE` join
/// each pin to its internal node `b'`, `c'` or `e'`. With `vbe = s * (V(b') - V(e'))`,
/// `vbc = s * (V(b') - V(c'))`, `s` +1 for `npn` and -1 for `pnp`, and `VT` the junction's:
///
/// ```text
/// ibe = IS * (exp(vbe / (NF*VT)) - 1)
/// ibc = IS * (exp(vbc / (NR*VT)) - 1)
/// ic  = (ibe - ibc) * (1 - vbc/VAF) - ibc/BR - GMIN*vbc
/// ib  = ibe/BF + ibc/BR + GMIN*(vbe + vbc)
/// ```
///
/// Below `vbe = -3 * NF*VT`, `ibe` takes a `Junction`'s reverse law.
/// Below `vbc = -3 * NR*VT`, `ibc` does.
///
/// `s * ic` enters `c'` and `s * ib` enters `b'`. `CJE` is a constant capacitance from `b'` to
/// `e'`, and `CJC` from `b'` to `c'`. Every value is positive, as in a SPICE model card.
#[derive(Clone, Debug, PartialEq)]
pub struct Bjt {
    /// Nets of pins `collector`, `base`, `emitter`.
    pub pins: [NetId; 3],
    /// Ampere. `IS`.
    pub saturation_current: Value,
    /// Unit `1`. `BF`.
    pub forward_beta: Value,
    /// Unit `1`. `BR`.
    pub reverse_beta: Value,
    /// Unit `1`. `NF`.
    pub forward_emission: Value,
    /// Unit `1`. `NR`.
    pub reverse_emission: Value,
    /// Volt. `VAF`.
    pub early_voltage: Value,
    /// Ohm. `RB`.
    pub base_resistance: Value,
    /// Ohm. `RC`.
    pub collector_resistance: Value,
    /// Ohm. `RE`.
    pub emitter_resistance: Value,
    /// Farad. `CJE`.
    pub base_emitter_capacitance: Value,
    /// Farad. `CJC`.
    pub base_collector_capacitance: Value,
    /// Ampere, greater than 0. Bounds `|ic|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl Bjt {
    pub const PINS: [&'static str; 3] = ["collector", "base", "emitter"];
}

/// An N-channel MOSFET. Level 1 with the bulk tied to the source, so `W/L` is folded into `KP`
/// and there is no body effect. No temperature scaling.
///
/// With `vgs = V(gate) - V(source)` and `vds = V(drain) - V(source)`, and `vds >= 0`:
///
/// ```text
/// cutoff, vgs <= VTO:                 id = 0
/// saturation, vds >= vgs - VTO:       id = KP/2 * (vgs - VTO)^2 * (1 + LAMBDA*vds)
/// linear:                             id = KP * vds * (vgs - VTO - vds/2) * (1 + LAMBDA*vds)
/// ```
///
/// `vds < 0` swaps the terminals, `id = -f(vgs - vds, -vds)`. `id` then gains `GMIN * vds`,
/// which is ngspice-42's drain-bulk `gmin` once the bulk sits on the source and `is` is 0.
/// The gate draws no DC current. `CGS`, `CGD` and `CDS` are constant capacitances between the pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Mosfet {
    /// Nets of pins `drain`, `gate`, `source`.
    pub pins: [NetId; 3],
    /// Ampere per volt squared. `KP`. Greater than 0.
    pub transconductance_parameter: Value,
    /// Volt. `VTO`. Any finite value, so a depletion device can state a negative threshold.
    pub threshold_voltage: Value,
    /// Per volt. `LAMBDA`. Zero is a flat saturation current.
    pub channel_length_modulation: Value,
    /// Farad. From gate to source.
    pub gate_source_capacitance: Value,
    /// Farad. From gate to drain.
    pub gate_drain_capacitance: Value,
    /// Farad. From drain to source.
    pub drain_source_capacitance: Value,
    /// Ampere, greater than 0. Bounds `|id|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl Mosfet {
    pub const PINS: [&'static str; 3] = ["drain", "gate", "source"];
}

/// A static linear regulator. The pass branch's voltage falls with its own current.
/// `in` feeds the pass element and the ground current. `out` is the regulated pin.
/// `gnd` is the reference for both voltages and the return for the ground current.
///
/// `vset = output_voltage + line_regulation * (V(in) - V(gnd) - output_voltage)`.
/// The regulating line is `vset - output_resistance * i`. The dropout line is
/// `V(in) - V(gnd) - dropout_voltage - dropout_resistance * i`. The pass voltage
/// uses the smooth maximum of that line and `-Rf * i`, where `Rf` is a constant
/// of the law, then the smooth minimum against the regulating line, a current
/// limit and a reverse block. At no load the floor is the ground pin. A load
/// that draws current with the input at the pin pulls the output below it.
/// `i` is the current in the pass branch, from `in` to `out`. The ground current is
/// `(quiescent_current + ground_current_ratio * i)` times a smooth step of
/// `(V(in) - V(gnd)) / dropout_voltage`. The constants of that law are in the
/// engine, not fields.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearRegulator {
    /// Nets of pins `in`, `out`, `gnd`.
    pub pins: [NetId; 3],
    /// Volt. The output at the line-regulation reference input, at no load.
    pub output_voltage: Value,
    /// Unit `1`. The change in the set voltage per volt of input, at least 0 and below 1.
    pub line_regulation: Value,
    /// Ohm. The fall of the regulating line per ampere.
    pub output_resistance: Value,
    /// Volt. The dropout line's intercept.
    pub dropout_voltage: Value,
    /// Ohm. The fall of the dropout line per ampere.
    pub dropout_resistance: Value,
    /// Ampere. The pass current the limit holds.
    pub current_limit: Value,
    /// Ampere. The ground current at no load, once the input is above `dropout_voltage`.
    pub quiescent_current: Value,
    /// Unit `1`. The extra ground current per ampere of pass current. Zero is a constant ground current.
    pub ground_current_ratio: Value,
    /// Ampere, greater than 0. Bounds `|iout|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl LinearRegulator {
    pub const PINS: [&'static str; 3] = ["in", "out", "gnd"];
}

/// A forward junction and a breakdown junction, each with its own series resistance.
///
/// `series_resistance` joins the anode to internal node 0. The forward junction runs from
/// there to the cathode, with the diode's law. `breakdown_resistance` joins internal node 1
/// to the cathode, and the breakdown junction runs from internal node 1 to internal node 0.
/// Breakdown current does not flow in the forward junction.
///
/// `breakdown` is derived at parse time:
/// `nbvt = (zener_impedance - series_resistance - breakdown_resistance) * zener_current`,
/// `bvj = zener_voltage - zener_current * (series_resistance + breakdown_resistance)`,
/// `Isb = zener_current / expm1(bvj / nbvt)`.
/// At `zener_current` the pin voltage is `zener_voltage`. The slope there is
/// `series_resistance + breakdown_resistance + nbvt / (zener_current + Isb)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Zener {
    /// Nets of pins `anode`, `cathode`.
    pub pins: [NetId; 2],
    /// Ampere. The forward junction's `Is`.
    pub saturation_current: Value,
    /// Unit `1`. The forward junction's `N`.
    pub emission_coefficient: Value,
    /// Ohm. From the anode to internal node 0.
    pub series_resistance: Value,
    /// Volt. `VZ`, the pin voltage at `zener_current` in breakdown.
    pub zener_voltage: Value,
    /// Ampere. `IZT`.
    pub zener_current: Value,
    /// Ohm. `ZZT`, the slope of the pin voltage at `zener_current`.
    pub zener_impedance: Value,
    /// Ohm. In series with the breakdown junction only.
    pub breakdown_resistance: Value,
    /// The breakdown junction derived from the row and the two resistances.
    pub breakdown: Breakdown,
    /// Ampere, greater than 0. Bounds `|i|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

/// `Isb`, `nbvt` and `vcrit` of a zener's breakdown junction.
#[derive(Clone, Debug, PartialEq)]
pub struct Breakdown {
    /// Ampere.
    pub saturation_current: f64,
    /// Volt. `NBV * VT`.
    pub nvt: f64,
    /// Volt. The breakdown junction's start voltage.
    pub vcrit: f64,
}

/// Which derived breakdown value breaks `finite_junction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakdownFault {
    /// `nbvt` is not normal.
    Impedance,
    /// `Isb` is not normal, or `vcrit` is not finite and positive.
    Voltage,
}

impl BreakdownFault {
    pub fn field(self) -> &'static str {
        match self {
            BreakdownFault::Impedance => "zener_impedance",
            BreakdownFault::Voltage => "zener_voltage",
        }
    }
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
    /// The field that breaks the rule. `emission` names the emission field, which is
    /// `emission_coefficient` on a diode and `forward_emission` or `reverse_emission` on a
    /// transistor.
    pub fn field(self, emission: &'static str) -> &'static str {
        match self {
            JunctionFault::EmissionCoefficient => emission,
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
    let vcrit = nvt * libm::log(nvt / (SQRT_2 * saturation));
    if !(vcrit.is_finite() && vcrit > 0.0) {
        return Err(if saturation.is_normal() {
            JunctionFault::EmissionCoefficient
        } else {
            JunctionFault::SaturationCurrent
        });
    }
    Ok(JunctionScale { nvt, vcrit })
}

/// The breakdown junction of a row whose impedance and voltage already clear the two resistances.
///
/// `nbvt` must be normal. `Isb` must be normal and positive, and its `vcrit` must be finite and
/// positive. A failure names [`BreakdownFault::Impedance`] or [`BreakdownFault::Voltage`].
pub fn breakdown_scale(
    series_resistance: f64,
    breakdown_resistance: f64,
    zener_voltage: f64,
    zener_current: f64,
    zener_impedance: f64,
) -> Result<Breakdown, BreakdownFault> {
    let nbvt = (zener_impedance - series_resistance - breakdown_resistance) * zener_current;
    if !nbvt.is_normal() {
        return Err(BreakdownFault::Impedance);
    }
    let bvj = zener_voltage - zener_current * (series_resistance + breakdown_resistance);
    let isb = zener_current / libm::expm1(bvj / nbvt);
    if !isb.is_normal() || isb <= 0.0 {
        return Err(BreakdownFault::Voltage);
    }
    let vcrit = nbvt * libm::log(nbvt / (SQRT_2 * isb));
    if !(vcrit.is_finite() && vcrit > 0.0) {
        return Err(BreakdownFault::Voltage);
    }
    Ok(Breakdown {
        saturation_current: isb,
        nvt: nbvt,
        vcrit,
    })
}

/// What a rating bounds. Each maps to one quantity and one diagnostic code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RatingKind {
    Current,
    Power,
}

impl RatingKind {
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
    Datasheet {
        doc: String,
        at: String,
        basis: Basis,
    },
}

/// Which figure of a datasheet a value is. A new basis keeps the netlist version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// A typical column, or a typical-only curve.
    Typical,
    /// A minimum column.
    Min,
    /// A maximum column, recommended operating conditions included.
    Max,
    /// A part's stated value, such as a schematic value or a marked resistance.
    Nominal,
    /// An absolute maximum rating.
    AbsoluteMax,
}
