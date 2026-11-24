//! An `ssd1306`: its pins, its address, 0x3C or 0x3D, and SDA's open-drain stage.

use orbe_engine::diagnostic::{Diagnostic, ValueRule};
use orbe_engine::netlist::{self, Chip, Model, engine_kinds};

fn chip(address: u32, low_drop: f64) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": 1000000000, "max_step_ps": 1000000, "grid_ps": 1000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vdd", "scl", "sda", "res"],
      "parts": [{{
        "name": "u1",
        "kind": "ssd1306",
        "pins": {{ "VDD": "vdd", "VSS": "gnd", "SCL": "scl", "SDA": "sda", "RES": "res" }},
        "address": {address},
        "low_drop": {{ "value": {low_drop}, "unit": "V" }},
        "low_resistance": {{ "value": 1000.0, "unit": "ohm" }},
        "input_threshold": {{
          "low": {{ "value": 0.2, "unit": "1" }},
          "high": {{ "value": 0.8, "unit": "1" }}
        }}
      }}]
    }}"#
    )
}

#[test]
fn a_part_keeps_its_address_and_its_stage() {
    let netlist = netlist::parse(chip(0x3c, 0.0).as_bytes()).unwrap();
    let Model::Chip(Chip::Ssd1306(dev)) = &netlist.parts().next().unwrap().1.model else {
        panic!("u1 is an ssd1306");
    };
    assert_eq!(dev.address, 0x3c);
    assert_eq!(dev.low_resistance.si.get(), 1000.0);
    assert_eq!(dev.input_threshold.high.si.get(), 0.8);
    let (_, pins, _) = engine_kinds()
        .iter()
        .find(|(name, _, _)| *name == "ssd1306")
        .expect("orbe check knows the kind");
    assert_eq!(*pins, ["VDD", "VSS", "SCL", "SDA", "RES"]);
}

#[test]
fn an_address_sa0_cannot_select_is_unsupported() {
    assert!(netlist::parse(chip(0x3d, 0.0).as_bytes()).is_ok());
    let err = netlist::parse(chip(0x3e, 0.0).as_bytes()).unwrap_err();
    assert!(err.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistValue { field, rule: ValueRule::Unsupported, .. } if field == "address"
    )));
}

#[test]
fn a_negative_low_drop_is_refused() {
    let err = netlist::parse(chip(0x3c, -0.1).as_bytes()).unwrap_err();
    assert!(err.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistValue { field, rule: ValueRule::NonNegative, .. } if field == "low_drop"
    )));
}
