//! The SSD1306 decoder, called the way the I2C engine calls it: a START, the bytes, a STOP.

use orbe_model::{
    Ack, Cx, Device, Dir, Drawn, Fields, I2cTarget, Levels, Time, Value as FieldValue, View,
};

use super::model::{Addressing, FRAME_BYTES, Ssd1306, WATCH_RES};
use crate::display::panel;

/// The init Adafruit SSD1306 2.5 sends a 128 by 64 panel with its charge pump, split into the
/// transfers it sends them in. A parameter can come in a transfer of its own.
const ADAFRUIT_INIT: [&[u8]; 13] = [
    &[0x00, 0xae, 0xd5, 0x80, 0xa8],
    &[0x00, 0x3f],
    &[0x00, 0xd3, 0x00, 0x40, 0x8d],
    &[0x00, 0x14],
    &[0x00, 0x20, 0x00, 0xa1, 0xc8],
    &[0x00, 0xda],
    &[0x00, 0x12],
    &[0x00, 0x81],
    &[0x00, 0xcf],
    &[0x00, 0xd9],
    &[0x00, 0xf1],
    &[0x00, 0xdb, 0x40, 0xa4, 0xa6, 0x2e, 0xaf],
    &[0x00, 0x22, 0x00, 0xff, 0x21, 0x00, 0x7f],
];

struct Bus {
    dev: Ssd1306,
    t: u64,
    broke: Vec<&'static str>,
    drawn: Vec<Drawn>,
}

impl Bus {
    fn new() -> Bus {
        let mut dev = Ssd1306::new(&Fields::NONE).unwrap();
        dev.input(&mut Cx::default(), Time::ZERO, powered(true));
        Bus {
            dev,
            t: 0,
            broke: Vec::new(),
            drawn: Vec::new(),
        }
    }

    fn at(&mut self) -> Time {
        self.t += 1000;
        Time::from_ps(self.t)
    }

    fn keep(&mut self, cx: Cx) {
        if let Some((rule, _)) = cx.broken() {
            self.broke.push(rule);
        }
        if let Some(drawn) = cx.drawn() {
            self.drawn.push(drawn);
        }
    }

    /// One write transfer of `bytes`, START to STOP.
    fn send(&mut self, bytes: &[u8]) {
        let (mut cx, at) = (Cx::default(), self.at());
        self.dev.start(&mut cx, at, Dir::Write);
        self.keep(cx);
        for &byte in bytes {
            let (mut cx, at) = (Cx::default(), self.at());
            self.dev.write(&mut cx, at, byte);
            self.keep(cx);
        }
        let (mut cx, at) = (Cx::default(), self.at());
        self.dev.stop(&mut cx, at);
        self.keep(cx);
    }

    fn init(&mut self) {
        for transfer in ADAFRUIT_INIT {
            self.send(transfer);
        }
    }

    fn frame(&self) -> &[u8] {
        match self.dev.view() {
            View::Screen { frame, .. } => frame,
            _ => unreachable!(),
        }
    }

    /// The glass's lit pixels, as `(x, y)`.
    fn lit(&self) -> Vec<(usize, usize)> {
        let view = self.dev.view();
        let pixels = super::glass(&panel(Ssd1306::SHOWS, view), self.frame());
        (0..128 * 64)
            .filter(|k| pixels[k * 4] == 255)
            .map(|k| (k % 128, k / 128))
            .collect()
    }
}

#[test]
fn the_adafruit_init_leaves_the_registers_it_sends() {
    let mut bus = Bus::new();
    bus.init();
    let r = *bus.dev.registers();
    assert!(r.display_on);
    assert_eq!(r.multiplex, 63, "a parameter in a transfer of its own");
    assert_eq!(r.charge_pump, 0x14);
    assert_eq!(r.addressing, Addressing::Horizontal);
    assert!(r.segment_remap && r.com_scan);
    assert_eq!((r.com_pins, r.contrast), (0x12, 0xcf));
    assert_eq!((r.columns, r.pages), ((0, 127), (0, 7)));
    assert_eq!(r.stored, 0b111);
    assert!(bus.broke.is_empty(), "{:?}", bus.broke);
    assert!(bus.drawn.is_empty());
}

#[test]
fn a_frame_in_31_byte_transfers_fills_the_ram_and_draws_one_row_each() {
    let mut bus = Bus::new();
    bus.init();
    let picture: Vec<u8> = (0..FRAME_BYTES).map(|k| (k * 7 % 251) as u8).collect();
    for chunk in picture.chunks(31) {
        let mut transfer = vec![0x40];
        transfer.extend_from_slice(chunk);
        bus.send(&transfer);
    }
    assert_eq!(bus.drawn.len(), FRAME_BYTES.div_ceil(31));
    assert_eq!(bus.drawn[0].bytes, 31);
    assert_eq!(
        (
            bus.drawn[0].x0,
            bus.drawn[0].y0,
            bus.drawn[0].x1,
            bus.drawn[0].y1
        ),
        (0, 0, 127, 63)
    );
    // Segment remap stores column address c at segment 127 - c.
    for page in 0..8 {
        for column in 0..128 {
            assert_eq!(
                bus.frame()[page * 128 + 127 - column],
                picture[page * 128 + column],
                "page {page} column {column}"
            );
        }
    }
}

