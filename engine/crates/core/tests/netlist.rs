use std::fs;
use std::path::PathBuf;

use sim_core::name::Ref;
use sim_core::netlist::{self, Analysis, Contact, Model, RatingKind, Source, Transient};
use sim_core::time::Time;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/invalid")
        .join(name);
    fs::read(path).unwrap()
}

#[test]
fn one_fixture_per_netlist_code() {
    let cases = [
        ("netlist.syntax.json", "netlist.syntax", &[][..]),
        ("netlist.version.json", "netlist.version", &[]),
        ("netlist.name.json", "netlist.name", &[]),
        ("netlist.duplicate.json", "netlist.duplicate", &["#r1"]),
        (
            "netlist.unknown_net.json",
            "netlist.unknown_net",
            &["#r2.pin.b"],
        ),
        ("netlist.pins.json", "netlist.pins", &["#r1"]),
        ("netlist.unit.json", "netlist.unit", &["#r1"]),
        ("netlist.value.json", "netlist.value", &["#r1"]),
        ("netlist.ground.json", "netlist.ground", &[]),
    ];
    for (file, code, refs) in cases {
        let err = netlist::parse(&fixture(file)).unwrap_err();
        assert_eq!(err.len(), 1, "{file}: {err:?}");
        assert_eq!(err[0].code(), code, "{file}");
        let got: Vec<String> = err[0].refs().iter().map(ToString::to_string).collect();
        assert_eq!(got, refs, "{file}");
    }
}

#[test]
fn one_pass_collects_every_netlist_diagnostic() {
    let err = netlist::parse(&fixture("several.json")).unwrap_err();
    assert_eq!(err.len(), 2, "{err:?}");
    let mut codes: Vec<_> = err.iter().map(|d| d.code()).collect();
    codes.sort_unstable();
    assert_eq!(codes, ["netlist.unit", "netlist.unknown_net"]);
    let unit = err.iter().find(|d| d.code() == "netlist.unit").unwrap();
    let unknown = err
        .iter()
        .find(|d| d.code() == "netlist.unknown_net")
        .unwrap();
    assert_eq!(unit.refs(), vec!["#r1".parse::<Ref>().unwrap()]);
    assert_eq!(unknown.refs(), vec!["#r2.pin.b".parse::<Ref>().unwrap()]);
}

#[test]
fn duplicate_net_refs_the_net() {
    let json = br#"{
      "version": 0,
      "seed": 1,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin", "vin"],
      "parts": []
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].code(), "netlist.duplicate");
    assert_eq!(err[0].refs(), vec!["#net.vin".parse().unwrap()]);
}

#[test]
fn future_version_wins_over_a_schema_error() {
    let err = netlist::parse(br#"{"version": 2, "nope": true}"#).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].code(), "netlist.version");
    assert!(err[0].refs().is_empty());
}

#[test]
fn analysis_must_be_an_op_object() {
    let string = br#"{
      "version": 0,
      "seed": 0,
      "analysis": "op",
      "ground": "gnd",
      "nets": ["gnd"],
      "parts": []
    }"#;
    let err = netlist::parse(string).unwrap_err();
    assert_eq!(err[0].code(), "netlist.syntax");

    let extra = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": { "step": 1 } },
      "ground": "gnd",
      "nets": ["gnd"],
      "parts": []
    }"#;
    let err = netlist::parse(extra).unwrap_err();
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn divider_is_canonical_and_keeps_its_source() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/divider/netlist.json");
    let netlist = netlist::parse(&fs::read(path).unwrap()).unwrap();
    assert_eq!(netlist.seed(), 0);
    assert_eq!(netlist.analysis(), &Analysis::Op);
    let nets: Vec<_> = netlist.nets().map(|(_, n)| n.as_str()).collect();
    assert_eq!(nets, ["gnd", "vin", "vout"]);
    let parts: Vec<_> = netlist.parts().map(|(_, p)| p.name.as_str()).collect();
    assert_eq!(parts, ["r1", "r2", "v1"]);

    let (_, r1) = netlist
        .parts()
        .find(|(_, p)| p.name.as_str() == "r1")
        .unwrap();
    match &r1.model {
        Model::Resistor(r) => match &r.resistance.source {
            Source::Datasheet { doc, at, typical } => {
                assert_eq!(doc, "resistor-nominal");
                assert_eq!(at, "Table 1, nominal");
                assert!(!typical);
            }
            other => panic!("r1 source {other:?}"),
        },
        other => panic!("r1 model {other:?}"),
    }
}

