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
/// - Only `run_until` changes the drives. What any other call causes comes at least 1 ps
///   later, at a stop.
pub trait Device: State {
    /// What it shows besides its drives. Nothing, unless it says otherwise.
    const SHOWS: Shows = Shows::Nothing;
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
    /// What it shows now, laid out as [`SHOWS`](Self::SHOWS) says. The host reads it after a
    /// call that ran [`Cx::show`], and at the end of a run.
    fn view(&self) -> &[u8] {
        &[]
    }
}

/// What a device shows besides its drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shows {
    Nothing,
    /// A chain of LEDs. [`Device::view`] is three bytes per LED, LED 0 first, in `order`, such
    /// as `grb`. `kind` names the chain, such as `ws2812`.
    Leds {
        kind: &'static str,
        order: &'static str,
    },
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
    /// The level the line rests at, and how long it holds that level before it is idle.
    const IDLE: Idle;
    /// A level that held for `width` picoseconds ended at `at`.
    fn pulse(&mut self, cx: &mut Cx, at: Time, level: bool, width: u64);
    /// The line has held `level`, [`IDLE`](Self::IDLE)'s level, for its time, at `at`. A line
    /// that rests from power-on is not idle: the host times it from the first change to it.
    fn idle(&mut self, _cx: &mut Cx, _at: Time, _level: bool) {}
}

/// When a pulsed line is idle: after it holds `level` for `after` picoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Idle {
    pub level: bool,
    pub after: u64,
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

/// What a device may do during a call besides change its own state. The host reads both after
/// every call.
///
/// [`broke`](Cx::broke) names a rule and the time. The first call wins. [`show`](Cx::show) says
/// what the device shows changed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cx {
    broken: Option<(&'static str, Time)>,
    shown: bool,
}

impl Cx {
    /// The rule `rule` broke at `at`. A later call in this same `Cx` changes nothing. The host
    /// knows each rule by name and stops on one it does not know.
    pub fn broke(&mut self, rule: &'static str, at: Time) {
        if self.broken.is_none() {
            self.broken = Some((rule, at));
        }
    }

    /// What the device shows changed during this call. The host reads [`Device::view`] after
    /// it, at the call's time. A device that shows nothing never calls this.
    pub fn show(&mut self) {
        self.shown = true;
    }

    /// The first rule named in this call, and its time.
    pub fn broken(&self) -> Option<(&'static str, Time)> {
        self.broken
    }

    /// Whether this call ran [`show`](Self::show).
    pub fn shown(&self) -> bool {
        self.shown
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
    fn the_first_broken_rule_wins_and_a_show_stays_shown() {
        let mut cx = Cx::default();
        assert_eq!(cx.broken(), None);
        assert!(!cx.shown());
        cx.broke("bit", Time::from_ps(3));
        cx.broke("other", Time::from_ps(4));
        assert_eq!(cx.broken(), Some(("bit", Time::from_ps(3))));
        cx.show();
        assert!(cx.shown());
    }
}
