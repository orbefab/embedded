//! `orbe test` on the first target: each live command's reply as text and as JSON, each error
//! code, and how a failing test reports.

mod live;

use std::fs;

use live::{common, failed, orbe, project, replies};

/// Every Block's tests pass where it sits in the repo, reading its parts from the catalog.
#[test]
fn each_block_passes() {
    let mut ids: Vec<String> = fs::read_dir(common::repo().join("blocks"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "nano-button-led",
            "nano-led-fade",
            "nano-light-comparator",
            "uno-button-led",
        ]
    );
    for id in &ids {
        let run = orbe(&common::repo(), &["test", &format!("blocks/{id}")]);
        assert!(
            run.stdout.ends_with("1 passed, 0 failed\n"),
            "{id}: {}{}",
            run.stdout,
            run.stderr
        );
        assert_eq!(run.code, 0, "{id}: {}", run.stderr);
    }
}

fn board_alone(board: &str, wires: &str, test: &str) -> tempfile::TempDir {
    let dir = project(test);
    fs::write(
        dir.path().join("scene.jsonc"),
        format!(
            r#"{{ "format": 1, "parts": {{ "b": {{ "type": "{board}", "firmware": "build/button.ino.hex" }} }}, "wires": [{wires}] }}"#
        ),
    )
    .unwrap();
    dir
}

/// Each board's fastest of three runs is compared, so a burst of load on the machine does not
/// decide it.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the clock compares the Uno's speed with the Nano's"
)]
fn an_uno_simulates_about_as_fast_as_a_nano() {
    let fastest = |board: &str| {
        let dir = board_alone(board, "", "open\nstep 50ms\n");
        (0..3)
            .map(|_| {
                let started = std::time::Instant::now();
                let run = orbe(dir.path(), &["test"]);
                assert_eq!(run.code, 0, "{board}: {}", run.stdout);
                started.elapsed()
            })
            .min()
            .unwrap()
    };
    let nano = fastest("nano-board");
    let uno = fastest("uno-board");
    assert!(
        uno < nano * 5,
        "the Uno took {uno:?} where the Nano took {nano:?}"
    );
}

