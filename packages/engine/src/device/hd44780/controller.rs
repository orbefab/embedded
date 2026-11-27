//! The Hitachi HD44780U controller (ADE-207-272), shared by the parallel `hd44780` and the
//! PCF8574 `hd44780_i2c`. Each front end turns its pins into `E` edges with the levels of
//! `RS`, `R/W` and `DB0..DB7`, and the controller does the rest.
//!
//! - Power-on reset (p. 23, Figs. 23–24 pp. 45–46): 8-bit bus (`DL = 1`), 1-line (`N = 0`), 5×8
//!   font (`F = 0`), display off (`D = 0`, `C = 0`, `B = 0`), entry increment (`I/D = 1`, `S = 0`),
//!   DDRAM filled with `0x20`, address counter 0, display shift 0. Wait more than 15 ms after `VCC`
//!   rises to 4.5 V, or more than 40 ms after `VCC` rises to 2.7 V, before the first Function Set.
//!   The internal reset holds the busy flag for 10 ms after `VCC` rises to 4.5 V (p. 23), and a
//!   write inside it is dropped and warns `init_busy`. Below 4.5 V the sheet gives no busy time,
//!   so the 40 ms wait is the busy time too. A write after the busy time and before the wait
//!   runs, and warns `init_wait`.
//! - 8-bit to 4-bit initialization (Figure 24, p. 46): Function Set with `DL = 1` (`0x30`) keeps
//!   8-bit mode and resets nibble sync; Function Set with `DL = 0` (`0x20`) enters 4-bit mode.
//! - Commands and data writes latch on `E`'s falling edge (`RW = 0`). Writes received while busy
//!   (`BF = 1`) are dropped as on the chip. Clear Display (`0x01`) and Return Home (`0x02`) hold
//!   busy for 1.52 ms; other instructions and RAM writes hold busy for 37 µs (Table 6, p. 24).
//! - Reads (`RW = 1`) drive the bus while `E` is high: `RS = 0` returns busy flag and address
//!   counter even while busy; `RS = 1` returns the byte at the current DDRAM or CGRAM address and
//!   advances the address counter when the read ends, on the second `E` falling edge in 4-bit
//!   mode and on the first in 8-bit mode.
//! - With `DB0..DB3` not wired, as on a PCF8574 backpack, any operation in 8-bit mode other than
//!   Function Set warns `bus_width` and is dropped. Requesting one line of the 5×10 font (`N = 0`,
//!   `F = 1`) warns `font` and keeps 5×8. With two lines `F` is don't care (Table 8, p. 29).
//! - With `B = 1` the character at the cursor switches between all its dots and itself every
//!   409.6 ms at f_osc = 250 kHz (p. 26), so every [`BLINK_PS`] at 270 kHz. The blink counter
//!   runs from power-on, and [`Controller::next_blink`] is each switch while the display is on.

use orbe_model::{Cx, Drawn, Sha256, Time};

/// Rendered glass dimensions in pixels (320 × 68, aspect ratio 4.71 matching 64.5 × 13.8 mm).
pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 68;

/// 80 bytes of DDRAM followed by 64 bytes of CGRAM (8 custom 5×8 glyphs).
pub const DDRAM_BYTES: usize = 80;
pub const CGRAM_BYTES: usize = 64;
pub const FRAME_BYTES: usize = DDRAM_BYTES + CGRAM_BYTES;

/// HD44780U power-on reset waits (Hitachi ADE-207-272 Figs. 23–24, pp. 45–46) and instruction
/// execution times at f_osc = 270 kHz (Table 6, p. 24).
pub const POWER_ON_WAIT_4V5_PS: u64 = 15_000_000_000; // 15 ms after VCC rises to 4.5 V
pub const POWER_ON_WAIT_2V7_PS: u64 = 40_000_000_000; // 40 ms after VCC rises to 2.7 V
/// The internal reset's busy time, 10 ms after VCC rises to 4.5 V (p. 23).
pub const POWER_ON_BUSY_4V5_PS: u64 = 10_000_000_000;
pub const CLEAR_BUSY_PS: u64 = 1_520_000_000; // 1.52 ms Clear Display / Return Home
pub const EXEC_BUSY_PS: u64 = 37_000_000; // 37 us other instructions and RAM access
/// The blink's half period: 409.6 ms at f_osc = 250 kHz (p. 26), 102,400 clocks, which at the
/// 270 kHz of Table 6 is 409.6 ms x 250 / 270, the sheet's own 379.2 ms example.
pub const BLINK_PS: u64 = 102_400 * 1_000_000_000_000 / 270_000;

/// Rule names reported via `Cx::broke`.
const BUS_WIDTH: &str = "bus_width";
const FONT: &str = "font";
const COMMAND: &str = "command";
const INIT_WAIT: &str = "init_wait";
const INIT_BUSY: &str = "init_busy";

