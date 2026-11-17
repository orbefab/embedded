use std::collections::BTreeMap;

use super::*;
use crate::add::snapshot::Folder;
use crate::catalog;

const ADD: &str = "orbe add";

fn files(body: &str) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert("part.jsonc".into(), body.as_bytes().to_vec());
    files
}

fn leaf(id: &str, version: &str, title: &str) -> CatalogEntry {
    catalog::entry_from_files(
        id,
        files(&format!(
            "{{\n  \"format\": 1,\n  \"part\": \"{id}\",\n  \"version\": \"{version}\",\n  \"title\": \"{title}\",\n  \"model\": {{ \"kind\": \"x\" }}\n}}\n"
        )),
    )
    .unwrap()
}

fn board(id: &str, version: &str, uses: &[(&str, &str)]) -> CatalogEntry {
    let uses = uses
        .iter()
        .map(|(child, version)| format!("\"{child}\": \"{version}\""))
        .collect::<Vec<_>>()
        .join(", ");
    catalog::entry_from_files(
        id,
        files(&format!(
            "{{\n  \"format\": 1,\n  \"part\": \"{id}\",\n  \"version\": \"{version}\",\n  \"uses\": {{ {uses} }},\n  \"parts\": {{}}\n}}\n"
        )),
    )
    .unwrap()
}

fn catalog_of(entries: Vec<CatalogEntry>) -> Catalog {
    let mut catalog = Catalog::official();
    for entry in entries {
        catalog.insert(entry);
    }
    catalog
}

fn empty() -> Snapshot {
    Snapshot {
        targets: BTreeMap::new(),
        lock: BTreeMap::new(),
        lock_text: None,
        lock_error: None,
    }
}

fn put(snapshot: &mut Snapshot, id: &str, files: BTreeMap<String, Vec<u8>>) {
    snapshot
        .targets
        .insert(id.into(), Target::Folder(Folder::from_files(files)));
}

/// The copy as `orbe add` left it, with its lock line.
fn install(snapshot: &mut Snapshot, catalog: &Catalog, entry: &CatalogEntry) {
    put(snapshot, &entry.id, entry.files.clone());
    snapshot.lock.insert(
        entry.id.clone(),
        LockFields {
            id: entry.id.clone(),
            version: entry.version.to_string(),
            registry: catalog.registry().to_string(),
            hash: entry.hash.clone(),
        },
    );
}

fn with_folder(entry: &CatalogEntry) -> Snapshot {
    let mut snapshot = empty();
    put(&mut snapshot, &entry.id, entry.files.clone());
    snapshot
}

fn edited(entry: &CatalogEntry) -> BTreeMap<String, Vec<u8>> {
    let mut files = entry.files.clone();
    let mut bytes = files["part.jsonc"].clone();
    bytes.extend_from_slice(b"// edit\n");
    files.insert("part.jsonc".into(), bytes);
    files
}

fn ask(ids: &[&str], overwrite: bool, diff: bool) -> Request {
    Request {
        ids: ids.iter().map(|id| (*id).to_string()).collect(),
        overwrite,
        diff,
        relock: false,
    }
}

fn relock() -> Request {
    Request {
        ids: vec![],
        overwrite: false,
        diff: false,
        relock: true,
    }
}

fn refusal(plan: Plan) -> Refusal {
    match plan {
        Plan::Refuse(refusal) => refusal,
        other => panic!("expected a refusal, got {}", name(&other)),
    }
}

fn added(plan: Plan) -> AddPlan {
    match plan {
        Plan::Add(plan) => plan,
        Plan::Refuse(refusal) => panic!(
            "expected a write plan, got a refusal: {}\n{}",
            refusal.message, refusal.hint
        ),
        other => panic!("expected a write plan, got {}", name(&other)),
    }
}

fn name(plan: &Plan) -> &'static str {
    match plan {
        Plan::Add(_) => "add",
        Plan::Diff(_) => "diff",
        Plan::Relock(_) => "relock",
        Plan::Refuse(_) => "refuse",
        Plan::Usage(_) => "usage",
    }
}

fn wrote(plan: &AddPlan, id: &str) -> Option<Version> {
    plan.write
        .iter()
        .find(|write| write.id == id)
        .map(|write| write.version)
}