#[test]
fn text_drawn_at_the_top_left_lights_the_top_left_of_the_glass() {
    let mut bus = Bus::new();
    bus.init();
    let mut picture = vec![0u8; FRAME_BYTES];
    // A 2 by 2 square at x 1 to 2, y 9 to 10: page 1, bits 1 and 2.
    picture[128 + 1] = 0b110;
    picture[128 + 2] = 0b110;
    let mut transfer = vec![0x40];
    transfer.extend_from_slice(&picture);
    bus.send(&transfer);
    assert_eq!(bus.lit(), [(1, 9), (2, 9), (1, 10), (2, 10)]);
    bus.send(&[0x80, 0xa7]);
    assert_eq!(bus.lit().len(), 128 * 64 - 4, "inverse display");
    bus.send(&[0x80, 0xae]);
    assert!(bus.lit().is_empty(), "display off");
}

#[test]
fn page_addressing_writes_where_u8g2_points_and_wraps_in_its_page() {
    let mut bus = Bus::new();
    bus.init();
    bus.send(&[0x00, 0x20, 0x02]);
    // Page 3, column 0x7e, as U8g2 points with 10h, 00h and B0h.
    bus.send(&[0x00, 0x17, 0x0e, 0xb3]);
    bus.send(&[0x40, 1, 2, 3]);
    let ram = bus.frame();
    // Past column 127 the pointer goes back to the column 00h and 10h set, in the same page.
    assert_eq!(ram[3 * 128 + 127 - 0x7f], 2);
    assert_eq!(ram[3 * 128 + 127 - 0x7e], 3);
    assert_eq!(bus.dev.registers().column, 0x7f);
    assert_eq!(bus.dev.registers().page, 3);
    assert_eq!(
        bus.drawn.last().map(|d| (d.x0, d.y0, d.x1, d.y1)),
        Some((0x7e, 24, 127, 31))
    );
}

#[test]
fn vertical_addressing_runs_down_the_pages_first() {
    let mut bus = Bus::new();
    bus.init();
    bus.send(&[0x00, 0x20, 0x01, 0x21, 0x10, 0x11, 0x22, 0x02, 0x03]);
    bus.send(&[0x40, 1, 2, 3, 4, 5]);
    let ram = bus.frame();
    let at = |page: usize, column: usize| ram[page * 128 + 127 - column];
    // The fifth byte wraps back to the window's first column and page.
    assert_eq!(
        [at(2, 0x10), at(3, 0x10), at(2, 0x11), at(3, 0x11)],
        [5, 2, 3, 4]
    );
    assert_eq!(
        (bus.dev.registers().column, bus.dev.registers().page),
        (0x10, 3)
    );
}

#[test]
fn a_command_with_co_set_is_one_byte_then_a_control_byte() {
    let mut bus = Bus::new();
    bus.send(&[0x80, 0x81, 0x80, 0x42, 0x80, 0xaf, 0xc0, 0x5a]);
    let r = *bus.dev.registers();
    assert_eq!(r.contrast, 0x42);
    assert!(r.display_on);
    assert_eq!(bus.frame()[0], 0x5a, "a single data byte after Co and D/C#");
}

#[test]
fn scroll_reads_and_unknown_commands_break_their_rules_by_name() {
    let mut bus = Bus::new();
    bus.send(&[0x00, 0x26, 0, 0, 0, 7, 0, 0xff, 0x2f]);
    // An unknown command's next byte is a command of its own.
    bus.send(&[0x00, 0xd6, 0x01]);
    bus.send(&[0x00, 0x20, 0x03]);
    bus.send(&[0x00, 0xa8, 0x0e]);
    assert_eq!(
        bus.broke,
        ["scroll", "scroll", "command", "command", "command"]
    );
    assert_eq!(bus.dev.registers().addressing, Addressing::Page);
    assert_eq!(bus.dev.registers().multiplex, 63);
    let mut cx = Cx::default();
    bus.dev.start(&mut cx, Time::from_ps(1), Dir::Read);
    assert_eq!(cx.broken().map(|(rule, _)| rule), Some("read"));
    assert_eq!(bus.dev.read(&mut Cx::default(), Time::from_ps(2)), 0xff);
}

#[test]
fn res_low_resets_the_registers_and_keeps_the_ram() {
    let mut bus = Bus::new();
    bus.init();
    bus.send(&[0x40, 0xaa]);
    bus.dev
        .input(&mut Cx::default(), Time::from_ps(9), powered_res(false));
    let r = *bus.dev.registers();
    assert!(!r.display_on);
    assert_eq!(r.addressing, Addressing::Page);
    assert_eq!(bus.frame()[127], 0xaa);
    let _ = WATCH_RES;
}

