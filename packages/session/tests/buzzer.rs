//! A passive buzzer reads the frequency of the drive across its coil, and 0 when that
//! drive has stopped. The pins are `+` and `-`. A peak outside 3 V to 5 V warns and still sounds.

use std::collections::BTreeMap;
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
    // 708 ms in, `tone(8, 440, 500)` once a second has sounded for 500 ms.
    let over = drive_over_ps(&mut live);
    assert!(
        (490_000_000_000..=510_000_000_000).contains(&over),
        "500 ms of tone counted {over} ps"
    );
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
fn a_31_hz_tone_reads_steadily_and_a_25_hz_click_train_reads_0() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "low");
    assert!(run(&mut live, "step 70ms").ok());
    let low = sweep(&mut live, "step 20ms", 5);
    for hz in &low {
        assert_near(*hz, 31.0, 0.01);
    }
    // 40 ms between clicks is a rest, longer than any half-cycle of a note.
    phase(&mut live, "clicks");
    assert!(run(&mut live, "step 100ms").ok());
    let clicks = sweep(&mut live, "step 10ms", 8);
    assert!(clicks.iter().all(|hz| *hz == 0.0), "{clicks:?}");
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
    assert_eq!(only_notes(&got, &[440.1, 880.3]), [440.1, 880.3], "{got:?}");
    let last_440 = got.iter().rposition(|hz| near(*hz, 440.1, 0.005)).unwrap();
    let first_880 = got.iter().position(|hz| near(*hz, 880.3, 0.005)).unwrap();
    assert!(first_880 - last_440 <= 3, "880 lagged: {got:?}");
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
    assert_eq!(only_notes(&got, &[880.3, 440.1]), [880.3, 440.1], "{got:?}");
    let reply = run(&mut live, "step --until @bz.hz '<' 1Hz --timeout 80ms");
    assert!(!reply.ok(), "until stopped on a note: {}", reply.text());
}

#[test]
fn a_melody_of_30_ms_notes_shows_each_note() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "melody");
    settle(&mut live);
    let got = sweep(&mut live, "step 1ms", 110);
    let order = only_notes(&got, &[440.1, 880.3, 661.4]);
    assert_eq!(order, [440.1, 880.3, 661.4, 0.0], "{got:?}");
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
fn ten_kilohertz_reads_and_a_tone_above_hearing_or_analog_write_on_d8_reads_0() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "high");
    assert!(run(&mut live, "step 40ms").ok());
    assert_near(hz(&mut live), 10_000.0, 0.001);
    phase(&mut live, "ultra");
    settle(&mut live);
    let playing = run(&mut live, "step 5ms").json();
    let edges = playing["data"]["changes"].as_array().unwrap().len()
        + playing["data"]["more"].as_u64().unwrap() as usize;
    assert!(edges > 200, "D8 is not playing above 20 kHz: {playing}");
    let ultra = sweep(&mut live, "step 5ms", 8);
    assert!(ultra.iter().all(|hz| *hz == 0.0), "{ultra:?}");
    // D8 has no PWM, so `analogWrite(8, 128)` holds it high.
    phase(&mut live, "flat");
    settle(&mut live);
    let flat = sweep(&mut live, "step 5ms", 6);
    assert!(flat.iter().all(|hz| *hz == 0.0), "{flat:?}");
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

/// Steps past the rest that ends the phase before, to 2 ms before the phase's first note.
fn settle(live: &mut Live) {
    assert!(run(live, "step 18ms").ok());
}

/// Each 1 ms reading is 0 or within 0.5% of one of `notes`. The first cycle after `tone()`
/// starts is up to 0.5% short, 9 cents, as the timer was part way through a count. Returns
/// what read in order, each stretch once, with 0 for a rest and no 0 before the first note.
fn only_notes(got: &[f64], notes: &[f64]) -> Vec<f64> {
    let mut order: Vec<f64> = Vec::new();
    for (ms, hz) in got.iter().copied().enumerate() {
        let note = if hz == 0.0 {
            0.0
        } else {
            notes
                .iter()
                .copied()
                .find(|note| near(hz, *note, 0.005))
                .unwrap_or_else(|| panic!("{hz} Hz at {} ms is no note played: {got:?}", ms + 1))
        };
        if order.last() != Some(&note) && !(order.is_empty() && note == 0.0) {
            order.push(note);
        }
    }
    order
}

