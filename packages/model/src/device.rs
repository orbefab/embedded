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

/// What a device may do during a call besides change its own state. Nothing yet: the calls that
/// write to the run, such as sending a record or marking a rule broken, join with the first
/// device that needs them.
#[derive(Debug, Default)]
pub struct Cx {
    _private: (),
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
}
