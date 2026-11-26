//! How `orbe test` finds a test's project, refuses what it cannot run, and exits, and how a
//! refusal reads: a failed `open`, a step past the stop, a ref with nothing to read.

mod live;
#[path = "registry_env.rs"]
mod registry_env;

use std::fs;

use live::{failed, orbe, project, replies};

#[test]
fn a_test_file_outside_its_projects_tests_folder_is_refused() {
    let dir = project("open\n");
    fs::write(dir.path().join("smoke.test"), "open\nexpect @led1 off\n").unwrap();
    let run = orbe(dir.path(), &["test", "smoke.test"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stderr,
        format!(
            "error[usage]: smoke.test is not in its project's tests/ folder\n  hint: its real path is {}; its project is ., the nearest folder above it with scene.jsonc or parts/; put the file in tests/\n",
            dir.path()
                .canonicalize()
                .unwrap()
                .join("smoke.test")
                .display()
        )
    );

    let alone = tempfile::tempdir().unwrap();
    fs::write(alone.path().join("smoke.test"), "open\n").unwrap();
    let run = orbe(alone.path(), &["test", "smoke.test"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .starts_with("error[usage]: smoke.test is in no orbe project\n"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_test_belongs_to_the_nearest_project_above_it() {
    let dir = project("open\n");
    let inner = dir.path().join("tests/proj");
    fs::create_dir_all(&inner).unwrap();
    fs::copy(dir.path().join("scene.jsonc"), inner.join("scene.jsonc")).unwrap();
    fs::write(inner.join("smoke.test"), "open\nexpect @led1 off\n").unwrap();
    let run = orbe(dir.path(), &["test", "tests/proj/smoke.test"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stderr,
        format!(
            "error[usage]: tests/proj/smoke.test is not in its project's tests/ folder\n  hint: its real path is {}; its project is tests/proj, the nearest folder above it with scene.jsonc or parts/; put the file in tests/proj/tests/\n",
            inner.canonicalize().unwrap().join("smoke.test").display()
        )
    );
}

#[test]
fn a_test_run_inside_a_linked_tests_folder_opens_its_project() {
    let dir = project("open\nexpect @led1 off\n");
    fs::rename(dir.path().join("tests"), dir.path().join("checks")).unwrap();
    std::os::unix::fs::symlink("checks", dir.path().join("tests")).unwrap();
    let run = orbe(&dir.path().join("tests"), &["test", "-v", "a.test"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("\n  opened ../scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
}

#[test]
fn a_test_is_found_from_its_real_path() {
    let dir = project("open\nexpect @led1 off\n");
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(elsewhere.path().join("scene.jsonc"), "{}\n").unwrap();
    let real = elsewhere.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::create_dir(real.join("sub")).unwrap();
    fs::copy(dir.path().join("tests/a.test"), real.join("a.test")).unwrap();
    fs::remove_dir_all(dir.path().join("tests")).unwrap();
    std::os::unix::fs::symlink(&real, dir.path().join("tests")).unwrap();
    let real = real.canonicalize().unwrap();
    let elsewhere = std::path::Path::new("..").join(elsewhere.path().file_name().unwrap());

    for named in ["tests/sub/../a.test", "tests/a.test"] {
        let run = orbe(dir.path(), &["test", named]);
        assert_eq!(run.code, 2, "{named}: {}{}", run.stdout, run.stderr);
        assert_eq!(
            run.stderr,
            format!(
                "error[usage]: {named} is not in its project's tests/ folder\n  hint: its real path is {}; its project is {}, the nearest folder above it with scene.jsonc or parts/; put the file in {}/tests/\n",
                real.join("a.test").display(),
                elsewhere.display(),
                elsewhere.display()
            )
        );
    }
}

#[test]
fn the_working_folder_is_the_real_one_whatever_pwd_says() {
    let dir = project("open\nexpect @led1 off\n");
    let other = project("open\nexpect @led1 off\n");
    fs::remove_file(other.path().join("build/button.ino.hex")).unwrap();
    fs::create_dir(dir.path().join("tests/foo")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("tests"), other.path().join("jump")).unwrap();
    let spoof = other.path().join("jump/foo/../../tests");
    assert_eq!(
        spoof.canonicalize().unwrap(),
        dir.path().join("tests").canonicalize().unwrap()
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_orbe"))
        .args(["test", "-v", "a.test"])
        .current_dir(dir.path().join("tests"))
        .env("PWD", &spoof)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("\n  opened ../scene.jsonc at t=0: "),
        "{stdout}"
    );
}

#[test]
fn a_test_run_from_inside_tests_opens_the_project_scene() {
    let dir = project("open\nexpect @led1 off\n");
    let run = orbe(&dir.path().join("tests"), &["test", "-v", "a.test"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(
        run.stdout.contains("\n  opened ../scene.jsonc at t=0: "),
        "{}",
        run.stdout
    );
}

#[test]
fn a_test_with_no_commands_is_refused() {
    let dir = project("");
    fs::write(
        dir.path().join("tests/b.test"),
        "# TODO: write this test\n\n",
    )
    .unwrap();
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 2, "{}", run.stdout);
    assert_eq!(
        run.stdout,
        "FAIL tests/a.test\n  error[usage]: tests/a.test has no commands\n    hint: write live commands with `expect` lines, such as `open` and `expect @led1 off`\n\
         FAIL tests/b.test\n  error[usage]: tests/b.test has no commands\n    hint: write live commands with `expect` lines, such as `open` and `expect @led1 off`\n\
         0 passed, 2 failed\n"
    );
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.json()["data"]["tests"][0]["error"]["code"], "usage");
}

#[test]
fn an_unreadable_tests_folder_is_an_io_error() {
    let dir = project("open\n");
    let tests = dir.path().join("tests");
    let mode = |m| {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tests, fs::Permissions::from_mode(m)).unwrap();
    };
    mode(0o000);
    let readable = fs::read_dir(&tests).is_ok();
    let run = orbe(dir.path(), &["test"]);
    mode(0o755);
    if readable {
        return;
    }
    assert_eq!(run.code, 3, "{}", run.stderr);
    assert!(
        run.stderr
            .starts_with("error[io_error]: cannot read tests: Permission denied"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_test_exits_as_orbe_check_does() {
    let exit = |test: &str| orbe(project(test).path(), &["test"]).code;
    assert_eq!(exit("open\nexpect @led1 lit\n"), 1, "a failed expect");
    assert_eq!(
        exit("open\nstep --until @led1 lit --timeout 1ms\n"),
        1,
        "a failed until"
    );
    assert_eq!(exit("open\npress @r1\n"), 1, "a runtime refusal");
    assert_eq!(exit("open\nstpe 10ms\n"), 2, "a malformed line");
    assert_eq!(
        exit("open\nexpect serial contains \"\"\n"),
        2,
        "an empty match"
    );
    assert_eq!(exit("open nope.jsonc\n"), 3, "a missing scene");

    let dir = project("open\nexpect @led1 lit\n");
    fs::write(dir.path().join("tests/b.test"), "open nope.jsonc\n").unwrap();
    assert_eq!(
        orbe(dir.path(), &["test"]).code,
        3,
        "the worst file decides"
    );
}

#[test]
fn a_failed_open_leaves_the_running_session() {
    let json = replies("open\nstep 20ms\nopen missing.jsonc\n");
    let reopen = &json[2];
    assert_eq!(reopen["error"]["code"], "io_error");
    assert_eq!(
        (reopen["t_ps"].as_u64(), reopen["state"].as_str()),
        (Some(20_000_000_000), Some("paused")),
        "{reopen}"
    );

    let dir = project("open\nstep 20ms\nopen missing.jsonc\n");
    fs::write(
        dir.path().join("tests/b.test"),
        "open\nstep 20ms\nopen bad.jsonc\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("bad.jsonc"),
        "{ \"format\": 1, \"parts\": {} ",
    )
    .unwrap();
    let run = orbe(dir.path(), &["test", "--json", "-v"]);
    let reply = &run.json()["data"]["tests"][1]["replies"][2]["reply"];
    assert_eq!(reply["t_ps"], 20_000_000_000u64, "{reply}");
}

#[test]
fn a_json_open_refusal_carries_its_findings() {
    let dir = project("open\n");
    fs::remove_dir_all(dir.path().join("parts/led-5mm")).unwrap();
    fs::remove_dir_all(dir.path().join("parts/resistor-axial")).unwrap();
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 1);
    let entry = &run.json()["data"]["tests"][0];
    assert_eq!(entry["error"]["code"], "part_missing");
    assert!(
        !entry["error"]["hint"].as_str().unwrap().contains("above"),
        "{entry}"
    );
    let found = entry["data"]["diagnostics"].as_array().unwrap();
    assert_eq!(found.len(), 2, "{entry}");
    for d in found {
        assert!(
            d["hint"].as_str().unwrap().starts_with("run `orbe add"),
            "{d}"
        );
    }
}

#[test]
fn a_step_past_the_stop_is_refused_before_it_runs() {
    let (error, entry) = failed("open\nstep 7201s\n");
    assert_eq!(error["code"], "past_stop", "{entry}");
    assert_eq!(
        error["message"],
        "the step would end past the session's 2-hour stop"
    );
    assert_eq!(
        error["hint"],
        "the session has 7200s left; step at most that"
    );

    let held = replies("open\nstep 1s\nstep --until @led1 off --timeout 7200s\n");
    assert_eq!(
        held[2]["ok"], true,
        "an until that holds takes no time: {}",
        held[2]
    );
    assert_eq!(held[2]["t_ps"], 1_000_000_000_000u64);

    let (error, _) = failed("open\nstep 18446744s\n");
    assert_eq!(error["code"], "past_stop");
    let (error, _) = failed("open\nstep 18446744073709552s\n");
    assert_eq!(error["code"], "usage");
}

#[test]
fn a_ref_with_nothing_to_read_says_what_can_be_read() {
    let (error, _) = failed("open\nget @nano.mcu\n");
    assert_eq!(error["code"], "no_reading", "{error}");
    assert_eq!(error["message"], "`nano.mcu` has no reading of its own");
    let hint = error["hint"].as_str().unwrap();
    assert!(hint.starts_with("read one of its pins: "), "{hint}");
    assert!(hint.contains("PB5"), "{hint}");

    let (error, _) = failed("open\nget @nano\n");
    assert_eq!(error["code"], "no_reading", "{error}");
    let hint = error["hint"].as_str().unwrap();
    assert!(hint.contains("D13"), "{hint}");
    assert!(hint.contains("parts: "), "{hint}");

    let (error, _) = failed("open\nget @led1.ib\n");
    assert_eq!(error["code"], "no_reading", "{error}");
    assert_eq!(error["hint"], "`led1` reports i, v and p");

    let (error, _) = failed("open\nget @nano.mcu.i\n");
    assert_eq!(
        error["hint"],
        "`nano.mcu` reports no quantity; read one of its pins"
    );

    let (error, _) = failed("open two-buttons.jsonc\nget @btn1.i\n");
    assert_eq!(error["code"], "no_reading", "{error}");
    assert_eq!(error["hint"], "name one of its parts: r, sw");
}

#[test]
fn a_folder_named_like_a_test_file_is_skipped_and_the_run_summarizes() {
    let dir = project("open\nexpect @led1 off\n");
    fs::create_dir(dir.path().join("tests/mid.test")).unwrap();
    fs::write(dir.path().join("tests/z.test"), "open\nexpect @led1 off\n").unwrap();
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (
            0,
            "PASS tests/a.test (scene.jsonc)\nPASS tests/z.test (scene.jsonc)\n2 passed, 0 failed\n",
            ""
        )
    );

    fs::remove_file(dir.path().join("tests/a.test")).unwrap();
    fs::remove_file(dir.path().join("tests/z.test")).unwrap();
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr.starts_with("error[usage]: no tests in tests\n"),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("Is a directory"), "{}", run.stderr);
}

#[test]
fn a_utf8_byte_order_mark_on_a_scene_or_a_test_is_accepted() {
    let dir = project("open\nexpect @led1 off\n");
    for name in ["scene.jsonc", "tests/a.test"] {
        let path = dir.path().join(name);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("\u{feff}{text}")).unwrap();
    }
    let check = orbe(dir.path(), &["check", "scene.jsonc"]);
    assert_eq!(check.code, 0, "{}{}", check.stdout, check.stderr);
    assert!(
        check.stdout.starts_with("scene.jsonc: ok"),
        "{}",
        check.stdout
    );
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (
            0,
            "PASS tests/a.test (scene.jsonc)\n1 passed, 0 failed\n",
            ""
        )
    );
}

#[test]
fn a_test_with_no_expect_line_fails() {
    let dir = project("open\n");
    fs::write(
        dir.path().join("tests/b.test"),
        "open\nstep 10ms\nget @led1\n",
    )
    .unwrap();
    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 2, "{}", run.stdout);
    assert_eq!(
        run.stdout,
        "FAIL tests/a.test (scene.jsonc)\n  error[usage]: tests/a.test has no `expect` line, so it checks nothing\n    hint: add an `expect` line, such as `expect @led1 off` or `expect no diagnostics`\n\
         FAIL tests/b.test (scene.jsonc)\n  error[usage]: tests/b.test has no `expect` line, so it checks nothing\n    hint: add an `expect` line, such as `expect @led1 off` or `expect no diagnostics`\n\
         0 passed, 2 failed\n"
    );
    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 2, "{}", run.stdout);
    let json = run.json();
    assert_eq!(json["ok"], false);
    assert_eq!(json["data"]["tests"][0]["ok"], false);
    assert_eq!(json["data"]["tests"][0]["scene"], "scene.jsonc");
    assert_eq!(json["data"]["tests"][0]["error"]["code"], "usage");
    assert!(
        json["data"]["tests"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("checks nothing"),
        "{}",
        json
    );
}

#[test]
fn orbe_test_names_the_scene_each_test_opened() {
    let dir = project("open\nexpect @led1 off\n");
    fs::copy(dir.path().join("scene.jsonc"), dir.path().join("alt.jsonc")).unwrap();
    fs::write(
        dir.path().join("tests/b.test"),
        "open alt.jsonc\nexpect @led1 off\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("tests/c.test"),
        "open alt.jsonc\nexpect @led1 lit\n",
    )
    .unwrap();

    let run = orbe(dir.path(), &["test"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(
        run.stdout.contains("PASS tests/a.test (scene.jsonc)\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("PASS tests/b.test (alt.jsonc)\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("FAIL tests/c.test:2 (alt.jsonc): expect @led1 lit\n"),
        "{}",
        run.stdout
    );

    let run = orbe(dir.path(), &["test", "--json"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    let tests = run.json()["data"]["tests"].as_array().unwrap().clone();
    assert_eq!(tests[0]["file"], "tests/a.test");
    assert_eq!(tests[0]["scene"], "scene.jsonc");
    assert_eq!(tests[0]["ok"], true);
    assert_eq!(tests[1]["file"], "tests/b.test");
    assert_eq!(tests[1]["scene"], "alt.jsonc");
    assert_eq!(tests[1]["ok"], true);
    assert_eq!(tests[2]["file"], "tests/c.test");
    assert_eq!(tests[2]["scene"], "alt.jsonc");
    assert_eq!(tests[2]["ok"], false);
}

/// Copies `blocks/nano-oled` (which runs much slower than real time during its I2C transfer
/// after t=122ms) into a temporary project with its `tests/` replaced by `tests`.
fn slow_oled_project(tests: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    live::common::copy_dir(&live::common::repo().join("blocks/nano-oled"), dir.path());
    fs::remove_file(dir.path().join("block.jsonc")).unwrap();
    std::os::unix::fs::symlink(live::common::repo().join("parts"), dir.path().join("parts"))
        .unwrap();
    fs::remove_dir_all(dir.path().join("tests")).unwrap();
    fs::create_dir(dir.path().join("tests")).unwrap();
    for (name, body) in tests {
        fs::write(dir.path().join("tests").join(name), body).unwrap();
    }
    dir
}

fn orbe_with_env(dir: &std::path::Path, args: &[&str], envs: &[(&str, &str)]) -> live::Run {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_orbe"));
    registry_env::apply(&mut command);
    command
        .args(args)
        .current_dir(dir)
        .env_remove("ORBE_TEST_STEP_BUDGET")
        .env_remove("ORBE_TEST_STEP_PROGRESS");
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    live::Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

#[test]
fn a_slow_test_step_hits_step_timeout_and_the_remaining_tests_still_run() {
    let dir = slow_oled_project(&[
        (
            "a_slow.test",
            "open\nstep 122ms\nstep 3000ms\nexpect no diagnostics\n",
        ),
        ("b_after.test", "open\nstep 10ms\nexpect no diagnostics\n"),
    ]);

    // Default limit path (shortened via the debug-only seam so the test runs quickly).
    let run = orbe_with_env(dir.path(), &["test"], &[("ORBE_TEST_STEP_BUDGET", "200ms")]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("FAIL tests/a_slow.test:3 (scene.jsonc): step 3000ms\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("error[step_timeout]: step stopped at t="),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("of the 3s asked"), "{}", run.stdout);
    assert!(
        run.stdout.contains("after the 200ms wall-time limit"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("slower than real time"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("--step-timeout"), "{}", run.stdout);
    assert!(
        run.stdout
            .contains("PASS tests/b_after.test (scene.jsonc)\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.ends_with("1 passed, 1 failed\n"),
        "{}",
        run.stdout
    );

    // Same failure with explicit `--step-timeout` and `--json`.
    let json_run = orbe(dir.path(), &["test", "--step-timeout", "200ms", "--json"]);
    assert_eq!(json_run.code, 1, "{}{}", json_run.stdout, json_run.stderr);
    assert!(json_run.stderr.is_empty(), "{}", json_run.stderr);
    let envelope = json_run.json();
    assert_eq!(envelope["ok"], false, "{envelope}");
    assert_eq!(envelope["data"]["passed"], 1, "{envelope}");
    assert_eq!(envelope["data"]["failed"], 1, "{envelope}");
    let tests = envelope["data"]["tests"].as_array().unwrap();
    assert_eq!(tests[0]["file"], "tests/a_slow.test");
    assert_eq!(tests[0]["ok"], false);
    assert_eq!(tests[0]["line"], 3);
    assert_eq!(tests[0]["error"]["code"], "step_timeout");
    let msg = tests[0]["error"]["message"].as_str().unwrap();
    assert!(msg.contains("slower than real time"), "{msg}");
    assert!(msg.contains("of the 3s asked"), "{msg}");
    assert_eq!(tests[1]["file"], "tests/b_after.test");
    assert_eq!(tests[1]["ok"], true);
}

#[test]
fn step_timeout_flag_raises_limit_and_handles_until_and_usage() {
    let dir = slow_oled_project(&[(
        "a.test",
        "open\nstep 122ms\nstep 2ms\nexpect no diagnostics\n",
    )]);

    // Too short: fails with step_timeout.
    let short = orbe(dir.path(), &["test", "--step-timeout", "1ms"]);
    assert_eq!(short.code, 1, "{}{}", short.stdout, short.stderr);
    assert!(
        short.stdout.contains("error[step_timeout]:"),
        "{}",
        short.stdout
    );

    // Longer limit: passes (both after and before the subcommand).
    let long = orbe(dir.path(), &["test", "--step-timeout", "30s"]);
    assert_eq!(long.code, 0, "{}{}", long.stdout, long.stderr);
    let before = orbe(dir.path(), &["--step-timeout", "30s", "test"]);
    assert_eq!(before.code, 0, "{}{}", before.stdout, before.stderr);

    // `step --until` hitting the wall-time limit fails with `step_timeout`.
    let until_dir = slow_oled_project(&[(
        "until.test",
        "open\nstep 122ms\nstep --until @oled.display.screen lit --timeout 10s\nexpect no diagnostics\n",
    )]);
    let until = orbe(until_dir.path(), &["test", "--step-timeout", "200ms"]);
    assert_eq!(until.code, 1, "{}{}", until.stdout, until.stderr);
    assert!(
        until
            .stdout
            .contains("error[step_timeout]: step --until stopped at t="),
        "{}",
        until.stdout
    );
    assert!(
        until.stdout.contains("of the 10s timeout"),
        "{}",
        until.stdout
    );
    assert!(
        until.stdout.contains("expected: @oled.display.screen lit"),
        "{}",
        until.stdout
    );
    assert!(until.stdout.contains("--step-timeout"), "{}", until.stdout);

    // Help documents `--step-timeout`.
    let help = orbe(dir.path(), &["test", "--help"]);
    assert_eq!(help.code, 0, "{}{}", help.stdout, help.stderr);
    assert!(help.stdout.contains("--step-timeout"), "{}", help.stdout);

    // Invalid `--step-timeout` is a usage error (exit 2) in text and JSON.
    for bad in ["0s", "nope"] {
        let text = orbe(dir.path(), &["test", "--step-timeout", bad]);
        assert_eq!(text.code, 2, "{bad}: {}{}", text.stdout, text.stderr);
        assert!(
            text.stderr.contains("error[usage]:"),
            "{bad}: {}",
            text.stderr
        );

        let json = orbe(dir.path(), &["test", "--step-timeout", bad, "--json"]);
        assert_eq!(json.code, 2, "{bad}: {}{}", json.stdout, json.stderr);
        assert!(json.stderr.is_empty(), "{bad}: {}", json.stderr);
        assert_eq!(json.json()["error"]["code"], "usage");
    }
}

#[test]
fn a_long_test_prints_one_progress_line_to_stderr_unless_json() {
    let dir = slow_oled_project(&[(
        "a.test",
        "open\nstep 122ms\nstep 2ms\nstep 2ms\nexpect no diagnostics\n",
    )]);

    let run = orbe_with_env(
        dir.path(),
        &["test", "--step-timeout", "30s"],
        &[("ORBE_TEST_STEP_PROGRESS", "50ms")],
    );
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let lines: Vec<&str> = run.stderr.lines().collect();
    assert_eq!(lines.len(), 1, "expected one progress line: {:?}", lines);
    let after_t = lines[0]
        .strip_prefix("tests/a.test:")
        .and_then(|rest| rest.split_once(": still running at t="))
        .map(|(_, reached)| reached)
        .unwrap_or_else(|| panic!("unexpected progress line: {}", lines[0]));
    assert!(
        orbe_session::Duration::parse(after_t).is_ok_and(|reached| reached.as_ps() > 0),
        "{}",
        lines[0]
    );

    let json = orbe_with_env(
        dir.path(),
        &["test", "--step-timeout", "30s", "--json"],
        &[("ORBE_TEST_STEP_PROGRESS", "50ms")],
    );
    assert_eq!(json.code, 0, "{}{}", json.stdout, json.stderr);
    assert!(json.stderr.is_empty(), "{}", json.stderr);
    assert_eq!(json.json()["ok"], true);
}