#[test]
fn reordering_parts_and_nets_keeps_the_same_ids() {
    let shuffled = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["vout", "vin", "gnd"],
      "parts": [
        {
          "name": "r2",
          "kind": "resistor",
          "pins": { "b": "gnd", "a": "vout" },
          "resistance": { "value": 2000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        },
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "n": "gnd", "p": "vin" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" }
        },
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "b": "vout", "a": "vin" },
          "resistance": {
            "value": 1000.0,
            "unit": "ohm",
            "source": { "closed_form": { "note": "divider" } }
          },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let netlist = netlist::parse(shuffled).unwrap();
    let nets: Vec<_> = netlist
        .nets()
        .map(|(_, n)| n.as_str().to_string())
        .collect();
    assert_eq!(nets, ["gnd", "vin", "vout"]);
    let parts: Vec<_> = netlist
        .parts()
        .map(|(_, p)| p.name.as_str().to_string())
        .collect();
    assert_eq!(parts, ["r1", "r2", "v1"]);

    let pin_nets = |part: &str| -> Vec<String> {
        let (_, found) = netlist
            .parts()
            .find(|(_, p)| p.name.as_str() == part)
            .unwrap();
        found
            .model
            .pins()
            .iter()
            .map(|id| netlist.net_name(*id).as_str().to_string())
            .collect()
    };
    assert_eq!(pin_nets("r1"), ["vin", "vout"]);
    assert_eq!(pin_nets("r2"), ["vout", "gnd"]);
    assert_eq!(pin_nets("v1"), ["vin", "gnd"]);
    assert_eq!(
        netlist
            .part(netlist.parts().next().unwrap().0)
            .name
            .as_str(),
        "r1"
    );
}

#[test]
fn duplicate_pin_key_is_syntax() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin", "vout"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vout", "b": "gnd", "b": "vin" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains("duplicate pin `b`"), "{text}");
    assert!(text.contains("\"path\":\"parts[0]\""), "{text}");
}

#[test]
fn trailing_bytes_are_syntax_and_whitespace_is_kept() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/divider/netlist.json");
    let bytes = fs::read(path).unwrap();
    let mut spaced = bytes.clone();
    spaced.extend(b"\n  \n");
    assert!(netlist::parse(&spaced).is_ok());

    let mut trailed = bytes;
    trailed.extend(b"\nthis is not json");
    let err = netlist::parse(&trailed).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn future_version_wins_over_trailing_bytes() {
    let err = netlist::parse(br#"{"version": 2} this is not json"#).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].code(), "netlist.version");
}

#[test]
fn resistance_with_a_non_finite_reciprocal_is_a_value_error() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd" },
          "resistance": { "value": 1e-320, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.value");
    assert_eq!(err[0].refs(), vec!["#r1".parse().unwrap()]);
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""rule":"finite_reciprocal""#), "{text}");
}

#[test]
fn a_very_small_finite_conductance_parses() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd" },
          "resistance": { "value": 1e-300, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }
      ]
    }"#;
    assert!(netlist::parse(json).is_ok());
}

#[test]
fn a_number_past_the_f64_range_is_syntax() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 1e309, "unit": "V", "source": "user" }
        }
      ]
    }"#;
    let err = netlist::parse(json).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn unexpected_pin_on_an_unknown_net_reports_both() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd", "c": "missing" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
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
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd", "1": "missing" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
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
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "net",
          "kind": "resistor",
          "pins": { "c": "missing" },
          "resistance": { "value": 0, "unit": "farad", "source": "user" },
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
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "1bad",
          "kind": "resistor",
          "pins": { "c": "missing" },
          "resistance": { "value": 0, "unit": "farad", "source": "user" },
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

#[test]
fn negative_voltage_is_legal() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": -5.0, "unit": "V", "source": "user" }
        }
      ]
    }"#;
    let netlist = netlist::parse(json).unwrap();
    let (_, v1) = netlist.parts().next().unwrap();
    match &v1.model {
        Model::VoltageSource(v) => assert_eq!(v.voltage.si.get(), -5.0),
        other => panic!("{other:?}"),
    }
}

