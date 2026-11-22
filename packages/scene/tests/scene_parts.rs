//! The parts a scene places, for a page that lists them.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use orbe_scene::{Code, Files, scene_parts};

fn repo() -> PathBuf {
    common::repo()
}

fn catalog(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut parts = Vec::new();
    for entry in fs::read_dir(root.join("parts")).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path().join("part.jsonc");
        if !path.is_file() {
            continue;
        }
        let id = entry.file_name().into_string().unwrap();
        parts.push((format!("{id}/part.jsonc"), fs::read(path).unwrap()));
    }
    parts
}

fn memory_block(root: &Path, id: &str, scene: &str) -> Files {
    let folder = root.join("blocks").join(id);
    Files::memory([
        ("block.jsonc", fs::read(folder.join("block.jsonc")).unwrap()),
        ("scene.jsonc", scene.as_bytes().to_vec()),
    ])
    .with_catalog(catalog(root))
}

#[test]
fn a_button_block_lists_its_parts_in_file_order() {
    let root = repo();
    let scene = root.join("blocks/nano-button-led/scene.jsonc");
    let parts = scene_parts(&Files::disk(), &scene).expect("the button scene reads");
    let refs: Vec<_> = parts.iter().map(|part| part.r#ref.as_str()).collect();
    assert_eq!(refs, ["bb", "nano", "btn1", "r2", "r1", "led1"]);

    let r1 = parts.iter().find(|part| part.r#ref == "r1").unwrap();
    assert_eq!(r1.part, "resistor-axial");
    assert!(!r1.press);
    assert_eq!(r1.values.len(), 1);
    assert_eq!(r1.values[0].name, "resistance");
    assert_eq!(r1.values[0].amount, Some(220.0));
    assert_eq!(r1.values[0].unit.as_deref(), Some("ohm"));

    let r2 = parts.iter().find(|part| part.r#ref == "r2").unwrap();
    assert_eq!(r2.values[0].amount, Some(10_000.0));

    let led = parts.iter().find(|part| part.r#ref == "led1").unwrap();
    assert_eq!(led.values[0].text.as_deref(), Some("red"));

    let nano = parts.iter().find(|part| part.r#ref == "nano").unwrap();
    assert_eq!(nano.values[0].file.as_deref(), Some("build/button.ino.hex"));
    assert!(
        parts
            .iter()
            .filter(|part| part.press)
            .map(|part| part.r#ref.as_str())
            .eq(["btn1"])
    );
}

#[test]
fn a_light_block_lists_the_trimmer_as_written() {
    let root = repo();
    let scene = root.join("blocks/nano-light-comparator/scene.jsonc");
    let parts = scene_parts(&Files::disk(), &scene).expect("the light scene reads");
    let trim = parts.iter().find(|part| part.r#ref == "trim").unwrap();
    assert_eq!(trim.values.len(), 2);
    assert_eq!(trim.values[0].name, "resistance");
    assert_eq!(trim.values[0].amount, Some(10_000.0));
    assert_eq!(trim.values[0].unit.as_deref(), Some("ohm"));
    assert_eq!(trim.values[1].name, "position");
    assert_eq!(trim.values[1].amount, Some(0.5));
    assert_eq!(trim.values[1].unit.as_deref(), Some("1"));
    let bottom = parts.iter().find(|part| part.r#ref == "rbot").unwrap();
    assert_eq!(bottom.values[0].amount, Some(2_200.0));
}

#[test]
fn a_block_in_memory_keeps_a_firmware_path_when_the_file_is_absent() {
    let root = repo();
    let scene = fs::read_to_string(root.join("blocks/nano-button-led/scene.jsonc")).unwrap();
    let parts = scene_parts(
        &memory_block(&root, "nano-button-led", &scene),
        Path::new("scene.jsonc"),
    )
    .expect("the scene reads without the hex");
    let nano = parts.iter().find(|part| part.r#ref == "nano").unwrap();
    assert_eq!(nano.values[0].file.as_deref(), Some("build/button.ino.hex"));
}

#[test]
fn an_unknown_param_is_the_readers_finding() {
    let root = repo();
    let scene = fs::read_to_string(root.join("blocks/nano-button-led/scene.jsonc")).unwrap();
    let scene = scene.replacen(
        "\"resistance\": \"220\"",
        "\"resistance\": \"220\", \"nope\": 1",
        1,
    );
    let found = scene_parts(
        &memory_block(&root, "nano-button-led", &scene),
        Path::new("scene.jsonc"),
    )
    .expect_err("an unknown param does not read");
    assert!(
        found
            .iter()
            .any(|d| d.code == Code::Param && d.message.contains("no param `nope`")),
        "{found:?}"
    );
}

#[test]
fn a_scene_that_does_not_parse_is_a_finding() {
    let files = Files::memory([("scene.jsonc", b"{".to_vec())]);
    let found =
        scene_parts(&files, Path::new("scene.jsonc")).expect_err("the scene does not parse");
    assert!(found.iter().any(|d| d.code == Code::Syntax), "{found:?}");
}
