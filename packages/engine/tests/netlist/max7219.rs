//! A `max7219` keeps its input thresholds, DOUT stage, VLED, draw and ratings.

use orbe_engine::netlist::{self, Basis, Chip, Model};

fn part() -> String {
    r#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vcc", "din", "clk", "load", "dout", "iset"],
      "parts": [{
        "name": "u1",
        "kind": "max7219",
        "pins": {
          "VCC": "vcc", "GND": "gnd", "DIN": "din", "CLK": "clk",
          "LOAD": "load", "DOUT": "dout", "ISET": "iset"
        },
        "input_threshold": {
          "low": { "value": 0.8, "unit": "V" },
          "high": { "value": 3.5, "unit": "V" }
        },
        "dout": {
          "high_drop": { "value": 0.0, "unit": "V" },
          "high_resistance": { "value": 1000.0, "unit": "ohm" },
          "low_drop": { "value": 0.0, "unit": "V" },
          "low_resistance": { "value": 250.0, "unit": "ohm" }
        },
        "led_forward_voltage": { "value": 1.85, "unit": "V" },
        "draw": {
          "shutdown_current": { "value": 0.00015, "unit": "A" },
          "logic_current": { "value": 0.008, "unit": "A" },
          "holds_above": { "value": 4.0, "unit": "V" }
        },
        "ratings": {
          "rail_margin": { "value": 0.3, "unit": "V" },
          "supply_voltage": { "value": 6.0, "unit": "V" },
          "input_voltage": { "value": 6.0, "unit": "V" },
          "supply_current": { "value": 0.5, "unit": "A", "basis": "peak" }
        }
      }]
    }"#
    .to_string()
}

#[test]
fn a_max7219_keeps_its_electrical_params_and_ratings() {
    let netlist = netlist::parse(part().as_bytes()).unwrap();
    let (_, parsed) = netlist.parts().next().unwrap();
    let Model::Chip(Chip::Max7219(dev)) = &parsed.model else {
        panic!("{:?}", parsed.model);
    };
    assert_eq!(
        parsed.model.pin_names(),
        ["VCC", "GND", "DIN", "CLK", "LOAD", "DOUT", "ISET"]
    );
    assert_eq!(dev.input_threshold.low.si.get(), 0.8);
    assert_eq!(dev.input_threshold.high.si.get(), 3.5);
    assert_eq!(dev.dout.high_resistance.si.get(), 1000.0);
    assert_eq!(dev.dout.low_resistance.si.get(), 250.0);
    assert_eq!(dev.led_forward_voltage.si.get(), 1.85);
    let draw = dev.draw.as_ref().unwrap();
    assert_eq!(draw.shutdown_current.si.get(), 0.00015);
    assert_eq!(draw.logic_current.si.get(), 0.008);
    assert_eq!(draw.holds_above.si.get(), 4.0);
    let ratings = dev.ratings.as_ref().unwrap();
    assert_eq!(ratings.rail_margin.as_ref().unwrap().si.get(), 0.3);
    assert_eq!(ratings.supply_voltage.as_ref().unwrap().si.get(), 6.0);
    assert_eq!(ratings.input_voltage.as_ref().unwrap().si.get(), 6.0);
    let sc = ratings.supply_current.as_ref().unwrap();
    assert_eq!(sc.si.get(), 0.5);
    assert_eq!(sc.basis, Some(Basis::Peak));
}

#[test]
fn inverted_threshold_is_rejected() {
    let json = part().replace("0.8", "4.0");
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    let text = format!("{err:?}");
    assert!(text.contains("input_threshold.low"), "{text}");
}
