//! A passive buzzer reads the frequency of the drive across its coil, and 0 when that
//! drive has stopped. The pins are `+` and `-`. A peak outside 3 V to 5 V warns and still sounds.

use std::path::{Path, PathBuf};

use orbe_session::{Command, Files, Live};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalog() -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}/"), out);
            } else {
                out.push((format!("{prefix}{name}"), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo().join("parts"), "", &mut files);
    files
}

fn opened(hex: &str, parts: &str, wires: &str) -> Live {
    let image =
        std::fs::read(repo().join(format!("packages/engine/tests/firmware/sketch/{hex}.hex")))
            .unwrap();
    let block = std::fs::read(repo().join("blocks/nano-buzzer/block.jsonc")).unwrap();
    let scene = format!(
        r#"{{
  "format": 1,
  "parts": {{
    "nano": {{ "type": "nano-board", "firmware": "firmware/{hex}.hex" }},
    {parts}
  }},
  "wires": [{wires}]
}}
"#
    );
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        (format!("firmware/{hex}.hex"), image),
        ("scene.jsonc".to_string(), scene.into_bytes()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let open = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(open.ok(), "{}", open.text());
    live
}

fn tone_scene(wires: &str) -> Live {
    opened(
        "tone",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },
    "bz": { "type": "buzzer-12mm" }"#,
        wires,
    )
}

fn drive_scene(wires: &str) -> Live {
    opened("drive", r#""bz": { "type": "buzzer-12mm" }"#, wires)
}

const DIVIDER: &str = r#"["nano.D8", "r1.a"], ["r1.b", "bz.+"], ["bz.-", "nano.GND"]"#;
const DRIVE: &str = r#"["nano.D8", "bz.+"], ["bz.-", "nano.GND"]"#;

fn run(live: &mut Live, line: &str) -> orbe_session::Reply {
    live.run(&Command::parse(line, Path::new(".")).unwrap().unwrap())
}

fn hz(live: &mut Live) -> f64 {
    let reply = run(live, "get @bz.hz");
    assert!(reply.ok(), "{}", reply.text());
    reply.json()["data"]["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{}", reply.text()))
}

fn volts(live: &mut Live) -> f64 {
    let reply = run(live, "get @bz.v");
    assert!(reply.ok(), "{}", reply.text());
    reply.json()["data"]["value"].as_f64().unwrap().abs()
}