/// AREF tied to a rail is a common external reference.
#[test]
fn an_uno_with_aref_on_either_rail_raises_no_diagnostic() {
    for rail in ["5V", "3V3"] {
        let dir = board_alone(
            "uno-board",
            &format!(r#"["b.{rail}", "b.AREF"]"#),
            "open\nexpect @b.5V high\nstep 10ms\nexpect no diagnostics\n",
        );
        let run = orbe(dir.path(), &["test"]);
        assert_eq!(run.code, 0, "{rail}: {}", run.stdout);
    }
}

#[test]
fn the_first_target_reads_the_engine_values_while_pressed() {
    let dir =
        project("open\nstep 50ms\npress @btn1\nstep 50ms\nget @nano.D13\nget @led1\nget @led1.i\n");
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(
        run.stdout
            .contains("\n  nano.D13 high 4.400713547712648 V\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("\n  led1 lit i=11.292504388060703 mA v="),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("\n  led1.i 11.292504388060703 mA\n"),
        "{}",
        run.stdout
    );

    let json =
        replies("open\nstep 50ms\npress @btn1\nstep 50ms\nget @nano.D13\nget @led1\nget @led1.i\n");
    assert_eq!(json[4]["data"]["ref"], "@nano.D13");
    assert_eq!(json[4]["data"]["kind"], "pin");
    assert_eq!(json[4]["data"]["value"]["level"], "high");
    assert_eq!(json[4]["data"]["value"]["v"], 4.400713547712648);
    assert_eq!(json[5]["data"]["kind"], "part");
    assert_eq!(json[5]["data"]["value"]["state"], "lit");
    assert_eq!(json[5]["data"]["value"]["i"], 0.011292504388060703);
    assert_eq!(json[5]["data"]["value"]["brightness"], 0.011292504388060728);
    assert_eq!(json[5]["data"]["value"]["over_ps"], 20_000_000_000u64);
    assert_eq!(json[6]["data"]["kind"], "quantity");
    assert_eq!(json[6]["data"]["value"], 0.011292504388060703);
    assert_eq!(json[6]["t_ps"], 100_000_000_000u64);
}

const TRANSCRIPT: &str = "\
# every live command once
open
step 100ms
expect @led1 off
press @btn1
step --until @led1 lit --timeout 50ms
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

    assert_eq!(
        block(0),
        [
            "2> open",
            "  opened scene.jsonc at t=0: nano, btn1, r2, r1, led1 (5 scene parts, expanding to 19 engine parts, 34 nets)",
        ]
    );
    assert_eq!(
        block(1),
        [
            "3> step 100ms",
            "  t=100ms (from 0s)",
            "    brightness led1 off 1.2775403302174304e-13 A over 20ms",
            "    brightness nano.led_l off 1.2775344350286488e-13 A over 20ms",
            "    brightness nano.led_pwr lit 3.0136985454632046 mA over 20ms",
        ]
    );
    assert_eq!(block(2)[0], "4> expect @led1 off");
    assert!(
        block(2)[1].starts_with("  passed @led1 off (led1 off, i="),
        "{}",
        blocks[2]
    );
    assert_eq!(
        block(3),
        [
            "5> press @btn1",
            "  pressed btn1 at t=100ms; reads show it after t=100ms"
        ]
    );

    let until = block(4);
    assert_eq!(until[0], "6> step --until @led1 lit --timeout 50ms");
    assert_eq!(until[1], "  t=104ms (from 100ms), @led1 lit by 104ms");
    assert_eq!(until[2], "    100ms nano.D2 high");
    assert!(
        until[3].starts_with("    100.") && until[3].ends_with("ms nano.D13 high"),
        "{}",
        until[3]
    );
    assert_eq!(
        until[4],
        "    by 103ms led1 dim, brightness=1.6915818683638897 mA over 20ms"
    );
    assert_eq!(
        until[6],
        "    by 104ms led1 lit, brightness=2.2562070877605347 mA over 20ms"
    );

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
            "  passed @led1.i > 5mA (led1.i = 11.2925043880606 mA)"
        ]
    );

    let snapshot = block(7);
    assert_eq!(snapshot[0], "9> snapshot");
    assert_eq!(snapshot[1], "  t=104ms  scene=scene.jsonc");
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
        "  led1  led-5mm             lit i=11.2925043880606 mA brightness=2.2562070877605347 mA over 20ms"
    );
    assert_eq!(snapshot[7], "  diagnostics  none");
    assert_eq!(snapshot.len(), 8, "{}", snapshot.join("\n"));

    assert_eq!(
        block(8),
        [
            "10> release @btn1",
            "  released btn1 at t=104ms; reads show it after t=104ms"
        ]
    );
    let step = block(9);
    assert_eq!(
        step[..3],
        [
            "11> step 50ms",
            "  t=154ms (from 104ms)",
            "    104ms nano.D2 low"
        ]
    );
    assert_eq!(
        step[5],
        "    by 126ms led1 off, brightness=1.277543604373932e-13 A over 20ms"
    );
    assert_eq!(
        block(10),
        [
            "12> expect no diagnostics",
            "  passed no diagnostics (no diagnostics; a chip warning can show up to 10ms late)"
        ]
    );
    assert_eq!(block(11)[..2], ["13> close", "  closed at t=154ms"]);
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
        (Some(19), Some(34))
    );
    assert_eq!(open["data"]["diagnostics"].as_array().unwrap().len(), 0);

    assert_eq!(step["t_ps"], 100_000_000_000u64);
    assert_eq!(
        step["data"],
        serde_json::json!({
            "from_ps": 0,
            "met_at_ps": null,
            "met_by_ps": null,
            "sent_at_ps": null,
            "changes": [],
            "more": 0,
            "leds": [
                { "ref": "@led1", "state": "off", "brightness": 1.2775403302174304e-13, "over_ps": 20_000_000_000u64 },
                { "ref": "@nano.led_l", "state": "off", "brightness": 1.2775344350286488e-13, "over_ps": 20_000_000_000u64 },
                { "ref": "@nano.led_pwr", "state": "lit", "brightness": 0.0030136985454632045, "over_ps": 20_000_000_000u64 },
            ]
        })
    );
    assert_eq!(expect["data"]["passed"], true);
    assert_eq!(expect["data"]["expected"], "@led1 off");
    assert_eq!(
        press["data"],
        serde_json::json!({ "ref": "@btn1", "landed_ps": 100_000_000_000u64 })
    );

    assert_eq!(until["t_ps"], 104_000_000_000u64);
    assert_eq!(until["data"]["from_ps"], 100_000_000_000u64);
    assert_eq!(until["data"]["met_by_ps"], 104_000_000_000u64);
    assert_eq!(until["data"]["check"]["passed"], true);
    let changes = until["data"]["changes"].as_array().unwrap();
    assert_eq!(
        changes[0],
        serde_json::json!({ "kind": "edge", "at_ps": 100_000_000_000u64, "ref": "@nano.D2", "level": "high" })
    );
    assert_eq!(changes[1]["ref"], "@nano.D13");
    assert_eq!(
        changes[2],
        serde_json::json!({
            "kind": "led",
            "by_ps": 103_000_000_000u64,
            "ref": "@led1",
            "state": "dim",
            "brightness": 0.0016915818683638896,
            "over_ps": 20_000_000_000u64
        })
    );

    assert_eq!(high["data"]["actual"], "nano.D13 high");
    assert_eq!(compare["data"]["actual"], "led1.i = 11.2925043880606 mA");

    let parts = snapshot["data"]["parts"].as_array().unwrap();
    assert_eq!(parts[0]["ref"], "@nano");
    assert_eq!(parts[0]["type"], "nano-board");
    assert_eq!(parts[0]["pins"]["D13"]["level"], "high");
    assert_eq!(parts[4]["value"]["state"], "lit");
    assert_eq!(parts[4]["value"]["brightness"], 0.0022562070877605346);
    assert_eq!(snapshot["data"]["serial"], "");
    assert_eq!(snapshot["data"]["diagnostics"].as_array().unwrap().len(), 0);

    assert_eq!(release["data"]["landed_ps"], 104_000_000_000u64);
    assert_eq!(step2["data"]["changes"][0]["level"], "low");
    assert_eq!(
        clean["data"]["actual"],
        "no diagnostics; a chip warning can show up to 10ms late"
    );
    assert_eq!(
        close["data"],
        serde_json::json!({ "closed": true, "at_ps": 154_000_000_000u64 })
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
    let dir = project("open\nstep 10ms\nexpect @led1 lit\nexpect @led1 off\n");
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 1);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[0], "FAIL tests/a.test:3: expect @led1 lit");
    assert_eq!(
        lines[1],
        "  error[expect_failed]: expected @led1 lit at t=10ms"
    );
    assert_eq!(lines[2], "    expected: @led1 lit");
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
    assert_eq!(entry["command"], "expect @led1 lit");
    assert_eq!(entry["expected"], "@led1 lit");
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
            "open\nexpect @r1 lit",
            "no_reading",
            "`r1` states no `lit` reading",
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
            "`nano` has no reading of its own",
        ),
        (
            "open\nget @nano.mcu.i",
            "no_reading",
            "the engine reports no `nano.mcu.i`",
        ),
        (
            "open\nexpect @led1 lit",
            "expect_failed",
            "expected @led1 lit",
        ),
        (
            "open\nexpect serial contains \"ready\"",
            "expect_failed",
            "expected serial contains \"ready\"",
        ),
        (
            "open\nstep --until @led1 lit --timeout 5ms",
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
    assert_eq!(
        entry["actual"],
        "no serial; serial can reach orbe up to one step (10ms) late"
    );
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
fn test_paths_name_files_or_project_folders() {
    let dir = project("open\nexpect @led1 off\n");
    fs::write(dir.path().join("tests/b.test"), "open\nexpect @led1 lit\n").unwrap();
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stdout
            .starts_with("PASS tests/a.test\nFAIL tests/b.test:2: expect @led1 lit\n"),
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

/// The catalog's GL5528 under 10 kohm, and a 10 kohm trimmer at half, seated on a breadboard.
/// From the GL5528 sheet, R is 15 kohm at 10 lx, the centre of its 10 to 20 kohm, and falls 0.6
/// decades per decade of light, to 3.768 kohm at 100 lx and 946.4 ohm at 1000 lx. In the dark
/// it is 1 Mohm. `ldr.a` is `V(5V) * R / (10k + R)` and the wiper is half the 5V pin, which sits
/// one USB diode drop under the host's 5 V, at 4.761 V.
#[test]
fn the_breadboard_ldr_follows_its_sheet_and_the_trimmer_divides() {
    let dir = common::overlaid(
        &common::repo().join("packages/scene/tests/good/ldr_and_trimmer_on_a_breadboard"),
    );
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    fs::write(
        dir.path().join("tests/light.test"),
        "\
open
expect @trim.wiper > 2.3797V   # 4.761 V / 2: 2.3807 V
expect @trim.wiper < 2.3817V
set light 10
step 1ms
expect @ldr.a > 2.8560V   # 15 kohm: 2.8570 V
expect @ldr.a < 2.8580V
set light 100
step 1ms
expect @ldr.a > 1.3021V   # 3.768 kohm: 1.3031 V
expect @ldr.a < 1.3041V
set light 1000
step 1ms
expect @ldr.a > 0.4107V   # 946.4 ohm: 0.4117 V
expect @ldr.a < 0.4127V
set light 0
step 1ms
expect @ldr.a > 4.7136V   # 1 Mohm: 4.7146 V
expect @ldr.a < 4.7156V
expect no diagnostics
",
    )
    .unwrap();
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
}

/// The scene's 10 kohm over a light-dependent resistor, whose resistance is `10 kohm * 10 lx /
/// light`. The scene holds 300 lx and steps to 50 lx at 20 ms. `ldr.v` is `V(5V) * R / (10k + R)`,
/// with the Nano's 5V pin at 4.761 V on USB.
#[test]
fn set_light_moves_the_divider_to_the_curve_s_value() {
    let dir = common::overlaid(&common::repo().join("packages/scene/tests/good/world_ldr"));
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    fs::write(
        dir.path().join("tests/light.test"),
        "\
open
step 10ms
expect @ldr.v > 0.15V   # 300 lx: 333 ohm, 0.154 V
expect @ldr.v < 0.16V
step 15ms
expect @ldr.v > 0.78V   # 50 lx from 20 ms: 2 kohm, 0.794 V
expect @ldr.v < 0.80V
set light 1000
step 1ms
expect @ldr.v > 0.045V  # 100 ohm, 0.0471 V
expect @ldr.v < 0.050V
set light 0
step 1ms
expect @ldr.v > 4.7V    # the dark: 1 Mohm, 4.715 V
",
    )
    .unwrap();
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("\n  set light to 1000 lx at t=25ms; reads show it after t=25ms\n"),
        "{}",
        run.stdout
    );

    fs::write(
        dir.path().join("tests/light.test"),
        "open\nstep 1ms\nset light 80\n",
    )
    .unwrap();
    let json = live::replies_in(&dir);
    assert_eq!(
        json[2]["data"],
        serde_json::json!({ "quantity": "light", "value": 80.0, "unit": "lx", "landed_ps": 20_000_002_000u64 }),
        "a set lands after the scene's last step, at 20 ms, plus the 2 ns gap"
    );

    fs::write(dir.path().join("tests/light.test"), "open\nset light 50\n").unwrap();
    let json = live::replies_in(&dir);
    assert_eq!(json[1]["ok"], true, "{}", json[1]);
    assert_eq!(
        json[1]["data"],
        serde_json::json!({
            "quantity": "light",
            "value": 50.0,
            "unit": "lx",
            "landed_ps": 20_000_000_000u64,
            "already": true,
        }),
        "50 lx is the scene's step at 20 ms, and setting it again names that time"
    );
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("\n  light is already set to 50 lx from t=20ms\n"),
        "{}",
        run.stdout
    );

    let (error, _) = failed("open\nset humidity 40\n");
    assert_eq!(error["code"], "not_settable");
    assert_eq!(error["message"], "the scene does not declare humidity");

    let (error, _) = failed("open\nget humidity\n");
    assert_eq!(error["code"], "not_settable");
    assert_eq!(error["message"], "the scene does not declare humidity");
}

