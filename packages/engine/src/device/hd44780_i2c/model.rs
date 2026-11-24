//! An HD44780U 16×2 character LCD behind a PCF8574 I2C backpack.
//!
//! The PCF8574 is an 8-bit quasi-bidirectional I/O expander on I2C: each write sets the output
//! latch `P0..P7`, and each read returns the pin levels (`latch & driven` on `D4..D7` while the
//! HD44780 drives the bus). The common backpack wires `P0 = RS`, `P1 = RW`, `P2 = E`,
//! `P3 = backlight`, and `P4..P7 = D4..D7`.
//!
//! The HD44780U follows Hitachi ADE-207-272:
//! - Power-on reset (p. 23): 8-bit bus (`DL = 1`), 1-line (`N = 0`), 5×8 font (`F = 0`), display
//!   off (`D = 0`, `C = 0`, `B = 0`), entry increment (`I/D = 1`, `S = 0`), DDRAM filled with
//!   `0x20`, address counter 0, display shift 0, busy for 10 ms.
//! - 8-bit to 4-bit initialization (Figure 24, p. 46): Function Set with `DL = 1` (`0x30`) keeps
//!   8-bit mode and resets nibble sync; Function Set with `DL = 0` (`0x20`) enters 4-bit mode.
//! - Commands and data writes latch on `E`'s falling edge (`RW = 0`). Writes received while busy
//!   (`BF = 1`) are dropped as on the chip. Clear Display (`0x01`) and Return Home (`0x02`) hold
//!   busy for 1.52 ms; other instructions and RAM writes hold busy for 37 µs (Table 6, p. 24).
//! - Reads (`RW = 1`) drive `D4..D7` while `E` is high: `RS = 0` returns busy flag and address
//!   counter even while busy; `RS = 1` returns the byte at the current DDRAM or CGRAM address and
//!   advances the address counter on the second `E` falling edge.
//! - Any non-Function-Set operation attempted while in 8-bit mode (where `D0..D3` are not wired on
//!   a PCF8574 backpack) warns `bus_width` and is dropped. Requesting the 5×10 font (`F = 1`)
//!   warns `font` and keeps 5×8.

use orbe_model::{
    Ack, Cx, Device, Dir, Draw, Drawn, Drives, Error, I2cTarget, Levels, Panel, Params, Settings,
    Sha256, Shows, State, Stop, Time, View,
};

/// Rendered glass dimensions in pixels (320 × 68, aspect ratio 4.71 matching 64.5 × 13.8 mm).
pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 68;

/// 80 bytes of DDRAM followed by 64 bytes of CGRAM (8 custom 5×8 glyphs).
pub const DDRAM_BYTES: usize = 80;
pub const CGRAM_BYTES: usize = 64;
pub const FRAME_BYTES: usize = DDRAM_BYTES + CGRAM_BYTES;

/// Watch order: SCL, SDA, then the supply comparator.
pub const WATCH_SCL: usize = 0;
pub const WATCH_SDA: usize = 1;
pub const WATCH_VDD: usize = 2;

/// It drives no output of its own; the I2C engine drives `SDA`.
pub const OUTPUTS: usize = 0;

/// PCF8574 backpack pin bits.
pub const PIN_RS: u8 = 0x01;
pub const PIN_RW: u8 = 0x02;
pub const PIN_E: u8 = 0x04;
pub const PIN_BL: u8 = 0x08;

/// HD44780U execution times at f_osc = 270 kHz (Hitachi ADE-207-272 p. 23, Table 6 p. 24).
pub const POWER_ON_BUSY_PS: u64 = 10_000_000_000; // 10 ms internal reset
pub const CLEAR_BUSY_PS: u64 = 1_520_000_000; // 1.52 ms Clear Display / Return Home
pub const EXEC_BUSY_PS: u64 = 37_000_000; // 37 us other instructions and RAM access

/// Rule names reported via `Cx::broke`.
const BUS_WIDTH: &str = "bus_width";
const FONT: &str = "font";
const COMMAND: &str = "command";

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

