//! The Block `nano-oled`'s sketch on its OLED module behind a SparkFun BSS138 level shifter.
//! The Nano's 5V feeds the shifter's HV and the module's VCC, its 3V3 the shifter's LV. A4 and
//! A5 are HV1 and HV2, and LV1 and LV2 are the module's SDA and SCL. The LV side then has the
//! shifter's 10 kohm and the module's own 4.7 kohm pull-ups, to two 3.3 V rails, and the FETs'
//! gates are on the CH340G's V3 with its 0.1 uF.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_session::{Files, test};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

const SCENE: &str = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/hello.ino.hex" },
    "oled": { "type": "oled-ssd1306-128x64" },
    "shifter": { "type": "bss138-level-shifter" }
  },
  "nets": { "GND": "nano.GND" },
  "wires": [
    ["nano.GND", "oled.GND"],
    ["nano.GND", "shifter.GND"],
    ["nano.5V", "shifter.HV"],
    ["nano.5V", "oled.VCC"],
    ["nano.3V3", "shifter.LV"],
    ["nano.A4", "shifter.HV1"],
    ["nano.A5", "shifter.HV2"],
    ["shifter.LV1", "oled.SDA"],
    ["shifter.LV2", "oled.SCL"]
  ]
}
"#;

/// The scene in a Block folder beside the Block's card, image and picture, in memory, with the
/// repo's parts as the catalog a Block reads.
fn files() -> Files {
    let block = repo().join("blocks/nano-oled");
    let read = |path: &str| fs::read(block.join(path)).unwrap();
    let files = vec![
        (PathBuf::from("shifter/block.jsonc"), read("block.jsonc")),
        (
            PathBuf::from("shifter/scene.jsonc"),
            SCENE.as_bytes().to_vec(),
        ),
        (
            PathBuf::from("shifter/build/hello.ino.hex"),
            read("build/hello.ino.hex"),
        ),
        (
            PathBuf::from("shifter/tests/hello.screen"),
            read("tests/hello.screen"),
        ),
    ];
    let parts = repo().join("parts");
    let mut catalog = Vec::new();
    walk(&parts, &parts, &mut catalog);
    Files::memory(files).with_catalog(catalog)
}

/// Each line of `text` played on the scene, as text, and whether every line passed.
/// The BSS138 level shifter runs near the 30 s default under parallel CI load, so this gives
/// it headroom.
fn play(text: &str) -> (bool, Vec<String>) {
    play_with_budget(text, std::time::Duration::from_secs(120))
}

fn play_with_budget(text: &str, budget: std::time::Duration) -> (bool, Vec<String>) {
    let report = test::run_with(
        files(),
        Path::new("shifter/tests/shifter.test"),
        text,
        Path::new("shifter"),
        budget,
        &mut |_| {},
    );
    let lines = report
        .lines
        .iter()
        .map(|line| {
            let reply = serde_json::to_string(&line.reply).unwrap();
            format!("{} {reply}", line.text)
        })
        .collect();
    (report.passed(), lines)
}

/// Adafruit SSD1306's `begin` through the shifter, from 122 ms, where the sketch's delay ends,
/// to the display on by 125 ms, with no diagnostic. Before the CH340G's V3 capacitor the gates'
/// rail was the V3 regulator alone, and an SCL fall at 122.23 ms pulled every FET node with it,
/// A4 to -1.95 V. Before the solver weighed each pivot against its row, the CH340G's idle DTR
/// current, about 5 pA beside the module's 1 uF on 5V, kept Newton from converging there.
#[test]
fn begin_through_a_level_shifter_turns_the_display_on() {
    let (passed, lines) =
        play("open\nstep 125ms\nget @oled.display.screen\nexpect no diagnostics\n");
    assert!(passed, "{lines:#?}");
    let screen = lines.iter().find(|line| line.starts_with("get ")).unwrap();
    assert!(screen.contains(r#""on":true"#), "{screen}");
}

/// The Block's whole picture through the shifter, with no diagnostic. Every I2C edge through
/// the shifter's 10 kohm and the BSS138 capacitances takes some 395 steps, so the frame's
/// 1024 bytes take about five minutes.
#[test]
#[ignore = "about five minutes: run it with --run-ignored only"]
fn the_block_picture_draws_on_the_module_behind_a_level_shifter() {
    let (passed, lines) = play_with_budget(
        "open\nstep 200ms\nexpect @oled.display.screen shows tests/hello.screen\n\
         expect no diagnostics\n",
        std::time::Duration::from_secs(20 * 60),
    );
    assert!(passed, "{lines:#?}");
}

fn walk(base: &Path, path: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            walk(base, &entry.unwrap().path(), out);
        }
    } else if path.is_file() {
        let key = path
            .strip_prefix(base)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        out.push((key, fs::read(path).unwrap()));
    }
}
