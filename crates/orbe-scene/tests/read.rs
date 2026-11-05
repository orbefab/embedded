//! The fixture scenes read into the netlists investigation 4 measured.

mod common;

use std::fs;

use common::{fixture, lock_line, locked_ids, project};
use serde_json::Value;

#[test]
fn parts_lock_matches_the_catalog() {
    let project = project("nano-button");
    let lines: String = locked_ids(project.path())
        .iter()
        .map(|id| lock_line(project.path(), id) + "\n")
        .collect();
    let committed = fixture("nano-button").join("parts.lock");
    if std::env::var_os("ORBE_BLESS").is_some() {
        fs::write(&committed, &lines).unwrap();
    }
    assert_eq!(
        fs::read_to_string(committed).unwrap(),
        lines,
        "parts.lock is stale; rerun with ORBE_BLESS=1"
    );
}

fn read(scene: &str) -> (orbe_scene::Netlist, Value) {
    let project = project("nano-button");
    let netlist = orbe_scene::read(&project.path().join(scene))
        .unwrap_or_else(|d| panic!("{scene} did not read: {d:#?}"));
    let json: Value = serde_json::from_str(&netlist.json).unwrap();
    (netlist, json)
}

#[test]
fn first_target_has_18_parts_and_31_nets() {
    let (netlist, json) = read("scene.jsonc");
    assert_eq!(netlist.parts, 18);
    assert_eq!(netlist.ground, "nano__GND");
    let mut nets = netlist.nets.clone();
    nets.sort();
    let mut expected: Vec<String> = [
        "nano__GND",
        "led1__anode",
        "nano__5V",
        "nano__AREF",
        "nano__RESET",
        "nano__c_reset__a",
        "nano__led_l__anode",
        "nano__led_pwr__anode",
        "nano__mcu__PB6",
        "nano__mcu__PB7",
        "nano__r_txd__a",
    ]
    .into_iter()
    .map(str::to_string)
    .chain((0..=5).map(|i| format!("nano__A{i}")))
    .chain((0..=13).map(|i| format!("nano__D{i}")))
    .collect();
    expected.sort();
    assert_eq!(nets, expected);
    assert_eq!(json["nets"][0], "nano__GND", "ground is the first net");
    let names: Vec<&str> = json["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    for name in [
        "btn1",
        "r1",
        "r2",
        "led1",
        "nano__mcu",
        "nano__usb__host",
        "nano__usb_serial__host_tx",
        "nano__usb_serial__host_dtr",
        "nano__resonator__load_in",
    ] {
        assert!(names.contains(&name), "no engine part {name}: {names:?}");
    }
    let mcu = json["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "nano__mcu")
        .unwrap();
    assert_eq!(mcu["pins"]["PB5"], "nano__D13");
    assert_eq!(mcu["pins"]["GND_8"], "nano__GND");
    assert_eq!(
        mcu["firmware"].as_str().unwrap(),
        fs::read_to_string(common::firmware()).unwrap()
    );
}

#[test]
fn scene_params_are_user_values_and_defaults_fill_the_rest() {
    let (_, json) = read("scene.jsonc");
    let part = |name: &str| {
        json["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(
        part("r1")["resistance"],
        serde_json::json!({ "value": 220.0, "unit": "ohm", "source": "user" })
    );
    assert_eq!(part("r2")["resistance"]["value"], 10_000.0);
    assert_eq!(part("led1")["saturation_current"]["value"], 6.711e-10);
    assert_eq!(
        part("btn1")["bounce"],
        Value::Null,
        "bounce stays off without `bounce`"
    );
    assert_eq!(part("nano__c_reset")["initial"], Value::Null);
    assert_eq!(
        part("nano__r_l")["resistance"]["source"]["datasheet"]["at"],
        "RN2A, 1K"
    );
}

#[test]
fn two_buttons_ground_is_btn1_gnd() {
    let (netlist, json) = read("two-buttons.jsonc");
    assert_eq!(netlist.parts, 18);
    assert_eq!(netlist.nets.len(), 30);
    assert_eq!(netlist.ground, "btn1__gnd");
    for net in ["btn1__out", "btn1__vcc", "btn2__out"] {
        assert!(netlist.nets.contains(&net.to_string()), "no net {net}");
    }
    let part = |name: &str| {
        json["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(
        part("btn1__r")["resistance"]["value"],
        10_000.0,
        "the part's default passes through"
    );
    assert_eq!(
        part("btn2__r")["resistance"]["value"],
        4_700.0,
        "the scene's value passes through"
    );
}

#[test]
fn two_buttons_with_nets_names_ground_gnd() {
    let (netlist, _) = read("two-buttons-nets.jsonc");
    assert_eq!(netlist.nets.len(), 30);
    assert_eq!(netlist.ground, "GND");
    for net in ["VCC", "btn1__out", "btn2__out"] {
        assert!(netlist.nets.contains(&net.to_string()), "no net {net}");
    }
}

#[test]
fn an_edited_nano_copy_is_modified_and_still_reads() {
    let project = project("nano-button");
    let file = project.path().join("parts/nano-board/part.jsonc");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        text.replacen(
            "\"r_l\": { \"at\": [8.5, 3.3, 4.14] }",
            "\"r_l\": { \"at\": [9.0, 3.3, 4.14] }",
            1,
        ),
    )
    .unwrap();
    let netlist = orbe_scene::read(&project.path().join("scene.jsonc")).unwrap();
    assert_eq!(netlist.modified, vec!["nano-board".to_string()]);
}

#[test]
fn a_sealed_edit_with_its_lock_line_rewritten_is_still_modified() {
    let project = project("nano-button");
    let file = project.path().join("parts/led-5mm/part.jsonc");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        text.replacen("\"value\": 2.115", "\"value\": 1.5", 1),
    )
    .unwrap();
    let lock = project.path().join("parts.lock");
    let lines: String = fs::read_to_string(&lock)
        .unwrap()
        .lines()
        .map(|l| if l.starts_with("led-5mm ") { lock_line(project.path(), "led-5mm") } else { l.to_string() } + "\n")
        .collect();
    fs::write(&lock, lines).unwrap();
    let found = orbe_scene::read(&project.path().join("scene.jsonc"))
        .err()
        .unwrap();
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::PartModified);
    assert_eq!(found[0].file, "parts/led-5mm/part.jsonc");
    assert!(
        found[0]
            .message
            .contains("`params.color.choices.red.sets./series_resistance.value`"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_bounce_window_fills_the_switch_with_its_default_count() {
    let project = project("nano-button");
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(
        &scene,
        text.replacen(
            "{ \"type\": \"tactile-switch-6mm\" }",
            "{ \"type\": \"tactile-switch-6mm\", \"bounce\": \"1ms\" }",
            1,
        ),
    )
    .unwrap();
    let netlist = orbe_scene::read(&scene).unwrap();
    let json: Value = serde_json::from_str(&netlist.json).unwrap();
    let btn1 = json["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "btn1")
        .unwrap();
    assert_eq!(
        btn1["bounce"]["window"],
        serde_json::json!({ "value": 1_000_000_000u64, "unit": "ps", "source": "user" })
    );
    assert_eq!(btn1["bounce"]["max_bounces"]["value"], 2);
}
