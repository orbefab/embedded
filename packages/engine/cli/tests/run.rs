mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::examples::{blob_args, transient_examples};
use common::{display_parts, files, has_kind, led_parts, repo, sends_records, transient_folder};

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
    let path = std::env::temp_dir().join(format!("orbe-engine-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    path
}

fn example() -> PathBuf {
    named_example("divider")
}

fn named_example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/circuits")
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
    let version = orbe_engine_core::run_folder::ENGINE_VERSION;
    let page = |name: &str| fs::read_to_string(repo(&format!("docs/engine/{name}"))).unwrap();
    let run_folder = page("run-folder.md");
    assert!(run_folder.contains(&format!("\"engine_version\": \"{version}\"")));
    assert!(run_folder.contains(&format!("The current values are 0 and `{version}`.")));
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
    let out = scratch("closed-stdout");
    let (read, write) = std::io::pipe().unwrap();
    drop(read);
    let output = Command::new(env!("CARGO_BIN_EXE_sim"))
        .args([
            "run",
            example().to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .stdout(write)
        .stderr(Stdio::piped())
        .output()
        .expect("spawn sim");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    let _ = fs::remove_dir_all(out);
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
        let mut args = vec![
            "run".to_string(),
            netlist.display().to_string(),
            "--out".to_string(),
            out.display().to_string(),
        ];
        args.extend(blob_args(netlist.parent().unwrap()));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, stdout, stderr) = sim(&args);
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
        } else if name == "esp32s3-dualcore" {
            // The 10 registers the app touches that the model does not keep, the write that
            // sets the reset of SYSTEM_WDG at start-up, and the 6 writes that pulse the resets
            // of SYSTIMER, TIMG0 and TIMG1 at start-up.
            let want = vec![("chip.unmodeled_register".to_string(), "#u1".to_string()); 17];
            assert_eq!(found, want, "{name}: {}", doc["diagnostics"]);
            assert_kept_resets(&name, &doc, 7);
        } else if name == "esp32s3-echo" {
            // The 9 registers the app touches that the model does not keep, the write that sets
            // the reset of SYSTEM_WDG at start-up, and the 6 writes that pulse the resets of
            // SYSTIMER, TIMG0 and TIMG1 at start-up.
            let want = vec![("chip.unmodeled_register".to_string(), "#u1".to_string()); 16];
            assert_eq!(found, want, "{name}: {}", doc["diagnostics"]);
            assert_kept_resets(&name, &doc, 7);
        } else if name == "esp32s3-encoder" {
            // The 10 registers the app touches that the model does not keep, among them the two
            // RTC IO pad words `gpio_config` reads, the write that sets the reset of SYSTEM_WDG
            // at start-up, and the 6 writes that pulse the resets of SYSTIMER, TIMG0 and TIMG1
            // at start-up.
            let want = vec![("chip.unmodeled_register".to_string(), "#u1".to_string()); 17];
            assert_eq!(found, want, "{name}: {}", doc["diagnostics"]);
            assert_kept_resets(&name, &doc, 7);
            // The ISR prints one `edge` line for each of the 7 closings, so it fails when the
            // seam hands the chip no pad.
            let serial = fs::read_to_string(out.join("traces/serial.csv")).unwrap();
            let console: String = serial
                .lines()
                .skip(1)
                .map(|row| char::from(row.rsplit(',').next().unwrap().parse::<u8>().unwrap()))
                .collect();
            let edges = console.lines().filter(|line| line.starts_with("edge "));
            assert_eq!(edges.count(), 7, "{name}: {console}");
        } else if name == "esp32s3-hello" {
            // The 8 registers the app touches that the model does not keep, the 9 writes whose
            // effect it does not run: the one that sets the reset of SYSTEM_WDG at start-up, the
            // pulses of the resets of SYSTIMER, TIMG0 and TIMG1 at start-up, and those of SPI0,
            // SPI1 and SYSTIMER at the restart, then its reset.
            let mut want = vec![("chip.unmodeled_register".to_string(), "#u1".to_string()); 17];
            want.push(("chip.restart".to_string(), "#u1".to_string()));
            assert_eq!(found, want, "{name}: {}", doc["diagnostics"]);
            assert_kept_resets(&name, &doc, 9);
        } else if name == "esp32s3-screen" {
            // The 9 registers the app touches that the model does not keep, the write that sets
            // the reset of SYSTEM_WDG at start-up, and the 6 writes that pulse the resets of
            // SYSTIMER, TIMG0 and TIMG1 at start-up.
            let want = vec![("chip.unmodeled_register".to_string(), "#u1".to_string()); 16];
            assert_eq!(found, want, "{name}: {}", doc["diagnostics"]);
            assert_kept_resets(&name, &doc, 7);
        } else if name == "esp32s3-touch" {
            // The 10 registers the app touches that the model does not keep, the write of
            // `I2C_CTR_REG` that changes bits the model keeps and does not run, the write that
            // sets the reset of SYSTEM_WDG at start-up, and the 6 writes that pulse the resets
            // of SYSTIMER, TIMG0 and TIMG1 at start-up.
            let want = vec![("chip.unmodeled_register".to_string(), "#u1".to_string()); 18];
            assert_eq!(found, want, "{name}: {}", doc["diagnostics"]);
            let kept: Vec<_> = doc["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| d["data"]["kept"] == true)
                .collect();
            assert_eq!(kept.len(), 8, "{name}: {}", doc["diagnostics"]);
            assert_eq!(
                kept.iter()
                    .filter(|d| d["data"]["register"] == "SYSTEM+0x20")
                    .count(),
                7,
                "{name}: {}",
                doc["diagnostics"]
            );
            assert_eq!(
                kept.iter()
                    .filter(|d| d["data"]["register"] == "I2C0+0x4")
                    .count(),
                1,
                "{name}: {}",
                doc["diagnostics"]
            );
        } else if name == "esp32s3-ring" {
            // The 8 registers the app touches that the model does not keep, the write that sets
            // the reset of SYSTEM_WDG at start-up, and the 6 writes that pulse the resets of
            // SYSTIMER, TIMG0 and TIMG1 at start-up.
            let want = vec![("chip.unmodeled_register".to_string(), "#u1".to_string()); 15];
            assert_eq!(found, want, "{name}: {}", doc["diagnostics"]);
            assert_kept_resets(&name, &doc, 7);
        } else {
            assert!(found.is_empty(), "{name}: {}", doc["diagnostics"]);
        }
        transient_folder(&name, &out);
        let _ = fs::remove_dir_all(&out);
    }
}

