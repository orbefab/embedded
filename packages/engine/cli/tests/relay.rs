//! The relay with no diode. The coil rings faster than the 50 us grid, so the rows are not
//! compared. The collector envelope and the release edge are.

mod common;

use std::fs;
use std::path::Path;

use orbe_engine_core::diagnostic::Severity;
use orbe_engine_core::logic::{Level, Logic};
use orbe_engine_core::timestamp::Timestamp;
use sha2::{Digest, Sha256};

const NGSPICE: &str = "ngspice-42";
const INDUCTOR_OPTIONS: &str = "temp=25 tnom=25 reltol=1e-5 abstol=1e-15 vntol=1e-12 gmin=1e-12";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn series(value: &serde_json::Value, key: &str) -> Vec<f64> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} is not a list"))
        .iter()
        .map(|item| {
            item.as_f64()
                .unwrap_or_else(|| panic!("{key} holds a non-number"))
        })
        .collect()
}

fn contact_changes(logic: &Logic, key: &str) -> Vec<(u64, &'static str)> {
    let signal = logic
        .signals()
        .iter()
        .position(|reference| reference.to_string() == key)
        .unwrap_or_else(|| panic!("the engine has no {key}"));
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == signal)
        .map(|change| {
            let level = match change.level {
                Level::Low => "low",
                Level::High => "high",
            };
            (change.at.as_ps(), level)
        })
        .collect()
}

#[test]
fn the_bare_flyback_matches_ngspice_on_the_peak_the_minimum_and_the_release() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p-relay-npn-bare");
    let input = fs::read(fixture.join("netlist.json")).unwrap();
    let deck = fs::read(fixture.join("deck.cir")).unwrap();
    let reference: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.join("ngspice.json")).unwrap()).unwrap();
    assert!(common::skips_reference_rows(&fixture));
    let reason = fs::read_to_string(fixture.join(common::ROWS_SKIPPED_FILE)).unwrap();
    assert_eq!(reference["format"].as_u64(), Some(0));
    assert_eq!(reference["ngspice"].as_str(), Some(NGSPICE));
    assert_eq!(reference["options"].as_str(), Some(INDUCTOR_OPTIONS));
    assert_eq!(reference["rows_skipped"].as_str(), Some(reason.trim()));
    assert_eq!(reference["collector"].as_str(), Some("#net.k"));
    assert_eq!(
        sha256_hex(&deck),
        reference["deck_sha256"].as_str().unwrap()
    );
    assert!(reference.get("tran").is_none(), "the rows are not stored");
    assert!(reference.get("op").is_none(), "the rows are not stored");

    let done = orbe_engine_cli::run(&input, Timestamp::from_unix_seconds(0));
    assert!(
        done.results
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity() != Severity::Error),
        "{:?}",
        done.results.diagnostics()
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(done.folder.file("manifest.json").unwrap()).unwrap();
    assert_eq!(manifest["input"]["sha256"], reference["netlist_sha256"]);

    let trace = done.results.trace().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|column| column.to_string() == "#net.k")
        .unwrap();
    let engine_peak = trace
        .max_column(index)
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let engine_minimum = trace
        .min_column(index)
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let peak = reference["peak"].as_f64().unwrap();
    let minimum = reference["minimum"].as_f64().unwrap();
    let bound = 1e-3 * peak.abs();
    assert!(
        (engine_peak - peak).abs() <= bound,
        "peak {engine_peak} V vs ngspice {peak} V, bound {bound} V"
    );
    assert!(
        (engine_minimum - minimum).abs() <= bound,
        "minimum {engine_minimum} V vs ngspice {minimum} V, bound {bound} V"
    );

    let release = &reference["release"];
    assert_eq!(release["ref"].as_str(), Some("#k1.logic.contact"));
    assert_eq!(release["coil"].as_str(), Some("#k1.i"));
    let wanted = release["t_ps"].as_u64().unwrap();
    let lows: Vec<u64> = contact_changes(trace.logic().unwrap(), "#k1.logic.contact")
        .into_iter()
        .filter(|(_, level)| *level == "low")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(lows.len(), 1, "release edges {lows:?}");
    let span = release["span"].as_f64().unwrap();
    let times = series(release, "t_s");
    let current = series(release, "i");
    assert_eq!(times.len(), 2);
    assert_eq!(current.len(), 2);
    let edge_bound = common::coil_edge_bound_ps(span, &times, &current);
    let miss = lows[0].abs_diff(wanted) as f64;
    assert!(
        miss <= edge_bound,
        "release at {} ps vs ngspice {wanted} ps, bound {edge_bound} ps",
        lows[0]
    );
}
