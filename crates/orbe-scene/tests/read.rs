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
        serde_json::json!({ "value": 220, "unit": "ohm", "source": "user" })
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

fn scene_errors(root: &std::path::Path) -> Vec<orbe_scene::Diagnostic> {
    orbe_scene::read(&root.join("scene.jsonc"))
        .err()
        .unwrap_or_else(|| panic!("scene read with no findings"))
}

fn rewrite_lock(root: &std::path::Path, id: &str, version: &str, registry: &str) {
    let hash = orbe_scene::folder_hash(&root.join("parts").join(id)).unwrap();
    let new_line = format!("{id} {version} {registry} {hash}");
    let path = root.join("parts.lock");
    let text = fs::read_to_string(&path).unwrap();
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            if line.split_whitespace().next() == Some(id) {
                new_line.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    fs::write(&path, lines.join("\n") + "\n").unwrap();
}

fn edit_led_resistance(root: &std::path::Path) {
    let file = root.join("parts/led-5mm/part.jsonc");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        text.replacen("\"value\": 2.115", "\"value\": 1000", 1),
    )
    .unwrap();
}

#[test]
fn a_sealed_edit_with_a_foreign_lock_version_is_part_modified() {
    let project = project("nano-button");
    edit_led_resistance(project.path());
    rewrite_lock(project.path(), "led-5mm", "9.9.9", orbe_scene::REGISTRY);
    let found = scene_errors(project.path());
    let modified: Vec<_> = found
        .iter()
        .filter(|d| d.code == orbe_scene::Code::PartModified)
        .collect();
    assert_eq!(modified.len(), 1, "{found:#?}");
    let d = modified[0];
    assert_eq!(d.file, "parts/led-5mm/part.jsonc");
    assert_eq!((d.line, d.column), (24, 46));
    assert!(
        d.message
            .contains("`params.color.choices.red.sets./series_resistance.value`"),
        "{}",
        d.message
    );
    assert!(d.message.contains("led-5mm@0.1.0"), "{}", d.message);
    assert!(d.hint.contains("orbe add"), "{}", d.hint);
}

#[test]
fn a_lock_version_outside_the_catalog_is_an_error() {
    let project = project("nano-button");
    rewrite_lock(project.path(), "led-5mm", "9.9.9", orbe_scene::REGISTRY);
    let found = scene_errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::PartModified);
    assert_eq!(found[0].file, "parts.lock");
    assert_eq!((found[0].line, found[0].column), (13, 1));
    assert!(found[0].message.contains("9.9.9"), "{}", found[0].message);
    assert!(
        found[0].message.contains("led-5mm@0.1.0"),
        "{}",
        found[0].message
    );
    assert!(!found[0].hint.is_empty());
}

#[test]
fn a_lock_registry_outside_the_catalog_is_an_error() {
    let project = project("nano-button");
    rewrite_lock(
        project.path(),
        "led-5mm",
        "0.1.0",
        "https://example.com/parts",
    );
    let found = scene_errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::PartModified);
    assert_eq!(found[0].file, "parts.lock");
    assert_eq!((found[0].line, found[0].column), (13, 1));
    assert!(
        found[0].message.contains("https://example.com/parts"),
        "{}",
        found[0].message
    );
    assert!(
        found[0].message.contains("led-5mm@0.1.0"),
        "{}",
        found[0].message
    );
    assert!(!found[0].hint.is_empty());
}

#[test]
fn deleting_a_sealed_model_does_not_unseal_the_part() {
    let project = project("nano-button");
    let file = project.path().join("parts/resistor-axial/part.jsonc");
    let text = fs::read_to_string(&file).unwrap();
    let from = "  \"model\": {\n    \"kind\": \"resistor\",\n    \"ratings\": { \"power\": { \"value\": 0.25, \"unit\": \"W\", \"source\": \"user\" } },\n  },";
    let edited = text.replacen(
        from,
        "  \"parts\": { \"r\": { \"type\": \"resistor-0603\", \"resistance\": \"1\" } },",
        1,
    );
    assert_ne!(text, edited);
    fs::write(&file, edited).unwrap();
    let found = scene_errors(project.path());
    let modified: Vec<_> = found
        .iter()
        .filter(|d| d.code == orbe_scene::Code::PartModified)
        .collect();
    assert_eq!(modified.len(), 1, "{found:#?}");
    assert_eq!(modified[0].file, "parts/resistor-axial/part.jsonc");
    assert_eq!((modified[0].line, modified[0].column), (15, 3));
    assert!(
        modified[0].message.contains("resistor-axial@0.1.0"),
        "{}",
        modified[0].message
    );
    assert!(
        modified[0].message.contains("differs"),
        "{}",
        modified[0].message
    );
    assert!(
        modified[0].hint.contains("orbe add"),
        "{}",
        modified[0].hint
    );
}

