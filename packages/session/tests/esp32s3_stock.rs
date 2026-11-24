//! Stock Arduino-ESP32 sketches, built with the plain ESP32S3 Dev Module fqbn, on the
//! DevKitC-1. Each fixture's app and partition table are the images named in
//! `packages/engine/tests/firmware/esp32s3/arduino/stock/NOTICE`.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_session::{Command, Data, Files, Live, Met};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

#[path = "../../scene/tests/common/mod.rs"]
mod layout;

/// The fixture plus every catalog part, with each part's own parts nested inside its folder.
fn project(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = repo();
    copy_dir(
        &root.join("packages/scene/tests/fixtures").join(name),
        dir.path(),
    );
    let catalog = root.join("parts");
    let mut ids: Vec<String> = fs::read_dir(&catalog)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| catalog.join(id).join("part.jsonc").is_file())
        .collect();
    ids.sort();
    for id in &ids {
        let dest = dir.path().join("parts").join(id);
        copy_dir(&catalog.join(id), &dest);
        layout::nest_uses(&catalog, &dest, &mut Vec::new());
    }
    dir
}

fn run(live: &mut Live, project: &Path, line: &str) -> Data {
    let command = Command::parse(line, project).unwrap().unwrap();
    let reply = live.run(&command);
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.data
}

fn open(name: &str) -> (tempfile::TempDir, Live) {
    let project = project(name);
    let mut live = Live::new("t", Files::disk());
    let Data::Opened { diagnostics, .. } = run(&mut live, project.path(), "open") else {
        panic!("open opened nothing");
    };
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    (project, live)
}

fn play(name: &str, lines: &[&str]) {
    let (project, mut live) = open(name);
    for line in lines {
        run(&mut live, project.path(), line);
    }
}

/// The picosecond `pin` is at `level`: now, when it already is, or at its next edge.
fn edge_on(live: &mut Live, project: &Path, pin: &str, level: &str) -> u64 {
    let line = format!("step --until @board.{pin} {level} --timeout 30ms");
    let Data::Stepped(stepped) = run(live, project, &line) else {
        panic!("{line} did not step");
    };
    match stepped.met {
        Some(Met::Edge(ps) | Met::At(ps)) => ps,
        met => panic!("{line}: {met:?}"),
    }
}

/// `pin`'s next whole PWM cycle, from a rising edge: its period and its high time, in ps.
fn cycle_on(live: &mut Live, project: &Path, pin: &str) -> (u64, u64) {
    edge_on(live, project, pin, "low");
    let rise = edge_on(live, project, pin, "high");
    let fall = edge_on(live, project, pin, "low");
    let next = edge_on(live, project, pin, "high");
    (next - rise, fall - rise)
}

/// GPIO18's next whole PWM cycle, from a rising edge: its period and its high time, in ps.
fn cycle(live: &mut Live, project: &Path) -> (u64, u64) {
    cycle_on(live, project, "GPIO18")
}

/// `duty` of `2^bits` steps of `period` ps, to the nearest ps. The chip rounds the high time
/// to its 480 MHz tick, 2083 ps.
fn high(period: u64, duty: u64, bits: u32) -> u64 {
    (duty * period + (1 << (bits - 1))) >> bits
}

fn near(got: u64, want: u64, within: u64) -> bool {
    got.abs_diff(want) <= within
}

#[test]
fn a_stock_blink_lights_the_led_and_prints_ready() {
    play(
        "esp32s3-stock-blink",
        &[
            "step 300ms",
            "expect @led1 lit",
            r#"expect serial contains "ready""#,
            "step 500ms",
            "expect @led1 off",
            "expect no diagnostics",
        ],
    );
}

#[test]
fn a_stock_sketch_prints_ready_and_ticks() {
    play(
        "esp32s3-stock-serial",
        &[
            "step 300ms",
            r#"expect serial contains "ready""#,
            r#"expect serial contains "tick""#,
            "expect no diagnostics",
        ],
    );
}

