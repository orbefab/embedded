mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::examples::transient_examples;
use common::{files, has_usart, repo, transient_folder};

fn sim(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_sim"))
        .args(args)
        .output()
        .expect("spawn sim");
    let code = output.status.code().expect("sim exited by signal");
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    (code, stdout, stderr)
}

fn json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).unwrap_or_else(|e| panic!("{e} in {stdout}"))
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("sim-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    path
}

fn example() -> PathBuf {
    named_example("divider")
}

fn named_example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .join("netlist.json")
}

fn codes_and_refs(doc: &serde_json::Value) -> Vec<(String, String)> {
    doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let refs = d["refs"].as_array().unwrap();
            assert_eq!(refs.len(), 1, "{d}");
            (
                d["code"].as_str().unwrap().to_string(),
                refs[0].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn assert_four_files(out: &Path) {
    for name in [
        "manifest.json",
        "netlist.json",
        "diagnostics.json",
        "observations.json",
    ] {
        assert!(out.join(name).is_file(), "missing {name}");
    }
}

#[test]
fn ok_run_writes_the_folder_and_exits_zero() {
    let out = scratch("ok");
    let netlist = example();
    let (code, stdout, stderr) = sim(&[
        "run",
        netlist.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "ok");
    assert_eq!(doc["out"], out.to_str().unwrap());
    assert_eq!(doc["errors"], 0);
    assert_eq!(doc["warnings"], 0);
    assert_eq!(doc["diagnostics"], serde_json::json!([]));
    assert_four_files(&out);
    assert_eq!(
        fs::read(out.join("netlist.json")).unwrap(),
        fs::read(&netlist).unwrap()
    );
    let observations: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("observations.json")).unwrap()).unwrap();
    let vout = observations["op"]["#net.vout"].as_f64().unwrap();
    let expect = 10.0 / 3.0;
    assert!((vout - expect).abs() <= 1e-8 * expect.abs());
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn rejected_netlist_exits_one_and_still_writes_diagnostics() {
    let out = scratch("rejected");
    let netlist = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/fixtures/invalid/several.json");
    let (code, stdout, stderr) = sim(&[
        "run",
        netlist.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "failed");
    assert_eq!(doc["errors"], 2);
    assert_eq!(doc["warnings"], 0);
    let codes: Vec<_> = doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["netlist.unit", "netlist.unknown_net"]);
    assert_four_files(&out);
    let on_disk: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("diagnostics.json")).unwrap()).unwrap();
    assert_eq!(on_disk, doc["diagnostics"]);
    let observations = fs::read(out.join("observations.json")).unwrap();
    assert_eq!(observations, b"{}\n");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert!(manifest["seed"].is_null());
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn missing_file_exits_three_and_writes_nothing() {
    let out = scratch("missing-out");
    let missing = scratch("missing-file").join("netlist.json");
    let (code, stdout, stderr) = sim(&[
        "run",
        missing.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 3, "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "io_error");
    assert_eq!(doc["path"], missing.to_str().unwrap());
    assert!(doc["message"].as_str().unwrap().contains("os error 2"));
    assert!(!out.exists());
}

#[test]
fn bad_argv_exits_two() {
    let (code, stdout, stderr) = sim(&[]);
    assert_eq!(code, 2, "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "usage_error");
    assert!(!doc["message"].as_str().unwrap().is_empty());
    assert!(doc.get("path").is_none());
}

/// The pages that state the version state this one.
#[test]
fn the_docs_name_the_engine_version() {
    let version = sim_core::run_folder::ENGINE_VERSION;
    let page = |name: &str| fs::read_to_string(repo(&format!("docs/{name}"))).unwrap();
    let run_folder = page("run-folder.md");
    assert!(run_folder.contains(&format!("\"engine_version\": \"{version}\"")));
    assert!(run_folder.contains(&format!("The current values are 0 and `{version}`.")));
    assert!(page("versions.md").contains(&format!("This engine is `{version}`.")));
}

#[test]
fn doctor_exits_zero() {
    let (code, stdout, stderr) = sim(&["doctor"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "ok");
    assert_eq!(doc["engine_version"], sim_core::run_folder::ENGINE_VERSION);
    assert_eq!(doc["netlist_versions"], serde_json::json!([0]));
    assert_eq!(doc["run_format"], 0);
    assert_eq!(doc["checks"][0]["name"], "self_test.divider");
    assert_eq!(doc["checks"][0]["ok"], true);
    assert_eq!(doc["checks"][1]["name"], "determinism");
    assert_eq!(doc["checks"][1]["ok"], true);
    assert_eq!(
        doc["checks"][1]["detail"],
        "two renders byte-identical (4 files)"
    );
    let detail = doc["checks"][0]["detail"].as_str().unwrap();
    assert!(detail.starts_with("#net.vout = "), "{detail}");
    assert!(detail.contains("closed form"), "{detail}");
}

#[cfg(unix)]
#[test]
fn non_utf8_out_path_is_a_document() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut bytes = scratch("utf8").into_os_string().into_vec();
    bytes.push(0xff);
    let out = PathBuf::from(OsString::from_vec(bytes));
    let netlist = example();
    let output = Command::new(env!("CARGO_BIN_EXE_sim"))
        .arg("run")
        .arg(&netlist)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("spawn sim");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "ok");
    assert!(
        doc["out"].as_str().unwrap().contains('\u{FFFD}'),
        "{stdout}"
    );
    assert!(out.join("manifest.json").is_file());
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn closed_stdout_still_exits_from_status() {
    let (read, write) = std::io::pipe().unwrap();
    drop(read);
    let output = Command::new(env!("CARGO_BIN_EXE_sim"))
        .args(["doctor"])
        .stdout(write)
        .stderr(Stdio::piped())
        .output()
        .expect("spawn sim");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
}

#[test]
fn a_floating_net_exits_zero_with_one_warning() {
    let dir = scratch("float");
    fs::create_dir_all(&dir).unwrap();
    let netlist = dir.join("netlist.json");
    let out = dir.join("out");
    fs::write(
        &netlist,
        br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "op": {} },
      "ground": "gnd",
      "nets": ["gnd", "isle"],
      "parts": []
    }"#,
    )
    .unwrap();
    let (code, stdout, stderr) = sim(&[
        "run",
        netlist.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "ok");
    assert_eq!(doc["warnings"], 1);
    assert_eq!(doc["errors"], 0);
    assert_eq!(doc["diagnostics"][0]["code"], "net.floating");
    assert_eq!(
        doc["diagnostics"][0]["refs"],
        serde_json::json!(["#net.isle"])
    );
    assert_four_files(&out);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn two_runs_differ_only_in_created_at() {
    let netlist = example();
    let first_out = scratch("twice-a");
    let second_out = scratch("twice-b");
    let (code, stdout, _) = sim(&[
        "run",
        netlist.to_str().unwrap(),
        "--out",
        first_out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stdout}");
    std::thread::sleep(Duration::from_millis(1100));
    let (code, stdout, _) = sim(&[
        "run",
        netlist.to_str().unwrap(),
        "--out",
        second_out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stdout}");
    for name in ["netlist.json", "diagnostics.json", "observations.json"] {
        assert_eq!(
            fs::read(first_out.join(name)).unwrap(),
            fs::read(second_out.join(name)).unwrap(),
            "{name}"
        );
    }
    let mut first: serde_json::Value =
        serde_json::from_slice(&fs::read(first_out.join("manifest.json")).unwrap()).unwrap();
    let mut second: serde_json::Value =
        serde_json::from_slice(&fs::read(second_out.join("manifest.json")).unwrap()).unwrap();
    let first_at = first.as_object_mut().unwrap().remove("created_at").unwrap();
    let second_at = second
        .as_object_mut()
        .unwrap()
        .remove("created_at")
        .unwrap();
    assert_ne!(first_at, second_at);
    assert_eq!(first, second);
    let _ = fs::remove_dir_all(&first_out);
    let _ = fs::remove_dir_all(&second_out);
}

#[test]
fn the_9v_led_breaks_three_ratings_and_still_writes_its_operating_point() {
    let out = scratch("led-9v");
    let netlist = named_example("led-9v");
    let (code, stdout, stderr) = sim(&[
        "run",
        netlist.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let doc = json(&stdout);
    assert_eq!(doc["status"], "ok");
    assert_eq!(doc["errors"], 0);
    assert_eq!(doc["warnings"], 3);
    let found = codes_and_refs(&doc);
    let expected = [
        ("rating.current", "#led1.i"),
        ("rating.power", "#led1.p"),
        ("rating.power", "#r1.p"),
    ];
    let expected: Vec<_> = expected
        .iter()
        .map(|(c, r)| (c.to_string(), r.to_string()))
        .collect();
    assert_eq!(found, expected);

    let observations: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("observations.json")).unwrap()).unwrap();
    for d in doc["diagnostics"].as_array().unwrap() {
        assert_eq!(d["severity"], "warning");
        assert_eq!(d["t_ps"], 0);
        assert!(d["data"]["over_ps"].is_null(), "{d}");
        assert_eq!(d["data"]["peak_ps"], 0);
        let quantity = d["refs"][0].as_str().unwrap();
        let observed = observations["op"][quantity].as_f64().unwrap();
        assert_eq!(d["data"]["peak"].as_f64().unwrap(), observed.abs());
        assert!(observed.abs() > d["data"]["limit"].as_f64().unwrap());
    }
    let on_disk: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("diagnostics.json")).unwrap()).unwrap();
    assert_eq!(on_disk, doc["diagnostics"]);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn the_5v_led_is_within_its_ratings() {
    let out = scratch("led-5v");
    let netlist = named_example("led-5v");
    let (code, stdout, _) = sim(&[
        "run",
        netlist.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stdout}");
    let doc = json(&stdout);
    assert_eq!(doc["diagnostics"], serde_json::json!([]));
    assert_eq!(fs::read(out.join("diagnostics.json")).unwrap(), b"[]\n");
    let observations: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("observations.json")).unwrap()).unwrap();
    assert!(observations["op"]["#led1.i"].as_f64().unwrap() < 0.03);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn each_transient_example_writes_a_trace_whose_row_0_is_the_operating_point() {
    for name in transient_examples() {
        let out = scratch(&format!("tran-{name}"));
        let netlist = named_example(&name);
        let (code, stdout, stderr) = sim(&[
            "run",
            netlist.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
        let doc = json(&stdout);
        let found = codes_and_refs(&doc);
        if name == "regulator-load-step" {
            assert_eq!(
                found,
                vec![
                    ("rating.current".to_string(), "#u1.iout".to_string()),
                    ("rating.power".to_string(), "#u1.p".to_string()),
                ],
                "{name}: {}",
                doc["diagnostics"]
            );
        } else {
            assert!(found.is_empty(), "{name}: {}", doc["diagnostics"]);
        }
        transient_folder(&name, &out);
        let _ = fs::remove_dir_all(&out);
    }
}

#[test]
fn two_runs_of_each_transient_example_differ_only_in_created_at() {
    let names = transient_examples();
    let outs = |round: &str| -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| {
                let out = scratch(&format!("tran-twice-{name}-{round}"));
                let netlist = named_example(name);
                let (code, stdout, _) = sim(&[
                    "run",
                    netlist.to_str().unwrap(),
                    "--out",
                    out.to_str().unwrap(),
                ]);
                assert_eq!(code, 0, "{stdout}");
                out
            })
            .collect()
    };
    let first = outs("a");
    std::thread::sleep(Duration::from_millis(1100));
    let second = outs("b");
    for ((left, right), name) in first.iter().zip(&second).zip(names) {
        let mut left_files = files(left);
        let mut right_files = files(right);
        let manifest = |files: &mut std::collections::BTreeMap<String, Vec<u8>>| {
            let mut doc: serde_json::Value =
                serde_json::from_slice(&files.remove("manifest.json").unwrap()).unwrap();
            doc.as_object_mut().unwrap().remove("created_at").unwrap()
        };
        assert_ne!(manifest(&mut left_files), manifest(&mut right_files));
        let observations: serde_json::Value =
            serde_json::from_slice(&left_files["observations.json"]).unwrap();
        let logic = usize::from(observations.get("logic").is_some());
        let netlist: serde_json::Value =
            serde_json::from_slice(&left_files["netlist.json"]).unwrap();
        let serial = usize::from(has_usart(&netlist));
        assert_eq!(left_files.len(), 7 + logic + serial, "{name}");
        assert_eq!(left_files, right_files, "{}", left.display());
        let _ = fs::remove_dir_all(left);
        let _ = fs::remove_dir_all(right);
    }
}
