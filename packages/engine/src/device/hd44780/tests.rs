use orbe_model::{Cx, Device, Drive, Fields, Levels, Time, Value as FieldValue};

use super::glass::{backlight_on, glass};
use super::glass::{text_visible, visible_lines};
use super::model::controller::BLINK_PS;
use super::model::{
    Hd44780, WATCH_BACKLIGHT, WATCH_CONTRAST, WATCH_DB0, WATCH_E, WATCH_OPERATING, WATCH_RS,
    WATCH_RW, WATCH_VDD, WATCH_VDD_4V5,
};
use crate::display;

const WATCHES: usize = 16;
const NS: u64 = 1_000;
const US: u64 = 1_000_000;

/// The pins of one `hd44780`, driven as a sketch drives them: each call sets levels at the
/// bench's time and hands them to the device.
struct Bench {
    dev: Hd44780,
    t_ps: u64,
    levels: [bool; WATCHES],
    /// Each rule broken, in the order first broken.
    broke: Vec<&'static str>,
    eight_bit: bool,
}

impl Bench {
    fn new() -> Bench {
        let fields = [
            ("logic_na", FieldValue::Int(350_000)),
            ("holds_above_mv", FieldValue::Int(2_700)),
        ];
        let mut levels = [false; WATCHES];
        levels[WATCH_CONTRAST] = true;
        levels[WATCH_BACKLIGHT] = true;
        Bench {
            dev: Hd44780::new(&Fields::new(&fields)).unwrap(),
            t_ps: 0,
            levels,
            broke: Vec::new(),
            eight_bit: false,
        }
    }

    fn input(&mut self) {
        let mut cx = Cx::default();
        self.dev.input(
            &mut cx,
            Time::from_ps(self.t_ps),
            Levels::from_slice(&self.levels),
        );
        // The host warns each rule the first time.
        if let Some((rule, _)) = cx.broken()
            && !self.broke.contains(&rule)
        {
            self.broke.push(rule);
        }
    }

    fn set(&mut self, watch: usize, high: bool) {
        self.levels[watch] = high;
        self.input();
    }

    fn wait(&mut self, ps: u64) {
        self.t_ps += ps;
    }

    /// `VDD` at 5 V, or off.
    fn power(&mut self, on: bool) {
        self.levels[WATCH_VDD] = on;
        self.levels[WATCH_VDD_4V5] = on;
        self.levels[WATCH_OPERATING] = on;
        self.input();
    }

    fn bus(&mut self, rs: bool, byte: u8) {
        self.levels[WATCH_RS] = rs;
        for line in 0..8 {
            self.levels[WATCH_DB0 + line] = byte & (1 << line) != 0;
        }
        self.input();
    }

    /// A 1 us E pulse, the one `LiquidCrystal` gives, then its 100 us settle.
    fn pulse(&mut self) {
        self.wait(US);
        self.set(WATCH_E, true);
        self.wait(US);
        self.set(WATCH_E, false);
        self.wait(100 * US);
    }

    fn write(&mut self, rs: bool, byte: u8) {
        if self.eight_bit {
            self.bus(rs, byte);
            self.pulse();
        } else {
            self.bus(rs, byte & 0xf0);
            self.pulse();
            self.bus(rs, byte << 4);
            self.pulse();
        }
    }

    fn command(&mut self, byte: u8) {
        self.write(false, byte);
        if byte == 0x01 || byte & 0xfe == 0x02 {
            self.wait(2_000 * US);
        }
    }

    fn print(&mut self, text: &str) {
        for byte in text.bytes() {
            self.write(true, byte);
        }
    }

    /// `LiquidCrystal::begin(16, 2)`: the Figure 24 or Figure 23 wake-up, then Function Set,
    /// display on, clear and entry mode.
    fn begin(&mut self, eight_bit: bool) {
        if !self.levels[WATCH_VDD] {
            self.power(true);
        }
        self.wait(50_000 * US);
        if eight_bit {
            for wait in [4_500, 150, 150] {
                self.bus(false, 0x30);
                self.pulse();
                self.wait(wait * US);
            }
            self.eight_bit = true;
            self.command(0x38);
        } else {
            for wait in [4_500, 4_500, 150] {
                self.bus(false, 0x30);
                self.pulse();
                self.wait(wait * US);
            }
            self.bus(false, 0x20);
            self.pulse();
            self.command(0x28);
        }
        self.command(0x0c);
        self.command(0x01);
        self.command(0x06);
    }