fn replacing<'a>(plan: &'a AddPlan, id: &str) -> Option<&'a str> {
    plan.write
        .iter()
        .find(|write| write.id == id)
        .and_then(|write| write.replacing.as_deref())
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn closure_is_copied_and_a_second_plan_writes_nothing() {
    let child = leaf("led", "0.1.0", "lamp");
    let parent = board("board", "0.1.0", &[("led", "0.1.0")]);
    let catalog = catalog_of(vec![child.clone(), parent.clone()]);
    let first = added(plan(
        &catalog,
        &empty(),
        &ask(&["board"], false, false),
        ADD,
    ));
    assert_eq!(wrote(&first, "board"), Some(parent.version));
    assert_eq!(wrote(&first, "led"), Some(child.version));
    let text = first.lock.clone().unwrap();
    assert_eq!(
        text,
        format!(
            "board 0.1.0 {} {}\nled 0.1.0 {} {}\n",
            catalog.registry(),
            parent.hash,
            catalog.registry(),
            child.hash
        )
    );

    let mut again = empty();
    install(&mut again, &catalog, &parent);
    install(&mut again, &catalog, &child);
    again.lock_text = Some(text);
    let second = added(plan(&catalog, &again, &ask(&["board"], false, false), ADD));
    assert!(second.write.is_empty());
    assert!(second.lock.is_none());
    assert_eq!(second.skip.len(), 2);
}

#[test]
fn a_local_edit_refuses_and_names_the_file() {
    let part = leaf("led", "0.1.0", "lamp");
    let catalog = catalog_of(vec![part.clone()]);
    let mut snapshot = empty();
    put(&mut snapshot, "led", edited(&part));
    let refusal = refusal(plan(&catalog, &snapshot, &ask(&["led"], false, false), ADD));
    assert_eq!(refusal.code.as_str(), "part_differs");
    assert_eq!(
        refusal.message,
        "nothing was written\n`led` is 0.1.0 in the project and differs in `part.jsonc`"
    );
    assert_eq!(refusal.hint, "run `orbe add led --overwrite`");

    let planned = added(plan(&catalog, &snapshot, &ask(&["led"], true, false), ADD));
    assert_eq!(replacing(&planned, "led"), Some("0.1.0 with local edits"));
}

#[test]
fn hints_carry_the_folder_the_run_named() {
    let part = leaf("led", "0.1.0", "lamp");
    let catalog = catalog_of(vec![part.clone()]);
    let mut snapshot = empty();
    put(&mut snapshot, "led", edited(&part));
    let refusal = refusal(plan(
        &catalog,
        &snapshot,
        &ask(&["led"], false, false),
        "orbe add -C proj",
    ));
    assert_eq!(refusal.hint, "run `orbe add -C proj led --overwrite`");
}

#[test]
fn a_file_at_the_target_is_listed_and_only_overwrite_replaces_it() {
    let part = leaf("led", "0.1.0", "lamp");
    let catalog = catalog_of(vec![part]);
    let mut snapshot = empty();
    snapshot
        .targets
        .insert("led".into(), Target::Other("a file"));
    let refused = refusal(plan(&catalog, &snapshot, &ask(&["led"], false, false), ADD));
    assert_eq!(refused.code.as_str(), "part_differs");
    assert_eq!(
        refused.message,
        "nothing was written\nparts/led is a file, not a part folder"
    );
    assert_eq!(refused.hint, "run `orbe add led --overwrite`");

    let planned = added(plan(&catalog, &snapshot, &ask(&["led"], true, false), ADD));
    assert_eq!(replacing(&planned, "led"), Some("a file"));
}

#[test]
fn a_uses_child_at_a_link_is_listed_by_its_own_path() {
    let child = leaf("r", "0.1.0", "r");
    let parent = board("nano", "0.1.0", &[("r", "0.1.0")]);
    let catalog = catalog_of(vec![child, parent]);
    let mut snapshot = empty();
    snapshot.targets.insert("r".into(), Target::Other("a link"));
    let refused = refusal(plan(
        &catalog,
        &snapshot,
        &ask(&["nano"], false, false),
        ADD,
    ));
    assert!(
        refused
            .message
            .contains("parts/r is a link, not a part folder"),
        "{}",
        refused.message
    );
}

#[test]
fn a_folder_without_part_jsonc_says_so_and_overwrite_restores_it() {
    let part = leaf("led", "0.1.0", "lamp");
    let catalog = catalog_of(vec![part]);
    let mut snapshot = empty();
    put(&mut snapshot, "led", BTreeMap::new());
    let refused = refusal(plan(&catalog, &snapshot, &ask(&["led"], false, false), ADD));
    assert_eq!(
        refused.message,
        "nothing was written\nparts/led is a folder with no part.jsonc"
    );
    assert_eq!(refused.hint, "run `orbe add led --overwrite`");
    let planned = added(plan(&catalog, &snapshot, &ask(&["led"], true, false), ADD));
    assert_eq!(wrote(&planned, "led"), Some(v("0.1.0")));
}