#[test]
fn a_rest_clears_the_note_before_it() {
    // 1760 Hz, 40 ms quiet, `tone(8, 440, 50)`, 40 ms quiet, `tone(8, 1000)`.
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "rest");
    settle(&mut live);
    let got = sweep(&mut live, "step 1ms", 225);
    let order = only_notes(&got, &[1760.6, 440.1, 1000.0]);
    assert_eq!(order, [1760.6, 0.0, 440.1, 0.0, 1000.0], "{got:?}");
}

#[test]
fn a_fall_of_more_than_an_octave_holds_the_note_until_the_next_one_reads() {
    // 1760 to 220, 880 to 110, 1000 to 31, with no rest between.
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "fall");
    assert!(run(&mut live, "step 25ms").ok());
    let got = sweep(&mut live, "step 1ms", 360);
    let order = only_notes(&got, &[1760.6, 220.1, 880.3, 110.0, 1000.0, 31.0]);
    assert_eq!(
        order,
        [1760.6, 220.1, 880.3, 110.0, 1000.0, 31.0],
        "{got:?}"
    );
    assert!(got.iter().all(|hz| *hz > 0.0), "a note read 0: {got:?}");
}

#[test]
fn a_new_note_reads_its_own_pitch_and_no_blend() {
    // 30 ms notes at 440, 494 and 523, then 31 to 1000 and 110 to 220 with no rest.
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "scale");
    settle(&mut live);
    let got = sweep(&mut live, "step 1ms", 100);
    let order = only_notes(&got, &[440.1, 494.1, 523.0]);
    assert_eq!(order, [440.1, 494.1, 523.0], "{got:?}");
    let right = got.iter().filter(|hz| near(**hz, 494.1, 0.002)).count();
    assert!(right >= 24, "494 read for {right} of 30 ms: {got:?}");

    phase(&mut live, "climb");
    settle(&mut live);
    let got = sweep(&mut live, "step 1ms", 225);
    let order = only_notes(&got, &[31.0, 1000.0, 110.0, 220.1]);
    assert_eq!(order, [31.0, 1000.0, 110.0, 220.1], "{got:?}");
}

#[test]
fn the_first_note_from_silence_reads_its_own_pitch() {
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "low");
    settle(&mut live);
    let got = sweep(&mut live, "step 1ms", 150);
    assert_eq!(only_notes(&got, &[31.0]), [31.0], "{got:?}");

    let mut live = opened(
        "tone",
        r#""rs": { "type": "resistor-axial", "resistance": "10" },
          "rb": { "type": "resistor-axial", "resistance": "1k" },
          "q1": { "type": "npn-2n2222-to92" },
          "bz": { "type": "buzzer-12mm" }"#,
        r#"["nano.5V", "rs.a"], ["rs.b", "bz.+"], ["bz.-", "q1.collector"],
           ["q1.emitter", "nano.GND"], ["nano.D8", "rb.a"], ["rb.b", "q1.base"]"#,
    );
    let got = sweep(&mut live, "step 1ms", 60);
    assert_eq!(only_notes(&got, &[440.1]), [440.1], "{got:?}");
}

/// D8 switches the coil's `-` to ground through a 2N2222. `supply` feeds `+`.
fn switched(parts: &str, supply: &str) -> Live {
    opened(
        "drive",
        &format!(
            r#""rb": {{ "type": "resistor-axial", "resistance": "1k" }},
          "q1": {{ "type": "npn-2n2222-to92" }},
          "bz": {{ "type": "buzzer-12mm" }}, {parts}"#
        ),
        &format!(
            r#"{supply}, ["bz.-", "q1.collector"],
           ["q1.emitter", "nano.GND"], ["nano.D8", "rb.a"], ["rb.b", "q1.base"]"#
        ),
    )
}

#[test]
fn a_coil_whose_supply_is_cut_reads_0_while_the_pin_still_plays() {
    // D9 feeds the coil through 10 ohm. It goes low 100 ms into a 440 Hz tone that plays on
    // for 60 ms.
    let mut live = switched(
        r#""rs": { "type": "resistor-axial", "resistance": "10" }"#,
        r#"["nano.D9", "rs.a"], ["rs.b", "bz.+"]"#,
    );
    phase(&mut live, "cut");
    assert!(run(&mut live, "step 100ms").ok());
    assert_near(hz(&mut live), 440.1, 0.002);
    assert!(run(&mut live, "step 25ms").ok());
    for _ in 0..4 {
        assert!(volts(&mut live) < 0.01);
        assert_eq!(hz(&mut live), 0.0);
        assert!(run(&mut live, "step 10ms").ok());
    }
}

