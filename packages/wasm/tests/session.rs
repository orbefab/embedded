//! The Worker's session, run natively: a Block opened from its files and its catalog parts, as a
//! page hands them in, plays, takes a press, and says so in the viewer's messages.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_wasm::{Block, Session};
use serde_json::{Value, json};

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
    assert!(
        opened[0].get("world").is_none() && opened[1].get("world").is_none(),
        "a scene with no world omits world"
    );
    assert!(messages(&mut session).is_empty(), "messages drain");
}

fn light_divider() -> Block {
    let mut block = Block::new();
    add_folder(
        &mut block,
        &repo().join("packages/session/tests/light-divider"),
        "",
    );
    block
}

#[test]
fn set_light_is_on_the_next_frame() {
    let mut session = Session::start("light", light_divider()).unwrap();
    let opened = messages(&mut session);
    assert_eq!(opened[0]["world"][0]["name"], "light");
    assert_eq!(opened[1]["world"], json!([300.0]));

    session.command(r#"{"id": 1, "cmd": "play"}"#);
    assert_eq!(messages(&mut session)[0]["reply"]["state"], "playing");

    session.command(r#"{"id": 2, "cmd": "set", "quantity": "light", "value": 50}"#);
    let answered = messages(&mut session);
    let reply = &answered[0]["reply"];
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["value"].as_f64(), Some(50.0));
    let landed = reply["data"]["landed_ps"].as_u64().unwrap();

    session.tick(16.0);
    let frame = messages(&mut session)
        .into_iter()
        .find(|message| message["type"] == "frame" && message["t_ps"].as_u64().unwrap() > landed)
        .expect("a frame after the set lands");
    assert_eq!(frame["world"], json!([50.0]));
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
            "scene.jsonc:12:23: error[part_missing]: part `led-5mm` is not in the catalog this Block reads\n  hint: the catalog has no part named `led-5mm`"
        ),
        "{text}"
    );
}

/// Every catalog part's files by `<id>/<path>`, leaving out the part `without`.
fn catalog(without: &str) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}/"), out);
            } else {
                out.push((format!("{prefix}{name}"), fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(repo().join("parts")).unwrap() {
        let dir = entry.unwrap().path();
        let part = dir.file_name().unwrap().to_str().unwrap().to_string();
        if part != without {
            walk(&dir, &format!("{part}/"), &mut files);
        }
    }
    files
}

#[test]
fn a_part_draws_on_its_own_with_its_children_and_no_channel() {
    let scene = orbe_session::viewer::part_scene("nano-board", catalog("")).unwrap();
    assert_eq!(scene["type"], "scene");
    assert_eq!(scene["session"], "nano-board");
    assert_eq!(
        scene["channels"],
        serde_json::json!({ "pins": [], "leds": [], "switches": [], "pixels": [] })
    );
    assert!(scene.get("circuit_hash").is_none());
    let instances = scene["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0]["name"], "nano_board");
    assert_eq!(instances[0]["children"].as_array().unwrap().len(), 28);
    assert_eq!(
        scene["parts"]["nano-board"]["model"],
        "/parts/nano-board/nano-board.glb"
    );
    assert!(scene["parts"]["led-0603"]["tints"]["color"].is_object());

    let led = orbe_session::viewer::part_scene("led-5mm", catalog("")).unwrap();
    assert_eq!(led["instances"][0]["params"], serde_json::json!({}));
}

#[test]
fn a_missing_part_is_refused_with_the_finding() {
    let error = orbe_session::viewer::part_scene("nano-board", catalog("led-0603")).unwrap_err();
    assert!(error.contains("error[part_missing]"), "{error}");
    assert!(error.contains("led-0603"), "{error}");
}

/// The catalog-defaults Block: its own files, and catalog parts whose file params default to a
/// file in the part folder. The text default is an AVR hex. The blob default is an ELF.
fn catalog_defaults() -> Block {
    let root = repo().join("packages/scene/tests/fixtures/catalog-defaults");
    let mut block = Block::new();
    for name in ["block.jsonc", "scene.jsonc"] {
        block.add(name, fs::read(root.join(name)).unwrap());
    }
    for entry in fs::read_dir(root.join("catalog")).unwrap() {
        let dir = entry.unwrap().path();
        let id = dir.file_name().unwrap().to_str().unwrap();
        add_folder(&mut block, &dir, &format!("parts/{id}/"));
    }
    block
}

#[test]
fn a_block_opens_with_a_catalog_text_default_and_a_catalog_blob_default() {
    let mut session = Session::start("catalog-defaults", catalog_defaults())
        .unwrap_or_else(|text| panic!("{text}"));
    let opened = messages(&mut session);
    let types: Vec<&str> = opened.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["scene", "frame"]);
    assert_eq!(opened[0]["session"], "catalog-defaults");
}
