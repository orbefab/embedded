//! A power rating is a mean through a first-order filter with a 10.2 ms time constant unless its
//! part names another basis, and each filter starts cold at the first step after t = 0. A
//! switching edge stays under. A held overload, and a pulse train whose average is over, warn.
//! A session lists a load over its rating now before its filter crosses.

use crate::common::simulate;

use orbe_engine::Session;
use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::trace::Trace;
use orbe_model::Time;
use serde_json::Value;

const TAU_PS: u64 = 10_200_000_000;
const PWM_490_PS: u64 = 2_040_000_000;

/// IRF520, Vishay Rev. C typical fit. Power rating is the free-air limit, 2.419 W.
const IRF520: &str = r#"{
  "name": "q1",
  "kind": "nmos",
  "pins": { "drain": "drain", "gate": "gate", "source": "gnd" },
  "transconductance_parameter": { "value": 1.259, "unit": "A/V^2" },
  "threshold_voltage": { "value": 3.18, "unit": "V" },
  "channel_length_modulation": { "value": 0.0014, "unit": "1/V" },
  "gate_source_capacitance": { "value": 3.26e-10, "unit": "F" },
  "gate_drain_capacitance": { "value": 3.4e-11, "unit": "F" },
  "drain_source_capacitance": { "value": 1.16e-10, "unit": "F" },
  "ratings": {
    "current": { "value": 9.2, "unit": "A" },
    "power": { "value": 2.419, "unit": "W" }
  }
}"#;

/// 12 V, 12 ohm, switched by the IRF520. `gate` is the gate source's voltage object.
/// The load and the gate resistor are unrated, so a warning can only be the MOSFET's.
fn switched(gate: &str, stop_ps: u64) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": 1000000000, "grid_ps": 10000000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "v12", "drain", "gate", "pin"],
      "parts": [
        {{
          "name": "v12",
          "kind": "voltage_source",
          "pins": {{ "p": "v12", "n": "gnd" }},
          "voltage": {{ "value": 12.0, "unit": "V" }}
        }},
        {{
          "name": "vgate",
          "kind": "voltage_source",
          "pins": {{ "p": "pin", "n": "gnd" }},
          "voltage": {gate}
        }},
        {{
          "name": "rg",
          "kind": "resistor",
          "pins": {{ "a": "pin", "b": "gate" }},
          "resistance": {{ "value": 220.0, "unit": "ohm" }},
          "ratings": {{ "power": null }}
        }},
        {{
          "name": "rload",
          "kind": "resistor",
          "pins": {{ "a": "v12", "b": "drain" }},
          "resistance": {{ "value": 12.0, "unit": "ohm" }},
          "ratings": {{ "power": null }}
        }},
        {IRF520}
      ]
    }}"#
    )
}

fn pulse(high: f64, delay_ps: u64, width_ps: u64, period_ps: u64) -> String {
    format!(
        r#"{{
          "kind": "pulse",
          "initial": {{ "value": 0.0, "unit": "V" }},
          "pulsed": {{ "value": {high}, "unit": "V" }},
          "delay_ps": {delay_ps}, "width_ps": {width_ps}, "period_ps": {period_ps}
        }}"#
    )
}

fn warnings(json: &str, code: &str) -> Vec<serde_json::Value> {
    simulate(json)
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == code)
        .map(|diagnostic| serde_json::to_value(diagnostic).unwrap())
        .collect()
}

fn power_warnings(json: &str) -> Vec<serde_json::Value> {
    warnings(json, "rating.power")
}

fn assert_switched(json: &str) {
    let results = simulate(json);
    let power: Vec<_> = results
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == "rating.power")
        .map(Diagnostic::code)
        .collect();
    assert!(power.is_empty(), "{:?}", results.diagnostics());
    let trace = results.trace().expect("a trace");
    let (low, high) = span(trace, "#net.drain");
    assert!(low < 1.0 && high > 11.0, "drain {low} to {high}");
    assert_eq!(trace.end().unwrap().as_ps(), 1_000_000_000_000);
}

/// Lowest and highest sample of `reference`, rows and the steps between them.
fn span(trace: &Trace, reference: &str) -> (f64, f64) {
    let reference: orbe_engine::name::Ref = reference.parse().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|r| *r == reference)
        .unwrap();
    let low = trace
        .min_column(index)
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let high = trace
        .max_column(index)
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    (low, high)
}

