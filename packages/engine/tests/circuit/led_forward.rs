//! Each 3 mm and 5 mm LED colour is its own part. Driven at its datasheet test current, it
//! sits within 2% of that sheet's typical forward voltage. The diode parameters are the ones
//! that part's model sets.

use std::path::PathBuf;

use jsonc_parser::{ParseOptions, parse_to_serde_value};
use orbe_engine::name::Ref;
use serde_json::{Value, json};

struct Expect {
    part: &'static str,
    milliamps: f64,
    typical: f64,
}

const PARTS: &[Expect] = &[
    Expect {
        part: "led-5mm",
        milliamps: 10.0,
        typical: 1.9,
    },
    Expect {
        part: "led-5mm-yellow",
        milliamps: 10.0,
        typical: 1.95,
    },
    Expect {
        part: "led-5mm-green",
        milliamps: 10.0,
        typical: 2.0,
    },
    Expect {
        part: "led-5mm-blue",
        milliamps: 20.0,
        typical: 3.3,
    },
    Expect {
        part: "led-5mm-white",
        milliamps: 20.0,
        typical: 3.3,
    },
    Expect {
        part: "led-3mm",
        milliamps: 10.0,
        typical: 1.9,
    },
    Expect {
        part: "led-3mm-yellow",
        milliamps: 10.0,
        typical: 1.95,
    },
    Expect {
        part: "led-3mm-green",
        milliamps: 10.0,
        typical: 2.0,
    },
    Expect {
        part: "led-3mm-blue",
        milliamps: 20.0,
        typical: 3.3,
    },
    Expect {
        part: "led-3mm-white",
        milliamps: 20.0,
        typical: 3.3,
    },
];

fn part_file(id: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../parts")
        .join(id)
        .join("part.jsonc");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path:?}: {err}"));
    parse_to_serde_value(&text, &ParseOptions::default())
        .unwrap_or_else(|err| panic!("{path:?}: {err}"))
}

fn model_number(model: &Value, field: &str) -> f64 {
    model[field]["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{field} in {model}"))
}

fn forward(is: f64, n: f64, rs: f64, amps: f64) -> f64 {
    let bytes = serde_json::to_vec(&json!({
        "version": 1,
        "seed": 0,
        "analysis": { "op": {} },
        "ground": "gnd",
        "nets": ["gnd", "a"],
        "parts": [
            {
                "name": "i1",
                "kind": "current_source",
                "pins": { "p": "a", "n": "gnd" },
                "current": { "value": amps, "unit": "A" }
            },
            {
                "name": "led1",
                "kind": "led",
                "pins": { "anode": "a", "cathode": "gnd" },
                "saturation_current": { "value": is, "unit": "A" },
                "emission_coefficient": { "value": n, "unit": "1" },
                "series_resistance": { "value": rs, "unit": "ohm" },
                "ratings": { "current": null, "power": null }
            }
        ]
    }))
    .unwrap();
    let netlist = orbe_engine::netlist::parse(&bytes).unwrap();
    let results = orbe_engine::simulate(&netlist);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let op = results.observations().op().expect("operating point");
    let i = op.values[&"#led1.i".parse::<Ref>().unwrap()].get();
    assert!((i - amps).abs() <= 1e-9 * amps, "{i} A, drive {amps} A");
    op.values[&"#led1.v".parse::<Ref>().unwrap()].get()
}

#[test]
fn a_segment_matches_the_forward_figure_at_20_milliamps() {
    for id in ["display-7seg-cc", "display-7seg-ca"] {
        let file = part_file(id);
        let model = &file["model"]["seg_a"];
        let v = forward(
            model_number(model, "saturation_current"),
            model_number(model, "emission_coefficient"),
            model_number(model, "series_resistance"),
            0.020,
        );
        assert!(
            (v - 1.94).abs() <= 0.02 * 1.94,
            "{id} segment a is {v} V at 20 mA, figure 1.94 V"
        );
    }
}

#[test]
fn every_led_color_matches_its_datasheet_forward_voltage() {
    for row in PARTS {
        let file = part_file(row.part);
        let model = &file["model"];
        let v = forward(
            model_number(model, "saturation_current"),
            model_number(model, "emission_coefficient"),
            model_number(model, "series_resistance"),
            row.milliamps * 1e-3,
        );
        let limit = 0.02 * row.typical;
        assert!(
            (v - row.typical).abs() <= limit,
            "{} is {v} V at {} mA, datasheet {} V",
            row.part,
            row.milliamps,
            row.typical
        );
    }
}
