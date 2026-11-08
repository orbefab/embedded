//! A `ws2812` count defaults to 13. A count from 0 to 65535 outside 1 to 1024 is rule `leds`,
//! and any other number is `netlist.syntax`.

use orbe_engine_core::diagnostic::{Diagnostic, ValueRule};
use orbe_engine_core::netlist::{self, Chip, Model};

fn chain(leds: &str) -> String {
    format!(
        r#"{{
  "version": 0,
  "seed": 0,
  "analysis": {{ "op": {{}} }},
  "ground": "gnd",
  "nets": ["gnd", "vcc", "din"],
  "parts": [
    {{
      "name": "v1",
      "kind": "voltage_source",
      "pins": {{ "p": "vcc", "n": "gnd" }},
      "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }}
    }},
    {{
      "name": "ring",
      "kind": "ws2812",
      "pins": {{ "VCC": "vcc", "GND": "gnd", "DIN": "din" }},
      {leds}
      "input_threshold": {{
        "low": {{ "value": 0.14, "unit": "1", "source": "user" }},
        "high": {{ "value": 0.7, "unit": "1", "source": "user" }}
      }}
    }}
  ]
}}"#
    )
}

fn count(netlist: &orbe_engine_core::netlist::Netlist) -> u16 {
    let part = netlist
        .parts()
        .find(|(_, part)| part.name.as_str() == "ring")
        .unwrap()
        .1;
    match &part.model {
        Model::Chip(Chip::Ws2812(chain)) => chain.leds,
        other => panic!("ring is {other:?}"),
    }
}

#[test]
fn a_missing_count_is_13() {
    let netlist = netlist::parse(chain("").as_bytes()).unwrap();
    assert_eq!(count(&netlist), 13);
}

#[test]
fn a_count_of_zero_or_1025_is_refused() {
    for leds in ["\"leds\": 0,", "\"leds\": 1025,"] {
        let err = netlist::parse(chain(leds).as_bytes()).unwrap_err();
        assert!(
            err.iter().any(|diagnostic| matches!(
                diagnostic,
                Diagnostic::NetlistValue { rule: ValueRule::Leds, field, .. } if field == "leds"
            )),
            "{leds} {err:?}"
        );
    }
}

#[test]
fn a_count_of_zero_prints_as_an_integer() {
    let err = netlist::parse(chain("\"leds\": 0,").as_bytes()).unwrap_err();
    let wire = serde_json::to_value(&err[0]).unwrap();
    assert_eq!(
        wire["message"],
        "#ring leds is 0; it must be from 1 to 1024"
    );
}

#[test]
fn a_count_that_is_no_u16_is_a_syntax_error() {
    for leds in [
        "\"leds\": 70000,",
        "\"leds\": -1,",
        "\"leds\": 1.5,",
        "\"leds\": 13.0,",
    ] {
        let err = netlist::parse(chain(leds).as_bytes()).unwrap_err();
        assert!(
            err.iter()
                .all(|diagnostic| diagnostic.code() == "netlist.syntax"),
            "{leds} {err:?}"
        );
    }
}
