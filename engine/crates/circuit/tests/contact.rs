use sim_core::name::Ref;
use sim_core::run_folder::Results;

fn simulate(json: &str) -> Results {
    let netlist = sim_core::netlist::parse(json.as_bytes()).unwrap();
    sim_circuit::simulate(&netlist)
}

fn value(results: &Results, reference: &str) -> f64 {
    let op = results.observations().op().unwrap();
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

/// 5 V, then a switch from `vin` to `vout`, then `load` from `vout` to ground.
fn switched(initial: &str, load: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "vin", "vout"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vin", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }}
        }},
        {{
          "name": "sw1",
          "kind": "switch",
          "pins": {{ "a": "vin", "b": "vout" }},
          "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "{initial}",
          "events": [{{ "t_ps": 1000000, "state": "{flipped}" }}],
          "bounce": null
        }},
        {load}
      ]
    }}"#,
        flipped = if initial == "open" { "closed" } else { "open" },
    )
}

const RESISTOR: &str = r#"{
  "name": "r1",
  "kind": "resistor",
  "pins": { "a": "vout", "b": "gnd" },
  "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
  "ratings": { "power": null }
}"#;

const CAPACITOR: &str = r#"{
  "name": "c1",
  "kind": "capacitor",
  "pins": { "a": "vout", "b": "gnd" },
  "capacitance": { "value": 1e-6, "unit": "F", "source": "user" },
  "initial": null
}"#;

#[test]
fn a_switch_holds_its_initial_contact_at_the_operating_point() {
    let closed = simulate(&switched("closed", RESISTOR));
    assert!(
        closed.diagnostics().is_empty(),
        "{:?}",
        closed.diagnostics()
    );
    let expect = 5.0 * 1000.0 / 1000.1;
    let vout = value(&closed, "#net.vout");
    assert!((vout - expect).abs() <= 1e-9, "{vout} vs {expect}");
    let i = value(&closed, "#sw1.i");
    assert!((i - expect / 1000.0).abs() <= 1e-10, "{i}");

    let open = simulate(&switched("open", RESISTOR));
    let expect = 5.0 * 1000.0 / (1e9 + 1000.0);
    let vout = value(&open, "#net.vout");
    assert!((vout - expect).abs() <= 1e-9, "{vout} vs {expect}");
}

#[test]
fn a_capacitor_is_open_at_the_operating_point() {
    let results = simulate(&switched("closed", CAPACITOR));
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    assert_eq!(value(&results, "#c1.i"), 0.0);
    assert_eq!(value(&results, "#c1.p"), 0.0);
    let vout = value(&results, "#net.vout");
    assert!((vout - 5.0).abs() <= 1e-9, "{vout}");
}

#[test]
fn a_net_reached_only_through_a_capacitor_floats() {
    let isolated = switched("closed", CAPACITOR)
        .replace(
            r#""nets": ["gnd", "vin", "vout"]"#,
            r#""nets": ["gnd", "vin", "vout", "top"]"#,
        )
        .replace(
            r#""pins": { "a": "vout", "b": "gnd" }"#,
            r#""pins": { "a": "top", "b": "vout" }"#,
        );
    let results = simulate(&isolated);
    let codes: Vec<_> = results
        .diagnostics()
        .iter()
        .map(|d| (d.code(), d.refs()[0].to_string()))
        .collect();
    assert_eq!(codes, [("net.floating", "#net.top".to_string())]);
}

#[test]
fn a_held_capacitor_is_a_voltage_at_the_operating_point() {
    let held = CAPACITOR.replace(
        r#""initial": null"#,
        r#""initial": { "value": 2.0, "unit": "V", "source": "user" }"#,
    );
    let results = simulate(&switched("closed", &held));
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let i = value(&results, "#c1.i");
    let expect = (5.0 - 2.0) / 0.1;
    assert!((i - expect).abs() <= 1e-6 * expect, "{i} vs {expect}");
    // The hold sits behind 1 fs / 1 uF ohm, so 30 A moves it by 30 nV.
    let vout = value(&results, "#net.vout");
    assert!((vout - 2.0).abs() <= i * 1e-15 / 1e-6 + 1e-12, "{vout}");
}

#[test]
fn a_net_reached_only_through_a_held_capacitor_has_a_dc_path() {
    let isolated = switched("closed", CAPACITOR)
        .replace(
            r#""nets": ["gnd", "vin", "vout"]"#,
            r#""nets": ["gnd", "vin", "vout", "top"]"#,
        )
        .replace(
            r#""pins": { "a": "vout", "b": "gnd" }"#,
            r#""pins": { "a": "top", "b": "vout" }"#,
        )
        .replace(
            r#""initial": null"#,
            r#""initial": { "value": 1.0, "unit": "V", "source": "user" }"#,
        );
    let results = simulate(&isolated);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let top = value(&results, "#net.top");
    assert!((top - 6.0).abs() <= 1e-9, "{top}");
}
