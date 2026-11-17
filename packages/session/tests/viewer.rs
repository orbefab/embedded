//! What a viewer gets from a session: the scene message, frames that never change a result, and
//! `play`'s ticks.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use orbe_session::viewer::{self, Feed, TICK};
use orbe_session::{Code, Command, Data, Duration, Files, FrameState, Live, Reading, Reply, State};
use serde_json::{Value, json};

fn first_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../blocks/nano-button-led")
}

fn command(line: &str) -> Command {
    Command::parse(line, &first_target()).unwrap().unwrap()
}

fn opened() -> Live {
    let mut live = Live::new("t", Files::disk());
    let reply = live.run(&command("open"));
    assert!(reply.ok(), "{}", reply.text());
    live
}

const BUTTON_TEST: [&str; 8] = [
    "step 100ms",
    "expect @led1 off",
    "press @btn1",
    "step --until @led1 lit --timeout 50ms",
    "expect @nano.D13 high",
    "release @btn1",
    "step 50ms",
    "expect @led1 off",
];

#[test]
fn frames_read_after_every_slice_change_no_reply() {
    let mut watched = opened();
    let mut frames = Feed::default();
    let mut sent: Vec<Value> = Vec::new();
    let watched_replies: Vec<Reply> = BUTTON_TEST
        .iter()
        .map(|line| {
            let reply = watched.run_with(&command(line), &mut |progress| {
                sent.extend(frames.frame(progress.sample(), None));
                ControlFlow::Continue(())
            });
            sent.extend(frames.frame(watched.sample().unwrap(), None));
            reply
        })
        .collect();

    let mut plain = opened();
    let plain_replies: Vec<Reply> = BUTTON_TEST
        .iter()
        .map(|line| plain.run(&command(line)))
        .collect();
    assert_eq!(watched_replies, plain_replies);
    assert!(watched_replies.iter().all(Reply::ok));

    let lit = sent
        .iter()
        .find(|frame| frame["led_a"][0].as_f64().unwrap() > 0.005)
        .expect("a frame with led1 lit");
    assert_eq!(lit["switches"][0], 1, "btn1 is held while led1 lights");
    assert_eq!(
        lit["pins"],
        json!([1, 1, 1]),
        "5V, D13 and D2 high, in the scene's channel order"
    );
    let seqs: Vec<u64> = sent.iter().map(|f| f["seq"].as_u64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");
    assert!(
        sent.len() > 150,
        "one frame per 1 ms slice: {} frames",
        sent.len()
    );
}

#[test]
fn a_frame_inside_a_step_says_stepping() {
    let mut live = opened();
    let mut frames = Feed::default();
    let mut inside = Vec::new();
    let stepped = live.run_with(&command("step 5ms"), &mut |progress| {
        inside.extend(frames.frame(progress.sample(), None));
        ControlFlow::Continue(())
    });
    assert!(stepped.ok(), "{}", stepped.text());
    assert_eq!(inside.len(), 5, "one frame per 1 ms slice");
    assert!(
        inside.iter().all(|f| f["state"] == "stepping"),
        "{inside:?}"
    );
    let after = frames.frame(live.sample().unwrap(), None);
    assert_eq!(
        after.expect("a frame once the step ends")["state"],
        "paused"
    );
}

#[test]
fn the_scene_message_names_each_channel() {
    let live = opened();
    let scene = live.scene().unwrap();
    assert_eq!(scene["type"], "scene");
    assert_eq!(scene["session"], "t");
    assert_eq!(
        scene["channels"],
        json!({
            "pins": ["@nano.5V", "@nano.D13", "@nano.D2"],
            "leds": ["@led1", "@nano.led_l", "@nano.led_pwr"],
            "switches": ["@btn1", "@nano.sw_reset"],
        })
    );
    assert_eq!(scene["instances"][2]["name"], "btn1");
    assert!(
        scene.get("world").is_none(),
        "a scene with no world omits world"
    );
}

#[test]
fn a_frame_shows_a_press_after_it_lands_and_skips_a_repeat() {
    let mut live = opened();
    let mut frames = Feed::default();
    let first = frames.frame(live.sample().unwrap(), None).unwrap();
    assert_eq!(first["switches"], json!([0, 0]));
    assert!(
        first.get("world").is_none(),
        "a scene with no world omits world"
    );
    assert!(frames.frame(live.sample().unwrap(), None).is_none());

    assert!(live.run(&command("press @btn1")).ok());
    let at_landing = frames.frame(live.sample().unwrap(), None);
    assert!(
        at_landing.is_none(),
        "reads at t show the circuit before an input at t"
    );
    assert!(live.run(&command("step 1ms")).ok());
    let after = frames.frame(live.sample().unwrap(), None).unwrap();
    assert_eq!(after["switches"], json!([1, 0]));
}

/// `10000 * (10 / light)` ohm, at most 1 Mohm.
fn ldr_ohm(light: f64) -> f64 {
    if light == 0.0 {
        1_000_000.0
    } else {
        (10_000.0 * (10.0 / light)).min(1_000_000.0)
    }
}

fn mid(light: f64) -> f64 {
    let ohm = ldr_ohm(light);
    5.0 * ohm / (10_000.0 + ohm)
}

fn pin_voltage(reply: &Reply) -> f64 {
    let Data::Got(got) = &reply.data else {
        panic!("{}", reply.text());
    };
    let Reading::Pin { v, .. } = &got.value else {
        panic!("{}", reply.text());
    };
    *v
}

fn close_to(found: f64, expected: f64) -> bool {
    (found - expected).abs() <= 1e-8 * expected.abs()
}

fn light_divider() -> Live {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/light-divider/scene.jsonc");
    let mut live = Live::new("t", Files::disk());
    let opened = live.run(&Command::Open(scene));
    assert!(opened.ok(), "{}", opened.text());
    live
}

#[test]
fn the_scene_message_describes_each_declared_quantity() {
    let live = light_divider();
    let scene = live.scene().unwrap();
    assert_eq!(
        scene["world"],
        json!([{
            "name": "light",
            "unit": "lx",
            "min": 0.0,
            "max": 100_000.0,
            "start": 300.0,
            "scale": { "kind": "log" },
        }])
    );
}

#[test]
fn a_paused_set_shows_on_the_next_frame_and_moves_the_divider_once_time_runs() {
    let mut live = light_divider();
    let mut frames = Feed::default();
    let first = frames.frame(live.sample().unwrap(), None).unwrap();
    assert_eq!(first["world"], json!([300.0]));
    let before = live.run(&command("get @ldr.a"));
    assert!(
        close_to(pin_voltage(&before), mid(300.0)),
        "mid {} at 300 lx",
        pin_voltage(&before)
    );

    let set = live.run(&command("set light 50"));
    assert!(set.ok(), "{}", set.text());
    let paused = frames
        .frame(live.sample().unwrap(), None)
        .expect("a paused set sends a frame");
    assert_eq!(
        (paused["state"].as_str(), &paused["world"]),
        (Some("paused"), &json!([50.0]))
    );

    let stepped = live.run(&command("step 1ms"));
    assert!(stepped.ok(), "{}", stepped.text());
    let after = frames.frame(live.sample().unwrap(), None).unwrap();
    assert_eq!(after["world"], json!([50.0]));
    let got = live.run(&command("get @ldr.a"));
    assert!(
        close_to(pin_voltage(&got), mid(50.0)),
        "mid {} at 50 lx, want {}",
        pin_voltage(&got),
        mid(50.0)
    );
}

#[test]
fn a_change_the_scene_schedules_shows_only_once_time_reaches_it() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/light-divider");
    for file in ["ldr.jsonc", "resistor.jsonc", "source.jsonc"] {
        std::fs::copy(fixture.join(file), dir.path().join(file)).unwrap();
    }
    let scene = std::fs::read_to_string(fixture.join("scene.jsonc"))
        .unwrap()
        .replace(
            r#""light": 300"#,
            r#""light": { "initial": 300, "steps": [{ "at": "2ms", "to": 0 }] }"#,
        );
    std::fs::write(dir.path().join("scene.jsonc"), scene).unwrap();
    let mut live = Live::new("t", Files::disk());
    let opened = live.run(&Command::Open(dir.path().join("scene.jsonc")));
    assert!(opened.ok(), "{}", opened.text());

    let mut world = Vec::new();
    for step in ["1ms", "500us", "1ms"] {
        assert!(live.run(&command(&format!("step {step}"))).ok());
        let mut frames = Feed::default();
        world.push(frames.frame(live.sample().unwrap(), None).unwrap()["world"].clone());
    }
    assert_eq!(world, [json!([300.0]), json!([300.0]), json!([0.0])]);
}