#[test]
fn a_symlink_loop_in_a_sealed_copy_is_an_error() {
    let project = project("nano-button");
    let folder = project.path().join("parts/led-5mm");
    std::os::unix::fs::symlink(".", folder.join("loop")).unwrap();
    edit_led_resistance(project.path());
    let found = scene_errors(project.path());
    let io: Vec<_> = found
        .iter()
        .filter(|d| d.code == orbe_scene::Code::Io)
        .collect();
    let modified: Vec<_> = found
        .iter()
        .filter(|d| d.code == orbe_scene::Code::PartModified)
        .collect();
    assert_eq!(io.len(), 1, "{found:#?}");
    assert_eq!(io[0].file, "parts/led-5mm/part.jsonc");
    assert_eq!((io[0].line, io[0].column), (1, 1));
    assert!(
        io[0].message.contains("cannot hash parts/led-5mm"),
        "{}",
        io[0].message
    );
    assert!(io[0].hint.contains("link"), "{}", io[0].hint);
    assert_eq!(modified.len(), 1, "{found:#?}");
    assert_eq!((modified[0].line, modified[0].column), (24, 46));
    assert!(
        modified[0].message.contains("led-5mm@0.1.0"),
        "{}",
        modified[0].message
    );
}

#[test]
fn a_symlink_loop_without_an_edit_does_not_pass() {
    let project = project("nano-button");
    let folder = project.path().join("parts/led-5mm");
    std::os::unix::fs::symlink(".", folder.join("loop")).unwrap();
    let found = scene_errors(project.path());
    assert!(
        found
            .iter()
            .any(|d| d.code == orbe_scene::Code::Io && d.hint.contains("link")),
        "{found:#?}"
    );
}

#[test]
fn a_passed_through_bounce_default_matches_setting_it() {
    let direct = project("nano-button");
    let scene = direct.path().join("scene.jsonc");
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
    let direct = orbe_scene::read(&scene).unwrap();

    let via = project("nano-button");
    fs::write(
        via.path().join("parts/bouncy.jsonc"),
        "{\n  \"format\": 1,\n  \"params\": { \"bounce\": { \"default\": \"1ms\" } },\n  \"parts\": { \"sw\": { \"type\": \"tactile-switch-6mm\", \"bounce\": { \"param\": \"bounce\" } } },\n  \"pins\": { \"a\": \"sw.a\", \"b\": \"sw.b\" }\n}\n",
    )
    .unwrap();
    let scene = via.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(
        &scene,
        text.replacen(
            "{ \"type\": \"tactile-switch-6mm\" }",
            "{ \"type\": \"./parts/bouncy.jsonc\" }",
            1,
        ),
    )
    .unwrap();
    let via = orbe_scene::read(&scene).unwrap_or_else(|d| panic!("{d:#?}"));
    let bounce = |json: &str| {
        let value: Value = serde_json::from_str(json).unwrap();
        value["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "btn1" || p["name"] == "btn1__sw")
            .unwrap()["bounce"]
            .clone()
    };
    assert_eq!(bounce(&direct.json), bounce(&via.json));
}

#[test]
fn max_bounces_without_bounce_is_a_finding() {
    let project = project("nano-button");
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(
        &scene,
        text.replacen(
            "{ \"type\": \"tactile-switch-6mm\" }",
            "{ \"type\": \"tactile-switch-6mm\", \"max_bounces\": 4 }",
            1,
        ),
    )
    .unwrap();
    let found = scene_errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::Param);
    assert_eq!(found[0].file, "scene.jsonc");
    assert_eq!((found[0].line, found[0].column), (6, 60));
    assert_eq!(
        found[0].message,
        "`btn1.max_bounces` has no effect while `bounce` is off; set `bounce` too"
    );
    assert!(found[0].hint.contains("\"1ms\""), "{}", found[0].hint);
}

/// The project as the fixture leaves it, with `from` replaced by `to` in the scene.
fn with_scene(from: &str, to: &str) -> tempfile::TempDir {
    let project = project("nano-button");
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    assert!(text.contains(from), "{from}");
    fs::write(&scene, text.replacen(from, to, 1)).unwrap();
    project
}

fn engine_part(netlist: &orbe_scene::Netlist, name: &str) -> Value {
    let json: Value = serde_json::from_str(&netlist.json).unwrap();
    json["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no engine part {name}"))
        .clone()
}

const BTN1: &str = "{ \"type\": \"tactile-switch-6mm\" }";

