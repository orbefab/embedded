// Each test crate uses its own part of `common`.
#[allow(dead_code)]
mod common;

use common::{resistor, simulate, source};
use sim_core::name::Ref;

const PART: &str = include_str!("../../core/tests/fixtures/atmega328p.json");

/// The documented ATmega328P on a 5 V supply, with `wired` moving pins to other nets.
fn board(wired: &[(&str, &str)], parts: &[String]) -> String {
    let mut chip: serde_json::Value = serde_json::from_str(PART).unwrap();
    for &(pin, net) in wired {
        chip["pins"][pin] = net.into();
    }
    let mut nets: Vec<&str> = chip["pins"]
        .as_object()
        .unwrap()
        .values()
        .filter_map(serde_json::Value::as_str)
        .chain(["gnd", "vcc", "x"])
        .collect();
    nets.sort();
    nets.dedup();
    let mut parts: Vec<String> = parts.to_vec();
    parts.push(source(5.0));
    parts.push(chip.to_string());
    format!(
        r#"{{ "version": 0, "seed": 0, "analysis": {{ "op": {{}} }}, "ground": "gnd",
          "nets": {}, "parts": [{}] }}"#,
        serde_json::to_string(&nets).unwrap(),
        parts.join(",")
    )
}

fn floating(json: &str) -> Vec<String> {
    simulate(json)
        .diagnostics()
        .iter()
        .map(|d| {
            assert_eq!(d.code(), "net.floating", "{d:?}");
            match &d.refs()[..] {
                [Ref::Net(net)] => net.as_str().to_string(),
                refs => panic!("{refs:?}"),
            }
        })
        .collect()
}

/// `x` is a net no part names until a test puts one on it.
#[test]
fn an_unconnected_port_pin_is_not_floating_and_the_crystal_and_aref_pins_are() {
    assert_eq!(floating(&board(&[], &[])), ["aref", "x", "xtal1", "xtal2"]);
}

#[test]
fn avcc_joins_nothing() {
    let json = board(&[("AVCC", "avcc")], &[]);
    assert_eq!(floating(&json), ["aref", "avcc", "x", "xtal1", "xtal2"]);
}

#[test]
fn a_net_whose_only_path_runs_through_a_floating_pin_floats() {
    let json = board(&[], &[resistor("rx", "x", "aref", 1000.0)]);
    assert_eq!(floating(&json), ["aref", "x", "xtal1", "xtal2"]);
    let json = board(&[], &[resistor("rx", "x", "pd3", 1000.0)]);
    assert_eq!(floating(&json), ["aref", "xtal1", "xtal2"]);
}

#[test]
fn a_net_on_reset_is_not_floating() {
    assert_eq!(
        floating(&board(&[("PC6", "x")], &[])),
        ["aref", "xtal1", "xtal2"]
    );
}
