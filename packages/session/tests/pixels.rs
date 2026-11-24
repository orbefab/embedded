//! Four SK68XXMINI-HS LEDs in a row on a Nano's D8, each `DOUT` on the next `DIN`. The sketch is
//! the engine's `ws2812.hex`, which sends 13 LEDs once at power-on: LED `i` is green `i`, red
//! `i + 16` and blue `i + 32`. Each LED reads the colour it latched, and `expect` checks it.

use std::path::{Path, PathBuf};

use orbe_session::{Code, Command, Data, Files, Live, Reading};
use serde_json::json;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const SCENE: &str = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "firmware/ws2812.hex" },
    "rpd": { "type": "resistor-axial", "resistance": "10k" },
    "led1": { "type": "sk68xxmini-hs" },
    "led2": { "type": "sk68xxmini-hs" },
    "led3": { "type": "sk68xxmini-hs" },
    "led4": { "type": "sk68xxmini-hs" },
  },
  "wires": [
    ["nano.5V", "led1.VDD"], ["nano.GND", "led1.GND"],
    ["nano.5V", "led2.VDD"], ["nano.GND", "led2.GND"],
    ["nano.5V", "led3.VDD"], ["nano.GND", "led3.GND"],
    ["nano.5V", "led4.VDD"], ["nano.GND", "led4.GND"],
    ["nano.D8", "led1.DIN"], ["rpd.a", "nano.D8"], ["rpd.b", "nano.GND"],
    ["led1.DOUT", "led2.DIN"],
    ["led2.DOUT", "led3.DIN"],
    ["led3.DOUT", "led4.DIN"],
  ],
}"#;

/// Each catalog part's files by `<id>/<path>`.
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

fn opened() -> Live {
    let hex =
        std::fs::read(repo().join("packages/engine/tests/firmware/sketch/ws2812.hex")).unwrap();
    let block = std::fs::read(repo().join("blocks/nano-button-led/block.jsonc")).unwrap();
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        ("firmware/ws2812.hex".to_string(), hex),
        ("scene.jsonc".to_string(), SCENE.as_bytes().to_vec()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let open = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(open.ok(), "{}", open.text());
    live
}

fn run(live: &mut Live, line: &str) -> orbe_session::Reply {
    live.run(&Command::parse(line, Path::new(".")).unwrap().unwrap())
}

/// LED `k`'s colour as red, green, blue.
fn colour(k: u8) -> [u8; 3] {
    [k + 16, k, k + 32]
}

#[test]
fn each_led_in_the_chain_reads_its_own_latched_colour() {
    let mut live = opened();
    let before = run(&mut live, "get @led1.color");
    assert_eq!(before.text(), "led1.color rgb(0,0,0) not latched yet");
    assert!(run(&mut live, "step 5ms").ok());
    for k in 0..4u8 {
        let reply = run(&mut live, &format!("get @led{}", k + 1));
        let Data::Got(got) = &reply.data else {
            panic!("{reply:?}")
        };
        let Reading::Part {
            color: Some(colors),
            ..
        } = &got.value
        else {
            panic!("{got:?}")
        };
        assert_eq!(colors.leds, [colour(k)], "led{}", k + 1);
        assert!(colors.latched_ps.is_some());
    }
    let json = run(&mut live, "get @led2.color").json();
    let latched = json["data"]["value"]["latched_ps"].as_u64().unwrap();
    assert_eq!(
        json["data"]["value"],
        json!({ "r": 17, "g": 1, "b": 33, "latched_ps": latched })
    );
    assert_eq!(json["data"]["kind"], "color");
}

#[test]
fn expect_checks_an_led_s_colour_by_its_part_or_its_reading() {
    let mut live = opened();
    assert!(run(&mut live, "step 5ms").ok());
    for line in [
        "expect @led1 rgb(16,0,32)",
        "expect @led2.color rgb(17,1,33)",
        "expect @led3 rgb(18, 2, 34)",
        "expect @led4 rgb(19,3,35)",
        "expect no diagnostics",
    ] {
        let reply = run(&mut live, line);
        assert!(reply.ok(), "{line}: {}", reply.text());
    }
    let failed = run(&mut live, "expect @led4 rgb(255,0,0)");
    assert_eq!(failed.error.as_ref().unwrap().code, Code::ExpectFailed);
    let Data::Checked(check) = &failed.data else {
        panic!("{failed:?}")
    };
    assert!(
        check
            .actual
            .starts_with("led4.color rgb(19,3,35) latched at "),
        "{}",
        check.actual
    );
}

#[test]
fn the_snapshot_shows_each_led_s_colour() {
    let mut live = opened();
    assert!(run(&mut live, "step 5ms").ok());
    let snapshot = run(&mut live, "snapshot").text();
    for k in 0..4u8 {
        let [r, g, b] = colour(k);
        let rgb = format!("color=rgb({r},{g},{b}) latched at ");
        assert!(
            snapshot
                .lines()
                .any(|line| line.starts_with(&format!("led{} ", k + 1)) && line.contains(&rgb)),
            "led{} {rgb} in\n{snapshot}",
            k + 1
        );
    }
}

#[test]
fn a_colour_is_no_state_and_a_part_without_one_says_so() {
    let mut live = opened();
    let reply = run(&mut live, "expect @rpd rgb(255,0,0)");
    let error = reply.error.unwrap();
    assert_eq!(error.code, Code::NoReading);
    assert_eq!(error.message, "@rpd has no colour");
    let reply = run(&mut live, "expect @led1.color > 2mA");
    assert_eq!(reply.error.unwrap().code, Code::NoReading);
}

fn stick_scene(supply_wire: Option<&str>) -> String {
    let power = supply_wire
        .map(|net| format!(", [\"{net}\", \"stick.5V\"]"))
        .unwrap_or_default();
    format!(
        r#"{{
  "format": 1,
  "parts": {{
    "nano": {{ "type": "nano-board", "firmware": "firmware/rainbow.hex" }},
    "r1": {{ "type": "resistor-axial", "resistance": "470" }},
    "stick": {{ "type": "neopixel-stick-8" }},
  }},
  "wires": [
    ["nano.D6", "r1.a"], ["r1.b", "stick.DIN"],
    ["nano.GND", "stick.GND"]{power}
  ],
}}"#
    )
}

