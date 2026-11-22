//! Power ratings are the average over the same 10.2 ms window a DC current uses.
//! A switching edge stays under. A held overload, and a pulse train whose average is over, warn.

use crate::common::simulate;

use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::trace::Trace;

const WINDOW_PS: u64 = 10_200_000_000;
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

/// On for half a second, then off, through one second.
#[test]
fn an_irf520_turned_fully_on_and_off_for_a_second_stays_under_its_power_rating() {
    let gate = pulse(5.0, 1_000_000, 500_000_000_000, 1_000_000_000_000);
    assert_switched(&switched(&gate, 1_000_000_000_000));
}

/// Vgs just above the 3.18 V threshold, so the 12 ohm load drops about half of 12 V
/// and the MOSFET dissipates about 3 W. The window starts full of that, and it warns.
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
        message.contains("averages") && message.contains("over the last 10.2 ms"),
        "{message}"
    );
    assert_eq!(warning["data"]["window_ps"], WINDOW_PS, "{warning}");
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
    assert_eq!(warning["data"]["window_ps"], WINDOW_PS);
    let peak = warning["data"]["peak"].as_f64().unwrap();
    assert!((peak - 2.5).abs() < 1e-9, "{peak}");
    assert_eq!(
        warning["message"],
        "#r1.p averages 2.5 W over the last 10.2 ms, over its 1.25 W rating"
    );
}

/// 5 V onto 10 ohm is 2.5 W while the pulse is high. At 50% of 490 Hz the average is 1.25 W,
/// over a 1 W rating. At 10% it is 0.25 W, under that rating, and the peaks are still 2.5 W.
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
        message.contains("averages") && message.contains("over the last 10.2 ms"),
        "{message}"
    );
    assert_eq!(warning["data"]["window_ps"], WINDOW_PS);
    let peak = warning["data"]["peak"].as_f64().unwrap();
    assert!((peak - 1.25).abs() < 0.02, "{peak}");
    assert!(run(PWM_490_PS / 10).is_empty());
}

/// A capacitor held discharged at t = 0 draws 2.5 W through r1 for that first solve, then
/// charges in 1 us. The heat is about a microjoule, so the 0.25 W rating and the source's
/// 0.2 A rating stay quiet. The same resistor with nothing held is a steady overload and
/// warns at open.
#[test]
fn a_capacitor_starting_discharged_does_not_count_its_first_surge_as_a_window_of_heat() {
    let json = |cap: &str| {
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
        {cap}
      ]
    }}"#
        )
    };
    let held = r#"{
          "name": "c1",
          "kind": "capacitor",
          "pins": { "a": "mid", "b": "gnd" },
          "capacitance": { "value": 1e-7, "unit": "F" },
          "initial": { "value": 0.0, "unit": "V" }
        }"#;
    let held = json(held);
    assert_eq!(power_warnings(&held), Vec::<serde_json::Value>::new());
    assert_eq!(
        warnings(&held, "rating.current"),
        Vec::<serde_json::Value>::new()
    );
    let shorted = r#"{
          "name": "r2",
          "kind": "resistor",
          "pins": { "a": "mid", "b": "gnd" },
          "resistance": { "value": 0.001, "unit": "ohm" },
          "ratings": { "power": null }
        }"#;
    let shorted = json(shorted);
    let found = power_warnings(&shorted);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["refs"][0], "#r1.p");
    assert_eq!(found[0]["t_ps"], 0);
    let current = warnings(&shorted, "rating.current");
    assert_eq!(current.len(), 1, "{current:?}");
    assert_eq!(current[0]["refs"][0], "#v1.i");
    assert_eq!(current[0]["t_ps"], 0);
}

/// A steady 2.5 W beside a held capacitor still fills that resistor's window at open. The
/// surge branch does not.
#[test]
fn a_steady_overload_beside_a_held_capacitor_still_warns_at_open() {
    let json = r#"{
      "version": 1,
      "seed": 1,
      "analysis": { "transient": { "stop_ps": 22000000000, "max_step_ps": 1000000000, "grid_ps": 10000000000 } },
      "ground": "gnd",
      "nets": ["gnd", "vcc", "mid"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vcc", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V" }
        },
        {
          "name": "rload",
          "kind": "resistor",
          "pins": { "a": "vcc", "b": "gnd" },
          "resistance": { "value": 10.0, "unit": "ohm" },
          "ratings": { "power": { "value": 0.25, "unit": "W" } }
        },
        {
          "name": "r_surge",
          "kind": "resistor",
          "pins": { "a": "vcc", "b": "mid" },
          "resistance": { "value": 10.0, "unit": "ohm" },
          "ratings": { "power": { "value": 0.25, "unit": "W" } }
        },
        {
          "name": "c1",
          "kind": "capacitor",
          "pins": { "a": "mid", "b": "gnd" },
          "capacitance": { "value": 1e-7, "unit": "F" },
          "initial": { "value": 0.0, "unit": "V" }
        }
      ]
    }"#;
    let found = power_warnings(json);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["refs"][0], "#rload.p");
    assert_eq!(found[0]["t_ps"], 0);
    let peak = found[0]["data"]["peak"].as_f64().unwrap();
    assert!((peak - 2.5).abs() < 1e-3, "{peak}");
}
