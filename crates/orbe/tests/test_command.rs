//! `orbe test` on the first target: each live command's reply as text and as JSON, each error
//! code, and how a failing test reports.

#[path = "../../orbe-scene/tests/common/mod.rs"]
mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {}", self.stdout))
    }
}

fn orbe(dir: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// The first-target project with one test file, `tests/a.test`.
fn project(test: &str) -> TempDir {
    let dir = common::project("nano-button");
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    fs::write(dir.path().join("tests/a.test"), test).unwrap();
    dir
}

/// Each reply of a passing `--json -v` run of `tests/a.test`.
fn replies(test: &str) -> Vec<Value> {
    let dir = project(test);
    let run = orbe(dir.path(), &["test", "--json", "-v"]);
    let json = run.json();
    json["data"]["tests"][0]["replies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reply"].clone())
        .collect()
}

/// The error a one-test run stopped on, from `--json`.
fn failed(test: &str) -> (Value, Value) {
    let dir = project(test);
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    let entry = run.json()["data"]["tests"][0].clone();
    (entry["error"].clone(), entry)
}

#[test]
fn the_first_target_example_passes() {
    let run = orbe(&common::repo(), &["test", "examples/first-target"]);
    assert_eq!(
        run.stdout,
        "PASS examples/first-target/tests/button.test\n1 passed, 0 failed\n"
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
}

#[test]
fn the_first_target_reads_the_engine_values_while_pressed() {
    let dir =
        project("open\nstep 50ms\npress @btn1\nstep 50ms\nget @nano.D13\nget @led1\nget @led1.i\n");
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(
        run.stdout
            .contains("\n  nano.D13 high 4.621848245658706 V\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("\n  led1 on i=0.012246558123229083 A v="),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("\n  led1.i 0.012246558123229083 A\n"),
        "{}",
        run.stdout
    );

    let json =
        replies("open\nstep 50ms\npress @btn1\nstep 50ms\nget @nano.D13\nget @led1\nget @led1.i\n");
    assert_eq!(json[4]["data"]["ref"], "@nano.D13");
    assert_eq!(json[4]["data"]["kind"], "pin");
    assert_eq!(json[4]["data"]["value"]["level"], "high");
    assert_eq!(json[4]["data"]["value"]["v"], 4.621848245658706);
    assert_eq!(json[5]["data"]["kind"], "part");
    assert_eq!(json[5]["data"]["value"]["on"], true);
    assert_eq!(json[5]["data"]["value"]["i"], 0.012246558123229083);
    assert_eq!(json[6]["data"]["kind"], "quantity");
    assert_eq!(json[6]["data"]["value"], 0.012246558123229083);
    assert_eq!(json[6]["t_ps"], 100_000_000_000u64);
}

const TRANSCRIPT: &str = "\
# every live command once
open
step 100ms
expect @led1 off
press @btn1
step --until @led1 on --timeout 50ms
expect @nano.D13 high
expect @led1.i > 5mA
snapshot
release @btn1
step 50ms
expect no diagnostics
close
";

#[test]
fn each_command_replies_in_text() {
    let dir = project(TRANSCRIPT);
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let blocks: Vec<&str> = run.stdout.split("tests/a.test:").skip(1).collect();
    let block = |n: usize| blocks[n].lines().collect::<Vec<_>>();

    assert_eq!(block(0)[0], "2> open");
    assert_eq!(
        block(0)[1],
        "  opened scene.jsonc at t=0: nano, btn1, r2, r1, led1 (18 engine parts, 31 nets)"
    );
    assert_eq!(
        block(0)[2],
        "    warning[net.floating] #net.nano__AREF has no DC path to ground"
    );
    assert_eq!(block(1), ["3> step 100ms", "  t=100ms (from 0s)"]);
    assert_eq!(block(2)[0], "4> expect @led1 off");
    assert!(
        block(2)[1].starts_with("  passed @led1 off (led1 off, i="),
        "{}",
        blocks[2]
    );
    assert_eq!(block(3), ["5> press @btn1", "  pressed btn1 at t=100ms"]);

    let until = block(4);
    assert_eq!(until[0], "6> step --until @led1 on --timeout 50ms");
    assert_eq!(until[1], "  t=101ms (from 100ms), @led1 on at 101ms");
    assert_eq!(until[2], "    100ms nano.D2 high");
    assert!(
        until[3].starts_with("    100.") && until[3].ends_with("ms nano.D13 high"),
        "{}",
        until[3]
    );
    assert_eq!(until[4], "    101ms led1 on");

    assert_eq!(
        block(5),
        [
            "7> expect @nano.D13 high",
            "  passed @nano.D13 high (nano.D13 high)"
        ]
    );
    assert_eq!(
        block(6),
        [
            "8> expect @led1.i > 5mA",
            "  passed @led1.i > 5mA (led1.i = 0.012246558123229083 A)"
        ]
    );

    let snapshot = block(7);
    assert_eq!(snapshot[0], "9> snapshot");
    assert_eq!(snapshot[1], "  t=101ms  scene=scene.jsonc");
    assert_eq!(
        snapshot[2],
        "  nano  nano-board          5V=high  D13=high  D2=high  GND=0.0V"
    );
    assert!(
        snapshot[3].starts_with("  btn1  tactile-switch-6mm  i="),
        "{}",
        snapshot[3]
    );
    assert!(
        snapshot[4].starts_with("  r2    resistor-axial      i="),
        "{}",
        snapshot[4]
    );
    assert!(
        snapshot[5].starts_with("  r1    resistor-axial      i="),
        "{}",
        snapshot[5]
    );
    assert_eq!(
        snapshot[6],
        "  led1  led-5mm             on i=0.012246558123229083 A"
    );
    assert_eq!(
        snapshot[7],
        "  warning[net.floating] #net.nano__AREF has no DC path to ground"
    );

    assert_eq!(
        block(8),
        ["10> release @btn1", "  released btn1 at t=101ms"]
    );
    let step = block(9);
    assert_eq!(
        step[..3],
        [
            "11> step 50ms",
            "  t=151ms (from 101ms)",
            "    101ms nano.D2 low"
        ]
    );
    assert_eq!(step[4], "    102ms led1 off");
    assert_eq!(
        block(10),
        [
            "12> expect no diagnostics",
            "  passed no diagnostics (no diagnostics)"
        ]
    );
    assert_eq!(block(11)[..2], ["13> close", "  closed at t=151ms"]);
    assert_eq!(
        run.stdout.lines().rev().take(2).collect::<Vec<_>>(),
        ["1 passed, 0 failed", "PASS tests/a.test"]
    );
}

#[test]
fn each_command_replies_with_one_json_envelope() {
    let json = replies(TRANSCRIPT);
    assert_eq!(json.len(), 12);
    for reply in &json {
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["session"], "tests/a.test");
        assert!(reply["error"].is_null());
    }
    let [
        open,
        step,
        expect,
        press,
        until,
        high,
        compare,
        snapshot,
        release,
        step2,
        clean,
        close,
    ] = json.as_slice()
    else {
        unreachable!()
    };
    assert_eq!(
        (open["t_ps"].as_u64(), open["state"].as_str()),
        (Some(0), Some("paused"))
    );
    assert_eq!(open["data"]["scene"], "scene.jsonc");
    assert_eq!(
        open["data"]["parts"],
        serde_json::json!(["nano", "btn1", "r2", "r1", "led1"])
    );
    assert_eq!(
        (
            open["data"]["engine_parts"].as_u64(),
            open["data"]["nets"].as_u64()
        ),
        (Some(18), Some(31))
    );
    assert_eq!(open["data"]["diagnostics"][0]["code"], "net.floating");

    assert_eq!(step["t_ps"], 100_000_000_000u64);
    assert_eq!(
        step["data"],
        serde_json::json!({ "from_ps": 0, "met_at_ps": null, "changes": [], "more": 0 })
    );
    assert_eq!(expect["data"]["passed"], true);
    assert_eq!(expect["data"]["expected"], "@led1 off");
    assert_eq!(
        press["data"],
        serde_json::json!({ "ref": "@btn1", "landed_ps": 100_000_000_000u64 })
    );

    assert_eq!(until["t_ps"], 101_000_000_000u64);
    assert_eq!(until["data"]["from_ps"], 100_000_000_000u64);
    assert_eq!(until["data"]["met_at_ps"], 101_000_000_000u64);
    assert_eq!(until["data"]["check"]["passed"], true);
    let changes = until["data"]["changes"].as_array().unwrap();
    assert_eq!(
        changes[0],
        serde_json::json!({ "kind": "edge", "at_ps": 100_000_000_000u64, "ref": "@nano.D2", "level": "high" })
    );
    assert_eq!(changes[1]["ref"], "@nano.D13");
    assert_eq!(
        changes[2],
        serde_json::json!({ "kind": "led", "at_ps": 101_000_000_000u64, "ref": "@led1", "on": true })
    );

    assert_eq!(high["data"]["actual"], "nano.D13 high");
    assert_eq!(compare["data"]["actual"], "led1.i = 0.012246558123229083 A");

    let parts = snapshot["data"]["parts"].as_array().unwrap();
    assert_eq!(parts[0]["ref"], "@nano");
    assert_eq!(parts[0]["type"], "nano-board");
    assert_eq!(parts[0]["pins"]["D13"]["level"], "high");
    assert_eq!(parts[4]["value"]["on"], true);
    assert_eq!(snapshot["data"]["serial"], "");
    assert_eq!(snapshot["data"]["diagnostics"][0]["code"], "net.floating");

    assert_eq!(release["data"]["landed_ps"], 101_000_000_000u64);
    assert_eq!(step2["data"]["changes"][0]["level"], "low");
    assert_eq!(clean["data"]["actual"], "no diagnostics");
    assert_eq!(
        close["data"],
        serde_json::json!({ "closed": true, "at_ps": 151_000_000_000u64 })
    );
    assert_eq!(
        (close["state"].as_str(), close["t_ps"].as_u64()),
        (Some("closed"), Some(0))
    );
}

#[test]
fn step_until_a_condition_that_already_holds_takes_no_time() {
    let json = replies("open\nstep 10ms\nstep --until @led1 off\n");
    assert_eq!(json[2]["t_ps"], 10_000_000_000u64);
    assert_eq!(json[2]["data"]["met_at_ps"], 10_000_000_000u64);
}

#[test]
fn a_failing_test_reports_its_line_with_expected_and_actual() {
    let dir = project("open\nstep 10ms\nexpect @led1 on\nexpect @led1 off\n");
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 1);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[0], "FAIL tests/a.test:3: expect @led1 on");
    assert_eq!(lines[1], "  error[expect_failed]: expected @led1 on");
    assert_eq!(lines[2], "    expected: @led1 on");
    assert!(
        lines[3].starts_with("    actual:   led1 off, i="),
        "{}",
        lines[3]
    );
    assert_eq!(lines[4], "0 passed, 1 failed");

    let run = orbe(dir.path(), &["test", "--json", "-v"]);
    assert_eq!(run.code, 1);
    let json = run.json();
    assert_eq!(json["ok"], false);
    assert_eq!(
        (
            json["data"]["passed"].as_u64(),
            json["data"]["failed"].as_u64()
        ),
        (Some(0), Some(1))
    );
    let entry = &json["data"]["tests"][0];
    assert_eq!(entry["file"], "tests/a.test");
    assert_eq!(entry["line"], 3);
    assert_eq!(entry["command"], "expect @led1 on");
    assert_eq!(entry["expected"], "@led1 on");
    assert!(
        entry["actual"]
            .as_str()
            .unwrap()
            .starts_with("led1 off, i=")
    );
    assert_eq!(entry["error"]["code"], "expect_failed");
    assert_eq!(
        entry["replies"].as_array().unwrap().len(),
        3,
        "the line after the failure never ran"
    );
}

