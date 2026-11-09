//! Parts plugged into a breadboard: where they sit, which holes the strips join, and that a
//! circuit on a breadboard is the circuit it is wired directly.

mod common;

use std::fs;

use orbe_scene::{Files, Netlist, Placement};

fn read(dir: &std::path::Path) -> Netlist {
    orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc"))
        .unwrap_or_else(|e| panic!("{}: {e:#?}", dir.display()))
}

/// Each Block's scene on its breadboard, and the same circuit wired pin to pin as
/// `fixtures/wired/<block>.jsonc` holds it, give the engine the same netlist, byte for byte. So
/// the same nets run the same simulation, and the board's empty holes add nothing to solve.
#[test]
fn each_block_on_its_breadboard_runs_the_netlist_of_its_circuit_wired_directly() {
    for block in ["nano-button-led", "nano-led-fade", "uno-button-led"] {
        let dir = tempfile::tempdir().unwrap();
        common::copy_dir(&common::repo().join("blocks").join(block), dir.path());
        let on_breadboard = read(dir.path());
        let wired = common::fixture("wired").join(format!("{block}.jsonc"));
        fs::copy(&wired, dir.path().join("scene.jsonc")).unwrap();
        let direct = read(dir.path());
        assert!(
            on_breadboard.json == direct.json,
            "{block}: the netlists differ"
        );
        assert_eq!(
            on_breadboard.view.circuit_hash, direct.view.circuit_hash,
            "{block}"
        );
        let parts = |n: &Netlist| {
            n.view
                .instances
                .iter()
                .map(|i| i.part.clone())
                .collect::<Vec<_>>()
        };
        assert!(
            parts(&on_breadboard).contains(&"breadboard-400".to_string()),
            "{block}"
        );
        assert!(
            !parts(&direct).contains(&"breadboard-400".to_string()),
            "{block}"
        );
    }
}

fn scene(layout: &str, wires: &str) -> tempfile::TempDir {
    let dir = common::project("nano-button");
    let text = format!(
        r#"{{
  "format": 1,
  "parts": {{
    "bb": {{ "type": "breadboard-830" }},
    "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }},
    "r1": {{ "type": "resistor-axial", "resistance": "220" }},
    "led1": {{ "type": "led-5mm", "color": "red" }},
  }},
  "layout": {{ "bb": {{ "at": [10, 20, 0], "rotate": [0, 0, 90] }}, {layout} }},
  "wires": [{wires}],
}}
"#
    );
    fs::write(dir.path().join("scene.jsonc"), text).unwrap();
    dir
}

fn placed(netlist: &Netlist, name: &str) -> Placement {
    netlist
        .view
        .instances
        .iter()
        .find(|i| i.name == name)
        .and_then(|i| i.placement)
        .unwrap_or_else(|| panic!("`{name}` has no placement"))
}

fn net(netlist: &Netlist, part: &str, pin: &str) -> Option<String> {
    netlist.refs.parts[part].pins[pin].clone()
}

/// A strip joins five holes of a column on one side of the centre channel, and a rail runs the
/// board's whole length. A leg joins the strip of the hole it lands in.
#[test]
fn a_strip_joins_its_holes_and_a_leg_joins_its_strip() {
    let dir = scene(
        r#""nano": { "on": { "D13": "bb.d1" } }, "r1": { "on": { "a": "bb.a20" } }, "led1": { "on": { "anode": "bb.c24" } }"#,
        r#"["bb.c1", "bb.e20"], ["bb.a25", "bb.W61"], ["bb.c14", "bb.W3"]"#,
    );
    let netlist = read(dir.path());
    let d13 = net(&netlist, "nano", "D13");
    assert_eq!(d13.as_deref(), Some("nano__D13"));
    for hole in ["a1", "b1", "c1", "e1", "a20", "e20"] {
        assert_eq!(net(&netlist, "bb", hole), d13, "{hole}");
    }
    assert_eq!(net(&netlist, "r1", "a"), d13);
    assert_eq!(
        net(&netlist, "bb", "f20"),
        None,
        "the channel parts the column"
    );
    assert_eq!(net(&netlist, "bb", "a21"), None, "the next column is apart");
    assert_eq!(net(&netlist, "r1", "b"), net(&netlist, "led1", "anode"));
    assert_eq!(
        net(&netlist, "r1", "b").as_deref(),
        Some("led1__anode"),
        "a hole names no net"
    );
    let gnd = net(&netlist, "nano", "GND");
    assert_eq!(net(&netlist, "led1", "cathode"), gnd, "through the rail");
    assert_eq!(
        net(&netlist, "bb", "W33"),
        gnd,
        "the rail runs the whole length"
    );
    assert!(
        netlist
            .view
            .wires
            .iter()
            .any(|w| w.kind == "contact" && w.ends == ["led1.anode", "bb.c24"])
    );
    assert!(
        netlist
            .view
            .wires
            .iter()
            .any(|w| w.kind == "contact" && w.ends == ["nano.j2.1", "bb.d1"])
    );
}

/// A part sits with its seating plane on the board's top, turned with the board, and an LED,
/// whose legs are longer than a hole is deep, stands on them. The Nano's two header rows,
/// 15.24 mm apart, land in rows d and h across the channel.
#[test]
fn a_part_on_a_breadboard_sits_on_its_holes() {
    let dir = scene(
        r#""nano": { "on": { "D13": "bb.d1" } }, "r1": { "on": { "a": "bb.a20" } }, "led1": { "on": { "anode": "bb.c24" }, "rotate": [0, 0, 180] }"#,
        r#"["bb.c1", "bb.e20"], ["bb.a25", "bb.W61"], ["bb.c14", "bb.W3"]"#,
    );
    let netlist = read(dir.path());
    let close = |got: Placement, at: [f64; 3], rotate: f64| {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(
            (0..3).all(|k| near(got.at[k], at[k])) && near(got.rotate[2], rotate),
            "{got:?} against {at:?} turned {rotate}"
        );
    };
    // The 830's column c sits at x = (c - 32) * 2.54 and row d at y = -6.35, in its frame; the
    // board turns a quarter about Z at (10, 20).
    let (x, y) = ((8.0 - 32.0) * 2.54, -6.35 + 7.62);
    close(placed(&netlist, "nano"), [10.0 - y, 20.0 + x, 8.5], 90.0);
    let (x, y) = ((22.0 - 32.0) * 2.54, -13.97);
    close(placed(&netlist, "r1"), [10.0 - y, 20.0 + x, 8.5], 90.0);
    let (x, y) = ((23.5 - 32.0) * 2.54, -8.89);
    close(
        placed(&netlist, "led1"),
        [10.0 - y, 20.0 + x, 8.5 + 27.4 - 8.0],
        -90.0,
    );
}
