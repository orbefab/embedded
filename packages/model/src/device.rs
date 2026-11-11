use crate::{Drives, Levels, Stop, Time};

/// Everything a device remembers: fixed-size plain data with no heap, so a copy of it is a
/// checkpoint. The host keeps several copies of each part's state and runs, rewinds and commits
/// them, so state must not point anywhere.
pub trait State: Copy + Send + 'static {}

/// A device on the circuit's clock. Each call reads nothing but the device and its arguments,
/// so two equal devices given the same calls stay equal.
///
/// The host keeps the contract the run depends on:
/// - `run_until(limit)` runs from the last time it reached and stops at the first time in
///   `(reached, limit]` where its drives may change or it samples, or returns `None` once it has
///   done everything up to `limit`. Two calls to `l1` then `l2` leave the state one call to `l2`
///   does. All of a time's effects are one stop.
/// - `input`, `sample` and `event` never change the drives. What they cause comes at least 1 ps
///   later, at a stop.
pub trait Device: State {
    /// Parses the part's logic params. Electrical params never reach the device.
    fn new(params: &Params<'_>) -> Result<Self, Error>;
    /// The first time in `(reached, limit]` where the drives may change or a sample is due.
    /// `None` once everything up to `limit` is done.
    fn run_until(&mut self, cx: &mut Cx, limit: Time) -> Option<Stop>;
    /// One drive per declared output, in declaration order.
    fn drive(&self) -> Drives;
    /// Its watches' levels from `at` on.
    fn input(&mut self, _cx: &mut Cx, _at: Time, _levels: Levels) {}
    /// The volts of each of its samples at its stop at `at`, in declaration order.
    fn sample(&mut self, _cx: &mut Cx, _at: Time, _volts: &[f64]) {}
    /// An event it declares, such as a touch, at `at`.
    fn event(&mut self, _cx: &mut Cx, _at: Time, _event: Event<'_>) {}
}

/// A device that is an I2C target. The host runs the bus: it watches SCL and SDA, matches the
/// address the device declares, drives SDA for each ACK and each read bit, and holds SDA's
/// timing. The device sees only transfers and bytes.
pub trait I2cTarget: Device {
    /// The master addressed this target, to write to it or to read from it.
    fn start(&mut self, _cx: &mut Cx, _at: Time, _dir: Dir) {}
    /// A byte the master wrote. The host ACKs it on [`Ack::Yes`].
    fn write(&mut self, cx: &mut Cx, at: Time, byte: u8) -> Ack;
    /// The next byte the master reads.
    fn read(&mut self, cx: &mut Cx, at: Time) -> u8;
    /// The master's ACK after a read byte. `more` is whether it asked for another byte.
    fn master_ack(&mut self, _cx: &mut Cx, _at: Time, _more: bool) {}
    /// A STOP.
    fn stop(&mut self, _cx: &mut Cx, _at: Time) {}
}

/// A device on a pulsed line. The host watches the line, times each level, and calls the device
/// when a level ends. The device sees the level and how long it held.
pub trait PulseTarget: Device {
    /// A level that held for `width` picoseconds ended at `at`.
    fn pulse(&mut self, cx: &mut Cx, at: Time, level: bool, width: u64);
    /// The line has held `level` for the idle time the host was given, at `at`.
    fn idle(&mut self, _cx: &mut Cx, _at: Time, _level: bool) {}
}

/// Which way a transfer goes, from the master's side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Write,
    Read,
}

/// Whether a target takes a written byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ack {
    Yes,
    No,
}

/// What a device may do during a call besides change its own state.
///
/// [`broke`](Cx::broke) names a rule and the time. The first call wins. [`view`](Cx::view)
/// replaces the bytes the host reads after the call, such as a chain's colours at a latch.
#[derive(Debug, Default)]
pub struct Cx {
    broken: Option<(&'static str, Time)>,
    view: Vec<u8>,
    viewed: bool,
}

impl Cx {
    /// The rule `rule` broke at `at`. A later call in this same `Cx` changes nothing.
    pub fn broke(&mut self, rule: &'static str, at: Time) {
        if self.broken.is_none() {
            self.broken = Some((rule, at));
        }
    }

    /// Replaces the bytes the host reads after this call.
    pub fn view(&mut self, bytes: &[u8]) {
        self.viewed = true;
        self.view.clear();
        self.view.extend_from_slice(bytes);
    }

    /// The first rule named in this call, and its time.
    pub fn broken(&self) -> Option<(&'static str, Time)> {
        self.broken
    }

    /// The bytes of the last [`view`](Self::view) in this call.
    pub fn view_bytes(&self) -> &[u8] {
        &self.view
    }

    /// Whether this call ran [`view`](Self::view).
    pub fn viewed(&self) -> bool {
        self.viewed
    }
}

/// A value of a param or an event field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bool(bool),
}

/// Named values in the order a device declares them.
#[derive(Clone, Copy, Debug)]
pub struct Fields<'a>(&'a [(&'a str, Value)]);

impl<'a> Fields<'a> {
    pub const NONE: Fields<'static> = Fields(&[]);

    pub const fn new(fields: &'a [(&'a str, Value)]) -> Fields<'a> {
        Fields(fields)
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.0
            .iter()
            .find(|(field, _)| *field == name)
            .map(|&(_, value)| value)
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        match self.get(name)? {
            Value::Int(value) => Some(value),
            Value::Bool(_) => None,
        }
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.get(name)? {
            Value::Bool(value) => Some(value),
            Value::Int(_) => None,
        }
    }
}

/// A part's logic params, which [`Device::new`] parses.
pub type Params<'a> = Fields<'a>;

/// Why [`Device::new`] refused a part's params, in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);

/// Something that happens to a device from outside the circuit.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Event<'a> {
    /// One of the events the device declares, such as a touch, with its fields. The host has
    /// checked each field against the declaration.
    Declared { name: &'a str, fields: Fields<'a> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_reads_back_by_name_and_kind() {
        let fields = Fields::new(&[("x", Value::Int(291)), ("down", Value::Bool(true))]);
        assert_eq!(fields.int("x"), Some(291));
        assert_eq!(fields.bool("down"), Some(true));
        assert_eq!(fields.bool("x"), None, "x is an int");
        assert_eq!(fields.int("y"), None, "no field y");
        assert_eq!(Fields::NONE.get("x"), None);
    }

    #[test]
    fn the_first_broken_rule_wins_and_a_view_replaces_the_bytes() {
        let mut cx = Cx::default();
        assert_eq!(cx.broken(), None);
        assert!(!cx.viewed());
        cx.broke("bit", Time::from_ps(3));
        cx.broke("other", Time::from_ps(4));
        assert_eq!(cx.broken(), Some(("bit", Time::from_ps(3))));
        cx.view(&[1, 2]);
        cx.view(&[9]);
        assert!(cx.viewed());
        assert_eq!(cx.view_bytes(), &[9]);
    }
}