#[test]
fn a_31_hz_coil_whose_supply_is_cut_reads_0_within_17_ms_while_the_base_still_toggles() {
    // `tone(8, 31)` switches the coil through the 2N2222 while D9 feeds it through 10 ohm. D9
    // goes low about 132.5 ms after `drop` prints, and D8 plays on for 60 ms after that.
    let mut live = opened(
        "drop",
        r#""rs": { "type": "resistor-axial", "resistance": "10" },
          "rb": { "type": "resistor-axial", "resistance": "1k" },
          "q1": { "type": "npn-2n2222-to92" },
          "bz": { "type": "buzzer-12mm" }"#,
        r#"["nano.D9", "rs.a"], ["rs.b", "bz.+"], ["bz.-", "q1.collector"],
           ["q1.emitter", "nano.GND"], ["nano.D8", "rb.a"], ["rb.b", "q1.base"]"#,
    );
    phase(&mut live, "drop");
    let (mut got, mut v) = (Vec::new(), Vec::new());
    for _ in 0..185 {
        assert!(run(&mut live, "step 1ms").ok());
        got.push(hz(&mut live));
        v.push(volts(&mut live));
    }
    // Sample `k` is `k + 1` ms in.
    assert_near(got[127], 31.0, 0.005);
    assert!(v[114..132].iter().any(|v| *v > 1.0), "{v:?}");
    assert!(v[133..].iter().all(|v| *v < 0.01), "{v:?}");
    // D9 fell before 134 ms and the coil has not swung since, so 17 ms later it reads 0.
    assert!(got[149..].iter().all(|hz| *hz == 0.0), "{got:?}");
}

#[test]
fn thirty_ms_of_110_hz_just_after_880_hz_reads_110() {
    // `tone(8, 880)` for 40 ms, then `tone(8, 110)` for 30 ms, then `noTone`.
    let mut live = drive_scene(DRIVE);
    phase(&mut live, "after");
    settle(&mut live);
    // `noTone` lands 72 ms in, reads 0 from 89 ms, and `cut` plays at 92 ms.
    let got = sweep(&mut live, "step 1ms", 90);
    let order = only_notes(&got, &[880.3, 110.0]);
    assert_eq!(order, [880.3, 110.0, 0.0], "{got:?}");
}

#[test]
fn a_fresh_tone_through_the_transistor_reads_only_its_note_at_every_1_ms() {
    // `tone(8, 440, 100)` from silence, the first note after the board starts.
    let mut live = switched(
        r#""rs": { "type": "resistor-axial", "resistance": "10" }"#,
        r#"["nano.5V", "rs.a"], ["rs.b", "bz.+"]"#,
    );
    phase(&mut live, "fresh");
    settle(&mut live);
    // The tone ends 102 ms in, reads 0 from 119 ms, and `after` plays again at 122 ms.
    let got = sweep(&mut live, "step 1ms", 121);
    assert_eq!(only_notes(&got, &[440.1]), [440.1, 0.0], "{got:?}");
    let note = got.iter().filter(|hz| near(**hz, 440.1, 0.005)).count();
    assert!(note >= 95, "440 read for {note} ms of 100: {got:?}");
}

