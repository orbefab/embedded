//! Nets are ground first, then the rest by name. Parts are by name.

use std::f64::consts::SQRT_2;

pub mod bounce;
mod document;
mod json;
mod waveform;

use serde::{Deserialize, Serialize, Serializer};

use crate::diagnostic::{Diagnostic, ValueRule};
use crate::finite::Finite;
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
    json::parse(json)
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
            Model::Potentiometer(_) => &Potentiometer::PINS,
            Model::VoltageSource(_) => &VoltageSource::PINS,
            Model::CurrentSource(_) => &CurrentSource::PINS,
            Model::Diode(_) | Model::Led(_) => &Junction::PINS,
            Model::Capacitor(_) => &Capacitor::PINS,
            Model::Switch(_) => &Switch::PINS,
            Model::Chip(Chip::Ne555(_)) => &Ne555::PINS,
            Model::Chip(Chip::Atmega328p(_)) => &Atmega328p::PINS,
        }
    }

    /// `i`, `v` and `p`. A chip and a potentiometer have none.
    pub fn has_quantities(&self) -> bool {
        match self {
            Model::Resistor(_)
            | Model::VoltageSource(_)
            | Model::CurrentSource(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Capacitor(_)
            | Model::Switch(_) => true,
            Model::Potentiometer(_) | Model::Chip(_) => false,
        }
    }

    pub fn pins(&self) -> &[NetId] {
        match self {
            Model::Resistor(r) => &r.pins,
            Model::Potentiometer(p) => &p.pins,
            Model::VoltageSource(v) => &v.pins,
            Model::CurrentSource(c) => &c.pins,
            Model::Diode(j) | Model::Led(j) => &j.pins,
            Model::Capacitor(c) => &c.pins,
            Model::Switch(s) => &s.pins,
            Model::Chip(Chip::Ne555(n)) => &n.pins,
            Model::Chip(Chip::Atmega328p(m)) => &m.pins,
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
            | Model::Switch(_)
            | Model::Chip(_) => (None, None),
            Model::Diode(j) | Model::Led(j) => (j.max_current.as_ref(), j.max_power.as_ref()),
        };
        let current = current.map(|v| (RatingKind::Current, v));
        let power = power.map(|v| (RatingKind::Power, v));
        current.into_iter().chain(power).collect()
    }

    /// The next contact edge, source jump, or potentiometer move strictly after `at`.
    pub fn next_change_after(&self, at: Time) -> Option<Time> {
        match self {
            Model::VoltageSource(source) => source.voltage.next_change_after(at),
            Model::CurrentSource(source) => source.current.next_change_after(at),
            Model::Switch(switch) => switch.schedule.next_change_after(at),
            Model::Potentiometer(pot) => pot.position.next_change_after(at),
            Model::Resistor(_)
            | Model::Diode(_)
            | Model::Led(_)
            | Model::Capacitor(_)
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
            | Model::Capacitor(_)
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
            | Model::Capacitor(_)
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
            | Model::Capacitor(_)
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
            | Model::Capacitor(_)
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