#[test]
fn play_ticks_until_pause() {
    let mut live = opened();
    let played = live.run(&Command::Play);
    assert!(played.ok(), "{}", played.text());
    assert_eq!(played.state, State::Playing);
    assert_eq!(played.text(), "playing from t=0s");
    assert!(live.tick(TICK).is_some());
    assert!(live.tick(TICK).is_some());
    let paused = live.run(&Command::Pause);
    assert_eq!((paused.t_ps, paused.state), (32_000_000_000, State::Paused));
    assert_eq!(paused.text(), "paused at t=32ms");
    assert!(live.tick(TICK).is_none(), "a paused session does not tick");
    assert_eq!(live.sample().unwrap().t_ps, 32_000_000_000);
}

#[test]
fn a_step_stops_play() {
    let mut live = opened();
    assert!(live.run(&Command::Play).ok());
    let stepped = live.run(&command("step 5ms"));
    assert_eq!(
        (stepped.t_ps, stepped.state),
        (5_000_000_000, State::Paused)
    );
    assert!(live.tick(TICK).is_none());
}

#[test]
fn a_refused_step_leaves_play_running() {
    let mut live = opened();
    assert!(live.run(&Command::Play).ok());
    assert!(live.tick(TICK).is_some());
    for (line, code) in [
        ("step 10000s", Code::PastStop),
        ("step --until @nope off", Code::UnknownRef),
    ] {
        let refused = live.run(&command(line));
        assert_eq!(refused.error.unwrap().code, code, "{line}");
        assert_eq!(
            (refused.t_ps, refused.state),
            (16_000_000_000, State::Playing),
            "{line} takes no time and leaves play running"
        );
    }
    assert!(live.tick(TICK).is_some());
}

