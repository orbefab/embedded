//! A passive buzzer reads the frequency of `tone()` on D8, and 0 during the rest.
//! Wired straight across the pin, the coil's 42 ohm draws more than the pin's 40 mA and
//! the run warns. A series resistor keeps that warning away and the frequency stays.

use std::path::{Path, PathBuf};

use orbe_session::{Command, Files, Live};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalog() -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}/"), out);
            } else {
                out.push((format!("{prefix}{name}"), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo().join("parts"), "", &mut files);
    files
}

fn opened(wires: &str) -> Live {
    let image =
        std::fs::read(repo().join("packages/engine/tests/firmware/sketch/tone.hex")).unwrap();
    let block = std::fs::read(repo().join("blocks/nano-buzzer/block.jsonc")).unwrap();
    let scene = format!(
        r#"{{
  "format": 1,
  "parts": {{
    "nano": {{ "type": "nano-board", "firmware": "firmware/tone.hex" }},
    "r1": {{ "type": "resistor-axial", "resistance": "220" }},
    "bz": {{ "type": "buzzer-12mm" }}
  }},
  "wires": [{wires}]
}}
"#
    );
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        ("firmware/tone.hex".to_string(), image),
        ("scene.jsonc".to_string(), scene.into_bytes()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let open = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(open.ok(), "{}", open.text());
    live
}

fn run(live: &mut Live, line: &str) -> orbe_session::Reply {
    live.run(&Command::parse(line, Path::new(".")).unwrap().unwrap())
}

fn hz(live: &mut Live) -> f64 {
    let reply = run(live, "get @bz.hz");
    assert!(reply.ok(), "{}", reply.text());
    reply.json()["data"]["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{}", reply.text()))
}

fn warnings(live: &mut Live) -> Vec<String> {
    let json = run(live, "snapshot").json();
    json["data"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("{json}"))
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_series_resistor_plays_440_then_goes_quiet_without_a_warning() {
    let mut live = opened(r#"["nano.D8", "r1.a"], ["r1.b", "bz.1"], ["bz.2", "nano.GND"]"#);
    assert!(run(&mut live, "step 100ms").ok());
    let sounding = hz(&mut live);
    assert!(
        (420.0..460.0).contains(&sounding),
        "440 Hz note read {sounding}"
    );
    assert!(
        !warnings(&mut live)
            .iter()
            .any(|code| code.starts_with("rating")),
        "{:?}",
        warnings(&mut live)
    );
    assert!(run(&mut live, "step 600ms").ok());
    let quiet = hz(&mut live);
    assert!(quiet < 1.0, "rest read {quiet}");
}

#[test]
fn a_pin_wired_straight_across_the_coil_still_plays_and_warns() {
    let mut live = opened(r#"["nano.D8", "bz.1"], ["bz.2", "nano.GND"]"#);
    assert!(run(&mut live, "step 100ms").ok());
    let sounding = hz(&mut live);
    assert!(
        (420.0..460.0).contains(&sounding),
        "direct drive read {sounding}"
    );
    let found = warnings(&mut live);
    assert!(
        found.iter().any(|code| code == "rating.current"),
        "expected rating.current, got {found:?}"
    );
}