/// 490 Hz, 50% duty, for a second. Each edge crosses a few watts for about 100 ns.
#[test]
fn a_switching_irf520_at_490_hz_for_a_second_stays_under_its_power_rating() {
    let gate = pulse(5.0, 1_000_000, PWM_490_PS / 2, PWM_490_PS);
    assert_switched(&switched(&gate, 1_000_000_000_000));
}

/// The same 490 Hz switching in a session, probed every 10 ns across the first rising edge at
/// 1 us and the falling edge 1.02 ms later. The MOSFET is past its 2.419 W rating for about
/// 100 ns at each edge, and no probe lists it over now: the average over the last 2.04 ms
/// stays far under the rating.
#[test]
fn a_switching_irf520_is_never_over_now_inside_its_edges() {
    let gate = pulse(5.0, 1_000_000, PWM_490_PS / 2, PWM_490_PS);
    let json = switched(&gate, 1_000_000_000_000);
    let mut session = Session::open(orbe_engine::netlist::parse(json.as_bytes()).unwrap()).unwrap();
    let power: orbe_engine::name::Ref = "#q1.p".parse().unwrap();
    let mut over = 0;
    for edge in [1_000_000, 1_000_000 + PWM_490_PS / 2] {
        for k in 0..300u64 {
            session.advance_to(Time::from_ps(edge - 500_000 + k * 10_000));
            let probe = session.probe();
            if probe.get(&power).unwrap().get() > 2.42 {
                over += 1;
            }
            assert_eq!(
                session.over_ratings(&probe),
                Vec::<Diagnostic>::new(),
                "at {:?}",
                probe.at()
            );
        }
    }
    assert!(over >= 10, "{over} probes past the rating");
}

/// On for half a second, then off, through one second.
#[test]
fn an_irf520_turned_fully_on_and_off_for_a_second_stays_under_its_power_rating() {
    let gate = pulse(5.0, 1_000_000, 500_000_000_000, 1_000_000_000_000);
    assert_switched(&switched(&gate, 1_000_000_000_000));
}

/// Vgs just above the 3.18 V threshold, so the 12 ohm load drops about half of 12 V
/// and the MOSFET dissipates about 3 W. The filter rises toward that, and it warns.
#[test]
fn an_irf520_held_half_on_warns_on_its_average_power() {
    let gate = r#"{ "value": 4.07, "unit": "V" }"#;
    let results = simulate(&switched(gate, 20_000_000_000));
    let found = power_of(&results);
    assert_eq!(found.len(), 1, "{:?}", results.diagnostics());
    let warning = &found[0];
    assert_eq!(warning["refs"][0], "#q1.p", "{warning}");
    let message = warning["message"].as_str().unwrap();
    assert!(
        message.contains("averages") && message.contains("with a 10.2 ms time constant"),
        "{message}"
    );
    assert_eq!(warning["data"]["tau_ps"], TAU_PS, "{warning}");
    let peak = warning["data"]["peak"].as_f64().unwrap();
    let limit = warning["data"]["limit"].as_f64().unwrap();
    assert!((limit - 2.419).abs() < 1e-9, "{limit}");
    assert!(peak > limit, "{peak}");
    let trace = results.trace().expect("a trace");
    let reference: orbe_engine::name::Ref = "#q1.p".parse().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|r| *r == reference)
        .unwrap();
    let settled = *trace.column(index).last().unwrap();
    assert!(settled > 2.419 && settled < 6.0, "{settled}");
}

fn power_of(results: &orbe_engine::run_folder::Results) -> Vec<serde_json::Value> {
    results
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == "rating.power")
        .map(|diagnostic| serde_json::to_value(diagnostic).unwrap())
        .collect()
}