/// `get light` reads the world value in force now. A `set` lands later, so the next `get`
/// still shows the previous value until a `step` passes that landing.
#[test]
fn get_light_reads_the_value_in_force_and_world_dot_reads_the_same() {
    let dir = common::overlaid(&common::repo().join("packages/scene/tests/good/world_ldr"));
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    fs::write(
        dir.path().join("tests/light.test"),
        "\
open
get light
set light 80
get light
step 21ms
get light
get @world.light
",
    )
    .unwrap();
    let json = live::replies_in(&dir);
    let world = |n: usize, value: f64| {
        assert_eq!(
            json[n]["data"],
            serde_json::json!({ "quantity": "light", "value": value, "unit": "lx" }),
            "{}",
            json[n]
        );
    };
    world(1, 300.0);
    assert_eq!(
        json[2]["data"],
        serde_json::json!({
            "quantity": "light",
            "value": 80.0,
            "unit": "lx",
            "landed_ps": 20_000_002_000u64,
        }),
        "the set still lands after the scene's last step, with no already flag"
    );
    world(3, 300.0);
    world(5, 80.0);
    world(6, 80.0);

    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stdout.matches("\n  light 300 lx\n").count(),
        2,
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(
            "\n  set light to 80 lx at t=20.000002ms; reads show it after t=20.000002ms\n"
        ),
        "{}",
        run.stdout
    );
    assert_eq!(
        run.stdout.matches("\n  light 80 lx\n").count(),
        2,
        "{}",
        run.stdout
    );
}

