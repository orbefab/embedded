//! What `orbe test` says about time: when an edge, a byte, an LED change or a diagnostic
//! happened, what a read right after an input shows, and serial as the firmware sent it.

mod live;

use std::fs;

use live::{change, common, edit_scene, orbe, project, replies, replies_in, with_firmware};
use serde_json::Value;

const PRESS_THEN_D13: &str =
    "open\nstep 100ms\npress @btn1\nstep --until @nano.D13 high --timeout 50ms\n";

fn met_at(reply: &Value) -> u64 {
    reply["data"]["met_at_ps"]
        .as_u64()
        .unwrap_or_else(|| panic!("not met: {reply}"))
}

#[test]
fn a_pulse_inside_one_slice_meets_until_at_its_edge() {
    let dir = with_firmware(
        "pulse",
        "open\nstep --until @nano.D13 high --timeout 100ms\n",
    );
    let json = replies_in(&dir);
    let step = &json[1];
    assert_eq!(step["ok"], true, "{step}");
    let edge = change(step, "edge", "@nano.D13");
    assert_eq!(edge["level"], "high");
    assert_eq!(met_at(step), edge["at_ps"].as_u64().unwrap());
    assert_eq!(met_at(step), 10_019_000_000);
    assert_eq!(step["t_ps"], 11_000_000_000u64, "the slice the edge is in");

    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(
        run.stdout
            .contains("  t=11ms (from 0s), @nano.D13 high at 10.019ms\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_pin_condition_meets_at_the_edge_on_its_net_whatever_the_part_order_or_label() {
    let first = replies(PRESS_THEN_D13);
    let edge_at = change(&first[3], "edge", "@nano.D13")["at_ps"]
        .as_u64()
        .unwrap();
    assert_eq!(met_at(&first[3]), edge_at);

    let dir = project(PRESS_THEN_D13);
    edit_scene(
        &dir,
        "    \"nano\": { \"type\": \"nano-board\", \"firmware\": \"build/button.ino.hex\" },\n",
        "",
    );
    edit_scene(
        &dir,
        "\"color\": \"red\" },",
        "\"color\": \"red\" },\n    \"nano\": { \"type\": \"nano-board\", \"firmware\": \"build/button.ino.hex\" },",
    );
    let reordered = replies_in(&dir);
    assert_eq!(reordered[0]["data"]["parts"][4], "nano");
    assert_eq!(met_at(&reordered[3]), edge_at, "{}", reordered[3]);
    assert_eq!(
        change(&reordered[3], "edge", "@nano.D13")["at_ps"],
        edge_at,
        "an edge keeps its chip pin's name"
    );
    change(&reordered[3], "edge", "@nano.D2");

    let other_label =
        replies("open\nstep 100ms\npress @btn1\nstep --until @r1.a high --timeout 50ms\n");
    assert_eq!(met_at(&other_label[3]), edge_at, "@r1.a shares D13's net");
}

#[test]
fn two_chip_pins_on_one_net_keep_their_own_names() {
    let dir = project("open\nstep 30ms\npress @btn1\nstep 5ms\n");
    edit_scene(
        &dir,
        "[\"nano.D13\", \"r1.a\"],",
        "[\"nano.D13\", \"r1.a\"],\n    [\"nano.D13\", \"nano.D3\"],",
    );
    let json = replies_in(&dir);
    let changes = json[3]["data"]["changes"].as_array().unwrap();
    let edges: Vec<(u64, &str)> = changes
        .iter()
        .filter(|c| c["kind"] == "edge")
        .map(|c| (c["at_ps"].as_u64().unwrap(), c["ref"].as_str().unwrap()))
        .collect();
    let mut unique = edges.clone();
    unique.dedup();
    assert_eq!(unique, edges, "no edge is named twice: {}", json[3]);
}

#[test]
fn a_serial_match_meets_at_the_start_bit_of_the_byte_that_completed_it() {
    let dir = project("open\nstep --until serial contains \"0\" --timeout 350us\n");
    fs::copy(
        common::repo().join("engine/firmware/sketch/serial_counter.hex"),
        dir.path().join("build/button.ino.hex"),
    )
    .unwrap();
    let json = replies_in(&dir);
    let step = &json[1];
    let byte = step["data"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "serial")
        .unwrap_or_else(|| panic!("{step}"));
    assert_eq!(met_at(step), byte["at_ps"].as_u64().unwrap());
    assert_eq!(met_at(step), 138_500_000);
}

#[test]
fn a_failed_step_shows_the_changes_it_saw() {
    let dir = project("open\nstep 10ms\npress @btn1\nstep --until @led1.i > 1A --timeout 5ms\n");
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(
        run.stdout.contains("error[until_timeout]"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("\n    10ms nano.D2 high\n"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("ms nano.D13 high\n"), "{}", run.stdout);

    let run = orbe(dir.path(), &["test", "--json"]);
    let entry = &run.json()["data"]["tests"][0];
    assert_eq!(entry["data"]["changes"][0]["ref"], "@nano.D2", "{entry}");
}

#[test]
fn an_led_change_is_labeled_as_sampled_at_the_slice_end() {
    let json = replies("open\nstep 10ms\npress @btn1\nstep 5ms\n");
    let led = change(&json[3], "led", "@led1");
    assert_eq!(led["by_ps"], 11_000_000_000u64, "{led}");
    assert!(led["at_ps"].is_null(), "{led}");

    let dir = project("open\nstep 10ms\npress @btn1\nstep 5ms\n");
    let run = orbe(dir.path(), &["test", "-v"]);
    assert!(
        run.stdout.contains("\n    by 11ms led1 on\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_read_right_after_an_input_agrees_with_itself() {
    let test = "open\nstep 1ms\npress @btn1\nget @nano.D2\nstep 1ms\nget @nano.D2\nrelease @btn1\nget @nano.D2\n";
    let json = replies(test);
    let read = |n: usize| {
        let value = &json[n]["data"]["value"];
        (
            value["level"].as_str().unwrap().to_string(),
            value["v"].as_f64().unwrap(),
        )
    };
    let (level, v) = read(3);
    assert!(level == "low" && v < 2.5, "at the press: {level} {v} V");
    let (level, v) = read(5);
    assert!(level == "high" && v > 2.5, "a step later: {level} {v} V");
    let (level, v) = read(7);
    assert!(level == "high" && v > 2.5, "at the release: {level} {v} V");

    let dir = project(test);
    let run = orbe(dir.path(), &["test", "-v"]);
    assert!(
        run.stdout
            .contains("\n  pressed btn1 at t=1ms; reads show it after t=1ms\n"),
        "{}",
        run.stdout
    );
}

fn hot(test: &str) -> tempfile::TempDir {
    let dir = project(test);
    edit_scene(&dir, "\"resistance\": \"220\"", "\"resistance\": \"10\"");
    dir
}

#[test]
fn no_diagnostics_fails_while_a_part_is_over_its_rating_now() {
    let dir = hot(
        "open\nstep 100ms\npress @btn1\nstep 9ms\nexpect @led1.i > 70mA\nexpect no diagnostics\n",
    );
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    let entry = &run.json()["data"]["tests"][0];
    assert_eq!(entry["line"], 6, "{entry}");
    assert!(
        entry["actual"].as_str().unwrap().contains("rating.current"),
        "{entry}"
    );
}

#[test]
fn a_step_reports_the_diagnostics_that_arose_during_it() {
    let dir = hot("open\nstep 100ms\npress @btn1\nstep 100ms\n");
    let json = replies_in(&dir);
    let codes: Vec<&str> = json[3]["data"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "diagnostic")
        .map(|c| c["diagnostic"]["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"rating.current"), "{}", json[3]);
    assert!(json[1]["data"]["changes"].as_array().unwrap().is_empty());

    let run = orbe(dir.path(), &["test", "-v"]);
    assert!(
        run.stdout.contains("warning[rating.current] #led1.i"),
        "{}",
        run.stdout
    );
}

#[test]
fn serial_contains_searches_everything_since_open() {
    let dir = with_firmware(
        "flood",
        "open\nstep 100ms\nexpect serial contains \"ready\"\nstep 7s\nexpect serial contains \"ready\"\n",
    );
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 0, "{}", &run.stdout[..run.stdout.len().min(2000)]);
}

#[test]
fn a_failed_serial_match_shows_a_short_actual() {
    let dir = with_firmware("flood", "open\nstep 1s\nexpect serial contains \"nope\"\n");
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 1, "{}", &run.stdout[..run.stdout.len().min(2000)]);
    let actual = run.json()["data"]["tests"][0]["actual"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(actual.len() < 400, "{actual}");
    assert!(actual.contains(" bytes"), "{actual}");
    assert!(
        actual.contains("0123456789\\r\\n"),
        "it ends with the latest output: {actual}"
    );
}

#[test]
fn serial_is_decoded_as_utf8() {
    let dir = with_firmware(
        "utf8",
        "open\nstep 200ms\nexpect serial contains \"café ok\"\nsnapshot\n",
    );
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(
        run.stdout.contains("\n  serial  \"café ok\"\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn every_number_a_reply_prints_reads_back_in_a_condition() {
    let dir = project("open\nget @led1.i\n");
    let run = orbe(dir.path(), &["test", "-v"]);
    let printed = run
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("  led1.i "))
        .and_then(|l| l.strip_suffix(" A"))
        .unwrap_or_else(|| panic!("{}", run.stdout))
        .to_string();
    assert!(printed.contains('e'), "a small current prints as {printed}");
    let dir = project(&format!(
        "open\nexpect @led1.i <= {printed}A\nexpect @led1.i >= {printed}A\n"
    ));
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
}
