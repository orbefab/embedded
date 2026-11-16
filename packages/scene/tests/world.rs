//! A scene's `world`, as the netlist the engine reads gets it.

mod common;

use std::fs;

use orbe_scene::Files;
use serde_json::{Value, json};

fn world_of(project: &std::path::Path) -> Option<Value> {
    let netlist = orbe_scene::read(&Files::disk(), &project.join("scene.jsonc")).unwrap();
    let json: Value = serde_json::from_str(&netlist.json).unwrap();
    json.get("world").cloned()
}

fn level(value: f64, unit: &str) -> Value {
    json!({ "value": value, "unit": unit, "source": "user" })
}

fn ldr_case() -> tempfile::TempDir {
    common::overlaid(&common::repo().join("packages/scene/tests/good/world_ldr"))
}

#[test]
fn the_netlist_declares_what_the_scene_sets_in_table_order() {
    let project = ldr_case();
    assert_eq!(
        world_of(project.path()),
        Some(json!({
            "temperature": level(30.0, "degC"),
            "light": {
                "kind": "steps",
                "initial": level(300.0, "lx"),
                "events": [{ "t_ps": 20_000_000_000u64, "level": level(50.0, "lx") }],
            },
        }))
    );
}

#[test]
fn a_quantity_a_part_reads_is_declared_at_its_default() {
    let project = ldr_case();
    let scene = project.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    let start = text.find("  \"world\"").unwrap();
    fs::write(&scene, format!("{}}}\n", &text[..start])).unwrap();
    assert_eq!(
        world_of(project.path()),
        Some(json!({ "light": level(300.0, "lx") }))
    );
}

#[test]
fn a_scene_with_no_world_writes_none() {
    let project = common::project("nano-button");
    assert_eq!(world_of(project.path()), None);
}
