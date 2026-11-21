//! A `ws2812` count defaults to 13. A count from 0 to 65535 outside 1 to 1024 is rule `leds`,
//! and any other number is `netlist.syntax`. `DOUT` is a pin exactly when `dout` is given, and
//! `dout.delay` is from 1 ps to 1 µs.

use orbe_engine::netlist::{self, Chip, Model};

use orbe_engine::diagnostic::{Diagnostic, ValueRule};

fn chain(leds: &str) -> String {
    format!(
        r#"{{
  "version": 0,
  "seed": 0,
  "analysis": {{ "op": {{}} }},
  "ground": "gnd",
  "nets": ["gnd", "vcc", "din", "dout"],
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

fn count(netlist: &orbe_engine::netlist::Netlist) -> u16 {
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

const DOUT_PIN: &str = r#""pins": { "VCC": "vcc", "GND": "gnd", "DIN": "din", "DOUT": "dout" },"#;
const PINS: &str = r#""pins": { "VCC": "vcc", "GND": "gnd", "DIN": "din" },"#;

fn dout(delay_ps: u64, high_resistance_unit: &str) -> String {
    format!(
        r#""dout": {{
        "output": {{
          "high_drop": {{ "value": 0.0, "unit": "V", "source": "user" }},
          "high_resistance": {{ "value": 50.0, "unit": "{high_resistance_unit}", "source": "user" }},
          "low_drop": {{ "value": 0.0, "unit": "V", "source": "user" }},
          "low_resistance": {{ "value": 50.0, "unit": "ohm", "source": "user" }}
        }},
        "delay": {{ "value": {delay_ps}, "unit": "ps", "source": "user" }}
      }},"#
    )
}

/// The chain with its pins line swapped for `pins`, and `extra` after `leds`.
fn wired(pins: &str, extra: &str) -> String {
    chain(&format!("\"leds\": 1,\n      {extra}")).replace(PINS, pins)
}

fn part(netlist: &orbe_engine::netlist::Netlist) -> orbe_engine::device::Ws2812 {
    let part = netlist
        .parts()
        .find(|(_, part)| part.name.as_str() == "ring")
        .unwrap()
        .1;
    match &part.model {
        Model::Chip(Chip::Ws2812(chain)) => (**chain).clone(),
        other => panic!("ring is {other:?}"),
    }
}

#[test]
fn dout_is_a_pin_with_its_output_and_delay() {
    let netlist = netlist::parse(wired(DOUT_PIN, &dout(500_000, "ohm")).as_bytes()).unwrap();
    let ring = part(&netlist);
    assert_eq!(ring.pins.len(), 4);
    assert_eq!(ring.dout.unwrap().delay.value, 500_000);
    let netlist = netlist::parse(wired(PINS, "").as_bytes()).unwrap();
    let ring = part(&netlist);
    assert_eq!((ring.pins.len(), ring.dout), (3, None));
}

#[test]
fn dout_without_its_pin_or_the_pin_without_dout_is_netlist_pins() {
    let err = netlist::parse(wired(PINS, &dout(500_000, "ohm")).as_bytes()).unwrap_err();
    assert!(
        matches!(&err[..], [Diagnostic::NetlistPins { missing, unexpected, .. }]
            if missing == &["DOUT"] && unexpected.is_empty()),
        "{err:?}"
    );
    let err = netlist::parse(wired(DOUT_PIN, "").as_bytes()).unwrap_err();
    assert!(
        matches!(&err[..], [Diagnostic::NetlistPins { missing, unexpected, .. }]
            if missing.is_empty() && unexpected == &["DOUT"]),
        "{err:?}"
    );
}

#[test]
fn a_dout_delay_past_1_us_is_rule_delay() {
    assert!(netlist::parse(wired(DOUT_PIN, &dout(1_000_000, "ohm")).as_bytes()).is_ok());
    let err = netlist::parse(wired(DOUT_PIN, &dout(1_000_001, "ohm")).as_bytes()).unwrap_err();
    let wire = serde_json::to_value(&err).unwrap();
    assert_eq!(wire[0]["data"]["rule"], "delay", "{wire}");
    assert_eq!(
        wire[0]["message"],
        "#ring dout.delay is 1000001.0; it must be at most 1000000"
    );
}

#[test]
fn a_dout_output_field_is_named_under_dout() {
    let err = netlist::parse(wired(DOUT_PIN, &dout(500_000, "V")).as_bytes()).unwrap_err();
    assert!(
        matches!(&err[..], [Diagnostic::NetlistUnit { field, .. }]
            if field == "dout.output.high_resistance"),
        "{err:?}"
    );
}
