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
    // Driving DIN while stick.5V is switched off (before sw closes) rightly warns `rating.voltage`
    // on the first pixel's DIN (VDD + 0.5 V); no other diagnostic should appear.
    let command = Command::parse("snapshot", dir.path()).unwrap().unwrap();
    let text = live.run(&command).text();
    let found: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("warning[") || line.starts_with("error["))
        .collect();
    assert_eq!(found.len(), 1, "{text}");
    assert!(
        found[0].starts_with("warning[rating.voltage] #stick__px1.pin.DIN"),
        "{text}"
    );
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

/// Opens `fixture` with `scene` replacing its `scene.jsonc`.
fn open_scene(fixture: &str, scene: &str) -> (tempfile::TempDir, Live) {
    let project = project(fixture);
    fs::write(project.path().join("scene.jsonc"), scene).unwrap();
    let mut live = Live::new("t", Files::disk());
    let Data::Opened { diagnostics, .. } = run(&mut live, project.path(), "open") else {
        panic!("open opened nothing");
    };
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    (project, live)
}

/// Steps up to 300 ms and asserts the run faults with `chip.link_unsupported`, returning both
/// the failed `step` text and the following `snapshot` text that a user sees.
fn expect_link_unsupported(fixture: &str, scene: &str) -> (String, String) {
    let (project, mut live) = open_scene(fixture, scene);
    let path = project.path();
    let step = Command::parse("step 300ms", path).unwrap().unwrap();
    let reply = live.run(&step);
    assert!(
        !reply.ok(),
        "expected step to fault with chip.link_unsupported, but it succeeded:\n{}",
        reply.text()
    );
    let step_text = reply.text();
    assert!(
        step_text.contains("chip.link_unsupported"),
        "step text:\n{step_text}"
    );
    let snapshot = Command::parse("snapshot", path).unwrap().unwrap();
    let snap_text = live.run(&snapshot).text();
    assert!(
        snap_text.contains("error[chip.link_unsupported]"),
        "snapshot text:\n{snap_text}"
    );
    (step_text, snap_text)
}

#[test]
fn a_stock_neopixel_through_a_series_resistor_stops_with_link_unsupported() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "board": { "type": "esp32s3-devkitc-1", "app": "app.bin", "partitions": "partitions.bin" },
        "r1": { "type": "resistor-axial", "resistance": "330ohm" },
        "stick": { "type": "neopixel-stick-8" }
      },
      "wires": [
        ["board.GPIO4", "r1.a"],
        ["r1.b", "stick.DIN"],
        ["board.5V", "stick.5V"],
        ["board.GND", "stick.GND"]
      ],
      "ground": "board.GND"
    }"#;
    let (step_text, snap_text) = expect_link_unsupported("esp32s3-stock-neopixel", scene);
    assert!(!snap_text.contains("first"), "{snap_text}");
    for text in [&step_text, &snap_text] {
        assert!(text.contains("board.GPIO4"), "{text}");
        assert!(text.contains("RMT"), "{text}");
        assert!(text.contains("#r1.pin.a"), "{text}");
        assert!(
            text.contains("the data line must be wired straight to the pixel's data input"),
            "{text}"
        );
    }
}

#[test]
fn a_stock_neopixel_through_a_bss138_level_shifter_stops_with_link_unsupported() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "board": { "type": "esp32s3-devkitc-1", "app": "app.bin", "partitions": "partitions.bin" },
        "shifter": { "type": "bss138-level-shifter" },
        "stick": { "type": "neopixel-stick-8" }
      },
      "wires": [
        ["board.3V3", "shifter.LV"],
        ["board.5V", "shifter.HV"],
        ["board.5V", "stick.5V"],
        ["board.GND", "shifter.GND"],
        ["board.GND", "stick.GND"],
        ["board.GPIO4", "shifter.LV1"],
        ["shifter.HV1", "stick.DIN"]
      ],
      "ground": "board.GND"
    }"#;
    let (step_text, snap_text) = expect_link_unsupported("esp32s3-stock-neopixel", scene);
    assert!(!snap_text.contains("first"), "{snap_text}");
    for text in [&step_text, &snap_text] {
        assert!(text.contains("board.GPIO4"), "{text}");
        assert!(text.contains("RMT"), "{text}");
        assert!(text.contains("shifter.LV1"), "{text}");
        assert!(
            text.contains("the data line must be wired straight to the pixel's data input"),
            "{text}"
        );
    }
}

#[test]
fn a_stock_neopixel_with_a_pull_down_resistor_still_works() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "board": { "type": "esp32s3-devkitc-1", "app": "app.bin", "partitions": "partitions.bin" },
        "rpd": { "type": "resistor-axial", "resistance": "10k" },
        "stick": { "type": "neopixel-stick-8" }
      },
      "wires": [
        ["board.GPIO4", "stick.DIN"],
        ["board.GPIO4", "rpd.a"],
        ["rpd.b", "board.GND"],
        ["board.5V", "stick.5V"],
        ["board.GND", "stick.GND"]
      ],
      "ground": "board.GND"
    }"#;
    let (project, mut live) = open_scene("esp32s3-stock-neopixel", scene);
    let path = project.path();
    run(&mut live, path, r#"step --until serial contains "first""#);
    run(&mut live, path, "step 1ms");
    for (k, color) in FIRST.iter().enumerate() {
        run(
            &mut live,
            path,
            &format!("expect @stick.px{} {color}", k + 1),
        );
    }
    run(&mut live, path, "expect no diagnostics");
}