/// A copy of the catalog ATmega as a project part, with a `clock` param in Hz that sets the
/// engine's integer `clock_hz`, used as `u2`.
fn clock_project(clock: Option<&str>) -> tempfile::TempDir {
    let project = project("nano-button");
    let catalog =
        fs::read_to_string(common::repo().join("parts/atmega328p-tqfp32/part.jsonc")).unwrap();
    let firmware = "\"firmware\": { \"file\": \"hex\", \"as\": \"text\", \"required\": true, \"sets\": \"/firmware\" }";
    assert!(catalog.contains(firmware));
    let part = catalog
        .replacen("\"part\": \"atmega328p-tqfp32\", \"version\": \"0.1.0\", ", "", 1)
        .replacen(
            firmware,
            &format!("{firmware}, \"clock\": {{ \"unit\": \"Hz\", \"default\": \"16M\", \"sets\": \"/clock_hz\" }}"),
            1,
        );
    fs::write(project.path().join("parts/mcu.jsonc"), part).unwrap();
    let clock = clock
        .map(|c| format!(", \"clock\": {c}"))
        .unwrap_or_default();
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene)
        .unwrap()
        .replacen(
            "\"parts\": {",
            &format!("\"parts\": {{\n    \"u2\": {{ \"type\": \"./parts/mcu.jsonc\", \"firmware\": \"build/button.ino.hex\"{clock} }},"),
            1,
        )
        .replacen("\"wires\": [", "\"wires\": [\n    [\"u2.GND\", \"nano.GND\"],", 1);
    fs::write(&scene, text).unwrap();
    project
}

#[test]
fn a_hertz_value_reaches_an_integer_field_as_an_integer() {
    for (clock, hz) in [
        (None, 16_000_000u64),
        (Some("\"8M\""), 8_000_000),
        (Some("8000000"), 8_000_000),
        (
            Some("{ \"value\": 8000000, \"unit\": \"Hz\", \"source\": \"user\" }"),
            8_000_000,
        ),
        (
            Some("{ \"value\": 8000000.0, \"unit\": \"Hz\", \"source\": \"user\" }"),
            8_000_000,
        ),
    ] {
        let project = clock_project(clock);
        let netlist = orbe_scene::read(&project.path().join("scene.jsonc"))
            .unwrap_or_else(|d| panic!("{clock:?}: {d:#?}"));
        let value = &engine_part(&netlist, "u2")["clock_hz"]["value"];
        assert_eq!(value.as_u64(), Some(hz), "{clock:?}: {value}");
    }
}

#[test]
fn a_whole_value_object_is_an_integer() {
    let project = with_scene(
        BTN1,
        "{ \"type\": \"tactile-switch-6mm\", \"bounce\": { \"value\": 1000000000.0, \"unit\": \"ps\", \"source\": \"user\" }, \"max_bounces\": { \"value\": 2.0, \"unit\": \"1\", \"source\": \"user\" } }",
    );
    let netlist =
        orbe_scene::read(&project.path().join("scene.jsonc")).unwrap_or_else(|d| panic!("{d:#?}"));
    let bounce = &engine_part(&netlist, "btn1")["bounce"];
    assert_eq!(
        bounce["window"]["value"].as_u64(),
        Some(1_000_000_000),
        "{bounce}"
    );
    assert_eq!(bounce["max_bounces"]["value"].as_u64(), Some(2), "{bounce}");
}

/// A switch part whose model already has a bounce object, with `bounce` defaulting to 1 ms.
const ON_SWITCH: &str = r#"{
  "format": 1,
  "pins": { "a": [1], "b": [2] },
  "params": {
    "bounce": { "unit": "s", "default": "1ms", "sets": "/bounce/window" },
    "speed": { "default": "fast", "choices": { "fast": { "sets": { "/bounce/max_bounces": { "value": 2.0, "unit": "1", "source": "user" } } } } },
  },
  "model": {
    "kind": "switch",
    "on_resistance": { "value": 0.1, "unit": "ohm", "source": "user" },
    "off_resistance": { "value": 1e8, "unit": "ohm", "source": "user" },
    "initial": "open",
    "events": [],
    "bounce": { "window": { "value": 1000000000, "unit": "ps", "source": "user" }, "max_bounces": { "value": 3, "unit": "1", "source": "user" } },
  },
}
"#;

