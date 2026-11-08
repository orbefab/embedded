//! What a viewer gets from a session: the scene message, frames that never change a result, and
//! `play`'s ticks.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use orbe_session::{Code, Command, Duration, Files, FrameState, Frames, Live, Reply, State};
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
    "step --until @led1 on --timeout 50ms",
    "expect @nano.D13 high",
    "release @btn1",
    "step 50ms",
    "expect @led1 off",
];

#[test]
fn frames_read_after_every_slice_change_no_reply() {
    let mut watched = opened();
    let mut frames = Frames::default();
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
    let mut frames = Frames::default();
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
    assert_eq!(scene["instances"][1]["name"], "btn1");
}

#[test]
fn a_frame_shows_a_press_after_it_lands_and_skips_a_repeat() {
    let mut live = opened();
    let mut frames = Frames::default();
    let first = frames.frame(live.sample().unwrap(), None).unwrap();
    assert_eq!(first["switches"], json!([0, 0]));
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

#[test]
fn play_ticks_until_pause() {
    let mut live = opened();
    let played = live.run(&Command::Play);
    assert!(played.ok(), "{}", played.text());
    assert_eq!(played.state, State::Playing);
    assert_eq!(played.text(), "playing from t=0s");
    assert!(live.tick(16_000_000_000));
    assert!(live.tick(16_000_000_000));
    let paused = live.run(&Command::Pause);
    assert_eq!((paused.t_ps, paused.state), (32_000_000_000, State::Paused));
    assert_eq!(paused.text(), "paused at t=32ms");
    assert!(!live.tick(16_000_000_000), "a paused session does not tick");
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
    assert!(!live.tick(16_000_000_000));
}

#[test]
fn a_refused_step_leaves_play_running() {
    let mut live = opened();
    assert!(live.run(&Command::Play).ok());
    assert!(live.tick(16_000_000_000));
    for (line, code) in [
        ("step 10000s", Code::PastStop),
        ("step --until @nope on", Code::UnknownRef),
    ] {
        let refused = live.run(&command(line));
        assert_eq!(refused.error.unwrap().code, code, "{line}");
        assert_eq!(
            (refused.t_ps, refused.state),
            (16_000_000_000, State::Playing),
            "{line} takes no time and leaves play running"
        );
    }
    assert!(live.tick(16_000_000_000));
}

#[test]
fn the_next_step_lists_what_play_ran() {
    let mut live = opened();
    assert!(live.run(&Command::Play).ok());
    assert!(live.run(&command("press @btn1")).ok());
    for _ in 0..4 {
        assert!(live.tick(16_000_000_000));
    }
    let paused = live.run(&Command::Pause);
    assert_eq!(paused.text(), "paused at t=64ms");
    let stepped = live.run(&command("step 1ms"));
    assert!(stepped.ok(), "{}", stepped.text());
    let text = stepped.text();
    assert!(text.starts_with("t=65ms (from 0s)\n"), "{text}");
    assert!(text.contains(" nano.D13 high"), "{text}");
    assert!(text.contains(" led1 on"), "{text}");
    let again = live.run(&command("step 1ms"));
    assert_eq!(
        again.text(),
        "t=66ms (from 65ms)",
        "each change is listed once"
    );
}

#[test]
fn a_step_refused_at_the_stop_lists_what_play_ran() {
    let mut live = Live::new("t", Files::disk());
    live.stop_at(Some((Duration::from_ms(40), "ORBE_TEST_STOP".into())));
    assert!(live.run(&command("open")).ok());
    assert!(live.run(&Command::Play).ok());
    assert!(live.run(&command("press @btn1")).ok());
    while live.tick(16_000_000_000) {}
    let ended = live.run(&command("step 1ms"));
    assert_eq!(ended.error.as_ref().unwrap().code, Code::Ended);
    let text = ended.text();
    assert!(text.starts_with("t=40ms (from 0s)\n"), "{text}");
    assert!(text.contains(" led1 on"), "{text}");
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
    while live.tick(16_000_000_000) {
        ticks += 1;
    }
    assert_eq!(ticks, 2, "16 ms, 32 ms, then the stop at 40 ms ends play");
    let sample = live.sample().unwrap();
    assert_eq!(
        (sample.t_ps, sample.state),
        (40_000_000_000, FrameState::Session(State::Stopped))
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