    fn panel(&self) -> display::Panel {
        display::panel(Hd44780::SHOWS, self.dev.view())
    }

    fn lines(&self) -> [String; 2] {
        visible_lines(&self.panel(), display::frame(self.dev.view()))
    }

    /// Runs the device to `limit` and returns its drives on `DB0..DB7`.
    fn run_to(&mut self, limit: u64) -> [Drive; 8] {
        let mut cx = Cx::default();
        while self.dev.run_until(&mut cx, Time::from_ps(limit)).is_some() {}
        let drives = self.dev.drive();
        std::array::from_fn(|line| drives.get(line))
    }
}

fn line0(text: &str) -> String {
    format!("{text:<16}")
}

#[test]
fn four_bit_hello_world_shows_on_the_glass() {
    let mut bench = Bench::new();
    bench.begin(false);
    bench.print("hello, world!");
    bench.command(0xc0);
    bench.print("0");
    assert!(bench.dev.is_four_bit());
    assert_eq!(bench.lines(), [line0("hello, world!"), line0("0")]);
    assert_eq!(bench.broke, Vec::<&str>::new());
}

#[test]
fn eight_bit_bus_writes_a_whole_byte_per_pulse() {
    let mut bench = Bench::new();
    bench.begin(true);
    bench.print("hello, world!");
    assert!(!bench.dev.is_four_bit());
    assert_eq!(bench.lines()[0], line0("hello, world!"));
    assert_eq!(&bench.dev.ddram()[..5], b"hello");
    assert_eq!(bench.broke, Vec::<&str>::new());
}

#[test]
fn unpowered_part_does_nothing_and_draws_nothing() {
    let mut bench = Bench::new();
    bench.bus(false, 0x30);
    bench.pulse();
    bench.bus(true, b'A');
    bench.pulse();
    assert_eq!(bench.dev.draw().amps, 0.0);
    assert!(!text_visible(&bench.panel()));
    assert_eq!(bench.dev.ddram()[0], 0x20);
    assert_eq!(bench.broke, Vec::<&str>::new());
}

#[test]
fn v0_near_vdd_blanks_the_glass_and_warns_contrast() {
    let mut bench = Bench::new();
    bench.levels[WATCH_CONTRAST] = false;
    bench.begin(false);
    bench.print("hello, world!");
    assert_eq!(&bench.dev.ddram()[..5], b"hello");
    assert_eq!(bench.lines(), [line0(""), line0("")]);
    assert_eq!(bench.broke, ["contrast"]);
    // Turning the pot down brings the text back.
    bench.set(WATCH_CONTRAST, true);
    assert_eq!(bench.lines()[0], line0("hello, world!"));
}

#[test]
fn unlit_backlight_leaves_the_blue_glass_unreadable() {
    let mut bench = Bench::new();
    bench.levels[WATCH_BACKLIGHT] = false;
    bench.begin(false);
    bench.print("hi");
    assert_eq!(bench.lines(), [line0(""), line0("")]);
    bench.set(WATCH_BACKLIGHT, true);
    assert_eq!(bench.lines()[0], line0("hi"));
}

#[test]
fn a_short_enable_pulse_still_latches_and_warns_enable_width() {
    let mut bench = Bench::new();
    bench.begin(false);
    bench.bus(true, b'A' & 0xf0);
    bench.wait(US);
    bench.set(WATCH_E, true);
    bench.wait(200 * NS);
    bench.set(WATCH_E, false);
    bench.wait(100 * US);
    bench.bus(true, b'A' << 4);
    bench.pulse();
    assert_eq!(bench.broke, ["enable_width"]);
    assert_eq!(bench.lines()[0], line0("A"));
}

