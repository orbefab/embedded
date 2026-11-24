use orbe_model::{Ack, Cx, Device, Dir, Fields, I2cTarget, Levels, Time, Value as FieldValue};

use super::glass::{backlight_on, glass, text_visible, visible_lines};
use super::model::{
    CLEAR_BUSY_PS, EXEC_BUSY_PS, Hd44780I2c, PIN_BL, PIN_E, PIN_RS, PIN_RW, POWER_ON_BUSY_PS,
};
use crate::display;

struct Bench {
    dev: Hd44780I2c,
    t_ps: u64,
    bl: u8,
}

impl Bench {
    fn new(backlight_jumper: bool) -> Bench {
        let fields = [
            ("backlight_jumper", FieldValue::Bool(backlight_jumper)),
            ("logic_na", FieldValue::Int(1_500_000)),
            ("backlight_na", FieldValue::Int(20_000_000)),
            ("holds_above_mv", FieldValue::Int(2_700)),
        ];
        Bench {
            dev: Hd44780I2c::new(&Fields::new(&fields)).unwrap(),
            t_ps: 0,
            bl: PIN_BL,
        }
    }

    fn power(&mut self, on: bool) {
        let mut cx = Cx::default();
        let levels = Levels::from_slice(&[true, true, on]);
        self.dev.input(&mut cx, Time::from_ps(self.t_ps), levels);
    }

    fn advance_us(&mut self, us: u64) {
        self.t_ps += us * 1_000_000;
    }

    fn write_port(&mut self, byte: u8) -> (Ack, Cx) {
        let mut cx = Cx::default();
        let at = Time::from_ps(self.t_ps);
        self.dev.start(&mut cx, at, Dir::Write);
        let ack = self.dev.write(&mut cx, at, byte);
        self.dev.stop(&mut cx, at);
        (ack, cx)
    }

    fn read_port(&mut self) -> u8 {
        let mut cx = Cx::default();
        let at = Time::from_ps(self.t_ps);
        self.dev.start(&mut cx, at, Dir::Read);
        let byte = self.dev.read(&mut cx, at);
        self.dev.stop(&mut cx, at);
        byte
    }

    fn pulse_nibble(&mut self, data: u8) -> Cx {
        let val = data | self.bl;
        self.write_port(val);
        self.advance_us(1);
        self.write_port(val | PIN_E);
        self.advance_us(1);
        let (_, cx) = self.write_port(val & !PIN_E);
        cx
    }

    fn send_byte(&mut self, byte: u8, rs: bool, wait_us: u64) -> Cx {
        let mode = if rs { PIN_RS } else { 0 };
        let hi = (byte & 0xf0) | mode;
        let lo = ((byte << 4) & 0xf0) | mode;
        let cx_hi = self.pulse_nibble(hi);
        self.advance_us(1);
        let cx_lo = self.pulse_nibble(lo);
        self.advance_us(wait_us);
        if cx_hi.broken().is_some() {
            cx_hi
        } else {
            cx_lo
        }
    }

    fn command(&mut self, byte: u8) -> Cx {
        let wait = if byte == 0x01 || byte & 0xfe == 0x02 {
            2_000
        } else {
            50
        };
        self.send_byte(byte, false, wait)
    }

    fn data(&mut self, byte: u8) -> Cx {
        self.send_byte(byte, true, 50)
    }

    fn print(&mut self, text: &str) {
        for b in text.bytes() {
            let cx = self.data(b);
            assert_eq!(cx.broken(), None);
        }
    }

    fn read_hd44780(&mut self, rs: bool) -> u8 {
        let base = 0xf0 | PIN_RW | (if rs { PIN_RS } else { 0 }) | self.bl;
        // High nibble
        self.write_port(base);
        self.advance_us(1);
        self.write_port(base | PIN_E);
        self.advance_us(1);
        let hi = self.read_port() & 0xf0;
        self.write_port(base);
        self.advance_us(1);
        // Low nibble
        self.write_port(base | PIN_E);
        self.advance_us(1);
        let lo = (self.read_port() >> 4) & 0x0f;
        self.write_port(base);
        self.advance_us(1);
        hi | lo
    }

