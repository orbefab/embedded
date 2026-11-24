//! Pin voltage ratings on real catalog boards and modules:
//! - A 5 V Nano output wired straight into an ESP32-S3 DevKitC-1 GPIO warns `rating.voltage`
//!   while the S3 keeps running and reads the pin high.
//! - A 3.3 V drive into the same S3 GPIO reads high with no warning.
//! - Driving the S3 GPIO below -0.3 V warns `rating.voltage`.
//! - An SSD1306 OLED module on a 5 V I2C bus warns `rating.voltage` on `SCL` and `SDA` while
//!   still answering and lighting the glass.
//! - A 3.3 V-powered HD44780 I2C LCD or SK68XX LED driven by 5 V warns `rating.voltage`.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_session::{Command, Files, Live};
use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
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

fn catalog() -> Vec<(String, Vec<u8>)> {
    let parts = repo().join("parts");
    let mut out = Vec::new();
    walk(&parts, &parts, &mut out);
    out
}

/// `ldi r16, 0x04; out DDRD, r16; out PORTD, r16; rjmp .-2` — drives only PD2 (Nano D2) high.
const NANO_D2_HIGH: &str = ":0800000004E00AB90BB9FFCFBF\n:00000001FF\n";

fn run_cmd(live: &mut Live, project: &Path, line: &str) -> (Value, String) {
    let command = Command::parse(line, project).unwrap().unwrap();
    let reply = live.run(&command);
    assert!(reply.ok(), "{line}: {}", reply.text());
    (reply.json(), reply.text())
}

fn diagnostics_of(live: &mut Live, project: &Path) -> (Vec<Value>, String) {
    let (snap, text) = run_cmd(live, project, "snapshot");
    let diags = snap["data"]["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    (diags, text)
}

fn devkit_files(scene: &str, extra: Vec<(PathBuf, Vec<u8>)>) -> Files {
    let block = repo().join("blocks/devkitc-button-led");
    let read = |path: &str| fs::read(block.join(path)).unwrap();
    let mut files = vec![
        (PathBuf::from("proj/block.jsonc"), read("block.jsonc")),
        (PathBuf::from("proj/scene.jsonc"), scene.as_bytes().to_vec()),
        (
            PathBuf::from("proj/build/button.ino.bin"),
            read("build/button.ino.bin"),
        ),
        (
            PathBuf::from("proj/build/button.ino.partitions.bin"),
            read("build/button.ino.partitions.bin"),
        ),
    ];
    files.extend(extra);
    Files::memory(files).with_catalog(catalog())
}

#[test]
fn a_nano_driving_an_esp32s3_gpio_at_5v_warns_and_the_s3_still_reads_high() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "nano": { "type": "nano-board", "firmware": "build/d2_high.hex" },
        "s3": {
          "type": "esp32s3-devkitc-1",
          "app": "build/button.ino.bin",
          "partitions": "build/button.ino.partitions.bin"
        }
      },
      "ground": "nano.GND",
      "wires": [
        ["nano.GND", "s3.GND"],
        ["nano.D2", "s3.GPIO4"]
      ]
    }"#;
    let files = devkit_files(
        scene,
        vec![(
            PathBuf::from("proj/build/d2_high.hex"),
            NANO_D2_HIGH.as_bytes().to_vec(),
        )],
    );
    let project = Path::new("proj");
    let mut live = Live::new("s3-5v", files);
    run_cmd(&mut live, project, "open");
    run_cmd(&mut live, project, "step 5ms");
    let (diagnostics, snap_text) = diagnostics_of(&mut live, project);
    let voltage: Vec<&Value> = diagnostics
        .iter()
        .filter(|d| d["code"] == "rating.voltage")
        .collect();
    assert_eq!(voltage.len(), 1, "{snap_text}\n{diagnostics:#?}");
    let d = voltage[0];
    let refs: Vec<&str> = d["refs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        refs.iter().any(|r| r.contains("GPIO4")),
        "refs={refs:?} ({snap_text})"
    );
    let peak = d["data"]["peak"].as_f64().unwrap();
    let high = d["data"]["high"].as_f64().unwrap();
    let low = d["data"]["low"].as_f64().unwrap();
    assert!((4.7..=4.85).contains(&peak), "peak={peak} ({snap_text})");
    assert!((3.55..=3.65).contains(&high), "high={high} ({snap_text})");
    assert_eq!(low, -0.3);
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("outside its -0.3 V to 3.6"),
        "{snap_text}"
    );
    assert!(
        snap_text.contains("warning[rating.voltage]") && snap_text.contains("(s3.GPIO4)"),
        "{snap_text}"
    );

    let (get_json, get_text) = run_cmd(&mut live, project, "get @s3.GPIO4");
    assert_eq!(
        get_json["data"]["value"]["level"], "high",
        "{get_text} {get_json}"
    );
    let v = get_json["data"]["value"]["v"].as_f64().unwrap();
    assert!((4.7..=4.85).contains(&v), "{get_text}");
}