fn led_with_ratings(ratings: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "vled"],
      "parts": [
        {{
          "name": "led1",
          "kind": "led",
          "pins": {{ "anode": "vled", "cathode": "gnd" }},
          "saturation_current": {{ "value": 6.711e-10, "unit": "A", "source": "user" }},
          "emission_coefficient": {{ "value": 4.427, "unit": "1", "source": "user" }},
          "series_resistance": {{ "value": 2.115, "unit": "ohm", "source": "user" }},
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
            "current": { "value": 0.03, "unit": "A", "source": "user" },
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
        .map(|(kind, value)| (kind, value.si.get()))
        .collect();
    assert_eq!(ratings, [(RatingKind::Current, 0.03)]);
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
            "current": { "value": 30.0, "unit": "mA", "source": "user" },
            "power": { "value": 0.0, "unit": "W", "source": "user" }
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

#[test]
fn a_resistor_without_ratings_is_syntax() {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "vin", "b": "gnd" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" }
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
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" },
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

fn diode_at(is: &str, n: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "a", "b"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "a", "n": "gnd" }},
          "voltage": {{ "value": 1.0, "unit": "V", "source": "user" }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "a", "b": "b" }},
          "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "power": null }}
        }},
        {{
          "name": "d1",
          "kind": "diode",
          "pins": {{ "anode": "b", "cathode": "gnd" }},
          "saturation_current": {{ "value": {is}, "unit": "A", "source": "user" }},
          "emission_coefficient": {{ "value": {n}, "unit": "1", "source": "user" }},
          "series_resistance": {{ "value": 1.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "current": null, "power": null }}
        }}
      ]
    }}"#
    )
}

