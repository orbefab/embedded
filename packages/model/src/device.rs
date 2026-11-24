use crate::{Drives, Duty, Levels, Pulses, Quantity, Readings, Spi, SpiPhase, Stop, Time};

/// Everything a device remembers. A clone is a checkpoint. The host keeps several copies of
/// each part's state and runs, rewinds and commits them. A move is not a checkpoint, so a
/// memory too large for the wasm stack lives behind a `Box` and [`clone`](Clone::clone) copies
/// its bytes. A device small enough to stay `Copy` still copies by value.
pub trait State: Clone + Send + 'static {
    /// Becomes `source`. The host copies a state only with this, and only onto a copy made
    /// before or after it. The whole state by default. A state that holds [`Pages`](crate::Pages)
    /// copies them with [`Pages::copy_from`](crate::Pages::copy_from), which skips the pages
    /// that match, and the rest whole.
    fn copy_from(&mut self, source: &Self) {
        *self = source.clone();
    }
}

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
/// - The host stamps the [`Draw`] it read at the device's last stop. A draw that another call
///   changes takes effect at a stop the host puts 1 ps after that call.
pub trait Device: State {
    /// What it shows besides its drives. Nothing, unless it says otherwise.
    const SHOWS: Shows = Shows::Nothing;
    /// The world quantities it reads with [`Cx::world`]. A scene with a part of this device
    /// declares each one. None, unless it says otherwise.
    const WORLD: &'static [Quantity] = &[];
    /// Volt. Below this supply the host stamps every output off. [`Device::drive`] still
    /// reports the logical level. `None` drives at any supply.
    const MIN_SUPPLY_V: Option<f64> = None;
    /// Parses the part's logic params. Electrical params never reach the device.
    fn new(params: &Params<'_>) -> Result<Self, Error>;
    /// The first time in `(reached, limit]` where the drives may change or a sample is due.
    /// `None` once everything up to `limit` is done.
    fn run_until(&mut self, cx: &mut Cx, limit: Time) -> Option<Stop>;
    /// One drive per declared output, in declaration order.
    fn drive(&self) -> Drives;
    /// What it draws from its supply now. [`Draw::NONE`] unless it says otherwise.
    fn draw(&self) -> Draw {
        Draw::NONE
    }
    /// Its watches' levels from `at` on.
    fn input(&mut self, _cx: &mut Cx, _at: Time, _levels: Levels) {}
    /// The volts of each of its samples at its stop at `at`, in declaration order.
    fn sample(&mut self, _cx: &mut Cx, _at: Time, _volts: &[f64]) {}
    /// An event it declares, such as a touch, at `at`.
    fn event(&mut self, _cx: &mut Cx, _at: Time, _event: Event<'_>) {}
    /// What it shows now, the [`View`] that [`SHOWS`](Self::SHOWS) declares. The host reads it
    /// after a call that ran [`Cx::show`], and at the end of a run.
    fn view(&self) -> View<'_> {
        View::Nothing
    }
}

/// A device's draw from its supply: `amps` from its supply pin into its ground pin while the
/// supply is at or above `holds_above_v`. Below that it fades smoothly to none at 0 V, so it
/// never pulls the supply below its ground. The host stamps it with its slope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    /// Ampere, at least 0.
    pub amps: f64,
    /// Volt, greater than 0.
    pub holds_above_v: f64,
}

impl Draw {
    /// No draw.
    pub const NONE: Draw = Draw {
        amps: 0.0,
        holds_above_v: 1.0,
    };

    /// The current at a supply of `volts`, and its slope in siemens. A cubic ramp,
    /// `3 x^2 - 2 x^3` of `amps` with `x = volts / holds_above_v`, joins none at 0 V to all of
    /// it at `holds_above_v`, with no step in the current or its slope at either end.
    pub fn at(&self, volts: f64) -> (f64, f64) {
        let x = volts / self.holds_above_v;
        if x <= 0.0 {
            (0.0, 0.0)
        } else if x >= 1.0 {
            (self.amps, 0.0)
        } else {
            let ramp = x * x * (3.0 - 2.0 * x);
            let slope = 6.0 * x * (1.0 - x) / self.holds_above_v;
            (self.amps * ramp, self.amps * slope)
        }
    }
}

/// What a device shows besides its drives. Its [`Device::view`] is the matching [`View`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shows {
    Nothing,
    /// A chain of LEDs. `kind` names the chain, such as `ws2812`, and `order` names its bytes,
    /// such as `grb`.
    Leds {
        kind: &'static str,
        order: &'static str,
    },
    /// A screen of `width` by `height` pixels whose frame memory holds each pixel in `format`:
    /// `rgb565`, two bytes a pixel, rows top to bottom and each row left to right, or `mono-pages`,
    /// one bit a pixel in pages of 8 rows, where byte `page * width + column` holds rows
    /// `8 * page` to `8 * page + 7`, bit 0 the top one. `kind` names its controller, such as
    /// `st77916` or `ssd1306`.
    Screen {
        kind: &'static str,
        width: u16,
        height: u16,
        format: &'static str,
    },
}

