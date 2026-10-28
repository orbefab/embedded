//! `examples/atmega-cst816`: an ATmega328P at 3.3 V and 8 MHz reads a `cst816` over the TWI and
//! prints the finger count and the point. The touches sit on the part, and a session that
//! applies the same points writes the folder `sim run` writes for its export.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::atmega::logic_edges;
use common::{read_vcd, repo};
use sim_circuit::{Input, Session, Setting, Status};
use sim_core::name::Name;
use sim_core::netlist::Touch;
use sim_core::run_folder::RunFolder;
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;

const TEXT: &str = "1 291 69\r\n1 16 32\r\n";

const FIRST: Touch = Touch {
    x: 291,
    y: 69,
    down: true,
};
const SECOND: Touch = Touch {
    x: 16,
    y: 32,
    down: true,
};

const FIRST_AT: u64 = 20_000_000_000;
const SECOND_AT: u64 = 41_000_000_000;
const INT_PULSE_PS: u64 = 100_000_000;

#[test]
fn the_chip_runs_cst816_hex_at_8_mhz() {
    let path = repo("examples/atmega-cst816/netlist.json");
    let netlist: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let chip = netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == "u1")
        .unwrap();
    assert_eq!(chip["clock_hz"]["value"], 8_000_000);
    assert_eq!(
        common::records(chip["firmware"].as_str().unwrap()),
        common::records(&std::fs::read_to_string(repo("firmware/sketch/cst816.hex")).unwrap())
    );
}

#[test]
fn the_sketch_prints_the_points_and_the_decoder_agrees() {
    let folder = render_example();
    assert_eq!(serial_text(folder.file("traces/serial.csv").unwrap()), TEXT);
    assert_eq!(
        phases(folder.file("traces/i2c.csv").unwrap()),
        read_phases()
    );
    let vcd = read_vcd(std::str::from_utf8(folder.file("traces/logic.vcd").unwrap()).unwrap());
    let int = logic_edges(&vcd, "#u2.logic.INT");
    assert_eq!(
        int,
        vec![
            (FIRST_AT + 1, false),
            (FIRST_AT + 1 + INT_PULSE_PS, true),
            (SECOND_AT + 1, false),
            (SECOND_AT + 1 + INT_PULSE_PS, true),
        ]
    );
    let dir = scratch_dir();
    folder.write(&dir).unwrap();
    let output = Command::new("uv")
        .args(["run", "scripts/i2c_decode.py"])
        .arg(&dir)
        .current_dir(repo("."))
        .output()
        .expect("uv run scripts/i2c_decode.py: uv is not installed");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(text.contains("22 of 22 rows match"), "{text}");
}

#[test]
fn a_session_touch_finishes_as_sim_run_of_the_export() {
    let mut json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repo("examples/atmega-cst816/netlist.json")).unwrap(),
    )
    .unwrap();
    let parts = json["parts"].as_array_mut().unwrap();
    let chip = parts.iter_mut().find(|part| part["name"] == "u2").unwrap();
    chip.as_object_mut().unwrap().remove("touches");
    let bytes = serde_json::to_vec(&json).unwrap();
    let mut session = Session::record_document(&bytes).unwrap();
    let part = Name::new("u2").unwrap();
    let first = session
        .apply(&touch(&part, FIRST_AT, FIRST))
        .expect("the first point lands");
    let second = session
        .apply(&touch(&part, SECOND_AT, SECOND))
        .expect("the second point lands");
    assert_eq!(first.at, Time::from_ps(FIRST_AT));
    assert_eq!(second.at, Time::from_ps(SECOND_AT));
    assert_eq!(session.advance_to(Time::from_ps(u64::MAX)), Status::Stopped);
    let export = session.export().unwrap();
    let stamp = Timestamp::from_unix_seconds(1_700_000_000);
    let batch = sim_cli::run(&export, stamp).folder;
    let folder = session.finish(stamp).unwrap();
    assert_eq!(folder_paths(&folder), folder_paths(&batch));
    for ((path, got), (_, want)) in folder.files().zip(batch.files()) {
        assert!(got == want, "{path} differs");
    }
    assert_eq!(serial_text(folder.file("traces/serial.csv").unwrap()), TEXT);
}

fn touch(part: &Name, at: u64, point: Touch) -> Input {
    Input {
        part: part.clone(),
        at: Time::from_ps(at),
        setting: Setting::Touch(point),
    }
}

#[test]
fn a_20_mhz_twi_reads_the_chip_id_and_nacks() {
    let folder = render_bytes(
        &std::fs::read(repo(
            "crates/cli/tests/fixtures/i2c-cst816-fast/netlist.json",
        ))
        .unwrap(),
    );
    assert!(
        folder.file("diagnostics.json").unwrap() == b"[]\n",
        "{}",
        std::str::from_utf8(folder.file("diagnostics.json").unwrap()).unwrap()
    );
    assert_eq!(
        phases(folder.file("traces/i2c.csv").unwrap()),
        chip_id_read()
    );
    let vcd = read_vcd(std::str::from_utf8(folder.file("traces/logic.vcd").unwrap()).unwrap());
    for (scl_name, sda_name) in [
        ("#u1.logic.PC5", "#u1.logic.PC4"),
        ("#u2.logic.SCL", "#u2.logic.SDA"),
    ] {
        let moves =
            sda_levels_while_scl_high(&logic_edges(&vcd, scl_name), &logic_edges(&vcd, sda_name));
        assert_eq!(
            moves,
            vec![false, false, true],
            "{sda_name} moved while SCL was high outside the start, the repeated start and the stop"
        );
    }
    let dir = scratch_dir().with_file_name("cst816-fast-check");
    folder.write(&dir).unwrap();
    let output = Command::new("uv")
        .args(["run", "scripts/i2c_decode.py"])
        .arg(&dir)
        .current_dir(repo("."))
        .output()
        .expect("uv run scripts/i2c_decode.py: uv is not installed");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(text.contains("7 of 7 rows match"), "{text}");
}

