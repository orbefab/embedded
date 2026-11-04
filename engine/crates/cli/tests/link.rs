//! examples/spi-replay: `sim run` writes each event of a `spi_replay` part to `link.csv`, one
//! row per event at the end its clock gives. Two replays whose events end together write
//! those rows by part name.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use common::repo;

fn run(netlist: &Path, out: &Path) -> Vec<u8> {
    let _ = fs::remove_dir_all(out);
    let status = Command::new(env!("CARGO_BIN_EXE_sim"))
        .args([
            "run",
            netlist.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("spawn sim");
    assert!(status.status.success(), "{status:?}");
    let csv = fs::read(out.join("traces/link.csv")).unwrap();
    let _ = fs::remove_dir_all(out);
    csv
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sim-cli-link-{name}-{}", std::process::id()))
}

/// The row each event of a replay part writes, from the netlist alone: its end is its start
/// plus each phase's bits over its lanes and the data's bits over its lanes, each rounded up,
/// in cycles of `clock_hz`, rounded down to a picosecond.
fn expected_rows(part: &serde_json::Value) -> Vec<(u64, String, String)> {
    let clock = u128::from(part["clock_hz"]["value"].as_u64().unwrap());
    let name = part["name"].as_str().unwrap();
    let mut rows = Vec::new();
    for event in part["events"].as_array().unwrap() {
        let bytes: Vec<u8> = event["bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|byte| u8::try_from(byte.as_u64().unwrap()).unwrap())
            .collect();
        let lanes = event["lanes"].as_u64().unwrap();
        let mut cycles = (8 * bytes.len() as u64).div_ceil(lanes);
        let mut phases = Vec::new();
        for phase in event["phases"].as_array().unwrap() {
            let bits = phase["bits"].as_u64().unwrap();
            let lanes = phase["lanes"].as_u64().unwrap();
            cycles += bits.div_ceil(lanes);
            let digits = bits.div_ceil(4) as usize;
            let value = phase["value"].as_u64().unwrap();
            phases.push(format!("{bits}/{lanes}:{value:0digits$x}"));
        }
        let start = event["t_ps"].as_u64().unwrap();
        let end = start + u64::try_from(u128::from(cycles) * 1_000_000_000_000 / clock).unwrap();
        let hash: String = Sha256::digest(&bytes)[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        rows.push((
            end,
            name.to_string(),
            format!(
                "{end},{start},{name},CS,spi,{},{},{},{hash},{lanes}",
                event["cs"].as_str().unwrap(),
                phases.join(" "),
                bytes.len()
            ),
        ));
    }
    rows
}

fn csv(rows: &[(u64, String, String)]) -> String {
    let mut text = String::from("t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash,lanes\n");
    for (_, _, row) in rows {
        text.push_str(row);
        text.push('\n');
    }
    text
}

fn example() -> serde_json::Value {
    let path = repo("examples/spi-replay/netlist.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn the_replay_example_writes_its_event_list_to_link_csv() {
    let netlist = example();
    let rows = expected_rows(&netlist["parts"][0]);
    assert_eq!(rows.len(), 7);
    let got = run(
        &repo("examples/spi-replay/netlist.json"),
        &scratch("example"),
    );
    assert_eq!(String::from_utf8(got).unwrap(), csv(&rows));
}

/// `rp2` replays on its own nets, and its one event ends when `rp1`'s fifth does. The rows
/// that end on that picosecond are written by part name. Listing `rp2` before `rp1` or after
/// it parses to the same part order, because parse sorts parts by name, and both runs write
/// that file.
#[test]
fn two_replays_listed_in_either_order_write_the_same_link_csv() {
    let mut netlist = example();
    let nets = netlist["nets"].as_array_mut().unwrap();
    nets.extend(["e0", "e1", "e2", "e3", "ecs", "esclk"].map(serde_json::Value::from));
    let mut rp2 = netlist["parts"][0].clone();
    rp2["name"] = "rp2".into();
    rp2["pins"] = serde_json::json!({
        "VCC": "v3v3", "GND": "gnd", "CS": "ecs", "SCLK": "esclk",
        "D0": "e0", "D1": "e1", "D2": "e2", "D3": "e3"
    });
    let fifth_end = expected_rows(&netlist["parts"][0])[4].0;
    rp2["events"] = serde_json::json!([
        { "t_ps": fifth_end - 200_000, "phases": [], "lanes": 1, "bytes": [1], "cs": "released" }
    ]);
    let mut ends = Vec::new();
    for (order, label) in [([0, 1], "after"), ([1, 0], "before")] {
        let parts = [netlist["parts"][0].clone(), rp2.clone()];
        let mut doc = netlist.clone();
        let listed: Vec<_> = order.iter().map(|&k| parts[k].clone()).collect();
        doc["parts"] =
            serde_json::Value::Array([listed, vec![netlist["parts"][1].clone()]].concat());
        let dir = scratch(label);
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("netlist.json");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        ends.push(String::from_utf8(run(&input, &dir.join("out"))).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }
    assert_eq!(ends[0], ends[1]);
    let mut rows = expected_rows(&netlist["parts"][0]);
    rows.extend(expected_rows(&rp2));
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    assert_eq!(rows[4].0, rows[5].0, "rp2's event ends with rp1's fifth");
    assert_eq!(ends[0], csv(&rows));
}
