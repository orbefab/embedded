//! The examples the tests scan. `crates/cli/tests/common` includes this file by path, so it uses
//! only `sim_core` and `std`.

use std::fs;
use std::path::{Path, PathBuf};

use sim_core::blob::{Blobs, Sha256};
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

/// The blobs in an example's `blobs` folder, each under the hash its file is named by. `sim run`
/// reads the same folder beside the netlist.
pub fn blobs(example: &Path) -> Blobs {
    let mut blobs = Blobs::new();
    let Ok(entries) = fs::read_dir(example.join("blobs")) else {
        return blobs;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        let sha256 = Sha256::parse(name).unwrap_or_else(|| panic!("{} is a hash", path.display()));
        blobs.insert(sha256, fs::read(&path).unwrap().into());
    }
    blobs
}

/// An example's netlist, parsed with its blobs.
pub fn parse(example: &Path) -> Netlist {
    let json = fs::read(example.join("netlist.json")).unwrap();
    netlist::parse_with(&json, &blobs(example))
        .unwrap_or_else(|errors| panic!("{}: {errors:?}", example.display()))
}

/// Whether a part is an active chip, whose core no ngspice deck has.
pub fn runs_a_core(netlist: &Netlist) -> bool {
    netlist.parts().any(|(_, part)| match &part.model {
        Model::Chip(chip) => match chip {
            Chip::Ne555(_) => false,
            Chip::Atmega328p(_)
            | Chip::SpiReplay(_)
            | Chip::St77916(_)
            | Chip::Cst816(_)
            | Chip::Esp32s3(_)
            | Chip::Ws2812(_)
            | Chip::PulsesReplay(_) => true,
        },
        _ => false,
    })
}

/// The examples the scans of every example leave out, by name. `crates/cli/tests/blink.rs`,
/// `crates/cli/tests/button.rs` and `crates/cli/tests/fade.rs` each run that example once
/// through `sim run`, with its steps, its grid and a grid with no row before `stop_ps`
/// beside it, all at once. They hold the envelope to the steps, and the gridless run's stats,
/// diagnostics, observations, `logic.vcd`, `serial.csv` and `i2c.csv` to the grid's.
/// `examples/npn-blinker` is here too. One run of it accepts on the order of 2e5 steps, and
/// `crates/cli/tests/blinker.rs` is the check. A scan that also keeps every step repeats that cost.
pub const CHECKED_ON_THEIR_OWN: &[&str] = &[
    "atmega-blink",
    "atmega-button",
    "atmega-fade",
    "npn-blinker",
];

pub fn transient_examples() -> Vec<String> {
    examples()
        .into_iter()
        .filter(|example| matches!(parse(example).analysis(), Analysis::Transient(_)))
        .map(|example| example.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| !CHECKED_ON_THEIR_OWN.contains(&name.as_str()))
        .collect()
}
