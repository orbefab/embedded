//! Each folder in `tests/good/` is laid over the `nano-button` fixture, and its `scene.jsonc`
//! must read. The `expect` file's first line is `<n> engine parts`. Each further line is
//! `<engine part> <JSON pointer> <JSON>`, the value the engine gets there. `orbe check` prints
//! ok for each and exits 0, in the CLI's own tests.

mod common;

use std::fs;

use serde_json::Value;

#[test]
fn every_good_fixture_reads_with_its_values() {
    let mut failures = Vec::new();
    for case in common::cases("good") {
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let expect = fs::read_to_string(case.join("expect")).unwrap();
        let mut lines = expect.lines();
        let parts: usize = lines
            .next()
            .and_then(|l| l.strip_suffix(" engine parts"))
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{name}: the first line is `<n> engine parts`"));
        let project = common::overlaid(&case);
        let netlist = match orbe_scene::read(&project.path().join("scene.jsonc")) {
            Ok(netlist) => netlist,
            Err(found) => {
                failures.push(format!("{name}: {found:#?}"));
                continue;
            }
        };
        if netlist.parts != parts {
            failures.push(format!(
                "{name}: {} engine parts, expected {parts}",
                netlist.parts
            ));
        }
        let json: Value = serde_json::from_str(&netlist.json).unwrap();
        for line in lines {
            let mut words = line.splitn(3, ' ');
            let (part, pointer, want) = (
                words.next().unwrap(),
                words.next().unwrap(),
                words.next().unwrap(),
            );
            let want: Value = serde_json::from_str(want).unwrap();
            let found = json["parts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == part)
                .and_then(|p| p.pointer(pointer));
            if found != Some(&want) {
                failures.push(format!(
                    "{name}: {part} {pointer} is {found:?}, expected {want}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
