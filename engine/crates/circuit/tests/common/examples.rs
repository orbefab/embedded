//! The examples the tests scan. `crates/cli/tests/common` includes this file by path, so it uses
//! only `sim_core` and `std`.

use std::fs;
use std::path::PathBuf;

use sim_core::netlist::{self, Analysis, Chip, Model, Netlist};

/// The directory of every example, sorted.
pub fn examples() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut found: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("netlist.json").is_file())
        .collect();
    found.sort();
    found
}

pub fn runs_firmware(netlist: &Netlist) -> bool {
    netlist.parts().any(|(_, part)| match &part.model {
        Model::Chip(chip) => match chip {
            Chip::Ne555(_) => false,
            Chip::Atmega328p(_) => true,
        },
        _ => false,
    })
}

/// The examples the scans of every example leave out, by name. `crates/cli/tests/blink.rs`,
/// `crates/cli/tests/button.rs` and `crates/cli/tests/fade.rs` each run that example once
/// through `sim run`, with its steps, its grid and a grid with no row before `stop_ps`
/// beside it, all at once. They hold the envelope to the steps, and the gridless run's stats,
/// diagnostics, observations, `logic.vcd` and `serial.csv` to the grid's.
pub const CHECKED_ON_THEIR_OWN: &[&str] = &["atmega-blink", "atmega-button", "atmega-fade"];

pub fn transient_examples() -> Vec<String> {
    examples()
        .into_iter()
        .filter(|example| {
            let netlist = netlist::parse(&fs::read(example.join("netlist.json")).unwrap()).unwrap();
            matches!(netlist.analysis(), Analysis::Transient(_))
        })
        .map(|example| example.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| !CHECKED_ON_THEIR_OWN.contains(&name.as_str()))
        .collect()
}
