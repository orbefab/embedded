//! A red and a blue 5 mm LED in series carry one current. Each is judged by its own datasheet:
//! red rates its light at 10 mA and blue at 20 mA, and red's reverse current allows a 100 uA
//! dim floor where blue's allows 500 uA.

use std::path::Path;

use orbe_session::{Command, Data, Files, Got, Live, Reading};
use serde_json::{Value, json};

/// A 5 V source through `ohms` into red, then blue, then back to the source.
fn series(ohms: &str) -> (tempfile::TempDir, Live) {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/light-divider");
    for file in ["resistor.jsonc", "source.jsonc"] {
        std::fs::copy(fixture.join(file), dir.path().join(file)).unwrap();
    }
    let parts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts");
    std::os::unix::fs::symlink(parts, dir.path().join("parts")).unwrap();
    let scene = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "r1": { "type": "./resistor.jsonc", "resistance": ohms },
            "red": { "type": "led-5mm" },
            "blue": { "type": "led-5mm-blue" },
        },
        "wires": [
            ["v1.pos", "r1.a"],
            ["r1.b", "red.anode"],
            ["red.cathode", "blue.anode"],
            ["blue.cathode", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    std::fs::write(dir.path().join("scene.jsonc"), scene.to_string()).unwrap();
    let mut live = Live::new("t", Files::disk());
    let opened = live.run(&Command::Open(dir.path().join("scene.jsonc")));
    assert!(opened.ok(), "{}", opened.text());
    (dir, live)
}

fn run(live: &mut Live, dir: &Path, line: &str) -> String {
    let command = Command::parse(line, dir).unwrap().unwrap();
    let reply = live.run(&command);
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.text()
}

/// The average current `get @<part>.brightness` reads, in amperes.
fn brightness(live: &mut Live, dir: &Path, part: &str) -> f64 {
    let command = Command::parse(&format!("get @{part}.brightness"), dir)
        .unwrap()
        .unwrap();
    let reply = live.run(&command);
    match reply.data {
        Data::Got(Got {
            value: Reading::Brightness { value, .. },
            ..
        }) => value,
        other => panic!("{other:?}"),
    }
}

fn full(scene: &Value, id: &str) -> Value {
    scene["parts"][id]["readings"]["brightness"]["full"].clone()
}

#[test]
fn each_colour_glows_fully_at_the_current_its_datasheet_rates_light_at() {
    let (_dir, live) = series("3.6k");
    let scene = live.scene().unwrap();
    assert_eq!(full(&scene, "led-5mm"), json!(0.01));
    assert_eq!(full(&scene, "led-5mm-blue"), json!(0.02));
}

#[test]
fn at_one_current_between_the_two_floors_red_is_dim_and_blue_is_off() {
    let (dir, mut live) = series("3.6k");
    run(&mut live, dir.path(), "step 30ms");
    let red = brightness(&mut live, dir.path(), "red");
    let blue = brightness(&mut live, dir.path(), "blue");
    assert!(
        (red - blue).abs() <= 1e-6 * red,
        "one current: {red} A and {blue} A"
    );
    assert!(
        red > 100e-6 && red <= 500e-6,
        "{red} A is between the two floors"
    );
    run(&mut live, dir.path(), "expect @red dim");
    run(&mut live, dir.path(), "expect @blue off");
}

#[test]
fn above_both_floors_and_below_two_milliamps_both_are_dim() {
    let (dir, mut live) = series("1k");
    run(&mut live, dir.path(), "step 30ms");
    let red = brightness(&mut live, dir.path(), "red");
    assert!(red > 500e-6 && red <= 2e-3, "{red} A");
    run(&mut live, dir.path(), "expect @red dim");
    run(&mut live, dir.path(), "expect @blue dim");
}

#[test]
fn reversed_supply_past_vr_warns_on_3mm_and_5mm_leds_and_under_vr_stays_quiet() {
    for (part, volts, expect_warn) in [
        ("led-3mm", "6", true),
        ("led-3mm", "4", false),
        ("led-3mm", "5", false),
        ("led-5mm", "6", true),
        ("led-5mm", "4", false),
        ("led-5mm", "5", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/light-divider");
        for file in ["resistor.jsonc", "source.jsonc"] {
            std::fs::copy(fixture.join(file), dir.path().join(file)).unwrap();
        }
        let parts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts");
        std::os::unix::fs::symlink(parts, dir.path().join("parts")).unwrap();
        let scene = json!({
            "format": 1,
            "parts": {
                "v1": { "type": "./source.jsonc", "voltage": volts },
                "r1": { "type": "./resistor.jsonc", "resistance": "1k" },
                "d1": { "type": part },
            },
            "wires": [
                ["v1.pos", "r1.a"],
                ["r1.b", "d1.cathode"],
                ["d1.anode", "v1.neg"],
            ],
            "ground": "v1.neg",
        });
        std::fs::write(dir.path().join("scene.jsonc"), scene.to_string()).unwrap();
        let mut live = Live::new("t", Files::disk());
        let opened = live.run(&Command::Open(dir.path().join("scene.jsonc")));
        assert!(opened.ok(), "{part} at {volts} V: {}", opened.text());

        run(&mut live, dir.path(), "step 10ms");
        run(&mut live, dir.path(), "expect @d1 off");
        let snap_cmd = Command::parse("snapshot", dir.path()).unwrap().unwrap();
        let snap = live.run(&snap_cmd);
        assert!(snap.ok(), "{part} at {volts} V: {}", snap.text());
        let diags = snap.json()["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .clone();
        if expect_warn {
            assert!(
                diags.iter().any(|d| d["code"] == "rating.voltage"
                    && d["message"]
                        .as_str()
                        .unwrap_or("")
                        .contains("#d1.v reaches -6 V, outside its -5 V floor")),
                "{part} at {volts} V: expected rating.voltage, got {diags:?}"
            );
        } else {
            run(&mut live, dir.path(), "expect no diagnostics");
            assert!(
                diags.is_empty(),
                "{part} at {volts} V: expected quiet, got {diags:?}"
            );
        }
    }
}
