//! `--previous`: a new build keeps serving every record an earlier build served.

mod common;

use std::fs;
use std::path::Path;

use common::{LAMP, put, third_party, written};
use orbe_registry::{Error, build};
use serde_json::Value;

fn lamp_at(src: &Path, version: &str) {
    put(
        src,
        "parts/lamp/part.jsonc",
        &LAMP.replace("1.2.0", version),
    );
    put(
        src,
        "blocks/lamp-demo/block.jsonc",
        &format!(
            r#"{{ "format": 1, "block": "lamp-demo", "version": "{version}", "title": "A lamp", "board": "lamp", "license": "MIT" }}"#
        ),
    );
}

fn versions(index: &Value, name: &str) -> Vec<String> {
    index["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["name"] == name)
        .map(|row| row["version"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_new_version_keeps_the_old_one_served_and_orders_versions_by_number() {
    let src = third_party();
    lamp_at(src.path(), "0.9.0");
    let first = src.path().join("first");
    build(src.path(), None).unwrap().write(&first).unwrap();

    lamp_at(src.path(), "0.10.0");
    let second = src.path().join("second");
    let registry = build(src.path(), Some(&first)).unwrap();
    assert_eq!(registry.report.carried, 2);
    registry.write(&second).unwrap();

    let index: Value =
        serde_json::from_slice(&fs::read(second.join("index.json")).unwrap()).unwrap();
    assert_eq!(versions(&index, "lamp"), ["0.9.0", "0.10.0"]);
    assert_eq!(versions(&index, "lamp-demo"), ["0.9.0", "0.10.0"]);
    let old = written(&first);
    let new = written(&second);
    for (path, bytes) in &old {
        if path.ends_with("index.json") {
            continue;
        }
        assert!(new.get(path) == Some(bytes), "{path} is served unchanged");
    }

    let third = src.path().join("third");
    let again = build(src.path(), Some(&second)).unwrap();
    assert_eq!(again.report.carried, 2, "a carried record carries again");
    again.write(&third).unwrap();
    assert_eq!(
        written(&third),
        new,
        "a build with nothing new changes nothing"
    );
}

#[test]
fn a_changed_record_at_a_served_version_is_refused() {
    let src = third_party();
    let first = src.path().join("first");
    build(src.path(), None).unwrap().write(&first).unwrap();
    put(
        src.path(),
        "parts/lamp/part.jsonc",
        &LAMP.replace("\"Lamp\"", "\"A brighter lamp\""),
    );
    let refused = build(src.path(), Some(&first)).err().unwrap();
    let Error::Invalid(message) = refused else {
        panic!("{refused}")
    };
    assert!(
        message
            .contains("`lamp@1.2.0` builds to a record other than the one served at that version"),
        "{message}"
    );
    assert!(
        message.contains("give the change a new version"),
        "{message}"
    );
}

#[test]
fn a_previous_build_that_does_not_check_out_is_refused() {
    let src = third_party();
    let first = src.path().join("first");
    build(src.path(), None).unwrap().write(&first).unwrap();
    lamp_at(src.path(), "2.0.0");
    let refused = |dir: &Path| match build(src.path(), Some(dir)) {
        Err(Error::Invalid(message)) => message,
        other => panic!("{:?}", other.map(|r| r.report)),
    };

    let record = first.join("lamp@1.2.0.json");
    let bytes = fs::read(&record).unwrap();
    fs::write(&record, [bytes.as_slice(), b" "].concat()).unwrap();
    assert!(refused(&first).contains("does not match the hash and size its index row gives"));
    fs::write(&record, &bytes).unwrap();

    let file = fs::read_dir(first.join("files"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&file, "other bytes").unwrap();
    assert!(refused(&first).contains("does not match the hash and size"));

    let other = third_party();
    put(
        other.path(),
        "registry.jsonc",
        r#"{ "format": 1, "name": "@other" }"#,
    );
    let theirs = other.path().join("out");
    build(other.path(), None).unwrap().write(&theirs).unwrap();
    assert!(
        refused(&theirs).contains("it is the registry `@other`, and registry.jsonc names `@acme`")
    );
}
