//! `orbe add` copies a catalog part into `parts/<id>/` and leaves that copy alone.

mod common;

use orbe_scene::Request;

fn ask(ids: &[&str]) -> Request {
    Request {
        ids: ids.iter().map(|id| (*id).to_string()).collect(),
    }
}

#[test]
fn an_existing_folder_is_part_exists_and_nothing_is_written() {
    let project = common::project("nano-button");
    let path = project.path().join("parts/led-5mm/part.jsonc");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("// edited\n");
    std::fs::write(&path, &text).unwrap();
    let reply =
        orbe_scene::add_parts(Some(project.path()), &ask(&["led-5mm", "resistor-axial"])).unwrap();
    assert_eq!(reply.code(), 1, "{}", reply.text());
    let shown = reply.text();
    assert!(shown.starts_with("error[part_exists]:"), "{shown}");
    assert!(
        shown.contains("parts/led-5mm is already led-5mm "),
        "{shown}"
    );
    assert!(
        shown.contains("parts/resistor-axial is already resistor-axial "),
        "{shown}"
    );
    assert!(shown.contains("remove them and run `orbe add"), "{shown}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert!(!project.path().join("parts.lock").exists());
    assert!(!project.path().join("parts/.orbe-add").exists());
}