#[test]
fn a_stock_spi_loopback_stops_with_link_unsupported() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "board": { "type": "esp32s3-devkitc-1", "app": "app.bin", "partitions": "partitions.bin" }
      },
      "wires": [
        ["board.GPIO11", "board.GPIO13"]
      ],
      "ground": "board.GND"
    }"#;
    let (step_text, snap_text) = expect_link_unsupported("esp32s3-stock-rtcio", scene);
    assert!(!snap_text.contains("ready"), "{snap_text}");
    for text in [&step_text, &snap_text] {
        assert!(text.contains("board.GPIO11"), "{text}");
        assert!(text.contains("board.GPIO13"), "{text}");
        assert!(text.contains("SPI2"), "{text}");
        assert!(
            text.contains("runs hardware SPI only into parts that take SPI directly"),
            "{text}"
        );
        assert!(text.contains("shiftOut or bit-banged SPI"), "{text}");
    }
}

#[test]
fn a_stock_spi_into_hc595_stops_with_link_unsupported() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "board": { "type": "esp32s3-devkitc-1", "app": "app.bin", "partitions": "partitions.bin" },
        "sr": { "type": "hc595-dip16" }
      },
      "wires": [
        ["board.3V3", "sr.VCC"],
        ["board.3V3", "sr.SRCLR"],
        ["board.GND", "sr.GND"],
        ["board.GND", "sr.OE"],
        ["board.GPIO11", "sr.SER"],
        ["board.GPIO12", "sr.SRCLK"],
        ["board.GPIO10", "sr.RCLK"]
      ],
      "ground": "board.GND"
    }"#;
    let (step_text, snap_text) = expect_link_unsupported("esp32s3-stock-rtcio", scene);
    assert!(!snap_text.contains("ready"), "{snap_text}");
    for text in [&step_text, &snap_text] {
        assert!(text.contains("SPI2"), "{text}");
        assert!(
            text.contains("board.GPIO12") || text.contains("board.GPIO11"),
            "{text}"
        );
        assert!(
            text.contains("#sr.pin.SRCLK") || text.contains("#sr.pin.SER"),
            "{text}"
        );
        assert!(
            text.contains("runs hardware SPI only into parts that take SPI directly"),
            "{text}"
        );
        assert!(text.contains("shiftOut or bit-banged SPI"), "{text}");
    }
}