/// Active 4-bit read transaction across two `E` pulses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadCycle {
    /// True for data read (`RS = 1`), false for busy flag + address (`RS = 0`).
    data: bool,
    /// Latched byte for the two nibbles (for busy+AC, bit 7 is refreshed live on read).
    byte: u8,
    /// False while outputting the high nibble, true for the low nibble.
    low_nibble: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    bytes: u64,
    hash: Sha256,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hd44780I2c {
    /// `0..80` is DDRAM (line 0 at `0..40`, line 1 at `40..80`), `80..144` is CGRAM.
    ram: [u8; FRAME_BYTES],
    powered: bool,
    /// Whether the backpack's backlight jumper is fitted.
    backlight_jumper: bool,
    /// PCF8574 8-bit port latch (`0xFF` on power-on).
    port: u8,
    /// HD44780 bus interface: false = 8-bit (power-on), true = 4-bit.
    four_bit: bool,
    /// `N`: false = 1-line, true = 2-line.
    two_line: bool,
    /// `D`: display on/off.
    display_on: bool,
    /// `C`: underline cursor on/off.
    cursor_on: bool,
    /// `B`: blinking cursor block on/off.
    blink_on: bool,
    /// `I/D`: true = increment, false = decrement.
    increment: bool,
    /// `S`: true = shift display on DDRAM write.
    auto_shift: bool,
    /// Visible window start offset along the line (0..40 in 2-line mode, 0..80 in 1-line mode).
    shift: u8,
    /// Current DDRAM address (0x00..=0x7F; row 0 is 0x00..=0x27, row 1 is 0x40..=0x67).
    ddram_addr: u8,
    /// Current CGRAM address (0x00..=0x3F).
    cgram_addr: u8,
    /// True when the address counter selects CGRAM rather than DDRAM.
    in_cgram: bool,
    /// First nibble captured in 4-bit write mode: `(rs, high_nibble_in_top_4_bits)`.
    high_nibble: Option<(bool, u8)>,
    /// Active 4-bit read cycle across two `E` pulses.
    read_cycle: Option<ReadCycle>,
    /// Internal operation finishes at this time; before it, `BF = 1`.
    busy_until: Time,
    /// Open run of RAM writes to report on STOP.
    run: Option<Run>,
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
        Ok(Hd44780I2c::reset_state(backlight_jumper, supply))
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }

    fn input(&mut self, _cx: &mut Cx, at: Time, levels: Levels) {
        let powered = levels.high(WATCH_VDD);
        if powered != self.powered {
            *self = Hd44780I2c::reset_state(self.backlight_jumper, self.supply);
            self.powered = powered;
            if powered {
                self.busy_until = Time::from_ps(at.as_ps().saturating_add(POWER_ON_BUSY_PS));
            }
        }
    }

    fn draw(&self) -> Draw {
        let amps = if !self.powered {
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
        View::Screen {
            frame: &self.ram,
            panel: Panel {
                on: self.powered && self.display_on,
                inverted: false,
                reset: !self.powered,
                asleep: !self.powered || !self.display_on,
                settings: self.settings(),
                registers: 0,
                backlight: None,
            },
        }
    }
}

impl I2cTarget for Hd44780I2c {
    fn awake(&self) -> bool {
        self.powered
    }

    fn start(&mut self, cx: &mut Cx, _at: Time, _dir: Dir) {
        if !self.powered {
            return;
        }
        self.close_run(cx);
    }

    fn write(&mut self, cx: &mut Cx, at: Time, byte: u8) -> Ack {
        if !self.powered {
            return Ack::No;
        }
        let prev = self.port;
        self.port = byte;
        let prev_e = prev & PIN_E != 0;
        let e = byte & PIN_E != 0;
        if !prev_e && e && (byte & PIN_RW != 0) {
            self.on_read_rise(cx, at, byte);
        } else if prev_e && !e {
            self.on_e_fall(cx, at, prev);
        }
        Ack::Yes
    }

