//! What a viewer gets of the first target, read from the scene and the part files.

use std::path::Path;

use orbe_scene::Files;
use serde_json::{Value, json};

fn first_target() -> Value {
    let scene =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../blocks/nano-button-led/scene.jsonc");
    let netlist =
        orbe_scene::read(&Files::disk(), &scene).unwrap_or_else(|found| panic!("{found:?}"));
    serde_json::to_value(&netlist.view).unwrap()
}

fn instance<'v>(list: &'v Value, name: &str) -> &'v Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no instance {name}"))
}

#[test]
fn instances_carry_their_parsed_params_and_placements() {
    let view = first_target();
    let instances = &view["instances"];
    let names: Vec<&str> = instances
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["bb", "nano", "btn1", "r2", "r1", "led1"],
        "the scene's order"
    );
    assert_eq!(
        instance(instances, "r2")["params"],
        json!({ "resistance": 10000.0 })
    );
    assert_eq!(instance(instances, "led1")["params"], json!({}));
    assert_eq!(
        instance(instances, "btn1")["placement"],
        json!({ "at": [13.97, 0.0, 8.5], "rotate": [0.0, 0.0, 90.0] }),
        "plugged in across the breadboard's channel, on its top"
    );

    let nano = &instance(instances, "nano")["children"];
    assert_eq!(nano.as_array().unwrap().len(), 28);
    assert_eq!(
        instance(nano, "j1")["placement"],
        json!({ "at": [0.0, 7.62, 0.0], "rotate": [0.0, 0.0, 180.0] })
    );
    let r_tx = instance(nano, "r_tx");
    assert_eq!(r_tx["params"], json!({ "resistance": 1000.0 }));
    assert_eq!(
        r_tx["not_simulated"],
        "It feeds the TX LED, which follows the CH340G."
    );
    assert_eq!(
        instance(nano, "led_l")["params"],
        json!({ "color": "yellow" })
    );
    assert!(instance(nano, "led_l").get("not_simulated").is_none());
}

#[test]
fn parts_carry_their_3d_fields() {
    let view = first_target();
    let button = &view["parts"]["tactile-switch-6mm"];
    assert_eq!(
        button["model"],
        "/parts/tactile-switch-6mm/tactile-switch-6mm.glb"
    );
    assert_eq!(button["leads"], json!({ "a": [1, 2], "b": [3, 4] }));
    assert_eq!(
        button["controls"],
        json!({ "press": { "joint": "plunger" } })
    );
    assert_eq!(
        button["joints"],
        json!({ "plunger": { "node": "stem", "slide": [0.0, 0.0, -1.0], "range": 0.25 } })
    );
    assert_eq!(button["anchors"].as_array().unwrap().len(), 4);

    let led = &view["parts"]["led-5mm"];
    assert_eq!(led["readings"], json!({ "brightness": { "full": 0.01 } }));
    let chip = &view["parts"]["led-0603"];
    assert_eq!(
        chip["tints"],
        json!({
            "color": {
                "green": "#c5d6b0",
                "red": "#e0b0a4",
                "yellow": "#e8c56a",
            }
        })
    );
    assert_eq!(
        led["view"],
        json!({ "glow": [{ "node": "body", "by": "brightness" }] })
    );

    let nano = &view["parts"]["nano-board"];
    assert_eq!(nano["labels"]["D13"], json!(["j2.1"]));
    assert_eq!(nano["labels"]["GND"], json!(["j1.4", "j2.14"]));
    assert_eq!(nano["anchors"], json!([]));
}

#[test]
fn wires_hold_the_scene_s_and_each_part_s_with_prefixed_ends() {
    let view = first_target();
    let wires = view["wires"].as_array().unwrap();
    assert_eq!(
        wires[0],
        json!({ "ends": ["bb.a12", "bb.W11"], "kind": "jumper", "color": "#d12b2b" })
    );
    let jumpers = wires.iter().filter(|w| w["kind"] == "jumper").count();
    assert_eq!(jumpers, 6, "the scene's 6");
    assert!(
        wires
            .iter()
            .any(|w| w["ends"] == json!(["btn1.a", "bb.f22"]) && w["kind"] == "contact"),
        "a leg in its hole"
    );
    assert!(
        wires
            .iter()
            .any(|w| w["ends"] == json!(["nano.j2.1", "nano.mcu.PB5"])
                || w["ends"] == json!(["nano.mcu.PB5", "nano.j2.1"])),
        "the Nano's D13 trace"
    );
}

#[test]
fn not_simulated_pins_take_the_ref_hover_shows() {
    let view = first_target();
    let idle = view["not_simulated"].as_array().unwrap();
    let pin = idle.iter().find(|p| p["ref"] == "@nano.3V3").unwrap();
    assert_eq!(
        pin["why"],
        "It is the CH340G's 3.3 V output. The engine has no USB-serial chip, so the pin supplies nothing."
    );
    assert!(
        idle.iter().all(|p| p["ref"] != "@nano.j2.2"),
        "header pin 2 shows as its label"
    );
    assert!(
        idle.iter().all(|p| p["ref"] != "@nano.A6"),
        "A6 reads its voltage"
    );
    assert!(idle.iter().all(|p| p["ref"] != "@nano.D13"));
}