#[test]
fn a_set_bounce_takes_effect_whatever_its_default() {
    let direct = with_scene(
        BTN1,
        "{ \"type\": \"./parts/sw.jsonc\", \"bounce\": \"2ms\" }",
    );
    fs::write(direct.path().join("parts/sw.jsonc"), ON_SWITCH).unwrap();
    let netlist =
        orbe_scene::read(&direct.path().join("scene.jsonc")).unwrap_or_else(|d| panic!("{d:#?}"));
    let bounce = &engine_part(&netlist, "btn1")["bounce"];
    assert_eq!(
        bounce["window"]["value"].as_u64(),
        Some(2_000_000_000),
        "{bounce}"
    );
    assert_eq!(
        bounce["max_bounces"]["value"].as_u64(),
        Some(2),
        "the choice's 2.0 is an integer: {bounce}"
    );

    for child in ["./sw.jsonc", "tactile-switch-6mm"] {
        let via = with_scene(BTN1, "{ \"type\": \"./parts/bouncy.jsonc\" }");
        fs::write(via.path().join("parts/sw.jsonc"), ON_SWITCH).unwrap();
        fs::write(
            via.path().join("parts/bouncy.jsonc"),
            format!("{{\n  \"format\": 1,\n  \"params\": {{ \"bounce\": {{ \"default\": \"2ms\" }} }},\n  \"parts\": {{ \"sw\": {{ \"type\": \"{child}\", \"bounce\": {{ \"param\": \"bounce\" }} }} }},\n  \"pins\": {{ \"a\": \"sw.a\", \"b\": \"sw.b\" }}\n}}\n"),
        )
        .unwrap();
        let netlist = orbe_scene::read(&via.path().join("scene.jsonc"))
            .unwrap_or_else(|d| panic!("{child}: {d:#?}"));
        let bounce = &engine_part(&netlist, "btn1__sw")["bounce"];
        assert_eq!(
            bounce["window"]["value"].as_u64(),
            Some(2_000_000_000),
            "{child}: {bounce}"
        );
    }
}

#[test]
fn a_deleted_copy_is_only_part_missing() {
    let project = project("nano-button");
    fs::remove_dir_all(project.path().join("parts/led-5mm")).unwrap();
    let found = scene_errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::PartMissing);
    assert_eq!(found[0].file, "scene.jsonc");
    assert_eq!((found[0].line, found[0].column), (9, 23));
    assert_eq!(
        found[0].hint,
        format!("run `orbe add -C {} led-5mm`", project.path().display())
    );
}

#[test]
fn a_lock_line_the_scene_no_longer_uses_is_not_checked() {
    let project = project("nano-button");
    fs::remove_dir_all(project.path().join("parts/led-5mm")).unwrap();
    let scene = project.path().join("scene.jsonc");
    let mut text = fs::read_to_string(&scene).unwrap();
    for (from, to) in [
        (
            "\"led1\": { \"type\": \"led-5mm\", \"color\": \"red\" },",
            "",
        ),
        ("[\"r1.b\", \"led1.anode\"],", "[\"r1.b\", \"nano.GND\"],"),
        ("[\"led1.cathode\", \"nano.GND\"],", ""),
    ] {
        assert!(text.contains(from), "{from}");
        text = text.replacen(from, to, 1);
    }
    fs::write(&scene, text).unwrap();
    let netlist = orbe_scene::read(&scene).unwrap_or_else(|d| panic!("{d:#?}"));
    assert_eq!(netlist.parts, 17);
}

fn unreadable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
}

#[test]
fn an_unreadable_part_file_is_one_io_error() {
    let project = project("nano-button");
    unreadable(&project.path().join("parts/led-5mm/part.jsonc"));
    let found = scene_errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::Io);
    assert_eq!(found[0].file, "parts/led-5mm/part.jsonc");
    assert!(
        found[0]
            .message
            .contains("cannot read parts/led-5mm/part.jsonc"),
        "{}",
        found[0].message
    );
    assert!(
        found[0].hint.contains("chmod u+r parts/led-5mm/part.jsonc"),
        "{}",
        found[0].hint
    );
}

#[test]
fn an_unreadable_file_in_a_copy_is_one_io_error_that_names_it() {
    let project = project("nano-button");
    let extra = project.path().join("parts/led-5mm/datasheet.txt");
    fs::write(&extra, "kept").unwrap();
    unreadable(&extra);
    let found = scene_errors(project.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::Io);
    assert!(
        found[0]
            .message
            .contains("cannot read parts/led-5mm/datasheet.txt"),
        "{}",
        found[0].message
    );
    assert!(
        found[0]
            .hint
            .contains("chmod u+r parts/led-5mm/datasheet.txt"),
        "{}",
        found[0].hint
    );
    assert!(!found[0].hint.contains("link"), "{}", found[0].hint);
}