fn junction_value_error(json: &str) -> String {
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    assert_eq!(err.len(), 1, "{err:?}");
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains(r#""code":"netlist.value""#), "{text}");
    assert!(text.contains(r#""rule":"finite_junction""#), "{text}");
    assert!(
        text.contains("n*VT must be normal, and vcrit must be finite and positive"),
        "{text}"
    );
    text
}

#[test]
fn a_junction_with_a_non_finite_vcrit_is_a_value_error() {
    let text = junction_value_error(&diode_at("1e-308", "100"));
    assert!(text.contains(r#""field":"saturation_current""#), "{text}");
    assert!(text.contains(r##""refs":["#d1"]"##), "{text}");
}

#[test]
fn a_huge_emission_coefficient_is_a_value_error() {
    let text = junction_value_error(&diode_at("1e-14", "1e308"));
    assert!(text.contains(r#""field":"emission_coefficient""#), "{text}");
}

#[test]
fn a_tiny_emission_coefficient_is_a_value_error() {
    let text = junction_value_error(&diode_at("1e-14", "1e-300"));
    assert!(text.contains(r#""field":"emission_coefficient""#), "{text}");
}

#[test]
fn a_subnormal_saturation_current_prints_like_its_data_number() {
    let text = junction_value_error(&diode_at("1e-308", "100"));
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let number = doc["data"]["value"].to_string();
    let message = doc["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("is {number};")),
        "{message} vs {number}"
    );
    assert_eq!(number, "1e-308");
}

#[test]
fn a_junction_just_inside_the_domain_parses() {
    let netlist = netlist::parse(diode_at("1e-308", "90").as_bytes()).unwrap();
    let (_, diode) = netlist
        .parts()
        .find(|(_, part)| part.name.as_str() == "d1")
        .unwrap();
    assert!(matches!(diode.model, Model::Diode(_)), "{:?}", diode.model);
}

fn with_parts(analysis: &str, parts: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 7,
      "analysis": {analysis},
      "ground": "gnd",
      "nets": ["gnd", "a"],
      "parts": [{parts}]
    }}"#
    )
}

fn switch(events: &str, bounce: &str) -> String {
    with_parts(
        r#"{ "op": {} }"#,
        &format!(
            r#"{{
          "name": "sw1",
          "kind": "switch",
          "pins": {{ "a": "a", "b": "gnd" }},
          "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "open",
          "events": {events},
          "bounce": {bounce}
        }}"#
        ),
    )
}

fn bounce(window: &str, max_bounces: &str) -> String {
    format!(
        r#"{{
          "window": {{ "value": {window}, "unit": "ps", "source": "user" }},
          "max_bounces": {{ "value": {max_bounces}, "unit": "1", "source": "user" }}
        }}"#
    )
}

fn errors(json: &str) -> Vec<String> {
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    err.iter()
        .map(|d| serde_json::to_value(d).unwrap())
        .map(|d| format!("{} {} {}", d["code"], d["refs"], d["data"]))
        .collect()
}

#[test]
fn a_capacitor_keeps_its_value_and_has_no_ratings() {
    let part = r#"{
      "name": "c1",
      "kind": "capacitor",
      "pins": { "a": "a", "b": "gnd" },
      "capacitance": { "value": 1e-6, "unit": "F", "source": "user" }
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
    let rated = part.replace(
        r#""source": "user" }"#,
        r#""source": "user" }, "ratings": {}"#,
    );
    let err = netlist::parse(with_parts(r#"{ "op": {} }"#, &rated).as_bytes()).unwrap_err();
    assert_eq!(err[0].code(), "netlist.syntax");
}

#[test]
fn a_switch_resolves_its_events_and_bounce_into_one_schedule() {
    let json = switch(
        r#"[{ "t_ps": 2000000000, "state": "closed" }, { "t_ps": 12000000000, "state": "open" }]"#,
        &bounce("1000000000", "4"),
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let (_, sw1) = netlist.parts().next().unwrap();
    let Model::Switch(switch) = &sw1.model else {
        panic!("{:?}", sw1.model);
    };
    assert_eq!(switch.events.len(), 2);
    let schedule = &switch.schedule;
    assert_eq!(schedule.initial(), Contact::Open);
    assert_eq!(schedule.edges().len(), 14);
    assert_eq!(
        schedule.edges(),
        netlist::bounce::expand(7, &sw1.name, &switch.events, switch.bounce.as_ref())
    );
    assert_eq!(schedule.before(Time::ZERO), Contact::Open);
    assert_eq!(schedule.before(Time::from_ps(2_000_000_000)), Contact::Open);
    assert_eq!(
        schedule.before(Time::from_ps(2_000_000_001)),
        Contact::Closed
    );
    assert_eq!(
        schedule.before(Time::from_ps(20_000_000_000)),
        Contact::Open
    );
    assert!(sw1.model.ratings().is_empty());

    let other_seed = json.replace(r#""seed": 7"#, r#""seed": 8"#);
    let reseeded = netlist::parse(other_seed.as_bytes()).unwrap();
    let Model::Switch(moved) = &reseeded.parts().next().unwrap().1.model else {
        panic!("not a switch");
    };
    assert_ne!(moved.schedule.edges(), schedule.edges());
    assert_eq!(moved.schedule.edges()[0], schedule.edges()[0]);
}

#[test]
fn each_broken_event_names_its_index_and_rule() {
    let cases = [
        (
            r#"[{ "t_ps": 0, "state": "closed" }]"#,
            "null",
            "at_zero",
            0,
        ),
        (
            r#"[{ "t_ps": 5, "state": "open" }]"#,
            "null",
            "same_state",
            0,
        ),
        (
            r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 6999, "state": "open" }]"#,
            "null",
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 9000, "state": "closed" }, { "t_ps": 5000, "state": "open" }]"#,
            "null",
            "not_increasing",
            1,
        ),
        (
            r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 5000, "state": "open" }]"#,
            "null",
            "not_increasing",
            1,
        ),
        (
            r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 10999, "state": "open" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 18446744073709550000, "state": "closed" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "overflow",
            0,
        ),
        (
            r#"[{ "t_ps": 18446744073709550000, "state": "closed" }, { "t_ps": 18446744073709551000, "state": "open" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "overflow",
            0,
        ),
        (
            r#"[{ "t_ps": 18446744073709550115, "state": "closed" }, { "t_ps": 18446744073709551115, "state": "open" }]"#,
            "null",
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 18446744073709545616, "state": "closed" }, { "t_ps": 18446744073709545716, "state": "open" }]"#,
            r#"{ "window": { "value": 4000, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 1, "unit": "1", "source": "user" } }"#,
            "too_close",
            1,
        ),
        (
            r#"[{ "t_ps": 10000, "state": "closed" }, { "t_ps": 10001, "state": "open" }]"#,
            r#"{ "window": { "value": 18446744073709540615, "unit": "ps", "source": "user" },
                 "max_bounces": { "value": 8, "unit": "1", "source": "user" } }"#,
            "too_close",
            1,
        ),
    ];
    for (events, bounce, rule, event) in cases {
        assert_eq!(
            errors(&switch(events, bounce)),
            [format!(
                r##""netlist.schedule" ["#sw1"] {{"event":{event},"rule":"{rule}"}}"##
            )],
            "{events}"
        );
    }
    let spaced = r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 7000, "state": "open" }]"#;
    assert!(netlist::parse(switch(spaced, "null").as_bytes()).is_ok());
    let after_window =
        r#"[{ "t_ps": 5000, "state": "closed" }, { "t_ps": 11000, "state": "open" }]"#;
    assert!(netlist::parse(switch(after_window, &bounce("4000", "1")).as_bytes()).is_ok());
    let gap_fits_at_the_top = r#"[{ "t_ps": 18446744073709549615, "state": "closed" }, { "t_ps": 18446744073709551615, "state": "open" }]"#;
    assert!(netlist::parse(switch(gap_fits_at_the_top, "null").as_bytes()).is_ok());
}

