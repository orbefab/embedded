//! What docs/registry.md promises a client, checked against what the build writes: every record
//! and index validates against the schemas it ships with, the schemas refuse what the contract
//! forbids, absent fields are left out, and a record maps to `orbe part --json` as documented.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use common::{record, repo, third_party};
use jsonschema::Validator;
use orbe_registry::{Dependency, INDEX_SCHEMA, ITEM_SCHEMA, build, path_problem, pattern};
use serde_json::{Value, json};

fn validator(output: &BTreeMap<String, Vec<u8>>, name: &str) -> Validator {
    let schema: Value = serde_json::from_slice(&output[name]).unwrap();
    assert!(
        jsonschema::draft202012::meta::is_valid(&schema),
        "{name} is a Draft 2020-12 schema"
    );
    jsonschema::draft202012::new(&schema).unwrap()
}

fn errors(validator: &Validator, value: &Value) -> Vec<String> {
    validator
        .iter_errors(value)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect()
}

/// Every record and the index of the registry built from `src` validate against its schemas.
fn every_file_validates(src: &std::path::Path) -> usize {
    let registry = build(src).unwrap();
    let items = validator(&registry.output, ITEM_SCHEMA);
    let index = validator(&registry.output, INDEX_SCHEMA);
    let value: Value = serde_json::from_slice(&registry.output["index.json"]).unwrap();
    assert_eq!(errors(&index, &value), Vec::<String>::new(), "index.json");
    let mut records = 0;
    for (path, bytes) in &registry.output {
        if path.contains('@') {
            let value: Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(errors(&items, &value), Vec::<String>::new(), "{path}");
            records += 1;
        }
    }
    records
}

#[test]
fn every_record_and_the_index_validate_against_the_schemas() {
    let records = every_file_validates(&repo());
    assert_eq!(records, build(&repo()).unwrap().items.len());
    assert_eq!(every_file_validates(third_party().path()), 2);
}

#[test]
fn the_schemas_refuse_what_the_contract_forbids_and_allow_new_fields() {
    let registry = build(&repo()).unwrap();
    let items = validator(&registry.output, ITEM_SCHEMA);
    let index = validator(&registry.output, INDEX_SCHEMA);
    let led = record(&registry.output, "led-5mm@0.3.0");
    let block = record(&registry.output, "nano-button-led@0.3.1");
    let row: Value = serde_json::from_slice::<Value>(&registry.output["index.json"]).unwrap();

    let mutated = |base: &Value, pointer: &str, value: Value| {
        let mut base = base.clone();
        *base
            .pointer_mut(pointer)
            .unwrap_or_else(|| panic!("{pointer}")) = value;
        base
    };
    let refused = [
        (
            mutated(&led, "/name", json!("../../evil")),
            "a name that is a path",
        ),
        (mutated(&led, "/name", json!("index")), "the index's name"),
        (
            mutated(&led, "/name", json!("con")),
            "a Windows device name",
        ),
        (
            mutated(&led, "/version", json!("banana")),
            "a version that is not numbers",
        ),
        (
            mutated(&led, "/version", json!("+01.2.3")),
            "a version with a sign and a leading zero",
        ),
        (
            mutated(&led, "/format", json!(2)),
            "a format this client does not know",
        ),
        (
            mutated(&led, "/description", Value::Null),
            "null for an absent field",
        ),
        (mutated(&led, "/title", json!("")), "an empty title"),
        (mutated(&led, "/license", json!("")), "an empty licence"),
        (
            mutated(&led, "/hash", json!("sha256:XYZ")),
            "a folder hash that is not hex",
        ),
        (
            mutated(&led, "/files/0/path", json!("../../escape")),
            "a file path that climbs out",
        ),
        (
            mutated(&led, "/files/0/path", json!("/etc/passwd")),
            "an absolute file path",
        ),
        (
            mutated(&led, "/files/0/path", json!("a\\b")),
            "a file path with a backslash",
        ),
        (
            mutated(&led, "/files/0/sha256", json!("not-hex")),
            "a file hash that is not hex",
        ),
        (
            mutated(&led, "/files/0/url", json!("https://evil.example/x")),
            "a file URL on another host",
        ),
        (
            mutated(&led, "/registryDependencies", json!(["a@b@c"])),
            "a dependency outside the grammar",
        ),
        (
            mutated(&block, "/board", json!("Nano Board")),
            "a board that is not an id",
        ),
    ];
    for (value, what) in refused {
        assert!(!items.is_valid(&value), "the item schema takes {what}");
    }
    let mut rows = row.clone();
    rows["items"][0]["url"] = json!("../other.json");
    assert!(
        !index.is_valid(&rows),
        "the index schema takes a row URL outside the registry"
    );
    let mut rows = row.clone();
    rows["name"] = json!("acme");
    assert!(
        !index.is_valid(&rows),
        "the index schema takes a name with no @"
    );

    let mut newer = led.clone();
    newer["aFieldFromLater"] = json!({ "any": "value" });
    newer["files"][0]["mirror"] = json!("x");
    assert!(
        items.is_valid(&newer),
        "a client ignores fields it does not know"
    );
    assert!(items.is_valid(&mutated(
        &led,
        "/registryDependencies",
        json!(["@acme/lamp@1.0.0"])
    )));
    let mut rows = row;
    rows["mirrors"] = json!([]);
    assert!(index.is_valid(&rows));
}

