//! Bench-honesty checks for the 5 mm RGB LEDs (`led-5mm-rgb-cc`, `led-5mm-rgb-ca`),
//! the 16-pixel WS2812B ring (`neopixel-ring-16`) and the KY-023 thumb joystick (`joystick-ky023`).

use std::path::{Path, PathBuf};

use orbe_session::viewer::{self, Feed};
use orbe_session::{Command, Files, Live};
use serde_json::{Value, json};

/// Bare-metal ATmega328P @ 16 MHz image (no runtime): shifts 48 bytes of 0xFF (16 white WS2812B
/// pixels) out on PB0 (Nano D8) with T1H = 812.5 ns / T1L = 500 ns, then holds low.
const WS2812_WHITE_16_HEX: &str = "\
:10000000F894209A289880E398E0289A000000004D\n\
:1000100000000000000000000000000000000000E0\n\
:10002000000028989A9519F48A9571F7FFCF00007F\n\
:02003000ECCF13\n\
:00000001FF\n";

const DC_SOURCE_PART: &str = r#"{
  "format": 4,
  "part": "test-dc-source",
  "version": "0.1.0",
  "title": "Test DC source",
  "license": "MIT OR Apache-2.0",
  "pins": { "pos": [1], "neg": [2] },
  "params": { "voltage": { "unit": "V", "required": true, "sets": "/voltage" } },
  "model": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }
}"#;

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

fn open_scene(scene: Value, extra: Vec<(String, Vec<u8>)>) -> Live {
    let block = json!({
        "format": 1,
        "block": "test",
        "version": "0.0.0",
        "title": "test",
        "board": "nano-board",
        "license": "MIT OR Apache-2.0"
    });
    let mut entries = vec![
        ("block.jsonc".to_string(), block.to_string().into_bytes()),
        (
            "source.jsonc".to_string(),
            DC_SOURCE_PART.as_bytes().to_vec(),
        ),
        ("scene.jsonc".to_string(), scene.to_string().into_bytes()),
    ];
    entries.extend(extra);
    let files = Files::memory(entries).with_catalog(catalog());
    let mut live = Live::new("t", files);
    let reply = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(reply.ok(), "{}", reply.text());
    live
}

fn run(live: &mut Live, line: &str) -> orbe_session::Reply {
    let reply = live.run(&Command::parse(line, Path::new(".")).unwrap().unwrap());
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply
}

fn number(live: &mut Live, target: &str) -> f64 {
    let reply = run(live, &format!("get {target}"));
    let val = &reply.json()["data"]["value"];
    val.as_f64()
        .or_else(|| val["v"].as_f64())
        .or_else(|| val["i"].as_f64())
        .unwrap_or_else(|| panic!("{target}: {}", reply.text()))
}

