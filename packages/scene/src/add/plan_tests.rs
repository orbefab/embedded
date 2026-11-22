//! `orbe add` copies the closure of a request, and refuses when a destination folder is already
//! there. Nothing in that folder is compared.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::Plan;
use crate::add::snapshot::{Folder, Snapshot, Target};
use crate::add::{AddCode, Request};
use crate::catalog::{self, Catalog};

fn part(id: &str, version: &str, uses: &[(&str, &str)]) -> catalog::CatalogEntry {
    let mut body =
        format!("{{\n  \"format\": 3,\n  \"id\": \"{id}\",\n  \"version\": \"{version}\"");
    if !uses.is_empty() {
        let listed = uses
            .iter()
            .map(|(child, version)| format!("\"{child}\": \"{version}\""))
            .collect::<Vec<_>>()
            .join(", ");
        body.push_str(&format!(",\n  \"uses\": {{ {listed} }}"));
    }
    body.push_str("\n}\n");
    let mut files = BTreeMap::new();
    files.insert("part.jsonc".into(), body.into_bytes());
    catalog::entry_from_files(id, files).unwrap()
}

fn catalog() -> Catalog {
    let mut catalog = Catalog::official();
    catalog.insert(part("led", "0.1.0", &[]));
    catalog.insert(part("board", "0.2.0", &[("led", "0.1.0")]));
    catalog
}

fn snap(targets: BTreeMap<String, Target>) -> Snapshot {
    Snapshot {
        targets,
        parts_denied: None,
        root: PathBuf::from("."),
    }
}

fn ask(ids: &[&str]) -> Request {
    Request {
        ids: ids.iter().map(|id| (*id).to_string()).collect(),
    }
}

fn folder(version: Option<&str>) -> Target {
    let mut files = BTreeMap::new();
    if let Some(version) = version {
        files.insert(
            "part.jsonc".into(),
            format!("{{\"version\": \"{version}\"}}\n").into_bytes(),
        );
    }
    Target::Folder(Folder::from_files(files))
}

fn refusal(plan: Plan) -> super::Refusal {
    match plan {
        Plan::Refuse(refusal) => refusal,
        other => panic!("expected a refusal, got {}", kind(&other)),
    }
}

fn kind(plan: &Plan) -> &'static str {
    match plan {
        Plan::Add(_) => "add",
        Plan::Refuse(_) => "refuse",
        Plan::Usage(_) => "usage",
    }
}

fn wrote(plan: Plan) -> Vec<String> {
    match plan {
        Plan::Add(planned) => planned
            .write
            .into_iter()
            .map(|write| format!("{} {}", write.id, write.version))
            .collect(),
        other => panic!("expected a write, got {}", kind(&other)),
    }
}

#[test]
fn a_board_copies_the_part_it_uses() {
    let ids = wrote(super::plan(
        &catalog(),
        &snap(BTreeMap::new()),
        &ask(&["board"]),
        "orbe add",
    ));
    assert_eq!(ids, ["board 0.2.0", "led 0.1.0"]);
}

#[test]
fn an_existing_folder_is_part_exists_and_names_the_copy_version() {
    let mut targets = BTreeMap::new();
    targets.insert("led".into(), folder(Some("0.1.1")));
    let refused = refusal(super::plan(
        &catalog(),
        &snap(targets),
        &ask(&["led"]),
        "orbe add",
    ));
    assert_eq!(refused.code, AddCode::PartExists);
    assert_eq!(refused.message, "parts/led is already led 0.1.1");
    assert_eq!(refused.hint, "remove it and run `orbe add led`");
}

#[test]
fn a_folder_without_a_version_and_a_file_are_still_part_exists() {
    let mut targets = BTreeMap::new();
    targets.insert("led".into(), folder(None));
    targets.insert("board".into(), Target::Other("a file"));
    let refused = refusal(super::plan(
        &catalog(),
        &snap(targets),
        &ask(&["board"]),
        "orbe add",
    ));
    assert_eq!(refused.code, AddCode::PartExists);
    assert_eq!(
        refused.message,
        "parts/board is already a file, not a folder\nparts/led is already there"
    );
    assert_eq!(refused.hint, "remove them and run `orbe add board`");
}

#[test]
fn two_unnamed_versions_of_one_id_are_a_usage_error() {
    let mut catalog = Catalog::official();
    catalog.insert(part("led", "0.1.0", &[]));
    catalog.insert(part("led", "0.2.0", &[]));
    catalog.insert(part("a", "1.0.0", &[("led", "0.1.0")]));
    catalog.insert(part("b", "1.0.0", &[("led", "0.2.0")]));
    let plan = super::plan(
        &catalog,
        &snap(BTreeMap::new()),
        &ask(&["a", "b"]),
        "orbe add",
    );
    match plan {
        Plan::Usage(message) => {
            assert_eq!(message, "`led` is used at `0.2.0` and at `0.1.0`");
        }
        other => panic!("expected usage, got {}", kind(&other)),
    }
}

#[test]
fn a_named_version_wins_over_the_one_a_parent_uses() {
    let mut catalog = Catalog::official();
    catalog.insert(part("led", "0.1.0", &[]));
    catalog.insert(part("led", "0.2.0", &[]));
    catalog.insert(part("board", "1.0.0", &[("led", "0.1.0")]));
    let ids = wrote(super::plan(
        &catalog,
        &snap(BTreeMap::new()),
        &ask(&["board", "led@0.2.0"]),
        "orbe add",
    ));
    assert_eq!(ids, ["board 1.0.0", "led 0.2.0"]);
}

#[test]
fn a_missing_catalog_version_is_unknown_part() {
    let refused = refusal(super::plan(
        &catalog(),
        &snap(BTreeMap::new()),
        &ask(&["led@9.9.9"]),
        "orbe add",
    ));
    assert_eq!(refused.code, AddCode::UnknownPart);
    assert!(refused.message.contains("led@9.9.9"), "{}", refused.message);
    assert!(refused.message.contains("0.1.0"), "{}", refused.message);
}