#[test]
fn a_stock_sketch_reads_the_divider_in_millivolts() {
    play(
        "esp32s3-stock-adc",
        &[
            "step 400ms",
            r#"expect serial contains "raw 1779 mv 1650""#,
            "expect no diagnostics",
        ],
    );
}

#[test]
fn a_stock_sketch_reads_a_button() {
    play(
        "esp32s3-stock-button",
        &[
            "step 300ms",
            r#"expect serial contains "open""#,
            "press @btn1",
            "step 100ms",
            r#"expect serial contains "down""#,
            "expect no diagnostics",
        ],
    );
}

#[test]
fn a_stock_sketch_latches_the_board_rgb_led() {
    play(
        "esp32s3-stock-rgb",
        &[
            "step 300ms",
            "expect @board.led rgb(0,40,0)",
            r#"expect serial contains "ready""#,
            "expect no diagnostics",
        ],
    );
}

/// Each pixel of the stick and the colour it shows, `0xRRGGBB`, after each frame the sketch
/// sends: eight colours, then eight others 20 ms later.
const FIRST: [&str; 8] = [
    "rgb(255,0,0)",
    "rgb(0,255,0)",
    "rgb(0,0,255)",
    "rgb(255,255,0)",
    "rgb(0,255,255)",
    "rgb(255,0,255)",
    "rgb(16,32,48)",
    "rgb(128,64,32)",
];
const SECOND: [&str; 8] = [
    "rgb(1,2,3)",
    "rgb(64,0,0)",
    "rgb(0,64,0)",
    "rgb(0,0,64)",
    "rgb(128,128,128)",
    "rgb(10,20,30)",
    "rgb(192,255,238)",
    "rgb(255,255,255)",
];