/// Five `tone(4, f, 200)` notes back to back with no `noTone()` between play each note's
/// frequency on GPIO4 at 50% duty.
#[test]
fn a_stock_tone_melody_without_notone_plays_each_note() {
    let (project, mut live) = open("esp32s3-stock-tonemelody");
    let path = project.path();
    for (i, freq) in [(0, 262u64), (1, 294), (2, 330), (3, 349), (4, 392)] {
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
    }
    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcAttach(4, 1000, 10)` then `ledcWriteTone(4, 440)` then `ledcWriteTone(4, 880)` plays
/// 440 Hz then 880 Hz at 50% duty on GPIO4.
#[test]
fn a_stock_ledc_write_tone_changes_frequency_while_running() {
    let (project, mut live) = open("esp32s3-stock-ledctone");
    let path = project.path();
    for (marker, freq) in [("tone440", 440u64), ("tone880", 880)] {
        run(
            &mut live,
            path,
            &format!(r#"step --until serial contains "{marker}""#),
        );
        let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
        let got_hz = (1_000_000_000_000 + period / 2) / period;
        assert!(
            got_hz.abs_diff(freq) <= 1,
            "{marker}: got {got_hz} Hz, want {freq} Hz (period {period} ps)"
        );
        let want = high(period, 0x1ff, 10);
        assert!(
            near(high_ps, want, TICK),
            "{marker}: high {high_ps} ps, want {want}"
        );
    }
    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcAttach(4, 1000, 10)`, `ledcWrite(4, 256)`, then `ledcChangeFrequency(4, 2000, 10)` keeps
/// 25% duty while moving GPIO4 from 1 kHz to 2 kHz.
#[test]
fn a_stock_ledc_change_frequency_keeps_duty_share_while_running() {
    let (project, mut live) = open("esp32s3-stock-ledcfreq");
    let path = project.path();
    for (marker, want_period) in [("first", MS), ("second", 500 * US)] {
        run(
            &mut live,
            path,
            &format!(r#"step --until serial contains "{marker}""#),
        );
        let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
        assert!(
            near(period, want_period, 1),
            "{marker}: period {period} ps, want {want_period}"
        );
        let want_high = high(period, 256, 10);
        assert!(
            near(high_ps, want_high, TICK),
            "{marker}: high {high_ps} ps, want {want_high}"
        );
    }
    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "expect no diagnostics");
}

/// Two pins attached at the same frequency and resolution share one timer; `ledcChangeFrequency`
/// on one moves both pins to the new frequency while each keeps its own duty.
#[test]
fn a_stock_ledc_shared_timer_frequency_change_moves_both_pins() {
    let (project, mut live) = open("esp32s3-stock-ledcshared");
    let path = project.path();
    for (marker, want_period) in [("first", MS), ("second", 500 * US)] {
        run(
            &mut live,
            path,
            &format!(r#"step --until serial contains "{marker}""#),
        );
        for (pin, duty) in [("GPIO4", 256u64), ("GPIO5", 768)] {
            let (period, high_ps) = cycle_on(&mut live, path, pin);
            assert!(
                near(period, want_period, 1),
                "{marker} {pin}: period {period} ps, want {want_period}"
            );
            let want_high = high(period, duty, 10);
            assert!(
                near(high_ps, want_high, TICK),
                "{marker} {pin}: high {high_ps} ps, want {want_high}"
            );
        }
    }
    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "expect no diagnostics");
}

/// `ledcChangeFrequency` changing resolution across active duties:
/// - 10 to 8 bits with duty 256 (>= 2^8): `DUTY_R` stays 256 and GPIO4 holds steady high with no
///   falling edges (TRM v1.8 35.3.3).
/// - 10 to 8 bits with duty 200: 200/256 at 2000 Hz.
/// - 8 to 13 bits with duty 64: 64/8192 at 400 Hz.
#[test]
fn a_stock_ledc_resolution_change_holds_high_at_or_above_steps_and_rescales_below() {
    let (project, mut live) = open("esp32s3-stock-ledcres");
    let path = project.path();

    // p1: 1000 Hz, 10 bits, duty 256 -> 1 ms period, 250 us high.
    run(&mut live, path, r#"step --until serial contains "p1""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, MS, 1), "p1 period {period} ps");
    assert!(
        near(high_ps, high(MS, 256, 10), TICK),
        "p1 high {high_ps} ps"
    );

    // p2: ledcChangeFrequency(4, 2000, 8) leaves duty 256 >= 2^8 -> steady high, no fall.
    run(&mut live, path, r#"step --until serial contains "p2 256""#);
    run(&mut live, path, "expect @board.GPIO4 high");
    assert_eq!(
        try_edge_on(&mut live, path, "GPIO4", "low", 15),
        None,
        "p2 expected steady high with no falling edge for 15 ms"
    );
    run(&mut live, path, "expect @board.GPIO4 high");

    // p3: back to 1000 Hz, 10 bits, duty 200 -> 1 ms period, 200/1024 high.
    run(&mut live, path, r#"step --until serial contains "p3""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, MS, 1), "p3 period {period} ps");
    assert!(
        near(high_ps, high(MS, 200, 10), TICK),
        "p3 high {high_ps} ps"
    );

    // p4: 10 to 8 bits with duty 200 -> 2000 Hz (500 us period), 200/256 high.
    run(&mut live, path, r#"step --until serial contains "p4""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 500 * US, 1), "p4 period {period} ps");
    assert!(
        near(high_ps, high(500 * US, 200, 8), TICK),
        "p4 high {high_ps} ps"
    );

    // p5: 8 to 13 bits with duty 64 -> 400 Hz (2.5 ms period), 64/8192 high.
    run(&mut live, path, r#"step --until serial contains "p5""#);
    let (period, high_ps) = cycle_on(&mut live, path, "GPIO4");
    assert!(near(period, 2500 * US, 1), "p5 period {period} ps");
    assert!(
        near(high_ps, high(2500 * US, 64, 13), TICK),
        "p5 high {high_ps} ps"
    );

    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "expect no diagnostics");
}

/// Returns the picosecond `pin` reaches `level` within `timeout_ms`, or `None` on timeout.
fn try_edge_on(
    live: &mut Live,
    project: &Path,
    pin: &str,
    level: &str,
    timeout_ms: u32,
) -> Option<u64> {
    let line = format!("step --until @board.{pin} {level} --timeout {timeout_ms}ms");
    let command = Command::parse(&line, project).unwrap().unwrap();
    let reply = live.run(&command);
    if reply.ok() {
        let Data::Stepped(stepped) = reply.data else {
            panic!("{line} did not step");
        };
        match stepped.met {
            Some(Met::Edge(ps) | Met::At(ps)) => Some(ps),
            met => panic!("{line}: {met:?}"),
        }
    } else {
        assert!(
            reply.text().contains("did not hold within"),
            "{line}: unexpected failure: {}",
            reply.text()
        );
        None
    }
}

/// Collects every `(rise_ps, fall_ps)` pulse on `pin`, waiting up to `start_ms` for the first rise
/// and stopping once `pin` stays low for `quiet_ms`.
fn collect_pulses(
    live: &mut Live,
    project: &Path,
    pin: &str,
    start_ms: u32,
    quiet_ms: u32,
) -> Vec<(u64, u64)> {
    let mut pulses = Vec::new();
    let mut wait = start_ms;
    while let Some(rise) = try_edge_on(live, project, pin, "high", wait) {
        let fall = try_edge_on(live, project, pin, "low", 5)
            .unwrap_or_else(|| panic!("pulse starting at {rise} ps did not fall"));
        pulses.push((rise, fall));
        wait = quiet_ms;
    }
    pulses
}

/// 1 ms `tone()` notes at 4000 Hz (divider 2500, 250 us), 8000 Hz (divider 1250, 125 us) and
/// 4321 Hz (divider 2314, 231.4 us) back to back and spaced by `delay(2)`: every full period
/// inside each note matches the timer divider's exact period and 511/1024 duty.
#[test]
fn a_stock_1ms_tone_notes_hold_exact_divider_periods() {
    let (project, mut live) = open("esp32s3-stock-tonefast");
    let path = project.path();

    // Back-to-back 1 ms burst: tone(4, 4000, 1); tone(4, 8000, 1); tone(4, 4321, 1);
    run(&mut live, path, r#"step --until serial contains "burst""#);
    let burst = collect_pulses(&mut live, path, "GPIO4", 5, 3);
    assert!(burst.len() >= 12, "burst pulses: {burst:?}");

    let p4000 = 250 * US;
    let p8000 = 125 * US;
    let p4321 = 231_400_000u64; // 40 MHz / (2314 / 256 * 1024) = 4321.521 Hz
    let mut c4000 = 0usize;
    let mut c8000 = 0usize;
    let mut c4321 = 0usize;
    let mut transitions = 0usize;
    for pair in burst.windows(2) {
        let (r0, f0) = pair[0];
        let (r1, _) = pair[1];
        let period = r1 - r0;
        let high_ps = f0 - r0;
        if near(period, p4000, 1) {
            assert_eq!(c8000 + c4321, 0, "4000 Hz after later note");
            assert!(
                near(high_ps, high(p4000, 0x1ff, 10), TICK),
                "4000 Hz high {high_ps}"
            );
            c4000 += 1;
        } else if near(period, p8000, 1) {
            assert_eq!(c4321, 0, "8000 Hz after 4321 Hz");
            assert!(
                near(high_ps, high(p8000, 0x1ff, 10), TICK),
                "8000 Hz high {high_ps}"
            );
            c8000 += 1;
        } else if near(period, p4321, 1) {
            assert!(
                near(high_ps, high(p4321, 0x1ff, 10), TICK),
                "4321 Hz high {high_ps}"
            );
            c4321 += 1;
        } else {
            // Only boundary pulses at the two note switches may differ from the exact divider period
            // (a merged high or the 37.5 ns RST pulse width from ledc_ll_timer_rst).
            transitions += 1;
            assert!(
                high_ps >= high(p8000, 0x1ff, 10) - TICK,
                "unexpected runt high {high_ps} ps in transition {pair:?}"
            );
        }
    }
    assert!(c4000 >= 2, "4000 Hz full periods: {c4000}");
    assert!(c8000 >= 5, "8000 Hz full periods: {c8000}");
    assert!(c4321 >= 3, "4321 Hz full periods: {c4321}");
    assert!(
        transitions <= 3,
        "at most 3 boundary pulses across 2 note switches, got {transitions}"
    );

    // Spaced 1 ms notes with delay(2) between them: every intra-note period matches its divider.
    run(&mut live, path, r#"step --until serial contains "spaced""#);
    let spaced = collect_pulses(&mut live, path, "GPIO4", 5, 3);
    let mut s4000 = 0usize;
    let mut s8000 = 0usize;
    let mut s4321 = 0usize;
    for pair in spaced.windows(2) {
        let (r0, f0) = pair[0];
        let (r1, _) = pair[1];
        let period = r1 - r0;
        if period > MS {
            continue; // silent gap between spaced notes
        }
        let high_ps = f0 - r0;
        if near(period, p4000, 1) {
            assert_eq!(s8000 + s4321, 0, "spaced 4000 Hz after later note");
            assert!(
                near(high_ps, high(p4000, 0x1ff, 10), TICK),
                "spaced 4000 Hz high {high_ps}"
            );
            s4000 += 1;
        } else if near(period, p8000, 1) {
            assert_eq!(s4321, 0, "spaced 8000 Hz after 4321 Hz");
            assert!(
                near(high_ps, high(p8000, 0x1ff, 10), TICK),
                "spaced 8000 Hz high {high_ps}"
            );
            s8000 += 1;
        } else if near(period, p4321, 1) {
            assert!(
                near(high_ps, high(p4321, 0x1ff, 10), TICK),
                "spaced 4321 Hz high {high_ps}"
            );
            s4321 += 1;
        } else {
            panic!("spaced intra-note period {period} ps matches no divider: {pair:?}");
        }
    }
    assert!(s4000 >= 2, "spaced 4000 Hz full periods: {s4000}");
    assert!(s8000 >= 2, "spaced 8000 Hz full periods: {s8000}");
    assert!(s4321 >= 1, "spaced 4321 Hz full periods: {s4321}");

    run(&mut live, path, r#"step --until serial contains "done""#);
    run(&mut live, path, "expect @board.GPIO4 low");
    run(&mut live, path, "expect no diagnostics");
}

/// Scanner, 1-byte write, 200-byte write, 6-byte read and 40-byte read to an absent address 0x3C
/// at 100 kHz and 400 kHz on a bus with pull-ups only. On stock Arduino-ESP32 3.3.12, the 0-byte
/// probe calls `i2c_master_probe` (returning `ESP_ERR_NOT_FOUND` -> 2), non-zero writes call
/// `i2c_master_transmit` (returning `ESP_ERR_INVALID_STATE` -> 4), and `requestFrom` returns 0.
#[test]
fn a_stock_wire_nack_sketch_returns_expected_codes_on_an_empty_bus() {
    play(
        "esp32s3-stock-wirenack",
        &[
            r#"step --until serial contains "done" --timeout 1s"#,
            r#"expect serial contains "k100 scan 2 w1 4 w200 4 r6 0 r40 0""#,
            r#"expect serial contains "k400 scan 2 w1 4 w200 4 r6 0 r40 0""#,
            "expect no diagnostics",
        ],
    );
}

/// With an OLED wired at 0x3C, probing, writing (1 byte and 200 bytes) and reading (6 and 40
/// bytes) absent address 0x3D at 100 kHz and 400 kHz returns the expected NACK codes and leaves
/// I2C0 ready to scan, initialize and draw to the real OLED at 0x3C.
#[test]
fn a_stock_wire_nack_with_a_device_at_another_address_still_talks_to_the_oled() {
    play(
        "esp32s3-stock-wirenackother",
        &[
            r#"step --until serial contains "oled scan 0 init 0 wrote 0" --timeout 1s"#,
            r#"expect serial contains "k100 scan 2 w1 4 w200 4 r6 0 r40 0""#,
            r#"expect serial contains "k400 scan 2 w1 4 w200 4 r6 0 r40 0""#,
            "expect @oled.display.screen shows wirenackother.screen",
            "expect no diagnostics",
        ],
    );
}

/// Stock Adafruit_SSD1306 `display.begin(SSD1306_SWITCHCAPVCC, 0x3C)` and `display.display()` on
/// a bus with pull-ups only (no OLED). `Adafruit_SSD1306::begin()` only returns false on malloc
/// failure, and `display.display()` sends 128-byte writes at 400 kHz that must finish with NACKs
/// and no diagnostics instead of faulting with `I2C0 busy_write`.
#[test]
fn a_stock_adafruit_ssd1306_sketch_with_no_oled_finishes_with_no_diagnostics() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "board": { "type": "esp32s3-devkitc-1", "app": "app.bin", "partitions": "partitions.bin" },
        "r_sda": { "type": "resistor-axial", "resistance": "4.7k" },
        "r_scl": { "type": "resistor-axial", "resistance": "4.7k" }
      },
      "wires": [
        ["board.3V3", "r_sda.a"],
        ["r_sda.b", "board.GPIO8"],
        ["board.3V3", "r_scl.a"],
        ["r_scl.b", "board.GPIO9"]
      ],
      "ground": "board.GND"
    }"#;
    let (project, mut live) = open_scene("esp32s3-stock-oled", scene);
    let path = project.path();
    run(
        &mut live,
        path,
        r#"step --until serial contains "drawn" --timeout 1s"#,
    );
    run(&mut live, path, "expect no diagnostics");
}

/// Stock sketch built with `CDCOnBoot=cdc` on Arduino-ESP32 3.3.12 prints over USB Serial/JTAG
/// CDC and lights the LED with no diagnostics.
#[test]
fn a_stock_cdc_sketch_prints_and_lights_the_led_on_core_3_3_12() {
    play(
        "esp32s3-stock-cdc",
        &[
            "step 100ms",
            "expect @led1 lit",
            r#"expect serial contains "cdc-ready""#,
            "expect no diagnostics",
        ],
    );
}

/// Same stock sketch built with `CDCOnBoot=cdc` on Arduino-ESP32 4.0.0, whose `HWCDC::begin()`
/// also enables `IN_TOKEN_REC_IN_EP1` in `INT_ENA`.
#[test]
fn a_stock_cdc_sketch_prints_and_lights_the_led_on_core_4_0_0() {
    play(
        "esp32s3-stock-cdc4",
        &[
            "step 100ms",
            "expect @led1 lit",
            r#"expect serial contains "cdc-ready""#,
            "expect no diagnostics",
        ],
    );
}

/// `Serial.read()` over USB Serial/JTAG CDC reads bytes delivered by the netlist's `console`
/// input on both Arduino-ESP32 3.3.12 and 4.0.0, with no diagnostics.
#[test]
fn a_stock_cdc_sketch_reads_console_bytes_on_both_cores() {
    for fixture in ["esp32s3-stock-cdc", "esp32s3-stock-cdc4"] {
        let dir = project(fixture);
        let netlist = orbe_scene::read(&Files::disk(), &dir.path().join("scene.jsonc"))
            .unwrap_or_else(|e| panic!("{fixture}: scene read failed: {e:?}"));
        let mut doc: serde_json::Value = serde_json::from_str(&netlist.json).unwrap();
        let parts = doc["parts"].as_array_mut().unwrap();
        let mcu = parts
            .iter_mut()
            .find(|p| p["kind"] == "esp32s3")
            .expect("scene has an esp32s3 part");
        mcu["console"] = serde_json::json!([
            { "t_ps": 30_000_000_000u64, "bytes": [103] }
        ]);
        let json = serde_json::to_vec(&doc).unwrap();
        let mut session = orbe_engine::Session::open_json(&json, &netlist.blobs)
            .unwrap_or_else(|e| panic!("{fixture}: open_json failed: {e:?}"));
        let status = session.advance_to(orbe_model::Time::from_ps(100_000_000_000));
        assert_eq!(
            status,
            orbe_engine::Status::Running,
            "{fixture}: expected Running, got {status:?}"
        );
        let bytes: Vec<u8> = session
            .serial_since(orbe_engine::SerialCursor::default())
            .map(|(slice, _)| slice.iter().map(|b| b.byte).collect())
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("cdc-ready"),
            "{fixture}: missing cdc-ready in {text:?}"
        );
        assert!(
            text.contains("got 103"),
            "{fixture}: missing echoed console byte in {text:?}"
        );
        let diagnostics = session.diagnostics();
        assert!(
            diagnostics.is_empty(),
            "{fixture}: unexpected diagnostics: {diagnostics:?}"
        );
    }
}

/// GPIO20 (USB D+) reads high from 0 ps under the USB PHY's 2.4 kohm D+ pull-up on both
/// Arduino-ESP32 3.3.12 and 4.0.0 with CDC on or off, and holds an external 10 kohm pull-down
/// resistor high (~2.66 V).
#[test]
fn gpio20_is_pulled_up_by_usb_phy_on_both_cores_and_drives_external_load() {
    for fixture in [
        "esp32s3-stock-cdc",
        "esp32s3-stock-cdc4",
        "esp32s3-stock-blink",
    ] {
        play(
            fixture,
            &[
                "expect @board.GPIO20 high",
                "step 50ms",
                "expect @board.GPIO20 high",
                "expect no diagnostics",
            ],
        );
    }

    let load_scene = r#"{
  "format": 1,
  "parts": {
    "board": {
      "type": "esp32s3-devkitc-1",
      "app": "app.bin",
      "partitions": "partitions.bin"
    },
    "r_dp": { "type": "resistor-axial", "resistance": "10k" }
  },
  "wires": [
    ["board.GPIO20", "r_dp.a"],
    ["r_dp.b", "board.GND"]
  ],
  "ground": "board.GND"
}"#;
    for fixture in ["esp32s3-stock-cdc", "esp32s3-stock-cdc4"] {
        let (dir, mut live) = open_scene(fixture, load_scene);
        run(&mut live, dir.path(), "expect @board.GPIO20 high");
        run(&mut live, dir.path(), "step 50ms");
        run(&mut live, dir.path(), "expect @board.GPIO20 high");
        let reply = live.run(
            &Command::parse("get @board.GPIO20", dir.path())
                .unwrap()
                .unwrap(),
        );
        assert!(reply.ok(), "{fixture}: {}", reply.text());
        assert!(
            reply.text().contains("high"),
            "{fixture}: expected high into 10k load, got {}",
            reply.text()
        );
        run(&mut live, dir.path(), "expect no diagnostics");
    }
}

/// Copies a Block from `blocks/<name>` beside the catalog parts in a temporary repo layout so
/// scene tests can edit `scene.jsonc` without touching the repo.
fn block_project(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = repo();
    let block_dir = dir.path().join("blocks").join(name);
    copy_dir(&root.join("blocks").join(name), &block_dir);
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
    (dir, block_dir)
}

/// Removing the 3V3-to-VCC wire from `blocks/devkitc-gy-bmp280` leaves the module's pull-ups
/// dragging SCL/SDA toward the dead rail. I2C0 raises `TIME_OUT` after `2^I2C_TIME_OUT_VALUE`
/// cycles, Wire returns an error, `bmp.begin()` fails, and the stock Adafruit_BMP280 sketch prints
/// its own missing-sensor message instead of faulting the simulator.
#[test]
fn an_unpowered_gy_bmp280_on_devkitc_times_out_and_prints_its_error() {
    let (_dir, block_dir) = block_project("devkitc-gy-bmp280");
    let scene_path = block_dir.join("scene.jsonc");
    let scene = fs::read_to_string(&scene_path)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("3V3 to VCC"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&scene_path, scene).unwrap();

    let mut live = Live::new("t", Files::disk());
    run(&mut live, &block_dir, "open");
    run(
        &mut live,
        &block_dir,
        r#"step --until serial contains "SensorID was:" --timeout 200ms"#,
    );
    run(&mut live, &block_dir, "step 20ms");
    run(
        &mut live,
        &block_dir,
        r#"expect serial contains "BMP280 test""#,
    );
    run(
        &mut live,
        &block_dir,
        r#"expect serial contains "Could not find a valid BMP280 sensor, check wiring or try a different address!""#,
    );
    run(
        &mut live,
        &block_dir,
        r#"expect serial contains "SensorID was: 0x0""#,
    );
    let snap = live
        .run(&Command::parse("snapshot", &block_dir).unwrap().unwrap())
        .text();
    let errors: Vec<_> = snap
        .lines()
        .filter(|line| line.starts_with("error["))
        .collect();
    assert!(errors.is_empty(), "unexpected error diagnostics:\n{snap}");
}

/// A part holding SCL low longer than the configured I2C timeout (a closed switch from SCL to GND
/// beside a powered GY-BMP280) triggers `TIME_OUT`, fails `bmp.begin()` cleanly with the sketch's
/// error text, and leaves SCL idle high once released with no diagnostics.
#[test]
fn a_part_holding_scl_low_past_timeout_fails_bmp280_begin_without_faulting() {
    let (_dir, block_dir) = block_project("devkitc-gy-bmp280");
    let scene_path = block_dir.join("scene.jsonc");
    let scene = fs::read_to_string(&scene_path)
        .unwrap()
        .replace(
            r#""mod": { "type": "gy-bmp280" },"#,
            r#""mod": { "type": "gy-bmp280" },
    "sw": { "type": "tactile-switch-6mm" },"#,
        )
        .replace(
            r#"["bb.j11", "bb.j27"], // GPIO8 (SDA) to SDA"#,
            r#"["bb.j11", "bb.j27"], // GPIO8 (SDA) to SDA
    ["bb.i28", "sw.a"],
    ["sw.b", "bb.i1"],"#,
        );
    fs::write(&scene_path, scene).unwrap();

    let mut live = Live::new("t", Files::disk());
    run(&mut live, &block_dir, "open");
    run(&mut live, &block_dir, "press @sw");
    run(&mut live, &block_dir, "step 50ms");
    run(&mut live, &block_dir, "release @sw");
    run(
        &mut live,
        &block_dir,
        r#"step --until serial contains "SensorID was:" --timeout 200ms"#,
    );
    run(&mut live, &block_dir, "step 20ms");
    run(
        &mut live,
        &block_dir,
        r#"expect serial contains "BMP280 test""#,
    );
    run(
        &mut live,
        &block_dir,
        r#"expect serial contains "Could not find a valid BMP280 sensor, check wiring or try a different address!""#,
    );
    run(
        &mut live,
        &block_dir,
        r#"expect serial contains "SensorID was: 0x0""#,
    );
    run(&mut live, &block_dir, "expect @board.GPIO9 high");
    run(&mut live, &block_dir, "expect no diagnostics");
}

/// Stock `Preferences` counter and typed keys write to NVS flash, read back across close/reopen,
/// and keep incrementing in flash across button presses.
#[test]
fn a_stock_preferences_sketch_writes_and_reads_keys_and_increments_on_press() {
    play(
        "esp32s3-stock-preferences",
        &[
            r#"step --until serial contains "prefs count=1 signed=-42 name=orbe raw=12345678 len=4" --timeout 1s"#,
            "press @btn1",
            r#"step --until serial contains "press count=2" --timeout 1s"#,
            "release @btn1",
            "step 30ms",
            "press @btn1",
            r#"step --until serial contains "press count=3" --timeout 1s"#,
            "release @btn1",
            "expect no diagnostics",
        ],
    );
}

/// Stock `EEPROM.begin/write/commit/read` persists bytes across `end()`/`begin()` and overwrites.
#[test]
fn a_stock_eeprom_sketch_commits_reads_and_overwrites_bytes() {
    play(
        "esp32s3-stock-eeprom",
        &[
            r#"step --until serial contains "eeprom ok=111 commit=11 first=a5,5a,11,22 second=3c,5a,11,22" --timeout 1s"#,
            "expect no diagnostics",
        ],
    );
}

/// Stock `nvs_flash_init` and `nvs_flash_erase` initialize NVS, erase all NVS sectors, confirm
/// erased keys are gone, and read back newly written values with no error.
#[test]
fn a_stock_nvs_sketch_initializes_erases_and_reads_back_keys() {
    play(
        "esp32s3-stock-nvs",
        &[
            r#"step --until serial contains "nvs ok=1 count=99 tag=after" --timeout 2s"#,
            "expect no diagnostics",
        ],
    );
}

/// Stock `LittleFS` formats blank flash, writes and appends a file, unmounts, remounts without
/// reformatting, and reads back the saved bytes.
#[test]
fn a_stock_littlefs_sketch_formats_writes_appends_remounts_and_reads_back() {
    play(
        "esp32s3-stock-littlefs",
        &[
            r#"step --until serial contains "littlefs mount=11 wrote=11 text=hello flash" --timeout 2s"#,
            "expect no diagnostics",
        ],
    );
}

/// Written flash survives the board's supply dropping to 0 V and returning inside a session.
/// Note that the `esp32s3` MCU model does not power-cycle or reboot on a supply drop (nor does
/// `CHIP_PU`/`RST` reset it); this verifies the flash contents remain intact across a supply dip
/// and subsequent button-driven read-modify-write.
#[test]
fn written_flash_survives_a_supply_cycle_in_a_session() {
    let dir = project("esp32s3-stock-preferences");
    let netlist = orbe_scene::read(&Files::disk(), &dir.path().join("scene.jsonc")).unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(&netlist.json).unwrap();
    let parts = doc["parts"].as_array_mut().unwrap();
    let usb = parts
        .iter_mut()
        .find(|p| p["name"] == "board__j2__host")
        .expect("devkitc has board__j2__host voltage source");
    usb["voltage"] = serde_json::json!({
        "kind": "steps",
        "initial": { "value": 5.0, "unit": "V" },
        "events": [
            { "t_ps": 60_000_000_000u64, "level": { "value": 0.0, "unit": "V" } },
            { "t_ps": 80_000_000_000u64, "level": { "value": 5.0, "unit": "V" } }
        ]
    });
    let btn = parts
        .iter_mut()
        .find(|p| {
            p["kind"] == "switch" && p["name"].as_str().is_some_and(|s| s.starts_with("btn1"))
        })
        .expect("scene has btn1 switch");
    btn["events"] = serde_json::json!([
        { "t_ps": 110_000_000_000u64, "state": "closed" },
        { "t_ps": 140_000_000_000u64, "state": "open" }
    ]);
    let json = serde_json::to_vec(&doc).unwrap();
    let mut session = orbe_engine::Session::open_json(&json, &netlist.blobs).unwrap();
    assert_eq!(
        session.advance_to(orbe_model::Time::from_ps(180_000_000_000)),
        orbe_engine::Status::Running
    );
    let bytes: Vec<u8> = session
        .serial_since(orbe_engine::SerialCursor::default())
        .map(|(slice, _)| slice.iter().map(|b| b.byte).collect())
        .unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("prefs count=1 signed=-42 name=orbe raw=12345678 len=4"),
        "missing initial prefs write before supply cycle: {text:?}"
    );
    assert!(
        text.contains("press count=2"),
        "missing incremented count after supply cycle: {text:?}"
    );
    assert!(
        session.fault().is_none(),
        "unexpected fault: {:?}",
        session.fault()
    );
}

#[test]
fn a_stock_pcnt_sketch_counts_quadrature_filters_bounce_and_fires_limit_interrupts() {
    let (dir, mut live) = open("esp32s3-stock-pcnt");
    let path = dir.path();
    run(
        &mut live,
        path,
        r#"step --until serial contains "ready" --timeout 300ms"#,
    );
    // Forward quadrature cycle (+4).
    for line in [
        "press @enc_a",
        "step 2ms",
        "press @enc_b",
        "step 2ms",
        "release @enc_a",
        "step 2ms",
        "release @enc_b",
        "step 5ms",
        r#"expect serial contains "count 4""#,
    ] {
        run(&mut live, path, line);
    }
    // 2 us bounce is shorter than the 5 us glitch filter and must not advance the count.
    for line in ["press @enc_a", "step 2us", "release @enc_a", "step 5ms"] {
        run(&mut live, path, line);
    }
    let snap = live
        .run(&Command::parse("snapshot", path).unwrap().unwrap())
        .text();
    assert!(
        !snap.contains("count 5"),
        "short bounce was counted:\n{snap}"
    );
    // Second forward cycle hits high_limit (+8), resets to 0 and fires the watchpoint callback.
    for line in [
        "press @enc_a",
        "step 2ms",
        "press @enc_b",
        "step 2ms",
        "release @enc_a",
        "step 2ms",
        "release @enc_b",
        "step 5ms",
        r#"expect serial contains "limit 8""#,
    ] {
        run(&mut live, path, line);
    }
    // Reverse quadrature cycle (-4) and second reverse cycle to low_limit (-8).
    for line in [
        "press @enc_b",
        "step 2ms",
        "press @enc_a",
        "step 2ms",
        "release @enc_b",
        "step 2ms",
        "release @enc_a",
        "step 5ms",
        r#"expect serial contains "count -4""#,
        "press @enc_b",
        "step 2ms",
        "press @enc_a",
        "step 2ms",
        "release @enc_b",
        "step 2ms",
        "release @enc_a",
        "step 5ms",
        r#"expect serial contains "limit -8""#,
        "expect no diagnostics",
    ] {
        run(&mut live, path, line);
    }
}

#[test]
fn a_stock_temp_sketch_reads_ambient_and_default_temperature_across_ranges() {
    play(
        "esp32s3-stock-temp",
        &[
            r#"step --until serial contains "temp 25.0" --timeout 300ms"#,
            "set temperature 42",
            r#"step --until serial contains "temp 42.0" --timeout 200ms"#,
            "set temperature 95",
            r#"step --until serial contains "temp 95.0" --timeout 200ms"#,
            "set temperature -25",
            r#"step --until serial contains "temp -25.0" --timeout 200ms"#,
            "expect no diagnostics",
        ],
    );

    // Without a world block in scene.jsonc, ambient temperature defaults to 25 degC.
    let dir = project("esp32s3-stock-temp");
    let scene_path = dir.path().join("scene.jsonc");
    let no_world = fs::read_to_string(&scene_path)
        .unwrap()
        .replace(",\n  \"world\": { \"temperature\": 25 }", "");
    fs::write(&scene_path, no_world).unwrap();
    let mut live = Live::new("t", Files::disk());
    run(&mut live, dir.path(), "open");
    run(
        &mut live,
        dir.path(),
        r#"step --until serial contains "temp 25.0" --timeout 300ms"#,
    );
    run(&mut live, dir.path(), "expect no diagnostics");
}

#[test]
fn a_stock_crypto_sketch_verifies_sha_aes_and_rsa_vectors() {
    play(
        "esp32s3-stock-crypto",
        &[
            r#"step --until serial contains "all crypto ok" --timeout 500ms"#,
            r#"expect serial contains "sha1 ok""#,
            r#"expect serial contains "sha256 ok""#,
            r#"expect serial contains "sha384 ok""#,
            r#"expect serial contains "sha512 ok""#,
            r#"expect serial contains "sha512_dma ok""#,
            r#"expect serial contains "aes128_ecb ok""#,
            r#"expect serial contains "aes256_ecb ok""#,
            r#"expect serial contains "aes128_cbc ok""#,
            r#"expect serial contains "aes256_cbc ok""#,
            r#"expect serial contains "aes256_cbc_irq ok""#,
            r#"expect serial contains "aes128_ctr ok""#,
            r#"expect serial contains "aes256_ctr ok""#,
            r#"expect serial contains "aes128_gcm ok""#,
            r#"expect serial contains "rsa2048_pk ok""#,
            "expect no diagnostics",
        ],
    );
}
