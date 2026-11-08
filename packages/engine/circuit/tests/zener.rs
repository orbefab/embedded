use orbe_engine_core::name::Ref;
use orbe_engine_core::run_folder::Results;

fn simulate(json: &str) -> Results {
    let netlist = orbe_engine_core::netlist::parse(json.as_bytes()).unwrap();
    orbe_circuit::simulate(&netlist)
}

fn value(results: &Results, reference: &str) -> f64 {
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let op = results.observations().op().expect("operating point");
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

fn forward_fields() -> &'static str {
    r#"
      "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
      "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
      "series_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
    "#
}

fn zener_fields() -> String {
    format!(
        r#"{forward}
          "zener_voltage": {{ "value": 5.0, "unit": "V", "source": "user" }},
          "zener_current": {{ "value": 0.05, "unit": "A", "source": "user" }},
          "zener_impedance": {{ "value": 10.0, "unit": "ohm", "source": "user" }},
          "breakdown_resistance": {{ "value": 4.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "current": null, "power": null }}
        "#,
        forward = forward_fields()
    )
}

fn diode_fields() -> String {
    format!(
        r#"{forward}
          "ratings": {{ "current": null, "power": null }}
        "#,
        forward = forward_fields()
    )
}

fn biased(kind: &str, name: &str, anode: &str, cathode: &str, volts: f64, fields: &str) -> String {
    format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "op": {{}} }},
          "ground": "gnd",
          "nets": ["gnd", "p"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "p", "n": "gnd" }},
              "voltage": {{ "value": {volts}, "unit": "V", "source": "user" }}
            }},
            {{
              "name": "{name}",
              "kind": "{kind}",
              "pins": {{ "anode": "{anode}", "cathode": "{cathode}" }},
              {fields}
            }}
          ]
        }}"#
    )
}

#[test]
fn zener_breakdown_flows_cathode_to_anode() {
    let results = simulate(&biased("zener", "dz1", "gnd", "p", 5.0, &zener_fields()));
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let i = value(&results, "#dz1.i");
    let v = value(&results, "#dz1.v");
    assert!(i < 0.0, "{i}");
    assert!(v < 0.0, "{v}");
    assert!((i + 0.05).abs() < 1e-6, "{i}");
    assert!((v + 5.0).abs() < 1e-6, "{v}");
}

#[test]
fn zener_breakdown_resistance_stays_off_the_forward_path() {
    let forward_z = simulate(&biased("zener", "dz1", "p", "gnd", 0.8, &zener_fields()));
    let forward_d = simulate(&biased("diode", "d1", "p", "gnd", 0.8, &diode_fields()));
    let iz = value(&forward_z, "#dz1.i");
    let id = value(&forward_d, "#d1.i");
    assert!(iz > 1e-3, "{iz}");
    assert!((iz - id).abs() / iz < 1e-6, "zener {iz} diode {id}");

    let reverse_z = simulate(&biased("zener", "dz1", "gnd", "p", 5.0, &zener_fields()));
    let reverse_d = simulate(&biased("diode", "d1", "gnd", "p", 5.0, &diode_fields()));
    let iz = value(&reverse_z, "#dz1.i").abs();
    let id = value(&reverse_d, "#d1.i").abs();
    assert!(iz > 1e-3, "{iz}");
    assert!(id < 1e-6, "{id}");
}

#[test]
fn a_net_a_diode_would_not_float_does_not_float_with_a_zener() {
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "op": {{}} }},
          "ground": "gnd",
          "nets": ["gnd", "top", "vin"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "vin", "n": "gnd" }},
              "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }}
            }},
            {{
              "name": "c1",
              "kind": "capacitor",
              "pins": {{ "a": "vin", "b": "top" }},
              "capacitance": {{ "value": 1e-6, "unit": "F", "source": "user" }},
              "initial": null
            }},
            {{
              "name": "dz1",
              "kind": "zener",
              "pins": {{ "anode": "top", "cathode": "gnd" }},
              {fields}
            }}
          ]
        }}"#,
        fields = zener_fields()
    );
    let results = simulate(&json);
    assert!(
        results
            .diagnostics()
            .iter()
            .all(|d| d.code() != "net.floating"),
        "{:?}",
        results.diagnostics()
    );
}

#[test]
fn zener_examples_solve() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let shunt = std::fs::read(root.join("packages/engine/tests/circuits/zener-shunt/netlist.json"))
        .unwrap();
    let shunt_net = orbe_engine_core::netlist::parse(&shunt).unwrap();
    let shunt_results = orbe_circuit::simulate(&shunt_net);
    assert!(shunt_results.diagnostics().is_empty());
    let op = shunt_results.observations().op().unwrap();
    let read = |name: &str| op.values[&name.parse::<Ref>().unwrap()].get();
    let cathode = read("#net.vz");
    let voltage = read("#dz1.v");
    let current = read("#dz1.i");
    let power = read("#dz1.p");
    eprintln!("zener-shunt #net.vz {cathode} #dz1.v {voltage} #dz1.i {current} #dz1.p {power}");
    assert_eq!(cathode, 4.996963715824736);
    assert_eq!(voltage, -4.996963715824736);
    assert_eq!(current, -0.03503339912093094);
    assert_eq!(power, 0.1750606242492981);
    let sweep = std::fs::read(root.join("packages/engine/tests/circuits/zener-sweep/netlist.json"))
        .unwrap();
    let sweep_net = orbe_engine_core::netlist::parse(&sweep).unwrap();
    let (sweep_results, stats) = orbe_circuit::simulate_stats(&sweep_net);
    assert!(
        sweep_results.diagnostics().is_empty(),
        "{:?}",
        sweep_results.diagnostics()
    );
    eprintln!(
        "zener-sweep landings {} stops {}",
        stats.landings, stats.stops
    );
    assert_eq!(stats.landings, 190);
    assert_eq!(stats.stops, 0);
}

#[test]
fn zener_overload_warns_on_current_and_power() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../cli/tests/fixtures/zener-overload/netlist.json");
    let bytes = std::fs::read(path).unwrap();
    let netlist = orbe_engine_core::netlist::parse(&bytes).unwrap();
    let results = orbe_circuit::simulate(&netlist);
    let found: Vec<(String, String)> = results
        .diagnostics()
        .iter()
        .map(|d| {
            (
                d.code().to_string(),
                d.refs()
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("rating.current".to_string(), "#dz1.i".to_string()),
            ("rating.power".to_string(), "#dz1.p".to_string()),
        ]
    );
}