#[test]
fn a_missing_part_is_part_missing_with_the_orbe_add_hint() {
    let dir = project("open\n");
    fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 1);
    let entry = &run.json()["data"]["tests"][0];
    assert_eq!(entry["error"]["code"], "part_missing");
    assert!(
        entry["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("orbe add led-5mm"),
        "{entry}"
    );
    let check = orbe(dir.path(), &["check", "scene.jsonc", "--json"]);
    let check_hint = &check.json()["data"]["diagnostics"][0]["hint"];
    assert_eq!(
        &entry["error"]["hint"], check_hint,
        "open gives check's hint"
    );

    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stdout
            .starts_with("FAIL tests/a.test:1: open\n  scene.jsonc:"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("error[part_missing]"), "{}", run.stdout);
    assert!(
        !dir.path().join("parts/led-5mm").exists(),
        "open copies no part"
    );
}

#[test]
fn each_error_code() {
    let cases: &[(&str, &str, &str)] = &[
        ("blink", "usage", "`blink` is not a command"),
        ("step 100ms", "no_session", "no session is open"),
        (
            "open missing.jsonc",
            "io_error",
            "no scene file at missing.jsonc",
        ),
        (
            "open\npress @btn2",
            "unknown_ref",
            "@btn2: no part named `btn2`",
        ),
        (
            "open\nget @nano.D99",
            "unknown_ref",
            "`nano` has no pin or part named `D99`",
        ),
        (
            "open\nexpect @r1 on",
            "no_reading",
            "`r1` states no `on` reading",
        ),
        (
            "open\nexpect @led1 high",
            "no_reading",
            "@led1 cannot be high",
        ),
        (
            "open\nexpect @r1.b high",
            "no_reading",
            "no chip pin shares its net",
        ),
        (
            "open\nexpect @led1.i > 5V",
            "no_reading",
            "@led1.i is in A, not V",
        ),
        (
            "open\nget @nano",
            "no_reading",
            "`nano` has no engine part of its own",
        ),
        (
            "open\nget @nano.mcu.i",
            "no_reading",
            "the engine reports no `nano.mcu.i`",
        ),
        (
            "open\nexpect @led1 on",
            "expect_failed",
            "expected @led1 on",
        ),
        (
            "open\nexpect serial contains \"ready\"",
            "expect_failed",
            "expected serial contains \"ready\"",
        ),
        (
            "open\nstep --until @led1 on --timeout 5ms",
            "until_timeout",
            "did not hold within 5ms",
        ),
        ("open\npress @r1", "not_settable", "`r1` takes no input"),
        (
            "open\npress @nano",
            "not_settable",
            "`nano` has no engine part of its own",
        ),
        (
            "open\npress @nano.D2",
            "not_settable",
            "@nano.D2 is not a part",
        ),
        (
            "open\npress @btn1\npress @btn1",
            "no_change",
            "`btn1` is already in that state",
        ),
        (
            "open\nrelease @btn1",
            "no_change",
            "`btn1` is already in that state",
        ),
    ];
    for (test, code, message) in cases {
        let (error, _) = failed(test);
        assert_eq!(error["code"], *code, "{test}: {error}");
        assert!(
            error["message"].as_str().unwrap().contains(message),
            "{test}: {error}"
        );
    }
    let (error, _) = failed("open\npress @btn2");
    assert_eq!(error["hint"], "parts: nano, btn1, r2, r1, led1");
    let (_, entry) = failed("open\nexpect serial contains \"ready\"");
    assert_eq!(entry["actual"], "no serial");
}

