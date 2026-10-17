use std::fs;
use std::path::PathBuf;

use sim_core::name::Ref;
use sim_core::netlist::{self, Analysis, Model, RatingKind, Source};

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