    fn read(&mut self, _cx: &mut Cx, at: Time) -> u8 {
        if !self.powered {
            return 0xff;
        }
        if (self.port & PIN_RW != 0) && (self.port & PIN_E != 0) {
            let nibble = self.read_nibble(at);
            // PCF8574 quasi-bidirectional pins: a pin latched low stays low; a pin latched high
            // reads the external level driven by the HD44780 on D4..D7.
            (self.port & 0x0f) | (self.port & 0xf0 & (nibble << 4))
        } else {
            self.port
        }
    }

    fn stop(&mut self, cx: &mut Cx, _at: Time) {
        self.close_run(cx);
    }
}

impl Hd44780I2c {
    fn reset_state(backlight_jumper: bool, supply: Supply) -> Hd44780I2c {
        let mut ram = [0u8; FRAME_BYTES];
        ram[..DDRAM_BYTES].fill(0x20);
        Hd44780I2c {
            ram,
            powered: false,
            backlight_jumper,
            port: 0xff,
            four_bit: false,
            two_line: false,
            display_on: false,
            cursor_on: false,
            blink_on: false,
            increment: true,
            auto_shift: false,
            shift: 0,
            ddram_addr: 0,
            cgram_addr: 0,
            in_cgram: false,
            high_nibble: None,
            read_cycle: None,
            busy_until: Time::ZERO,
            run: None,
            supply,
        }
    }

    pub fn backlight_on(&self) -> bool {
        self.powered && self.backlight_jumper && (self.port & PIN_BL != 0)
    }

    fn settings(&self) -> Settings {
        Settings::new(
            &SETTINGS,
            [
                u16::from(self.backlight_on()),
                u16::from(self.cursor_on),
                u16::from(self.blink_on),
                u16::from(self.two_line),
                u16::from(self.shift),
                u16::from(self.current_ac()),
                u16::from(self.in_cgram),
                u16::from(self.powered),
            ],
        )
    }

    fn current_ac(&self) -> u8 {
        if self.in_cgram {
            self.cgram_addr & 0x3f
        } else {
            self.ddram_addr & 0x7f
        }
    }

    fn is_busy(&self, at: Time) -> bool {
        at < self.busy_until
    }

    fn set_busy(&mut self, at: Time, duration_ps: u64) {
        self.busy_until = Time::from_ps(at.as_ps().saturating_add(duration_ps));
    }

    fn on_read_rise(&mut self, cx: &mut Cx, at: Time, port: u8) {
        if !self.four_bit {
            cx.broke(BUS_WIDTH, at);
            self.read_cycle = None;
            return;
        }
        let data = port & PIN_RS != 0;
        if let Some(cycle) = self.read_cycle
            && cycle.low_nibble
            && cycle.data == data
        {
            return;
        }
        let byte = if data {
            if self.is_busy(at) {
                0x00
            } else {
                self.peek_ram()
            }
        } else {
            (u8::from(self.is_busy(at)) << 7) | (self.current_ac() & 0x7f)
        };
        self.read_cycle = Some(ReadCycle {
            data,
            byte,
            low_nibble: false,
        });
    }

    fn read_nibble(&self, at: Time) -> u8 {
        let Some(cycle) = self.read_cycle else {
            return 0x0f;
        };
        let byte = if !cycle.data {
            (u8::from(self.is_busy(at)) << 7) | (cycle.byte & 0x7f)
        } else {
            cycle.byte
        };
        if cycle.low_nibble {
            byte & 0x0f
        } else {
            (byte >> 4) & 0x0f
        }
    }

