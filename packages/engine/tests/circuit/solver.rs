use orbe_engine::diagnostic::Severity;

fn simulate(bytes: &[u8]) -> orbe_engine::run_folder::Results {
    let netlist = orbe_engine::netlist::parse(bytes).unwrap();
    orbe_engine::simulate(&netlist)
}

fn source_and_resistor(volts: &str, ohms: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "vin", "b": "gnd" }},
          "resistance": {{ "value": {ohms}, "unit": "ohm" }},
          "ratings": {{ "power": null }}
        }},
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vin", "n": "gnd" }},
          "voltage": {{ "value": {volts}, "unit": "V" }}
        }}
      ]
    }}"#
    )
}

#[test]
fn a_huge_voltage_across_one_ohm_is_nonfinite_at_the_resistor() {
    let results = simulate(source_and_resistor("1e200", "1.0").as_bytes());
    assert!(results.observations().op().is_none());
    assert_eq!(
        results.diagnostics().len(),
        1,
        "{:?}",
        results.diagnostics()
    );
    let found = &results.diagnostics()[0];
    assert_eq!(found.code(), "solver.nonfinite");
    assert_eq!(found.severity(), Severity::Error);
    assert_eq!(found.refs(), vec!["#r1".parse().unwrap()]);
    assert_eq!(found.at(), Some(orbe_model::Time::ZERO));
}

#[test]
fn a_huge_voltage_across_a_tiny_resistor_is_nonfinite_in_the_solution() {
    let results = simulate(source_and_resistor("1e200", "1e-300").as_bytes());
    assert!(results.observations().op().is_none());
    assert_eq!(
        results.diagnostics().len(),
        1,
        "{:?}",
        results.diagnostics()
    );
    let found = &results.diagnostics()[0];
    assert_eq!(found.code(), "solver.nonfinite");
    assert_eq!(found.refs(), vec!["#net.vin".parse().unwrap()]);
}

fn chain(signal_nets: usize) -> String {
    let mut nets = vec!["\"gnd\"".to_string()];
    let mut parts = Vec::new();
    let mut previous = "gnd".to_string();
    for index in 0..signal_nets {
        let net = format!("n{index}");
        nets.push(format!("\"{net}\""));
        parts.push(format!(
            r#"{{"name":"r{index}","kind":"resistor","pins":{{"a":"{previous}","b":"{net}"}},"resistance":{{"value":1000.0,"unit":"ohm"}},"ratings":{{"power":null}}}}"#
        ));
        previous = net;
    }
    format!(
        r#"{{"version":1,"seed":0,"analysis":{{"op":{{}}}},"ground":"gnd","nets":[{}],"parts":[{}]}}"#,
        nets.join(","),
        parts.join(",")
    )
}

#[test]
fn a_chain_one_past_the_solver_limit_is_too_large() {
    let results = simulate(chain(4097).as_bytes());
    assert!(results.observations().op().is_none());
    assert_eq!(
        results.diagnostics().len(),
        1,
        "{:?}",
        results.diagnostics()
    );
    let found = &results.diagnostics()[0];
    assert_eq!(found.code(), "solver.too_large");
    assert_eq!(found.severity(), Severity::Error);
    let text = serde_json::to_string(found).unwrap();
    assert!(
        text.contains(r#""data":{"limit":4096,"unknowns":4097}"#),
        "{text}"
    );
    assert_eq!(found.at(), Some(orbe_model::Time::ZERO));
}

/// `volts` on `vin`, `ohms` from `vin` to `a`, and a diode from `a` to ground, so every solve
/// is one of Newton's. `second` adds a second source on `vin`.
fn source_into_a_diode(volts: &str, ohms: &str, second: bool) -> String {
    let v2 = if second {
        r#",{"name":"v2","kind":"voltage_source","pins":{"p":"vin","n":"gnd"},"voltage":{"value":3.0,"unit":"V"}}"#
    } else {
        ""
    };
    format!(
        r#"{{"version":1,"seed":0,"analysis":{{"op":{{}}}},"ground":"gnd","nets":["gnd","vin","a"],"parts":[
          {{"name":"v1","kind":"voltage_source","pins":{{"p":"vin","n":"gnd"}},"voltage":{{"value":{volts},"unit":"V"}}}},
          {{"name":"r1","kind":"resistor","pins":{{"a":"vin","b":"a"}},"resistance":{{"value":{ohms},"unit":"ohm"}},"ratings":{{"power":null}}}},
          {{"name":"d1","kind":"diode","pins":{{"anode":"a","cathode":"gnd"}},
            "saturation_current":{{"value":1e-14,"unit":"A"}},"emission_coefficient":{{"value":1.0,"unit":"1"}},
            "series_resistance":{{"value":1.0,"unit":"ohm"}},"ratings":{{"current":null,"power":null}}}}{v2}]}}"#
    )
}

/// The one diagnostic `json`'s run gives, after checking it gave no operating point.
fn only_diagnostic(json: &str) -> orbe_engine::diagnostic::Diagnostic {
    let results = simulate(json.as_bytes());
    assert!(results.observations().op().is_none());
    let [found] = results.diagnostics() else {
        panic!("{:?}", results.diagnostics());
    };
    assert_eq!(found.severity(), Severity::Error);
    assert_eq!(found.at(), Some(orbe_model::Time::ZERO));
    found.clone()
}

#[test]
fn a_huge_voltage_across_a_tiny_resistor_into_a_diode_is_nonfinite_in_the_solution() {
    let found = only_diagnostic(&source_into_a_diode("1e200", "1e-300", false));
    assert_eq!(found.code(), "solver.nonfinite");
    assert_eq!(found.refs(), vec!["#net.vin".parse().unwrap()]);
}

#[test]
fn parallel_sources_beside_a_diode_are_singular_at_the_second_source() {
    let found = only_diagnostic(&source_into_a_diode("5.0", "1000.0", true));
    assert_eq!(found.code(), "solver.singular");
    assert_eq!(found.refs(), vec!["#v2".parse().unwrap()]);
}
