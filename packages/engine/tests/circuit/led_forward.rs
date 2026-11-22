//! Each color of the 3 mm and 5 mm LEDs, driven at its datasheet test current, sits within
//! 2% of that sheet's typical forward voltage. The drive and the typical are the datasheet's.
//! The diode parameters are the ones the part file sets for that color.

use std::path::PathBuf;

use jsonc_parser::{ParseOptions, parse_to_serde_value};
use orbe_engine::name::Ref;
use serde_json::{Value, json};

struct Expect {
    part: &'static str,
    color: &'static str,
    milliamps: f64,
    typical: f64,
}

const COLORS: &[Expect] = &[
    Expect {
        part: "led-5mm",
        color: "red",
        milliamps: 10.0,
        typical: 1.9,
    },
    Expect {
        part: "led-5mm",
        color: "yellow",
        milliamps: 10.0,
        typical: 1.95,
    },
    Expect {
        part: "led-5mm",
        color: "green",
        milliamps: 10.0,
        typical: 2.0,
    },
    Expect {
        part: "led-5mm",
        color: "blue",
        milliamps: 20.0,
        typical: 3.3,
    },
    Expect {
        part: "led-5mm",
        color: "white",
        milliamps: 20.0,
        typical: 3.3,
    },
    Expect {
        part: "led-3mm",
        color: "red",
        milliamps: 10.0,
        typical: 1.9,
    },
    Expect {
        part: "led-3mm",
        color: "yellow",
        milliamps: 10.0,
        typical: 1.95,
    },
    Expect {
        part: "led-3mm",
        color: "green",
        milliamps: 10.0,
        typical: 2.0,
    },
    Expect {
        part: "led-3mm",
        color: "blue",
        milliamps: 20.0,
        typical: 3.3,
    },
    Expect {
        part: "led-3mm",
        color: "white",
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

fn set_number(choice: &Value, pointer: &str) -> f64 {
    choice["sets"][pointer]["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{pointer} in {choice}"))
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
fn every_led_color_matches_its_datasheet_forward_voltage() {
    for part in ["led-3mm", "led-5mm"] {
        let file = part_file(part);
        let choices = file["params"]["color"]["choices"]
            .as_object()
            .unwrap_or_else(|| panic!("{part} has no color choices"));
        let rows: Vec<_> = COLORS.iter().filter(|row| row.part == part).collect();
        let mut names: Vec<_> = choices.keys().map(String::as_str).collect();
        let mut want: Vec<_> = rows.iter().map(|row| row.color).collect();
        names.sort_unstable();
        want.sort_unstable();
        assert_eq!(names, want, "{part}");
        for row in rows {
            let choice = &choices[row.color];
            let v = forward(
                set_number(choice, "/saturation_current"),
                set_number(choice, "/emission_coefficient"),
                set_number(choice, "/series_resistance"),
                row.milliamps * 1e-3,
            );
            let limit = 0.02 * row.typical;
            assert!(
                (v - row.typical).abs() <= limit,
                "{} {} is {v} V at {} mA, datasheet {} V",
                row.part,
                row.color,
                row.milliamps,
                row.typical
            );
        }
    }
}
