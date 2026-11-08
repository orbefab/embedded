//! `orbe part` and `orbe search`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn orbe(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    (output.status.code().unwrap(), stdout, stderr)
}

fn card_json(stdout: &str) -> Value {
    let envelope: Value = serde_json::from_str(stdout).unwrap();
    assert_eq!(envelope["ok"], true, "{stdout}");
    envelope["data"].clone()
}

#[test]
fn part_json_matches_the_checked_in_cards() {
    for id in ["resistor-axial", "atmega328p-dip28", "nano-board"] {
        let (code, stdout, stderr) = orbe(&["part", id, "--json"]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.is_empty());
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/cards/{id}.json"));
        let expected: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(card_json(&stdout), expected, "{id}");
    }
}

#[test]
fn a_chip_with_no_summary_prints_its_datasheet_name_and_no_url() {
    let (code, stdout, stderr) = orbe(&["part", "atmega328p-dip28"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("DS40002061B"), "{stdout}");
    assert!(stdout.contains("ATmega328P, PDIP-28"), "{stdout}");
    assert!(!stdout.to_lowercase().contains("http"), "{stdout}");
    assert!(!stdout.contains("categories"), "{stdout}");
}

#[test]
fn an_unknown_id_exits_with_near_matches() {
    let (code, stdout, stderr) = orbe(&["part", "resistor-axil"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("error[unknown_part]: no catalog part named `resistor-axil`\n"));
    assert!(stderr.contains("`resistor-axial`"), "{stderr}");

    let (code, stdout, stderr) = orbe(&["part", "LED-5MM", "--json"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "unknown_part");
    assert!(
        envelope["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("led-5mm")
    );
}

#[test]
fn search_prints_title_matches_before_the_rest() {
    let (code, stdout, stderr) = orbe(&["search", "led"]);
    assert_eq!(code, 0, "{stderr}");
    let names: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(names[0], "led-0603");
    let led = names.iter().position(|name| *name == "led-5mm").unwrap();
    let nano = names.iter().position(|name| *name == "nano-board").unwrap();
    assert!(led < nano);

    let (code, stdout, _) = orbe(&["search", "led-5mm", "--json"]);
    assert_eq!(code, 0);
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["data"]["parts"][0]["name"], "led-5mm");

    let (code, stdout, stderr) = orbe(&["search", "zzzz-no-such-part"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "no parts match\n");
    let (code, stdout, _) = orbe(&["search", "zzzz-no-such-part", "--json"]);
    assert_eq!(code, 0);
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["data"]["parts"].as_array().unwrap().len(), 0);
}