/// 5 V across 10 ohm is 2.5 W, twice a 1.25 W rating. The operating point already warns.
#[test]
fn a_resistor_at_twice_its_power_rating_warns_on_the_average() {
    let json = r#"{
      "version": 1,
      "seed": 1,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vcc"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vcc", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V" }
        },
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vcc", "b": "gnd" },
          "resistance": { "value": 10.0, "unit": "ohm" },
          "ratings": { "power": { "value": 1.25, "unit": "W" } }
        }
      ]
    }"#;
    let found = power_warnings(json);
    assert_eq!(found.len(), 1, "{found:?}");
    let warning = &found[0];
    assert_eq!(warning["refs"][0], "#r1.p");
    assert_eq!(warning["t_ps"], 0);
    assert_eq!(warning["data"]["tau_ps"], TAU_PS);
    let peak = warning["data"]["peak"].as_f64().unwrap();
    assert!((peak - 2.5).abs() < 1e-9, "{peak}");
    assert_eq!(
        warning["message"],
        "#r1.p averages 2.5 W with a 10.2 ms time constant, over its 1.25 W rating"
    );
}

/// 5 V onto 10 ohm is 2.5 W while the pulse is high. At 50% of 490 Hz the average is 1.25 W,
/// over a 1 W rating, and the filter reads it within 5% above. At 10% it is 0.25 W, under that
/// rating, and the peaks are still 2.5 W.
#[test]
fn a_pulse_train_warns_only_when_its_average_power_is_over() {
    let run = |width: u64| {
        let json = format!(
            r#"{{
          "version": 1,
          "seed": 1,
          "analysis": {{ "transient": {{ "stop_ps": 50000000000, "max_step_ps": 1000000000, "grid_ps": 10000000000 }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {}
            }},
            {{
              "name": "r1",
              "kind": "resistor",
              "pins": {{ "a": "vcc", "b": "gnd" }},
              "resistance": {{ "value": 10.0, "unit": "ohm" }},
              "ratings": {{ "power": {{ "value": 1.0, "unit": "W" }} }}
            }}
          ]
        }}"#,
            pulse(5.0, 2_000, width, PWM_490_PS)
        );
        let results = simulate(&json);
        let (_low, high) = span(results.trace().expect("a trace"), "#r1.p");
        assert!(high > 2.0, "{high}");
        power_of(&results)
    };
    let over = run(PWM_490_PS / 2);
    assert_eq!(over.len(), 1, "{over:?}");
    let warning = &over[0];
    assert_eq!(warning["refs"][0], "#r1.p");
    let message = warning["message"].as_str().unwrap();
    assert!(
        message.contains("averages") && message.contains("with a 10.2 ms time constant"),
        "{message}"
    );
    assert_eq!(warning["data"]["tau_ps"], TAU_PS);
    let peak = warning["data"]["peak"].as_f64().unwrap();
    assert!((1.25..1.32).contains(&peak), "{peak}");
    assert!(run(PWM_490_PS / 10).is_empty());
}

/// Each `rating.*` warning a session lists through its probe, as JSON.
fn over_ratings(session: &Session) -> Vec<Value> {
    session
        .over_ratings(&session.probe())
        .iter()
        .map(|diagnostic| serde_json::to_value(diagnostic).unwrap())
        .collect()
}

/// An ideal 5 V source rated 0.5 A on `basis`, 100 uF across it held at `initial` volts, and
/// 1 kohm. An empty `basis` takes the source's default.
fn held_across_a_source(initial: f64, basis: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": 20000000000, "max_step_ps": 1000000000, "grid_ps": 1000000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V" }},
          "ratings": {{ "current": {{ "value": 0.5, "unit": "A"{basis} }} }}
        }},
        {{
          "name": "c1",
          "kind": "capacitor",
          "pins": {{ "a": "vcc", "b": "gnd" }},
          "capacitance": {{ "value": 1e-4, "unit": "F" }},
          "initial": {{ "value": {initial}, "unit": "V" }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "vcc", "b": "gnd" }},
          "resistance": {{ "value": 1000.0, "unit": "ohm" }},
          "ratings": {{ "power": null }}
        }}
      ]
    }}"#
    )
}