#[test]
fn bounce_values_follow_the_value_rules() {
    let events = r#"[{ "t_ps": 5000, "state": "closed" }]"#;
    assert_eq!(
        errors(&switch(events, &bounce("0", "1"))),
        [r##""netlist.value" ["#sw1"] {"field":"bounce.window","rule":"positive","value":0.0}"##]
    );
    assert_eq!(
        errors(&switch(events, &bounce("4000", "0"))),
        [
            r##""netlist.value" ["#sw1"] {"field":"bounce.max_bounces","rule":"positive","value":0.0}"##
        ]
    );
    assert_eq!(
        errors(&switch(events, &bounce("7999", "2"))),
        [
            r##""netlist.value" ["#sw1"] {"field":"bounce.window","rule":"bounce_room","value":7999.0}"##
        ]
    );
    assert!(netlist::parse(switch(events, &bounce("8000", "2")).as_bytes()).is_ok());
    let unit = switch(events, &bounce("4000", "1")).replace(r#""unit": "ps""#, r#""unit": "s""#);
    assert_eq!(
        errors(&unit),
        [r##""netlist.unit" ["#sw1"] {"expected":"ps","field":"bounce.window","found":"s"}"##]
    );
    for (window, max_bounces) in [
        ("4000.0", "1"),
        ("1e9", "1"),
        ("4000", "256"),
        ("4000", "-1"),
    ] {
        let err =
            netlist::parse(switch(events, &bounce(window, max_bounces)).as_bytes()).unwrap_err();
        assert_eq!(err[0].code(), "netlist.syntax", "{window} {max_bounces}");
    }
    let missing = switch(events, "null").replace(",\n          \"bounce\": null", "");
    assert!(!missing.contains("bounce"), "{missing}");
    let err = netlist::parse(missing.as_bytes()).unwrap_err();
    let text = serde_json::to_string(&err[0]).unwrap();
    assert!(text.contains("missing field `bounce`"), "{text}");
}

fn transient(stop_ps: &str, max_step_ps: &str) -> String {
    with_parts(
        &format!(r#"{{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps} }} }}"#),
        "",
    )
}

#[test]
fn a_transient_analysis_keeps_its_stop_and_max_step() {
    let netlist = netlist::parse(transient("81000000000", "20000000").as_bytes()).unwrap();
    assert_eq!(
        netlist.analysis(),
        &Analysis::Transient(Transient {
            stop: Time::from_ps(81_000_000_000),
            max_step: 20_000_000,
        })
    );
    assert!(netlist::parse(transient("1000", "1000").as_bytes()).is_ok());
}

#[test]
fn a_transient_analysis_breaks_each_rule_with_its_field() {
    let cases = [
        (
            "0",
            "1000",
            vec![
                ("stop_ps", "positive", "0"),
                ("max_step_ps", "step_over_stop", "1000"),
            ],
        ),
        (
            "5000",
            "999",
            vec![("max_step_ps", "step_under_1ns", "999")],
        ),
        ("5000", "0", vec![("max_step_ps", "step_under_1ns", "0")]),
        (
            "5000",
            "5001",
            vec![("max_step_ps", "step_over_stop", "5001")],
        ),
    ];
    for (stop, max_step, expected) in cases {
        let expected: Vec<String> = expected
            .iter()
            .map(|(field, rule, value)| {
                format!(
                    r#""netlist.analysis" [] {{"field":"{field}","rule":"{rule}","value":{value}}}"#
                )
            })
            .collect();
        assert_eq!(
            errors(&transient(stop, max_step)),
            expected,
            "{stop} {max_step}"
        );
    }
    for shape in [
        r#"{ "transient": { "stop_ps": 5000 } }"#,
        r#"{ "transient": { "stop_ps": 5000, "max_step_ps": 1000.5 } }"#,
        r#"{ "transient": { "stop_ps": 5000, "max_step_ps": 1000, "min_step_ps": 1 } }"#,
        r#"{ "op": {}, "transient": { "stop_ps": 5000, "max_step_ps": 1000 } }"#,
    ] {
        let err = netlist::parse(with_parts(shape, "").as_bytes()).unwrap_err();
        assert_eq!(err[0].code(), "netlist.syntax", "{shape}");
    }
}