/// A Nano's A6 and A7 are the TQFP's ADC6 and ADC7. Dividers from 5V put 2/3 of it on A6 and
/// 1/3 on A7, and the sketch prints both codes every 10 ms.
#[test]
fn a_nano_reads_the_voltage_on_a6_and_a7() {
    let dir = project(
        "\
open
step 50ms
expect serial contains \"A6=682 A7=341\"
expect no diagnostics
",
    );
    fs::write(
        dir.path().join("scene.jsonc"),
        r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/analog_a6_a7.ino.hex" },
    "r6a": { "type": "resistor-axial", "resistance": "10k" },
    "r6b": { "type": "resistor-axial", "resistance": "20k" },
    "r7a": { "type": "resistor-axial", "resistance": "20k" },
    "r7b": { "type": "resistor-axial", "resistance": "10k" },
  },
  "wires": [
    ["nano.5V", "r6a.a"],
    ["r6a.b", "nano.A6"],
    ["nano.A6", "r6b.a"],
    ["r6b.b", "nano.GND"],
    ["nano.5V", "r7a.a"],
    ["r7a.b", "nano.A7"],
    ["nano.A7", "r7b.a"],
    ["r7b.b", "nano.GND"],
  ],
}
"#,
    )
    .unwrap();
    fs::copy(
        common::repo().join("packages/engine/tests/firmware/sketch/analog_a6_a7.hex"),
        dir.path().join("build/analog_a6_a7.ino.hex"),
    )
    .unwrap();
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
}