#[test]
fn a_scene_that_does_not_check_refuses_to_open() {
    let dir = project("open\n");
    let scene = dir.path().join("scene.jsonc");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(
        &scene,
        text.replace("\"nano.D13\", \"r1.a\"", "\"nano.D14\", \"r1.a\""),
    )
    .unwrap();
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(
        run.json()["data"]["tests"][0]["error"]["code"],
        "scene_invalid"
    );

    let dir = project("open\n");
    fs::remove_file(dir.path().join("build/button.ino.hex")).unwrap();
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(
        run.json()["data"]["tests"][0]["error"]["code"],
        "firmware_missing"
    );
}

#[test]
fn live_commands_outside_a_test_come_with_the_session_process() {
    let dir = project("open\n");
    let run = orbe(dir.path(), &["press", "@btn1"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        "error[usage]: `orbe press` is a live command, and live commands come with the session process\n  hint: write `press <ref>` in a tests/<name>.test file and run `orbe test`\n"
    );
    let run = orbe(dir.path(), &["open", "--json"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.json()["error"]["code"], "usage");
}

#[test]
fn test_paths_name_files_or_project_folders() {
    let dir = project("open\nexpect @led1 off\n");
    fs::write(dir.path().join("tests/b.test"), "open\nexpect @led1 on\n").unwrap();
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stdout
            .starts_with("PASS tests/a.test\nFAIL tests/b.test:2: expect @led1 on\n"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.ends_with("1 passed, 1 failed\n"));

    let run = orbe(dir.path(), &["test", "tests/a.test"]);
    assert_eq!(
        (run.code, run.stdout.as_str()),
        (0, "PASS tests/a.test\n1 passed, 0 failed\n")
    );

    let run = orbe(dir.path(), &["test", "nowhere"]);
    assert_eq!(run.code, 3);
    assert_eq!(
        run.stderr,
        "error[io_error]: no test file or folder at nowhere\n  hint: name a .test file or a project folder\n"
    );

    let run = orbe(dir.path(), &["test", "parts", "--json"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.json()["error"]["code"], "usage");
}