    fn on_e_fall(&mut self, cx: &mut Cx, at: Time, prev: u8) {
        let rs = prev & PIN_RS != 0;
        let rw = prev & PIN_RW != 0;
        if rw {
            // A real read in 8-bit mode already reported `bus_width` on `E` rising. Clearing the
            // PCF8574's power-on 0xFF latch (`RW = 1`, `E = 1`) on the first I2C write has no
            // active read cycle and is not a bus operation.
            if let Some(mut cycle) = self.read_cycle {
                if !cycle.low_nibble {
                    cycle.low_nibble = true;
                    self.read_cycle = Some(cycle);
                } else {
                    self.read_cycle = None;
                    if cycle.data && !self.is_busy(at) {
                        self.advance_ac();
                        self.set_busy(at, EXEC_BUSY_PS);
                    }
                }
            }
            return;
        }

        // Write cycle cancels any unfinished read cycle.
        self.read_cycle = None;

        if self.is_busy(at) {
            return;
        }

        let nibble = prev & 0xf0;
        if !self.four_bit {
            // In 8-bit power-on state, only D4..D7 are wired on the backpack. Function Set
            // (0x20 to switch into 4-bit mode, 0x30 during the 3x init sequence) is valid; any
            // other instruction or data write needs D0..D3 and stops loudly.
            if !rs && (nibble & 0xe0 == 0x20) {
                self.exec_function_set(cx, at, nibble);
            } else {
                cx.broke(BUS_WIDTH, at);
            }
            return;
        }

        match self.high_nibble.take() {
            None => {
                self.high_nibble = Some((rs, nibble));
            }
            Some((hi_rs, hi)) => {
                if hi_rs != rs {
                    cx.broke(COMMAND, at);
                    return;
                }
                let byte = hi | (nibble >> 4);
                if rs {
                    self.write_data(at, byte);
                } else {
                    self.exec_command(cx, at, byte);
                }
            }
        }
    }

    fn exec_function_set(&mut self, cx: &mut Cx, at: Time, byte: u8) {
        let dl_8bit = byte & 0x10 != 0;
        let two_line = byte & 0x08 != 0;
        let font_5x10 = byte & 0x04 != 0;
        if !dl_8bit && font_5x10 {
            cx.broke(FONT, at);
        }
        self.four_bit = !dl_8bit;
        self.two_line = two_line;
        self.high_nibble = None;
        self.read_cycle = None;
        self.set_busy(at, EXEC_BUSY_PS);
    }

    fn exec_command(&mut self, cx: &mut Cx, at: Time, byte: u8) {
        if byte == 0x00 {
            return;
        }
        if byte == 0x01 {
            // Clear Display (Table 6, p. 24 & p. 28).
            self.ram[..DDRAM_BYTES].fill(0x20);
            self.ddram_addr = 0;
            self.in_cgram = false;
            self.shift = 0;
            self.increment = true;
            self.record_write(byte);
            self.set_busy(at, CLEAR_BUSY_PS);
        } else if byte & 0xfe == 0x02 {
            // Return Home (Table 6, p. 24 & p. 28).
            self.ddram_addr = 0;
            self.in_cgram = false;
            self.shift = 0;
            self.set_busy(at, CLEAR_BUSY_PS);
        } else if byte & 0xfc == 0x04 {
            // Entry Mode Set.
            self.increment = byte & 0x02 != 0;
            self.auto_shift = byte & 0x01 != 0;
            self.set_busy(at, EXEC_BUSY_PS);
        } else if byte & 0xf8 == 0x08 {
            // Display ON/OFF Control.
            self.display_on = byte & 0x04 != 0;
            self.cursor_on = byte & 0x02 != 0;
            self.blink_on = byte & 0x01 != 0;
            self.set_busy(at, EXEC_BUSY_PS);
        } else if byte & 0xf0 == 0x10 {
            // Cursor or Display Shift.
            let display_shift = byte & 0x08 != 0;
            let right = byte & 0x04 != 0;
            if display_shift {
                self.shift_display(right);
            } else {
                self.move_ac(right);
            }
            self.set_busy(at, EXEC_BUSY_PS);
        } else if byte & 0xe0 == 0x20 {
            self.exec_function_set(cx, at, byte);
        } else if byte & 0xc0 == 0x40 {
            // Set CGRAM Address.
            self.in_cgram = true;
            self.cgram_addr = byte & 0x3f;
            self.set_busy(at, EXEC_BUSY_PS);
        } else if byte & 0x80 == 0x80 {
            // Set DDRAM Address.
            self.in_cgram = false;
            self.ddram_addr = byte & 0x7f;
            self.set_busy(at, EXEC_BUSY_PS);
        }
    }

