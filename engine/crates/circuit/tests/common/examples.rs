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

/// The examples the scans of every example leave out, by name. A debug run of `atmega-blink`
/// takes about 6.6 s, and a debug `sim run` of `atmega-button` takes about 8.5 s, so
/// `crates/cli/tests/blink.rs` and `crates/cli/tests/button.rs` each run that example's grid and
/// its steps once, side by side, and assert what `transient_folder` and
/// `every_example_writes_its_grid_and_its_steps_change_nothing_else` assert. No test runs either
/// twice to compare the folders byte for byte.
pub const CHECKED_ON_THEIR_OWN: &[&str] = &["atmega-blink", "atmega-button"];

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