fn open_stick(supply_wire: Option<&str>) -> Live {
    let hex =
        std::fs::read(repo().join("blocks/nano-neopixel-stick/build/rainbow.ino.hex")).unwrap();
    let block = std::fs::read(repo().join("blocks/nano-neopixel-stick/block.jsonc")).unwrap();
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        ("firmware/rainbow.hex".to_string(), hex),
        (
            "scene.jsonc".to_string(),
            stick_scene(supply_wire).into_bytes(),
        ),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let open = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(open.ok(), "{}", open.text());
    live
}

#[test]
fn an_unpowered_or_floating_stick_latches_nothing_and_stays_off() {
    for (label, supply) in [("0 V", Some("nano.GND")), ("floating", None)] {
        let mut live = open_stick(supply);
        let stepped = run(&mut live, "step 20ms");
        assert!(stepped.ok(), "{label}: {}", stepped.text());
        for n in 1..=8 {
            let reply = run(&mut live, &format!("get @stick.px{n}.color"));
            assert_eq!(
                reply.text(),
                format!("stick.px{n}.color rgb(0,0,0) not latched yet"),
                "{label} px{n}"
            );
        }
    }

    // A supply well below what the pixel needs (2 V) must also latch nothing on a 5 V Nano.
    let low_supply_scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "firmware/rainbow.hex" },
    "r_top": { "type": "resistor-axial", "resistance": "1.5k" },
    "r_bot": { "type": "resistor-axial", "resistance": "1k" },
    "r1": { "type": "resistor-axial", "resistance": "470" },
    "stick": { "type": "neopixel-stick-8" },
  },
  "wires": [
    ["nano.5V", "r_top.a"], ["r_top.b", "r_bot.a"], ["r_bot.b", "nano.GND"],
    ["r_top.b", "stick.5V"], ["nano.GND", "stick.GND"],
    ["nano.D6", "r1.a"], ["r1.b", "stick.DIN"],
  ],
}"#;
    let hex =
        std::fs::read(repo().join("blocks/nano-neopixel-stick/build/rainbow.ino.hex")).unwrap();
    let block = std::fs::read(repo().join("blocks/nano-neopixel-stick/block.jsonc")).unwrap();
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        ("firmware/rainbow.hex".to_string(), hex),
        (
            "scene.jsonc".to_string(),
            low_supply_scene.as_bytes().to_vec(),
        ),
    ])
    .with_catalog(catalog());
    let mut low = Live::new("t", files);
    assert!(low.run(&Command::Open(PathBuf::from("scene.jsonc"))).ok());
    assert!(run(&mut low, "step 20ms").ok());
    for n in 1..=8 {
        let reply = run(&mut low, &format!("get @stick.px{n}.color"));
        assert_eq!(
            reply.text(),
            format!("stick.px{n}.color rgb(0,0,0) not latched yet"),
            "low supply px{n}"
        );
    }

    let mut powered = open_stick(Some("nano.5V"));
    assert!(run(&mut powered, "step 20ms").ok());
    let reply = run(&mut powered, "expect @stick.px1 rgb(255,0,0)");
    assert!(reply.ok(), "powered px1: {}", reply.text());
    let reply = run(&mut powered, "expect @stick.px8 rgb(255,0,120)");
    assert!(reply.ok(), "powered px8: {}", reply.text());
}

