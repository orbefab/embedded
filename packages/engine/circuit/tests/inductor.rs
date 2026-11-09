use orbe_engine_core::name::Ref;
use orbe_engine_core::run_folder::Results;

fn simulate(json: &str) -> Results {
    let netlist = orbe_engine_core::netlist::parse(json.as_bytes()).unwrap();
    orbe_circuit::simulate(&netlist)
}

fn source(volts: f64) -> String {
    format!(
        r#"{{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": {volts:?}, "unit": "V", "source": "user" }} }}"#
    )
}

fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "resistor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "resistance": {{ "value": {ohm:?}, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }}"#
    )
}

fn inductor(name: &str, a: &str, b: &str, henry: f64, ohm: f64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "inductor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "inductance": {{ "value": {henry:?}, "unit": "H", "source": "user" }},
          "series_resistance": {{ "value": {ohm:?}, "unit": "ohm", "source": "user" }} }}"#
    )
}

fn op(nets: &str, parts: &[&str]) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": [{nets}],
      "parts": [{}]
    }}"#,
        parts.join(",")
    )
}

/// 5 V through the coil's 70 ohm and 30 ohm more. The inductance drops nothing.
#[test]
fn the_operating_point_is_a_short_behind_the_series_resistance() {
    let json = op(
        r#""gnd", "vcc", "mid""#,
        &[
            &source(5.0),
            &inductor("l1", "vcc", "mid", 0.505, 70.0),
            &resistor("r1", "mid", "gnd", 30.0),
        ],
    );
    let results = simulate(&json);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let op = results.observations().op().unwrap();
    let value = |reference: &str| op.values[&reference.parse::<Ref>().unwrap()].get();
    assert!((value("#l1.i") - 0.05).abs() <= 1e-12, "{}", value("#l1.i"));
    assert!((value("#l1.v") - 3.5).abs() <= 1e-9, "{}", value("#l1.v"));
    assert!((value("#net.mid") - 1.5).abs() <= 1e-9);
}

/// A capacitor alone would leave `top` floating. The inductor joins it to the supply.
#[test]
fn a_net_reached_only_through_an_inductor_has_a_dc_path() {
    let capacitor = r#"{ "name": "c1", "kind": "capacitor", "pins": { "a": "top", "b": "gnd" },
          "capacitance": { "value": 1e-6, "unit": "F", "source": "user" }, "initial": null }"#;
    let joined = op(
        r#""gnd", "vcc", "top""#,
        &[
            &source(5.0),
            &inductor("l1", "vcc", "top", 1e-3, 1.0),
            capacitor,
        ],
    );
    let results = simulate(&joined);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let floating = op(r#""gnd", "vcc", "top""#, &[&source(5.0), capacitor]);
    let codes: Vec<_> = simulate(&floating)
        .diagnostics()
        .iter()
        .map(|d| (d.code(), d.refs()[0].to_string()))
        .collect();
    assert_eq!(codes, [("net.floating", "#net.top".to_string())]);
}

/// `inductor-open-end` is an 0805 coil, 10 uH and 0.5 ohm, from a 5 V rail with no capacitor
/// into a net that draws nothing, the way an Uno's L2 feeds AVCC. A switch loads the rail with
/// an LED from 1 ms to 3 ms. ngspice-42 accepts 76 timepoints on its reference deck run with
/// the print step and the step cap both at the stop, and 106 at reltol=1e-9. The engine is held
/// to 1.5 times the 76.
#[test]
fn a_coil_into_an_open_net_steps_about_as_often_as_ngspice() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/circuits/inductor-open-end/netlist.json");
    let bytes = std::fs::read(&path).unwrap();
    let netlist = orbe_engine_core::netlist::parse(&bytes).unwrap();
    let (results, stats) = orbe_circuit::simulate_stats(&netlist);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let bound = 76 * 3 / 2;
    assert!(
        stats.landings <= bound,
        "{} steps, bound {bound}",
        stats.landings
    );
}
