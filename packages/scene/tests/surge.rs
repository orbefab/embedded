//! Each catalog part whose current rating is sustained with a peak limit rides out the surge its
//! sheet allows, from cold, with no diagnostic at any step during it or after it. A surge just
//! over the sheet's peak warns, and so does a current held just over the sustained rating. A
//! current source drives the part from anode to cathode, as the sheet's forward rows do.

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

/// `level` from 1 ms on, held.
fn held(level: f64) -> String {
    format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": {MS}, "level": {} }}] }}"#,
        amps(0.0),
        amps(level)
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

/// The rating warnings of `id` driven by `current`, read after each millisecond up to `ms`.
fn warnings(id: &str, extra: &str, current: &str, ms: u64) -> Vec<String> {
    let times: Vec<u64> = (1..=ms).map(|at| at * MS).collect();
    drive(id, extra, current, false, &times)
}

/// A current source on `id`. `reverse` drives the cathode, as a zener surge does.
fn scene(id: &str, extra: &str, current: &str, reverse: bool) -> tempfile::TempDir {
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
    let (hot, cold) = if reverse {
        ("cathode", "anode")
    } else {
        ("anode", "cathode")
    };
    let scene = format!(
        r#"{{
  "format": 1,
  "parts": {{
    "i1": {{ "type": "./source/part.jsonc" }},
    "d1": {{ "type": "{id}"{extra} }}
  }},
  "wires": [["i1.out", "d1.{hot}"], ["d1.{cold}", "i1.back"]],
  "ground": "i1.back"
}}
"#
    );
    fs::write(dir.join("scene.jsonc"), scene).unwrap();
    project
}

/// The rating warnings at each time in `times`, which must ascend. `reverse` drives the
/// cathode, as a zener surge does.
fn drive(id: &str, extra: &str, current: &str, reverse: bool, times: &[u64]) -> Vec<String> {
    let project = scene(id, extra, current, reverse);
    sample(project.path(), times)
}

fn sample(dir: &Path, times: &[u64]) -> Vec<String> {
    let netlist = orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc"))
        .unwrap_or_else(|found| panic!("{found:#?}"));
    let mut session = Session::open_json(netlist.json.as_bytes(), &Blobs::new()).unwrap();
    let mut seen = Vec::new();
    for at in times {
        session.advance_to(Time::from_ps(*at));
        assert!(session.fault().is_none(), "{:?}", session.fault());
        for d in session.diagnostics() {
            if let Diagnostic::RatingCurrent(e) | Diagnostic::RatingPower(e) = &d {
                let now = if d.over_now() { " now" } else { "" };
                let seen_at = format!("{} {}{now} by {}ps", d.code(), e.quantity, at);
                if !seen
                    .iter()
                    .any(|s: &String| s.split(" by ").next() == seen_at.split(" by ").next())
                {
                    seen.push(seen_at);
                }
            }
        }
    }
    seen
}

/// The rating messages present at `at`, as a user reads them.
fn messages(id: &str, extra: &str, current: &str, reverse: bool, at: u64) -> Vec<String> {
    let project = scene(id, extra, current, reverse);
    let netlist = orbe_scene::read(&Files::disk(), &project.path().join("scene.jsonc"))
        .unwrap_or_else(|found| panic!("{found:#?}"));
    let mut session = Session::open_json(netlist.json.as_bytes(), &Blobs::new()).unwrap();
    session.advance_to(Time::from_ps(at));
    assert!(session.fault().is_none(), "{:?}", session.fault());
    session
        .diagnostics()
        .iter()
        .filter(|d| matches!(d, Diagnostic::RatingCurrent(_) | Diagnostic::RatingPower(_)))
        .map(|d| d.message_with(&|r| r.to_string()))
        .collect()
}

/// The sheet's surge at a peak.
type Surge = fn(f64) -> String;