#[test]
fn bus_timing_breaks_setup_hold_and_cycle() {
    // RS moving 20 ns before E rises is under tAS, 40 ns.
    let mut bench = Bench::new();
    bench.begin(false);
    bench.bus(true, 0x40);
    bench.wait(20 * NS);
    bench.set(WATCH_E, true);
    assert_eq!(bench.broke, ["setup"]);

    // Data moving in the same instant E falls is under tH, 10 ns.
    let mut bench = Bench::new();
    bench.begin(false);
    bench.bus(true, 0x40);
    bench.wait(US);
    bench.set(WATCH_E, true);
    bench.wait(US);
    bench.levels[WATCH_E] = false;
    bench.bus(true, 0x10);
    assert_eq!(bench.broke, ["hold"]);

    // A second rise 300 ns after the first is under tcycE, 500 ns.
    let mut bench = Bench::new();
    bench.begin(false);
    bench.bus(true, 0x40);
    bench.wait(US);
    bench.set(WATCH_E, true);
    bench.wait(250 * NS);
    bench.set(WATCH_E, false);
    bench.wait(50 * NS);
    bench.set(WATCH_E, true);
    assert_eq!(bench.broke, ["enable_cycle"]);
}

#[test]
fn slower_table_applies_below_4v5() {
    // 300 ns of E is past PWEH at 5 V (230 ns) and under it at 3.3 V (450 ns).
    let pulse = |fast: bool| {
        let mut bench = Bench::new();
        bench.begin(false);
        bench.set(WATCH_VDD_4V5, fast);
        bench.bus(true, 0x40);
        bench.wait(US);
        bench.set(WATCH_E, true);
        bench.wait(300 * NS);
        bench.set(WATCH_E, false);
        bench.broke
    };
    assert_eq!(pulse(true), Vec::<&str>::new());
    assert_eq!(pulse(false), ["enable_width"]);
}

#[test]
fn a_read_drives_the_busy_flag_and_address_after_the_data_delay() {
    let mut bench = Bench::new();
    bench.begin(true);
    bench.print("ab");
    // Address counter 2, not busy: 0x02 on DB7..DB0, 160 ns after E rises.
    bench.levels[WATCH_RW] = true;
    bench.bus(false, 0xff);
    bench.wait(US);
    bench.set(WATCH_E, true);
    let rise = bench.t_ps;
    assert_eq!(bench.run_to(rise + 159 * NS), [Drive::PullUp; 8]);
    let read = bench.run_to(rise + 160 * NS);
    let byte = (0..8).fold(0u8, |byte, line| {
        byte | (u8::from(read[line] == Drive::High) << line)
    });
    assert_eq!(byte, 0x02);
    bench.wait(US);
    bench.set(WATCH_E, false);
    let fall = bench.t_ps;
    assert_eq!(bench.run_to(fall + 5 * NS), [Drive::PullUp; 8]);
    assert_eq!(bench.broke, Vec::<&str>::new());
}

#[test]
fn an_e_fall_with_no_rise_since_power_on_is_not_a_cycle() {
    let mut bench = Bench::new();
    bench.levels[WATCH_E] = true;
    bench.power(true);
    bench.wait(US);
    bench.set(WATCH_E, false);
    assert_eq!(bench.broke, Vec::<&str>::new());
}

#[test]
fn rs_moving_under_an_e_high_since_power_on_is_not_a_cycle() {
    let mut bench = Bench::new();
    bench.levels[WATCH_E] = true;
    bench.power(true);
    bench.wait(US);
    bench.set(WATCH_RS, true);
    bench.wait(US);
    bench.set(WATCH_E, false);
    assert_eq!(bench.broke, Vec::<&str>::new());
}

#[test]
fn a_write_inside_the_reset_busy_time_is_dropped_and_warns_init_busy() {
    let mut bench = Bench::new();
    bench.power(true);
    // 1 ms after VDD reaches 4.5 V, inside the 10 ms busy time (p. 23).
    bench.wait(1_000 * US);
    bench.bus(false, 0x20);
    bench.pulse();
    assert_eq!(bench.broke, ["init_busy"]);
    assert!(!bench.dev.is_four_bit());
}