#[test]
fn rgb_led_cc_and_ca_forward_curves_and_wrong_polarity_match_bench() {
    // 1. Common-cathode wired as CC (COM -> GND, R/G/B through resistors -> 5 V): all three light
    //    with Kingbright WP154A4SUREQBFZGW forward drops (Red ~2.0 V, Green/Blue ~3.2 V).
    let cc_as_cc = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "rr": { "type": "resistor-axial", "resistance": "150" },
            "rg": { "type": "resistor-axial", "resistance": "100" },
            "rb": { "type": "resistor-axial", "resistance": "100" },
            "rgb": { "type": "led-5mm-rgb-cc" },
        },
        "wires": [
            ["v1.pos", "rr.a"], ["rr.b", "rgb.R"],
            ["v1.pos", "rg.a"], ["rg.b", "rgb.G"],
            ["v1.pos", "rb.a"], ["rb.b", "rgb.B"],
            ["rgb.COM", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(cc_as_cc, vec![]);
    run(&mut live, "step 30ms");
    for die in ["@rgb.red", "@rgb.green", "@rgb.blue"] {
        run(&mut live, &format!("expect {die} lit"));
    }
    let vr = number(&mut live, "@rgb.R");
    let vg = number(&mut live, "@rgb.G");
    let vb = number(&mut live, "@rgb.B");
    assert!((1.85..2.15).contains(&vr), "CC Red VF = {vr} V");
    assert!((3.05..3.45).contains(&vg), "CC Green VF = {vg} V");
    assert!((3.05..3.45).contains(&vb), "CC Blue VF = {vb} V");

    // 2. Common-anode wired as CA (COM -> 5 V, R/G/B through resistors -> GND): all three light.
    let ca_as_ca = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "rr": { "type": "resistor-axial", "resistance": "150" },
            "rg": { "type": "resistor-axial", "resistance": "100" },
            "rb": { "type": "resistor-axial", "resistance": "100" },
            "rgb": { "type": "led-5mm-rgb-ca" },
        },
        "wires": [
            ["v1.pos", "rgb.COM"],
            ["rgb.R", "rr.a"], ["rr.b", "v1.neg"],
            ["rgb.G", "rg.a"], ["rg.b", "v1.neg"],
            ["rgb.B", "rb.a"], ["rb.b", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(ca_as_ca, vec![]);
    run(&mut live, "step 30ms");
    for die in ["@rgb.red", "@rgb.green", "@rgb.blue"] {
        run(&mut live, &format!("expect {die} lit"));
    }
    let vf_r = 5.0 - number(&mut live, "@rgb.R");
    let vf_g = 5.0 - number(&mut live, "@rgb.G");
    let vf_b = 5.0 - number(&mut live, "@rgb.B");
    assert!((1.85..2.15).contains(&vf_r), "CA Red VF = {vf_r} V");
    assert!((3.05..3.45).contains(&vf_g), "CA Green VF = {vf_g} V");
    assert!((3.05..3.45).contains(&vf_b), "CA Blue VF = {vf_b} V");

    // 3. Common-anode wired as common-cathode: all three dies are reverse-biased and stay off.
    let ca_as_cc = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "rr": { "type": "resistor-axial", "resistance": "150" },
            "rg": { "type": "resistor-axial", "resistance": "100" },
            "rb": { "type": "resistor-axial", "resistance": "100" },
            "rgb": { "type": "led-5mm-rgb-ca" },
        },
        "wires": [
            ["v1.pos", "rr.a"], ["rr.b", "rgb.R"],
            ["v1.pos", "rg.a"], ["rg.b", "rgb.G"],
            ["v1.pos", "rb.a"], ["rb.b", "rgb.B"],
            ["rgb.COM", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(ca_as_cc, vec![]);
    run(&mut live, "step 30ms");
    for die in ["@rgb.red", "@rgb.green", "@rgb.blue"] {
        run(&mut live, &format!("expect {die} off"));
    }

    // 4. Common-cathode wired as common-anode: all three dies are reverse-biased and stay off.
    let cc_as_ca = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "rr": { "type": "resistor-axial", "resistance": "150" },
            "rg": { "type": "resistor-axial", "resistance": "100" },
            "rb": { "type": "resistor-axial", "resistance": "100" },
            "rgb": { "type": "led-5mm-rgb-cc" },
        },
        "wires": [
            ["v1.pos", "rgb.COM"],
            ["rgb.R", "rr.a"], ["rr.b", "v1.neg"],
            ["rgb.G", "rg.a"], ["rg.b", "v1.neg"],
            ["rgb.B", "rb.a"], ["rb.b", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(cc_as_ca, vec![]);
    run(&mut live, "step 30ms");
    for die in ["@rgb.red", "@rgb.green", "@rgb.blue"] {
        run(&mut live, &format!("expect {die} off"));
    }

    // 5. Over-current on red die (20 ohm from 5 V -> ~120 mA > 30 mA rating): warns on red.
    let overcurrent = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "rr": { "type": "resistor-axial", "resistance": "20" },
            "rgb": { "type": "led-5mm-rgb-cc" },
        },
        "wires": [
            ["v1.pos", "rr.a"], ["rr.b", "rgb.R"],
            ["rgb.COM", "v1.neg"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(overcurrent, vec![]);
    run(&mut live, "step 20ms");
    let diags = run(&mut live, "snapshot").json()["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        diags.iter().any(|d| d["code"] == "rating.current"),
        "expected rating.current diagnostic, got {diags:?}"
    );
}

#[test]
fn neopixel_ring_16_full_white_current_matches_ws2812b_sheet_and_unpowered_does_nothing() {
    // 1. Powered from an external 5 V supply, all 16 chained pixels latch rgb(255,255,255)
    //    and draw 16 * (0.6 mA idle + 3 * 12 mA) = 585.6 mA (WS2812B-V5 p3).
    let ext_scene = json!({
        "format": 1,
        "parts": {
            "nano": { "type": "nano-board", "firmware": "firmware/white16.hex" },
            "vext": { "type": "./source.jsonc", "voltage": "5" },
            "r1": { "type": "resistor-axial", "resistance": "470" },
            "ring": { "type": "neopixel-ring-16" },
        },
        "wires": [
            ["nano.D8", "r1.a"], ["r1.b", "ring.DIN"],
            ["vext.pos", "ring.5V"],
            ["vext.neg", "ring.GND"],
            ["vext.neg", "nano.GND"],
        ],
    });
    let mut live = open_scene(
        ext_scene,
        vec![(
            "firmware/white16.hex".to_string(),
            WS2812_WHITE_16_HEX.as_bytes().to_vec(),
        )],
    );
    run(&mut live, "step 20ms");
    for n in 1..=16 {
        run(&mut live, &format!("expect @ring.px{n} rgb(255,255,255)"));
    }
    let i_white = number(&mut live, "@vext.i").abs();
    assert!(
        (0.580..0.592).contains(&i_white),
        "16 WS2812B-V5 pixels at full white should draw ~0.5856 A, got {i_white} A"
    );

    // 2. Same 16 full-white pixels powered from Nano 5V exceed the 500 mA USB supply and warn.
    let usb_scene = json!({
        "format": 1,
        "parts": {
            "nano": { "type": "nano-board", "firmware": "firmware/white16.hex" },
            "r1": { "type": "resistor-axial", "resistance": "470" },
            "ring": { "type": "neopixel-ring-16" },
        },
        "wires": [
            ["nano.D8", "r1.a"], ["r1.b", "ring.DIN"],
            ["nano.5V", "ring.5V"],
            ["nano.GND", "ring.GND"],
        ],
    });
    let mut live = open_scene(
        usb_scene,
        vec![(
            "firmware/white16.hex".to_string(),
            WS2812_WHITE_16_HEX.as_bytes().to_vec(),
        )],
    );
    run(&mut live, "step 120ms");
    let i_usb = number(&mut live, "@nano.usb.i").abs();
    assert!(
        i_usb > 0.55,
        "USB draw with 16 full-white pixels: {i_usb} A"
    );
    let diags = run(&mut live, "snapshot").json()["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        diags.iter().any(|d| d["code"] == "rating.current"
            && d["message"]
                .as_str()
                .unwrap_or("")
                .starts_with("#nano__usb__host.i")),
        "expected USB supply rating.current warning at {i_usb} A, got {diags:?}"
    );

    // 3. Unpowered ring (5V = 0 V): does not latch pixels and draws 0 A.
    let unpowered = json!({
        "format": 1,
        "parts": {
            "nano": { "type": "nano-board", "firmware": "firmware/white16.hex" },
            "vext": { "type": "./source.jsonc", "voltage": "0" },
            "r1": { "type": "resistor-axial", "resistance": "470" },
            "ring": { "type": "neopixel-ring-16" },
        },
        "wires": [
            ["nano.D8", "r1.a"], ["r1.b", "ring.DIN"],
            ["vext.pos", "ring.5V"],
            ["vext.neg", "ring.GND"],
            ["vext.neg", "nano.GND"],
        ],
    });
    let mut live = open_scene(
        unpowered,
        vec![(
            "firmware/white16.hex".to_string(),
            WS2812_WHITE_16_HEX.as_bytes().to_vec(),
        )],
    );
    run(&mut live, "step 20ms");
    run(&mut live, "expect @ring.px1 rgb(0,0,0)");
    run(&mut live, "expect @ring.px16 rgb(0,0,0)");
    let i_off = number(&mut live, "@vext.i").abs();
    assert!(i_off < 1e-4, "unpowered ring drew {i_off} A");
}

#[test]
fn joystick_ky023_axes_button_viewer_controls_and_unpowered_behavior() {
    let scene = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "5" },
            "rpu": { "type": "resistor-axial", "resistance": "10k" },
            "joy": { "type": "joystick-ky023" },
        },
        "wires": [
            ["v1.pos", "joy.VCC"],
            ["v1.neg", "joy.GND"],
            ["v1.pos", "rpu.a"],
            ["rpu.b", "joy.SW"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(scene, vec![]);
    run(&mut live, "step 10ms");

    // Viewer scene exposes both live axis sliders and the push switch.
    let scene_msg = live.scene().unwrap();
    let params = scene_msg["params"].as_array().unwrap();
    let refs: Vec<&str> = params.iter().map(|p| p["ref"].as_str().unwrap()).collect();
    assert_eq!(refs, ["@joy.x.position", "@joy.y.position"]);
    assert_eq!(
        scene_msg["channels"]["switches"],
        json!(["@joy.sw"]),
        "thumbstick switch must be pressable on the page"
    );

    // Centered: VRX ~ 2.5 V, VRY ~ 2.5 V, SW high (~5 V).
    assert!((number(&mut live, "@joy.VRX") - 2.5).abs() < 1e-3);
    assert!((number(&mut live, "@joy.VRY") - 2.5).abs() < 1e-3);
    assert!((number(&mut live, "@joy.SW") - 5.0).abs() < 1e-3);

    // Move axes and press button via viewer protocol commands (as the page does).
    let (r1, _) = viewer::answer(
        &mut live,
        r#"{"id": 1, "cmd": "set", "ref": "@joy.x.position", "value": 0.9}"#,
    );
    assert!(r1.ok(), "{}", r1.text());
    let (r2, _) = viewer::answer(
        &mut live,
        r#"{"id": 2, "cmd": "set", "ref": "@joy.y.position", "value": 0.1}"#,
    );
    assert!(r2.ok(), "{}", r2.text());
    let (r3, _) = viewer::answer(&mut live, r#"{"id": 3, "cmd": "press", "ref": "@joy.sw"}"#);
    assert!(r3.ok(), "{}", r3.text());
    run(&mut live, "step 10ms");

    assert!((number(&mut live, "@joy.VRX") - 4.5).abs() < 1e-3);
    assert!((number(&mut live, "@joy.VRY") - 0.5).abs() < 1e-3);
    assert!(number(&mut live, "@joy.SW") < 0.05);
    assert_eq!(
        Feed::default().now(&live, None)[0]["params"],
        json!([0.9, 0.1])
    );

    let (r4, _) = viewer::answer(
        &mut live,
        r#"{"id": 4, "cmd": "release", "ref": "@joy.sw"}"#,
    );
    assert!(r4.ok(), "{}", r4.text());
    run(&mut live, "step 10ms");
    assert!((number(&mut live, "@joy.SW") - 5.0).abs() < 1e-3);

    // Unpowered joystick (VCC = 0 V): outputs 0 V regardless of axis position.
    let unpowered = json!({
        "format": 1,
        "parts": {
            "v1": { "type": "./source.jsonc", "voltage": "0" },
            "joy": { "type": "joystick-ky023", "x": 0.9, "y": 0.2 },
        },
        "wires": [
            ["v1.pos", "joy.VCC"],
            ["v1.neg", "joy.GND"],
        ],
        "ground": "v1.neg",
    });
    let mut live = open_scene(unpowered, vec![]);
    run(&mut live, "step 10ms");
    assert!(number(&mut live, "@joy.VRX").abs() < 1e-6);
    assert!(number(&mut live, "@joy.VRY").abs() < 1e-6);
}
