mod common;

use std::collections::BTreeSet;
use std::fs;

use common::examples::examples;
use common::repo;

fn documented_ids() -> BTreeSet<String> {
    let doc = fs::read_to_string(repo("docs/viewer.md")).unwrap();
    let table = doc.split("| Id | Package |").nth(1).unwrap();
    let table = table.split("\n\n").next().unwrap();
    table
        .lines()
        .filter(|line| line.starts_with("| `"))
        .map(|line| {
            line.trim_start_matches("| `")
                .split('`')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn kebab(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

#[test]
fn every_example_names_a_placeholder_model_for_each_part() {
    let documented = documented_ids();
    assert!(documented.iter().all(|id| kebab(id)), "{documented:?}");
    let mut used = BTreeSet::new();
    for example in examples() {
        let name = example.file_name().unwrap().to_str().unwrap();
        let netlist: serde_json::Value =
            serde_json::from_slice(&fs::read(example.join("netlist.json")).unwrap()).unwrap();
        let path = example.join("viewer.json");
        assert!(path.is_file(), "{name} has no viewer.json");
        let viewer: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(viewer["version"].as_u64(), Some(0), "{name}");
        let want: BTreeSet<_> = netlist["parts"]
            .as_array()
            .unwrap_or_else(|| panic!("{name} parts"))
            .iter()
            .map(|part| part["name"].as_str().unwrap().to_string())
            .collect();
        let parts = viewer["parts"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} viewer parts"));
        let got: BTreeSet<_> = parts.keys().cloned().collect();
        assert_eq!(got, want, "{name}");
        for (part, value) in parts {
            let model = value["model"]
                .as_str()
                .unwrap_or_else(|| panic!("{name} {part} model"));
            let Some(id) = model.strip_prefix("placeholder:") else {
                panic!("{name} {part} model is {model}");
            };
            assert!(kebab(id), "{name} {part} id {id} is not kebab case");
            assert!(
                documented.contains(id),
                "{name} uses {id}, which is not in docs/viewer.md"
            );
            used.insert(id.to_string());
        }
        let count = |id: &str| {
            parts
                .values()
                .filter(|value| value["model"] == format!("placeholder:{id}"))
                .count()
        };
        if count("on-encoder") + count("rotary-encoder") > 0 {
            assert_eq!(count("on-encoder"), 1, "{name} has one second contact");
            assert_eq!(
                count("rotary-encoder"),
                1,
                "{name} pairs it with one encoder"
            );
        }
    }
    let missing: Vec<_> = documented.difference(&used).collect();
    assert!(
        missing.is_empty(),
        "no example uses {}",
        missing
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}