#[test]
fn overwrite_takes_the_newest_version_within_the_copys_major() {
    let a = leaf("led", "0.1.0", "a");
    let b = leaf("led", "0.2.0", "b");
    let c = leaf("led", "1.0.0", "c");
    let catalog = catalog_of(vec![a.clone(), b, c]);

    let mut snapshot = empty();
    put(&mut snapshot, "led", edited(&a));
    let planned = added(plan(&catalog, &snapshot, &ask(&["led"], true, false), ADD));
    assert_eq!(wrote(&planned, "led"), Some(v("0.2.0")));
    assert_eq!(replacing(&planned, "led"), Some("0.1.0 with local edits"));
    assert!(
        planned
            .notes
            .iter()
            .any(|note| note
                .contains("`orbe add led@1.0.0 --overwrite` takes it, and tests may change")),
        "{:?}",
        planned.notes
    );
    assert!(
        !planned.notes.iter().any(|note| note == TESTS_MAY_CHANGE),
        "{:?}",
        planned.notes
    );

    let snapshot = with_folder(&a);
    let planned = added(plan(&catalog, &snapshot, &ask(&["led"], true, false), ADD));
    assert_eq!(wrote(&planned, "led"), Some(v("0.2.0")));
    assert_eq!(replacing(&planned, "led"), Some("0.1.0"));
}

#[test]
fn a_version_outside_the_catalog_that_matches_its_lock_is_not_a_local_edit() {
    let carried = leaf("led", "0.1.0", "lamp");
    let held = leaf("led", "0.0.9", "lamp");
    let catalog = catalog_of(vec![carried]);
    let mut snapshot = empty();
    put(&mut snapshot, "led", held.files.clone());
    snapshot.lock.insert(
        "led".into(),
        LockFields {
            id: "led".into(),
            version: "0.0.9".into(),
            registry: catalog.registry().to_string(),
            hash: held.hash.clone(),
        },
    );
    let planned = added(plan(&catalog, &snapshot, &ask(&["led"], true, false), ADD));
    assert_eq!(replacing(&planned, "led"), Some("0.0.9"));

    let mut edited_copy = empty();
    put(&mut edited_copy, "led", edited(&held));
    edited_copy.lock.insert(
        "led".into(),
        LockFields {
            id: "led".into(),
            version: "0.0.9".into(),
            registry: catalog.registry().to_string(),
            hash: held.hash.clone(),
        },
    );
    let planned = added(plan(
        &catalog,
        &edited_copy,
        &ask(&["led"], true, false),
        ADD,
    ));
    assert_eq!(replacing(&planned, "led"), Some("0.0.9 with local edits"));
}

#[test]
fn a_clean_older_copy_is_reported_by_version_only() {
    let a = leaf("led", "0.1.0", "a");
    let b = leaf("led", "0.2.0", "b");
    let catalog = catalog_of(vec![a.clone(), b]);
    let refused = refusal(plan(
        &catalog,
        &with_folder(&a),
        &ask(&["led"], false, false),
        ADD,
    ));
    assert_eq!(
        refused.message,
        "nothing was written\n`led` is 0.1.0 in the project, 0.2.0 in the catalog"
    );
    assert_eq!(refused.hint, "run `orbe add led --overwrite`");
}

#[test]
fn an_edited_older_copy_names_both_versions_and_the_edit() {
    let a = leaf("led", "0.1.0", "a");
    let b = leaf("led", "0.2.0", "b");
    let catalog = catalog_of(vec![a.clone(), b]);
    let mut snapshot = empty();
    put(&mut snapshot, "led", edited(&a));
    let refused = refusal(plan(&catalog, &snapshot, &ask(&["led"], false, false), ADD));
    assert_eq!(
        refused.message,
        "nothing was written\n`led` is 0.1.0 in the project, 0.2.0 in the catalog, and the copy differs from 0.1.0 in `part.jsonc`"
    );
}