/// Each part, its scene extras, the shape of the surge its sheet allows, that surge's peak, and
/// its current rating.
fn cases() -> Vec<(&'static str, &'static str, Surge, f64, f64)> {
    // The CD1206's 4 A surge gives no width. 100 us of it fills a hundredth of the 10.2 ms time
    // constant.
    let short: Surge = |peak| square(peak, 100_000_000);
    let mut cases: Vec<(&str, &str, Surge, f64, f64)> = vec![
        ("diode-sod123", "", half_sine, 5.5, 0.5),
        ("diode-1n4007", "", half_sine, 30.0, 1.0),
        ("diode-m7-sma", "", half_sine, 30.0, 1.0),
        ("diode-cd1206", "", short, 4.0, 0.15),
    ];
    for (id, extra, peak, dc) in [
        ("led-5mm", "", 0.16, 0.03),
        ("led-5mm-green", "", 0.14, 0.025),
        ("led-3mm-blue", "", 0.1, 0.03),
        ("led-0603", r#", "color": "red""#, 0.185, 0.03),
        ("led-0805", r#", "color": "red""#, 0.185, 0.03),
    ] {
        cases.push((id, extra, tenth_duty as Surge, peak, dc));
    }
    cases
}

#[test]
fn each_sustained_rating_rides_out_its_sheet_s_surge_and_warns_when_held_over() {
    for (id, extra, surge, peak, rating) in cases() {
        assert_eq!(
            warnings(id, extra, &surge(peak), 30),
            Vec::<String>::new(),
            "{id} surge"
        );
        // 10% over the sheet's peak warns on the peak limit, during the surge.
        let over = warnings(id, extra, &surge(1.1 * peak), 30);
        assert!(
            over.iter()
                .any(|w| w.starts_with("rating.current #d1.i by ")),
            "{id} over its peak: {over:?}"
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

/// A step that lands inside the sheet's surge used to call power over now, because the power
/// rating has no peak of its own. The current peak bounds that pulse.
#[test]
fn a_step_inside_a_legal_surge_does_not_call_power_over_now() {
    let short = |peak| square(peak, 100_000_000);
    assert_eq!(
        drive("diode-cd1206", "", &short(4.0), false, &[1_050_000_000]),
        Vec::<String>::new(),
        "CD1206 at 4 A, 50 us into its 100 us surge"
    );
    let zener = |peak| square(peak, 8_300_000_000);
    assert_eq!(
        drive("zener-1n4733a", "", &zener(0.89), true, &[2_000_000_000]),
        Vec::<String>::new(),
        "1N4733A at 0.89 A reverse, 1 ms into the surge"
    );
}

/// The model's 8.45 W at 0.89 A used to cross the 1 W rating during the sheet's 8.3 ms surge,
/// because the power time constant was fitted to 4.5 W at the typical 5.1 V.
#[test]
fn a_legal_1n4733a_reverse_surge_stays_quiet_and_a_longer_one_warns() {
    let legal = square(0.89, 8_300_000_000);
    assert_eq!(
        drive(
            "zener-1n4733a",
            "",
            &legal,
            true,
            &[8 * MS, 9 * MS, 9_300_000_000, 12 * MS]
        ),
        Vec::<String>::new(),
        "0.89 A reverse for the sheet's 8.3 ms"
    );
    let held_surge = held(0.89);
    let warned = messages("zener-1n4733a", "", &held_surge, true, 21 * MS);
    assert_eq!(
        warned,
        vec![
            "#d1.i averages 0.338 A with a 34 ms time constant while it is 0.89 A, over its 0.196 A rating".to_string(),
            "#d1.p averages 1.82 W with a 67 ms time constant while it is 8.45 W, over its 1 W rating".to_string(),
        ]
    );
}

/// 4.02 A is just over the CD1206's 4 A surge peak. The current peak warns at once. Power is
/// never over now: the filter warns once the heat crosses.
#[test]
fn a_cd1206_just_over_its_surge_peak_warns_on_current_at_once() {
    let early = messages("diode-cd1206", "", &held(4.02), false, 1_100_000_000);
    assert_eq!(
        early,
        vec!["#d1.i peaks at 4.02 A, over its 4 A rating".to_string()]
    );
    let later = messages("diode-cd1206", "", &held(4.02), false, 2 * MS);
    assert!(
        later.iter().any(|w| {
            w == "#d1.p averages 0.816 W with a 10.2 ms time constant while it is 16.3 W, over its 0.4 W rating"
        }),
        "{later:?}"
    );
    assert!(later.iter().all(|w| !w.contains(" now")), "{later:?}");
}

/// An LED held inside its pulse peak is over now on power.
#[test]
fn an_led_inside_its_pulse_peak_is_over_now_on_power() {
    let found = messages("led-5mm", "", &held(0.10), false, 3 * MS);
    assert!(
        found
            .iter()
            .any(|w| w.starts_with("#d1.p is 0.235 W now, over its 0.075 W rating")),
        "{found:?}"
    );
}