fn pin_net(json: &str, part: &str, pin: &str) -> String {
    let value: Value = serde_json::from_str(json).unwrap();
    value["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == part)
        .unwrap_or_else(|| panic!("no part {part}"))["pins"][pin]
        .as_str()
        .unwrap()
        .to_string()
}

fn jumper_board(root: &std::path::Path, drop_jumper: bool) {
    let mark = if drop_jumper {
        ", \"not_simulated\": \"left out\""
    } else {
        ""
    };
    fs::write(
        root.join("parts/jumper.jsonc"),
        "{\n  \"format\": 1,\n  \"pins\": { \"a\": [1], \"b\": [2] },\n  \"wires\": [[\"a\", \"b\"]]\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("parts/board.jsonc"),
        format!(
            "{{\n  \"format\": 1,\n  \"parts\": {{\n    \"j\": {{ \"type\": \"./jumper.jsonc\"{mark} }},\n    \"ra\": {{ \"type\": \"resistor-axial\", \"resistance\": \"100\" }},\n    \"rb\": {{ \"type\": \"resistor-axial\", \"resistance\": \"100\" }}\n  }},\n  \"wires\": [[\"j.a\", \"ra.a\"], [\"j.b\", \"rb.a\"]],\n  \"pins\": {{ \"pa\": \"ra.b\", \"pb\": \"rb.b\" }}\n}}\n"
        ),
    )
    .unwrap();
    let scene = root.join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    let text = text.replacen(
        "\"led1\": { \"type\": \"led-5mm\", \"color\": \"red\" },",
        "\"led1\": { \"type\": \"led-5mm\", \"color\": \"red\" },\n    \"board\": { \"type\": \"./parts/board.jsonc\" },",
        1,
    );
    let text = text.replacen(
        "[\"led1.cathode\", \"nano.GND\"],",
        "[\"led1.cathode\", \"nano.GND\"],\n    [\"board.pa\", \"nano.D13\"],\n    [\"board.pb\", \"nano.GND\"],",
        1,
    );
    fs::write(&scene, text).unwrap();
}

#[test]
fn a_dropped_jumper_does_not_join_its_wires() {
    let kept = project("nano-button");
    jumper_board(kept.path(), false);
    let kept =
        orbe_scene::read(&kept.path().join("scene.jsonc")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert_eq!(
        pin_net(&kept.json, "board__ra", "a"),
        pin_net(&kept.json, "board__rb", "a")
    );

    let dropped = project("nano-button");
    jumper_board(dropped.path(), true);
    let dropped =
        orbe_scene::read(&dropped.path().join("scene.jsonc")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert_ne!(
        pin_net(&dropped.json, "board__ra", "a"),
        pin_net(&dropped.json, "board__rb", "a")
    );
}

/// A local switch with `params` and a model whose `bounce` is `bounce` and whose events are
/// `events`, used as `btn1` with `values`.
fn switch_project(params: &str, bounce: &str, events: &str, values: &str) -> tempfile::TempDir {
    let project = with_scene(
        BTN1,
        &format!("{{ \"type\": \"./parts/sw.jsonc\"{values} }}"),
    );
    fs::write(
        project.path().join("parts/sw.jsonc"),
        format!(
            r#"{{
  "format": 1,
  "pins": {{ "a": [1], "b": [2] }},
  "params": {{ {params} }},
  "model": {{
    "kind": "switch",
    "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
    "off_resistance": {{ "value": 1e8, "unit": "ohm", "source": "user" }},
    "initial": "open",
    "events": {events},
    "bounce": {bounce},
  }},
}}
"#
        ),
    )
    .unwrap();
    project
}

#[test]
fn every_netlist_counts_the_parts_it_writes() {
    let mut netlists = Vec::new();
    for scene in ["scene.jsonc", "two-buttons.jsonc", "two-buttons-nets.jsonc"] {
        netlists.push(read(scene).0);
    }
    let chosen = switch_project(
        r#""contacts": { "required": true, "choices": { "gold": { "sets": { "/bounce": { "window": { "value": 1000000000, "unit": "ps", "source": "user" } } } } } }, "max_bounces": { "unit": "1", "default": 2, "sets": "/bounce/max_bounces" }"#,
        "null",
        "[]",
        ", \"contacts\": \"gold\"",
    );
    netlists.push(
        orbe_scene::read(&chosen.path().join("scene.jsonc")).unwrap_or_else(|d| panic!("{d:#?}")),
    );
    for netlist in netlists {
        let json: Value = serde_json::from_str(&netlist.json).unwrap();
        assert_eq!(
            netlist.parts,
            json["parts"].as_array().unwrap().len(),
            "the count is the parts written"
        );
    }
}

#[test]
fn a_choice_that_writes_the_whole_object_opens_it() {
    let contacts = r#""contacts": { "required": true, "choices": {
      "gold": { "sets": { "/bounce": { "window": { "value": 1000000000, "unit": "ps", "source": "user" } } } },
      "silver": { "sets": { "/bounce": { "window": { "value": 3000000000, "unit": "ps", "source": "user" } } } } } }"#;
    let max = r#""max_bounces": { "unit": "1", "default": 2, "sets": "/bounce/max_bounces" }"#;
    for (params, values, window, edges) in [
        (
            format!("{contacts}, {max}"),
            ", \"contacts\": \"gold\", \"max_bounces\": 4",
            1_000_000_000u64,
            4u64,
        ),
        (
            format!("{max}, {contacts}"),
            ", \"contacts\": \"silver\"",
            3_000_000_000,
            2,
        ),
    ] {
        let project = switch_project(&params, "null", "[]", values);
        let netlist = orbe_scene::read(&project.path().join("scene.jsonc"))
            .unwrap_or_else(|d| panic!("{values}: {d:#?}"));
        let bounce = &engine_part(&netlist, "btn1")["bounce"];
        assert_eq!(bounce["window"]["value"].as_u64(), Some(window), "{bounce}");
        assert_eq!(
            bounce["max_bounces"]["value"].as_u64(),
            Some(edges),
            "{bounce}"
        );
    }
}