/// An active read across one `E` pulse in 8-bit mode, or two in 4-bit mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadCycle {
    /// True for data read (`RS = 1`), false for busy flag + address (`RS = 0`).
    data: bool,
    /// Latched byte (for busy+AC, bit 7 is refreshed live on read).
    byte: u8,
    /// In 4-bit mode, false while outputting the high nibble, true for the low nibble.
    low_nibble: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    bytes: u64,
    hash: Sha256,
}

/// An HD44780U: its RAM, its instruction state, its busy flag and its power-on wait.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controller {
    /// `0..80` is DDRAM (line 0 at `0..40`, line 1 at `40..80`), `80..144` is CGRAM.
    ram: [u8; FRAME_BYTES],
    /// Whether `DB0..DB3` reach the bus. A PCF8574 backpack wires only `DB4..DB7`.
    low_lines: bool,
    powered: bool,
    /// Whether `VCC` has reached 4.5 V during the current power cycle.
    reached_4v5: bool,
    /// Power-on reset wait completes at this time (15 ms after 4.5 V or 40 ms after 2.7 V).
    init_ready_at: Time,
    /// The internal reset's busy flag clears at this time (10 ms after 4.5 V, or the 40 ms wait).
    reset_busy_until: Time,
    /// When `VCC` reached 2.7 V, where the oscillator and its blink counter start.
    on_at: Time,
    /// While `B = 1` and the display is on: whether the cursor's character shows all its dots,
    /// and when it next switches.
    blink: Option<(bool, Time)>,
    /// Bus interface: false = 8-bit (power-on), true = 4-bit.
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
    read_cycle: Option<ReadCycle>,
    /// Internal operation finishes at this time; before it, `BF = 1`.
    busy_until: Time,
    /// Open run of RAM writes, reported when the front end closes it.
    run: Option<Run>,
}

impl Controller {
    /// The controller at power-on reset, unpowered.
    pub fn new(low_lines: bool) -> Controller {
        let mut ram = [0u8; FRAME_BYTES];
        ram[..DDRAM_BYTES].fill(0x20);
        Controller {
            ram,
            low_lines,
            powered: false,
            reached_4v5: false,
            init_ready_at: Time::ZERO,
            reset_busy_until: Time::ZERO,
            on_at: Time::ZERO,
            blink: None,
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
        }
    }

    /// The supply at `at`: `powered` once `VCC` is at 2.7 V, `fast` once it is at 4.5 V. A change
    /// of `powered` resets the controller and returns true.
    pub fn supply(&mut self, at: Time, powered: bool, fast: bool) -> bool {
        let fast = powered && fast;
        if powered != self.powered {
            *self = Controller::new(self.low_lines);
            self.powered = powered;
            if powered {
                self.on_at = at;
                self.reached_4v5 = fast;
                let wait = if fast {
                    POWER_ON_WAIT_4V5_PS
                } else {
                    POWER_ON_WAIT_2V7_PS
                };
                self.init_ready_at = Time::from_ps(at.as_ps().saturating_add(wait));
                let busy = if fast { POWER_ON_BUSY_4V5_PS } else { wait };
                self.reset_busy_until = Time::from_ps(at.as_ps().saturating_add(busy));
            }
            return true;
        }
        if powered && fast && !self.reached_4v5 {
            self.reached_4v5 = true;
            let fast_ready = Time::from_ps(at.as_ps().saturating_add(POWER_ON_WAIT_4V5_PS));
            if fast_ready < self.init_ready_at {
                self.init_ready_at = fast_ready;
            }
            let fast_busy = Time::from_ps(at.as_ps().saturating_add(POWER_ON_BUSY_4V5_PS));
            if fast_busy < self.reset_busy_until {
                self.reset_busy_until = fast_busy;
            }
        }
        false
    }

    pub fn powered(&self) -> bool {
        self.powered
    }

    pub fn ram(&self) -> &[u8; FRAME_BYTES] {
        &self.ram
    }

    pub fn four_bit(&self) -> bool {
        self.four_bit
    }

    pub fn two_line(&self) -> bool {
        self.two_line
    }

    pub fn display_on(&self) -> bool {
        self.display_on
    }

    pub fn cursor_on(&self) -> bool {
        self.cursor_on
    }

    pub fn blink_on(&self) -> bool {
        self.blink_on
    }

    /// While `B = 1`, whether the cursor's character shows all its dots now rather than itself.
    pub fn blink_block(&self) -> bool {
        matches!(self.blink, Some((true, _)))
    }

    /// The next time the blinking character switches, while `B = 1` and the display is on.
    pub fn next_blink(&self) -> Option<Time> {
        self.blink.map(|(_, at)| at)
    }