#[test]
fn the_next_step_lists_what_play_ran() {
    let mut live = opened();
    assert!(live.run(&Command::Play).ok());
    assert!(live.run(&command("press @btn1")).ok());
    for _ in 0..4 {
        assert!(live.tick(TICK).is_some());
    }
    let paused = live.run(&Command::Pause);
    assert_eq!(paused.text(), "paused at t=64ms");
    let stepped = live.run(&command("step 1ms"));
    assert!(stepped.ok(), "{}", stepped.text());
    let text = stepped.text();
    assert!(text.starts_with("t=65ms (from 0s)\n"), "{text}");
    assert!(text.contains(" nano.D13 high"), "{text}");
    assert!(text.contains(" led1 lit"), "{text}");
    let again = live.run(&command("step 1ms"));
    let again_text = again.text();
    let changes: Vec<_> = again_text
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("  brightness "))
        .collect();
    assert!(
        changes.is_empty(),
        "each change is listed once: {again_text}"
    );
    assert!(
        again_text.starts_with("t=66ms (from 65ms)\n"),
        "{again_text}"
    );
}

#[test]
fn a_step_refused_at_the_stop_lists_what_play_ran() {
    let mut live = Live::new("t", Files::disk());
    live.stop_at(Some((Duration::from_ms(40), "ORBE_TEST_STOP".into())));
    assert!(live.run(&command("open")).ok());
    assert!(live.run(&Command::Play).ok());
    assert!(live.run(&command("press @btn1")).ok());
    while live.tick(TICK).is_some() {}
    let ended = live.run(&command("step 1ms"));
    assert_eq!(ended.error.as_ref().unwrap().code, Code::Ended);
    let text = ended.text();
    assert!(text.starts_with("t=40ms (from 0s)\n"), "{text}");
    assert!(text.contains(" led1 lit"), "{text}");
    let again = live.run(&command("step 1ms"));
    assert_eq!(
        again.text(),
        "error[ended]: the session reached its 40ms stop\n  hint: run `open` to start again"
    );
}

#[test]
fn play_ends_at_the_stop() {
    let mut live = Live::new("t", Files::disk());
    live.stop_at(Some((Duration::from_ms(40), "ORBE_TEST_STOP".into())));
    assert!(live.run(&command("open")).ok());
    assert!(live.run(&Command::Play).ok());
    let mut ticks = 0;
    while live.tick(TICK).is_some() {
        ticks += 1;
    }
    assert_eq!(ticks, 2, "16 ms, 32 ms, then the stop at 40 ms ends play");
    let sample = live.sample().unwrap();
    assert_eq!(
        (sample.t_ps, sample.state),
        (40_000_000_000, FrameState::Stopped)
    );
    let again = live.run(&Command::Play);
    assert_eq!(again.error.unwrap().code, Code::Ended);
}

#[test]
fn a_test_file_refuses_play() {
    let file = first_target().join("tests/play.test");
    let report = orbe_session::test::run(Files::disk(), &file, "open\nplay\n", &first_target());
    let failed = report.failure().unwrap();
    assert_eq!(failed.text, "play");
    let error = failed.reply.error.as_ref().unwrap();
    assert_eq!(error.code, Code::Usage);
    assert_eq!(error.hint, "advance time with `step`, such as `step 100ms`");
}

#[test]
fn a_joining_viewer_gets_the_scene_then_the_present() {
    let live = opened();
    let mut feed = Feed::default();
    let joined = feed.join(&live);
    let types: Vec<&str> = joined.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["scene", "frame"]);
    assert_eq!(joined[1]["seq"], 0);
    assert!(feed.now(&live, None).is_none(), "nothing new since");
    let again = feed.join(&live);
    assert_eq!(
        again.len(),
        2,
        "a second viewer gets the scene and a frame too"
    );
    assert_eq!(again[1]["seq"], 1);
}