#[test]
fn a_choice_sets_a_list_item() {
    let project = switch_project(
        r#""late": { "default": "no", "choices": { "no": { "sets": { "/events/0/t_ps": 1000000000 } }, "yes": { "sets": { "/events/0/t_ps": 2000000000 } } } }"#,
        "null",
        r#"[{ "t_ps": 1000000000, "state": "closed" }]"#,
        ", \"late\": \"yes\"",
    );
    let netlist =
        orbe_scene::read(&project.path().join("scene.jsonc")).unwrap_or_else(|d| panic!("{d:#?}"));
    assert_eq!(
        engine_part(&netlist, "btn1")["events"][0]["t_ps"].as_u64(),
        Some(2_000_000_000)
    );
}

#[test]
fn an_unset_null_default_keeps_the_model_value() {
    let resistor = |values: &str| {
        let project = with_scene(
            "\"r1\": { \"type\": \"resistor-axial\", \"resistance\": \"220\" }",
            &format!("\"r1\": {{ \"type\": \"./parts/r.jsonc\"{values} }}"),
        );
        fs::write(
            project.path().join("parts/r.jsonc"),
            r#"{
  "format": 1,
  "pins": { "a": [1], "b": [2] },
  "params": { "wipe": { "unit": "ohm", "default": null, "sets": "/resistance" } },
  "model": {
    "kind": "resistor",
    "resistance": { "value": 1000, "unit": "ohm", "source": "user" },
    "ratings": { "power": null },
  },
}
"#,
        )
        .unwrap();
        let netlist = orbe_scene::read(&project.path().join("scene.jsonc"))
            .unwrap_or_else(|d| panic!("{values}: {d:#?}"));
        engine_part(&netlist, "r1")["resistance"]["value"].clone()
    };
    assert_eq!(resistor(""), 1000);
    assert_eq!(resistor(", \"wipe\": \"470\""), 470);
}

/// Every order of `params`' items.
fn orders(params: &[&str]) -> Vec<Vec<String>> {
    if params.len() <= 1 {
        return vec![params.iter().map(|p| p.to_string()).collect()];
    }
    let mut all = Vec::new();
    for (i, first) in params.iter().enumerate() {
        let mut rest = params.to_vec();
        rest.remove(i);
        for mut tail in orders(&rest) {
            tail.insert(0, first.to_string());
            all.push(tail);
        }
    }
    all
}

