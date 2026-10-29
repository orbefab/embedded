//! The examples the tests scan. `crates/cli/tests/common` includes this file by path, so it uses
//! only `sim_core` and `std`.

use std::fs;
use std::path::{Path, PathBuf};

use sim_core::blob::Blobs;
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

/// The blobs an example's netlist names, each under its hash: from the example's `blobs` folder,
/// which `sim run` reads beside the netlist, and the rest from the cache `scripts/blobs.sh`
/// fills, which no example commits. Panics naming the script when the cache lacks one.
pub fn blobs(example: &Path) -> Blobs {
    let json = fs::read(example.join("netlist.json")).unwrap();
    let mut blobs = Blobs::new();
    for sha256 in netlist::blob_names(&json) {
        let name = sha256.to_string();
        let path = [example.join("blobs"), cache()]
            .into_iter()
            .map(|folder| folder.join(&name))
            .find(|path| path.is_file())
            .unwrap_or_else(|| {
                panic!(
                    "{} names blob {name}, which neither its blobs folder nor {} holds; run scripts/blobs.sh",
                    example.display(),
                    cache().display()
                )
            });
        blobs.insert(sha256, fs::read(&path).unwrap().into());
    }
    blobs
}

/// What `sim run` needs beside an example's netlist to find its blobs: `--blobs` and the cache
/// `scripts/blobs.sh` fills, when the netlist names a blob its `blobs` folder lacks.
pub fn blob_args(example: &Path) -> Vec<String> {
    let json = fs::read(example.join("netlist.json")).unwrap();
    let lacks = netlist::blob_names(&json)
        .iter()
        .any(|sha256| !example.join("blobs").join(sha256.to_string()).is_file());
    if lacks {
        vec!["--blobs".to_string(), cache().display().to_string()]
    } else {
        Vec::new()
    }
}

/// The folder `scripts/blobs.sh` fills and prints.
fn cache() -> PathBuf {
    let set = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(folder) = set("SIM_BLOBS") {
        return PathBuf::from(folder);
    }
    let cache = set("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".cache"));
    cache.join("sim-engine/blobs")
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