#[test]
fn sda_while_scl_high_reaches_diagnostics_json() {
    let mut json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repo(
            "crates/cli/tests/fixtures/i2c-cst816-fast/netlist.json",
        ))
        .unwrap(),
    )
    .unwrap();
    json["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "u1")
        .unwrap()["clock_hz"]["value"] = serde_json::json!(40_000_000);
    let folder = render_bytes(&serde_json::to_vec(&json).unwrap());
    let found: Vec<serde_json::Value> =
        serde_json::from_slice(folder.file("diagnostics.json").unwrap()).unwrap();
    let rule = found
        .iter()
        .find(|item| item["data"]["rule"] == "sda_while_scl_high")
        .expect("sda_while_scl_high is in diagnostics.json");
    assert_eq!(rule["code"], "chip.protocol");
    assert_eq!(rule["refs"], serde_json::json!(["#u2"]));
    let t_ps = rule["t_ps"].as_u64().unwrap();
    assert_eq!(
        rule["message"],
        format!(
            "#u2 left SDA unmoved because SCL was high at {t_ps} ps, breaking rule sda_while_scl_high, the first time"
        )
    );
}

fn chip_id_read() -> Vec<(String, [Option<u8>; 4])> {
    let none = [None; 4];
    let row = |phase: &str, cells: [Option<u8>; 4]| (phase.to_string(), cells);
    vec![
        row("start", none),
        row("address", [Some(0x15), Some(0), None, Some(1)]),
        row("data", [None, Some(0), Some(0xA7), Some(1)]),
        row("repeated_start", none),
        row("address", [Some(0x15), Some(1), None, Some(1)]),
        row("data", [None, Some(1), Some(0xB4), Some(0)]),
        row("stop", none),
    ]
}

fn sda_levels_while_scl_high(scl: &[(u64, bool)], sda: &[(u64, bool)]) -> Vec<bool> {
    let before_first_edge = scl.first().is_none_or(|&(_, high)| !high);
    let scl_high_at = |t: u64| match scl.iter().rfind(|&&(at, _)| at <= t) {
        Some(&(_, high)) => high,
        None => before_first_edge,
    };
    sda.iter()
        .filter(|&&(t, _)| scl_high_at(t))
        .map(|&(_, level)| level)
        .collect()
}

fn render_bytes(bytes: &[u8]) -> RunFolder {
    let parsed = sim_core::netlist::parse(bytes).unwrap();
    let results = sim_circuit::simulate(&parsed);
    sim_core::run_folder::render(
        bytes,
        Some(&parsed),
        &results,
        Timestamp::from_unix_seconds(0),
    )
}

fn render_example() -> RunFolder {
    let path = repo("examples/atmega-cst816/netlist.json");
    let bytes = std::fs::read(path).unwrap();
    let parsed = sim_core::netlist::parse(&bytes).unwrap();
    let results = sim_circuit::simulate(&parsed);
    assert!(
        results.diagnostics().is_empty(),
        "{}",
        serde_json::to_string(results.diagnostics()).unwrap()
    );
    sim_core::run_folder::render(
        &bytes,
        Some(&parsed),
        &results,
        Timestamp::from_unix_seconds(0),
    )
}

fn scratch_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("cst816-check")
}

fn serial_text(csv: &[u8]) -> String {
    let mut text = String::new();
    for line in std::str::from_utf8(csv).unwrap().lines().skip(1) {
        let byte: u8 = line.split(',').nth(3).unwrap().parse().unwrap();
        text.push(char::from(byte));
    }
    text
}

/// `(phase, address, read, byte, ack)` of every row.
fn phases(csv: &[u8]) -> Vec<(String, [Option<u8>; 4])> {
    let mut lines = std::str::from_utf8(csv).unwrap().lines();
    assert_eq!(lines.next(), Some("t_ps,part,phase,address,read,byte,ack"));
    lines
        .map(|line| {
            let cell: Vec<&str> = line.split(',').collect();
            assert_eq!(cell.len(), 7, "{line}");
            let number = |k: usize| (!cell[k].is_empty()).then(|| cell[k].parse().unwrap());
            (
                cell[2].to_string(),
                [number(3), number(4), number(5), number(6)],
            )
        })
        .collect()
}

fn read_phases() -> Vec<(String, [Option<u8>; 4])> {
    let none = [None; 4];
    let row = |phase: &str, cells: [Option<u8>; 4]| (phase.to_string(), cells);
    let mut out = Vec::new();
    for (fingers, x_high, x_low, y_high, y_low) in [(1, 1, 35, 0, 69), (1, 0, 16, 0, 32)] {
        out.push(row("start", none));
        out.push(row("address", [Some(0x15), Some(0), None, Some(1)]));
        out.push(row("data", [None, Some(0), Some(0x02), Some(1)]));
        out.push(row("repeated_start", none));
        out.push(row("address", [Some(0x15), Some(1), None, Some(1)]));
        out.push(row("data", [None, Some(1), Some(fingers), Some(1)]));
        out.push(row("data", [None, Some(1), Some(x_high), Some(1)]));
        out.push(row("data", [None, Some(1), Some(x_low), Some(1)]));
        out.push(row("data", [None, Some(1), Some(y_high), Some(1)]));
        out.push(row("data", [None, Some(1), Some(y_low), Some(0)]));
        out.push(row("stop", none));
    }
    out
}

fn folder_paths(folder: &RunFolder) -> Vec<String> {
    folder.files().map(|(path, _)| path.to_string()).collect()
}