    /// Standard Hitachi Figure 24 8-bit to 4-bit init sequence.
    fn init_4bit(&mut self) {
        self.power(true);
        self.advance_us(45_000); // > 40 ms after power-on
        assert!(!self.dev.is_four_bit());
        assert_eq!(self.pulse_nibble(0x30).broken(), None);
        self.advance_us(4_500);
        assert_eq!(self.pulse_nibble(0x30).broken(), None);
        self.advance_us(150);
        assert_eq!(self.pulse_nibble(0x30).broken(), None);
        self.advance_us(150);
        assert_eq!(self.pulse_nibble(0x20).broken(), None);
        self.advance_us(150);
        assert!(self.dev.is_four_bit());
        assert_eq!(self.command(0x28).broken(), None); // 4-bit, 2-line, 5x8
        assert_eq!(self.command(0x0c).broken(), None); // display on, cursor off, blink off
        assert_eq!(self.command(0x01).broken(), None); // clear
        assert_eq!(self.command(0x06).broken(), None); // entry mode: increment, no shift
    }

    fn lines(&self) -> [String; 2] {
        let view = self.dev.view();
        let panel = display::panel(Hd44780I2c::SHOWS, view);
        let frame = display::frame(view);
        visible_lines(&panel, frame)
    }

    fn panel(&self) -> display::Panel {
        display::panel(Hd44780I2c::SHOWS, self.dev.view())
    }
}

#[test]
fn unpowered_part_does_nothing_and_draws_nothing() {
    let mut b = Bench::new(true);
    assert!(!b.dev.awake());
    assert_eq!(b.dev.draw().amps, 0.0);
    let (ack, _) = b.write_port(0x30 | PIN_E | PIN_BL);
    assert_eq!(ack, Ack::No);
    assert_eq!(b.read_port(), 0xff);
    assert!(!backlight_on(&b.panel()));
    assert_eq!(b.lines(), ["                ", "                "]);
}

#[test]
fn power_on_busy_drops_early_commands_and_init_enters_4bit_mode() {
    let mut b = Bench::new(true);
    b.power(true);
    // Inside 10 ms internal reset busy window, switching to 4-bit is dropped.
    b.t_ps = POWER_ON_BUSY_PS - 10_000_000;
    assert_eq!(b.pulse_nibble(0x20).broken(), None);
    assert!(!b.dev.is_four_bit());

    // After 10 ms internal reset completes, Figure 24 init works.
    b.t_ps = POWER_ON_BUSY_PS + 1_000_000;
    assert_eq!(b.pulse_nibble(0x30).broken(), None);
    b.advance_us(5_000);
    assert_eq!(b.pulse_nibble(0x30).broken(), None);
    b.advance_us(150);
    assert_eq!(b.pulse_nibble(0x30).broken(), None);
    b.advance_us(150);
    assert_eq!(b.pulse_nibble(0x20).broken(), None);
    assert!(b.dev.is_four_bit());
}

#[test]
fn resync_from_4bit_mode_recovers_even_after_half_nibble() {
    let mut b = Bench::new(true);
    b.init_4bit();
    b.print("Before");
    // Leave half a nibble pending, simulating an MCU reset mid-byte.
    b.pulse_nibble(0x40);
    b.advance_us(100);
    // Standard 3x 0x30 then 0x20 resync sequence.
    assert_eq!(b.pulse_nibble(0x30).broken(), None);
    b.advance_us(4_500);
    assert_eq!(b.pulse_nibble(0x30).broken(), None);
    b.advance_us(4_500);
    assert_eq!(b.pulse_nibble(0x30).broken(), None);
    b.advance_us(150);
    assert_eq!(b.pulse_nibble(0x20).broken(), None);
    b.advance_us(150);
    assert!(b.dev.is_four_bit());
    assert_eq!(b.command(0x28).broken(), None);
    assert_eq!(b.command(0x01).broken(), None);
    b.print("Recovered");
    assert_eq!(b.lines()[0].trim_end(), "Recovered");
}