/// Where `value` holds null, as JSON pointers with array indexes as `*`.
fn nulls(value: &Value, at: String, found: &mut Vec<String>) {
    match value {
        Value::Null => found.push(at),
        Value::Array(items) => {
            for item in items {
                nulls(item, format!("{at}/*"), found);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                nulls(item, format!("{at}/{key}"), found);
            }
        }
        _ => {}
    }
}

/// Where a schema allows null, as JSON pointers.
fn nullable(schema: &Value, at: String, found: &mut Vec<String>) {
    match schema {
        Value::String(s) if s == "null" => found.push(at),
        Value::Array(items) => {
            for item in items {
                nullable(item, at.clone(), found);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if key != "description" {
                    nullable(item, format!("{at}/{key}"), found);
                }
            }
        }
        _ => {}
    }
}

#[test]
fn an_absent_field_is_left_out_and_null_is_only_ever_a_value() {
    let registry = build(&repo()).unwrap();
    let mut found = Vec::new();
    for (path, bytes) in &registry.output {
        if path.ends_with(".json") && !path.contains("schema") {
            let value: Value = serde_json::from_slice(bytes).unwrap();
            nulls(&value, String::new(), &mut found);
        }
    }
    found.sort();
    found.dedup();
    assert_eq!(found, ["/params/*/default", "/ratings/*/value"]);

    let mut schema_nulls = Vec::new();
    let schema: Value = serde_json::from_slice(&registry.output[ITEM_SCHEMA]).unwrap();
    nullable(&schema, String::new(), &mut schema_nulls);
    let index: Value = serde_json::from_slice(&registry.output[INDEX_SCHEMA]).unwrap();
    nullable(&index, String::new(), &mut schema_nulls);
    let fields: Vec<&str> = schema_nulls
        .iter()
        .map(|at| &at[..at.match_indices('/').nth(4).map_or(at.len(), |(i, _)| i)])
        .collect();
    assert_eq!(
        fields,
        [
            "/$defs/Param/properties/default",
            "/$defs/Rating/properties/value"
        ],
        "{schema_nulls:?}"
    );
}

