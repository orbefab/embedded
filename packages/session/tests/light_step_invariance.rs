//! Light readings (LED brightness, LED state changes, 7-segment digits) stay identical
//! however time is sliced across `step` commands or `play` ticks.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;

use orbe_session::{BRIGHTNESS_WINDOW, Change, Command, Data, Files, Light, Live, Reading, Reply};

fn block_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../blocks")
        .join(name)
}

fn parse(dir: &Path, line: &str) -> Command {
    Command::parse(line, dir).unwrap().unwrap()
}

#[derive(Debug, PartialEq)]
struct LedEvent {
    by_ps: u64,
    part: String,
    state: Light,
    brightness: f64,
    over_ps: u64,
}

fn collect_led_changes(reply: &Reply, out: &mut Vec<LedEvent>) {
    let changes = match &reply.data {
        Data::Stepped(stepped) => stepped.changes.as_slice(),
        Data::Paced {
            played: Some(stepped),
            ..
        } => stepped.changes.as_slice(),
        _ => return,
    };
    for change in changes {
        if let Change::Led {
            by_ps,
            part,
            state,
            brightness,
            over_ps,
        } = change
        {
            out.push(LedEvent {
                by_ps: *by_ps,
                part: part.clone(),
                state: *state,
                brightness: *brightness,
                over_ps: *over_ps,
            });
        }
    }
}

fn brightness(live: &mut Live, dir: &Path, target: &str) -> (f64, u64) {
    let reply = live.run(&parse(dir, &format!("get {target}.brightness")));
    assert!(reply.ok(), "{}", reply.text());
    let Data::Got(got) = reply.data else {
        panic!("{}", reply.text());
    };
    let Reading::Brightness { value, over_ps } = got.value else {
        panic!("{:?}", got.value);
    };
    (value, over_ps)
}

fn current(live: &mut Live, dir: &Path, target: &str) -> f64 {
    let reply = live.run(&parse(dir, &format!("get {target}.i")));
    assert!(reply.ok(), "{}", reply.text());
    let Data::Got(got) = reply.data else {
        panic!("{}", reply.text());
    };
    let Reading::Quantity { value, .. } = got.value else {
        panic!("{:?}", got.value);
    };
    value
}

fn digit(live: &mut Live, dir: &Path, target: &str) -> String {
    let reply = live.run(&parse(dir, &format!("get {target}.digit")));
    assert!(reply.ok(), "{}", reply.text());
    let Data::Got(got) = reply.data else {
        panic!("{}", reply.text());
    };
    let Reading::Shown(shown) = got.value else {
        panic!("{:?}", got.value);
    };
    shown
}

fn run_fade(steps: &[&str]) -> (f64, (f64, u64), Vec<LedEvent>) {
    let dir = block_dir("nano-led-fade");
    let mut live = Live::new("fade", Files::disk());
    assert!(live.run(&parse(&dir, "open")).ok());
    let mut events = Vec::new();
    for &step in steps {
        let reply = live.run(&parse(&dir, &format!("step {step}")));
        assert!(reply.ok(), "{step}: {}", reply.text());
        collect_led_changes(&reply, &mut events);
    }
    let led1_events: Vec<LedEvent> = events
        .into_iter()
        .filter(|event| event.part == "led1")
        .collect();
    (
        current(&mut live, &dir, "@led1"),
        brightness(&mut live, &dir, "@led1"),
        led1_events,
    )
}

