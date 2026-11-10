//! Nets are ground first, then the rest by name. Parts are by name.

pub mod bounce;
mod document;
pub mod elf;
pub mod flash;
pub(crate) mod json;
pub(crate) mod waveform;

use serde::{Deserialize, Serialize, Serializer};

use orbe_model::Time;

use crate::blob::{Blob, Blobs, Sha256};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::finite::Finite;
use crate::name::{Name, Quantity};

use crate::chip::{LinearRegulator, Ne555, Relay};
use crate::device::{Cst816, PulsesReplay, SpiReplay, St77916, Touch, Ws2812};
use crate::mcu::{Atmega328p, Esp32s3};
use crate::passive::{
    Capacitor, CurrentSource, Inductor, Potentiometer, Resistor, Switch, VoltageSource,
};
use crate::semiconductor::{Bjt, Junction, Mosfet, Zener};

pub use document::Document;
pub use waveform::{Baud, Frame, Pulse, Serial, SerialEvent, SerialSend, Shape, Step, Waveform};

/// The netlist `version`s this engine reads, ascending. See docs/engine/netlist.md.
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
    pub(crate) initial: T,
    pub(crate) events: Vec<Event<T>>,
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
