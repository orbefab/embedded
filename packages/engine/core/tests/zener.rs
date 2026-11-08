use orbe_engine_core::netlist::{self, Model};

fn zener(fields: &str) -> String {
    format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "op": {{}} }},
          "ground": "gnd",
          "nets": ["gnd", "a"],
          "parts": [
            {{
              "name": "dz1",
              "kind": "zener",
              "pins": {{ "anode": "a", "cathode": "gnd" }},
              {fields}
            }}
          ]
        }}"#
    )
}

fn row(vz: &str, zzt: &str, rb: &str) -> String {
    zener(&format!(
        r#"
          "saturation_current": {{ "value": 1e-14, "unit": "A", "source": "user" }},
          "emission_coefficient": {{ "value": 1.0, "unit": "1", "source": "user" }},
          "series_resistance": {{ "value": 1.0, "unit": "ohm", "source": "user" }},
          "zener_voltage": {{ "value": {vz}, "unit": "V", "source": "user" }},
          "zener_current": {{ "value": 0.05, "unit": "A", "source": "user" }},
          "zener_impedance": {{ "value": {zzt}, "unit": "ohm", "source": "user" }},
          "breakdown_resistance": {{ "value": {rb}, "unit": "ohm", "source": "user" }},
          "ratings": {{ "current": null, "power": null }}
        "#
    ))
}

fn codes(json: &str) -> Vec<String> {
    netlist::parse(json.as_bytes())
        .unwrap_err()
        .iter()
        .map(|d| {
            let text = serde_json::to_string(d).unwrap();
            let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
            format!(
                "{} {} {}",
                doc["code"].as_str().unwrap(),
                doc["data"]["field"].as_str().unwrap_or(""),
                doc["data"]["rule"].as_str().unwrap_or("")
            )
        })
        .collect()
}

#[test]
fn zener_matches_its_row() {
    let netlist = netlist::parse(row("5.1", "7.0", "4.5").as_bytes()).unwrap();
    let Model::Zener(zener) = &netlist.parts().next().unwrap().1.model else {
        panic!("not a zener");
    };
    let rs = zener.series_resistance.si.get();
    let rb = zener.breakdown_resistance.si.get();
    let vz = zener.zener_voltage.si.get();
    let izt = zener.zener_current.si.get();
    let zzt = zener.zener_impedance.si.get();
    let voltage = zener.breakdown.nvt * libm::log1p(izt / zener.breakdown.saturation_current)
        + izt * (rs + rb);
    let slope = rs + rb + zener.breakdown.nvt / izt;
    assert!((voltage - vz).abs() < 1e-9, "voltage {voltage} is not {vz}");
    assert!((slope - zzt).abs() < 1e-9, "slope {slope} is not {zzt}");
}

#[test]
fn an_impedance_that_does_not_clear_the_resistances_is_above_resistance() {
    let found = codes(&row("5.1", "5.0", "4.0"));
    assert_eq!(found, ["netlist.value zener_impedance above_resistance"]);
}

#[test]
fn a_voltage_that_does_not_clear_the_resistive_drop_is_above_drop() {
    let found = codes(&row("0.25", "10.0", "4.0"));
    assert_eq!(found, ["netlist.value zener_voltage above_drop"]);
}

#[test]
fn both_breakdown_inequalities_report_and_finite_junction_does_not() {
    let found = codes(&row("0.2", "5.0", "4.0"));
    assert_eq!(
        found,
        [
            "netlist.value zener_impedance above_resistance",
            "netlist.value zener_voltage above_drop"
        ]
    );
}

#[test]
fn a_sharp_knee_names_isb() {
    let json = zener(
        r#"
          "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
          "series_resistance": { "value": 0.35, "unit": "ohm", "source": "user" },
          "zener_voltage": { "value": 12.0, "unit": "V", "source": "user" },
          "zener_current": { "value": 0.021, "unit": "A", "source": "user" },
          "zener_impedance": { "value": 9.0, "unit": "ohm", "source": "user" },
          "breakdown_resistance": { "value": 8.0, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null }
        "#,
    );
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""field":"zener_voltage""#), "{text}");
    assert!(text.contains(r#""rule":"finite_junction""#), "{text}");
    assert!(
        text.contains("Isb must be normal and positive, and vcrit must be finite and positive"),
        "{text}"
    );
    assert!(!text.contains("n*VT"), "{text}");
}

#[test]
fn a_breakdown_that_overflows_names_zener_voltage() {
    let json = zener(
        r#"
          "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
          "series_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
          "zener_voltage": { "value": 100.0, "unit": "V", "source": "user" },
          "zener_current": { "value": 1.0, "unit": "A", "source": "user" },
          "zener_impedance": { "value": 2.0000000001, "unit": "ohm", "source": "user" },
          "breakdown_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null }
        "#,
    );
    let found = codes(&json);
    assert_eq!(found, ["netlist.value zener_voltage finite_junction"]);
}

#[test]
fn each_zener_field_names_its_unit_and_a_rating_names_its_dotted_field() {
    let json = zener(
        r#"
          "saturation_current": { "value": 1e-14, "unit": "V", "source": "user" },
          "emission_coefficient": { "value": 1.0, "unit": "A", "source": "user" },
          "series_resistance": { "value": 1.0, "unit": "V", "source": "user" },
          "zener_voltage": { "value": 5.0, "unit": "ohm", "source": "user" },
          "zener_current": { "value": 0.05, "unit": "V", "source": "user" },
          "zener_impedance": { "value": 10.0, "unit": "A", "source": "user" },
          "breakdown_resistance": { "value": 4.0, "unit": "V", "source": "user" },
          "ratings": {
            "current": { "value": 0.178, "unit": "ohm", "source": "user" },
            "power": { "value": 1.3, "unit": "A", "source": "user" }
          }
        "#,
    );
    let found = codes(&json);
    assert_eq!(
        found,
        [
            "netlist.unit saturation_current ",
            "netlist.unit emission_coefficient ",
            "netlist.unit series_resistance ",
            "netlist.unit zener_voltage ",
            "netlist.unit zener_current ",
            "netlist.unit zener_impedance ",
            "netlist.unit breakdown_resistance ",
            "netlist.unit ratings.current ",
            "netlist.unit ratings.power "
        ]
    );
}

#[test]
fn a_non_positive_breakdown_resistance_is_a_value_error() {
    let found = codes(&row("5.1", "10.0", "0.0"));
    assert_eq!(found, ["netlist.value breakdown_resistance positive"]);
}