#[test]
fn a_supply_below_the_minimum_nacks_and_the_glass_stays_dark_until_power_returns() {
    let mut bus = Bus::new();
    bus.init();
    bus.send(&[0x40, 0xff]);
    assert!(!bus.lit().is_empty(), "the pump is on and a byte is set");
    bus.dev
        .input(&mut Cx::default(), Time::from_ps(9), powered(false));
    assert!(bus.lit().is_empty(), "an unpowered glass is dark");
    assert_eq!(bus.frame(), &[0; FRAME_BYTES], "the GDDRAM is lost");
    assert!(!bus.dev.awake());
    assert_eq!(
        bus.dev.write(&mut Cx::default(), Time::from_ps(10), 0x00),
        Ack::No
    );
    assert_eq!(bus.frame(), &[0; FRAME_BYTES], "a NACK writes nothing");
    bus.dev
        .input(&mut Cx::default(), Time::from_ps(11), powered(true));
    assert!(bus.dev.awake());
    assert!(!bus.dev.registers().display_on);
    assert_eq!(bus.dev.registers().charge_pump, 0x10);
    bus.init();
    assert!(bus.dev.registers().display_on);
    assert_eq!(bus.dev.registers().charge_pump, 0x14);
    assert!(bus.lit().is_empty(), "power-on RAM is empty");
}

/// The panel part's draw: 1 uA asleep, 180 uA of logic, and the pump's 17.3 mA with half the
/// glass lit and 23.1 mA with all of it, held above 1.65 V.
const DRAW: [(&str, FieldValue); 5] = [
    ("sleep_na", FieldValue::Int(1_000)),
    ("logic_na", FieldValue::Int(180_000)),
    ("half_lit_na", FieldValue::Int(17_300_000)),
    ("all_lit_na", FieldValue::Int(23_100_000)),
    ("holds_above_mv", FieldValue::Int(1_650)),
];

fn amps(bus: &Bus) -> f64 {
    let draw = bus.dev.draw();
    assert!((draw.holds_above_v - 1.65).abs() < 1e-12);
    draw.amps
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12
}

#[test]
fn the_draw_sleeps_with_the_display_off_and_follows_the_lit_pixels_with_it_on() {
    let mut bus = Bus::new();
    bus.dev = Ssd1306::new(&Fields::new(&DRAW)).unwrap();
    assert!(
        close(amps(&bus), 1e-6),
        "unpowered, it is the sleep current"
    );
    bus.dev
        .input(&mut Cx::default(), Time::from_ps(1), powered(true));
    assert!(close(amps(&bus), 1e-6), "the display is off after power-on");
    bus.init();
    // The line through half and all lit meets 11.5 mA with none lit.
    assert!(close(amps(&bus), 180e-6 + 11.5e-3), "{}", amps(&bus));
    bus.send(&[0x40, 0xff, 0x0f]);
    assert!(close(
        amps(&bus),
        180e-6 + 11.5e-3 + 11.6e-3 * 12.0 / 8192.0
    ));
    bus.send(&[0x00, 0xa5]);
    assert!(
        close(amps(&bus), 180e-6 + 23.1e-3),
        "entire display on lights all"
    );
    bus.send(&[0x00, 0xa4, 0xa7]);
    assert!(close(
        amps(&bus),
        180e-6 + 11.5e-3 + 11.6e-3 * 8180.0 / 8192.0
    ));
    bus.send(&[0x00, 0x8d, 0x10]);
    assert!(
        close(amps(&bus), 180e-6),
        "with the pump off, the logic alone"
    );
    bus.send(&[0x00, 0x8d, 0x14, 0xae]);
    assert!(close(amps(&bus), 1e-6), "display off is sleep");
    bus.send(&[0x00, 0xaf]);
    bus.dev
        .input(&mut Cx::default(), Time::from_ps(9), powered_res(false));
    assert!(close(amps(&bus), 1e-6), "RES# low holds it reset");
    bus.dev
        .input(&mut Cx::default(), Time::from_ps(10), powered(false));
    assert!(close(amps(&bus), 1e-6), "below the logic minimum");
    assert!(close(Ssd1306::new(&Fields::NONE).unwrap().draw().amps, 0.0));
}

/// SCL and SDA high, RES# high, and the supply watch at `on`.
fn powered(on: bool) -> Levels {
    powered_res_supply(true, on)
}

/// SCL and SDA high, RES# at `res_high`, supply at or above the minimum.
fn powered_res(res_high: bool) -> Levels {
    powered_res_supply(res_high, true)
}

fn powered_res_supply(res_high: bool, on: bool) -> Levels {
    Levels::from_slice(&[true, true, res_high, on])
}