#[test]
fn reordering_param_declarations_does_not_change_the_netlist() {
    let params = [
        r#""contacts": { "default": "ideal", "choices": {
      "ideal": { "sets": { "/bounce": null } },
      "gold": { "sets": { "/bounce": { "window": { "value": 5000000000, "unit": "ps", "source": "user" }, "max_bounces": { "value": 3, "unit": "1", "source": "user" } } } },
      "silver": { "sets": { "/bounce": { "window": { "value": 3000000000, "unit": "ps", "source": "user" } } } } } }"#,
        r#""bounce": { "unit": "s", "default": null, "sets": "/bounce/window" }"#,
        r#""max_bounces": { "unit": "1", "default": 2, "sets": "/bounce/max_bounces" }"#,
    ];
    for (values, window, edges) in [
        (
            ", \"contacts\": \"gold\"",
            Some(5_000_000_000u64),
            Some(3u64),
        ),
        (
            ", \"contacts\": \"gold\", \"max_bounces\": 5",
            Some(5_000_000_000),
            Some(5),
        ),
        (", \"contacts\": \"silver\"", Some(3_000_000_000), Some(2)),
        (
            ", \"contacts\": \"silver\", \"bounce\": \"1ms\"",
            Some(1_000_000_000),
            Some(2),
        ),
        (", \"bounce\": \"1ms\"", Some(1_000_000_000), Some(2)),
        ("", None, None),
    ] {
        let mut seen = Vec::new();
        for order in orders(&params) {
            let project = switch_project(&order.join(", "), "null", "[]", values);
            let netlist = orbe_scene::read(&project.path().join("scene.jsonc"))
                .unwrap_or_else(|d| panic!("{values} {order:?}: {d:#?}"));
            seen.push(engine_part(&netlist, "btn1"));
        }
        let bounce = &seen[0]["bounce"];
        assert_eq!(
            bounce["window"]["value"].as_u64(),
            window,
            "{values}: {bounce}"
        );
        assert_eq!(
            bounce["max_bounces"]["value"].as_u64(),
            edges,
            "{values}: {bounce}"
        );
        assert!(
            seen.iter().all(|part| *part == seen[0]),
            "{values}: the order of params changed btn1: {seen:#?}"
        );
    }
}

#[test]
fn every_catalog_part_file_reads_without_a_finding() {
    let project = project("nano-button");
    let ids = common::locked_ids(project.path());
    let catalog: Vec<String> = fs::read_dir(common::repo().join("parts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        ids.len(),
        catalog.len(),
        "the fixture locks every catalog part"
    );
    let required = |id: &str| match id {
        "capacitor-0603"
        | "capacitor-tantalum-3216"
        | "capacitor-ceramic-disc"
        | "capacitor-electrolytic-radial" => ", \"capacitance\": \"100n\"",
        "resistor-axial" | "resistor-0603" | "potentiometer-wh148" => ", \"resistance\": \"1k\"",
        "atmega328p-tqfp32" | "nano-board" => ", \"firmware\": \"build/button.ino.hex\"",
        "led-5mm" | "led-0603" | "led-3mm" => ", \"color\": \"red\"",
        _ => "",
    };
    let parts: String = ids
        .iter()
        .enumerate()
        .map(|(i, id)| format!("\"u{i}\": {{ \"type\": \"{id}\"{} }}, ", required(id)))
        .collect();
    fs::write(
        project.path().join("scene.jsonc"),
        format!("{{ \"format\": 1, \"parts\": {{ {parts} }} }}"),
    )
    .unwrap();
    let found = orbe_scene::read(&project.path().join("scene.jsonc"))
        .err()
        .unwrap_or_default();
    let in_parts: Vec<_> = found.iter().filter(|d| d.file != "scene.jsonc").collect();
    assert!(in_parts.is_empty(), "{in_parts:#?}");
    for id in &ids {
        assert!(
            catalog.contains(id),
            "{id} is in the fixture's parts.lock but not in parts/"
        );
    }
}

#[test]
fn a_bad_item_at_the_end_of_a_long_list_is_placed_quickly() {
    let short = bad_item_at_the_end_of(4_000);
    let long = bad_item_at_the_end_of(32_000);
    assert!(
        long < short * 16,
        "8 times the bytes took {long:?} against {short:?}; a search that grows with the square of the list is back"
    );
}

/// Places the error on a 256 after `count - 1` good bytes, and returns how long reading took.
#[expect(clippy::disallowed_methods, reason = "the clock is the measurement")]
fn bad_item_at_the_end_of(count: usize) -> std::time::Duration {
    let project = with_scene(
        "\"led1\": { \"type\": \"led-5mm\", \"color\": \"red\" },",
        "\"led1\": { \"type\": \"led-5mm\", \"color\": \"red\" },\n    \"tx\": { \"type\": \"./parts/ser.jsonc\" },",
    );
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap().replacen(
        "[\"led1.cathode\", \"nano.GND\"],",
        "[\"led1.cathode\", \"nano.GND\"], [\"tx.pos\", \"nano.D2\"], [\"tx.neg\", \"nano.GND\"],",
        1,
    );
    fs::write(&scene, text).unwrap();
    let bytes = format!("{}256", "85,".repeat(count - 1));
    let part = format!(
        r#"{{
  "format": 1,
  "pins": {{ "pos": [1], "neg": [2] }},
  "model": {{
    "kind": "voltage_source",
    "voltage": {{
      "kind": "serial",
      "mark": {{ "value": 5, "unit": "V", "source": "user" }},
      "space": {{ "value": 0, "unit": "V", "source": "user" }},
      "events": [{{ "t_ps": 1000000000, "baud": 115200, "frame": "8N1", "bytes": [{bytes}] }}],
    }},
    "pins": {{ "p": "pos", "n": "neg" }},
  }},
}}
"#
    );
    let column = part.lines().nth(9).unwrap().find("256").unwrap() + 1;
    fs::write(project.path().join("parts/ser.jsonc"), part).unwrap();
    let started = std::time::Instant::now();
    let found = scene_errors(project.path());
    let took = started.elapsed();
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, orbe_scene::Code::Engine("netlist.syntax"));
    assert_eq!(found[0].file, "parts/ser.jsonc");
    assert_eq!((found[0].line, found[0].column), (10, column));
    assert!(
        found[0].message.contains("256 is outside 0 to 255"),
        "{}",
        found[0].message
    );
    assert!(!found[0].hint.is_empty());
    took
}

