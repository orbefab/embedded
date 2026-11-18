//! Parts plugged into a breadboard: where they sit, which holes the strips join, and that a
//! circuit on a breadboard is the circuit it is wired directly.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use orbe_scene::{Code, Diagnostic, Files, Netlist, Placement};

fn read(dir: &std::path::Path) -> Netlist {
    orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc"))
        .unwrap_or_else(|e| panic!("{}: {e:#?}", dir.display()))
}

/// Each Block's scene on its breadboard, and the same circuit wired pin to pin as
/// `fixtures/wired/<block>.jsonc` holds it, give the engine the same netlist, byte for byte. So
/// the same nets run the same simulation, and the board's empty holes add nothing to solve.
#[test]
fn each_block_on_its_breadboard_runs_the_netlist_of_its_circuit_wired_directly() {
    for block in [
        "nano-button-led",
        "nano-led-fade",
        "nano-light-comparator",
        "uno-button-led",
    ] {
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
        let on_board = parts(&on_breadboard);
        let direct_parts = parts(&direct);
        assert!(
            on_board.iter().any(|part| part.starts_with("breadboard-")),
            "{block} has no breadboard"
        );
        assert!(
            !direct_parts
                .iter()
                .any(|part| part.starts_with("breadboard-")),
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
        r#"["bb.c1", "bb.e20"], ["bb.a25", "bb.X61"], ["bb.c14", "bb.X3"]"#,
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
        net(&netlist, "bb", "X33"),
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
        r#"["bb.c1", "bb.e20"], ["bb.a25", "bb.X61"], ["bb.c14", "bb.X3"]"#,
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

/// A half-size board at the origin with `parts` beside it, laid out and wired as given.
fn on_400(parts: &str, layout: &str, wires: &str) -> tempfile::TempDir {
    let dir = common::project("nano-button");
    let text = format!(
        r#"{{
  "format": 1,
  "parts": {{
    "bb": {{ "type": "breadboard-400" }},
    "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }},
    {parts}
  }},
  "layout": {{ "bb": {{ "at": [0, 0, 0] }}, {layout} }},
  "wires": [{wires}],
}}
"#
    );
    fs::write(dir.path().join("scene.jsonc"), text).unwrap();
    dir
}

/// The holes each pin of `part` sits in, by its contact wires.
fn holes_of(netlist: &Netlist, part: &str, pin: &str) -> Vec<String> {
    let me = format!("{part}.{pin}");
    let mut holes: Vec<String> = netlist
        .view
        .wires
        .iter()
        .filter(|w| w.kind == "contact" && w.ends[0] == me)
        .map(|w| w.ends[1].trim_start_matches("bb.").to_string())
        .collect();
    holes.sort();
    holes
}

/// The hints of the findings that stop a scene, one per finding, without repeats.
fn hints(dir: &tempfile::TempDir) -> Vec<String> {
    let found = orbe_scene::read(&Files::disk(), &dir.path().join("scene.jsonc"))
        .err()
        .expect("the scene has findings");
    let mut hints: Vec<String> = found.into_iter().map(|d| d.hint).collect();
    hints.dedup();
    hints
}

/// A pin on two legs, as a switch's `a` or a DIP's `GND`, goes in by its lowest lead, and the
/// part's other legs follow from it. Naming the hole another of its legs sat in moves the part.
#[test]
fn a_pin_on_two_legs_goes_in_by_its_lowest_lead() {
    let switch = |hole: &str| {
        on_400(
            r#""btn1": { "type": "tactile-switch-6mm" },"#,
            &format!(r#""btn1": {{ "on": {{ "a": "bb.{hole}" }}, "rotate": [0, 0, 90] }}"#),
            r#"["bb.j22", "nano.D2"], ["bb.j20", "nano.5V"]"#,
        )
    };
    let across = read(switch("f22").path());
    assert_eq!(
        holes_of(&across, "btn1", "a"),
        ["e22", "f22"],
        "lead 1 in f22"
    );
    assert_eq!(holes_of(&across, "btn1", "b"), ["e20", "f20"]);
    assert_eq!(net(&across, "btn1", "a"), net(&across, "nano", "D2"));
    let moved = read(switch("e22").path());
    assert_eq!(
        holes_of(&moved, "btn1", "a"),
        ["b22", "e22"],
        "lead 1 in e22, so lead 2 lands in b22"
    );
    assert_ne!(net(&moved, "btn1", "a"), net(&moved, "nano", "D2"));

    let dip = |hole: &str| {
        on_400(
            r#""u1": { "type": "atmega328p-dip28", "firmware": "build/button.ino.hex" },"#,
            &format!(r#""u1": {{ "on": {{ "GND": "bb.{hole}" }} }}"#),
            "",
        )
    };
    let chip = read(dip("e16").path());
    assert_eq!(
        holes_of(&chip, "u1", "GND"),
        ["e16", "f15"],
        "lead 8 in e16, lead 22 across the channel"
    );
    let moved = read(dip("f15").path());
    assert_eq!(holes_of(&moved, "u1", "GND"), ["f15", "i14"]);
}

/// A leg that misses gets a hint found by trying it: a quarter turn from the same hole when one
/// seats every leg, else a nearby hole and turn that does, else a plain no.
#[test]
fn a_missed_leg_s_hint_names_only_a_placement_that_seats_the_part() {
    let led = on_400(
        r#""led1": { "type": "led-5mm", "color": "red" },"#,
        r#""led1": { "on": { "anode": "bb.j30" } }"#,
        "",
    );
    assert_eq!(
        hints(&led),
        [
            "turn `led1` with \"rotate\": [0, 0, 180], and every leg lands in a hole with no strip joining two of its pins"
        ],
        "off the end, so it turns back along the row"
    );
    let resistor = on_400(
        r#""r1": { "type": "resistor-axial", "resistance": "220" },"#,
        r#""r1": { "on": { "a": "bb.d15" }, "rotate": [0, 0, 45] }"#,
        "",
    );
    assert_eq!(
        hints(&resistor),
        [
            "turn `r1` with \"rotate\": [0, 0, 0], and every leg lands in a hole with no strip joining two of its pins"
        ],
        "a quarter turn on the board's own grid, not 45 plus 90"
    );
    let nano = |hole: &str| {
        on_400(
            "",
            &format!(r#""nano": {{ "on": {{ "D13": "bb.{hole}" }} }}"#),
            "",
        )
    };
    assert_eq!(
        hints(&nano("a1")),
        [
            "no quarter turn seats `nano` from bb.a1; plug it in as \"nano\": { \"on\": { \"D13\": \"bb.b1\" }, \"rotate\": [0, 0, 0] }"
        ]
    );
    assert_eq!(
        hints(&nano("Z29")),
        [
            "no quarter turn seats `nano` from bb.Z29, nor from a hole within 10.16 mm of it; plug `nano.D13` into a hole where `nano`'s other legs land in holes too"
        ]
    );
    let suggested = on_400("", r#""nano": { "on": { "D13": "bb.b1" } }"#, "");
    read(suggested.path());
}

fn findings(dir: &Path) -> Vec<Diagnostic> {
    orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc"))
        .err()
        .unwrap_or_default()
}

fn layout_of(dir: &Path) -> Vec<Diagnostic> {
    findings(dir)
        .into_iter()
        .filter(|d| d.code == Code::Layout)
        .collect()
}

/// The byte offset of a 1-based line and character column.
fn at_line_col(text: &str, line: usize, column: usize) -> usize {
    let mut rest = text;
    let mut offset = 0;
    for _ in 1..line {
        let step = rest.find('\n').expect("the finding's line") + 1;
        offset += step;
        rest = &rest[step..];
    }
    offset
        + rest
            .chars()
            .take(column - 1)
            .map(char::len_utf8)
            .sum::<usize>()
}

/// Move each shared lead to the hole its hint names. The hints of one check name different
/// holes, and the scene checks once every one of them is applied.
fn apply_every_hint(dir: &Path) -> Vec<String> {
    let found = findings(dir);
    let layout: Vec<&Diagnostic> = found.iter().filter(|d| d.code == Code::Layout).collect();
    assert!(!layout.is_empty(), "no shared hole:\n{found:#?}");
    let mut suggested = Vec::new();
    let mut edits: BTreeMap<String, Vec<(usize, usize, String)>> = BTreeMap::new();
    for d in &layout {
        let hole = d
            .hint
            .split("such as ")
            .nth(1)
            .unwrap_or_else(|| panic!("no hole in {}", d.hint));
        suggested.push(hole.to_string());
        let path = dir.join(&d.file);
        let text = fs::read_to_string(&path).unwrap();
        let offset = at_line_col(&text, d.line, d.column);
        let literal = &text[offset..];
        assert!(
            literal.starts_with('"'),
            "{}:{}:{} is not a wire end",
            d.file,
            d.line,
            d.column
        );
        let end = literal[1..].find('"').expect("the end of the wire ref") + 1;
        edits
            .entry(d.file.clone())
            .or_default()
            .push((offset + 1, offset + end, hole.to_string()));
    }
    let mut unique = suggested.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        suggested.len(),
        "two hints name one hole: {suggested:?}"
    );
    for (file, mut spans) in edits {
        spans.sort_by_key(|span| std::cmp::Reverse(span.0));
        let path = dir.join(file);
        let mut text = fs::read_to_string(&path).unwrap();
        for (start, end, hole) in spans {
            text.replace_range(start..end, &hole);
        }
        fs::write(path, text).unwrap();
    }
    orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc")).unwrap_or_else(|found| {
        panic!(
            "after every hint:\n{}",
            found
                .iter()
                .map(|d| format!("{d}\n  hint was applied from {suggested:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    suggested
}

/// One check's hints name different holes, including the two `e31` ends and the two `f31`
/// ends, and both orders of a wire whose other end is the nearest hole.
#[test]
fn applying_every_hint_in_one_check_leaves_each_hole_with_one_lead() {
    let case = common::repo().join("packages/scene/tests/bad/a_hole_holds_one_lead");
    let dir = common::overlaid(&case);
    let holes = apply_every_hint(dir.path());
    assert_eq!(holes.iter().filter(|hole| *hole == "bb.d31").count(), 1);
    assert_eq!(holes.iter().filter(|hole| *hole == "bb.g31").count(), 1);

    for (first, second) in [("X22", "X21"), ("X21", "X22")] {
        let dir = on_400(
            r#""r2": { "type": "resistor-axial", "resistance": "220" },"#,
            r#""r2": { "on": { "a": "bb.b22" }, "rotate": [0, 0, -90] }"#,
            &format!(r#"["bb.{first}", "bb.{second}"]"#),
        );
        let holes = apply_every_hint(dir.path());
        assert_eq!(holes.len(), 1, "{first} then {second}: {holes:?}");
        assert_ne!(holes[0], format!("bb.{first}"));
        assert_ne!(holes[0], format!("bb.{second}"));
    }
}

fn loose(kind: &str, parts: &str, layout: &str, wires: &str, ground: &str) -> tempfile::TempDir {
    let dir = common::project("nano-button");
    let text = format!(
        r#"{{
  "format": 1,
  "parts": {{
    "bb": {{ "type": "{kind}" }},
    {parts}
  }},
  "layout": {{ "bb": {{ "at": [0, 0, 0] }}, {layout} }},
  "wires": [{wires}],
  "ground": "{ground}",
}}
"#
    );
    fs::write(dir.path().join("scene.jsonc"), text).unwrap();
    dir
}

/// A column of five and the 400's 25-hole X rail, each already full, say so.
#[test]
fn a_full_strip_says_the_strip_is_full() {
    let full = "a hole holds one lead; this strip is full, so wire it to a strip with a free hole and move one lead there";
    let column = loose(
        "breadboard-400",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },"#,
        r#""r1": { "on": { "a": "bb.a30" }, "rotate": [0, 0, 90] }"#,
        r#"["bb.b30", "bb.j1"], ["bb.c30", "bb.j2"], ["bb.d30", "bb.j3"], ["bb.a30", "bb.j4"]"#,
        "r1.a",
    );
    let found = layout_of(column.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].hint, full);
    assert!(found[0].message.contains("bb.a30"), "{}", found[0].message);

    let columns: Vec<u32> = (1..=29).filter(|n| n % 6 != 0).collect();
    assert_eq!(columns.len(), 25);
    let mut wires: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, col)| format!(r#"["bb.X{col}", "bb.a{}"]"#, i + 1))
        .collect();
    wires.push(r#"["bb.X1", "bb.j30"]"#.to_string());
    let rail = loose(
        "breadboard-400",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },"#,
        "",
        &wires.join(", "),
        "r1.a",
    );
    let found = layout_of(rail.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].hint, full, "{}", found[0].message);
    assert!(
        found[0].message.contains("names bb.X1"),
        "{}",
        found[0].message
    );
}

fn kit(part: &str, scene: &str) -> tempfile::TempDir {
    let dir = common::project("nano-button");
    fs::write(dir.path().join("parts/kit.jsonc"), part).unwrap();
    fs::write(dir.path().join("scene.jsonc"), scene).unwrap();
    dir
}

/// A breadboard that is a child of a part follows the same rule for that part's wires and
/// for a scene wire into one of its holes. A header pin inside a part still takes two wires.
#[test]
fn a_breadboard_inside_a_part_holds_one_lead_per_hole() {
    let two = kit(
        r#"{
  "format": 1,
  "joins_only": true,
  "parts": { "bb": { "type": "breadboard-830" } },
  "wires": [
    ["bb.a1", "bb.b2"],
    ["bb.a1", "bb.c3"],
  ],
}
"#,
        r#"{
  "format": 1,
  "parts": {
    "kit": { "type": "./parts/kit.jsonc" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
  },
  "ground": "r1.a",
}
"#,
    );
    let found = layout_of(two.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].message.contains(
            "the wire from `kit.bb.a1` to `kit.bb.c3` names kit.bb.a1, which the wire from `kit.bb.a1` to `kit.bb.b2` already holds"
        ),
        "{}",
        found[0].message
    );
    assert!(found[0].hint.contains("such as bb.b1"), "{}", found[0].hint);
    apply_every_hint(two.path());

    let shared = kit(
        r#"{
  "format": 1,
  "joins_only": true,
  "parts": { "bb": { "type": "breadboard-830" } },
  "wires": [["bb.a1", "bb.b2"]],
}
"#,
        r#"{
  "format": 1,
  "parts": {
    "kit": { "type": "./parts/kit.jsonc" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
  },
  "wires": [["kit.bb.a1", "r1.a"]],
  "ground": "r1.a",
}
"#,
    );
    let found = layout_of(shared.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].message.contains(
            "the wire from `kit.bb.a1` to `r1.a` names kit.bb.a1, which the wire from `kit.bb.a1` to `kit.bb.b2` already holds"
        ),
        "{}",
        found[0].message
    );
    assert!(
        found[0].hint.contains("such as kit.bb.b1"),
        "{}",
        found[0].hint
    );
    apply_every_hint(shared.path());

    let header = kit(
        r#"{
  "format": 1,
  "joins_only": true,
  "parts": { "hdr": { "type": "pin-header-1x15" } },
  "wires": [
    ["hdr.1", "hdr.2"],
    ["hdr.1", "hdr.3"],
  ],
}
"#,
        r#"{
  "format": 1,
  "parts": {
    "kit": { "type": "./parts/kit.jsonc" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
  },
  "wires": [["kit.hdr.1", "r1.a"]],
  "ground": "r1.a",
}
"#,
    );
    read(header.path());
}