/// That `doc` reports `count` writes to `SYSTEM_PERIP_RST_EN0_REG` whose effect the model does
/// not run, and no other register it keeps.
fn assert_kept_resets(name: &str, doc: &serde_json::Value, count: usize) {
    let kept: Vec<_> = doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["data"]["kept"] == true)
        .collect();
    assert_eq!(kept.len(), count, "{name}: {}", doc["diagnostics"]);
    for d in kept {
        let message = d["message"].as_str().unwrap();
        assert!(
            message.contains("whose effect the model does not run"),
            "{message}"
        );
        assert_eq!(d["data"]["register"], "SYSTEM+0x20", "{message}");
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
                let mut args = vec![
                    "run".to_string(),
                    netlist.display().to_string(),
                    "--out".to_string(),
                    out.display().to_string(),
                ];
                args.extend(blob_args(netlist.parent().unwrap()));
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let (code, stdout, _) = sim(&args);
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
        // An ATmega328P writes `serial.csv` for its USART and `i2c.csv` for its TWI, an
        // ESP32-S3 writes `serial.csv` for its console, a replay or an ESP32-S3 writes
        // `link.csv`, display parts write `display.csv` and one image each, and a `ws2812`
        // writes `leds.csv`.
        let displays = display_parts(&netlist).len();
        let chip = 2 * usize::from(has_kind(&netlist, "atmega328p"))
            + usize::from(has_kind(&netlist, "esp32s3") && !has_kind(&netlist, "atmega328p"))
            + usize::from(sends_records(&netlist))
            + usize::from(displays > 0)
            + displays
            + usize::from(led_parts(&netlist));
        assert_eq!(left_files.len(), 7 + logic + chip, "{name}");
        assert_eq!(left_files, right_files, "{}", left.display());
        let _ = fs::remove_dir_all(left);
        let _ = fs::remove_dir_all(right);
    }
}
