//! The Worker's session, run natively: a Block opened from its files and its catalog parts, as a
//! page hands them in, plays, takes a press, and says so in the viewer's messages.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_wasm::{Block, Session};
use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn add_folder(block: &mut Block, dir: &Path, prefix: &str) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if path.is_dir() {
            add_folder(block, &path, &format!("{prefix}{name}/"));
        } else {
            block.add(&format!("{prefix}{name}"), fs::read(&path).unwrap());
        }
    }
}

/// The Block's folder, and every catalog part, as a page hands them in.
fn block(id: &str, without: &str) -> Block {
    let mut block = Block::new();
    add_folder(&mut block, &repo().join("blocks").join(id), "");
    for entry in fs::read_dir(repo().join("parts")).unwrap() {
        let dir = entry.unwrap().path();
        let part = dir.file_name().unwrap().to_str().unwrap().to_string();
        if part != without {
            add_folder(&mut block, &dir, &format!("parts/{part}/"));
        }
    }
    block
}

fn messages(session: &mut Session) -> Vec<Value> {
    session
        .messages()
        .iter()
        .map(|text| serde_json::from_str(text).unwrap())
        .collect()
}

fn channel(scene: &Value, kind: &str, name: &str) -> usize {
    scene["channels"][kind]
        .as_array()
        .unwrap()
        .iter()
        .position(|r| r == name)
        .unwrap_or_else(|| panic!("no {kind} channel {name}"))
}

/// Ticks of 16 ms until `until` holds of a frame, up to `ms` of simulated time.
fn play_until(session: &mut Session, ms: u64, until: impl Fn(&Value) -> bool) -> Option<Value> {
    for _ in 0..ms / 16 {
        session.tick(16.0);
        if let Some(frame) = messages(session).into_iter().find(|m| until(m)) {
            return Some(frame);
        }
    }
    None
}

#[test]
fn a_block_opens_with_its_scene_then_a_frame() {
    let mut session = Session::start("nano-button-led", block("nano-button-led", "")).unwrap();
    let opened = messages(&mut session);
    let types: Vec<&str> = opened.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["scene", "frame"]);
    assert_eq!(opened[0]["session"], "nano-button-led");
    assert_eq!(
        opened[0]["parts"]["led-5mm"]["model"],
        "/parts/led-5mm/led-5mm.glb"
    );
    assert_eq!(
        (opened[1]["t_ps"].as_u64(), opened[1]["state"].as_str()),
        (Some(0), Some("paused"))
    );
    assert!(messages(&mut session).is_empty(), "messages drain");
}

#[test]
fn a_press_lights_the_led_while_it_plays() {
    let mut session = Session::start("t", block("nano-button-led", "")).unwrap();
    let scene = messages(&mut session).remove(0);
    let d13 = channel(&scene, "pins", "@nano.D13");
    let led = channel(&scene, "leds", "@led1");

    session.command(r#"{"id": 1, "cmd": "play"}"#);
    let played = messages(&mut session);
    assert_eq!(
        (played[0]["type"].as_str(), played[0]["id"].as_u64()),
        (Some("reply"), Some(1))
    );
    assert_eq!(played[0]["reply"]["state"], "playing");
    assert!(session.playing());

    let dark = play_until(&mut session, 160, |m| {
        m["t_ps"].as_u64() >= Some(100_000_000_000)
    });
    let dark = dark.expect("a frame at 100 ms");
    assert_eq!(dark["pins"][d13], 0);
    assert_eq!(
        dark["ratio"], 1.0,
        "a 16 ms tick of 16 ms of wall time runs at speed 1"
    );

    session.command(r#"{"id": 2, "cmd": "press", "ref": "@btn1"}"#);
    assert_eq!(messages(&mut session)[0]["reply"]["ok"], true);
    let lit = play_until(&mut session, 100, |m| {
        m["pins"][d13] == 1 && m["led_a"][led].as_f64() > Some(0.005)
    });
    assert!(lit.is_some(), "D13 goes high and led1's current rises");

    session.command(r#"{"id": 3, "cmd": "pause"}"#);
    let paused = messages(&mut session);
    assert_eq!(paused[0]["reply"]["state"], "paused");
    assert!(!session.playing());
    let at = paused[0]["reply"]["t_ps"].clone();
    session.tick(16.0);
    assert!(
        messages(&mut session).is_empty(),
        "a paused session does not move"
    );
    session.command(r#"{"id": 4, "cmd": "step", "ref": "1ms"}"#);
    let refused = messages(&mut session);
    assert_eq!(refused[0]["reply"]["error"]["code"], "usage");
    assert_eq!(refused[0]["reply"]["t_ps"], at);
}

#[test]
fn a_block_missing_a_part_is_refused_as_orbe_open_says() {
    let Err(text) = Session::start("t", block("nano-button-led", "led-5mm")) else {
        panic!("opened without led-5mm");
    };
    assert!(
        text.starts_with(
            "scene.jsonc:9:23: error[part_missing]: part `led-5mm` is not in the catalog this Block reads\n  hint: did you mean `led-3mm`?"
        ),
        "{text}"
    );
}
