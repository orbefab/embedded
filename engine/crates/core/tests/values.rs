use sim_core::netlist::{self, Model, Shape};

#[test]
fn resistance_with_a_non_finite_reciprocal_is_a_value_error() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd" },
          "resistance": { "value": 1e-320, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.value");
    assert_eq!(err[0].refs(), vec!["#r1".parse().unwrap()]);
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""rule":"finite_reciprocal""#), "{text}");
}

#[test]
fn a_very_small_finite_conductance_parses() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd" },
          "resistance": { "value": 1e-300, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    assert!(netlist::parse(json).is_ok());
}

#[test]
fn a_number_past_the_f64_range_is_syntax() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 1e309, "unit": "V", "source": "user" }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn negative_voltage_is_legal() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": -5.0, "unit": "V", "source": "user" }
        }
      ]
    }"#;
    let netlist = netlist::parse(json).unwrap();
    let (_, v1) = netlist.parts().next().unwrap();
    match &v1.model {
        Model::VoltageSource(v) => {
            assert!(matches!(
                v.voltage.shape(),
                Shape::Steps(schedule)
                    if schedule.events().is_empty() && schedule.initial().si.get() == -5.0
            ))
        }
        other => panic!("{other:?}"),
    }
}

fn diode_at(is: &str, n: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "a", "b"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "a", "n": "gnd" }},
          "voltage": {{ "value": 1.0, "unit": "V", "source": "user" }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "a", "b": "b" }},
          "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "power": null }}
        }},
        {{
          "name": "d1",
          "kind": "diode",
          "pins": {{ "anode": "b", "cathode": "gnd" }},
          "saturation_current": {{ "value": {is}, "unit": "A", "source": "user" }},
          "emission_coefficient": {{ "value": {n}, "unit": "1", "source": "user" }},
          "series_resistance": {{ "value": 1.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "current": null, "power": null }}
        }}
      ]
    }}"#
    )
}

fn junction_value_error(json: &str) -> String {
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""code":"netlist.value""#), "{text}");
    assert!(text.contains(r#""rule":"finite_junction""#), "{text}");
    assert!(
        text.contains("n*VT must be normal, and vcrit must be finite and positive"),
        "{text}"
    );
    text
}

#[test]
fn a_junction_with_a_non_finite_vcrit_is_a_value_error() {
    let text = junction_value_error(&diode_at("1e-308", "100"));
    assert!(text.contains(r#""field":"saturation_current""#), "{text}");
    assert!(text.contains(r##""refs":["#d1"]"##), "{text}");
}

#[test]
fn a_huge_emission_coefficient_is_a_value_error() {
    let text = junction_value_error(&diode_at("1e-14", "1e308"));
    assert!(text.contains(r#""field":"emission_coefficient""#), "{text}");
}

#[test]
fn a_tiny_emission_coefficient_is_a_value_error() {
    let text = junction_value_error(&diode_at("1e-14", "1e-300"));
    assert!(text.contains(r#""field":"emission_coefficient""#), "{text}");
}

#[test]
fn a_subnormal_saturation_current_prints_like_its_data_number() {
    let text = junction_value_error(&diode_at("1e-308", "100"));
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let number = doc["data"]["value"].to_string();
    let message = doc["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("is {number};")),
        "{message} vs {number}"
    );
    assert_eq!(number, "1e-308");
}

#[test]
fn a_junction_just_inside_the_domain_parses() {
    let netlist = netlist::parse(diode_at("1e-308", "90").as_bytes()).unwrap();
    let (_, diode) = netlist
        .parts()
        .find(|(_, part)| part.name.as_str() == "d1")
        .unwrap();
    assert!(matches!(diode.model, Model::Diode(_)), "{:?}", diode.model);
}