fn diagnostics(live: &mut Live) -> Vec<(String, String)> {
    let json = run(live, "snapshot").json();
    json["data"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("{json}"))
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_string(),
                d["message"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn phase(live: &mut Live, name: &str) {
    let reply = run(
        live,
        &format!(r#"step --until serial contains "{name}" --timeout 5s"#),
    );
    assert!(reply.ok(), "{name}: {}", reply.text());
}

fn near(got: f64, want: f64, tol: f64) -> bool {
    (got - want).abs() <= want.abs() * tol
}

fn assert_near(got: f64, want: f64, tol: f64) {
    assert!(near(got, want, tol), "{got} is not within {tol} of {want}");
}

/// Samples `n` times, stepping `step` before each read. The first read is after one step.
fn sweep(live: &mut Live, step: &str, n: usize) -> Vec<f64> {
    (0..n)
        .map(|_| {
            assert!(run(live, step).ok());
            hz(live)
        })
        .collect()
}

#[test]
fn a_series_resistor_plays_440_and_warns_that_the_coil_is_under_3_v() {
    let mut live = tone_scene(DIVIDER);
    assert!(run(&mut live, "step 100ms").ok());
    assert_near(hz(&mut live), 440.0, 0.001);
    let found = diagnostics(&mut live);
    assert!(
        !found.iter().any(|(code, _)| code == "rating.current"),
        "the 220 ohm keeps the pin inside its current rating: {found:?}"
    );
    let voltage: Vec<_> = found
        .iter()
        .filter(|(code, _)| code == "rating.voltage")
        .collect();
    assert_eq!(voltage.len(), 1, "drive under 3 V: {found:?}");
    assert!(
        voltage[0].1.contains("operating voltage"),
        "{}",
        voltage[0].1
    );
    let mut peak = 0.0_f64;
    for _ in 0..8 {
        assert!(run(&mut live, "step 1ms").ok());
        peak = peak.max(volts(&mut live));
    }
    assert!(peak < 3.0, "220 ohm coil peak {peak} V");
    assert!(run(&mut live, "step 600ms").ok());
    let quiet = hz(&mut live);
    assert!(quiet < 1.0, "rest read {quiet}");
}

#[test]
fn a_pin_wired_straight_across_the_coil_still_plays_and_warns() {
    let mut live = opened(
        "tone",
        r#""bz": { "type": "buzzer-12mm" }"#,
        r#"["nano.D8", "bz.+"], ["bz.-", "nano.GND"]"#,
    );
    assert!(run(&mut live, "step 100ms").ok());
    assert_near(hz(&mut live), 440.0, 0.001);
    let found = diagnostics(&mut live);
    assert!(
        found.iter().any(|(code, _)| code == "rating.current"),
        "expected rating.current, got {found:?}"
    );
}

#[test]
fn an_npn_from_5_v_plays_inside_3_v_to_5_v_with_no_rating_tripped() {
    let mut live = opened(
        "tone",
        r#""rs": { "type": "resistor-axial", "resistance": "10" },
          "rb": { "type": "resistor-axial", "resistance": "1k" },
          "q1": { "type": "npn-2n2222-to92" },
          "bz": { "type": "buzzer-12mm" }"#,
        r#"["nano.5V", "rs.a"], ["rs.b", "bz.+"], ["bz.-", "q1.collector"],
           ["q1.emitter", "nano.GND"], ["nano.D8", "rb.a"], ["rb.b", "q1.base"]"#,
    );
    assert!(run(&mut live, "step 100ms").ok());
    assert_near(hz(&mut live), 440.0, 0.001);
    let found = diagnostics(&mut live);
    assert!(
        found.iter().all(|(code, _)| !code.starts_with("rating")),
        "ratings tripped: {found:?}"
    );
    let mut peak = 0.0_f64;
    for _ in 0..12 {
        assert!(run(&mut live, "step 1ms").ok());
        peak = peak.max(volts(&mut live));
    }
    assert!(
        (3.0..5.0).contains(&peak),
        "NPN drive peak {peak} V, want 3 V to 5 V"
    );
}

#[test]
fn whole_kilohertz_tones_read_steadily() {
    let mut live = drive_scene(DRIVE);
    for (name, want) in [("khz", 1000.0), ("two", 2000.0), ("four", 4000.0)] {
        phase(&mut live, name);
        assert!(run(&mut live, "step 40ms").ok());
        let got = sweep(&mut live, "step 10ms", 4);
        for hz in &got {
            assert_near(*hz, want, 0.001);
        }
    }
}

#[test]
fn a_31_hz_tone_and_a_25_hz_click_train_read_steadily() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "low");
    assert!(run(&mut live, "step 70ms").ok());
    let low = sweep(&mut live, "step 20ms", 5);
    for hz in &low {
        assert_near(*hz, 31.0, 0.01);
    }
    phase(&mut live, "clicks");
    assert!(run(&mut live, "step 100ms").ok());
    let clicks = sweep(&mut live, "step 10ms", 8);
    for hz in &clicks {
        assert_near(*hz, 25.0, 0.05);
    }
}

#[test]
fn notes_at_440_880_and_2048_are_within_a_tenth_of_a_percent() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "notes");
    assert!(run(&mut live, "step 35ms").ok());
    assert_near(hz(&mut live), 440.0, 0.001);
    assert!(run(&mut live, "step 40ms").ok());
    assert_near(hz(&mut live), 880.0, 0.001);
    assert!(run(&mut live, "step 40ms").ok());
    assert_near(hz(&mut live), 2048.0, 0.001);
}

#[test]
fn a_rising_note_reads_its_pitch_within_one_period_and_not_a_blend() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "up");
    assert!(run(&mut live, "step 70ms").ok());
    let got = sweep(&mut live, "step 1ms", 50);
    let mut saw_880_after = None;
    let mut last_440 = None;
    for (i, hz) in got.iter().copied().enumerate() {
        let is_440 = near(hz, 440.0, 0.05);
        let is_880 = near(hz, 880.0, 0.05);
        assert!(hz < 1.0 || is_440 || is_880, "blend {hz} at sample {i}");
        if is_440 {
            last_440 = Some(i);
        }
        if is_880 && saw_880_after.is_none() {
            saw_880_after = Some(i);
        }
    }
    let (from, at) = (last_440.expect("440"), saw_880_after.expect("880"));
    assert!(at > from, "880 before 440: {got:?}");
    assert!(at - from <= 3, "880 lagged {} ms: {got:?}", at - from);
}

#[test]
fn a_falling_note_does_not_read_silence() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "down");
    assert!(run(&mut live, "step 80ms").ok());
    let got = sweep(&mut live, "step 1ms", 50);
    assert!(
        got.iter().all(|hz| *hz > 1.0),
        "silence in a melody: {got:?}"
    );
    assert!(
        got.iter().any(|hz| near(*hz, 440.0, 0.05)),
        "440 never arrived: {got:?}"
    );
    let reply = run(&mut live, "step --until @bz.hz '<' 1Hz --timeout 80ms");
    assert!(!reply.ok(), "until stopped on a note: {}", reply.text());
}

