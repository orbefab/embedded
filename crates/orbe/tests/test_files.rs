//! How `orbe test` finds a test's project, refuses what it cannot run, and exits, and how a
//! refusal reads: a failed `open`, a step past the stop, a ref with nothing to read.

mod live;

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
            "error[usage]: smoke.test is not in its project's tests/ folder\n  hint: its real path is {}; its project is ., the nearest folder above it with scene.jsonc or parts.lock; put the file in tests/\n",
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
            "error[usage]: tests/proj/smoke.test is not in its project's tests/ folder\n  hint: its real path is {}; its project is tests/proj, the nearest folder above it with scene.jsonc or parts.lock; put the file in tests/proj/tests/\n",
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
                "error[usage]: {named} is not in its project's tests/ folder\n  hint: its real path is {}; its project is {}, the nearest folder above it with scene.jsonc or parts.lock; put the file in {}/tests/\n",
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
    assert_eq!(exit("open\nexpect @led1 on\n"), 1, "a failed expect");
    assert_eq!(
        exit("open\nstep --until @led1 on --timeout 1ms\n"),
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

    let dir = project("open\nexpect @led1 on\n");
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