#[test]
fn crossing_a_major_version_needs_the_version_named() {
    let old = leaf("led", "0.1.0", "lamp");
    let new = leaf("led", "1.0.0", "lamp v2");
    let catalog = catalog_of(vec![old.clone(), new.clone()]);
    let snapshot = with_folder(&old);

    let planned = added(plan(&catalog, &snapshot, &ask(&["led"], true, false), ADD));
    assert!(planned.write.is_empty());
    assert_eq!(planned.skip, vec![("led".to_string(), old.version)]);

    let refused = refusal(plan(
        &catalog,
        &snapshot,
        &ask(&["led@1.0.0"], false, false),
        ADD,
    ));
    assert_eq!(refused.hint, "run `orbe add led@1.0.0 --overwrite`");

    let planned = added(plan(
        &catalog,
        &snapshot,
        &ask(&["led@1.0.0"], true, false),
        ADD,
    ));
    assert_eq!(wrote(&planned, "led"), Some(new.version));
    assert!(planned.notes.iter().any(|note| note == TESTS_MAY_CHANGE));
}

#[test]
fn two_uses_of_one_id_stop_until_the_clash_is_named() {
    let old = leaf("r", "0.1.0", "old");
    let newer = leaf("r", "0.2.0", "newer");
    let major = leaf("r", "1.0.0", "major");
    let nano = board("nano", "0.1.0", &[("r", "0.1.0")]);
    let uno = board("uno", "0.1.0", &[("r", "0.2.0")]);
    let mega = board("mega", "0.1.0", &[("r", "1.0.0")]);
    let catalog = catalog_of(vec![old, newer.clone(), major.clone(), nano, uno, mega]);
    let refused = refusal(plan(
        &catalog,
        &empty(),
        &ask(&["nano", "uno"], false, false),
        ADD,
    ));
    assert_eq!(refused.code.as_str(), "part_conflict");
    assert_eq!(
        refused.message,
        "`r` is `0.1.0` in `nano` and `0.2.0` in `uno`"
    );
    assert_eq!(refused.hint, "run `orbe add nano uno --overwrite`");

    let planned = added(plan(
        &catalog,
        &empty(),
        &ask(&["nano", "uno"], true, false),
        ADD,
    ));
    assert_eq!(wrote(&planned, "r"), Some(newer.version));
    assert!(
        planned
            .notes
            .iter()
            .any(|note| note.contains("the newer of")),
        "{:?}",
        planned.notes
    );

    let refused = refusal(plan(
        &catalog,
        &empty(),
        &ask(&["nano", "mega"], true, false),
        ADD,
    ));
    assert_eq!(
        refused.message,
        "`r` is `0.1.0` in `nano` and `1.0.0` in `mega`"
    );
    assert_eq!(
        refused.hint,
        "run `orbe add nano mega r@0.1.0 --overwrite` or `orbe add nano mega r@1.0.0 --overwrite`"
    );

    let planned = added(plan(
        &catalog,
        &empty(),
        &ask(&["nano", "mega", "r@1.0.0"], true, false),
        ADD,
    ));
    assert_eq!(wrote(&planned, "r"), Some(major.version));
    assert!(planned.notes.iter().any(|note| note == TESTS_MAY_CHANGE));

    let refused = refusal(plan(
        &catalog,
        &empty(),
        &ask(&["nano", "mega", "r@1.0.0"], false, false),
        ADD,
    ));
    assert_eq!(refused.code.as_str(), "part_conflict");
}

/// The project holds `nano`, which uses `r` 0.2.0. Adding `uno`, which uses `r` 0.1.0, must
/// not downgrade `r` under `nano`.
fn uno_after_nano(nano_r: &str) -> (Catalog, Snapshot) {
    let r1 = leaf("r", "0.1.0", "r1");
    let r2 = leaf("r", "0.2.0", "r2");
    let r3 = leaf("r", "1.0.0", "r3");
    let nano = board("nano", "0.1.0", &[("r", nano_r)]);
    let uno = board("uno", "0.1.0", &[("r", "0.1.0")]);
    let catalog = catalog_of(vec![r1, r2.clone(), r3.clone(), nano.clone(), uno]);
    let mut snapshot = empty();
    install(&mut snapshot, &catalog, &nano);
    let r = if nano_r == "0.2.0" { r2 } else { r3 };
    install(&mut snapshot, &catalog, &r);
    (catalog, snapshot)
}