#[test]
fn a_melody_of_30_ms_notes_shows_each_note() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "melody");
    assert!(run(&mut live, "step 25ms").ok());
    let got = sweep(&mut live, "step 2ms", 45);
    for want in [440.0, 880.0, 660.0] {
        assert!(
            got.iter().any(|hz| near(*hz, want, 0.05)),
            "{want} missing from {got:?}"
        );
    }
}

#[test]
fn the_startup_pulse_of_tone_does_not_count_as_a_cycle() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "start");
    for delay in ["step 25ms", "step 15ms", "step 20ms"] {
        assert!(run(&mut live, delay).ok());
        assert_near(hz(&mut live), 600.0, 0.01);
    }
}

#[test]
fn narrow_pwm_reads_steadily() {
    let mut on_nine = drive_scene(r#"["nano.D9", "bz.+"], ["bz.-", "nano.GND"]"#);
    phase(&mut on_nine, "pwm");
    assert!(run(&mut on_nine, "step 30ms").ok());
    assert_near(hz(&mut on_nine), 490.2, 0.05);
    assert!(run(&mut on_nine, "step 25ms").ok());
    let narrow = sweep(&mut on_nine, "step 5ms", 6);
    for hz in &narrow {
        assert_near(*hz, 490.2, 0.05);
    }
    assert!(run(&mut on_nine, "step 20ms").ok());
    let wide = sweep(&mut on_nine, "step 5ms", 6);
    for hz in &wide {
        assert_near(*hz, 490.2, 0.05);
    }

    let mut on_five = drive_scene(r#"["nano.D5", "bz.+"], ["bz.-", "nano.GND"]"#);
    phase(&mut on_five, "pwm");
    assert!(run(&mut on_five, "step 170ms").ok());
    assert_near(hz(&mut on_five), 976.6, 0.01);
}

#[test]
fn a_short_tone_and_no_tone_read_0_at_once() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "brief");
    assert!(run(&mut live, "step 40ms").ok());
    assert_near(hz(&mut live), 440.0, 0.01);
    assert!(run(&mut live, "step 50ms").ok());
    let quiet = hz(&mut live);
    assert!(quiet < 1.0, "50 ms tone still read {quiet}");

    phase(&mut live, "off");
    assert!(run(&mut live, "step 40ms").ok());
    assert_near(hz(&mut live), 880.0, 0.01);
    assert!(run(&mut live, "step 40ms").ok());
    let stopped = hz(&mut live);
    assert!(stopped < 1.0, "noTone still read {stopped}");
}

#[test]
fn dc_unpowered_floating_and_a_buzzer_to_5_v_read_as_a_listener_would() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "dc");
    assert!(run(&mut live, "step 40ms").ok());
    let held = hz(&mut live);
    assert!(held < 1.0, "DC read {held}");

    let mut high_side = opened(
        "tone",
        r#""bz": { "type": "buzzer-12mm" }"#,
        r#"["nano.5V", "bz.+"], ["bz.-", "nano.D8"]"#,
    );
    assert!(run(&mut high_side, "step 100ms").ok());
    assert_near(hz(&mut high_side), 440.0, 0.001);

    let mut open = opened("tone", r#""bz": { "type": "buzzer-12mm" }"#, "");
    assert!(run(&mut open, "step 50ms").ok());
    assert!(hz(&mut open) < 1.0, "unpowered");

    let mut floating = opened(
        "tone",
        r#""bz": { "type": "buzzer-12mm" }"#,
        r#"["nano.D8", "bz.+"]"#,
    );
    assert!(run(&mut floating, "step 50ms").ok());
    assert!(hz(&mut floating) < 1.0, "floating");
}

#[test]
fn expect_on_hz_rejects_a_voltage() {
    let mut live = tone_scene(DIVIDER);
    let reply = run(&mut live, "expect @bz.hz > 4V");
    assert!(!reply.ok(), "{}", reply.text());
    assert!(reply.text().contains("Hz"), "{}", reply.text());
}

#[test]
fn the_old_pin_numbers_are_not_pins() {
    let image =
        std::fs::read(repo().join("packages/engine/tests/firmware/sketch/tone.hex")).unwrap();
    let block = std::fs::read(repo().join("blocks/nano-buzzer/block.jsonc")).unwrap();
    let scene = br#"{
      "format": 1,
      "parts": {
        "nano": { "type": "nano-board", "firmware": "firmware/tone.hex" },
        "bz": { "type": "buzzer-12mm" }
      },
      "wires": [["nano.D8", "bz.1"], ["bz.2", "nano.GND"]]
    }"#;
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        ("firmware/tone.hex".to_string(), image),
        ("scene.jsonc".to_string(), scene.to_vec()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let open = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(!open.ok(), "bz.1 still opened: {}", open.text());
}
