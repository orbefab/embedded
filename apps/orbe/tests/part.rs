//! `orbe part` and `orbe search`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

#[path = "../../../packages/scene/tests/common/mod.rs"]
mod common;
#[path = "registry_env.rs"]
mod registry_env;

fn orbe(args: &[&str]) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbe"));
    registry_env::apply(&mut command);
    let output = command.args(args).output().unwrap();
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
    assert!(
        stdout.contains(
            "\nbrightness\n  the average i over 20ms\n  lit while that average is above 2 mA\n  dim while that average is above 100 uA and at or below 2 mA\n  off while that average is at or below 100 uA\n"
        ),
        "{stdout}"
    );
    let (code, stdout, stderr) = orbe(&["part", "led-5mm", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let data = card_json(&stdout);
    assert_eq!(data["lit"]["above"], 0.002);
    assert_eq!(data["lit"]["of"], "i");
    assert_eq!(data["dim"]["above"], 0.0001);
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
fn a_chip_prints_its_summary_categories_and_datasheet_url() {
    let (code, stdout, stderr) = orbe(&["part", "atmega328p-dip28"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("DS40002061B"), "{stdout}");
    assert!(stdout.contains("ATmega328P, PDIP-28"), "{stdout}");
    assert!(
        stdout.contains("ATmega328P-PU in a 300 mil PDIP-28"),
        "{stdout}"
    );
    assert!(
        stdout.contains("https://ww1.microchip.com/downloads/en/DeviceDoc/ATmega48A-PA-88A-PA-168A-PA-328-P-DS-DS40002061B.pdf"),
        "{stdout}"
    );
    assert!(
        stdout.contains("categories  microcontroller, avr, dip"),
        "{stdout}"
    );
}

#[test]
fn an_unknown_id_exits_with_near_matches() {
    let (code, stdout, stderr) = orbe(&["part", "resistor-axil"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("error[unknown_part]: no Block or part named `resistor-axil`\n"));
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
fn search_with_no_words_lists_the_parts_a_scene_can_place() {
    let (code, stdout, stderr) = orbe(&["search"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("\nldr-5mm  "), "{stdout}");
    assert!(stdout.contains("\nresistor-axial  "), "{stdout}");
    assert!(stdout.contains("atmega328p-dip28"), "{stdout}");
    assert!(stdout.contains("led-5mm"), "{stdout}");
    assert!(!stdout.contains("atmega328p-tqfp32"), "{stdout}");
    assert!(!stdout.contains("led-0603"), "{stdout}");
    let blocks = stdout
        .rfind("\nblocks\n")
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(stdout[..blocks].contains("resistor-axial"), "{stdout}");
    assert!(stdout[blocks..].contains("nano-button-led"), "{stdout}");
    assert!(!stdout[blocks..].contains("resistor-axial"), "{stdout}");

    let (code, stdout, stderr) = orbe(&["search", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["ok"], true, "{stdout}");
    let groups = envelope["data"]["groups"].as_array().unwrap();
    assert_eq!(groups.last().unwrap()["category"], "blocks");
    let ids: Vec<&str> = groups
        .iter()
        .flat_map(|group| group["items"].as_array().unwrap())
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"ldr-5mm") && ids.contains(&"resistor-axial"));
    assert!(ids.contains(&"atmega328p-dip28"));
    assert!(ids.contains(&"led-5mm"));
    assert!(!ids.contains(&"atmega328p-tqfp32"));
    assert!(!ids.contains(&"led-0603"));
    assert!(envelope["data"].get("parts").is_none(), "{stdout}");
}

#[test]
fn part_says_how_it_plugs_into_a_breadboard() {
    let text = |id: &str| {
        let (code, stdout, stderr) = orbe(&["part", id]);
        assert_eq!(code, 0, "{stderr}{stdout}");
        stdout
    };
    assert!(
        text("led-5mm").contains(
            "plugs into a breadboard: name one lead's hole with on, such as \"on\": { \"anode\": \"bb.c21\" }, and orbe seats the other lead in the next hole\n"
        ),
        "{}",
        text("led-5mm")
    );
    assert!(
        text("resistor-axial").contains(
            "plugs into a breadboard: name one lead's hole with on, such as \"on\": { \"a\": \"bb.c21\" }, and orbe seats the other lead 4 holes along the row, so the part spans 5 holes\n"
        ),
        "{}",
        text("resistor-axial")
    );
    assert!(
        text("ldr-5mm").contains(
            "plugs into a breadboard: name one lead's hole with on, such as \"on\": { \"a\": \"bb.c21\" }, and orbe seats the other lead in the next hole\n"
        ),
        "{}",
        text("ldr-5mm")
    );
    assert!(
        text("led-0603")
            .contains("does not plug into a breadboard: it has no leads that go into a hole\n"),
        "{}",
        text("led-0603")
    );
    assert!(
        text("breadboard-400").contains("it is a breadboard; other parts plug into its holes\n"),
        "{}",
        text("breadboard-400")
    );
    assert!(
        text("nano-board").contains("plugs into a breadboard"),
        "{}",
        text("nano-board")
    );
    assert!(
        text("npn-2n2222-to92").contains(
            "plugs into a breadboard: name one pin's hole with on, such as \"on\": { \"collector\": \"bb.c21\" }, and orbe seats the other leads\n"
        ),
        "{}",
        text("npn-2n2222-to92")
    );
    assert!(
        text("pnp-2n3906-to92").contains(
            "plugs into a breadboard: name one pin's hole with on, such as \"on\": { \"emitter\": \"bb.c21\" }, and orbe seats the other leads\n"
        ),
        "{}",
        text("pnp-2n3906-to92")
    );
    assert!(
        text("zener-1n4733a").contains(
            "plugs into a breadboard: name one lead's hole with on, such as \"on\": { \"anode\": \"bb.c21\" }, and orbe seats the other lead 4 holes along the row, so the part spans 5 holes\n"
        ),
        "{}",
        text("zener-1n4733a")
    );
    assert!(
        text("esp32s3-devkitc-1").contains(
            "plugs into a breadboard: name one pin's hole with on, such as \"on\": { \"GPIO3\": \"bb.c21\" }, \"rotate\": [0, 0, 90], and orbe seats the other leads\n"
        ),
        "{}",
        text("esp32s3-devkitc-1")
    );
    assert!(
        text("neopixel-stick-8").contains(
            "plugs into a breadboard: name one pin's hole with on, such as \"on\": { \"GND\": \"bb.c21\" }, \"rotate\": [0, 0, 90], and orbe seats the other leads\n"
        ),
        "{}",
        text("neopixel-stick-8")
    );
    assert!(
        text("tactile-switch-6mm").contains(
            "plugs into a breadboard: name one pin's hole with on, such as \"on\": { \"b\": \"bb.d1\" }, \"rotate\": [0, 0, 90], and orbe seats the other leads\n"
        ),
        "{}",
        text("tactile-switch-6mm")
    );

    let (code, stdout, stderr) = orbe(&["part", "led-5mm", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stdout.contains("plugs into a breadboard"), "{stdout}");
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
    assert_eq!(names[0], "led-3mm");
    assert!(!names.contains(&"led-0603"), "{stdout}");
    assert!(!names.contains(&"led-0805"), "{stdout}");
    assert!(!names.contains(&"sk68xxmini-hs"), "{stdout}");
    let led = names.iter().position(|name| *name == "led-5mm").unwrap();
    let board = names
        .iter()
        .position(|name| *name == "esp32s3-devkitc-1")
        .unwrap();
    assert!(led < board);

    let (code, stdout, stderr) = orbe(&["search", "blue", "led"]);
    assert_eq!(code, 0, "{stderr}");
    let blue: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(blue, ["led-3mm-blue", "led-5mm-blue"]);

    let (code, stdout, _) = orbe(&["search", "led-5mm", "--json"]);
    assert_eq!(code, 0);
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["data"]["parts"][0]["id"], "led-5mm");

    let (code, stdout, stderr) = orbe(&["search", "zzzz-no-such-part"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "no parts match\n");
    let (code, stdout, stderr) = orbe(&["search", "potentiometer", "knob", "dimmer"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        "no parts match\nhint: every word must match; try one word, such as 'orbe search potentiometer'\n"
    );
    let (code, stdout, _) = orbe(&["search", "zzzz-no-such-part", "--json"]);
    assert_eq!(code, 0);
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["data"]["parts"].as_array().unwrap().len(), 0);
    assert_eq!(envelope["data"]["blocks"].as_array().unwrap().len(), 0);
}

#[test]
fn search_finds_a_block_and_part_describes_it() {
    let (code, stdout, stderr) = orbe(&["search", "fade"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "nano-led-fade  LED fade on a Nano\n");
    let (code, stdout, _) = orbe(&["search", "fade", "--json"]);
    assert_eq!(code, 0);
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["data"]["parts"].as_array().unwrap().len(), 0);
    assert_eq!(envelope["data"]["blocks"][0]["id"], "nano-led-fade");

    let (code, stdout, stderr) = orbe(&["blocks"]);
    assert_eq!(code, 0, "{stderr}");
    for id in common::block_ids() {
        assert!(stdout.contains(&id), "{stdout}");
    }

    let (code, stdout, stderr) = orbe(&["part", "nano-led-fade"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("LED fade on a Nano\n"), "{stdout}");
    assert!(
        stdout.contains("An LED on D9 of an Arduino Nano fades up and down by PWM.\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("a Block. install it with `orbe add nano-led-fade` in an empty folder\n"),
        "{stdout}"
    );
    let (code, stdout, stderr) = orbe(&["part", "fade"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("nano-led-fade  "), "{stdout}");

    let (code, stdout, stderr) = orbe(&["part", "nano-led-fad"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("did you mean `nano-led-fade`"), "{stderr}");
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
    assert!(!names.contains(&"tactile-switch-smd"), "{stdout}");
    assert!(!names.contains(&"tactile-switch-ts42"), "{stdout}");
    assert!(!names.contains(&"diode-cd1206"), "{stdout}");
    let rest = names
        .iter()
        .position(|name| *name == "barrel-jack-dc21")
        .unwrap();
    for name in [
        "nmos-irf520-to220",
        "npn-2n2222-to92",
        "pmos-irf9540n-to220",
        "tactile-switch-6mm",
    ] {
        let at = names.iter().position(|found| *found == name).unwrap();
        assert!(
            at < rest,
            "{name} at {at}, barrel-jack-dc21 at {rest}\n{stdout}"
        );
    }

    let (code, stdout, stderr) = orbe(&["search", "board"]);
    assert_eq!(code, 0, "{stderr}");
    let names = names_of(&stdout);
    assert_eq!(&names[..2], ["nano-board", "uno-board"]);
}

#[test]
fn search_for_crystal_finds_the_hc49us_and_hides_crystal_3215_that_the_nano_installs() {
    let (code, stdout, stderr) = orbe(&["search", "crystal"]);
    assert_eq!(code, 0, "{stderr}");
    let names = names_of(&stdout);
    assert!(!names.contains(&"crystal-3215"), "{stdout}");
    assert!(names.contains(&"crystal-hc49us"), "{stdout}");

    let (code, stdout, stderr) = orbe(&["part", "crystal-3215"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("crystal-3215"), "{stdout}");

    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbe"));
    registry_env::apply(&mut command);
    let output = command
        .args(["add", "nano-board"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert!(
        dir.path()
            .join("parts/nano-board/parts/crystal-3215/part.jsonc")
            .is_file(),
        "{stdout}"
    );
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
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("tactile-switch-6mm  "), "{stdout}");

    let (code, stdout, stderr) = orbe(&["part", "capacitor"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("error[unknown_part]: no Block or part named `capacitor`\n"));
    for name in ["capacitor-ceramic-disc", "capacitor-electrolytic-radial"] {
        assert!(stderr.contains(&format!("`{name}`")), "{stderr}");
    }
    for hidden in [
        "capacitor-0603",
        "capacitor-electrolytic-smd",
        "capacitor-tantalum-3216",
    ] {
        assert!(!stderr.contains(hidden), "{stderr}");
    }

    let (code, stdout, stderr) = orbe(&["part", "capacitor", "--json"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "unknown_part");
    let hint = envelope["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("capacitor-ceramic-disc"), "{hint}");
    assert!(hint.contains("capacitor-electrolytic-radial"), "{hint}");
    assert!(!hint.contains("capacitor-0603"), "{hint}");
}

#[test]
fn unknown_id_suggestions_skip_unlisted_parts() {
    let dir = tempfile::tempdir().unwrap();
    let run_add = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orbe"));
        registry_env::apply(&mut command);
        let output = command.args(args).current_dir(dir.path()).output().unwrap();
        (
            output.status.code().unwrap(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };

    let (code, stdout, stderr) = run_add(&["add", "esp32"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr,
        "error[unknown_part]: no Block or part named `esp32`\n  hint: did you mean `esp32s3-devkitc-1`?\n"
    );

    let (code, stdout, stderr) = run_add(&["add", "esp32", "--json"]);
    assert_eq!(code, 1, "{stderr}");
    let envelope: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "unknown_part");
    assert_eq!(
        envelope["error"]["hint"],
        "did you mean `esp32s3-devkitc-1`?"
    );

    let (code, stdout, stderr) = orbe(&["part", "esp32"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr,
        "error[unknown_part]: no Block or part named `esp32`\n  hint: did you mean `esp32s3-devkitc-1`?\n"
    );

    let (code, stdout, stderr) = orbe(&["part", "esp32s3-qfn56"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("esp32s3-qfn56  "), "{stdout}");
}

#[test]
fn a_null_rating_prints_unrated() {
    let (code, stdout, stderr) = orbe(&["part", "diode-1n4007"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("power  unrated\n"), "{stdout}");
    assert!(!stdout.contains("current  unrated\n"), "{stdout}");
    let (code, stdout, stderr) = orbe(&["part", "resistor-0603", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let ratings = &card_json(&stdout)["ratings"];
    assert_eq!(ratings.as_array().unwrap().len(), 1);
    assert_eq!(ratings[0]["name"], "power");
    assert!(ratings[0]["value"].is_null());
}

/// Every catalog part whose card prints a breadboard seating line seats as printed on a
/// breadboard-830 (or the board the card names) with no layout error and no placement warning.
#[test]
fn every_part_card_breadboard_line_seats_as_printed() {
    let repo = common::repo();
    let parts_dir = repo.join("parts");
    let mut ids: Vec<String> = fs::read_dir(&parts_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| parts_dir.join(id).join("part.jsonc").is_file())
        .collect();
    ids.sort();

    let cache = tempfile::tempdir().unwrap();
    let registry = registry_env::template();
    let run_part = |args: &[&str]| -> (i32, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
            .args(args)
            .env("ORBE_REGISTRY", &registry)
            .env("XDG_CACHE_HOME", cache.path())
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };

    let project = common::project("nano-button");
    let devkitc_build = repo.join("blocks/devkitc-button-led/build");
    fs::copy(
        devkitc_build.join("button.ino.bin"),
        project.path().join("build/button.ino.bin"),
    )
    .unwrap();
    fs::copy(
        devkitc_build.join("button.ino.partitions.bin"),
        project.path().join("build/button.ino.partitions.bin"),
    )
    .unwrap();

    let mut checked = Vec::new();
    let mut failures = Vec::new();
    for id in &ids {
        let (code, stdout, stderr) = run_part(&["part", id]);
        assert_eq!(code, 0, "{id}: {stderr}");
        let Some(line) = stdout
            .lines()
            .find(|line| line.starts_with("plugs into a breadboard:"))
        else {
            continue;
        };
        checked.push(id.as_str());

        let Some(after) = line.split("such as ").nth(1) else {
            failures.push(format!("{id}: no `such as ` in `{line}`"));
            continue;
        };
        let Some(snippet) = after.split(", and orbe seats").next() else {
            failures.push(format!("{id}: no `, and orbe seats` in `{line}`"));
            continue;
        };
        let layout_entry: Value = match serde_json::from_str(&format!("{{ {snippet} }}")) {
            Ok(value) => value,
            Err(err) => {
                failures.push(format!("{id}: invalid layout snippet `{snippet}`: {err}"));
                continue;
            }
        };

        let board = if line.contains("breadboard-400") {
            "breadboard-400"
        } else {
            "breadboard-830"
        };

        let card = serde_json::to_value(orbe_scene::card(&parts_dir, id).unwrap()).unwrap();
        let mut part_spec = serde_json::Map::new();
        part_spec.insert("type".into(), Value::String(id.clone()));
        if let Some(params) = card["params"].as_array() {
            for param in params {
                if param["required"].as_bool() != Some(true) {
                    continue;
                }
                let name = param["name"].as_str().unwrap();
                let value = match name {
                    "firmware" => Value::String("build/button.ino.hex".into()),
                    "app" => Value::String("build/button.ino.bin".into()),
                    "partitions" => Value::String("build/button.ino.partitions.bin".into()),
                    _ => param["example"].clone(),
                };
                assert!(
                    !value.is_null(),
                    "{id}: required param `{name}` has no example in card"
                );
                part_spec.insert(name.into(), value);
            }
        }

        let part_text = fs::read_to_string(parts_dir.join(id).join("part.jsonc")).unwrap();
        let has_ground = orbe_scene::names_ground(&part_text);
        let mut parts = serde_json::Map::new();
        parts.insert("bb".into(), serde_json::json!({ "type": board }));
        parts.insert("dut".into(), Value::Object(part_spec));
        let mut layout = serde_json::Map::new();
        layout.insert("bb".into(), serde_json::json!({ "at": [0, 0, 0] }));
        layout.insert("dut".into(), layout_entry);
        let mut scene = serde_json::Map::new();
        scene.insert("format".into(), serde_json::json!(1));
        if !has_ground {
            parts.insert(
                "ref_gnd".into(),
                serde_json::json!({ "type": "resistor-axial", "resistance": "10k" }),
            );
            layout.insert("ref_gnd".into(), serde_json::json!({ "at": [200, 0, 0] }));
            scene.insert("ground".into(), serde_json::json!("ref_gnd.a"));
        }
        scene.insert("parts".into(), Value::Object(parts));
        scene.insert("layout".into(), Value::Object(layout));

        let scene_path = project.path().join("scene.jsonc");
        fs::write(&scene_path, serde_json::to_string_pretty(&scene).unwrap()).unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
            .args(["lint", "scene.jsonc", "--json"])
            .current_dir(project.path())
            .output()
            .unwrap();
        let lint_out = String::from_utf8(output.stdout).unwrap();
        let lint_err = String::from_utf8(output.stderr).unwrap();
        let envelope: Value = serde_json::from_str(&lint_out)
            .unwrap_or_else(|e| panic!("{id}: bad lint json ({e}): {lint_out}{lint_err}"));
        if envelope["ok"] != true {
            failures.push(format!("{id}: failed to load ({line})\n  {lint_out}"));
            continue;
        }
        let bad: Vec<&Value> = envelope["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| {
                matches!(
                    d["code"].as_str(),
                    Some(
                        "scene.unplaced" | "scene.shorted" | "scene.covered_hole" | "scene.overlap"
                    )
                )
            })
            .collect();
        if !bad.is_empty() {
            failures.push(format!(
                "{id}: lint warnings ({line})\n  {}",
                serde_json::to_string_pretty(&bad).unwrap()
            ));
        }
    }

    assert!(
        checked.contains(&"esp32s3-devkitc-1")
            && checked.contains(&"tactile-switch-6mm")
            && checked.contains(&"neopixel-stick-8"),
        "checked parts: {checked:?}"
    );
    assert!(
        failures.is_empty(),
        "{} part card(s) failed to seat as printed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