fn drive_over_ps(live: &mut Live) -> u64 {
    let json = run(live, "snapshot").json();
    let found: Vec<u64> = json["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "rating.voltage")
        .map(|d| d["data"]["over_ps"].as_u64().unwrap())
        .collect();
    assert_eq!(found.len(), 1, "{json}");
    found[0]
}

#[test]
fn a_drive_over_5_v_counts_while_it_sounds_and_not_while_it_rests() {
    // A 6 V jack feeds the coil. `noTone` lands 160 ms into the `cut` phase's tone.
    let mut live = switched(
        r#""j1": { "type": "barrel-jack-dc21", "voltage": 6 }"#,
        r#"["j1.center", "bz.+"], ["j1.sleeve", "nano.GND"]"#,
    );
    phase(&mut live, "cut");
    assert!(run(&mut live, "step 50ms").ok());
    let sounding = drive_over_ps(&mut live);
    assert!(run(&mut live, "step 100ms").ok());
    let grew = drive_over_ps(&mut live) - sounding;
    assert!(
        (95_000_000_000..=105_000_000_000).contains(&grew),
        "100 ms of tone counted {grew} ps"
    );
    assert!(run(&mut live, "step 60ms").ok());
    let after = drive_over_ps(&mut live);
    assert!(run(&mut live, "step 40ms").ok());
    assert_eq!(drive_over_ps(&mut live), after, "the rest counted");
}

/// The sketch `melody`'s xorshift32, from the same seed.
struct Draw(u32);

impl Draw {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// What one of `melody`'s events plays: a timer frequency on a pin, or nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    Tone { hz: f64 },
    Pwm { pin: u8, duty: u8, hz: f64 },
    Rest,
}

/// The frequency Timer2 plays for `tone(pin, f)`, as the AVR runtime 1.8.6 picks its prescaler
/// and compare value.
fn timer2(f: u32) -> f64 {
    let half = 16_000_000 / f / 2;
    let presc = [1, 8, 32, 64, 128, 256, 1024]
        .into_iter()
        .find(|p| half / p - 1 <= 255)
        .unwrap_or(1024);
    let ocr = half / presc - 1;
    16e6 / (2.0 * presc as f64 * (ocr + 1) as f64)
}

/// `melody`'s first `n` events, in the order it plays them.
fn melody(n: usize) -> Vec<Event> {
    let mut draw = Draw(0x2545_F491);
    let mut last: u32 = 440;
    let mut out = Vec::new();
    while out.len() < n {
        let kind = draw.below(16);
        let pitch = |draw: &mut Draw| -> u32 {
            match draw.below(16) {
                0 => 31,
                1 => 4000,
                _ => {
                    let octave = draw.below(7);
                    let step = draw.below(1024);
                    (31 << octave) * (1024 + step) / 1024
                }
            }
        };
        let length = |draw: &mut Draw| {
            if draw.below(4) != 0 {
                draw.below(36);
            } else {
                draw.below(296);
            }
        };
        let event = if kind < 6 {
            last = pitch(&mut draw);
            length(&mut draw);
            Event::Tone { hz: timer2(last) }
        } else if kind < 8 {
            last = pitch(&mut draw);
            length(&mut draw);
            length(&mut draw);
            Event::Tone { hz: timer2(last) }
        } else if kind == 8 {
            length(&mut draw);
            Event::Tone { hz: timer2(last) }
        } else if kind == 9 {
            let ratio = 2048 + draw.below(6144);
            let f = if draw.below(2) == 1 {
                last * ratio / 1024
            } else {
                last * 1024 / ratio
            };
            last = f.clamp(31, 4000);
            length(&mut draw);
            Event::Tone { hz: timer2(last) }
        } else if kind < 12 {
            if draw.below(4) != 0 {
                draw.below(41);
            } else {
                draw.below(401);
            }
            Event::Rest
        } else {
            let pin = [3, 5, 9][draw.below(3) as usize];
            let duty = [1, 4, 128, 254][draw.below(4) as usize];
            length(&mut draw);
            // Timer0 runs fast PWM on D5, Timers 1 and 2 phase-correct PWM on D9 and D3.
            let hz = if pin == 5 {
                16e6 / 64.0 / 256.0
            } else {
                16e6 / 64.0 / 510.0
            };
            Event::Pwm { pin, duty, hz }
        };
        out.push(event);
    }
    out
}

/// A pin edge, `true` for a rise.
type Edge = (u64, bool);

/// What a melody played: each event with when it began, each pin's edges, and each reading
/// with its time.
type Played = (Vec<(u64, Event)>, BTreeMap<u8, Vec<Edge>>, Vec<(u64, f64)>);

/// Plays `melody` on the 2N2222 switch and reads `@bz.hz` every 1 ms until `n` events have
/// started and the last has had 300 ms. Returns the events, each with when it began, the
/// edges of D3, D5, D8 and D9, and each reading with its time.
fn play_melody(n: usize) -> Played {
    let mut live = opened(
        "melody",
        r#""rb": { "type": "resistor-axial", "resistance": "1k" },
          "r3": { "type": "resistor-axial", "resistance": "1k" },
          "r5": { "type": "resistor-axial", "resistance": "1k" },
          "r9": { "type": "resistor-axial", "resistance": "1k" },
          "rs": { "type": "resistor-axial", "resistance": "10" },
          "q1": { "type": "npn-2n2222-to92" },
          "bz": { "type": "buzzer-12mm" }"#,
        r#"["nano.5V", "rs.a"], ["rs.b", "bz.+"], ["bz.-", "q1.collector"],
           ["q1.emitter", "nano.GND"], ["nano.D8", "rb.a"], ["rb.b", "q1.base"],
           ["nano.D3", "r3.a"], ["r3.b", "q1.base"], ["nano.D5", "r5.a"], ["r5.b", "q1.base"],
           ["nano.D9", "r9.a"], ["r9.b", "q1.base"]"#,
    );
    let mut starts = Vec::new();
    let mut edges: BTreeMap<u8, Vec<Edge>> = BTreeMap::new();
    let mut reads = Vec::new();
    let mut end = u64::MAX;
    loop {
        let json = run(&mut live, "step 1ms").json();
        let data = &json["data"];
        assert_eq!(data["more"], 0, "a step left out edges: {data}");
        for change in data["changes"].as_array().unwrap() {
            let at = change["at_ps"].as_u64().unwrap();
            let high = change["level"] == "high";
            let pin = change["ref"].as_str().unwrap();
            match pin
                .strip_prefix("@nano.D")
                .and_then(|d| d.parse::<u8>().ok())
            {
                Some(12) => starts.push(at),
                Some(d) => edges.entry(d).or_default().push((at, high)),
                None => {}
            }
        }
        let now = data["from_ps"].as_u64().unwrap() + 1_000_000_000;
        reads.push((now, hz(&mut live)));
        if starts.len() >= n && end == u64::MAX {
            end = now + 300_000_000_000;
        }
        if now >= end {
            break;
        }
    }
    let events = melody(starts.len());
    (starts.into_iter().zip(events).collect(), edges, reads)
}

/// When a note that starts at `from` must read, before `until`: the end of its first full
/// cycle, rise to rise on its pin. A tone's first full cycle starts after `tone()` has set the
/// timer, which takes it under 400 us, so its start pulse begins no cycle. PWM, whose high and
/// low differ, reads from the end of its second full cycle: the first whose high and low
/// repeat the cycle before, within 1% of the period. `analogWrite` can start part way through
/// a cycle.
fn first_cycle(edges: &[Edge], from: u64, until: u64, pwm: Option<f64>) -> Option<u64> {
    let set = if pwm.is_none() {
        from + 400_000_000
    } else {
        from
    };
    let after: Vec<Edge> = edges
        .iter()
        .copied()
        .filter(|(at, _)| *at >= set && *at < until)
        .collect();
    // Each full cycle as its rise, its fall and the next rise.
    let cycles: Vec<(u64, u64, u64)> = after
        .windows(3)
        .filter(|w| w[0].1 && !w[1].1 && w[2].1)
        .map(|w| (w[0].0, w[1].0, w[2].0))
        .collect();
    let Some(hz) = pwm else {
        return cycles.first().map(|c| c.2);
    };
    let close = |a: u64, b: u64| a.abs_diff(b) as f64 <= 0.01e12 / hz;
    cycles
        .windows(2)
        .find(|pair| {
            let (a, b) = (pair[0], pair[1]);
            close(a.1 - a.0, b.1 - b.0) && close(a.2 - a.1, b.2 - b.1)
        })
        .map(|pair| pair[1].2)
}

/// Checks each reading against the acceptance rule. Returns each sample that breaks it.
fn judge(
    events: &[(u64, Event)],
    edges: &BTreeMap<u8, Vec<Edge>>,
    reads: &[(u64, f64)],
) -> Vec<String> {
    const REST: u64 = 17_000_000_000;
    let pin_of = |event: &Event| match event {
        Event::Tone { .. } => Some(8),
        Event::Pwm { pin, .. } => Some(*pin),
        Event::Rest => None,
    };
    let rate = |event: &Event| match event {
        Event::Tone { hz } | Event::Pwm { hz, .. } => *hz,
        Event::Rest => 0.0,
    };
    // For each sounding event: its start, the next event's start, its rate, when it must read
    // if it plays that long, and its last edge before the next event.
    struct Note {
        from: u64,
        to: u64,
        hz: f64,
        heard: Option<u64>,
        last_edge: Option<u64>,
        event: Event,
    }
    let mut notes = Vec::new();
    for (k, (from, event)) in events.iter().enumerate() {
        let Some(pin) = pin_of(event) else { continue };
        let to = events.get(k + 1).map_or(u64::MAX, |(at, _)| *at);
        let own = edges.get(&pin).map(Vec::as_slice).unwrap_or(&[]);
        let heard = first_cycle(own, *from, to, (pin != 8).then(|| rate(event)));
        let last_edge = own
            .iter()
            .filter(|(at, _)| *at >= *from && *at < to)
            .map(|(at, _)| *at)
            .next_back();
        notes.push(Note {
            from: *from,
            to,
            hz: rate(event),
            heard,
            last_edge,
            event: *event,
        });
    }
    let all: Vec<u64> = {
        let mut all: Vec<u64> = edges.values().flatten().map(|(at, _)| *at).collect();
        all.sort_unstable();
        all
    };
    let near = |got: f64, want: f64| (got - want).abs() <= 0.005 * want;
    let mut bad = Vec::new();
    for &(now, got) in reads {
        let moved = all.partition_point(|at| *at <= now);
        let idle = moved == 0 || now - all[moved - 1] > REST;
        let cur = notes.iter().rposition(|n| n.from <= now);
        let why = if idle {
            (got != 0.0).then(|| "the drive has stopped".to_string())
        } else if let Some(c) = cur {
            let note = &notes[c];
            // The note that was playing, and the last one heard when that one never was.
            let before = [
                c.checked_sub(1).map(|b| notes[b].hz),
                notes[..c]
                    .iter()
                    .rev()
                    .find(|n| n.heard.is_some())
                    .map(|n| n.hz),
            ];
            let due = note.heard.is_some_and(|h| now >= h)
                && note.last_edge.is_some_and(|last| now <= last)
                && now < note.to;
            if due && !near(got, note.hz) {
                Some(format!("{:?} is due", note.event))
            } else if got != 0.0
                && !near(got, note.hz)
                && !before.iter().flatten().any(|b| near(got, *b))
            {
                Some(format!("{:?} plays, {before:?} before it", note.event))
            } else {
                None
            }
        } else {
            (got != 0.0).then(|| "nothing has played".to_string())
        };
        if let Some(why) = why {
            bad.push(format!("{:.3} ms read {got}: {why}", now as f64 / 1e9));
        }
    }
    bad
}

fn random_melodies(n: usize) {
    let (events, edges, reads) = play_melody(n);
    let bad = judge(&events, &edges, &reads);
    let shown: Vec<&String> = bad.iter().take(40).collect();
    assert!(
        bad.is_empty(),
        "{} of {} readings break the rule:\n{shown:#?}",
        bad.len(),
        reads.len()
    );
}

/// The first 14 events of `melody`, about 1 s of play.
#[test]
fn random_melodies_read_only_the_notes_played() {
    random_melodies(14);
}

/// The first 200 events of `melody`, about 12 s of play.
#[test]
#[ignore = "about 10 minutes: run it with --run-ignored only"]
fn many_random_melodies_read_only_the_notes_played() {
    random_melodies(200);
}

#[test]
fn a_drive_under_3_v_counts_until_its_last_pulse_ends() {
    // D8 drives the coil through 220 ohm. `cut` plays 440 Hz from 20 ms to 180 ms, and the
    // `khz` tone after it starts at 260 ms.
    let mut live = opened(
        "drive",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },
    "bz": { "type": "buzzer-12mm" }"#,
        DIVIDER,
    );
    phase(&mut live, "cut");
    let before = drive_over_ps(&mut live);
    assert!(run(&mut live, "step 230ms").ok());
    let over = drive_over_ps(&mut live) - before;
    // The engine's next sample after the last pulse can be a step of 10 ms later.
    assert!(
        (155_000_000_000..=161_000_000_000).contains(&over),
        "160 ms of tone counted {over} ps"
    );
    // A snapshot counts a pulse still sounding up to it, so this one ends in the rest.
    assert!(run(&mut live, "step 20ms").ok());
    assert_eq!(drive_over_ps(&mut live) - before, over, "the rest counted");
}
