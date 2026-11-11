//! The CST816 touch controller as a device: its register file, `INT` and `RST`. The host's I2C
//! engine runs `SCL` and `SDA` and hands this device each transfer's bytes.
//!
//! A touch is an event. The drives do not change then. `INT` falls 1 ps later and rises after
//! [`INT_PULSE_PS`]. That width is a guess.

use orbe_model::{
    Ack, Cx, Device, Dir, Drive, Drives, Error, Event, Fields, I2cTarget, Levels, Params, State,
    Stop, Time,
};

/// Watch order: SCL, SDA, INT, RST.
pub const WATCH_SCL: usize = 0;
pub const WATCH_SDA: usize = 1;
pub const WATCH_INT: usize = 2;
pub const WATCH_RST: usize = 3;

/// It drives one output of its own, `INT`.
pub const OUTPUTS: usize = 1;

/// Register addresses 0x01 to 0x06 and 0xA7. The layout the open drivers read, with no
/// datasheet in hand.
pub const REG_GESTURE: usize = 0x01;
pub const REG_FINGERS: usize = 0x02;
pub const REG_X_HIGH: usize = 0x03;
pub const REG_X_LOW: usize = 0x04;
pub const REG_Y_HIGH: usize = 0x05;
pub const REG_Y_LOW: usize = 0x06;
pub const REG_CHIP: usize = 0xA7;

/// The byte those drivers expect at [`REG_CHIP`] for a CST816S. A guess, with no sheet in hand.
pub const CHIP_ID: u8 = 0xB4;

/// How long INT stays low after the 1 ps fall. A guess.
pub const INT_PULSE_PS: u64 = 100_000_000;

/// The register file, `INT`, and whether `RST` holds it in reset.
///
/// A write transfer's first data byte sets `pointer`. Each later write byte, and each read byte,
/// uses `pointer` and then advances it by one, wrapping at 256. That is the burst the open
/// drivers read. The advance is inferred from that burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cst816 {
    mem: [u8; 256],
    pointer: u8,
    set_pointer: bool,
    in_reset: bool,
    int: Drive,
    /// INT falls at this time.
    int_low: Option<Time>,
    /// INT releases at this time.
    int_high: Option<Time>,
}

impl State for Cst816 {}

/// One point of a touch: the finger is down at `(x, y)`, or up.
struct Point {
    x: u16,
    y: u16,
    down: bool,
}

impl Point {
    fn of(fields: Fields<'_>) -> Option<Point> {
        Some(Point {
            x: u16::try_from(fields.int("x")?).ok()?,
            y: u16::try_from(fields.int("y")?).ok()?,
            down: fields.bool("down")?,
        })
    }
}

impl Cst816 {
    const fn power_on() -> Cst816 {
        let mut mem = [0; 256];
        mem[REG_CHIP] = CHIP_ID;
        Cst816 {
            mem,
            pointer: 0,
            set_pointer: false,
            in_reset: false,
            int: Drive::Off,
            int_low: None,
            int_high: None,
        }
    }

    fn put(&mut self, point: Point) {
        self.mem[REG_GESTURE] = 0;
        self.mem[REG_FINGERS] = u8::from(point.down);
        if !point.down {
            return;
        }
        self.mem[REG_X_HIGH] = ((point.x >> 8) as u8) & 0x0F;
        self.mem[REG_X_LOW] = point.x as u8;
        self.mem[REG_Y_HIGH] = ((point.y >> 8) as u8) & 0x0F;
        self.mem[REG_Y_LOW] = point.y as u8;
    }

    fn release_int(&mut self, at: Time) {
        self.int_low = None;
        if self.int == Drive::Low || self.int_high.is_some() {
            self.int_high = at.as_ps().checked_add(1).map(Time::from_ps);
        }
    }
}

impl Device for Cst816 {
    fn new(_params: &Params<'_>) -> Result<Cst816, Error> {
        Ok(Cst816::power_on())
    }

    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        let at = [self.int_low, self.int_high]
            .into_iter()
            .flatten()
            .min()
            .filter(|&at| at <= limit)?;
        if self.int_low == Some(at) {
            self.int_low = None;
            self.int = Drive::Low;
        }
        if self.int_high == Some(at) {
            self.int_high = None;
            if self.int_low.is_none() {
                self.int = Drive::Off;
            }
        }
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        Drives::new([self.int])
    }

    /// `RST` low clears the register file and releases `INT` 1 ps later.
    fn input(&mut self, _cx: &mut Cx, at: Time, levels: Levels) {
        let reset = !levels.high(WATCH_RST);
        if reset && !self.in_reset {
            self.mem = Cst816::power_on().mem;
            self.pointer = 0;
            self.set_pointer = false;
            self.release_int(at);
        }
        self.in_reset = reset;
    }

    /// A `touch` sets the point registers. INT falls 1 ps later, or stays low longer when it
    /// is already low.
    fn event(&mut self, _cx: &mut Cx, at: Time, event: Event<'_>) {
        let Event::Declared {
            name: "touch",
            fields,
        } = event
        else {
            return;
        };
        let Some(point) = Point::of(fields) else {
            return;
        };
        if self.in_reset {
            return;
        }
        self.put(point);
        let fall = Time::from_ps(at.as_ps().saturating_add(1));
        let rise = Time::from_ps(fall.as_ps().saturating_add(INT_PULSE_PS));
        let already = self.int == Drive::Low || self.int_low.is_some();
        if !already {
            self.int_low = Some(fall);
        }
        self.int_high = Some(match self.int_high {
            Some(old) if already => old.max(rise),
            _ => rise,
        });
    }
}

impl I2cTarget for Cst816 {
    fn start(&mut self, _cx: &mut Cx, _at: Time, dir: Dir) {
        if dir == Dir::Write {
            self.set_pointer = true;
        }
    }

    fn write(&mut self, _cx: &mut Cx, _at: Time, byte: u8) -> Ack {
        if self.set_pointer {
            self.pointer = byte;
            self.set_pointer = false;
            return Ack::Yes;
        }
        self.mem[usize::from(self.pointer)] = byte;
        self.pointer = self.pointer.wrapping_add(1);
        Ack::Yes
    }

    fn read(&mut self, _cx: &mut Cx, _at: Time) -> u8 {
        let byte = self.mem[usize::from(self.pointer)];
        self.pointer = self.pointer.wrapping_add(1);
        byte
    }
}