/// The sketch sends each frame as one RMT transmission. Every pixel of the stick shows its
/// own colour, after the first frame and again after the second.
fn stick_frames(name: &str) {
    let (project, mut live) = open(name);
    let path = project.path();
    for (marker, colors) in [("first", FIRST), ("second", SECOND)] {
        run(
            &mut live,
            path,
            &format!(r#"step --until serial contains "{marker}""#),
        );
        run(&mut live, path, "step 1ms");
        for (k, color) in colors.iter().enumerate() {
            run(
                &mut live,
                path,
                &format!("expect @stick.px{} {color}", k + 1),
            );
        }
    }
    run(&mut live, path, "expect no diagnostics");
}

#[test]
fn a_stock_neopixel_sketch_lights_every_pixel_of_the_stick() {
    stick_frames("esp32s3-stock-neopixel");
}

#[test]
fn a_stock_rmt_write_lights_every_pixel_of_the_stick() {
    stick_frames("esp32s3-stock-rmtstrip");
}

#[test]
fn an_unpowered_stick_on_s3_latches_nothing_and_lights_after_power_returns() {
    let dir = project("esp32s3-stock-neopixel");
    let scene_path = dir.path().join("scene.jsonc");
    // Hold stick.5V at 0 V: neither frame latches.
    let zero_scene = fs::read_to_string(&scene_path).unwrap().replace(
        r#"["board.5V", "stick.5V"]"#,
        r#"["board.GND", "stick.5V"]"#,
    );
    fs::write(&scene_path, &zero_scene).unwrap();
    let mut live = Live::new("t", Files::disk());
    run(&mut live, dir.path(), "open");
    run(
        &mut live,
        dir.path(),
        r#"step --until serial contains "first""#,
    );
    run(&mut live, dir.path(), "step 1ms");
    for k in 1..=8 {
        let reply = live.run(
            &Command::parse(&format!("get @stick.px{k}.color"), dir.path())
                .unwrap()
                .unwrap(),
        );
        assert_eq!(
            reply.text(),
            format!("stick.px{k}.color rgb(0,0,0) not latched yet"),
            "0 V px{k}"
        );
    }

    // Leave stick.5V floating: neither frame latches.
    let float_scene = fs::read_to_string(&scene_path)
        .unwrap()
        .replace("    [\"board.GND\", \"stick.5V\"],\n", "");
    fs::write(&scene_path, &float_scene).unwrap();
    let mut live = Live::new("t", Files::disk());
    run(&mut live, dir.path(), "open");
    run(
        &mut live,
        dir.path(),
        r#"step --until serial contains "first""#,
    );
    run(&mut live, dir.path(), "step 1ms");
    for k in 1..=8 {
        let reply = live.run(
            &Command::parse(&format!("get @stick.px{k}.color"), dir.path())
                .unwrap()
                .unwrap(),
        );
        assert_eq!(
            reply.text(),
            format!("stick.px{k}.color rgb(0,0,0) not latched yet"),
            "floating px{k}"
        );
    }

    // Power switched on between the first and second frames: first is ignored, second lights every pixel.
    let switched_scene = r#"{
  "format": 1,
  "parts": {
    "board": {
      "type": "esp32s3-devkitc-1",
      "app": "app.bin",
      "partitions": "partitions.bin"
    },
    "sw": { "type": "tactile-switch-6mm" },
    "stick": { "type": "neopixel-stick-8" }
  },
  "wires": [
    ["board.GPIO4", "stick.DIN"],
    ["board.5V", "sw.a"],
    ["sw.b", "stick.5V"],
    ["board.GND", "stick.GND"]
  ],
  "ground": "board.GND"
}"#;
    fs::write(&scene_path, switched_scene).unwrap();
    let mut live = Live::new("t", Files::disk());
    run(&mut live, dir.path(), "open");
    run(
        &mut live,
        dir.path(),
        r#"step --until serial contains "first""#,
    );
    run(&mut live, dir.path(), "step 1ms");
    for k in 1..=8 {
        let reply = live.run(
            &Command::parse(&format!("get @stick.px{k}.color"), dir.path())
                .unwrap()
                .unwrap(),
        );
        assert_eq!(
            reply.text(),
            format!("stick.px{k}.color rgb(0,0,0) not latched yet"),
            "before power px{k}"
        );
    }
    run(&mut live, dir.path(), "press @sw");
    run(
        &mut live,
        dir.path(),
        r#"step --until serial contains "second""#,
    );
    run(&mut live, dir.path(), "step 1ms");
    for (k, color) in SECOND.iter().enumerate() {
        run(
            &mut live,
            dir.path(),
            &format!("expect @stick.px{} {color}", k + 1),
        );
    }
    run(&mut live, dir.path(), "expect no diagnostics");
}

const MS: u64 = 1_000_000_000;
const US: u64 = 1_000_000;
const TICK: u64 = 2_084;

