use std::fs;
use std::path::PathBuf;

use orbe_engine::name::Ref;
use orbe_engine::netlist::{self, Analysis, Basis, Model, Source};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/netlist/fixtures/invalid")
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
    let wire = serde_json::to_value(&err[0]).unwrap();
    assert_eq!(
        wire["data"],
        serde_json::json!({"found": 2, "supported": [0]})
    );
    assert_eq!(
        wire["message"],
        "netlist version 2 is not supported; this engine reads 0"
    );
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/circuits/divider/netlist.json");
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
            Source::Datasheet { doc, at, basis } => {
                assert_eq!(doc, "resistor-nominal");
                assert_eq!(at, "Table 1, nominal");
                assert_eq!(*basis, Basis::Nominal);
            }
            other => panic!("r1 source {other:?}"),
        },
        other => panic!("r1 model {other:?}"),
    }
}

#[test]
fn a_basis_outside_the_list_and_the_old_typical_flag_are_syntax() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/circuits/divider/netlist.json");
    let divider = fs::read_to_string(path).unwrap();
    for (from, to) in [
        (r#""basis": "nominal""#, r#""basis": "typ""#),
        (r#""basis": "nominal""#, r#""typical": false"#),
    ] {
        let err = netlist::parse(divider.replace(from, to).as_bytes()).unwrap_err();
        assert_eq!(err.len(), 1, "{to}");
        assert_eq!(err[0].code(), "netlist.syntax", "{to}");
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/circuits/divider/netlist.json");
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