    fn write_data(&mut self, at: Time, byte: u8) {
        if self.in_cgram {
            let idx = DDRAM_BYTES + usize::from(self.cgram_addr & 0x3f);
            self.ram[idx] = byte & 0x1f;
        } else {
            let idx = self.ddram_index(self.ddram_addr);
            self.ram[idx] = byte;
            if self.auto_shift {
                // With I/D = 1 (increment), display shifts left; with I/D = 0, shifts right.
                self.shift_display(!self.increment);
            }
        }
        self.advance_ac();
        self.record_write(byte);
        self.set_busy(at, EXEC_BUSY_PS);
    }

    fn peek_ram(&self) -> u8 {
        if self.in_cgram {
            self.ram[DDRAM_BYTES + usize::from(self.cgram_addr & 0x3f)]
        } else {
            self.ram[self.ddram_index(self.ddram_addr)]
        }
    }

    fn ddram_index(&self, addr: u8) -> usize {
        if self.two_line {
            if addr >= 0x40 {
                40 + usize::from(addr - 0x40) % 40
            } else {
                usize::from(addr) % 40
            }
        } else {
            usize::from(addr) % DDRAM_BYTES
        }
    }

    fn advance_ac(&mut self) {
        self.move_ac(self.increment);
    }

    fn move_ac(&mut self, forward: bool) {
        if self.in_cgram {
            self.cgram_addr = if forward {
                (self.cgram_addr + 1) & 0x3f
            } else {
                self.cgram_addr.wrapping_sub(1) & 0x3f
            };
            return;
        }
        self.ddram_addr = if self.two_line {
            if forward {
                match self.ddram_addr {
                    0x27 => 0x40,
                    0x67 => 0x00,
                    a => (a + 1) & 0x7f,
                }
            } else {
                match self.ddram_addr {
                    0x00 => 0x67,
                    0x40 => 0x27,
                    a => a.wrapping_sub(1) & 0x7f,
                }
            }
        } else if forward {
            if self.ddram_addr >= 0x4f {
                0x00
            } else {
                self.ddram_addr + 1
            }
        } else if self.ddram_addr == 0x00 {
            0x4f
        } else {
            self.ddram_addr - 1
        };
    }

    fn shift_display(&mut self, right: bool) {
        let span: u8 = if self.two_line { 40 } else { 80 };
        self.shift = if right {
            (self.shift + span - 1) % span
        } else {
            (self.shift + 1) % span
        };
    }

    fn record_write(&mut self, byte: u8) {
        let run = self.run.get_or_insert(Run {
            bytes: 0,
            hash: Sha256::new(),
        });
        run.bytes += 1;
        run.hash.update(&[byte]);
    }

    fn close_run(&mut self, cx: &mut Cx) {
        let Some(run) = self.run.take() else {
            return;
        };
        let mut hash = [0u8; 8];
        hash.copy_from_slice(&run.hash.finish()[..8]);
        cx.drew(Drawn {
            x0: 0,
            y0: 0,
            x1: WIDTH - 1,
            y1: HEIGHT - 1,
            bytes: run.bytes,
            hash,
        });
    }

    #[cfg(test)]
    pub(crate) fn ddram(&self) -> &[u8] {
        &self.ram[..DDRAM_BYTES]
    }

    #[cfg(test)]
    pub(crate) fn cgram(&self) -> &[u8] {
        &self.ram[DDRAM_BYTES..]
    }

    #[cfg(test)]
    pub(crate) fn is_four_bit(&self) -> bool {
        self.four_bit
    }

    #[cfg(test)]
    pub(crate) fn ddram_addr(&self) -> u8 {
        self.ddram_addr
    }
}