/// A capacitor held at a voltage its source disagrees with draws 1e9 A per volt per uF for
/// the t = 0 solve, which is the hold and no current the source carries. The source is at
/// 5 V from the first step on and carries 5 mA, so neither a run nor a session warns, on a
/// peak rating as on a sustained one.
#[test]
fn a_held_capacitor_that_disagrees_with_its_source_does_not_warn() {
    let bases = [
        "",
        r#", "basis": "peak""#,
        r#", "basis": { "sustained": { "average": "mean", "tau": { "value": 0.001, "unit": "s" } } }"#,
    ];
    for (initial, basis) in [4.999, 0.0]
        .into_iter()
        .flat_map(|initial| bases.map(|basis| (initial, basis)))
    {
        let json = held_across_a_source(initial, basis);
        let found: Vec<_> = simulate(&json)
            .diagnostics()
            .iter()
            .map(|diagnostic| serde_json::to_value(diagnostic).unwrap())
            .collect();
        assert_eq!(found, Vec::<Value>::new(), "initial {initial}{basis}");
        let netlist = orbe_engine::netlist::parse(json.as_bytes()).unwrap();
        let mut session = Session::open(netlist).unwrap();
        assert_eq!(
            over_ratings(&session),
            Vec::<Value>::new(),
            "initial {initial}{basis}"
        );
        session.advance_to(Time::from_ps(1_000_000));
        assert_eq!(
            over_ratings(&session),
            Vec::<Value>::new(),
            "initial {initial}{basis}"
        );
    }
}

/// 5 V, `r1` of 10 ohm rated 0.25 W from `vcc` to `mid`, and `load` from `mid` to ground,
/// with a source rated 0.2 A.
fn into_mid(load: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": 22000000000, "max_step_ps": 1000000000, "grid_ps": 10000000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "mid"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V" }},
          "ratings": {{ "current": {{ "value": 0.2, "unit": "A" }} }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "vcc", "b": "mid" }},
          "resistance": {{ "value": 10.0, "unit": "ohm" }},
          "ratings": {{ "power": {{ "value": 0.25, "unit": "W" }} }}
        }},
        {load}
      ]
    }}"#
    )
}

/// A capacitor held discharged charges through r1 in 1 us. The heat is about a microjoule, so
/// the 0.25 W rating and the source's 0.2 A stay quiet.
#[test]
fn a_capacitor_starting_discharged_does_not_count_its_first_surge_as_a_window_of_heat() {
    let held = into_mid(
        r#"{
          "name": "c1",
          "kind": "capacitor",
          "pins": { "a": "mid", "b": "gnd" },
          "capacitance": { "value": 1e-7, "unit": "F" },
          "initial": { "value": 0.0, "unit": "V" }
        }"#,
    );
    assert_eq!(power_warnings(&held), Vec::<Value>::new());
    assert_eq!(warnings(&held, "rating.current"), Vec::<Value>::new());
}

/// 1 mohm from `mid` to ground puts 2.5 W on r1, ten times its rating, and 0.5 A through the
/// source, rated 0.2 A, from t = 0. A session lists both over now at open, with the sample and
/// how long it would hold: r1's filter passes 0.25 W after `tau * ln(2.5 / 2.25)`, 1.07 ms, and
/// the source's RMS passes 0.2 A after `tau * ln(0.25 / 0.21)`, 1.78 ms. A run warns on the
/// first step past each crossing, once the reading rounds above the rating.
#[test]
fn a_steady_overload_from_t_0_is_over_now_at_open_and_warns_once_its_window_crosses() {
    let json = into_mid(
        r#"{
          "name": "r2",
          "kind": "resistor",
          "pins": { "a": "mid", "b": "gnd" },
          "resistance": { "value": 0.001, "unit": "ohm" },
          "ratings": { "power": null }
        }"#,
    );
    let netlist = orbe_engine::netlist::parse(json.as_bytes()).unwrap();
    let session = Session::open(netlist).unwrap();
    let mut now = over_ratings(&session);
    now.sort_by_key(|d| d["code"].to_string());
    let codes: Vec<_> = now.iter().map(|d| (&d["code"], &d["refs"][0])).collect();
    assert_eq!(
        codes,
        [
            (&"rating.current".into(), &"#v1.i".into()),
            (&"rating.power".into(), &"#r1.p".into())
        ]
    );
    for (warning, sample, held_ms) in [(&now[0], 0.5, 1.78), (&now[1], 2.5, 1.07)] {
        let data = &warning["data"];
        assert_eq!(data["now"], true, "{warning}");
        assert_eq!(data["tau_ps"], TAU_PS);
        assert!(
            (data["sample"].as_f64().unwrap() / sample - 1.0).abs() < 1e-3,
            "{warning}"
        );
        assert!(data["peak"].as_f64().unwrap() < data["limit"].as_f64().unwrap());
        let held = data["held_ps"].as_f64().unwrap() / 1e9;
        assert!((held / held_ms - 1.0).abs() < 0.01, "{warning}");
        let t = warning["t_ps"].as_u64().unwrap();
        assert!(t > 0 && t <= 1_000_000, "{warning}");
    }

    let run = |code| warnings(&json, code);
    for (warning, first_ms) in [
        (&run("rating.power")[0], 1.08),
        (&run("rating.current")[0], 1.79),
    ] {
        assert_eq!(warning["data"]["now"], false, "{warning}");
        assert_eq!(warning["data"]["held_ps"], Value::Null, "{warning}");
        // The first accepted step past the crossing, and steps here reach 1 ms.
        let first = warning["t_ps"].as_f64().unwrap() / 1e9;
        assert!(first >= first_ms && first < first_ms + 1.0, "{warning}");
    }
    assert_eq!(run("rating.power").len(), 1);
    assert_eq!(run("rating.current").len(), 1);
}

