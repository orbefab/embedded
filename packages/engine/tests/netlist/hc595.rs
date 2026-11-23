//! An `hc595` keeps its stages, its thresholds and its current ratings.

use orbe_engine::netlist::{self, Chip, Model};

fn part() -> String {
    r#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vcc", "ser", "srclk", "rclk", "qa"],
      "parts": [{
        "name": "u1",
        "kind": "hc595",
        "pins": {
          "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd", "QH": "gnd",
          "GND": "gnd", "QHp": "gnd", "SRCLR": "vcc", "SRCLK": "srclk", "RCLK": "rclk",
          "OE": "gnd", "SER": "ser", "QA": "qa", "VCC": "vcc"
        },
        "output": {
          "high_drop": { "value": 0.0, "unit": "V" },
          "high_resistance": { "value": 33.333333333333336, "unit": "ohm" },
          "low_drop": { "value": 0.0, "unit": "V" },
          "low_resistance": { "value": 28.333333333333332, "unit": "ohm" }
        },
        "serial_output": {
          "high_drop": { "value": 0.0, "unit": "V" },
          "high_resistance": { "value": 50.0, "unit": "ohm" },
          "low_drop": { "value": 0.0, "unit": "V" },
          "low_resistance": { "value": 42.5, "unit": "ohm" }
        },
        "input_threshold": {
          "low": { "value": 0.3, "unit": "1" },
          "high": { "value": 0.7, "unit": "1" }
        },
        "ratings": {
          "pin_current": { "value": 0.035, "unit": "A" },
          "supply_current": { "value": 0.07, "unit": "A" },
          "input_margin": { "value": 0.5, "unit": "V" },
          "output_margin": { "value": 0.5, "unit": "V" }
        }
      }]
    }"#
    .to_string()
}

#[test]
fn an_hc595_keeps_its_drive_and_its_ratings() {
    let netlist = netlist::parse(part().as_bytes()).unwrap();
    let (_, parsed) = netlist.parts().next().unwrap();
    let Model::Chip(Chip::Hc595(dev)) = &parsed.model else {
        panic!("{:?}", parsed.model);
    };
    assert_eq!(
        parsed.model.pin_names(),
        [
            "QB", "QC", "QD", "QE", "QF", "QG", "QH", "GND", "QHp", "SRCLR", "SRCLK", "RCLK", "OE",
            "SER", "QA", "VCC"
        ]
    );
    assert_eq!(dev.output.high_resistance.si.get(), 33.333333333333336);
    assert_eq!(dev.serial_output.low_resistance.si.get(), 42.5);
    assert_eq!(dev.input_threshold.low.si.get(), 0.3);
    assert_eq!(dev.input_threshold.high.si.get(), 0.7);
    assert_eq!(dev.ratings.pin_current.as_ref().unwrap().si.get(), 0.035);
    assert_eq!(dev.ratings.supply_current.as_ref().unwrap().si.get(), 0.07);
    assert_eq!(dev.ratings.input_margin.as_ref().unwrap().si.get(), 0.5);
    assert_eq!(dev.ratings.output_margin.as_ref().unwrap().si.get(), 0.5);
}

#[test]
fn a_threshold_above_one_is_rejected() {
    let json = part().replace("0.7", "1.2");
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    let text = format!("{err:?}");
    assert!(text.contains("input_threshold.high"), "{text}");
}