/// The `good` fixture's SPI part with `params` in place of its own, used as `rp` with `args`.
fn spi_with(params: &Value, args: &str) -> tempfile::TempDir {
    let case = common::repo()
        .join("crates/orbe-scene/tests/good/item_of_a_list_every_choice_writes_at_another_depth");
    let project = common::overlaid(&case);
    let part = project.path().join("parts/spi.jsonc");
    let text = fs::read_to_string(&part).unwrap();
    let line = text.lines().find(|l| l.contains("\"params\":")).unwrap();
    fs::write(
        &part,
        text.replacen(line, &format!("  \"params\": {params},"), 1),
    )
    .unwrap();
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(&scene, text.replacen("\"n\": \"nine\"", args, 1)).unwrap();
    project
}

/// A choice param that writes `pointer`, with `arms` arms.
fn chooser(pointer: &str, arms: usize, value: impl Fn(usize) -> Value) -> Value {
    let choices: serde_json::Map<String, Value> = (0..arms)
        .map(|j| {
            (
                format!("a{j}"),
                serde_json::json!({ "sets": { pointer: value(j) } }),
            )
        })
        .collect();
    serde_json::json!({ "default": "a0", "choices": choices })
}

#[test]
#[expect(clippy::disallowed_methods, reason = "the clock is the measurement")]
fn choice_params_on_one_way_are_checked_in_time_that_follows_their_arms() {
    let event = |j: usize| serde_json::json!([{ "t_ps": 10_000_000 + j * 1_000_000, "lanes": 1, "bytes": [85], "cs": "released", "phases": [{ "bits": 8, "lanes": 1, "value": 1 }] }]);
    let deep = serde_json::json!({
        "pe": chooser("/events", 30, event),
        "pe0": chooser("/events/0", 30, |j| serde_json::json!({ "t_ps": 10_000_000 + j * 1_000_000 })),
        "pp": chooser("/events/0/phases", 30, |j| serde_json::json!([{ "bits": 8, "lanes": 1, "value": j % 2 }])),
        "pp0": chooser("/events/0/phases/0", 30, |j| serde_json::json!({ "value": j % 2 })),
        "bits": { "default": "eight", "choices": { "eight": { "sets": { "/events/0/phases/0/bits": 8 } }, "four": { "sets": { "/events/0/phases/0/bits": 4 } } } },
    });
    let mut one_prefix = serde_json::Map::new();
    for k in 0..20 {
        let cs = |j: usize| serde_json::json!({ "cs": if j == 0 { "released" } else { "held" } });
        one_prefix.insert(format!("p{k}"), chooser("/events/0", 2, cs));
    }
    one_prefix.insert(
        "n".into(),
        serde_json::json!({ "default": "nine", "choices": { "nine": { "sets": { "/events/0/t_ps": 9 } } } }),
    );
    let cases = [
        (
            "four params of 30 arms on one way",
            spi_with(&deep, "\"bits\": \"four\""),
        ),
        (
            "twenty params on one prefix",
            spi_with(&Value::Object(one_prefix), "\"n\": \"nine\""),
        ),
    ];
    let (send, receive) = std::sync::mpsc::channel();
    for (label, project) in cases {
        let send = send.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let found = orbe_scene::read(&project.path().join("scene.jsonc")).err();
            send.send((label, found, started.elapsed())).unwrap();
        });
    }
    for _ in 0..2 {
        let (label, found, took) = receive
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect(
                "a part file's check took over a second; it grows with the product of the arms",
            );
        println!("{label}: {took:?}");
        match label {
            "four params of 30 arms on one way" => assert!(found.is_none(), "{found:#?}"),
            _ => {
                let found = found.unwrap();
                assert_eq!(found.len(), 19, "{found:#?}");
                assert!(
                    found
                        .iter()
                        .all(|d| d.message.starts_with("params `p0` and `p")
                            && d.message.ends_with("both write `/events/0`")),
                    "{found:#?}"
                );
            }
        }
    }
}
