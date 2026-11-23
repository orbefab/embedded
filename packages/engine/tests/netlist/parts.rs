use orbe_engine::name::Quantity;
use orbe_engine::netlist::{self, Model, RatingKind};

use crate::common;

use common::{errors, with_parts};

#[test]
fn unexpected_pin_on_an_unknown_net_reports_both() {
    let json = br#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd", "c": "missing" },
          "resistance": { "value": 1000.0, "unit": "ohm" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 2, "{err:?}");
    let pins = err.iter().find(|d| d.code() == "netlist.pins").unwrap();
    let unknown = err
        .iter()
        .find(|d| d.code() == "netlist.unknown_net")
        .unwrap();
    assert_eq!(pins.refs(), vec!["#r1".parse().unwrap()]);
    assert_eq!(unknown.refs(), vec!["#r1.pin.c".parse().unwrap()]);
    let text = serde_json::to_string(unknown).unwrap();
    assert!(text.contains(r#""net":"missing""#), "{text}");
}

#[test]
fn an_illegal_unexpected_pin_key_stays_a_pin_mismatch() {
    let json = br#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd", "1": "missing" },
          "resistance": { "value": 1000.0, "unit": "ohm" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.pins");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""unexpected":["1"]"#), "{text}");
}

#[test]
fn a_reserved_part_name_still_reports_pin_unit_and_value() {
    let json = br#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "net",
          "kind": "resistor",
          "pins": { "c": "missing" },
          "resistance": { "value": 0, "unit": "farad" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    let mut codes: Vec<_> = err.iter().map(|d| d.code()).collect();
    codes.sort_unstable();
    assert_eq!(
        codes,
        [
            "netlist.name",
            "netlist.pins",
            "netlist.unit",
            "netlist.unknown_net",
            "netlist.value",
        ]
    );
    let unknown = err
        .iter()
        .find(|d| d.code() == "netlist.unknown_net")
        .unwrap();
    assert_eq!(unknown.refs()[0].to_string(), "#net.pin.c");
}

#[test]
fn a_name_that_fails_the_grammar_stops() {
    let json = br#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "1bad",
          "kind": "resistor",
          "pins": { "c": "missing" },
          "resistance": { "value": 0, "unit": "farad" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.name");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""problem":"bad_char""#), "{text}");
}

fn led_with_ratings(ratings: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "vled"],
      "parts": [
        {{
          "name": "led1",
          "kind": "led",
          "pins": {{ "anode": "vled", "cathode": "gnd" }},
          "saturation_current": {{ "value": 6.711e-10, "unit": "A" }},
          "emission_coefficient": {{ "value": 4.427, "unit": "1" }},
          "series_resistance": {{ "value": 2.115, "unit": "ohm" }},
          "ratings": {ratings}
        }}
      ]
    }}"#
    )
}

#[test]
fn an_led_keeps_its_parameters_and_ratings() {
    let json = led_with_ratings(
        r#"{
            "current": { "value": 0.03, "unit": "A" },
            "power": null
          }"#,
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let (_, led) = netlist.parts().next().unwrap();
    let Model::Led(junction) = &led.model else {
        panic!("{:?}", led.model);
    };
    assert_eq!(junction.saturation_current.si.get(), 6.711e-10);
    assert_eq!(junction.emission_coefficient.si.get(), 4.427);
    assert_eq!(junction.series_resistance.si.get(), 2.115);
    assert_eq!(led.model.pin_names(), ["anode", "cathode"]);
    let ratings: Vec<_> = led
        .model
        .ratings()
        .into_iter()
        .map(|(kind, quantity, value)| (kind, quantity, value.si.get()))
        .collect();
    assert_eq!(ratings, [(RatingKind::Current, Quantity::I, 0.03)]);
}

#[test]
fn a_missing_rating_key_is_syntax() {
    let json = led_with_ratings(r#"{ "current": null }"#);
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains("missing field `power`"), "{text}");
}

#[test]
fn an_unknown_rating_key_is_syntax() {
    let json = led_with_ratings(r#"{ "current": null, "power": null, "voltage": null }"#);
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn a_rating_in_the_wrong_unit_or_not_positive_names_its_field() {
    let json = led_with_ratings(
        r#"{
            "current": { "value": 30.0, "unit": "mA" },
            "power": { "value": 0.0, "unit": "W" }
          }"#,
    );
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 2, "{err:?}");
    let unit = serde_json::to_string(&err[0]).unwrap();
    assert!(unit.contains(r#""code":"netlist.unit""#), "{unit}");
    assert!(unit.contains(r#""field":"ratings.current""#), "{unit}");
    let value = serde_json::to_string(&err[1]).unwrap();
    assert!(value.contains(r#""code":"netlist.value""#), "{value}");
    assert!(value.contains(r#""field":"ratings.power""#), "{value}");
}

/// A sustained rating may carry its sheet's peak limit, in the rating's unit and above 0.
#[test]
fn a_sustained_rating_keeps_its_peak_limit_and_checks_it_as_a_value() {
    let sustained = |peak: &str| {
        led_with_ratings(&format!(
            r#"{{
            "current": {{ "value": 0.03, "unit": "A", "basis": {{ "sustained": {{ "average": "mean", "tau": {{ "value": 0.001, "unit": "s" }}, "peak": {peak} }} }} }},
            "power": null
          }}"#
        ))
    };
    let netlist =
        netlist::parse(sustained(r#"{ "value": 0.16, "unit": "A" }"#).as_bytes()).unwrap();
    let (_, led) = netlist.parts().next().unwrap();
    let [(_, _, rating)] = led.model.ratings().try_into().unwrap();
    assert_eq!(rating.peak.map(|peak| peak.get()), Some(0.16));

    let err =
        netlist::parse(sustained(r#"{ "value": 160.0, "unit": "mA" }"#).as_bytes()).unwrap_err();
    let [unit] = err.as_slice() else {
        panic!("{err:?}")
    };
    let unit = serde_json::to_string(unit).unwrap();
    assert!(unit.contains(r#""code":"netlist.unit""#), "{unit}");
    assert!(
        unit.contains(r#""field":"ratings.current.basis.sustained.peak""#),
        "{unit}"
    );
    let err = netlist::parse(sustained(r#"{ "value": 0.0, "unit": "A" }"#).as_bytes()).unwrap_err();
    let [value] = err.as_slice() else {
        panic!("{err:?}")
    };
    let value = serde_json::to_string(value).unwrap();
    assert!(value.contains(r#""rule":"positive""#), "{value}");
    assert!(
        value.contains(r#""field":"ratings.current.basis.sustained.peak""#),
        "{value}"
    );
}

/// A peak basis has nothing beside it to bound.
#[test]
fn a_peak_basis_takes_no_peak_limit() {
    let json = led_with_ratings(
        r#"{
            "current": { "value": 0.03, "unit": "A", "basis": { "peak": { "value": 0.16, "unit": "A" } } },
            "power": null
          }"#,
    );
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn a_resistor_without_ratings_is_syntax() {
    let json = br#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd" },
          "resistance": { "value": 1000.0, "unit": "ohm" }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains("missing field `ratings`"), "{text}");
}

#[test]
fn a_voltage_source_has_no_ratings() {
    let json = br#"{
      "version": 1,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn a_diode_takes_the_same_fields_as_an_led() {
    let json = led_with_ratings(r#"{ "current": null, "power": null }"#)
        .replace(r#""kind": "led""#, r#""kind": "diode""#)
        .replace(r#""unit": "1""#, r#""unit": "ohm""#);
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""field":"emission_coefficient""#), "{text}");

    let json = led_with_ratings(r#"{ "current": null, "power": null }"#)
        .replace(r#""kind": "led""#, r#""kind": "diode""#);
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let (_, diode) = netlist.parts().next().unwrap();
    assert!(matches!(diode.model, Model::Diode(_)), "{:?}", diode.model);
    assert!(diode.model.ratings().is_empty());
}

#[test]
fn a_capacitor_keeps_its_value_and_has_no_ratings() {
    let part = r#"{
      "name": "c1",
      "kind": "capacitor",
      "pins": { "a": "a", "b": "gnd" },
      "capacitance": { "value": 1e-6, "unit": "F" },
      "initial": null
    }"#;
    let netlist = netlist::parse(with_parts(r#"{ "op": {} }"#, part).as_bytes()).unwrap();
    let (_, c1) = netlist.parts().next().unwrap();
    let Model::Capacitor(capacitor) = &c1.model else {
        panic!("{:?}", c1.model);
    };
    assert_eq!(capacitor.capacitance.si.get(), 1e-6);
    assert!(c1.model.ratings().is_empty());

    let zero = part.replace("1e-6", "0.0");
    assert_eq!(
        errors(&with_parts(r#"{ "op": {} }"#, &zero)),
        [r##""netlist.value" ["#c1"] {"field":"capacitance","rule":"positive","value":0.0}"##]
    );
    let rated = part.replace(r#""unit": "F" }"#, r#""unit": "F" }, "ratings": {}"#);
    let err = netlist::parse(with_parts(r#"{ "op": {} }"#, &rated).as_bytes()).unwrap_err();
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn a_capacitor_initial_is_required_and_any_finite_voltage() {
    let part = r#"{
      "name": "c1",
      "kind": "capacitor",
      "pins": { "a": "a", "b": "gnd" },
      "capacitance": { "value": 1e-6, "unit": "F" },
      "initial": { "value": -2.5, "unit": "V" }
    }"#;
    let netlist = netlist::parse(with_parts(r#"{ "op": {} }"#, part).as_bytes()).unwrap();
    let (_, c1) = netlist.parts().next().unwrap();
    let Model::Capacitor(capacitor) = &c1.model else {
        panic!("{:?}", c1.model);
    };
    assert_eq!(capacitor.initial.as_ref().unwrap().si.get(), -2.5);

    let open = part.replace(r#"{ "value": -2.5, "unit": "V" }"#, "null");
    let netlist = netlist::parse(with_parts(r#"{ "op": {} }"#, &open).as_bytes()).unwrap();
    let Model::Capacitor(capacitor) = &netlist.parts().next().unwrap().1.model else {
        panic!("not a capacitor");
    };
    assert_eq!(capacitor.initial, None);

    let millivolts = part.replace(r#""unit": "V""#, r#""unit": "mV""#);
    assert_eq!(
        errors(&with_parts(r#"{ "op": {} }"#, &millivolts)),
        [r##""netlist.unit" ["#c1"] {"expected":"V","field":"initial","found":"mV"}"##]
    );
    let missing = part.replace(
        r#",
      "initial": { "value": -2.5, "unit": "V" }"#,
        "",
    );
    let err = netlist::parse(with_parts(r#"{ "op": {} }"#, &missing).as_bytes()).unwrap_err();
    assert_eq!(err[0].code(), "netlist.syntax");
}