/// 5 V, two 10 ohm resistors, and a capacitor from their midpoint to ground held at 0 V. The
/// bottom resistor, `r1`, is rated 0.25 W.
fn midpoint_held(farads: &str, stop_ps: u64, max_step_ps: u64) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps}, "grid_ps": 1000000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "mid"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V" }}
        }},
        {{
          "name": "r2",
          "kind": "resistor",
          "pins": {{ "a": "vcc", "b": "mid" }},
          "resistance": {{ "value": 10.0, "unit": "ohm" }},
          "ratings": {{ "power": null }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "mid", "b": "gnd" }},
          "resistance": {{ "value": 10.0, "unit": "ohm" }},
          "ratings": {{ "power": {{ "value": 0.25, "unit": "W" }} }}
        }},
        {{
          "name": "c1",
          "kind": "capacitor",
          "pins": {{ "a": "mid", "b": "gnd" }},
          "capacitance": {{ "value": {farads}, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }}
        }}
      ]
    }}"#
    )
}

/// 1 F from the midpoint, so RC is 5 s. At t = 0 the bottom resistor carries nothing. With
/// the capacitor open it would carry 0.625 W, and it stays at microwatts for tens of milliseconds.
#[test]
fn a_farad_held_at_0_v_does_not_warn_on_a_cold_load() {
    let json = midpoint_held("1", 50_000_000_000, 1_000_000_000);
    let results = simulate(&json);
    assert_eq!(power_of(&results), Vec::<serde_json::Value>::new());
    let (_low, high) = span(results.trace().expect("a trace"), "#r1.p");
    assert!(high < 1e-4, "{high}");
}

/// 1000 µF, so RC is 5 ms. Through the first 10.2 ms the filter reads under 0.25 W, about
/// 0.18 W, while the resistor's own power climbs past the rating. At the stop the resistor
/// takes about 0.47 W and has been over for the last 2.04 ms, so the run lists it over now.
#[test]
fn a_1000_uf_cold_load_does_not_warn_over_its_first_window() {
    let json = midpoint_held("0.001", TAU_PS, 10_000_000);
    let results = simulate(&json);
    let found = power_of(&results);
    assert!(
        found.iter().all(|warning| warning["t_ps"] != 0),
        "warned at open: {found:?}"
    );
    let [over_now] = found.as_slice() else {
        panic!("not one power warning: {found:?}");
    };
    assert_eq!(
        over_now["data"]["now"], true,
        "warned while the filter stays under 0.25 W: {found:?}"
    );
    let filter = over_now["data"]["peak"].as_f64().unwrap();
    assert!(filter > 0.15 && filter < 0.25, "{filter}");
    assert!(
        over_now["data"]["sample"].as_f64().unwrap() > 0.4,
        "{found:?}"
    );
    assert_eq!(over_now["data"]["peak_ps"], TAU_PS);
    let (_low, high) = span(results.trace().expect("a trace"), "#r1.p");
    assert!(high > 0.4, "{high}");
}
