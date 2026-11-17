//! A part opened by a `./` path is the user's file. An older format names that file and says
//! how to update it. It never names `orbe add`, which would write `parts/<id>/` and leave the
//! file as it is.

use std::fs;

use orbe_scene::{Code, Files};

fn owned_hint() -> String {
    let current = orbe_scene::FORMATS.iter().copied().max().unwrap();
    format!(
        "update this file to format {current}, including its model. if it copies a catalog part, point the scene at that id instead of this path"
    )
}

fn part_text(part_line: &str) -> String {
    let part = if part_line.is_empty() {
        String::new()
    } else {
        format!("\n  {part_line}")
    };
    format!(
        r#"{{
  "format": 0,{part}
  "pins": {{ "a": [1], "b": [2] }}
}}
"#
    )
}

fn check(part_line: &str) -> orbe_scene::Diagnostic {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("mine")).unwrap();
    fs::write(dir.path().join("mine/part.jsonc"), part_text(part_line)).unwrap();
    let scene_format = orbe_scene::FORMATS.iter().copied().max().unwrap();
    fs::write(
        dir.path().join("scene.jsonc"),
        format!(
            "{{\n  \"format\": {scene_format},\n  \"parts\": {{ \"lamp\": {{ \"type\": \"./mine/part.jsonc\" }} }}\n}}\n"
        ),
    )
    .unwrap();
    let Err(mut found) = orbe_scene::read(&Files::disk(), &dir.path().join("scene.jsonc")) else {
        panic!("an older path part checked");
    };
    assert_eq!(found.len(), 1, "{found:#?}");
    found.remove(0)
}

#[test]
fn a_path_part_at_an_older_format_is_updated_in_place() {
    let hint = owned_hint();
    for (part_line, label) in [
        ("\"part\": \"led-5mm\",", "catalog id in the file"),
        ("\"part\": \"my-lamp\",", "a name that is not a catalog id"),
        ("", "no part key"),
    ] {
        let found = check(part_line);
        assert_eq!(found.code, Code::PartFormat, "{label}");
        assert_eq!(
            found.message, "mine/part.jsonc is written for an older orbe",
            "{label}"
        );
        assert_eq!(found.hint, hint, "{label}");
        assert!(
            !found.hint.contains("orbe add"),
            "{label} printed an add: {}",
            found.hint
        );
    }
}