/// What a device shows now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View<'a> {
    Nothing,
    /// Three bytes per LED, LED 0 first, in the declared order.
    Leds(&'a [u8]),
    /// The frame memory in the declared format, and the panel's state.
    Screen {
        frame: &'a [u8],
        panel: Panel,
    },
}

/// A screen controller's state besides its frame memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panel {
    /// The display is on.
    pub on: bool,
    pub inverted: bool,
    /// Its reset pin reads low.
    pub reset: bool,
    pub asleep: bool,
    /// The controller's own settings, such as the ST77916's `madctl` or the SSD1306's
    /// `contrast`: what a reader needs besides the flags to draw the glass from the frame memory.
    pub settings: Settings,
    /// How many registers hold a write it stores and does not run.
    pub registers: u16,
    /// The last duty its backlight pin took, `None` before the first.
    pub backlight: Option<Duty>,
}

/// A screen controller's own settings, each a name and a value, in the order the controller
/// declares them. At most [`Settings::MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    names: &'static [&'static str],
    values: [u16; Settings::MAX],
}

impl Settings {
    /// The most settings a panel carries.
    pub const MAX: usize = 12;

    /// `values[k]` is the setting `names[k]`. Panics when there are more than
    /// [`MAX`](Self::MAX).
    pub const fn new<const N: usize>(
        names: &'static [&'static str; N],
        values: [u16; N],
    ) -> Settings {
        assert!(N <= Settings::MAX, "a panel carries at most 12 settings");
        let mut all = [0; Settings::MAX];
        let mut k = 0;
        while k < N {
            all[k] = values[k];
            k += 1;
        }
        Settings { names, values: all }
    }

    /// The value of the setting `name`.
    pub fn get(&self, name: &str) -> Option<u16> {
        self.iter()
            .find(|&(setting, _)| setting == name)
            .map(|(_, value)| value)
    }

    /// Each setting and its value, in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, u16)> + '_ {
        self.names.iter().copied().zip(self.values)
    }
}

/// A map of each name to its value, in declaration order.
#[cfg(feature = "serde")]
impl serde::Serialize for Settings {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.names.len()))?;
        for (name, value) in self.iter() {
            map.serialize_entry(name, &value)?;
        }
        map.end()
    }
}

/// A pixel write a screen closed: its window, both ends included, before any mirroring, the
/// bytes it took, and the first 8 bytes of their SHA-256, as [`Sha256`](crate::Sha256) makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drawn {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
    pub bytes: u64,
    pub hash: [u8; 8],
}

/// A device that is an I2C target. The host runs the bus: it watches SCL and SDA, matches the
/// address the device declares, drives SDA for each ACK and each read bit, and holds SDA's
/// timing. The device sees only transfers and bytes.
pub trait I2cTarget: Device {
    /// Whether this target answers its address. A target that is not awake leaves the address
    /// unacknowledged, and the host calls nothing else for that transfer. Awake, unless it
    /// says otherwise.
    fn awake(&self) -> bool {
        true
    }
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
///
/// A device whose line is [`OPEN_DRAIN`](Self::OPEN_DRAIN) also answers on it: it queues a train
/// with [`Cx::pulses`], and the host plays it. The host pulls the line low for each low level
/// and releases it for each high one, and the line rises as its pull-up and its load let it.
pub trait PulseTarget: Device {
    /// The level the line rests at, and how long it holds that level before it is idle.
    const IDLE: Idle;
    /// Whether the device pulls its line low through an open drain, ahead of its own outputs in
    /// the image. It does not, unless it says so.
    const OPEN_DRAIN: bool = false;
    /// A level that held for `width` picoseconds ended at `at`.
    fn pulse(&mut self, cx: &mut Cx, at: Time, level: bool, width: u64);
    /// The line has held `level`, [`IDLE`](Self::IDLE)'s level, for its time, at `at`. A line
    /// that rests from power-on is not idle: the host times it from the first change to it.
    fn idle(&mut self, _cx: &mut Cx, _at: Time, _level: bool) {}
}

/// A device that is an SPI target. The host takes each transaction whose CS and SCLK sit on the
/// device's pins, checks that each lane it used sits on the device's own data pin, and holds the
/// bus while the device is in reset. The device sees whole transactions, and answers each byte
/// the host reads on MISO before the transaction ends.
pub trait SpiTarget: Device {
    /// A transaction that ended at `at`, after the host asked for each byte it reads.
    fn transaction(&mut self, cx: &mut Cx, at: Time, spi: &Spi);
    /// The byte it shifts out on MISO after `phases` and `index` bytes of the data stage, and
    /// `data`, the bytes the host sent before it. In a full-duplex transaction the byte goes out
    /// beside data byte `index`, so `data` is `index` bytes long. In a read phase it follows all
    /// the data. The host asks for each byte it reads once, in order, at or before the
    /// transaction's end. `None` leaves MISO to the host, which reads `0xFF` there. A device
    /// that never answers keeps the default.
    fn answer(
        &mut self,
        _cx: &mut Cx,
        _at: Time,
        _phases: &[SpiPhase],
        _data: &[u8],
        _index: usize,
    ) -> Option<u8> {
        None
    }
}

/// A device with a PWM input pin. The host gives it each duty a chip sets on that pin.
pub trait DutyTarget: Device {
    /// The duty set at `at`.
    fn duty(&mut self, cx: &mut Cx, at: Time, duty: Duty);
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

/// What a device may do during a call besides change its own state, and what it reads of the
/// world. The host fills it before every call and reads it after.
///
/// [`world`](Cx::world) reads a quantity the device declares. [`broke`](Cx::broke) names a rule
/// and the time. The first call wins. [`show`](Cx::show) says what an LED chain shows changed.
/// [`drew`](Cx::drew) is a pixel write a screen closed. [`pulses`](Cx::pulses) queues a train
/// on an open-drain line. [`replayed`](Cx::replayed) says the call replays a record.
#[derive(Clone, Debug, Default)]
pub struct Cx {
    world: Readings,
    broken: Option<(&'static str, Time)>,
    shown: bool,
    drawn: Option<Drawn>,
    queued: Option<(Time, Pulses)>,
    replay: bool,
}

impl Cx {
    /// A call in which the device reads `world`.
    pub fn new(world: Readings) -> Cx {
        Cx {
            world,
            ..Cx::default()
        }
    }

