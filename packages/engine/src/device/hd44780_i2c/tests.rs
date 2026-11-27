use orbe_model::{Ack, Cx, Device, Dir, Fields, I2cTarget, Levels, Time, Value as FieldValue};

use super::model::{Hd44780I2c, PIN_BL, PIN_E, PIN_RS, PIN_RW};
use crate::device::hd44780::glass::{backlight_on, glass, text_visible, visible_lines};
use crate::device::hd44780::model::controller::{
    CLEAR_BUSY_PS, EXEC_BUSY_PS, POWER_ON_BUSY_4V5_PS, POWER_ON_WAIT_2V7_PS, POWER_ON_WAIT_4V5_PS,
};
use crate::diagnostic::{Diagnostic, ProtocolRule};
use crate::display;
use crate::netlist;
use crate::simulate;

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

    fn power_levels(&mut self, v2v7: bool, v4v5: bool) {
        let mut cx = Cx::default();
        let levels = Levels::from_slice(&[true, true, v2v7, v4v5]);
        self.dev.input(&mut cx, Time::from_ps(self.t_ps), levels);
    }

    fn power(&mut self, on: bool) {
        self.power_levels(on, on);
    }

    fn power_3v3(&mut self) {
        self.power_levels(true, false);
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

    /// Sends the Hitachi Figure 24 8-bit to 4-bit init sequence and writes `ch`, returning the
    /// first protocol rule broken (if any) along with its timestamp.
    fn run_fig24_and_write(&mut self, ch: u8) -> Option<(&'static str, Time)> {
        let mut first = None;
        let mut note = |cx: Cx| {
            if first.is_none() {
                first = cx.broken();
            }
        };
        note(self.pulse_nibble(0x30));
        self.advance_us(4_500);
        note(self.pulse_nibble(0x30));
        self.advance_us(150);
        note(self.pulse_nibble(0x30));
        self.advance_us(150);
        note(self.pulse_nibble(0x20));
        self.advance_us(150);
        note(self.command(0x28)); // 4-bit, 2-line, 5x8
        note(self.command(0x0c)); // display on, cursor off, blink off
        note(self.command(0x01)); // clear
        note(self.command(0x06)); // entry mode: increment, no shift
        note(self.data(ch));
        first
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
fn power_on_wait_warns_and_drops_early_commands_until_sheet_wait_elapses() {
    let mut b = Bench::new(true);
    b.power(true);
    // Inside the 10 ms reset busy time, switching to 4-bit warns `init_busy` and is dropped.
    b.t_ps = POWER_ON_BUSY_4V5_PS - 10_000_000;
    let cx = b.pulse_nibble(0x20);
    assert_eq!(cx.broken().map(|(rule, _)| rule), Some("init_busy"));
    assert!(!b.dev.is_four_bit());

    // Past the busy time and inside the 15 ms wait, the switch still warns, and the controller
    // runs it.
    let mut late = Bench::new(true);
    late.power(true);
    late.t_ps = POWER_ON_WAIT_4V5_PS - 10_000_000;
    let cx = late.pulse_nibble(0x20);
    assert_eq!(cx.broken().map(|(rule, _)| rule), Some("init_wait"));
    assert!(late.dev.is_four_bit());

    // After 15 ms at >= 4.5 V, Figure 24 init succeeds with no warning.
    b.t_ps = POWER_ON_WAIT_4V5_PS + 1_000_000;
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
fn early5_and_early15_enforce_hd44780_power_on_reset_waits() {
    // early5 at 5 V (VCC >= 4.5 V at t = 0): waiting 5 ms before the first Function Set is
    // inside the 15 ms Fig. 24 wait. The first write at 5.002 ms lands inside the 10 ms reset
    // busy time and warns `init_busy`, and the Function Sets that enter 4-bit mode are dropped
    // with it, so 'E' never appears.
    let mut early5 = Bench::new(true);
    early5.power(true);
    early5.advance_us(5_000);
    let broken = early5.run_fig24_and_write(b'E');
    assert_eq!(broken, Some(("init_busy", Time::from_ps(5_002_000_000))));
    assert!(!early5.dev.is_four_bit());
    assert_eq!(early5.lines(), ["                ", "                "]);

    // early15 at 5 V (VCC >= 4.5 V at t = 0): waiting 15 ms means the first E fall lands at
    // 15.002 ms (>= 15 ms since VCC reached 4.5 V), which satisfies the 4.5 V Fig. 24 line.
    let mut early15_5v = Bench::new(true);
    early15_5v.power(true);
    early15_5v.advance_us(15_000);
    assert_eq!(early15_5v.run_fig24_and_write(b'E'), None);
    assert!(early15_5v.dev.is_four_bit());
    assert_eq!(early15_5v.lines()[0].trim_end(), "E");

    // early15 at 5 V with a delayed supply rise: VCC reaches 2.7 V at 0 ms and 4.5 V at 5 ms,
    // so ready time is min(0 + 40 ms, 5 + 15 ms) = 20 ms and the busy flag clears at
    // min(40 ms, 5 + 10 ms) = 15 ms. Writing after 15 ms from t = 0 arrives only 10.002 ms
    // after VCC reached 4.5 V, so it warns `init_wait`, and the controller runs it.
    let mut early15_delayed = Bench::new(true);
    early15_delayed.power_3v3();
    early15_delayed.advance_us(5_000);
    early15_delayed.power(true); // crosses 4.5 V at t = 5 ms -> ready at 20 ms
    early15_delayed.advance_us(10_000); // t = 15 ms
    let broken = early15_delayed.run_fig24_and_write(b'E');
    assert_eq!(broken, Some(("init_wait", Time::from_ps(15_002_000_000))));
    assert!(early15_delayed.dev.is_four_bit());
    assert_eq!(early15_delayed.lines()[0].trim_end(), "E");

    // early15 at 3.3 V (VCC >= 2.7 V at t = 0, never reaches 4.5 V): the 40 ms Fig. 24 line
    // applies, and below 4.5 V it is the busy time too. Waiting 15 ms warns `init_busy` at
    // 15.002 ms and drops every write, so 'E' does not appear. Waiting past 40 ms succeeds with
    // no warning.
    let mut early15_3v3 = Bench::new(true);
    early15_3v3.power_3v3();
    early15_3v3.advance_us(15_000);
    let broken = early15_3v3.run_fig24_and_write(b'E');
    assert_eq!(broken, Some(("init_busy", Time::from_ps(15_002_000_000))));
    assert!(!early15_3v3.dev.is_four_bit());
    assert_eq!(
        early15_3v3.lines(),
        ["                ", "                "]
    );

    let mut ready_3v3 = Bench::new(true);
    ready_3v3.power_3v3();
    ready_3v3.t_ps = POWER_ON_WAIT_2V7_PS;
    assert_eq!(ready_3v3.run_fig24_and_write(b'E'), None);
    assert!(ready_3v3.dev.is_four_bit());
    assert_eq!(ready_3v3.lines()[0].trim_end(), "E");
}

/// Electrical MNA check of both supply comparators (2.7 V and 4.5 V): sends one I2C write pulsing
/// `E` high then low for Function Set `0x30` at `write_at_ms`, verifying `chip.protocol` rule
/// `init_busy` or `init_wait` fires based on the supply's actual rise time and voltage.
#[test]
fn supply_comparators_enforce_15ms_at_5v_and_40ms_at_3v3() {
    fn run_i2c_write(vcc_v: f64, vcc_step_ms: Option<u64>, write_at_ms: u64) -> Vec<Diagnostic> {
        let mut scl_ev: Vec<(u64, f64)> = Vec::new();
        let mut sda_ev: Vec<(u64, f64)> = Vec::new();
        let mut t = write_at_ms * 1_000_000_000;
        let high = vcc_v;
        let half = 5_000_000u64; // 5 us half-period (100 kHz I2C)
        let mut cur_scl = high;
        let mut cur_sda = high;
        let mut set_scl = |t: u64, v: f64| {
            if (v - cur_scl).abs() > 0.01 {
                scl_ev.push((t, v));
                cur_scl = v;
            }
        };
        let mut set_sda = |t: u64, v: f64| {
            if (v - cur_sda).abs() > 0.01 {
                sda_ev.push((t, v));
                cur_sda = v;
            }
        };
        // START: SDA falls while SCL is high.
        set_sda(t, 0.0);
        t += half;
        set_scl(t, 0.0);
        t += half;
        // Send address 0x27<<1 (write), then port byte 0x34 (E=1), then 0x30 (E=0).
        for byte in [0x27u8 << 1, 0x34, 0x30] {
            for bit in (0..8).rev() {
                let v = if (byte >> bit) & 1 == 1 { high } else { 0.0 };
                set_sda(t, v);
                t += half;
                set_scl(t, high);
                t += half;
                set_scl(t, 0.0);
                t += half;
            }
            // ACK clock (release SDA high).
            set_sda(t, high);
            t += half;
            set_scl(t, high);
            t += half;
            set_scl(t, 0.0);
            t += half;
        }
        // STOP: SDA rises while SCL is high.
        set_sda(t, 0.0);
        t += half;
        set_scl(t, high);
        t += half;
        set_sda(t, high);
        let stop_ps = t + 1_000_000_000;

        let fmt_events = |ev: &[(u64, f64)]| -> String {
            ev.iter()
                .map(|(at, v)| {
                    format!(r#"{{ "t_ps": {at}, "level": {{ "value": {v}, "unit": "V" }} }}"#)
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let vcc_source = match vcc_step_ms {
            None => format!(r#"{{ "value": {vcc_v}, "unit": "V" }}"#),
            Some(step_ms) => format!(
                r#"{{ "kind": "steps", "initial": {{ "value": 0.0, "unit": "V" }}, "events": [{{ "t_ps": {}, "level": {{ "value": {vcc_v}, "unit": "V" }} }}] }}"#,
                step_ms * 1_000_000_000
            ),
        };
        let json = format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": 1000000000, "grid_ps": 1000000000 }} }},
              "ground": "gnd",
              "nets": ["gnd", "vcc", "scl", "sda", "sda_drv"],
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }}, "voltage": {vcc_source} }},
                {{ "name": "vscl", "kind": "voltage_source", "pins": {{ "p": "scl", "n": "gnd" }},
                  "voltage": {{ "kind": "steps", "initial": {{ "value": {high}, "unit": "V" }}, "events": [{}] }} }},
                {{ "name": "vsda", "kind": "voltage_source", "pins": {{ "p": "sda_drv", "n": "gnd" }},
                  "voltage": {{ "kind": "steps", "initial": {{ "value": {high}, "unit": "V" }}, "events": [{}] }} }},
                {{ "name": "rsda", "kind": "resistor", "pins": {{ "a": "sda_drv", "b": "sda" }},
                  "resistance": {{ "value": 4700.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
                {{ "name": "lcd", "kind": "hd44780_i2c",
                  "pins": {{ "VCC": "vcc", "GND": "gnd", "SCL": "scl", "SDA": "sda" }},
                  "address": 39,
                  "low_drop": {{ "value": 0.0, "unit": "V" }},
                  "low_resistance": {{ "value": 133.33333333333334, "unit": "ohm" }},
                  "input_threshold": {{ "low": {{ "value": 0.3, "unit": "1" }}, "high": {{ "value": 0.7, "unit": "1" }} }} }}
              ]
            }}"#,
            fmt_events(&scl_ev),
            fmt_events(&sda_ev),
        );
        let parsed = netlist::parse(json.as_bytes()).unwrap();
        simulate(&parsed).diagnostics().to_vec()
    }

    // The power-on rules each run raises.
    let init = |diags: &[Diagnostic]| -> Vec<ProtocolRule> {
        diags
            .iter()
            .filter_map(|d| match d {
                Diagnostic::ChipProtocol { rule, .. }
                    if matches!(rule, ProtocolRule::InitWait | ProtocolRule::InitBusy) =>
                {
                    Some(*rule)
                }
                _ => None,
            })
            .collect()
    };

    // 5 V from t = 0: early5 (5 ms) is inside the 10 ms busy time and is dropped; early15
    // (15 ms) is legal (> 15 ms at E fall).
    let d_5v_5ms = run_i2c_write(5.0, None, 5);
    assert_eq!(init(&d_5v_5ms), [ProtocolRule::InitBusy], "{d_5v_5ms:?}");
    let d_5v_15ms = run_i2c_write(5.0, None, 15);
    assert_eq!(init(&d_5v_15ms), [], "{d_5v_15ms:?}");

    // 5 V stepping up at 5 ms: write at 15 ms (only 10 ms after 4.5 V) runs early and warns; at
    // 20 ms is legal.
    let d_step_15ms = run_i2c_write(5.0, Some(5), 15);
    assert_eq!(
        init(&d_step_15ms),
        [ProtocolRule::InitWait],
        "{d_step_15ms:?}"
    );
    let d_step_20ms = run_i2c_write(5.0, Some(5), 20);
    assert_eq!(init(&d_step_20ms), [], "{d_step_20ms:?}");

    // 3.3 V from t = 0: early15 (15 ms < 40 ms) is inside the busy time below 4.5 V; 40 ms is
    // legal (> 40 ms at E fall).
    let d_3v3_15ms = run_i2c_write(3.3, None, 15);
    assert_eq!(
        init(&d_3v3_15ms),
        [ProtocolRule::InitBusy],
        "{d_3v3_15ms:?}"
    );
    let d_3v3_40ms = run_i2c_write(3.3, None, 40);
    assert_eq!(init(&d_3v3_40ms), [], "{d_3v3_40ms:?}");
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

#[test]
fn the_screen_art_shows_the_white_dots_on_the_backlit_glass() {
    let mut b = Bench::new(true);
    b.power(true);
    b.t_ps = POWER_ON_WAIT_4V5_PS + 1_000_000;
    assert_eq!(b.run_fig24_and_write(b'E'), None);
    let panel = b.panel();
    assert!(backlight_on(&panel));
    let pixels = glass(&panel, display::frame(b.dev.view()));
    // The A00 ROM's `E` lights 18 of its 40 dots, each 2 by 2 pixels, and the blue glass none.
    assert_eq!(display::lit_count(&panel, &pixels), 18 * 4);
    let art = display::art(&panel, &pixels);
    let rows: Vec<&str> = art.lines().collect();
    assert_eq!(&rows[8][18..33], "##.##.##.##.##.");
    assert_eq!(&rows[11][18..33], "##.............");
}