/// The project already holds the newer `r`, so adding `uno` keeps it, with or without
/// `--overwrite`, and only the note differs.
#[test]
fn a_part_in_the_project_keeps_the_child_version_it_uses() {
    let (catalog, snapshot) = uno_after_nano("0.2.0");
    let planned = added(plan(&catalog, &snapshot, &ask(&["uno"], false, false), ADD));
    assert_eq!(wrote(&planned, "uno"), Some(v("0.1.0")));
    assert_eq!(wrote(&planned, "r"), None);
    assert!(planned.skip.contains(&("r".to_string(), v("0.2.0"))));
    assert_eq!(
        planned.notes,
        [
            "kept `r` 0.2.0, which the project holds, the newer of `0.1.0` in `uno` and `0.2.0` in `nano`, which the project holds"
        ]
    );

    let planned = added(plan(&catalog, &snapshot, &ask(&["uno"], true, false), ADD));
    assert_eq!(wrote(&planned, "uno"), Some(v("0.1.0")));
    assert_eq!(wrote(&planned, "r"), None);
    assert!(planned.skip.contains(&("r".to_string(), v("0.2.0"))));
    assert!(
        planned.notes.iter().any(|note| note
            == "took `r` 0.2.0, the newer of `0.1.0` in `uno` and `0.2.0` in `nano`, which the project holds"),
        "{:?}",
        planned.notes
    );
    let lock = planned.lock.unwrap();
    assert!(lock.contains("\nr 0.2.0 "), "{lock}");
}

/// Once `--overwrite` settled the clash, every later plain add and `--diff` of either part
/// plans without it, and so does re-adding a deleted folder, which is check's plain hint.
#[test]
fn a_clash_settled_by_overwrite_stays_settled() {
    let (catalog, mut snapshot) = uno_after_nano("0.2.0");
    let uno = catalog.get("uno", v("0.1.0")).unwrap().clone();
    install(&mut snapshot, &catalog, &uno);
    snapshot.lock_text = Some(project::render_lock(&snapshot.lock));
    for ids in [&["uno"][..], &["nano"], &["r"], &["nano", "uno"]] {
        let planned = added(plan(&catalog, &snapshot, &ask(ids, false, false), ADD));
        assert!(planned.write.is_empty(), "{ids:?}");
        assert!(planned.lock.is_none(), "{ids:?}");
    }
    let Plan::Diff(diff) = plan(&catalog, &snapshot, &ask(&["uno"], false, true), ADD) else {
        panic!("expected a diff");
    };
    assert!(
        diff.notes
            .iter()
            .any(|note| note.starts_with("kept `r` 0.2.0")),
        "{:?}",
        diff.notes
    );
    assert!(
        diff.notes.iter().any(|note| note == "nothing would change"),
        "{:?}",
        diff.notes
    );

    snapshot.targets.remove("nano");
    let planned = added(plan(
        &catalog,
        &snapshot,
        &ask(&["nano"], false, false),
        ADD,
    ));
    assert_eq!(wrote(&planned, "nano"), Some(v("0.1.0")));
    assert_eq!(wrote(&planned, "r"), None);

    let refused = refusal(plan(
        &catalog,
        &snapshot,
        &ask(&["uno", "r@0.1.0"], false, false),
        ADD,
    ));
    assert_eq!(refused.code.as_str(), "part_conflict");
}

#[test]
fn a_child_version_in_another_major_stops_until_one_is_named() {
    let (catalog, snapshot) = uno_after_nano("1.0.0");
    let refused = refusal(plan(&catalog, &snapshot, &ask(&["uno"], true, false), ADD));
    assert_eq!(refused.code.as_str(), "part_conflict");
    assert_eq!(
        refused.message,
        "`r` is `0.1.0` in `uno` and `1.0.0` in `nano`, which the project holds"
    );
    assert_eq!(
        refused.hint,
        "run `orbe add uno r@0.1.0 --overwrite` or `orbe add uno r@1.0.0 --overwrite`"
    );
    let planned = added(plan(
        &catalog,
        &snapshot,
        &ask(&["uno", "r@1.0.0"], true, false),
        ADD,
    ));
    assert!(planned.skip.contains(&("r".to_string(), v("1.0.0"))));
    assert!(planned.notes.iter().any(|note| note == TESTS_MAY_CHANGE));
}