/// docs/registry.md's mapping from `orbe part --json` to a part record.
fn card_to_record(card: &Value) -> Value {
    let mut map = card.as_object().unwrap().clone();
    let uses = map.remove("uses").unwrap();
    let deps: Vec<String> = uses
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, version)| format!("{id}@{}", version.as_str().unwrap()))
        .collect();
    map.insert("registryDependencies".into(), json!(deps));
    map.remove("used_by").unwrap();
    let gaps = map.remove("not_simulated").unwrap();
    map.insert("notSimulated".into(), gaps);
    for file in map["files"].as_array_mut().unwrap() {
        let sha = file["sha256"].as_str().unwrap().to_string();
        file["url"] = json!(format!("files/{sha}"));
    }
    map.insert("type".into(), json!("part"));
    map.insert("$schema".into(), json!(ITEM_SCHEMA));
    map.insert("format".into(), json!(1));
    Value::Object(map)
}

/// The same mapping backwards. `used_by` is not in a record, so it does not come back.
fn record_to_card(record: &Value) -> Value {
    let mut map = record.as_object().unwrap().clone();
    for key in ["type", "$schema", "format"] {
        map.remove(key).unwrap();
    }
    let deps = map.remove("registryDependencies").unwrap();
    let uses: serde_json::Map<String, Value> = deps
        .as_array()
        .unwrap()
        .iter()
        .map(|dep| {
            let dep = Dependency::parse(dep.as_str().unwrap()).unwrap();
            (dep.id, json!(dep.version.to_string()))
        })
        .collect();
    map.insert("uses".into(), Value::Object(uses));
    let gaps = map.remove("notSimulated").unwrap();
    map.insert("not_simulated".into(), gaps);
    for file in map["files"].as_array_mut().unwrap() {
        file.as_object_mut().unwrap().remove("url").unwrap();
    }
    Value::Object(map)
}

/// Part ids `registry.jsonc` leaves out of search: the parts a jumper wire cannot land on.
const HIDDEN: &[&str] = &[
    "ams1117-sot223",
    "atmega16u2-qfn32",
    "atmega328p-tqfp32",
    "capacitor-0603",
    "capacitor-electrolytic-smd",
    "capacitor-tantalum-3216",
    "ch340g",
    "cp2102n-qfn28",
    "crystal-3215",
    "diode-cd1206",
    "diode-do41-flush",
    "diode-m7-sma",
    "diode-sod123",
    "esp32s3-qfn56",
    "esp32s3-wroom-1-n16r8",
    "inductor-0805",
    "led-0603",
    "led-0805",
    "lesd5d5-sod523",
    "lmv358-msop8",
    "lp2985-sot23-5",
    "npn-l8050-sot23",
    "nmos-bss138-sot23",
    "pmos-fdn340p-sot23",
    "ptc-fuse-1812",
    "resistor-0603",
    "resistor-array-4x0603",
    "resonator-cstce",
    "sgm2212-sot223",
    "sk68xxmini-hs",
    "tactile-switch-ntc013",
    "tactile-switch-smd",
    "tactile-switch-ts42",
    "usb-micro-b",
    "usb-mini-b",
];