    /// This call, marked as one that replays a level a `Pulses` record carried. The host marks
    /// each [`PulseTarget`] call it makes from a record.
    pub fn replaying(mut self) -> Cx {
        self.replay = true;
        self
    }

    /// Whether this call replays a level a `Pulses` record carried. A record lands at its end,
    /// so the call's time is at or before the device's own: anything the device would start
    /// from it on a pin of its own would start in the past.
    pub fn replayed(&self) -> bool {
        self.replay
    }

    /// The value of `quantity`, in its unit, at the call's time: after every change strictly
    /// before it. In `run_until`, the value holds from the time the device reached to the
    /// limit, since a change of a quantity it reads stops every run there. Panics on a quantity
    /// the device does not declare in [`Device::WORLD`].
    pub fn world(&self, quantity: Quantity) -> f64 {
        self.world.get(quantity).unwrap_or_else(|| {
            panic!(
                "a device reads only a quantity it declares, not {}",
                quantity.name()
            )
        })
    }

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

    /// A screen closed the pixel write `drawn` during this call, at the call's time. A call
    /// closes at most one.
    pub fn drew(&mut self, drawn: Drawn) {
        assert!(
            self.drawn.is_none(),
            "a call closes at most one pixel write"
        );
        self.drawn = Some(drawn);
    }

    /// Queues `train` on the device's open-drain line from `start`, after the call's time and
    /// after the last train it queued has ended. A low level pulls the line low and a high one
    /// releases it. The line is released after the train, so `train.idle` is high. A call
    /// queues at most one.
    pub fn pulses(&mut self, start: Time, train: Pulses) {
        assert!(self.queued.is_none(), "a call queues at most one train");
        assert!(train.idle, "an open-drain line is released after its train");
        self.queued = Some((start, train));
    }

    /// The first rule named in this call, and its time.
    pub fn broken(&self) -> Option<(&'static str, Time)> {
        self.broken
    }

    /// Whether this call ran [`show`](Self::show).
    pub fn shown(&self) -> bool {
        self.shown
    }

    /// The pixel write this call closed.
    pub fn drawn(&self) -> Option<Drawn> {
        self.drawn
    }

    /// The train this call queued, and its start.
    pub fn queued(&self) -> Option<&(Time, Pulses)> {
        self.queued.as_ref()
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
        assert_eq!(cx.drawn(), None);
        let drawn = Drawn {
            x0: 1,
            y0: 2,
            x1: 3,
            y1: 4,
            bytes: 18,
            hash: [9; 8],
        };
        cx.drew(drawn);
        assert_eq!(cx.drawn(), Some(drawn));
    }

    #[test]
    fn a_draw_ramps_from_none_at_0_v_to_all_of_it_at_its_hold() {
        let draw = Draw {
            amps: 0.6,
            holds_above_v: 3.7,
        };
        assert_eq!(draw.at(-1.0), (0.0, 0.0));
        assert_eq!(draw.at(0.0), (0.0, 0.0));
        assert_eq!(draw.at(3.7), (0.6, 0.0));
        assert_eq!(draw.at(5.0), (0.6, 0.0));
        let (half, slope) = draw.at(1.85);
        assert!((half - 0.3).abs() < 1e-15, "{half}");
        assert!((slope - 1.5 * 0.6 / 3.7).abs() < 1e-15, "{slope}");
        let h = 1e-6;
        for volts in [0.1, 1.0, 2.0, 3.0, 3.6] {
            let (up, down) = (draw.at(volts + h).0, draw.at(volts - h).0);
            let slope = draw.at(volts).1;
            assert!(
                ((up - down) / (2.0 * h) - slope).abs() < 1e-8,
                "at {volts} V"
            );
            assert!(up >= down, "at {volts} V");
        }
    }
}