#[test]
fn an_unknown_id_suggests_a_catalog_id_and_a_registry_id_is_refused() {
    let catalog = catalog_of(vec![leaf("led-5mm", "0.1.0", "lamp")]);
    let refused = refusal(plan(
        &catalog,
        &empty(),
        &ask(&["led-5m"], false, false),
        ADD,
    ));
    assert_eq!(refused.code.as_str(), "unknown_part");
    assert!(refused.hint.contains("led-5mm"), "{}", refused.hint);
    let refused = refusal(plan(
        &catalog,
        &empty(),
        &ask(&["@acme/led-5mm"], false, false),
        ADD,
    ));
    assert_eq!(refused.code.as_str(), "registry");
    assert!(
        refused
            .message
            .contains("v1 reads only the official catalog"),
        "{}",
        refused.message
    );
}

#[test]
fn wanted_reaches_every_version_of_every_child_and_nothing_else() {
    let catalog = catalog_of(vec![
        leaf("r", "0.1.0", "r"),
        leaf("c", "0.1.0", "c"),
        leaf("other", "0.1.0", "o"),
        board("nano", "0.1.0", &[("r", "0.1.0")]),
        board("nano", "0.2.0", &[("c", "0.1.0")]),
    ]);
    let ids: Vec<String> = wanted(&catalog, &ask(&["nano", "nope"], false, false))
        .into_iter()
        .collect();
    assert_eq!(ids, ["c", "nano", "r"]);
}

#[test]
fn diff_states_whether_a_test_can_change() {
    let patch_old = leaf("led", "0.1.0", "lamp");
    let patch = leaf("led", "0.1.1", "lamp patched");
    let minor = leaf("led", "0.2.0", "lamp minor");
    let major = leaf("led", "1.0.0", "lamp major");
    let catalog = catalog_of(vec![patch_old.clone(), patch, minor, major]);
    let snapshot = with_folder(&patch_old);

    let diff = |ids: &[&str]| match plan(&catalog, &snapshot, &ask(ids, false, true), ADD) {
        Plan::Diff(diff) => diff,
        other => panic!("{}", name(&other)),
    };
    let patched = diff(&["led@0.1.1"]);
    assert!(
        patched
            .notes
            .iter()
            .any(|note| note.contains("a patch version cannot change a passing test")),
        "{:?}",
        patched.notes
    );
    assert!(patched.files.contains("-  \"title\": \"lamp\""));
    assert!(patched.files.contains("+  \"title\": \"lamp patched\""));
    assert!(
        diff(&["led@0.2.0"])
            .notes
            .iter()
            .any(|note| note.contains("a minor version cannot change a passing test"))
    );
    assert!(
        diff(&["led@1.0.0"])
            .notes
            .iter()
            .any(|note| note.contains("a major version may change a passing test"))
    );
}

#[test]
fn relock_refuses_a_sealed_edit_and_records_a_composite_edit() {
    let sealed = leaf("led", "0.1.0", "lamp");
    let open = board("board", "0.1.0", &[]);
    let catalog = catalog_of(vec![sealed.clone(), open.clone()]);

    let mut snapshot = empty();
    put(&mut snapshot, "led", edited(&sealed));
    let refused = refusal(plan(&catalog, &snapshot, &relock(), ADD));
    assert_eq!(refused.code.as_str(), "part_modified");
    assert!(
        refused.message.contains("`part.jsonc`"),
        "{}",
        refused.message
    );
    assert!(
        refused.hint.contains("orbe add led --overwrite"),
        "{}",
        refused.hint
    );

    let mut board_files = open.files.clone();
    board_files.insert("note.txt".into(), b"local".to_vec());
    let folder = Folder::from_files(board_files);
    let hash = folder.hash.clone();
    assert_ne!(hash, open.hash);
    let mut snapshot = with_folder(&sealed);
    snapshot
        .targets
        .insert("board".into(), Target::Folder(folder));
    snapshot.lock_text = Some("mangled\n".into());
    let planned = match plan(&catalog, &snapshot, &relock(), ADD) {
        Plan::Relock(plan) => plan,
        other => panic!("{}", name(&other)),
    };
    assert_eq!(planned.modified, vec!["board".to_string()]);
    let text = planned.lock.unwrap();
    assert!(text.contains(&hash), "{text}");
    assert!(text.contains(&sealed.hash), "{text}");
    assert!(text.starts_with("board "), "{text}");
}

#[test]
fn a_bad_version_token_is_usage() {
    let catalog = catalog_of(vec![leaf("led", "0.1.0", "lamp")]);
    match plan(&catalog, &empty(), &ask(&["led@1"], false, false), ADD) {
        Plan::Usage(message) => assert!(message.contains("three numbers"), "{message}"),
        other => panic!("{}", name(&other)),
    }
}