#[test]
fn power_loss_clears_latch_time_until_a_new_frame_latches() {
    let scene = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "firmware/rainbow.hex" },
    "sw": { "type": "tactile-switch-6mm" },
    "r1": { "type": "resistor-axial", "resistance": "470" },
    "stick": { "type": "neopixel-stick-8" },
  },
  "wires": [
    ["nano.D6", "r1.a"], ["r1.b", "stick.DIN"],
    ["nano.5V", "sw.a"], ["sw.b", "stick.5V"],
    ["nano.GND", "stick.GND"],
  ],
}"#;
    let hex =
        std::fs::read(repo().join("blocks/nano-neopixel-stick/build/rainbow.ino.hex")).unwrap();
    let block = std::fs::read(repo().join("blocks/nano-neopixel-stick/block.jsonc")).unwrap();
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        ("firmware/rainbow.hex".to_string(), hex),
        ("scene.jsonc".to_string(), scene.as_bytes().to_vec()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let open = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(open.ok(), "{}", open.text());

    assert!(run(&mut live, "press @sw").ok());
    assert!(run(&mut live, "step 20ms").ok());
    let powered = run(&mut live, "get @stick.px1.color");
    assert!(
        powered
            .text()
            .starts_with("stick.px1.color rgb(255,0,0) latched at "),
        "{}",
        powered.text()
    );
    let first_latched = powered.json()["data"]["value"]["latched_ps"]
        .as_u64()
        .unwrap();

    // Open the supply switch: the rail drops and the pixel must read as never latched since then.
    assert!(run(&mut live, "release @sw").ok());
    assert!(run(&mut live, "step 5ms").ok());
    let dropped = run(&mut live, "get @stick.px1.color");
    assert_eq!(dropped.text(), "stick.px1.color rgb(0,0,0) not latched yet");
    assert_eq!(dropped.json()["data"]["value"]["latched_ps"], json!(null));

    // Close the switch again without a new frame: still not latched yet.
    assert!(run(&mut live, "press @sw").ok());
    assert!(run(&mut live, "step 5ms").ok());
    let repowered = run(&mut live, "get @stick.px1.color");
    assert_eq!(
        repowered.text(),
        "stick.px1.color rgb(0,0,0) not latched yet"
    );
    assert_eq!(repowered.json()["data"]["value"]["latched_ps"], json!(null));

    // Reset the Nano so the sketch sends a fresh frame after power returned (past the 65 ms SUT).
    assert!(run(&mut live, "press @nano.sw_reset").ok());
    assert!(run(&mut live, "step 1ms").ok());
    assert!(run(&mut live, "release @nano.sw_reset").ok());
    assert!(run(&mut live, "step 80ms").ok());
    let recovered = run(&mut live, "get @stick.px1.color");
    assert!(
        recovered
            .text()
            .starts_with("stick.px1.color rgb(255,0,0) latched at "),
        "{}",
        recovered.text()
    );
    let second_latched = recovered.json()["data"]["value"]["latched_ps"]
        .as_u64()
        .unwrap();
    assert!(
        second_latched > first_latched,
        "second latch {second_latched} should be after first {first_latched}"
    );
}