#[test]
fn clear_display_holds_busy_for_1_52ms_and_drops_writes_while_busy() {
    let mut b = Bench::new(true);
    b.init_4bit();
    b.print("Old");
    // Send Clear Display (0x01) without waiting.
    b.send_byte(0x01, false, 0);
    // At +500 us (< 1.52 ms), busy flag reads 1 and writes are dropped.
    b.advance_us(500);
    let status = b.read_hd44780(false);
    assert_ne!(status & 0x80, 0, "BF must be set during clear");
    assert_eq!(status & 0x7f, 0x00, "AC is 0 after clear");
    b.send_byte(b'X', true, 0);
    assert_eq!(b.dev.ddram()[0], b' ', "write during busy must be dropped");

    // Past 1.52 ms (CLEAR_BUSY_PS), BF clears and writes land.
    b.t_ps += CLEAR_BUSY_PS;
    let status = b.read_hd44780(false);
    assert_eq!(status & 0x80, 0, "BF must clear after 1.52 ms");
    b.data(b'A');
    assert_eq!(b.lines()[0].trim_end(), "A");
}

#[test]
fn two_line_addressing_cursor_moves_and_display_shift() {
    let mut b = Bench::new(true);
    b.init_4bit();
    b.command(0x80); // row 0 col 0
    b.print("Hello");
    b.command(0xc0); // row 1 col 0 (0x40)
    b.print("World");
    assert_eq!(b.lines(), ["Hello           ", "World           "]);

    // Display shift left moves visible window right by 1 column.
    b.command(0x18);
    assert_eq!(b.lines(), ["ello            ", "orld            "]);

    // Display shift right restores original window.
    b.command(0x1c);
    assert_eq!(b.lines(), ["Hello           ", "World           "]);

    // Return home resets shift and AC to 0 without clearing DDRAM.
    b.command(0x18);
    b.command(0x02);
    assert_eq!(b.dev.ddram_addr(), 0);
    assert_eq!(b.lines(), ["Hello           ", "World           "]);

    // Cursor move left/right updates AC without changing DDRAM.
    b.command(0x80 | 0x27);
    b.command(0x14); // move right wraps 0x27 -> 0x40
    assert_eq!(b.dev.ddram_addr(), 0x40);
    b.command(0x10); // move left wraps 0x40 -> 0x27
    assert_eq!(b.dev.ddram_addr(), 0x27);
}

#[test]
fn entry_mode_decrement_and_autoshift() {
    let mut b = Bench::new(true);
    b.init_4bit();
    // Decrement, no shift (0x04).
    b.command(0x04);
    b.command(0x80 | 5);
    b.print("CBA");
    assert_eq!(b.lines()[0].trim_end(), "   ABC");

    // Clear restores I/D = 1 (ADE-207-272 p. 28).
    b.command(0x01);
    // Increment + auto-shift (0x07).
    b.command(0x07);
    b.command(0x80 | 16);
    b.print("Hi");
    // Window shifted left by 2, so column 16..17 now appear at columns 14..15.
    assert_eq!(b.lines()[0], "              Hi");
}

#[test]
fn cgram_custom_characters_render_on_glass_and_read_back() {
    let mut b = Bench::new(true);
    b.init_4bit();
    let heart: [u8; 8] = [0x00, 0x0a, 0x1f, 0x1f, 0x0e, 0x04, 0x00, 0x00];
    b.command(0x40 | (2 << 3)); // CGRAM slot 2
    for &row in &heart {
        b.data(row);
    }
    assert_eq!(&b.dev.cgram()[16..24], &heart);

    // Read back CGRAM slot 2 via RW=1, RS=1.
    b.command(0x40 | (2 << 3));
    for &expected in &heart {
        let got = b.read_hd44780(true);
        b.advance_us(50);
        assert_eq!(got, expected);
    }

    // Display slot 2 (code 2) and its mirror (code 10 = 0x0A) on DDRAM row 0.
    b.command(0x80);
    b.data(2);
    b.data(10);
    assert_eq!(b.lines()[0].trim_end(), "□□");

    // Read back DDRAM via RW=1, RS=1.
    b.command(0x80);
    assert_eq!(b.read_hd44780(true), 2);
    b.advance_us(50);
    assert_eq!(b.read_hd44780(true), 10);

    // Glass draws lit white dots for the heart bitmap.
    let view = b.dev.view();
    let panel = display::panel(Hd44780I2c::SHOWS, view);
    let pixels = glass(&panel, display::frame(view));
    assert!(
        pixels.chunks(4).any(|p| p == [255, 255, 255, 255]),
        "custom CGRAM glyph must light white pixels on backlit glass"
    );
}

