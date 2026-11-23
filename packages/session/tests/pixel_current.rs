//! Eight SK6812MINI-HS pixels on a Nano's 5V, and the same eight plus eight more on the same
//! DIN. Each channel draws 12 mA at 255 and each pixel draws 1 mA idle, so eight at full white
//! are 296 mA and stay under the USB supply's 500 mA. Sixteen pass it. The warning is the
//! supply's own `rating.current`.

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

/// `extra` single pixels share the stick's DIN, so each latches the frame's first pixel.
fn scene(extra: usize) -> String {
    let mut parts = String::from(
        r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "firmware/frame.hex" },
    "r1": { "type": "resistor-axial", "resistance": "470" },
    "stick": { "type": "neopixel-stick-8" }"#,
    );
    for n in 1..=extra {
        parts.push_str(&format!(
            ",\n    \"led{n}\": {{ \"type\": \"sk68xxmini-hs\" }}"
        ));
    }
    parts.push_str(
        r#"
  },
  "wires": [
    ["nano.D8", "r1.a"], ["r1.b", "stick.DIN"],
    ["nano.5V", "stick.5V"], ["nano.GND", "stick.GND"]"#,
    );
    for n in 1..=extra {
        parts.push_str(&format!(
            ",\n    [\"r1.b\", \"led{n}.DIN\"], [\"nano.5V\", \"led{n}.VDD\"], [\"nano.GND\", \"led{n}.GND\"]"
        ));
    }
    parts.push_str("\n  ]\n}\n");
    parts
}

fn opened(hex: &str, extra: usize) -> Live {
    let image =
        std::fs::read(repo().join(format!("packages/engine/tests/firmware/sketch/{hex}.hex")))
            .unwrap();
    let block = std::fs::read(repo().join("blocks/nano-button-led/block.jsonc")).unwrap();
    let files = Files::memory([
        ("block.jsonc".to_string(), block),
        ("firmware/frame.hex".to_string(), image),
        ("scene.jsonc".to_string(), scene(extra).into_bytes()),
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

fn usb_amps(live: &mut Live) -> f64 {
    let reply = run(live, "get @nano.usb.i");
    assert!(reply.ok(), "{}", reply.text());
    reply.json()["data"]["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{}", reply.text()))
        .abs()
}

fn rating_current(live: &mut Live) -> Vec<String> {
    let json = run(live, "snapshot").json();
    json["data"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("{json}"))
        .iter()
        .filter(|d| d["code"] == "rating.current")
        .map(|d| d["message"].as_str().unwrap().to_string())
        .collect()
}

fn expect_colour(live: &mut Live, target: &str, rgb: &str) {
    let line = format!("expect {target} {rgb}");
    let reply = run(live, &line);
    assert!(reply.ok(), "{line}: {}", reply.text());
}

/// The stick `step` after open, once the sketch has latched, with the USB current and the
/// current warnings.
fn settled(hex: &str, extra: usize, rgb: &str, step: &str) -> (f64, Vec<String>) {
    let mut live = opened(hex, extra);
    let stepped = run(&mut live, step);
    assert!(stepped.ok(), "{}", stepped.text());
    for n in 1..=8 {
        expect_colour(&mut live, &format!("@stick.px{n}"), rgb);
    }
    for n in 1..=extra {
        expect_colour(&mut live, &format!("@led{n}"), rgb);
    }
    (usb_amps(&mut live), rating_current(&mut live))
}

#[test]
fn eight_white_pixels_stay_under_the_usb_rating_and_sixteen_go_over() {
    let (white, white_over) = settled("ws2812-white", 0, "rgb(255,255,255)", "step 20ms");
    let (dim, dim_over) = settled("ws2812-dim", 0, "rgb(0,8,0)", "step 20ms");
    assert!(
        white_over.is_empty(),
        "eight full white pixels draw {white} A and warn {white_over:?}"
    );
    assert!(
        dim_over.is_empty(),
        "eight dim pixels draw {dim} A and warn {dim_over:?}"
    );
    let rise = white - dim;
    assert!(
        (0.25..0.31).contains(&rise) && (0.28..0.45).contains(&white) && dim < 0.15,
        "white {white} A, dim {dim} A, rise {rise} A"
    );

    // The USB supply's 0.5 A is a peak. The Schottky's 0.5 A is a mean with a 55 ms time
    // constant, which about 0.62 A crosses `55 ms * ln(0.62 / 0.12)`, about 90 ms, after it
    // starts.
    let (full, over) = settled("ws2812-white", 8, "rgb(255,255,255)", "step 120ms");
    assert!(full > 0.5, "sixteen pixels draw {full} A");
    for part in [
        "#nano__usb__host.i (nano.usb)",
        "#nano__diode.i (nano.diode)",
    ] {
        assert!(
            over.iter().any(|message| message.starts_with(part)),
            "{part} missing from {over:?} at {full} A"
        );
    }
}
