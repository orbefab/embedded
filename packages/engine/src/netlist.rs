//! Nets are ground first, then the rest by name. Parts are by name.

pub mod bounce;
pub mod elf;
pub(crate) mod json;
pub(crate) mod waveform;

mod catalog;

pub(crate) use catalog::Kind;
pub use catalog::{Conduction, Model, engine_kinds, live_field};

use serde::{Deserialize, Serialize, Serializer};

use orbe_model::Time;

use crate::blob::{Blob, Blobs, Sha256};
use crate::diagnostic::{Diagnostic, ValueRule};
use crate::finite::Finite;
use crate::name::Name;
use crate::world::{Quantity, World};

use crate::chip::Ne555;
use crate::device::{
    Bmp280, Cst816, Declared, Dht22, Events, Hd44780, Hd44780I2c, Max7219, PulsesReplay, Row, Sg90,
    SpiReplay, Ssd1306, St77916, Ws2812,
};
use crate::mcu::{Atmega328p, Esp32s3};

pub use waveform::{Baud, Frame, Pulse, Serial, SerialEvent, SerialSend, Shape, Step, Waveform};

/// The netlist `version`s this engine reads, ascending. See docs/engine/netlist.md.
pub const VERSIONS: &[u64] = &[1];

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
    world: World,
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

    /// The world quantities the netlist declares, each with its schedule.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Appends a change of declared quantity `quantity` to `level` at `at`, a time
    /// `Schedule::earliest` returned for it, and moves every part field that reads it.
    pub fn append_world(&mut self, quantity: Quantity, at: Time, level: Finite) {
        self.world.append(quantity, at, level);
        self.follow_world(quantity);
    }

    /// Drops events of declared quantity `quantity` at or after `from`. A read at `from` is
    /// still the value from before those events, so they have not taken effect. Moves every
    /// part field that reads the quantity. Returns how many events it dropped.
    pub fn drop_world_from(&mut self, quantity: Quantity, from: Time) -> usize {
        let dropped = self.world.drop_from(quantity, from);
        self.follow_world(quantity);
        dropped
    }

    fn follow_world(&mut self, quantity: Quantity) {
        let schedule = self
            .world
            .schedule(quantity)
            .expect("a revised quantity is declared");
        for part in &mut self.parts {
            if let Some(driven) = part.model.driven_mut()
                && driven.quantity == quantity
            {
                driven.follow(schedule);
            }
        }
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
    /// potentiometer. A serial send goes on a `serial` source. A device's event goes on a part
    /// whose kind declares it. For a contact or a level, `at` is a time `Schedule::earliest`
    /// returned for that part, and the event changes the value in force. A serial send does not
    /// use that helper. Panics when the part cannot take that event.
    pub fn append(&mut self, id: PartId, at: Time, to: Appended) {
        match to {
            Appended::Contact(to) => self.append_contact(id, at, to),
            Appended::Level(level) => self.append_level(id, at, level),
            Appended::Serial(send) => self.append_serial(id, at, send),
            Appended::Event(event) => self.append_event(id, at, event),
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

    fn append_event(&mut self, id: PartId, at: Time, event: Declared) {
        let part = &mut self.parts[id.index()];
        let Some(events) = part.model.events_mut() else {
            panic!("{} takes no event", part.name.as_str());
        };
        events.push(at, event);
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

impl Model {
    /// The blobs it names, in its own field order.
    pub fn blobs(&self) -> Vec<&Blob> {
        match self {
            Model::Chip(Chip::Esp32s3(mcu)) => mcu.firmware.blobs(),
            _ => Vec::new(),
        }
    }

    /// The kind's row and the part's events, for a device on the `orbe-model` contract. This
    /// is the device table: a new device kind adds its arm here.
    pub(crate) fn device(&self) -> Option<(&'static Row, &Events)> {
        let Model::Chip(chip) = self else {
            return None;
        };
        match chip {
            Chip::Cst816(dev) => Some((&crate::device::cst816::ROW, &dev.events)),
            Chip::Dht22(_) => Some((&crate::device::dht22::ROW, &crate::device::NO_EVENTS)),
            Chip::Bmp280(_) => Some((&crate::device::bmp280::ROW, &crate::device::NO_EVENTS)),
            Chip::Hc595(_) => Some((&crate::device::hc595::ROW, &crate::device::NO_EVENTS)),
            Chip::Max7219(_) => Some((&crate::device::max7219::ROW, &crate::device::NO_EVENTS)),
            Chip::Ws2812(_) => Some((&crate::device::ws2812::ROW, &crate::device::NO_EVENTS)),
            Chip::St77916(_) => Some((&crate::device::st77916::ROW, &crate::device::NO_EVENTS)),
            Chip::Ssd1306(_) => Some((&crate::device::ssd1306::ROW, &crate::device::NO_EVENTS)),
            Chip::Hd44780I2c(_) => {
                Some((&crate::device::hd44780_i2c::ROW, &crate::device::NO_EVENTS))
            }
            Chip::Hd44780(_) => Some((&crate::device::hd44780::ROW, &crate::device::NO_EVENTS)),
            Chip::SpiReplay(_) => {
                Some((&crate::device::spi_replay::ROW, &crate::device::NO_EVENTS))
            }
            Chip::PulsesReplay(_) => Some((
                &crate::device::pulses_replay::ROW,
                &crate::device::NO_EVENTS,
            )),
            Chip::Sg90(_) => Some((&crate::device::sg90::ROW, &crate::device::NO_EVENTS)),
            Chip::Ne555(_) | Chip::Atmega328p(_) | Chip::Esp32s3(_) => None,
        }
    }

    /// [`device`](Self::device)'s events, for appending one.
    fn events_mut(&mut self) -> Option<&mut Events> {
        match self {
            Model::Chip(Chip::Cst816(dev)) => Some(&mut dev.events),
            _ => None,
        }
    }
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
    /// An event a device declares, such as a touch on a `cst816`.
    Event(Declared),
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

    /// Drops events at or after `from`. Events are ascending, so this removes a suffix.
    pub(crate) fn drop_from(&mut self, from: Time) -> usize {
        let before = self.events.len();
        self.events.retain(|event| event.at < from);
        before - self.events.len()
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
    pub(crate) fn append_level(&mut self, at: Time, level: Finite) {
        debug_assert_eq!(self.earliest(at, 0), Some(at));
        debug_assert_ne!(self.after(at).si, level);
        self.events.push(Event {
            at,
            to: Value { si: level },
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
    Ssd1306(Box<Ssd1306>),
    Hd44780I2c(Box<Hd44780I2c>),
    Hd44780(Box<Hd44780>),
    Cst816(Box<Cst816>),
    Dht22(Box<Dht22>),
    Bmp280(Box<Bmp280>),
    Hc595(Box<crate::device::Hc595>),
    Max7219(Box<Max7219>),
    Ws2812(Box<Ws2812>),
    PulsesReplay(Box<PulsesReplay>),
    Esp32s3(Box<Esp32s3>),
    Sg90(Box<Sg90>),
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

/// An integer quantity.
#[derive(Clone, Debug, PartialEq)]
pub struct IntValue {
    pub value: u64,
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

/// A number in SI base units. The JSON `unit` was checked against the field and is not kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    pub si: Finite,
}

/// A current or power rating: its limit in SI units and how its datasheet means it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rating {
    pub si: Finite,
    /// `None` when the netlist names no basis. The kind's default then applies, as
    /// `ratings::Ratings` says.
    pub basis: Option<Basis>,
    /// The most any one sample may take, in the same unit, when the datasheet gives a peak or
    /// surge limit beside a sustained one. Only a sustained basis carries it.
    pub peak: Option<Finite>,
    /// True when the sheet states no absolute maximum and this limit is the largest load the
    /// sheet characterizes (such as a primary alkaline cell's plotted test drains).
    pub characterized: bool,
}

/// How a datasheet means a rating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// Never to be passed, so each sample is checked against it.
    Peak,
    /// A limit on the heat a held value makes: `average` of the samples through a first-order
    /// filter with time constant `tau_ps`, which starts cold.
    Sustained { tau_ps: u64, average: Average },
}

/// What a sustained rating's filter takes of each sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Average {
    /// The square, and the reading is its root. A current through a resistance heats so.
    Rms,
    /// The magnitude. A power, or a current across a near-fixed drop, heats so.
    Mean,
}

impl Average {
    pub fn as_str(self) -> &'static str {
        match self {
            Average::Rms => "rms",
            Average::Mean => "mean",
        }
    }
}
