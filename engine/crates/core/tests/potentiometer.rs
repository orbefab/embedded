// Each test crate uses its own part of `common`.
#[allow(dead_code)]
mod common;

use common::errors;
use sim_core::netlist::{self, Model};
use sim_core::time::Time;

fn document(body: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "top", "mid"],
      "parts": [{{
        "name": "rv1",
        "kind": "potentiometer",
        {body}
      }}]
    }}"#
    )
}

fn fields(resistance: &str, position: &str) -> String {
    format!(
        r#""pins": {{ "a": "top", "wiper": "mid", "b": "gnd" }},
        "resistance": {resistance},
        "position": {position}"#
    )
}

fn value(number: &str, unit: &str) -> String {
    format!(r#"{{ "value": {number}, "unit": "{unit}", "source": "user" }}"#)
}

fn pot(resistance: &str, position: &str) -> String {
    document(&fields(&value(resistance, "ohm"), &value(position, "1")))
}

#[test]
fn a_potentiometer_parses_its_pins_resistance_and_position() {
    let netlist = netlist::parse(pot("10000.0", "0.3").as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Potentiometer(pot) = &part.model else {
        panic!("{:?}", part.model);
    };
    assert_eq!(part.model.pin_names(), ["a", "wiper", "b"]);
    assert!(part.model.ratings().is_empty());
    assert!(part.model.next_change_after(Time::ZERO).is_none());
    assert_eq!(netlist.net_name(pot.pins[0]).as_str(), "top");
    assert_eq!(netlist.net_name(pot.pins[1]).as_str(), "mid");
    assert_eq!(netlist.net_name(pot.pins[2]).as_str(), "gnd");
    assert_eq!(pot.resistance.si.get(), 10000.0);
    assert_eq!(pot.position.si.get(), 0.3);
}

#[test]
fn resistance_follows_the_resistor_rules_and_its_unit() {
    assert_eq!(
        errors(&pot("0", "0.3")),
        [r##""netlist.value" ["#rv1"] {"field":"resistance","rule":"positive","value":0.0}"##]
    );
    assert_eq!(
        errors(&pot("-4.0", "0.3")),
        [r##""netlist.value" ["#rv1"] {"field":"resistance","rule":"positive","value":-4.0}"##]
    );
    let tiny = errors(&pot("1e-320", "0.3"));
    assert_eq!(tiny.len(), 1, "{tiny:?}");
    assert!(
        tiny[0].contains(r#""field":"resistance","rule":"finite_reciprocal""#),
        "{tiny:?}"
    );
    assert_eq!(
        errors(&document(&fields(
            &value("10000.0", "kohm"),
            &value("0.3", "1"),
        ))),
        [r##""netlist.unit" ["#rv1"] {"expected":"ohm","field":"resistance","found":"kohm"}"##]
    );
}

#[test]
fn position_is_an_open_fraction_in_unit_1() {
    for (number, text) in [("0", "0.0"), ("1", "1.0"), ("-1", "-1.0"), ("2", "2.0")] {
        assert_eq!(
            errors(&pot("10000.0", number)),
            [format!(
                r##""netlist.value" ["#rv1"] {{"field":"position","rule":"fraction","value":{text}}}"##
            )],
            "{number}"
        );
    }
    assert_eq!(
        errors(&document(&fields(
            &value("10000.0", "ohm"),
            &value("0.3", "V"),
        ))),
        [r##""netlist.unit" ["#rv1"] {"expected":"1","field":"position","found":"V"}"##]
    );
    assert_eq!(
        errors(&document(&fields(
            &value("10000.0", "ohm"),
            &value("0", "V"),
        ))),
        [
            r##""netlist.unit" ["#rv1"] {"expected":"1","field":"position","found":"V"}"##,
            r##""netlist.value" ["#rv1"] {"field":"position","rule":"fraction","value":0.0}"##,
        ]
    );
}

#[test]
fn pins_are_exactly_a_wiper_and_b_and_every_field_is_required() {
    assert_eq!(
        errors(&document(
            r#""pins": { "a": "top", "b": "gnd" },
            "resistance": { "value": 10000.0, "unit": "ohm", "source": "user" },
            "position": { "value": 0.3, "unit": "1", "source": "user" }"#
        )),
        [r##""netlist.pins" ["#rv1"] {"missing":["wiper"],"unexpected":[]}"##]
    );
    assert_eq!(
        errors(&document(
            r#""pins": { "a": "top", "wiper": "mid", "b": "gnd", "c": "top" },
            "resistance": { "value": 10000.0, "unit": "ohm", "source": "user" },
            "position": { "value": 0.3, "unit": "1", "source": "user" }"#
        )),
        [r##""netlist.pins" ["#rv1"] {"missing":[],"unexpected":["c"]}"##]
    );
    for body in [
        r#""pins": { "a": "top", "wiper": "mid", "b": "gnd" },
          "resistance": { "value": 10000.0, "unit": "ohm", "source": "user" }"#,
        r#""pins": { "a": "top", "wiper": "mid", "b": "gnd" },
          "resistance": { "value": 10000.0, "unit": "ohm", "source": "user" },
          "position": { "value": 0.3, "unit": "1", "source": "user" },
          "ratings": { "power": null }"#,
    ] {
        let found = errors(&document(body));
        assert_eq!(found.len(), 1, "{body} {found:?}");
        assert!(
            found[0].starts_with(r#""netlist.syntax""#),
            "{body} {found:?}"
        );
    }
}

#[test]
fn each_leg_uses_the_resistor_rule_on_field_resistance() {
    let tiny = netlist::parse(pot("1.0", "1e-320").as_bytes()).unwrap_err();
    assert_eq!(tiny.len(), 1, "{tiny:?}");
    let text = serde_json::to_value(&tiny[0]).unwrap();
    assert_eq!(text["code"], "netlist.value");
    assert_eq!(text["data"]["field"], "resistance");
    assert_eq!(text["data"]["rule"], "finite_reciprocal");
    assert_eq!(text["data"]["value"].as_f64().unwrap(), 1e-320);

    assert_eq!(
        errors(&pot("1e-10", "1e-320")),
        [r##""netlist.value" ["#rv1"] {"field":"resistance","rule":"positive","value":0.0}"##]
    );
}