#[test]
fn nano_led_fade_reports_identical_brightness_and_led_changes_however_stepped() {
    let baseline = run_fade(&["200ms"]);
    assert_eq!(baseline.1.1, BRIGHTNESS_WINDOW.as_ps());
    assert_eq!(baseline.2.len(), 1, "{:?}", baseline.2);
    assert_eq!(baseline.2[0].by_ps, 109_000_000_000);
    assert_eq!(baseline.2[0].state, Light::Dim);

    let one_ms: Vec<&str> = vec!["1ms"; 200];
    assert_eq!(run_fade(&one_ms), baseline, "step 1ms x 200");

    let mut uneven: Vec<&str> = vec!["1.7ms"; 117];
    uneven.push("1.1ms");
    assert_eq!(run_fade(&uneven), baseline, "step 1.7ms x 117 + 1.1ms");

    // led1 crosses into dim at 107 ms and finishes its 2 ms hold at 109 ms;
    // ending a step at 108 ms splits the hold across steps.
    assert_eq!(
        run_fade(&["108ms", "92ms"]),
        baseline,
        "step ending mid-hold at 108ms"
    );
    assert_eq!(
        run_fade(&["107ms", "2ms", "91ms"]),
        baseline,
        "step starting on hold start at 107ms"
    );

    // `play` ticks with uneven wall times (the viewer / wasm path) match too.
    let dir = block_dir("nano-led-fade");
    let mut live = Live::new("fade-play", Files::disk());
    assert!(live.run(&parse(&dir, "open")).ok());
    assert!(live.run(&Command::Play).ok());
    let mut elapsed_us = 0u64;
    let pattern = [16_000u64, 7_300, 1_100, 4_600, 900];
    let mut idx = 0usize;
    while elapsed_us < 200_000 {
        let next = pattern[idx % pattern.len()].min(200_000 - elapsed_us);
        idx += 1;
        assert!(live.tick(StdDuration::from_micros(next)).is_some());
        elapsed_us += next;
    }
    let paused = live.run(&Command::Pause);
    assert!(paused.ok(), "{}", paused.text());
    let mut play_events = Vec::new();
    collect_led_changes(&paused, &mut play_events);
    let led1_play: Vec<LedEvent> = play_events
        .into_iter()
        .filter(|event| event.part == "led1")
        .collect();
    assert_eq!(
        (
            current(&mut live, &dir, "@led1"),
            brightness(&mut live, &dir, "@led1"),
            led1_play,
        ),
        baseline,
        "play with uneven ticks"
    );
}

#[test]
fn off_grid_brightness_window_is_exact_and_step_independent() {
    let one = run_fade(&["198.9ms"]);
    assert_eq!(one.1.1, BRIGHTNESS_WINDOW.as_ps());

    let mut ms_plus = vec!["1ms"; 198];
    ms_plus.push("0.9ms");
    assert_eq!(run_fade(&ms_plus), one, "step 1ms x 198 + 0.9ms");

    let uneven = vec!["1.7ms"; 117];
    assert_eq!(run_fade(&uneven), one, "step 1.7ms x 117");
}

const SEGMENTS: [&str; 7] = [
    "@disp.seg_a",
    "@disp.seg_b",
    "@disp.seg_c",
    "@disp.seg_d",
    "@disp.seg_e",
    "@disp.seg_f",
    "@disp.seg_g",
];

fn step_to_and_read_7seg(steps: &[&str]) -> (String, Vec<(f64, u64)>, Vec<LedEvent>) {
    let dir = block_dir("nano-7seg-count");
    let mut live = Live::new("seg", Files::disk());
    assert!(live.run(&parse(&dir, "open")).ok());
    let mut events = Vec::new();
    for &step in steps {
        let reply = live.run(&parse(&dir, &format!("step {step}")));
        assert!(reply.ok(), "{step}: {}", reply.text());
        collect_led_changes(&reply, &mut events);
    }
    let seg_events: Vec<LedEvent> = events
        .into_iter()
        .filter(|event| event.part.starts_with("disp."))
        .collect();
    let seg_brightness: Vec<(f64, u64)> = SEGMENTS
        .iter()
        .map(|seg| brightness(&mut live, &dir, seg))
        .collect();
    (digit(&mut live, &dir, "@disp"), seg_brightness, seg_events)
}

