use sim_core::diagnostic::Severity;

fn simulate(bytes: &[u8]) -> sim_core::run_folder::Results {
    let netlist = sim_core::netlist::parse(bytes).unwrap();
    sim_circuit::simulate(&netlist)
}

fn source_and_resistor(volts: &str, ohms: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "vin", "b": "gnd" }},
          "resistance": {{ "value": {ohms}, "unit": "ohm", "source": "user" }}
        }},
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vin", "n": "gnd" }},
          "voltage": {{ "value": {volts}, "unit": "V", "source": "user" }}
        }}
      ]
    }}"#
    )
}

#[test]
fn a_huge_voltage_across_one_ohm_is_nonfinite_at_the_resistor() {
    let results = simulate(source_and_resistor("1e200", "1.0").as_bytes());
    assert!(results.observations().op.is_none());
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
    assert_eq!(found.at(), Some(sim_core::time::Time::ZERO));
}

#[test]
fn a_huge_voltage_across_a_tiny_resistor_is_nonfinite_in_the_solution() {
    let results = simulate(source_and_resistor("1e200", "1e-300").as_bytes());
    assert!(results.observations().op.is_none());
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
            r#"{{"name":"r{index}","kind":"resistor","pins":{{"a":"{previous}","b":"{net}"}},"resistance":{{"value":1000.0,"unit":"ohm","source":"user"}}}}"#
        ));
        previous = net;
    }
    format!(
        r#"{{"version":0,"seed":0,"analysis":{{"op":{{}}}},"ground":"gnd","nets":[{}],"parts":[{}]}}"#,
        nets.join(","),
        parts.join(",")
    )
}

#[test]
fn a_chain_one_past_the_dense_limit_is_too_large() {
    let results = simulate(chain(4097).as_bytes());
    assert!(results.observations().op.is_none());
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
    assert_eq!(found.at(), Some(sim_core::time::Time::ZERO));
}
