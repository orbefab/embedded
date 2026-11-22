//! A potentiometer turned while the session runs: `set @pot1.position`, its read back, a test
//! file that turns it, and a page's slider. A Nano lights its LED on D13 while A0 reads above
//! half scale, and the pot's wiper feeds A0 from 0 V at `a` to 5 V at `b`.

use std::path::{Path, PathBuf};

use orbe_session::viewer::{self, Feed, TICK};
use orbe_session::{Code, Command, Files, Live};
use serde_json::json;

const SCENE: &str = r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/pot.hex" },
    "pot1": { "type": "potentiometer-wh148", "resistance": "10k", "position": 0.2 },
    "r1": { "type": "resistor-axial", "resistance": "220" },
    "led1": { "type": "led-5mm", "color": "red" },
  },
  "wires": [
    ["pot1.a", "nano.GND"],
    ["pot1.wiper", "nano.A0"],
    ["pot1.b", "nano.5V"],
    ["nano.D13", "r1.a"],
    ["r1.b", "led1.anode"],
    ["led1.cathode", "nano.GND"],
  ],
}"#;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every catalog part's files, by `<id>/<path>`.
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

fn files() -> Files {
    let block = json!({ "format": 1, "block": "pot-threshold", "version": "0.0.0", "title": "pot", "board": "nano-board", "license": "MIT" });
    let hex = std::fs::read(repo().join("packages/engine/tests/firmware/sketch/pot_threshold.hex"))
        .unwrap();
    Files::memory([
        ("block.jsonc".to_string(), block.to_string().into_bytes()),
        ("build/pot.hex".to_string(), hex),
        ("scene.jsonc".to_string(), SCENE.as_bytes().to_vec()),
    ])
    .with_catalog(catalog())
}

fn command(line: &str) -> Command {
    Command::parse(line, Path::new("")).unwrap().unwrap()
}

fn opened() -> Live {
    let mut live = Live::new("t", files());
    let reply = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(reply.ok(), "{}", reply.text());
    live
}

/// Runs `line` and returns its text, failing on a refused command.
fn ok(live: &mut Live, line: &str) -> String {
    let reply = live.run(&command(line));
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.text()
}

fn volts(text: &str) -> f64 {
    let words: Vec<&str> = text.split_whitespace().collect();
    words[words.len() - 2].parse().unwrap()
}

#[test]
fn set_turns_the_knob_and_the_next_step_reads_it() {
    let mut live = opened();
    ok(&mut live, "step 100ms");
    ok(&mut live, "expect @led1 off");
    assert_eq!(ok(&mut live, "get @pot1.position"), "pot1.position 0.2");
    let rail = volts(&ok(&mut live, "get @nano.5V"));
    let low = volts(&ok(&mut live, "get @pot1.wiper"));
    assert!((low / rail - 0.2).abs() < 1e-3, "{low} of {rail}");

    assert_eq!(
        ok(&mut live, "set @pot1.position 0.8"),
        "set pot1.position to 0.8 at t=100ms; reads show it after t=100ms"
    );
    assert_eq!(
        ok(&mut live, "get @pot1.position"),
        "pot1.position 0.2",
        "a read at now shows the circuit before the set lands"
    );
    ok(&mut live, "step --until @led1 lit --timeout 100ms");
    assert_eq!(ok(&mut live, "get @pot1.position"), "pot1.position 0.8");
    let rail = volts(&ok(&mut live, "get @nano.5V"));
    let high = volts(&ok(&mut live, "get @pot1.wiper"));
    assert!((high / rail - 0.8).abs() < 1e-3, "{high} of {rail}");
    ok(&mut live, "step 50ms");
    let serial = live.run(&command("snapshot")).json()["data"]["serial"]
        .as_str()
        .unwrap()
        .to_string();
    let readings: Vec<u32> = serial
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect();
    assert!(
        matches!(readings.as_slice(), [low, high] if (190..215).contains(low) && (800..830).contains(high)),
        "analogRead follows the knob: {serial:?}"
    );

    let json = live.run(&command("get @pot1.position")).json();
    assert_eq!(
        json["data"],
        json!({ "ref": "@pot1.position", "kind": "param", "value": 0.8 })
    );
    let again = live.run(&command("set @pot1.position 0.8"));
    assert!(again.ok(), "{}", again.text());
    assert_eq!(again.text(), "pot1.position is already 0.8");
    assert_eq!(again.json()["data"]["already"], true);
}

