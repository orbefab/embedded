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

/// The picosecond GPIO18 is at `level`: now, when it already is, or at its next edge.
fn edge(live: &mut Live, project: &Path, level: &str) -> u64 {
    let line = format!("step --until @board.GPIO18 {level} --timeout 30ms");
    let Data::Stepped(stepped) = run(live, project, &line) else {
        panic!("{line} did not step");
    };
    match stepped.met {
        Some(Met::Edge(ps) | Met::At(ps)) => ps,
        met => panic!("{line}: {met:?}"),
    }
}

/// GPIO18's next whole PWM cycle, from a rising edge: its period and its high time, in ps.
fn cycle(live: &mut Live, project: &Path) -> (u64, u64) {
    edge(live, project, "low");
    let rise = edge(live, project, "high");
    let fall = edge(live, project, "low");
    let next = edge(live, project, "high");
    (next - rise, fall - rise)
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
