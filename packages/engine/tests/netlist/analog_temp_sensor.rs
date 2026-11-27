use orbe_engine::diagnostic::{Diagnostic, ValueRule};
use orbe_engine::name::Quantity;
use orbe_engine::netlist::{Model, parse};

fn doc(part_body: &str, world: &str) -> String {
    format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "op": {{}} }},
          "ground": "gnd",
          "nets": ["gnd", "vs", "vout"],
          {world}
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vs", "n": "gnd" }},
              "voltage": {{ "value": 5.0, "unit": "V" }} }},
            {{
              "name": "u1",
              "kind": "analog_temp_sensor",
              "pins": {{ "vs": "vs", "vout": "vout", "gnd": "gnd" }},
              {part_body}
            }}
          ]
        }}"#
    )
}

const WORLD_TEMP: &str = r#""world": { "temperature": { "value": 25.0, "unit": "degC" } },"#;

const VALID_BODY: &str = r#"
  "output_voltage": {
    "kind": "world",
    "quantity": "temperature",
    "curve": {
      "kind": "linear",
      "x0": { "value": 25.0, "unit": "degC" },
      "y0": { "value": 0.75, "unit": "V" },
      "x1": { "value": 125.0, "unit": "degC" },
      "y1": { "value": 1.75, "unit": "V" }
    }
  },
  "output_resistance": { "value": 60.0, "unit": "ohm" },
  "quiescent_current": { "value": 5e-5, "unit": "A" },
  "min_supply": { "value": 2.7, "unit": "V" },
  "ratings": {
    "supply_voltage": { "value": 7.0, "unit": "V" },
    "temperature": {
      "min": { "value": -40.0, "unit": "degC" },
      "max": { "value": 125.0, "unit": "degC" }
    }
  }
"#;

#[test]
fn valid_analog_temp_sensor_parses_pins_quantities_and_ratings() {
    let netlist = parse(doc(VALID_BODY, WORLD_TEMP).as_bytes()).unwrap();
    let part = netlist
        .parts()
        .find(|(_, p)| p.name.as_str() == "u1")
        .unwrap()
        .1;
    assert_eq!(part.model.pin_names(), &["vs", "vout", "gnd"]);
    assert_eq!(
        part.model.quantities(),
        &[Quantity::Iin, Quantity::Iout, Quantity::P]
    );
    let Model::AnalogTempSensor(sensor) = &part.model else {
        panic!("expected AnalogTempSensor");
    };
    assert_eq!(sensor.output_resistance.si.get(), 60.0);
    assert_eq!(sensor.quiescent_current.si.get(), 5e-5);
    assert_eq!(sensor.min_supply.si.get(), 2.7);
    assert_eq!(sensor.max_supply.as_ref().unwrap().si.get(), 7.0);
    assert_eq!(sensor.temperature, Some((-40.0, 125.0)));
}

#[test]
fn ratings_are_optional_and_fixed_output_voltage_works() {
    let body = r#"
      "output_voltage": { "value": 0.75, "unit": "V" },
      "output_resistance": { "value": 60.0, "unit": "ohm" },
      "quiescent_current": { "value": 0.0, "unit": "A" },
      "min_supply": { "value": 2.7, "unit": "V" }
    "#;
    let netlist = parse(doc(body, "").as_bytes()).unwrap();
    let Model::AnalogTempSensor(sensor) = &netlist
        .parts()
        .find(|(_, p)| p.name.as_str() == "u1")
        .unwrap()
        .1
        .model
    else {
        panic!("expected AnalogTempSensor");
    };
    assert_eq!(sensor.max_supply, None);
    assert_eq!(sensor.temperature, None);
}

#[test]
fn bad_units_and_values_are_rejected() {
    let bad = VALID_BODY
        .replace(r#""unit": "ohm""#, r#""unit": "V""#)
        .replace("5e-5", "-1e-5")
        .replace("2.7", "0.0")
        .replace("7.0", "-7.0");
    let found = parse(doc(&bad, WORLD_TEMP).as_bytes()).unwrap_err();
    let codes: Vec<_> = found.iter().map(Diagnostic::code).collect();
    assert_eq!(
        codes,
        vec![
            "netlist.unit",
            "netlist.value",
            "netlist.value",
            "netlist.value"
        ]
    );

    let inverted_temp = VALID_BODY
        .replace("-40.0", "130.0")
        .replace(r#""max": { "value": 125.0"#, r#""max": { "value": 20.0"#);
    let found = parse(doc(&inverted_temp, WORLD_TEMP).as_bytes()).unwrap_err();
    assert!(found.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistValue {
            field,
            rule: ValueRule::AboveHigh,
            ..
        } if field == "ratings.temperature.min"
    )));
}