fn unlisted_in_manifest() -> BTreeSet<String> {
    let text = fs::read_to_string(repo().join("registry.jsonc")).unwrap();
    let value: Value =
        jsonc_parser::parse_to_serde_value(&text, &jsonc_parser::ParseOptions::default()).unwrap();
    value["unlisted"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_part_left_out_of_search_is_false_on_its_index_row_and_absent_from_its_record() {
    let registry = build(&repo()).unwrap();
    let index: Value = serde_json::from_slice(&registry.output["index.json"]).unwrap();
    let rows = index["items"].as_array().unwrap();
    let row = |name: &str| rows.iter().find(|row| row["name"] == name).unwrap();
    let left_out = unlisted_in_manifest();
    let want: BTreeSet<String> = HIDDEN.iter().map(|id| (*id).to_string()).collect();
    assert_eq!(left_out, want);
    for name in &left_out {
        assert_eq!(row(name)["listed"], json!(false), "{name}");
        let version = row(name)["version"].as_str().unwrap();
        let record = record(&registry.output, &format!("{name}@{version}"));
        assert!(record.get("listed").is_none(), "{name}");
    }
    for row in rows.iter().filter(|row| row["type"] == "part") {
        let name = row["name"].as_str().unwrap();
        if !left_out.contains(name) {
            assert!(row.get("listed").is_none(), "{name} is offered");
        }
    }
    assert!(row("led-5mm").get("listed").is_none());
}

#[test]
fn a_part_record_maps_to_orbe_part_json_both_ways() {
    let registry = build(&repo()).unwrap();
    for card in orbe_scene::catalog_cards(&repo().join("parts")) {
        let cli = serde_json::to_value(&card).unwrap();
        let record = record(&registry.output, &format!("{}@{}", card.name, card.version));
        assert_eq!(card_to_record(&cli), record, "{} card to record", card.name);
        let mut without_used_by = cli.clone();
        without_used_by.as_object_mut().unwrap().remove("used_by");
        assert_eq!(
            record_to_card(&record),
            without_used_by,
            "{} record to card",
            card.name
        );
    }
}

#[test]
fn a_dependency_has_an_optional_registry_then_an_id_and_a_version() {
    let bare = Dependency::parse("led-5mm@0.1.0").unwrap();
    assert_eq!(
        (bare.registry.as_deref(), bare.id.as_str()),
        (None, "led-5mm")
    );
    let scoped = Dependency::parse("@acme/led-5mm@0.10.2").unwrap();
    assert_eq!(scoped.registry.as_deref(), Some("@acme"));
    assert_eq!(scoped.id, "led-5mm");
    assert_eq!(scoped.version.to_string(), "0.10.2");
    for text in ["led-5mm@0.1.0", "@acme/led-5mm@0.10.2"] {
        assert_eq!(Dependency::parse(text).unwrap().to_string(), text);
    }
    for bad in [
        "led-5mm",
        "led-5mm@01.0.0",
        "@acme/led-5mm",
        "@Acme/led-5mm@0.1.0",
        "acme/led-5mm@0.1.0",
        "@acme/led-5mm@0.1.0@0.2.0",
        "a@b@c",
        "@/led@0.1.0",
    ] {
        assert_eq!(Dependency::parse(bad), None, "{bad}");
    }
}

fn matches(pattern: &str, text: &str) -> bool {
    jsonschema::draft202012::new(&json!({ "type": "string", "pattern": pattern }))
        .unwrap()
        .is_valid(&json!(text))
}

/// The schema patterns and the build's own checks agree, so a client that validates with the
/// schema refuses what the build refuses.
#[test]
fn the_schema_patterns_say_what_the_build_checks() {
    let words = [
        "led-5mm",
        "index",
        "con",
        "com1",
        "lpt9",
        "console",
        "a",
        "Led",
        "5led",
        "led_5",
        "",
        "led-5mm@0.1.0",
        "@acme/led@1.0.0",
        "@acme/led@01.0.0",
        "led@1.2",
        "a@b@c",
        "0.1.0",
        "+01.2.3",
        "1.02.3",
        "10.0.0",
    ];
    for word in words {
        let id = orbe_scene::is_id(word) && !orbe_registry::RESERVED.contains(&word);
        assert_eq!(matches(pattern::ID, word), id, "id {word:?}");
        let version = orbe_scene::Version::parse(word).is_some();
        assert_eq!(matches(pattern::VERSION, word), version, "version {word:?}");
        let dependency = Dependency::parse(word).is_some();
        assert_eq!(
            matches(pattern::DEPENDENCY, word),
            dependency,
            "dependency {word:?}"
        );
    }
    for path in [
        "part.jsonc",
        "firmware/main.ino",
        ".env",
        "models/a b.glb",
        "../escape",
        "/abs",
        "a//b",
        "a/./b",
        "a\\b",
        "a:b",
        "a\u{7}b",
        "trailing.",
        "trailing ",
        "",
    ] {
        assert_eq!(
            matches(pattern::PATH, path),
            path_problem(path).is_none(),
            "path {path:?}"
        );
    }
    assert!(
        path_problem("firmware/AUX.c").is_some(),
        "the build also refuses device names"
    );
}