#[test]
fn a_value_out_of_range_or_a_fixed_param_is_refused_with_a_code() {
    let mut live = opened();
    let refused = |live: &mut Live, line: &str| {
        let reply = live.run(&command(line));
        let error = reply
            .error
            .unwrap_or_else(|| panic!("{line} was not refused"));
        (error.code, error.message, error.hint)
    };
    assert_eq!(
        refused(&mut live, "set @pot1.position 1.5"),
        (
            Code::Value,
            "pot1.position is above 0 and below 1, not 1.5".to_string(),
            "set pot1.position to a value above 0 and below 1".to_string()
        )
    );
    assert_eq!(
        refused(&mut live, "set @pot1.resistance 5000"),
        (
            Code::NotSettable,
            "`pot1.resistance` is fixed for the run; only a live param changes while the session runs".to_string(),
            "change it in the scene, then run `open` again; `pot1`'s live params are position".to_string()
        )
    );
    let (code, message, _) = refused(&mut live, "set @r1.resistance 330");
    assert_eq!(code, Code::NotSettable);
    assert_eq!(
        message,
        "`r1.resistance` is fixed for the run; only a live param changes while the session runs"
    );
    let (code, _, hint) = refused(&mut live, "get @r1.resistance");
    assert_eq!(code, Code::NoReading);
    assert_eq!(hint, "change it in the scene, then run `open` again");
    let (code, message, _) = refused(&mut live, "set @pot1.wiper 0.5");
    assert_eq!(
        (code, message),
        (Code::NotSettable, "@pot1.wiper is not a param".to_string())
    );
    let (code, _, hint) = refused(&mut live, "get @pot1.knob");
    assert_eq!(code, Code::UnknownRef);
    assert!(hint.ends_with("live params: position"), "{hint}");
    assert_eq!(
        live.run(&command("get @pot1.position")).text(),
        "pot1.position 0.2",
        "a refused set changes nothing"
    );
}

#[test]
fn a_test_file_turns_the_knob() {
    let text = "open\nstep 100ms\nexpect @led1 off\nset @pot1.position 0.8\nstep 100ms\nexpect @led1 lit\nexpect @nano.D13 high\nset @pot1.position 0.3\nstep 100ms\nexpect @led1 off\n";
    let report =
        orbe_session::test::run(files(), Path::new("tests/knob.test"), text, Path::new(""));
    assert!(
        report.failure().is_none(),
        "{:?}",
        report.failure().map(|f| &f.text)
    );
}

#[test]
fn a_page_sees_the_knob_and_turns_it_like_set() {
    let mut live = opened();
    ok(&mut live, "step 50ms");
    let scene = live.scene().unwrap();
    assert_eq!(
        scene["params"],
        json!([{ "ref": "@pot1.position", "unit": "1", "above": 0.0, "below": 1.0, "start": 0.2 }])
    );
    let knob = &scene["parts"]["potentiometer-wh148"]["joints"]["knob"];
    assert_eq!(
        knob,
        &json!({ "node": "shaft", "turn": [0.0, 0.0, -1.0], "range": 300.0, "by": { "param": "position", "min": 0.0, "max": 1.0 } })
    );
    let mut feed = Feed::default();
    let first = feed.now(&live, None);
    assert_eq!(first[0]["params"], json!([0.2]));

    let (played, _) = viewer::answer(&mut live, r#"{"id": 1, "cmd": "play"}"#);
    assert!(played.ok(), "{}", played.text());
    for _ in 0..4 {
        assert!(live.tick(TICK).is_some());
    }
    let (turned, message) = viewer::answer(
        &mut live,
        r#"{"id": 2, "cmd": "set", "ref": "@pot1.position", "value": 0.8}"#,
    );
    assert!(turned.ok(), "{}", turned.text());
    assert_eq!(message["reply"]["data"]["ref"], "@pot1.position");
    assert_eq!(feed.now(&live, None)[0]["params"], json!([0.8]));
    for _ in 0..8 {
        assert!(live.tick(TICK).is_some());
    }
    let (paused, _) = viewer::answer(&mut live, r#"{"id": 3, "cmd": "pause"}"#);
    assert!(paused.ok(), "{}", paused.text());

    let stepped = live.run(&command("step 1ms"));
    assert!(stepped.ok(), "{}", stepped.text());
    let text = stepped.text();
    assert!(
        text.contains(" nano.D13 high"),
        "the turn shows in what play ran:\n{text}"
    );
    assert!(text.contains(" led1 lit"), "{text}");
    assert_eq!(ok(&mut live, "get @pot1.position"), "pot1.position 0.8");
}

#[test]
fn a_paused_page_shows_a_set_before_it_lands() {
    let mut live = opened();
    ok(&mut live, "set @pot1.position 0.6");
    let frame = Feed::default().now(&live, None);
    assert_eq!(frame[0]["params"], json!([0.6]));
    assert_eq!(ok(&mut live, "get @pot1.position"), "pot1.position 0.2");
}