/// A Nano reads a DHT22 every 2 s, from 2 s on, and prints what it read. A read sends what the
/// sensor measured as the read before it ended, so a set after the first read shows on the third.
#[test]
fn set_temperature_and_humidity_change_a_later_dht22_reading() {
    let dir = project(
        "\
open
step 2100ms
expect serial contains \"n=1 t=23.4 h=61.7\"
set temperature 30
set humidity 40
step 2000ms
expect serial contains \"n=2 t=23.4 h=61.7\"
step 2000ms
expect serial contains \"n=3 t=30.0 h=40.0\"
expect no diagnostics
",
    );
    fs::write(
        dir.path().join("scene.jsonc"),
        r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/dht22.ino.hex" },
    "dht": { "type": "dht22" },
    "rp": { "type": "resistor-axial", "resistance": "5.1k" },
  },
  "wires": [
    ["nano.5V", "dht.VDD"],
    ["nano.GND", "dht.GND"],
    ["nano.D2", "dht.DATA"],
    ["nano.5V", "rp.a"],
    ["rp.b", "dht.DATA"],
    ["nano.5V", "nano.AREF"],
  ],
  "world": { "temperature": 23.4, "humidity": 61.7 },
}
"#,
    )
    .unwrap();
    fs::copy(
        common::repo().join("packages/engine/tests/firmware/sketch/dht22.hex"),
        dir.path().join("build/dht22.ino.hex"),
    )
    .unwrap();
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
}

#[test]
fn a_set_the_table_refuses_is_a_usage_error() {
    for (line, message) in [
        (
            "set lite 50",
            "`lite` is not a world quantity; the quantities are temperature, humidity, light; usage: set <quantity> <value>",
        ),
        (
            "set light 200000",
            "light is from 0 to 100000 lx, not 200000; usage: set <quantity> <value>",
        ),
        (
            "set temperature warm",
            "`warm` is not a number; write temperature in degC, such as 25; usage: set <quantity> <value>",
        ),
        (
            "set light",
            "takes a quantity and a value, such as `light 50`; usage: set <quantity> <value>",
        ),
    ] {
        let (error, _) = failed(&format!("open\n{line}\n"));
        assert_eq!(error["code"], "usage", "{line}");
        assert_eq!(error["message"], message, "{line}");
    }
}