#[test]
fn a_3v3_drive_into_an_esp32s3_gpio_reads_high_without_warning() {
    let scene = r#"{
      "format": 1,
      "parts": {
        "s3": {
          "type": "esp32s3-devkitc-1",
          "app": "build/button.ino.bin",
          "partitions": "build/button.ino.partitions.bin"
        },
        "r1": { "type": "resistor-axial", "resistance": "1k" }
      },
      "ground": "s3.GND",
      "wires": [
        ["s3.3V3", "r1.a"],
        ["r1.b", "s3.GPIO4"]
      ]
    }"#;
    let files = devkit_files(scene, vec![]);
    let project = Path::new("proj");
    let mut live = Live::new("s3-3v3", files);
    run_cmd(&mut live, project, "open");
    run_cmd(&mut live, project, "step 5ms");
    let (diagnostics, snap_text) = diagnostics_of(&mut live, project);
    assert!(diagnostics.is_empty(), "{snap_text}\n{diagnostics:#?}");

    let (get_json, get_text) = run_cmd(&mut live, project, "get @s3.GPIO4");
    assert_eq!(
        get_json["data"]["value"]["level"], "high",
        "{get_text} {get_json}"
    );
    let v = get_json["data"]["value"]["v"].as_f64().unwrap();
    assert!((3.2..=3.4).contains(&v), "{get_text}");
}

#[test]
fn a_negative_drive_below_ground_on_an_esp32s3_gpio_warns() {
    let neg_part = r#"{
      "format": 1,
      "pins": { "pos": [1], "neg": [2] },
      "model": {
        "kind": "voltage_source",
        "voltage": { "value": -0.5, "unit": "V", "source": "user" },
        "pins": { "p": "pos", "n": "neg" }
      }
    }"#;
    let scene = r#"{
      "format": 1,
      "parts": {
        "s3": {
          "type": "esp32s3-devkitc-1",
          "app": "build/button.ino.bin",
          "partitions": "build/button.ino.partitions.bin"
        },
        "vneg": { "type": "./neg.jsonc" },
        "r1": { "type": "resistor-axial", "resistance": "1k" }
      },
      "ground": "s3.GND",
      "wires": [
        ["vneg.neg", "s3.GND"],
        ["vneg.pos", "r1.a"],
        ["r1.b", "s3.GPIO4"]
      ]
    }"#;
    let files = devkit_files(
        scene,
        vec![(
            PathBuf::from("proj/neg.jsonc"),
            neg_part.as_bytes().to_vec(),
        )],
    );
    let project = Path::new("proj");
    let mut live = Live::new("s3-neg", files);
    run_cmd(&mut live, project, "open");
    run_cmd(&mut live, project, "step 5ms");
    let (diagnostics, snap_text) = diagnostics_of(&mut live, project);
    let voltage: Vec<&Value> = diagnostics
        .iter()
        .filter(|d| d["code"] == "rating.voltage")
        .collect();
    assert_eq!(voltage.len(), 1, "{snap_text}\n{diagnostics:#?}");
    let d = voltage[0];
    let peak = d["data"]["peak"].as_f64().unwrap();
    assert!((-0.51..=-0.49).contains(&peak), "peak={peak}");
    assert_eq!(d["data"]["low"].as_f64(), Some(-0.3));
}