#[test]
fn a_viewer_s_command_is_answered_by_its_id_and_followed_by_a_frame() {
    let mut live = opened();
    let mut feed = Feed::default();
    feed.join(&live);
    let (reply, message) = viewer::answer(&mut live, r#"{"id": 3, "cmd": "play"}"#);
    assert_eq!(reply.state, State::Playing);
    assert_eq!(message["type"], "reply");
    assert_eq!(message["id"], 3);
    assert_eq!(message["reply"], reply.json());
    let after = feed.after(&live, &reply);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["state"], "playing");

    let (refused, message) = viewer::answer(&mut live, r#"{"id": 4, "cmd": "step"}"#);
    assert_eq!(refused.error.unwrap().code, Code::Usage);
    assert_eq!(message["reply"]["ok"], false);
    assert_eq!(message["id"], 4);
}

#[test]
fn an_open_restarts_the_feed_with_its_scene() {
    let mut live = opened();
    let mut feed = Feed::default();
    feed.join(&live);
    let reopened = live.run(&command("open"));
    let after = feed.after(&live, &reopened);
    let types: Vec<&str> = after.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["scene", "frame"]);
}

#[test]
fn a_tick_runs_at_most_one_tick_of_simulated_time_and_says_its_pace() {
    let mut live = opened();
    assert!(live.run(&Command::Play).ok());
    let pace = live.tick(TICK * 2).unwrap();
    assert_eq!(live.sample().unwrap().t_ps, 16_000_000_000);
    assert_eq!((pace.speed, pace.ratio), (1.0, 0.5));
    let pace = live.tick(TICK / 2).unwrap();
    assert_eq!(live.sample().unwrap().t_ps, 24_000_000_000);
    assert_eq!(pace.ratio, 1.0);
    let mut feed = Feed::default();
    let frame = feed.now(&live, Some(pace)).unwrap();
    assert_eq!(
        (frame["speed"].as_f64(), frame["ratio"].as_f64()),
        (Some(1.0), Some(1.0))
    );
}

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
    walk(&first_target().join("../../parts"), "", &mut files);
    files
}

/// A Nano driving a 1e300 F capacitor from D13 faults as it opens. Its frame says `faulted` and
/// carries the fault as `snapshot` prints it, and a session that runs has no fault in its frame.
#[test]
fn a_faulted_session_s_frame_carries_the_fault() {
    let own = |path: &str| {
        (
            path.to_string(),
            std::fs::read(first_target().join(path)).unwrap(),
        )
    };
    let scene = r#"{
      "format": 1,
      "parts": {
        "nano": { "type": "nano-board", "firmware": "firmware/button.hex" },
        "c1": { "type": "capacitor-ceramic-disc", "capacitance": "1e300" },
      },
      "wires": [["c1.a", "nano.D13"], ["c1.b", "nano.GND"]],
    }"#;
    let files = Files::memory([
        own("block.jsonc"),
        own("firmware/button.hex"),
        ("scene.jsonc".to_string(), scene.as_bytes().to_vec()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let open = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(open.ok(), "{}", open.text());
    let snapshot = live.run(&command("snapshot")).text();
    let frame = Feed::default().now(&live, None).unwrap();
    let fault = frame["fault"].as_str().expect("a fault in the frame");
    assert_eq!(frame["state"], "faulted");
    assert!(fault.starts_with("error["), "{fault}");
    assert!(
        snapshot.lines().any(|line| line == fault),
        "{fault}\nis not a line of\n{snapshot}"
    );

    let running = Feed::default().now(&opened(), None).unwrap();
    assert_eq!(running["state"], "paused");
    assert!(running.get("fault").is_none(), "{running}");
}

/// A frame taken inside a step reads `stepping`, but the slice that ends the session reports the
/// state it left, so a viewer never shows a stopped or faulted session as still stepping.
#[test]
fn a_step_s_last_frame_reports_the_stop_it_reached() {
    let mut live = Live::new("t", Files::disk());
    live.stop_at(Some((Duration::from_ms(40), "ORBE_TEST_STOP".into())));
    assert!(live.run(&command("open")).ok());
    let mut states = Vec::new();
    let reply = live.run_with(&command("step 40ms"), &mut |progress| {
        states.push(progress.sample().state);
        ControlFlow::Continue(())
    });
    assert!(reply.ok(), "{}", reply.text());
    assert!(states.len() > 1, "{states:?}");
    assert!(
        states[..states.len() - 1]
            .iter()
            .all(|state| *state == FrameState::Stepping),
        "{states:?}"
    );
    assert_eq!(states.last(), Some(&FrameState::Stopped), "{states:?}");
}
