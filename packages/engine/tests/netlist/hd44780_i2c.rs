//! An `hd44780_i2c`: its pins, address 0x20..=0x27, backlight jumper, and SDA's open-drain stage.

use orbe_engine::diagnostic::{Diagnostic, ValueRule};
use orbe_engine::netlist::{self, Chip, Model, engine_kinds};

fn chip(address: u32, backlight_jumper: bool, low_drop: f64) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": 1000000000, "max_step_ps": 1000000, "grid_ps": 1000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "scl", "sda"],
      "parts": [{{
        "name": "u1",
        "kind": "hd44780_i2c",
        "pins": {{ "VCC": "vcc", "GND": "gnd", "SCL": "scl", "SDA": "sda" }},
        "address": {address},
        "backlight_jumper": {backlight_jumper},
        "low_drop": {{ "value": {low_drop}, "unit": "V" }},
        "low_resistance": {{ "value": 133.33333333333334, "unit": "ohm" }},
        "input_threshold": {{
          "low": {{ "value": 0.3, "unit": "1" }},
          "high": {{ "value": 0.7, "unit": "1" }}
        }}
      }}]
    }}"#
    )
}

#[test]
fn a_part_keeps_its_address_jumper_and_stage() {
    let netlist = netlist::parse(chip(0x27, true, 0.0).as_bytes()).unwrap();
    let Model::Chip(Chip::Hd44780I2c(dev)) = &netlist.parts().next().unwrap().1.model else {
        panic!("u1 is an hd44780_i2c");
    };
    assert_eq!(dev.address, 0x27);
    assert!(dev.backlight_jumper);
    assert_eq!(dev.input_threshold.low.si.get(), 0.3);
    assert_eq!(dev.input_threshold.high.si.get(), 0.7);
    let (_, pins, _) = engine_kinds()
        .iter()
        .find(|(name, _, _)| *name == "hd44780_i2c")
        .expect("orbe check knows the kind");
    assert_eq!(*pins, ["VCC", "GND", "SCL", "SDA"]);
}

#[test]
fn addresses_outside_pcf8574_range_are_unsupported() {
    for addr in 0x20..=0x27 {
        assert!(netlist::parse(chip(addr, false, 0.0).as_bytes()).is_ok());
    }
    for bad in [0x1f, 0x28, 0x3c] {
        let err = netlist::parse(chip(bad, true, 0.0).as_bytes()).unwrap_err();
        assert!(err.iter().any(|d| matches!(
            d,
            Diagnostic::NetlistValue { field, rule: ValueRule::Unsupported, .. } if field == "address"
        )));
    }
}

#[test]
fn a_negative_low_drop_is_refused() {
    let err = netlist::parse(chip(0x27, true, -0.1).as_bytes()).unwrap_err();
    assert!(err.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistValue { field, rule: ValueRule::NonNegative, .. } if field == "low_drop"
    )));
}