/// `ledcAttach(18, 50, 14)`, then `ledcWrite` of 1.0, 1.5 and 2.0 ms of the 20 ms period, each
/// for 200 ms: 819, 1228 and 1638 of 16384 steps.
#[test]
fn a_stock_servo_signal_is_50_hz_with_its_pulse_widths() {
    let (project, mut live) = open("esp32s3-stock-servo");
    let path = project.path();
    for (until, duty) in [("1000", 819), ("1500", 1228), ("2000", 1638)] {
        run(
            &mut live,
            path,
            &format!(r#"step --until serial contains "pulse {until}""#),
        );
        let (period, high_ps) = cycle(&mut live, path);
        assert!(near(period, 20 * MS, 1), "{until}: period {period} ps");
        let want = high(20 * MS, duty, 14);
        assert!(
            near(high_ps, want, TICK),
            "{until}: high {high_ps} ps, want {want}"
        );
    }
    run(&mut live, path, "expect no diagnostics");
}

/// `analogWrite(18, 128)` at the core's defaults, 1 kHz and 8 bits: half of each 1 ms period.
#[test]
fn a_stock_analog_write_is_1_khz_at_half_duty() {
    let (project, mut live) = open("esp32s3-stock-analogwrite");
    let path = project.path();
    run(&mut live, path, r#"step --until serial contains "ready""#);
    let (period, high_ps) = cycle(&mut live, path);
    assert!(near(period, MS, 1), "period {period} ps");
    assert!(near(high_ps, 500 * US, 1), "high {high_ps} ps");
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcAttach(18, 5000, 8)`, then a `ledcWrite` 5 steps brighter every 30 ms. Each cycle's
/// high time is a multiple of 5 of the 256 steps of 200 us, and it grows.
#[test]
fn a_stock_fade_raises_its_duty_over_time() {
    let (project, mut live) = open("esp32s3-stock-fade");
    let path = project.path();
    let mut duties = Vec::new();
    for at in ["70ms", "130ms"] {
        run(&mut live, path, &format!("step {at}"));
        let (period, high_ps) = cycle(&mut live, path);
        assert!(near(period, 200 * US, 1), "period {period} ps");
        let duty = (1..256)
            .find(|&duty| near(high_ps, high(200 * US, duty, 8), TICK))
            .unwrap_or_else(|| panic!("high {high_ps} ps is no step"));
        assert_eq!(duty % 5, 0, "duty {duty}");
        duties.push(duty);
    }
    assert!(duties[0] > 0 && duties[1] > duties[0], "{duties:?}");
    run(&mut live, path, "expect no diagnostics");
}

/// `Wire.begin()` on the default pins, SDA on GPIO8 and SCL on GPIO9, with nothing on the bus.
#[test]
fn a_stock_wire_begin_raises_no_diagnostic() {
    play(
        "esp32s3-stock-wire",
        &[
            "step 300ms",
            r#"expect serial contains "ready""#,
            "expect no diagnostics",
        ],
    );
}

/// Adafruit SSD1306 and GFX draw a frame and "orbe" on the module. `display()` writes the frame
/// in I2C transactions longer than the 32-byte FIFO.
#[test]
fn a_stock_adafruit_ssd1306_sketch_draws_on_the_oled() {
    play(
        "esp32s3-stock-oled",
        &[
            r#"step --until serial contains "drawn" --timeout 1s"#,
            "expect @oled.display.screen shows oled.screen",
            "expect no diagnostics",
        ],
    );
}

/// One Wire write of 65 bytes and one read of 40, each one transaction over the 32-byte FIFO.
/// The write lights GDDRAM page 0's first 64 columns, which the power-on scan puts at the
/// glass's bottom right. The module answers the read with the released line, 0xFF, and its
/// warning that it takes no reads is the run's only diagnostic.
#[test]
fn a_stock_wire_write_and_read_run_past_the_fifo() {
    let (project, mut live) = open("esp32s3-stock-wirelong");
    let path = project.path();
    for line in [
        r#"step --until serial contains "read" --timeout 1s"#,
        r#"expect serial contains "init 0""#,
        r#"expect serial contains "wrote 0""#,
        r#"expect serial contains "read 40 ff 40""#,
        "expect @oled.display.screen shows wirelong.screen",
    ] {
        run(&mut live, path, line);
    }
    let command = Command::parse("snapshot", path).unwrap().unwrap();
    let text = live.run(&command).text();
    let found: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("warning[") || line.starts_with("error["))
        .collect();
    assert_eq!(found.len(), 1, "{text}");
    assert!(
        found[0].starts_with("warning[chip.protocol] #oled__display")
            && found[0].contains("rule read"),
        "{text}"
    );
}

/// `tone(4, 1000); delay(50); noTone(4);` toggles GPIO4 at 1 kHz and then leaves it steady low.
#[test]
fn a_stock_tone_and_notone_toggle_at_1_khz_then_stop_low() {
    let (project, mut live) = open("esp32s3-stock-tone");
    let path = project.path();
    run(&mut live, path, r#"step --until serial contains "tone""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, MS, 1), "period {period} ps");
    let want = high(MS, 0x1ff, 10);
    assert!(near(high_ps, want, TICK), "high {high_ps} ps, want {want}");
    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "step 10ms");
    run(&mut live, path, "expect @board.GPIO4 low");
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcAttach(4, 5000, 10)` drives 5 kHz PWM, then `ledcDetach(4)` returns GPIO4 to plain GPIO.
#[test]
fn a_stock_ledc_detach_returns_the_pin_to_plain_gpio() {
    let (project, mut live) = open("esp32s3-stock-ledcdetach");
    let path = project.path();
    run(
        &mut live,
        path,
        r#"step --until serial contains "attached""#,
    );
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 200 * US, 1), "period {period} ps");
    assert!(near(high_ps, 100 * US, TICK), "high {high_ps} ps");
    run(
        &mut live,
        path,
        r#"step --until serial contains "gpio high""#,
    );
    run(&mut live, path, "step 2ms");
    run(&mut live, path, "expect @board.GPIO4 high");
    run(
        &mut live,
        path,
        r#"step --until serial contains "gpio low""#,
    );
    run(&mut live, path, "step 2ms");
    run(&mut live, path, "expect @board.GPIO4 low");
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcFade(4, 0, 1023, 100)` ramps duty from 0 to full over ~102.4 ms (512 steps of +2 at
/// 5 kHz, 10 bits), then `ledcFadeWithInterrupt(4, 1023, 0, 100, onFadeDone)` ramps back down
/// to 0 and runs the fade-end callback.
#[test]
fn a_stock_ledc_fade_ramps_duty_and_fires_its_callback() {
    let (project, mut live) = open("esp32s3-stock-ledcfade");
    let path = project.path();
    run(&mut live, path, r#"step --until serial contains "fade1""#);
    // At 5 kHz (200 us period) and 10 bits (1024 steps), IDF programs scale=2, cycle=1, step=512
    // so duty increases by 2 every 200 us (10 steps/ms). Sample three points along the ramp.
    let mut prev = 0u64;
    let mut prev_t = 0u64;
    for step_by in ["25ms", "25ms", "25ms"] {
        run(&mut live, path, &format!("step {step_by}"));
        edge_on(&mut live, path, "GPIO4", "low");
        let rise = edge_on(&mut live, path, "GPIO4", "high");
        let fall = edge_on(&mut live, path, "GPIO4", "low");
        let next = edge_on(&mut live, path, "GPIO4", "high");
        let period = next - rise;
        let high_ps = fall - rise;
        assert!(near(period, 200 * US, 1), "period {period} ps");
        let duty = (1..1024)
            .find(|&d| near(high_ps, high(200 * US, d, 10), TICK))
            .unwrap_or_else(|| panic!("high {high_ps} ps is no 10-bit step"));
        assert_eq!(duty % 2, 0, "scale=2 leaves even duties, got {duty}");
        if prev_t != 0 {
            // Each 200 us cycle adds 2 to duty.
            let cycles = (rise - prev_t + 100 * US) / (200 * US);
            assert_eq!(
                duty,
                prev + 2 * cycles,
                "TRM fade step math: prev {prev} + 2 * {cycles} cycles != {duty}"
            );
        }
        prev = duty;
        prev_t = rise;
    }
    assert!(prev > 600 && prev < 900, "75 ms sample {prev}");
    run(&mut live, path, r#"step --until serial contains "duty1""#);
    run(&mut live, path, r#"expect serial contains "duty1 1024""#);
    run(&mut live, path, "expect @board.GPIO4 high");

    run(&mut live, path, r#"step --until serial contains "fade2""#);
    // Fading down from 1024 -> 0: IDF starts at 1023 with scale=2, cycle=1, step=511, then
    // finishes the remainder 1 -> 0 in the ISR. Sample two points on the ramp down.
    let mut prev_down = 0u64;
    let mut prev_down_t = 0u64;
    for step_by in ["30ms", "30ms"] {
        run(&mut live, path, &format!("step {step_by}"));
        edge_on(&mut live, path, "GPIO4", "low");
        let rise = edge_on(&mut live, path, "GPIO4", "high");
        let fall = edge_on(&mut live, path, "GPIO4", "low");
        let next = edge_on(&mut live, path, "GPIO4", "high");
        let period = next - rise;
        let high_ps = fall - rise;
        assert!(near(period, 200 * US, 1), "down period {period} ps");
        let duty = (1..1024)
            .find(|&d| near(high_ps, high(200 * US, d, 10), TICK))
            .unwrap_or_else(|| panic!("down high {high_ps} ps is no 10-bit step"));
        assert_eq!(duty % 2, 1, "1023 - 2*k leaves odd duties, got {duty}");
        if prev_down_t != 0 {
            let cycles = (rise - prev_down_t + 100 * US) / (200 * US);
            assert_eq!(
                duty + 2 * cycles,
                prev_down,
                "TRM down fade step math: {prev_down} - 2 * {cycles} cycles != {duty}"
            );
        }
        prev_down = duty;
        prev_down_t = rise;
    }
    run(
        &mut live,
        path,
        r#"step --until serial contains "done 1 duty2 0""#,
    );
    run(&mut live, path, "expect @board.GPIO4 low");
    run(&mut live, path, "expect no diagnostics");
}

/// `tone(4, 1000)`, `delay`, `noTone(4)`, `delay`, `tone(4, 2500)`: GPIO4 runs 1 kHz, goes quiet,
/// then runs 2.5 kHz at 50% duty.
#[test]
fn a_stock_tone_notone_and_second_tone_change_frequency() {
    let (project, mut live) = open("esp32s3-stock-tonerestart");
    let path = project.path();
    run(&mut live, path, r#"step --until serial contains "tone1""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, MS, 1), "tone1 period {period} ps");
    let want = high(MS, 0x1ff, 10);
    assert!(
        near(high_ps, want, TICK),
        "tone1 high {high_ps} ps, want {want}"
    );
    run(&mut live, path, r#"step --until serial contains "quiet""#);
    run(&mut live, path, "step 5ms");
    run(&mut live, path, "expect @board.GPIO4 low");
    run(&mut live, path, "step 5ms");
    run(&mut live, path, "expect @board.GPIO4 low");
    run(&mut live, path, r#"step --until serial contains "tone2""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 400 * US, 1), "tone2 period {period} ps");
    let want = high(400 * US, 0x1ff, 10);
    assert!(
        near(high_ps, want, TICK),
        "tone2 high {high_ps} ps, want {want}"
    );
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcAttach(4, 5000, 10)`, `ledcWrite(4, 512)`, `ledcDetach(4)`, `ledcAttach(4, 2000, 8)`,
/// `ledcWrite(4, 64)`: GPIO4 ends at 2 kHz duty 64/256.
#[test]
fn a_stock_ledc_detach_and_reattach_updates_frequency_and_resolution() {
    let (project, mut live) = open("esp32s3-stock-ledcreattach");
    let path = project.path();
    run(&mut live, path, r#"step --until serial contains "first""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 200 * US, 1), "first period {period} ps");
    let want = high(200 * US, 512, 10);
    assert!(
        near(high_ps, want, TICK),
        "first high {high_ps} ps, want {want}"
    );
    run(
        &mut live,
        path,
        r#"step --until serial contains "detached""#,
    );
    run(&mut live, path, "step 2ms");
    run(&mut live, path, "expect @board.GPIO4 low");
    run(&mut live, path, r#"step --until serial contains "second""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 500 * US, 1), "second period {period} ps");
    let want = high(500 * US, 64, 8);
    assert!(
        near(high_ps, want, TICK),
        "second high {high_ps} ps, want {want}"
    );
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcFadeWithInterrupt(4, 0, 200, 400, onFadeDone)` followed by `ledcDetach(4)` mid-fade stops
/// the fade, returns GPIO4 to plain GPIO, and fires no fade-end callback even after reattaching.
#[test]
fn a_stock_ledc_detach_mid_fade_stops_fade_returns_gpio_and_fires_no_callback() {
    let (project, mut live) = open("esp32s3-stock-ledcfadedetach");
    let path = project.path();
    run(&mut live, path, r#"step --until serial contains "fading""#);
    run(&mut live, path, "step 20ms");
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 200 * US, 1), "fade period {period} ps");
    let duty = (1..200)
        .find(|&d| near(high_ps, high(200 * US, d, 10), TICK))
        .unwrap_or_else(|| panic!("mid-fade high {high_ps} ps is no 10-bit step in 1..200"));
    assert!(duty > 0 && duty < 200, "mid-fade duty {duty}");
    run(
        &mut live,
        path,
        r#"step --until serial contains "gpio high""#,
    );
    run(&mut live, path, "step 5ms");
    run(&mut live, path, "expect @board.GPIO4 high");
    run(
        &mut live,
        path,
        r#"step --until serial contains "gpio low""#,
    );
    run(&mut live, path, "step 5ms");
    run(&mut live, path, "expect @board.GPIO4 low");
    run(
        &mut live,
        path,
        r#"step --until serial contains "after_detach 0""#,
    );
    run(
        &mut live,
        path,
        r#"step --until serial contains "reattached""#,
    );
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 500 * US, 1), "reattach period {period} ps");
    let want = high(500 * US, 64, 8);
    assert!(
        near(high_ps, want, TICK),
        "reattach high {high_ps} ps, want {want}"
    );
    run(
        &mut live,
        path,
        r#"step --until serial contains "after_wait 0" --timeout 1s"#,
    );
    run(&mut live, path, "expect no diagnostics");
}

/// A four-note melody loop with `tone()` and `noTone()` plays each note at its frequency on GPIO4.
#[test]
fn a_stock_melody_plays_each_note_at_its_frequency() {
    let (project, mut live) = open("esp32s3-stock-melody");
    let path = project.path();
    for (i, freq) in [(0, 262u64), (1, 330), (2, 392), (3, 523)] {
        run(
            &mut live,
            path,
            &format!(r#"step --until serial contains "note {i} {freq}""#),
        );
        let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
        let got_hz = (1_000_000_000_000 + period / 2) / period;
        assert!(
            got_hz.abs_diff(freq) <= 1,
            "note {i}: got {got_hz} Hz, want {freq} Hz (period {period} ps)"
        );
        let want = high(period, 0x1ff, 10);
        assert!(
            near(high_ps, want, TICK),
            "note {i}: high {high_ps} ps, want {want}"
        );
        run(
            &mut live,
            path,
            &format!(r#"step --until serial contains "rest {i}""#),
        );
        run(&mut live, path, "step 3ms");
        run(&mut live, path, "expect @board.GPIO4 low");
    }
    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "expect no diagnostics");
}

#[test]
fn a_stock_sketch_configures_rtc_pads_with_no_diagnostics() {
    play(
        "esp32s3-stock-rtcio",
        &[
            "step 300ms",
            r#"expect serial contains "adc 1779 1178 2431 spi ff""#,
            r#"expect serial contains "ready""#,
            "expect @board.GPIO7 high",
            "expect @board.GPIO8 high",
            "expect @board.GPIO10 high",
            "expect @board.GPIO15 high",
            "expect @board.GPIO17 high",
            "expect @board.GPIO21 high",
            "expect no diagnostics",
        ],
    );
}