    /// Puts the blink where its counter stands at `at`: on a switch [`next_blink`] gave, or
    /// after a command that turns the display or the blink on or off.
    ///
    /// [`next_blink`]: Self::next_blink
    pub fn blink_at(&mut self, at: Time) {
        self.blink = (self.powered && self.display_on && self.blink_on).then(|| {
            let halves = at.as_ps().saturating_sub(self.on_at.as_ps()) / BLINK_PS;
            let next = self.on_at.as_ps().saturating_add((halves + 1) * BLINK_PS);
            (halves.is_multiple_of(2), Time::from_ps(next))
        });
    }

    pub fn shift(&self) -> u8 {
        self.shift
    }

    pub fn in_cgram(&self) -> bool {
        self.in_cgram
    }

    pub fn current_ac(&self) -> u8 {
        if self.in_cgram {
            self.cgram_addr & 0x3f
        } else {
            self.ddram_addr & 0x7f
        }
    }

    #[cfg(test)]
    pub(crate) fn ddram_addr(&self) -> u8 {
        self.ddram_addr
    }

    fn in_power_on_wait(&self, at: Time) -> bool {
        at < self.init_ready_at
    }

    fn is_busy(&self, at: Time) -> bool {
        at < self.reset_busy_until || at < self.busy_until
    }

    fn set_busy(&mut self, at: Time, duration_ps: u64) {
        self.busy_until = Time::from_ps(at.as_ps().saturating_add(duration_ps));
    }

    /// `E` rose at `at` with `R/W` high: a read starts, or its second nibble does.
    pub fn read_rise(&mut self, cx: &mut Cx, at: Time, rs: bool) {
        if !self.four_bit && !self.low_lines {
            cx.broke(BUS_WIDTH, at);
            self.read_cycle = None;
            return;
        }
        if self.four_bit
            && let Some(cycle) = self.read_cycle
            && cycle.low_nibble
            && cycle.data == rs
        {
            return;
        }
        let byte = if rs {
            if self.is_busy(at) {
                0x00
            } else {
                self.peek_ram()
            }
        } else {
            (u8::from(self.is_busy(at)) << 7) | (self.current_ac() & 0x7f)
        };
        self.read_cycle = Some(ReadCycle {
            data: rs,
            byte,
            low_nibble: false,
        });
    }

    /// What the controller puts on `DB7..DB0` during a read at `at`. In 4-bit mode the nibble is
    /// on `DB7..DB4` and `DB3..DB0` read high. With no read under way every line reads high.
    pub fn read_bus(&self, at: Time) -> u8 {
        let Some(cycle) = self.read_cycle else {
            return 0xff;
        };
        let byte = if cycle.data {
            cycle.byte
        } else {
            (u8::from(self.is_busy(at)) << 7) | (cycle.byte & 0x7f)
        };
        if !self.four_bit {
            byte
        } else if cycle.low_nibble {
            (byte << 4) | 0x0f
        } else {
            byte | 0x0f
        }
    }

    /// `E` fell at `at`, with `RS`, `R/W` and `DB7..DB0` as they stood before it.
    pub fn e_fall(&mut self, cx: &mut Cx, at: Time, rs: bool, rw: bool, bus: u8) {
        if rw {
            // A read in 8-bit mode on a backpack already reported `bus_width` on `E` rising.
            // Clearing the PCF8574's power-on 0xFF latch (`RW = 1`, `E = 1`) on the first I2C
            // write has no active read cycle and is not a bus operation.
            if let Some(mut cycle) = self.read_cycle {
                if self.four_bit && !cycle.low_nibble {
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

        if at < self.reset_busy_until {
            cx.broke(INIT_BUSY, at);
        } else if self.in_power_on_wait(at) {
            cx.broke(INIT_WAIT, at);
        }

        if self.is_busy(at) {
            return;
        }

        let nibble = bus & 0xf0;
        if !self.four_bit {
            if self.low_lines {
                if rs {
                    self.write_data(at, bus);
                } else {
                    self.exec_command(cx, at, bus);
                }
            } else if !rs && (nibble & 0xe0 == 0x20) {
                // With only DB4..DB7 wired, Function Set (0x20 to switch into 4-bit mode, 0x30
                // during the 3x init sequence) is valid; any other instruction or data write
                // needs DB0..DB3 and stops loudly.
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
        // Table 8 (p. 29): F is don't care with two lines, which are always 5x8.
        if font_5x10 && !two_line {
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
            self.blink_at(at);
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

    /// Reports the open run of RAM writes, if any, as one pixel write over the whole glass.
    pub fn close_run(&mut self, cx: &mut Cx) {
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
}
