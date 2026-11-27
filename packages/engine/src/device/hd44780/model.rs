//! An HD44780U 16×2 character LCD on its own parallel bus, the device of the `hd44780` kind.
//!
//! The pins are the module's: `RS`, `R/W`, `E` and `DB0..DB7`, 4-bit or 8-bit, after Hitachi
//! ADE-207-272. An `E` pulse, a rise then a fall, is one bus cycle. A write latches `RS` and
//! the data on the fall. A read with `R/W` high drives the busy flag and address, or the data,
//! from `tDDR` after the rise to `tDHR` after the fall: `DB7..DB4` in 4-bit mode and all eight
//! lines in 8-bit mode. Each cycle checks the bus timing of the sheet's table for the supply,
//! p. 52 at 4.5 V and above and p. 49 below. A cycle that breaks one still runs and warns:
//! `enable_cycle` (tcycE), `enable_width` (PWEH), `setup` (tAS, tDSW) or `hold` (tAH, tH).
//!
//! `DB0..DB7`, `RS` and `R/W` hold their MOS pull-ups (p. 51), so lines a 4-bit circuit leaves
//! open read high. The glass shows text only while the backlight is lit and `VDD - V0` is at
//! least the LCD voltage floor, VLCD1 (p. 51). The display on below that warns `contrast`.
//!
//! [`controller`] is the HD44780U itself, and [`backpack`] the `hd44780_i2c` front end on it.

#[path = "backpack.rs"]
pub mod backpack;
#[path = "controller.rs"]
pub mod controller;

use orbe_model::{
    Cx, Device, Draw, Drive, Drives, Error, Levels, Panel, Params, Settings, Shows, State, Stop,
    Time, View,
};

use controller::{Controller, HEIGHT, WIDTH};

/// Watch order: `RS`, `R/W`, `E`, `DB0..DB7`, then the 2.7 V and 4.5 V supply comparators,
/// `VDD - V0` at the LCD voltage floor, and the backlight's `A - K` at its lit voltage.
pub const WATCH_RS: usize = 0;
pub const WATCH_RW: usize = 1;
pub const WATCH_E: usize = 2;
pub const WATCH_DB0: usize = 3;
pub const WATCH_VDD: usize = 11;
pub const WATCH_VDD_4V5: usize = 12;
pub const WATCH_CONTRAST: usize = 13;
pub const WATCH_BACKLIGHT: usize = 14;
/// The bus levels in `prev`: `RS`, `R/W`, `E` and `DB0..DB7`.
const BUS: usize = 11;

/// `DB0..DB7`, then `RS` and `R/W`, each with its pull-up.
pub const OUTPUTS: usize = 10;
const OUT_RS: usize = 8;
const OUT_RW: usize = 9;

/// Hitachi ADE-207-272 Table 6, operating voltage minimum, 2.7 V.
pub const MIN_VDD_V: f64 = 2.7;

/// Rule names reported via `Cx::broke`.
const ENABLE_CYCLE: &str = "enable_cycle";
const ENABLE_WIDTH: &str = "enable_width";
const SETUP: &str = "setup";
const HOLD: &str = "hold";
const CONTRAST: &str = "contrast";

/// Panel settings, in [`Hd44780::settings`] order.
pub const SETTINGS: [&str; 9] = [
    "backlight",
    "cursor",
    "blink",
    "two_line",
    "shift",
    "ac",
    "in_cgram",
    "powered",
    "contrast",
];

/// Bus timing minimums and the read's data delay maximum, in picoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    /// tcycE.
    cycle: u64,
    /// PWEH.
    width: u64,
    /// tAS, `RS` and `R/W` to `E` rising.
    address_setup: u64,
    /// tAH, after `E` falls.
    address_hold: u64,
    /// tDSW, data to `E` falling.
    data_setup: u64,
    /// tH, data after `E` falls on a write.
    data_hold: u64,
    /// tDDR maximum, `E` rising to data out on a read.
    data_delay: u64,
    /// tDHR, data out held after `E` falls on a read.
    read_hold: u64,
}

/// Bus Timing Characteristics at VCC = 4.5 to 5.5 V, p. 52.
const FAST: Timing = Timing {
    cycle: 500_000,
    width: 230_000,
    address_setup: 40_000,
    address_hold: 10_000,
    data_setup: 80_000,
    data_hold: 10_000,
    data_delay: 160_000,
    read_hold: 5_000,
};