#[test]
fn nano_7seg_count_reports_identical_digits_brightness_and_led_changes_however_stepped() {
    // 100 ms captures digit 0 startup (all 6 segments cross dim at 69 ms and lit at 73 ms,
    // fitting inside a single step's change list cap).
    let zero = step_to_and_read_7seg(&["100ms"]);
    assert_eq!(zero.0, "0");
    assert!(
        zero.1
            .iter()
            .all(|&(_, over)| over == BRIGHTNESS_WINDOW.as_ps())
    );
    assert_eq!(zero.2.len(), 12, "{:?}", zero.2);

    assert_eq!(
        step_to_and_read_7seg(&vec!["1ms"; 100]),
        zero,
        "step 1ms x 100"
    );
    let mut uneven_100 = vec!["1.7ms"; 58];
    uneven_100.push("1.4ms");
    assert_eq!(
        step_to_and_read_7seg(&uneven_100),
        zero,
        "step 1.7ms x 58 + 1.4ms"
    );
    // Split mid-hold on both the dim hold (67 -> 69 ms) and lit hold (71 -> 73 ms).
    assert_eq!(
        step_to_and_read_7seg(&["68ms", "4ms", "28ms"]),
        zero,
        "steps ending mid-hold at 68ms and 72ms"
    );

    // 467 ms is just after the 1 -> 2 digit transition at ~466.15 ms.
    let two = step_to_and_read_7seg(&["467ms"]);
    assert_eq!(two.0, "2");
    assert!(
        two.1
            .iter()
            .all(|&(_, over)| over == BRIGHTNESS_WINDOW.as_ps())
    );

    let one_ms_467 = step_to_and_read_7seg(&vec!["1ms"; 467]);
    assert_eq!((one_ms_467.0, one_ms_467.1), (two.0.clone(), two.1.clone()));

    let mut uneven_467 = vec!["1.7ms"; 274];
    uneven_467.push("1.2ms");
    let uneven_467_res = step_to_and_read_7seg(&uneven_467);
    assert_eq!(
        (uneven_467_res.0, uneven_467_res.1),
        (two.0, two.1),
        "step 1.7ms x 274 + 1.2ms"
    );

    // Off-grid read at 466.5 ms (right after the 1 -> 2 transition) agrees across steppings.
    let off_grid = step_to_and_read_7seg(&["466.5ms"]);
    assert_eq!(off_grid.0, "2");
    assert!(
        off_grid
            .1
            .iter()
            .all(|&(_, over)| over == BRIGHTNESS_WINDOW.as_ps())
    );

    let mut off_uneven = vec!["1.7ms"; 274];
    off_uneven.push("0.7ms");
    let off_uneven_res = step_to_and_read_7seg(&off_uneven);
    assert_eq!(
        (off_uneven_res.0, off_uneven_res.1),
        (off_grid.0, off_grid.1),
        "step 1.7ms x 274 + 0.7ms"
    );
}

#[test]
fn cancelling_a_step_mid_hold_preserves_the_pending_led_change() {
    let dir = block_dir("nano-button-led");
    let open_and_press = || {
        let mut live = Live::new("btn", Files::disk());
        assert!(live.run(&parse(&dir, "open")).ok());
        assert!(live.run(&parse(&dir, "step 100ms")).ok());
        assert!(live.run(&parse(&dir, "press @btn1")).ok());
        live
    };

    let mut whole = open_and_press();
    let whole_reply = whole.run(&parse(&dir, "step 5ms"));
    assert!(whole_reply.ok(), "{}", whole_reply.text());
    let mut whole_events = Vec::new();
    collect_led_changes(&whole_reply, &mut whole_events);
    let whole_led1: Vec<LedEvent> = whole_events
        .into_iter()
        .filter(|e| e.part == "led1")
        .collect();
    assert!(!whole_led1.is_empty(), "whole step emits led1 dim");

    // Cancel at 102 ms, right in the middle of the 2 ms hold (101 ms -> 103 ms).
    let mut split = open_and_press();
    let mut split_events = Vec::new();
    let part1 = split.run_with(&parse(&dir, "step 5ms"), &mut |progress| {
        if progress.t_ps() == 102_000_000_000 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    collect_led_changes(&part1, &mut split_events);
    let part2 = split.run(&parse(&dir, "step 3ms"));
    assert!(part2.ok(), "{}", part2.text());
    collect_led_changes(&part2, &mut split_events);
    let split_led1: Vec<LedEvent> = split_events
        .into_iter()
        .filter(|e| e.part == "led1")
        .collect();

    assert_eq!(split_led1, whole_led1);
    assert_eq!(
        brightness(&mut split, &dir, "@led1"),
        brightness(&mut whole, &dir, "@led1")
    );
}
