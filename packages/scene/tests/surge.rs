//! Each catalog part whose current rating is sustained rides out the surge its sheet allows,
//! from cold, and warns on a current held just over its rating. A current source drives the
//! part from anode to cathode, as the sheet's forward rows do.

use std::f64::consts::PI;
use std::fs;
use std::path::Path;

use orbe_model::Time;

use orbe_engine::Session;
use orbe_engine::blob::Blobs;
use orbe_engine::diagnostic::Diagnostic;
use orbe_scene::Files;

mod common;

const MS: u64 = 1_000_000_000;

/// One 60 Hz half wave, the sheets' IFSM width.
const HALF_WAVE_PS: u64 = 8_333_333_333;

/// A value object with a source, as a part file writes one.
fn amps(value: f64) -> String {
    format!(r#"{{ "value": {value}, "unit": "A", "source": "user" }}"#)
}

/// One half sine of `peak` from 1 ms, in 65 held steps of its mean over each step. An odd count
/// keeps the two middle steps from being one level.
fn half_sine(peak: f64) -> String {
    let steps = 65;
    let width = HALF_WAVE_PS / steps;
    let events: Vec<String> = (0..steps)
        .map(|k| {
            let (a, b) = (k as f64 / steps as f64, (k + 1) as f64 / steps as f64);
            let mean = peak * (libm::cos(PI * a) - libm::cos(PI * b)) / (PI * (b - a));
            format!(
                r#"{{ "t_ps": {}, "level": {} }}"#,
                MS + k * width,
                amps(mean)
            )
        })
        .chain([format!(
            r#"{{ "t_ps": {}, "level": {} }}"#,
            MS + steps * width,
            amps(0.0)
        )])
        .collect();
    format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{}] }}"#,
        amps(0.0),
        events.join(", ")
    )
}

/// `level` for `width_ps` from 1 ms, once.
fn square(level: f64, width_ps: u64) -> String {
    format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": {MS}, "level": {} }}, {{ "t_ps": {}, "level": {} }}] }}"#,
        amps(0.0),
        amps(level),
        MS + width_ps,
        amps(0.0)
    )
}

/// `peak` at 1/10 duty in 0.1 ms pulses from 1 ms, a Kingbright sheet's peak forward current.
fn tenth_duty(peak: f64) -> String {
    format!(
        r#"{{ "kind": "pulse", "initial": {}, "pulsed": {}, "delay_ps": {MS}, "width_ps": 100000000, "period_ps": 1000000000 }}"#,
        amps(0.0),
        amps(peak)
    )
}

/// The rating warnings of `id` driven by `current` for `ms` milliseconds.
fn warnings(id: &str, extra: &str, current: &str, ms: u64) -> Vec<String> {
    let project = common::project("nano-button");
    let dir = project.path();
    let source = format!(
        r#"{{
  "format": 1,
  "pins": {{ "out": [1], "back": [2] }},
  "model": {{ "kind": "current_source", "pins": {{ "p": "out", "n": "back" }}, "current": {current} }}
}}
"#
    );
    fs::create_dir_all(dir.join("source")).unwrap();
    fs::write(dir.join("source/part.jsonc"), source).unwrap();
    let scene = format!(
        r#"{{
  "format": 1,
  "parts": {{
    "i1": {{ "type": "./source/part.jsonc" }},
    "d1": {{ "type": "{id}"{extra} }}
  }},
  "wires": [["i1.out", "d1.anode"], ["d1.cathode", "i1.back"]],
  "ground": "i1.back"
}}
"#
    );
    fs::write(dir.join("scene.jsonc"), scene).unwrap();
    run(dir, ms)
}

fn run(dir: &Path, ms: u64) -> Vec<String> {
    let netlist = orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc"))
        .unwrap_or_else(|found| panic!("{found:#?}"));
    let mut session = Session::open_json(netlist.json.as_bytes(), &Blobs::new()).unwrap();
    session.advance_to(Time::from_ps(ms * MS));
    assert!(session.fault().is_none(), "{:?}", session.fault());
    session
        .diagnostics()
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingCurrent(e) | Diagnostic::RatingPower(e) => {
                Some(format!("{} {}", d.code(), e.quantity))
            }
            _ => None,
        })
        .collect()
}

/// Each part, its scene extras, the surge its sheet allows, and its current rating.
fn cases() -> Vec<(&'static str, &'static str, String, f64)> {
    let mut cases = vec![
        ("diode-sod123", "", half_sine(5.5), 0.5),
        ("diode-1n4007", "", half_sine(30.0), 1.0),
        ("diode-m7-sma", "", half_sine(30.0), 1.0),
        // The sheet's 4 A surge gives no width. 100 us of it fills a hundredth of the 10.2 ms
        // time constant.
        ("diode-cd1206", "", square(4.0, 100_000_000), 0.15),
    ];
    for (id, extra, peak, dc) in [
        ("led-5mm", "", 0.16, 0.03),
        ("led-5mm-green", "", 0.14, 0.025),
        ("led-3mm-blue", "", 0.1, 0.03),
        ("led-0603", r#", "color": "red""#, 0.185, 0.03),
        ("led-0805", r#", "color": "red""#, 0.185, 0.03),
    ] {
        cases.push((id, extra, tenth_duty(peak), dc));
    }
    cases
}

#[test]
fn each_sustained_rating_rides_out_its_sheet_s_surge_and_warns_when_held_over() {
    for (id, extra, surge, rating) in cases() {
        assert_eq!(
            warnings(id, extra, &surge, 30),
            Vec::<String>::new(),
            "{id} surge"
        );
        // Held at 1.1 times the rating, a mean crosses after `tau * ln(11)`, under three time
        // constants. The slowest here is 0.16 s.
        let held = warnings(id, extra, &amps(1.1 * rating), 500);
        assert!(
            held.iter()
                .any(|w| w.contains("rating.current") && w.contains("#d1.i")),
            "{id} held: {held:?}"
        );
    }
}