/// Bus Timing Characteristics at VCC = 2.7 to 4.5 V, p. 49.
const SLOW: Timing = Timing {
    cycle: 1_000_000,
    width: 450_000,
    address_setup: 60_000,
    address_hold: 20_000,
    data_setup: 195_000,
    data_hold: 10_000,
    data_delay: 360_000,
    read_hold: 5_000,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hd44780 {
    controller: Controller,
    logic_amps: f64,
    holds_above_v: f64,
    /// The bus levels at the last input while powered.
    prev: Option<[bool; BUS]>,
    /// When each bus level last changed.
    changed: [Time; BUS],
    /// The last `E` rise while powered.
    rose: Option<Time>,
    /// The last `E` fall that ended a cycle, and whether that cycle wrote.
    fell: Option<(Time, bool)>,
    /// Whether `VDD` is at 4.5 V, which selects the faster table.
    fast: bool,
    /// Whether `VDD - V0` is at the LCD voltage floor.
    contrast: bool,
    /// Whether the backlight's `A - K` is at its lit voltage.
    backlight: bool,
    /// The byte a read drives, and whether only `DB7..DB4` carry it. `None` while released.
    out: Option<(u8, bool)>,
    /// A read's drive turns on (`true`) or off at this time.
    due: Option<(Time, bool)>,
}

impl State for Hd44780 {}

impl Device for Hd44780 {
    const SHOWS: Shows = Shows::Screen {
        kind: "hd44780",
        width: WIDTH,
        height: HEIGHT,
        format: "hd44780",
    };

    const MIN_SUPPLY_V: Option<f64> = Some(MIN_VDD_V);

    fn new(params: &Params<'_>) -> Result<Hd44780, Error> {
        let logic_amps = match params.int("logic_na").unwrap_or(0) {
            nanoamps @ 0.. => nanoamps as f64 * 1e-9,
            _ => return Err(Error("a draw is 0 or more nanoamperes")),
        };
        let holds_above_v = match params.int("holds_above_mv").unwrap_or(2_700) {
            millivolts @ 1.. => millivolts as f64 * 1e-3,
            _ => return Err(Error("holds_above_mv is more than 0")),
        };
        Ok(Hd44780 {
            controller: Controller::new(true),
            logic_amps,
            holds_above_v,
            prev: None,
            changed: [Time::ZERO; BUS],
            rose: None,
            fell: None,
            fast: false,
            contrast: false,
            backlight: false,
            out: None,
            due: None,
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        let (at, on) = self.due.filter(|&(at, _)| at <= limit)?;
        self.due = None;
        self.out = on.then(|| (self.controller.read_bus(at), self.controller.four_bit()));
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        let mut drives = [Drive::PullUp; OUTPUTS];
        if let Some((byte, four_bit)) = self.out {
            let first = if four_bit { 4 } else { 0 };
            for (line, drive) in drives.iter_mut().enumerate().take(8).skip(first) {
                *drive = if byte & (1 << line) == 0 {
                    Drive::Low
                } else {
                    Drive::High
                };
            }
        }
        drives[OUT_RS] = Drive::PullUp;
        drives[OUT_RW] = Drive::PullUp;
        Drives::new(drives)
    }

    fn input(&mut self, cx: &mut Cx, at: Time, levels: Levels) {
        let powered = levels.high(WATCH_VDD);
        if self
            .controller
            .supply(at, powered, levels.high(WATCH_VDD_4V5))
        {
            self.prev = None;
            self.rose = None;
            self.fell = None;
            if self.out.is_some() || self.due.is_some() {
                self.due = Some((later(at, 1), false));
            }
        }
        self.fast = powered && levels.high(WATCH_VDD_4V5);
        self.contrast = levels.high(WATCH_CONTRAST);
        self.backlight = levels.high(WATCH_BACKLIGHT);
        if !powered {
            return;
        }
        let now: [bool; BUS] = std::array::from_fn(|watch| levels.high(watch));
        let Some(prev) = self.prev.replace(now) else {
            self.changed = [at; BUS];
            self.check_contrast(cx, at);
            return;
        };
        let rise = !prev[WATCH_E] && now[WATCH_E];
        let fall = prev[WATCH_E] && !now[WATCH_E];
        if fall {
            self.e_fall(cx, at, &prev);
        }
        self.changes(cx, at, &prev, &now);
        if rise {
            self.e_rise(cx, at, &now);
        }
        self.check_contrast(cx, at);
    }

    fn draw(&self) -> Draw {
        Draw {
            amps: if self.controller.powered() {
                self.logic_amps
            } else {
                0.0
            },
            holds_above_v: self.holds_above_v,
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

fn later(at: Time, ps: u64) -> Time {
    Time::from_ps(at.as_ps().saturating_add(ps))
}

fn since(at: Time, then: Time) -> u64 {
    at.as_ps().saturating_sub(then.as_ps())
}

impl Hd44780 {
    fn timing(&self) -> Timing {
        if self.fast { FAST } else { SLOW }
    }

    /// The bus lines that changed at `at`. A line that moves within its hold after the last
    /// fall breaks `hold`, and so does `RS` or `R/W` moving while `E` is high. `E` high with
    /// no rise since power-on is an undriven pin, as at a sketch's start, not a cycle.
    fn changes(&mut self, cx: &mut Cx, at: Time, prev: &[bool; BUS], now: &[bool; BUS]) {
        let timing = self.timing();
        for watch in 0..BUS {
            if watch == WATCH_E || prev[watch] == now[watch] {
                continue;
            }
            let address = watch == WATCH_RS || watch == WATCH_RW;
            let hold = match self.fell {
                Some((_, _)) if address => timing.address_hold,
                Some((_, true)) => timing.data_hold,
                _ => 0,
            };
            if self.fell.is_some_and(|(fell, _)| since(at, fell) < hold)
                || (address && prev[WATCH_E] && now[WATCH_E] && self.rose.is_some())
            {
                cx.broke(HOLD, at);
            }
            if watch == WATCH_RW && prev[WATCH_E] && now[WATCH_E] && self.out.is_some() {
                self.due = Some((later(at, timing.read_hold), false));
            }
            self.changed[watch] = at;
        }
    }

    fn e_rise(&mut self, cx: &mut Cx, at: Time, now: &[bool; BUS]) {
        let timing = self.timing();
        let address = self.changed[WATCH_RS].max(self.changed[WATCH_RW]);
        if since(at, address) < timing.address_setup {
            cx.broke(SETUP, at);
        }
        if self.rose.is_some_and(|rose| since(at, rose) < timing.cycle) {
            cx.broke(ENABLE_CYCLE, at);
        }
        self.rose = Some(at);
        if now[WATCH_RW] {
            self.controller.read_rise(cx, at, now[WATCH_RS]);
            self.due = Some((later(at, timing.data_delay), true));
        }
    }

    /// `E` fell at `at`. `prev` holds the levels just before. A fall with no rise seen since
    /// power-on is a line settling, not a cycle.
    fn e_fall(&mut self, cx: &mut Cx, at: Time, prev: &[bool; BUS]) {
        let Some(rose) = self.rose else {
            return;
        };
        let timing = self.timing();
        if since(at, rose) < timing.width {
            cx.broke(ENABLE_WIDTH, at);
        }
        let rs = prev[WATCH_RS];
        let rw = prev[WATCH_RW];
        let first = if self.controller.four_bit() { 4 } else { 0 };
        let mut bus = 0u8;
        for line in 0..8 {
            if prev[WATCH_DB0 + line] {
                bus |= 1 << line;
            }
        }
        if rw {
            self.due = Some((later(at, timing.read_hold), false));
        } else {
            let data = (first..8)
                .map(|line| self.changed[WATCH_DB0 + line])
                .max()
                .unwrap_or(Time::ZERO);
            if since(at, data) < timing.data_setup {
                cx.broke(SETUP, at);
            }
            if !rs {
                // An instruction ends the run of RAM writes before it.
                self.controller.close_run(cx);
            }
        }
        self.controller.e_fall(cx, at, rs, rw, bus);
        self.fell = Some((at, !rw));
    }

    /// The display on, powered, with `VDD - V0` under the LCD voltage floor shows nothing.
    fn check_contrast(&self, cx: &mut Cx, at: Time) {
        if cx.broken().is_none()
            && self.controller.powered()
            && self.controller.display_on()
            && !self.contrast
        {
            cx.broke(CONTRAST, at);
        }
    }

    pub fn backlight_on(&self) -> bool {
        self.controller.powered() && self.backlight
    }

    fn settings(&self) -> Settings {
        let c = &self.controller;
        Settings::new(
            &SETTINGS,
            [
                u32::from(self.backlight_on()),
                u32::from(c.cursor_on()),
                u32::from(c.blink_on()),
                u32::from(c.two_line()),
                u32::from(c.shift()),
                u32::from(c.current_ac()),
                u32::from(c.in_cgram()),
                u32::from(c.powered()),
                u32::from(self.contrast),
            ],
        )
    }

    #[cfg(test)]
    pub(crate) fn ddram(&self) -> &[u8] {
        &self.controller.ram()[..controller::DDRAM_BYTES]
    }

    #[cfg(test)]
    pub(crate) fn is_four_bit(&self) -> bool {
        self.controller.four_bit()
    }
}