#[test]
fn a_write_after_the_busy_time_inside_the_wait_runs_and_warns_init_wait() {
    let mut bench = Bench::new();
    bench.power(true);
    // 12 ms after VDD reaches 4.5 V: past the 10 ms busy time, inside the 15 ms wait.
    bench.wait(12_000 * US);
    bench.bus(false, 0x20);
    bench.pulse();
    assert_eq!(bench.broke, ["init_wait"]);
    assert!(bench.dev.is_four_bit());
}

/// Whether the character at row 0, column `col` shows all its dots on the glass.
fn cell_block(bench: &Bench, col: usize) -> bool {
    let pixels = glass(&bench.panel(), display::frame(bench.dev.view()));
    let (x, y) = (18 + col * 18, 8);
    (0..8).all(|row| {
        (0..5).all(|dot| {
            let off = ((y + row * 3) * 320 + x + dot * 3) * 4;
            pixels[off..off + 3] == [255, 255, 255]
        })
    })
}

#[test]
fn blink_switches_the_cursor_character_at_the_sheet_rate_from_power_on() {
    let mut bench = Bench::new();
    bench.begin(false);
    bench.print("hi");
    // Display on, cursor off, blink on.
    bench.command(0x0d);
    let start = bench.t_ps;
    let halves = start / BLINK_PS;
    // The blink counter runs from power-on at 0: even halves show all dots.
    assert_eq!(cell_block(&bench, 2), halves.is_multiple_of(2));
    assert_eq!(
        bench.panel().settings.get("blink"),
        Some(if halves.is_multiple_of(2) { 1 } else { 2 })
    );
    let mut block = cell_block(&bench, 2);
    for k in 1..6 {
        let switch = (halves + k) * BLINK_PS;
        bench.run_to(switch - 1);
        assert_eq!(cell_block(&bench, 2), block, "just before switch {k}");
        bench.run_to(switch);
        block = !block;
        assert_eq!(cell_block(&bench, 2), block, "at switch {k}");
        // The character itself does not blink, and the cell before it holds its `i`.
        assert!(!cell_block(&bench, 1));
    }
    // 409.6 ms x 250 / 270 (p. 26).
    assert_eq!(BLINK_PS, 379_259_259_259);
    // Blink off: the cell shows its space, and no switch comes.
    bench.t_ps = 6 * BLINK_PS + halves * BLINK_PS;
    bench.command(0x0c);
    assert!(!cell_block(&bench, 2));
    assert_eq!(bench.panel().settings.get("blink"), Some(0));
    let mut cx = Cx::default();
    assert_eq!(bench.dev.run_until(&mut cx, Time::from_ps(u64::MAX)), None);
}

#[test]
fn the_backlight_lights_from_its_led_without_the_controller_supply() {
    let mut bench = Bench::new();
    // VDD open: the LED's own A - K still carries its lit current.
    bench.set(WATCH_BACKLIGHT, true);
    assert!(backlight_on(&bench.panel()));
    assert!(!text_visible(&bench.panel()));
    let pixels = glass(&bench.panel(), display::frame(bench.dev.view()));
    assert_eq!(&pixels[..4], &[16, 84, 180, 255], "the glass glows blue");
    assert_eq!(bench.dev.draw().amps, 0.0);
    bench.set(WATCH_BACKLIGHT, false);
    assert!(!backlight_on(&bench.panel()));
}

#[test]
fn a_supply_under_the_module_minimum_runs_and_warns_low_supply() {
    let mut bench = Bench::new();
    // VDD past 4.5 V and under the module's 4.7 V.
    bench.levels[WATCH_VDD] = true;
    bench.levels[WATCH_VDD_4V5] = true;
    bench.levels[WATCH_OPERATING] = false;
    bench.input();
    bench.begin(false);
    bench.print("hello, world!");
    assert_eq!(bench.lines()[0], line0("hello, world!"));
    assert_eq!(bench.broke, ["low_supply"]);
}