#[test]
fn an_oled_on_a_5v_i2c_bus_warns_and_keeps_drawing() {
    let block = repo().join("blocks/nano-oled");
    let read = |path: &str| fs::read(block.join(path)).unwrap();
    let scene = r#"{
      "format": 1,
      "parts": {
        "nano": { "type": "nano-board", "firmware": "build/hello.ino.hex" },
        "oled": { "type": "oled-ssd1306-128x64" },
        "rscl": { "type": "resistor-axial", "resistance": "4.7k" },
        "rsda": { "type": "resistor-axial", "resistance": "4.7k" }
      },
      "nets": { "GND": "nano.GND" },
      "wires": [
        ["nano.GND", "oled.GND"],
        ["nano.5V", "oled.VCC"],
        ["nano.A5", "oled.SCL"],
        ["nano.A4", "oled.SDA"],
        ["nano.5V", "rscl.a"],
        ["rscl.b", "oled.SCL"],
        ["nano.5V", "rsda.a"],
        ["rsda.b", "oled.SDA"]
      ]
    }"#;
    let files = Files::memory(vec![
        (PathBuf::from("proj/block.jsonc"), read("block.jsonc")),
        (PathBuf::from("proj/scene.jsonc"), scene.as_bytes().to_vec()),
        (
            PathBuf::from("proj/build/hello.ino.hex"),
            read("build/hello.ino.hex"),
        ),
    ])
    .with_catalog(catalog());
    let project = Path::new("proj");
    let mut live = Live::new("oled-5v-bus", files);
    run_cmd(&mut live, project, "open");
    run_cmd(&mut live, project, "step 140ms");
    let (diagnostics, snap_text) = diagnostics_of(&mut live, project);
    let mut pins: Vec<String> = diagnostics
        .iter()
        .filter(|d| d["code"] == "rating.voltage")
        .flat_map(|d| {
            d["refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
        })
        .collect();
    pins.sort();
    // External 4.7k pull-ups to 5 V both overdrive SCL/SDA past VDD + 0.3 V once the display
    // wakes and back-feed the module's 3.3 V LDO through R1/R2 while the display is asleep
    // (1 uA draw), pushing VDD over its 4.0 V limit and leaving C3 on RES# briefly high as VDD
    // drops back to 3.3 V on wake.
    assert!(
        pins.contains(&"#oled__display.pin.SCL".to_string())
            && pins.contains(&"#oled__display.pin.SDA".to_string()),
        "{snap_text}\n{diagnostics:#?}"
    );
    for pin in ["#oled__display.pin.SCL", "#oled__display.pin.SDA"] {
        let d = diagnostics
            .iter()
            .find(|d| {
                d["code"] == "rating.voltage"
                    && d["refs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r.as_str() == Some(pin))
            })
            .unwrap_or_else(|| panic!("missing {pin}: {diagnostics:#?}"));
        let peak = d["data"]["peak"].as_f64().unwrap();
        let high = d["data"]["high"].as_f64().unwrap();
        assert!((3.9..=4.2).contains(&peak), "{pin}: peak={peak} ({d:#?})");
        assert!((3.55..=3.65).contains(&high), "{pin}: high={high} ({d:#?})");
        assert_eq!(d["data"]["low"].as_f64(), Some(-0.3));
    }

    let (screen_json, screen_text) = run_cmd(&mut live, project, "get @oled.display.screen");
    assert_eq!(
        screen_json["data"]["value"]["on"], true,
        "{screen_text} {screen_json}"
    );
}

#[test]
fn lcd1602_and_sk68xx_on_3v3_driven_by_5v_warn_rating_voltage() {
    let v3v3 = r#"{
      "format": 1,
      "pins": { "pos": [1], "neg": [2] },
      "model": {
        "kind": "voltage_source",
        "voltage": { "value": 3.3, "unit": "V", "source": "user" },
        "pins": { "p": "pos", "n": "neg" }
      }
    }"#;
    let v5 = r#"{
      "format": 1,
      "pins": { "pos": [1], "neg": [2] },
      "model": {
        "kind": "voltage_source",
        "voltage": { "value": 5.0, "unit": "V", "source": "user" },
        "pins": { "p": "pos", "n": "neg" }
      }
    }"#;
    let scene = r#"{
      "format": 1,
      "parts": {
        "v3": { "type": "./v3.jsonc" },
        "v5": { "type": "./v5.jsonc" },
        "lcd": { "type": "lcd1602-i2c" },
        "px": { "type": "sk68xxmini-hs" },
        "rscl": { "type": "resistor-axial", "resistance": "1k" },
        "rsda": { "type": "resistor-axial", "resistance": "1k" },
        "rdin": { "type": "resistor-axial", "resistance": "1k" }
      },
      "ground": "v3.neg",
      "wires": [
        ["v3.neg", "v5.neg"],
        ["v3.neg", "lcd.GND"],
        ["v3.pos", "lcd.VCC"],
        ["v5.pos", "rscl.a"],
        ["rscl.b", "lcd.SCL"],
        ["v5.pos", "rsda.a"],
        ["rsda.b", "lcd.SDA"],
        ["v3.neg", "px.GND"],
        ["v3.pos", "px.VDD"],
        ["v5.pos", "rdin.a"],
        ["rdin.b", "px.DIN"]
      ]
    }"#;
    let block = repo().join("blocks/nano-lcd1602-i2c");
    let files = Files::memory(vec![
        (
            PathBuf::from("proj/block.jsonc"),
            fs::read(block.join("block.jsonc")).unwrap(),
        ),
        (PathBuf::from("proj/scene.jsonc"), scene.as_bytes().to_vec()),
        (PathBuf::from("proj/v3.jsonc"), v3v3.as_bytes().to_vec()),
        (PathBuf::from("proj/v5.jsonc"), v5.as_bytes().to_vec()),
    ])
    .with_catalog(catalog());
    let project = Path::new("proj");
    let mut live = Live::new("lcd-px-5v", files);
    run_cmd(&mut live, project, "open");
    run_cmd(&mut live, project, "step 2ms");
    let (diagnostics, snap_text) = diagnostics_of(&mut live, project);
    let mut pins: Vec<String> = diagnostics
        .iter()
        .filter(|d| d["code"] == "rating.voltage")
        .flat_map(|d| {
            d["refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
        })
        .collect();
    pins.sort();
    assert_eq!(
        pins,
        [
            "#lcd__display.pin.SCL",
            "#lcd__display.pin.SDA",
            "#px.pin.DIN"
        ],
        "{snap_text}\n{diagnostics:#?}"
    );
}
