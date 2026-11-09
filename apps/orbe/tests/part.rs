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

/// File hashes, byte counts, the folder hash and `used_by` change when a comment or another
/// part changes. The checked-in cards keep the rest of the record.
fn without_volatile(mut value: Value) -> Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    object.remove("hash");
    object.remove("used_by");
    if let Some(files) = object.get_mut("files").and_then(Value::as_array_mut) {
        for file in files {
            if let Some(file) = file.as_object_mut() {
                file.remove("sha256");
                file.remove("bytes");
            }
        }
    }
    value
}

#[test]
fn part_json_matches_the_checked_in_cards() {
    for id in ["resistor-axial", "atmega328p-dip28", "nano-board"] {
        let (code, stdout, stderr) = orbe(&["part", id, "--json"]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.is_empty());
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/cards/{id}.json"));
        let expected: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let got = without_volatile(card_json(&stdout));
        let expected = without_volatile(expected);
        assert_eq!(
            got,
            expected,
            "{id}\nexpected:\n{}\ngot:\n{}",
            serde_json::to_string_pretty(&expected).unwrap(),
            serde_json::to_string_pretty(&got).unwrap()
        );
    }
}

#[test]
fn a_part_prints_where_to_buy_it_and_a_part_without_buy_says_none() {
    let (code, stdout, stderr) = orbe(&["part", "led-5mm"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("\nbuy\n"), "{stdout}");
    assert!(
        stdout.contains("  DigiKey  https://www.digikey.com/en/products/detail/kingbright/WP7113ID/1747663  2026-11-08\n"),
        "{stdout}"
    );
    let (code, stdout, stderr) = orbe(&["part", "led-5mm", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let data = card_json(&stdout);
    assert_eq!(
        data["buy"],
        serde_json::json!([
            {
                "seller": "DigiKey",
                "url": "https://www.digikey.com/en/products/detail/kingbright/WP7113ID/1747663",
                "checked": "2026-11-08"
            }
        ])
    );

    let (code, stdout, stderr) = orbe(&["part", "atmega328p-dip28"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("\nbuy\n  none\n"), "{stdout}");
    let (code, stdout, stderr) = orbe(&["part", "atmega328p-dip28", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(card_json(&stdout).get("buy").is_none());
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

fn names_of(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split_whitespace().next().unwrap())
        .collect()
}

#[test]
fn search_lists_switches_and_boards_before_summary_matches() {
    let (code, stdout, stderr) = orbe(&["search", "switch"]);
    assert_eq!(code, 0, "{stderr}");
    let names = names_of(&stdout);
    let diode = names
        .iter()
        .position(|name| *name == "diode-cd1206")
        .unwrap();
    for name in [
        "nmos-irf520-to220",
        "npn-2n2222-to92",
        "tactile-switch-6mm",
        "tactile-switch-smd",
        "tactile-switch-ts42",
    ] {
        let at = names.iter().position(|found| *found == name).unwrap();
        assert!(
            at < diode,
            "{name} at {at}, diode-cd1206 at {diode}\n{stdout}"
        );
    }

    let (code, stdout, stderr) = orbe(&["search", "board"]);
    assert_eq!(code, 0, "{stderr}");
    let names = names_of(&stdout);
    assert_eq!(&names[..2], ["nano-board", "uno-board"]);
}

#[test]
fn a_unique_id_segment_prints_that_part_and_several_list_like_an_unknown_id() {
    let (code, stdout, stderr) = orbe(&["part", "1n4007"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("diode-1n4007  "), "{stdout}");
    assert!(stdout.contains("license  MIT OR Apache-2.0\n"), "{stdout}");
    assert!(stdout.contains("\nhash  sha256:"), "{stdout}");
    assert!(stdout.contains("power  unrated\n"), "{stdout}");

    let (code, stdout, stderr) = orbe(&["part", "1N4007", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let data = card_json(&stdout);
    assert_eq!(data["name"], "diode-1n4007");
    assert_eq!(data["license"], "MIT OR Apache-2.0");
    assert!(data["hash"].as_str().unwrap().starts_with("sha256:"));
    assert!(
        data["ratings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rating| rating["name"] == "power" && rating["value"].is_null())
    );

    let (code, stdout, stderr) = orbe(&["part", "switch"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("error[unknown_part]: no catalog part named `switch`\n"));
    for name in [
        "tactile-switch-6mm",
        "tactile-switch-smd",
        "tactile-switch-ts42",
    ] {
        assert!(stderr.contains(&format!("`{name}`")), "{stderr}");
    }

    let (code, stdout, stderr) = orbe(&["part", "switch", "--json"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "unknown_part");
    let hint = envelope["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("tactile-switch-6mm"), "{hint}");
    assert!(hint.contains("tactile-switch-smd"), "{hint}");
}

#[test]
fn a_null_rating_prints_unrated() {
    let (code, stdout, stderr) = orbe(&["part", "led-3mm"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("current  unrated\n"), "{stdout}");
    assert!(stdout.contains("power  unrated\n"), "{stdout}");
    let (code, stdout, stderr) = orbe(&["part", "resistor-0603", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let ratings = &card_json(&stdout)["ratings"];
    assert_eq!(ratings.as_array().unwrap().len(), 1);
    assert_eq!(ratings[0]["name"], "power");
    assert!(ratings[0]["value"].is_null());
}
