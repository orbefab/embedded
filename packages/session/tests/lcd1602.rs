//! The parallel 16x2 LCD (`lcd1602`) on a Nano: LiquidCrystal's eight-pin bus, and V0 left open.

use std::path::{Path, PathBuf};

use orbe_session::{Command, Files, Live};
use serde_json::{Value, json};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalog() -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}/"), out);
            } else {
                out.push((format!("{prefix}{name}"), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo().join("parts"), "", &mut files);
    files
}

/// A Nano and an `lcd1602` with R/W, VSS and K on ground, VDD on 5V, A on 5V through 220 ohm
/// and RS and E on D12 and D11, plus `wires`. `contrast` puts a 10 kohm trimmer's wiper on V0
/// near 0.9 V.
fn open(hex: &str, wires: &[[&str; 2]], contrast: bool) -> Live {
    let mut parts = json!({
        "nano": { "type": "nano-board", "firmware": "build/lcd.hex" },
        "lcd": { "type": "lcd1602" },
        "r1": { "type": "resistor-axial", "resistance": "220" },
    });
    let mut all = vec![
        ["lcd.VSS", "nano.GND"],
        ["lcd.RW", "nano.GND"],
        ["lcd.K", "nano.GND"],
        ["lcd.VDD", "nano.5V"],
        ["r1.a", "nano.5V"],
        ["r1.b", "lcd.A"],
        ["lcd.RS", "nano.D12"],
        ["lcd.E", "nano.D11"],
    ];
    if contrast {
        parts["trim"] = json!({ "type": "trimmer-3362p", "resistance": "10k", "position": 0.1 });
        all.extend([
            ["trim.a", "nano.GND"],
            ["trim.b", "nano.5V"],
            ["trim.wiper", "lcd.V0"],
        ]);
    }
    all.extend_from_slice(wires);
    let scene = json!({ "format": 1, "parts": parts, "wires": all });
    let block = json!({
        "format": 1,
        "block": "test",
        "version": "0.0.0",
        "title": "test",
        "board": "nano-board",
        "license": "MIT OR Apache-2.0"
    });
    let hex = std::fs::read(repo().join(hex)).unwrap();
    let files = Files::memory([
        ("block.jsonc".to_string(), block.to_string().into_bytes()),
        ("build/lcd.hex".to_string(), hex),
        ("scene.jsonc".to_string(), scene.to_string().into_bytes()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let reply = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(reply.ok(), "{}", reply.text());
    live
}

fn ok(live: &mut Live, line: &str) -> String {
    let reply = live.run(&Command::parse(line, Path::new("")).unwrap().unwrap());
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.text()
}

/// The rule each diagnostic names, in the order the snapshot lists them.
fn rules(live: &mut Live) -> Vec<String> {
    let reply = live.run(&Command::parse("snapshot", Path::new("")).unwrap().unwrap());
    let diagnostics = reply.json()["data"]["diagnostics"].clone();
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d: &Value| {
            let message = d["message"].as_str().unwrap();
            let rule = message.split("breaking rule ").nth(1).unwrap_or(message);
            format!(
                "{} {}",
                d["code"].as_str().unwrap(),
                rule.split_whitespace().next().unwrap()
            )
        })
        .collect()
}

#[test]
fn eight_bit_constructor_prints_hello_world() {
    let bus = [
        ["lcd.D0", "nano.D9"],
        ["lcd.D1", "nano.D8"],
        ["lcd.D2", "nano.D7"],
        ["lcd.D3", "nano.D6"],
        ["lcd.D4", "nano.D5"],
        ["lcd.D5", "nano.D4"],
        ["lcd.D6", "nano.D3"],
        ["lcd.D7", "nano.D2"],
    ];
    let mut live = open(
        "packages/engine/tests/firmware/sketch/liquidcrystal_8bit.hex",
        &bus,
        true,
    );
    ok(&mut live, "step 1500ms");
    ok(&mut live, "expect @lcd.text line 0 \"hello, world!\"");
    ok(&mut live, "expect @lcd.text line 1 \"1\"");
    ok(&mut live, "step 1000ms");
    ok(&mut live, "expect @lcd.text line 1 \"2\"");
    // The constructor's own begin lands inside the reset wait, as in the 4-bit Block.
    assert_eq!(rules(&mut live), ["chip.protocol init_wait"]);
}

#[test]
fn an_open_v0_leaves_the_glass_blank_and_warns_contrast() {
    let bus = [
        ["lcd.D4", "nano.D5"],
        ["lcd.D5", "nano.D4"],
        ["lcd.D6", "nano.D3"],
        ["lcd.D7", "nano.D2"],
    ];
    let mut live = open("blocks/nano-lcd1602/build/hello.ino.hex", &bus, false);
    ok(&mut live, "step 1500ms");
    let volts = |live: &mut Live, pin: &str| -> f64 {
        let text = ok(live, &format!("get @lcd.{pin}"));
        let words: Vec<&str> = text.split_whitespace().collect();
        words[words.len() - 2].parse().unwrap()
    };
    let (v0, vdd) = (volts(&mut live, "V0"), volts(&mut live, "VDD"));
    assert!(
        vdd - v0 < 0.01,
        "the bias ladder holds an open V0 at VDD, {v0} of {vdd} V"
    );
    ok(&mut live, "expect @lcd.backlight lit");
    ok(&mut live, "expect @lcd.text line 0 \"\"");
    ok(&mut live, "expect @lcd.text line 1 \"\"");
    assert_eq!(
        rules(&mut live),
        ["chip.protocol init_wait", "chip.protocol contrast"]
    );
}
