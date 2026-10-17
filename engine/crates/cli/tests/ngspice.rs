use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sim_core::name::Ref;
use sim_core::timestamp::Timestamp;

/// What `scripts/ngspice_refs.py` writes into `ngspice.json`.
const NGSPICE: &str = "ngspice-42";
const OPTIONS: &str = "temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12";

fn examples() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut found: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("netlist.json").is_file())
        .collect();
    found.sort();
    found
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// `#net.*` and `*.v` are volts. `*.i` is amperes. `#net.i` is a voltage.
fn floor(reference: &str) -> f64 {
    if reference.starts_with("#net.") || reference.ends_with(".v") {
        1e-9
    } else if reference.ends_with(".i") {
        1e-12
    } else {
        panic!("{reference} is neither a voltage nor a current");
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn floor_treats_a_net_as_a_voltage_and_a_part_current_as_a_current() {
    assert_eq!(floor("#net.vout"), 1e-9);
    assert_eq!(floor("#net.i"), 1e-9);
    assert_eq!(floor("#r1.v"), 1e-9);
    assert_eq!(floor("#led1.i"), 1e-12);
}

#[test]
#[should_panic(expected = "neither a voltage nor a current")]
fn floor_rejects_a_power_ref() {
    floor("#r1.p");
}

#[test]
fn every_example_matches_its_ngspice_reference_within_a_tenth_of_a_percent() {
    let names: Vec<_> = examples()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["divider", "led-5v", "led-9v"]);
    for example in examples() {
        let label = example.display();
        let reference_path = example.join("ngspice.json");
        assert!(
            reference_path.is_file(),
            "{label} has no ngspice.json; run `uv run scripts/ngspice_refs.py {label}`"
        );
        let reference = json(&reference_path);
        let deck = fs::read(example.join("deck.cir")).unwrap();
        assert_eq!(
            sha256_hex(&deck),
            reference["deck_sha256"].as_str().unwrap(),
            "{label}/deck.cir does not match deck_sha256"
        );
        assert_eq!(reference["format"].as_u64(), Some(0), "{label} format");
        assert_eq!(
            reference["ngspice"].as_str(),
            Some(NGSPICE),
            "{label} ngspice"
        );
        assert_eq!(
            reference["options"].as_str(),
            Some(OPTIONS),
            "{label} options"
        );
        let input = fs::read(example.join("netlist.json")).unwrap();
        let done = sim_cli::run(&input, Timestamp::from_unix_seconds(0));
        let manifest: serde_json::Value =
            serde_json::from_slice(done.folder.file("manifest.json").unwrap()).unwrap();
        assert_eq!(
            manifest["input"]["sha256"], reference["netlist_sha256"],
            "{label}/netlist.json changed; run `uv run scripts/ngspice_refs.py {label}`"
        );
        let op = done
            .results
            .observations()
            .op
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: no operating point"));
        let wanted = reference["op"].as_object().unwrap();
        assert!(!wanted.is_empty(), "{label}");
        for (key, want) in wanted {
            let want = want.as_f64().unwrap();
            let got = op.values[&key.parse::<Ref>().unwrap()].get();
            let bound = 1e-3 * want.abs() + floor(key);
            assert!(
                (got - want).abs() <= bound,
                "{label} {key}: {got} vs ngspice {want}"
            );
        }
    }
}