#[test]
fn display_off_or_backlight_off_makes_blue_glass_blank() {
    let mut b = Bench::new(true);
    b.init_4bit();
    b.print("Visible");
    assert_eq!(b.lines()[0].trim_end(), "Visible");
    assert!(backlight_on(&b.panel()));
    assert!(text_visible(&b.panel()));

    // Turning backlight off via P3 hides text on the blue transmissive glass.
    b.bl = 0;
    b.write_port(0);
    assert!(!backlight_on(&b.panel()));
    assert!(!text_visible(&b.panel()));
    assert_eq!(b.lines(), ["                ", "                "]);
    assert!((b.dev.draw().amps - 1.5e-3).abs() < 1e-9);

    // Turning backlight back on reveals the DDRAM text again.
    b.bl = PIN_BL;
    b.write_port(PIN_BL);
    assert!(backlight_on(&b.panel()));
    assert_eq!(b.lines()[0].trim_end(), "Visible");
    assert!((b.dev.draw().amps - 21.5e-3).abs() < 1e-9);

    // Turning display off hides text even while backlight is on.
    b.command(0x08);
    assert!(backlight_on(&b.panel()));
    assert!(!text_visible(&b.panel()));
    assert_eq!(b.lines(), ["                ", "                "]);

    // Power loss clears DDRAM and turns everything off.
    b.power(false);
    assert!(!backlight_on(&b.panel()));
    assert_eq!(b.dev.ddram()[0], b' ');
}

#[test]
fn backlight_jumper_removed_keeps_backlight_off() {
    let mut b = Bench::new(false);
    b.init_4bit();
    b.print("Dark");
    assert!(!backlight_on(&b.panel()));
    assert_eq!(b.lines(), ["                ", "                "]);
    assert!((b.dev.draw().amps - 1.5e-3).abs() < 1e-9);
}

#[test]
fn pcf8574_reads_back_latched_pins_when_not_reading_hd44780() {
    let mut b = Bench::new(true);
    b.power(true);
    b.write_port(0xa8);
    assert_eq!(b.read_port(), 0xa8);
    b.write_port(0x00);
    assert_eq!(b.read_port(), 0x00);
}

#[test]
fn unsupported_8bit_bus_and_5x10_font_stop_loudly() {
    let mut b = Bench::new(true);
    b.power(true);
    b.advance_us(45_000);
    // Attempting a non-Function-Set command while still in 8-bit mode breaks `bus_width`.
    let cx = b.pulse_nibble(PIN_RS);
    assert_eq!(cx.broken().map(|(rule, _)| rule), Some("bus_width"));

    // Enter 4-bit mode, then request 5x10 font (0x24 = DL=0, N=0, F=1).
    let mut b2 = Bench::new(true);
    b2.init_4bit();
    let cx = b2.command(0x24);
    assert_eq!(cx.broken().map(|(rule, _)| rule), Some("font"));

    // Busy duration for regular instructions is 37 us (EXEC_BUSY_PS).
    let mut b3 = Bench::new(true);
    b3.init_4bit();
    b3.send_byte(b'A', true, 0);
    assert_ne!(b3.read_hd44780(false) & 0x80, 0);
    b3.t_ps += EXEC_BUSY_PS;
    assert_eq!(b3.read_hd44780(false) & 0x80, 0);
}
