use orbe_engine::diagnostic::{Diagnostic, ValueRule};
use orbe_engine::name::Quantity;
use orbe_engine::netlist::{Model, parse};
use orbe_engine::world::{Curve, Param};

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
  "output_resistance": {
    "kind": "world",
    "quantity": "temperature",
    "curve": {
      "kind": "step",
      "x": { "value": 105.0, "unit": "degC" },
      "below": { "value": 60.0, "unit": "ohm" },
      "above": { "value": 250.0, "unit": "ohm" }
    }
  },
  "quiescent_current": { "value": 5e-5, "unit": "A" },
  "ratings": {
    "supply_min": { "value": 2.7, "unit": "V" },
    "supply_max": { "value": 5.5, "unit": "V" },
    "supply_voltage": { "value": 7.0, "unit": "V" }
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
    let Param::World(driven) = &sensor.output_resistance else {
        panic!("expected world-driven output_resistance");
    };
    assert!(matches!(driven.curve, Curve::Step { .. }));
    assert_eq!(sensor.quiescent_current.si.get(), 5e-5);
    assert_eq!(sensor.supply_min.as_ref().unwrap().si.get(), 2.7);
    assert_eq!(sensor.supply_max.as_ref().unwrap().si.get(), 5.5);
    assert_eq!(sensor.max_supply.as_ref().unwrap().si.get(), 7.0);
}

#[test]
fn ratings_are_optional_and_fixed_output_voltage_and_resistance_work() {
    let body = r#"
      "output_voltage": { "value": 0.75, "unit": "V" },
      "output_resistance": { "value": 60.0, "unit": "ohm" },
      "quiescent_current": { "value": 0.0, "unit": "A" }
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
    assert_eq!(sensor.output_resistance.fixed().unwrap().si.get(), 60.0);
    assert_eq!(sensor.supply_min, None);
    assert_eq!(sensor.supply_max, None);
    assert_eq!(sensor.max_supply, None);
}

#[test]
fn bad_units_and_values_are_rejected() {
    let bad = r#"
      "output_voltage": { "value": 0.75, "unit": "V" },
      "output_resistance": { "value": 60.0, "unit": "V" },
      "quiescent_current": { "value": -1e-5, "unit": "A" },
      "ratings": {
        "supply_min": { "value": 0.0, "unit": "V" },
        "supply_max": { "value": 5.5, "unit": "V" },
        "supply_voltage": { "value": -7.0, "unit": "V" }
      }
    "#;
    let found = parse(doc(bad, "").as_bytes()).unwrap_err();
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

    let inverted_supply = VALID_BODY
        .replace(
            r#""supply_min": { "value": 2.7"#,
            r#""supply_min": { "value": 6.0"#,
        )
        .replace(
            r#""supply_max": { "value": 5.5"#,
            r#""supply_max": { "value": 2.7"#,
        );
    let found = parse(doc(&inverted_supply, WORLD_TEMP).as_bytes()).unwrap_err();
    assert!(found.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistValue {
            field,
            rule: ValueRule::AboveHigh,
            ..
        } if field == "ratings.supply_min"
    )));
}
