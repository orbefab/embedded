//! An HD44780U 16×2 character LCD behind a PCF8574 I2C backpack, the device of the
//! `hd44780_i2c` kind.
//!
//! The PCF8574 is an 8-bit quasi-bidirectional I/O expander on I2C: each write sets the output
//! latch `P0..P7`, and each read returns the pin levels (`latch & driven` on `D4..D7` while the
//! HD44780 drives the bus). The common backpack wires `P0 = RS`, `P1 = RW`, `P2 = E`,
//! `P3 = backlight`, and `P4..P7 = D4..D7`, and leaves `D0..D3` unwired. The
//! [`Controller`](super::controller::Controller) does the rest.

use orbe_model::{
    Ack, Cx, Device, Dir, Draw, Drives, Error, I2cTarget, Levels, Panel, Params, Settings, Shows,
    State, Stop, Time, View,
};

use super::controller::{Controller, HEIGHT, WIDTH};

/// Watch order: SCL, SDA, 2.7 V operating supply comparator, and 4.5 V fast-init comparator.
pub const WATCH_SCL: usize = 0;
pub const WATCH_SDA: usize = 1;
pub const WATCH_VDD: usize = 2;
pub const WATCH_VDD_4V5: usize = 3;

/// It drives no output of its own; the I2C engine drives `SDA`.
pub const OUTPUTS: usize = 0;

/// PCF8574 backpack pin bits.
pub const PIN_RS: u8 = 0x01;
pub const PIN_RW: u8 = 0x02;
pub const PIN_E: u8 = 0x04;
pub const PIN_BL: u8 = 0x08;

/// Panel settings, in [`Hd44780I2c::settings`] order.
pub const SETTINGS: [&str; 8] = [
    "backlight",
    "cursor",
    "blink",
    "two_line",
    "shift",
    "ac",
    "in_cgram",
    "powered",
];

#[derive(Clone, Copy, Debug, PartialEq)]
struct Supply {
    logic: f64,
    backlight: f64,
    holds_above_v: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hd44780I2c {
    controller: Controller,
    /// Whether the backpack's backlight jumper is fitted.
    backlight_jumper: bool,
    /// PCF8574 8-bit port latch (`0xFF` on power-on).
    port: u8,
    supply: Supply,
}

impl State for Hd44780I2c {}

impl Device for Hd44780I2c {
    const SHOWS: Shows = Shows::Screen {
        kind: "hd44780_i2c",
        width: WIDTH,
        height: HEIGHT,
        format: "hd44780",
    };

    fn new(params: &Params<'_>) -> Result<Hd44780I2c, Error> {
        let amps = |field| match params.int(field).unwrap_or(0) {
            nanoamps @ 0.. => Ok(nanoamps as f64 * 1e-9),
            _ => Err(Error("a draw is 0 or more nanoamperes")),
        };
        let holds_above_v = match params.int("holds_above_mv").unwrap_or(1_000) {
            millivolts @ 1.. => millivolts as f64 * 1e-3,
            _ => return Err(Error("holds_above_mv is more than 0")),
        };
        let backlight_jumper = params.bool("backlight_jumper").unwrap_or(true);
        let supply = Supply {
            logic: amps("logic_na")?,
            backlight: amps("backlight_na")?,
            holds_above_v,
        };
        Ok(Hd44780I2c {
            controller: Controller::new(false),
            backlight_jumper,
            port: 0xff,
            supply,
        })
    }

    /// Each switch of the blinking character while `B = 1`.
    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        let at = self.controller.next_blink().filter(|&at| at <= limit)?;
        self.controller.blink_at(at);
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }

    fn input(&mut self, _cx: &mut Cx, at: Time, levels: Levels) {
        let powered = levels.high(WATCH_VDD);
        let fast = levels.high(WATCH_VDD_4V5);
        if self.controller.supply(at, powered, fast) {
            self.port = 0xff;
        }
    }

    fn draw(&self) -> Draw {
        let amps = if !self.controller.powered() {
            0.0
        } else if self.backlight_on() {
            self.supply.logic + self.supply.backlight
        } else {
            self.supply.logic
        };
        Draw {
            amps,
            holds_above_v: self.supply.holds_above_v,
        }
    }

    fn view(&self) -> View<'_> {
        let powered = self.controller.powered();
        let display_on = self.controller.display_on();
        View::Screen {
            frame: self.controller.ram(),
            panel: Panel {
                on: powered && display_on,
                inverted: false,
                reset: !powered,
                asleep: !powered || !display_on,
                settings: self.settings(),
                registers: 0,
                backlight: None,
            },
        }
    }
}

impl I2cTarget for Hd44780I2c {
    fn awake(&self) -> bool {
        self.controller.powered()
    }

    fn start(&mut self, cx: &mut Cx, _at: Time, _dir: Dir) {
        if !self.controller.powered() {
            return;
        }
        self.controller.close_run(cx);
    }

    fn write(&mut self, cx: &mut Cx, at: Time, byte: u8) -> Ack {
        if !self.controller.powered() {
            return Ack::No;
        }
        let prev = self.port;
        self.port = byte;
        let prev_e = prev & PIN_E != 0;
        let e = byte & PIN_E != 0;
        if !prev_e && e && (byte & PIN_RW != 0) {
            self.controller.read_rise(cx, at, byte & PIN_RS != 0);
        } else if prev_e && !e {
            let rs = prev & PIN_RS != 0;
            let rw = prev & PIN_RW != 0;
            self.controller.e_fall(cx, at, rs, rw, prev);
        }
        Ack::Yes
    }

    fn read(&mut self, _cx: &mut Cx, at: Time) -> u8 {
        if !self.controller.powered() {
            return 0xff;
        }
        if (self.port & PIN_RW != 0) && (self.port & PIN_E != 0) {
            let bus = self.controller.read_bus(at);
            // PCF8574 quasi-bidirectional pins: a pin latched low stays low; a pin latched high
            // reads the external level driven by the HD44780 on D4..D7.
            (self.port & 0x0f) | (self.port & 0xf0 & bus)
        } else {
            self.port
        }
    }

    fn stop(&mut self, cx: &mut Cx, _at: Time) {
        self.controller.close_run(cx);
    }
}

impl Hd44780I2c {
    pub fn backlight_on(&self) -> bool {
        self.controller.powered() && self.backlight_jumper && (self.port & PIN_BL != 0)
    }

    fn settings(&self) -> Settings {
        let c = &self.controller;
        Settings::new(
            &SETTINGS,
            [
                u32::from(self.backlight_on()),
                u32::from(c.cursor_on()),
                super::blink(c),
                u32::from(c.two_line()),
                u32::from(c.shift()),
                u32::from(c.current_ac()),
                u32::from(c.in_cgram()),
                u32::from(c.powered()),
            ],
        )
    }

    #[cfg(test)]
    pub(crate) fn ddram(&self) -> &[u8] {
        &self.controller.ram()[..super::controller::DDRAM_BYTES]
    }

    #[cfg(test)]
    pub(crate) fn cgram(&self) -> &[u8] {
        &self.controller.ram()[super::controller::DDRAM_BYTES..]
    }

    #[cfg(test)]
    pub(crate) fn is_four_bit(&self) -> bool {
        self.controller.four_bit()
    }

    #[cfg(test)]
    pub(crate) fn ddram_addr(&self) -> u8 {
        self.controller.ddram_addr()
    }
}
