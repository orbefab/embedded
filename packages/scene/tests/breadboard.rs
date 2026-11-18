//! Parts plugged into a breadboard: where they sit, which holes the strips join, and that a
//! circuit on a breadboard is the circuit it is wired directly.

mod common;

use std::collections::{BTreeMap, BTreeSet};
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

/// A column of five and the 400's 25-hole X rail, each already full, name the move that
/// frees a hole. A board with nowhere to put that move says so. The skill and the decision
/// record quote those sentences.
#[test]
fn a_full_strip_says_the_strip_is_full() {
    let column = loose(
        "breadboard-400",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },"#,
        r#""r1": { "on": { "a": "bb.a30" }, "rotate": [0, 0, 90] }"#,
        r#"["bb.b30", "bb.j1"], ["bb.c30", "bb.j2"], ["bb.d30", "bb.j3"], ["bb.a30", "bb.j4"]"#,
        "r1.a",
    );
    let found = layout_of(column.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    let column_hint = "a hole holds one lead; this strip is full, so move the lead in bb.b30 to bb.a1, move this lead to bb.b1, and wire bb.b30 to bb.c1";
    assert_eq!(found[0].hint, column_hint);
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
    assert_eq!(
        found[0].hint,
        "a hole holds one lead; this strip is full, so move the lead in bb.X2 to bb.a26, move this lead to bb.b26, and wire bb.X2 to bb.c26",
        "{}",
        found[0].message
    );
    assert!(
        found[0].message.contains("names bb.X1"),
        "{}",
        found[0].message
    );

    let nowhere = no_strip_has_room();
    let found = layout_of(nowhere.path());
    assert_eq!(found.len(), 1, "{found:#?}");
    let nowhere_hint = "a hole holds one lead; this strip is full, and no strip on this board can take two leads and a wire without joining another net";
    assert_eq!(found[0].hint, nowhere_hint, "{}", found[0].message);
    let leg = "a hole holds one lead; this strip is full, and every other lead in it is a part leg, so there is no wire end to move off first";
    let repo = common::repo();
    for path in ["apps/orbe/skills/core.md", "docs/decisions.md"] {
        let text = fs::read_to_string(repo.join(path)).unwrap();
        assert!(
            text.contains(column_hint),
            "{path} does not quote the column hint"
        );
        assert!(
            text.contains(nowhere_hint),
            "{path} does not quote the no-room hint"
        );
        assert!(
            text.contains(leg),
            "{path} does not quote the part-leg hint"
        );
    }
}

/// Every hole of a 400-point board is taken, and one more wire lands in `a30`.
fn no_strip_has_room() -> tempfile::TempDir {
    let mut holes = Vec::new();
    for col in 1..=30 {
        for row in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"] {
            holes.push(format!("{row}{col}"));
        }
    }
    for row in ["W", "X", "Y", "Z"] {
        for col in (1..=29).filter(|n| n % 6 != 0) {
            holes.push(format!("{row}{col}"));
        }
    }
    assert_eq!(holes.len(), 400, "the 400-point board has 400 holes");
    let mut wires: Vec<String> = holes
        .chunks(2)
        .map(|pair| format!(r#"["bb.{}", "bb.{}"]"#, pair[0], pair[1]))
        .collect();
    wires.push(r#"["bb.a30", "r2.a"]"#.to_string());
    loose(
        "breadboard-400",
        r#""r2": { "type": "resistor-axial", "resistance": "220" },"#,
        "",
        &wires.join(", "),
        "r2.b",
    )
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

/// The two sentences a full strip used to print. Followed as they read, one takes a lead
/// off its net and the other lands a wire on the full strip.
const OLD_MOVE_ONE_LEAD: &str =
    "a hole holds one lead; the strip is full, so move one lead to another strip";
const OLD_WIRE_THE_FULL_STRIP: &str = "a hole holds one lead; this strip is full, so wire it to a strip with a free hole and move one lead there";

/// The printed hint, applied as written, leaves the scene clean and every lead on the net
/// `read` gave it before the extra lead had anywhere illegal to sit. Each old sentence fails
/// that, on the full column, the full X rail, and both strips at once.
#[test]
fn following_a_full_strip_hint_keeps_each_lead_on_its_net() {
    for make in [full_column, full_rail, two_full_strips] {
        let intended = make(true);
        let before = pin_nets(intended.path());
        let broken = make(false);
        follow(broken.path(), None).unwrap_or_else(|found| {
            panic!(
                "after the printed hint:\n{}",
                found
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
        same_nets(&before, &pin_nets(broken.path()))
            .unwrap_or_else(|why| panic!("{why}\n{}", scene_text(broken.path())));
        for old in [OLD_MOVE_ONE_LEAD, OLD_WIRE_THE_FULL_STRIP] {
            let dir = make(false);
            let followed = follow(dir.path(), Some(old));
            let kept = followed.is_ok() && same_nets(&before, &pin_nets(dir.path())).is_ok();
            assert!(!kept, "{old} left the leads on their nets");
        }
    }
}

fn scene_text(dir: &Path) -> String {
    fs::read_to_string(dir.join("scene.jsonc")).unwrap_or_default()
}

/// `legal` wires the extra lead to a pin or a free hole already on its net, so [`read`]
/// can name that net. The broken scene is the one the hint is printed for.
fn full_column(legal: bool) -> tempfile::TempDir {
    let extra = if legal {
        r#"["r1.a", "r2.a"]"#
    } else {
        r#"["bb.a30", "r2.a"]"#
    };
    loose(
        "breadboard-400",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },
        "r2": { "type": "resistor-axial", "resistance": "220" },"#,
        r#""r1": { "on": { "a": "bb.a30" }, "rotate": [0, 0, 90] }"#,
        &format!(r#"["bb.b30", "bb.j1"], ["bb.c30", "bb.j2"], ["bb.d30", "bb.j3"], {extra}"#,),
        "r1.a",
    )
}

fn rail_fill() -> Vec<String> {
    let columns: Vec<u32> = (1..=29).filter(|n| n % 6 != 0).collect();
    assert_eq!(columns.len(), 25);
    columns
        .iter()
        .enumerate()
        .map(|(i, col)| format!(r#"["bb.X{col}", "bb.a{}"]"#, i + 1))
        .collect()
}

fn full_rail(legal: bool) -> tempfile::TempDir {
    let mut wires = rail_fill();
    wires.push(r#"["bb.b1", "r1.a"]"#.to_string());
    if legal {
        wires.push(r#"["bb.d1", "bb.j30"]"#.to_string());
    } else {
        wires.push(r#"["bb.X1", "bb.j30"]"#.to_string());
    }
    wires.push(r#"["bb.i30", "r2.a"]"#.to_string());
    loose(
        "breadboard-400",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },
        "r2": { "type": "resistor-axial", "resistance": "220" },"#,
        "",
        &wires.join(", "),
        "r1.a",
    )
}

fn two_full_strips(legal: bool) -> tempfile::TempDir {
    let mut wires = vec![
        r#"["bb.b30", "bb.j1"]"#.to_string(),
        r#"["bb.c30", "bb.j2"]"#.to_string(),
        r#"["bb.d30", "bb.j3"]"#.to_string(),
    ];
    if legal {
        wires.push(r#"["r1.a", "r2.a"]"#.to_string());
    } else {
        wires.push(r#"["bb.a30", "r2.a"]"#.to_string());
    }
    wires.extend(rail_fill());
    wires.push(r#"["bb.c1", "r4.a"]"#.to_string());
    if legal {
        wires.push(r#"["bb.d1", "r3.a"]"#.to_string());
    } else {
        wires.push(r#"["bb.X1", "r3.a"]"#.to_string());
    }
    loose(
        "breadboard-400",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },
        "r2": { "type": "resistor-axial", "resistance": "220" },
        "r3": { "type": "resistor-axial", "resistance": "220" },
        "r4": { "type": "resistor-axial", "resistance": "220" },"#,
        r#""r1": { "on": { "a": "bb.a30" }, "rotate": [0, 0, 90] }"#,
        &wires.join(", "),
        "r1.a",
    )
}

fn pin_nets(dir: &Path) -> BTreeMap<String, String> {
    let netlist = read(dir);
    let mut pins = BTreeMap::new();
    for (part, map) in &netlist.refs.parts {
        for (pin, net) in &map.pins {
            if let Some(net) = net {
                pins.insert(format!("{part}.{pin}"), net.clone());
            }
        }
    }
    pins
}

/// Every pin that had a net still shares it with the same pins. Holes the hint's new wire
/// reaches may join that net; none of the old pins leave it.
fn same_nets(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (pin, net) in before {
        let Some(after_net) = after.get(pin) else {
            return Err(format!("{pin} lost its net"));
        };
        for (other, other_net) in before {
            let after_other = after.get(other);
            let together = other_net == net;
            let after_together = after_other.is_some_and(|got| got == after_net);
            if together != after_together {
                return Err(format!(
                    "{pin} ({net}) and {other} ({other_net}) before; after {after_net} and {after_other:?}"
                ));
            }
        }
    }
    Ok(())
}

struct Steps {
    span_to: String,
    move_from: Option<(String, String)>,
    wire: Option<(String, String)>,
}

fn procedure(hint: &str) -> Option<Steps> {
    let rest =
        hint.strip_prefix("a hole holds one lead; this strip is full, so move the lead in ")?;
    let (from, rest) = rest.split_once(" to ")?;
    let (moved, rest) = rest.split_once(", move this lead to ")?;
    let (extra, rest) = rest.split_once(", and wire ")?;
    let (wire_from, wire_to) = rest.split_once(" to ")?;
    Some(Steps {
        span_to: extra.to_string(),
        move_from: Some((from.to_string(), moved.to_string())),
        wire: Some((wire_from.to_string(), wire_to.to_string())),
    })
}

fn split_hole(hole: &str) -> (&str, &str, u32) {
    let (board, name) = hole.split_once('.').unwrap_or_else(|| panic!("{hole}"));
    let at = name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or_else(|| panic!("{hole}"));
    let (row, col) = name.split_at(at);
    let col = col.parse().unwrap_or_else(|_| panic!("{hole}"));
    (board, row, col)
}

/// The old "move one lead to another strip": the reported end goes to the next column.
fn old_move(span: &str) -> Steps {
    let (board, row, col) = split_hole(span);
    let next = if col > 1 { col - 1 } else { col + 1 };
    Steps {
        span_to: format!("{board}.{row}{next}"),
        move_from: None,
        wire: None,
    }
}

/// The old "wire it to a strip with a free hole and move one lead there": the new wire
/// starts on the full hole, and the reported lead moves to that other strip.
fn old_wire(span: &str) -> Steps {
    let (board, _, col) = split_hole(span);
    let dest = if col == 29 { 28 } else { 29 };
    Steps {
        span_to: format!("{board}.b{dest}"),
        move_from: None,
        wire: Some((span.to_string(), format!("{board}.a{dest}"))),
    }
}

fn steps_of(hint: &str, span: &str) -> Steps {
    if let Some(steps) = procedure(hint) {
        return steps;
    }
    if hint == OLD_MOVE_ONE_LEAD {
        return old_move(span);
    }
    if hint == OLD_WIRE_THE_FULL_STRIP {
        return old_wire(span);
    }
    panic!("hint has no steps a follower can apply: {hint}");
}

fn holes_named(hint: &str) -> BTreeSet<String> {
    hint.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .filter(|token| {
            token.contains('.')
                && token
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
        })
        .map(str::to_string)
        .collect()
}

fn quoted_span(text: &str, diag: &Diagnostic) -> (usize, usize, String) {
    let offset = at_line_col(text, diag.line, diag.column);
    let literal = &text[offset..];
    assert!(
        literal.starts_with('"'),
        "{}:{}:{} is not a wire end",
        diag.file,
        diag.line,
        diag.column
    );
    let end = literal[1..].find('"').expect("the end of the wire ref") + 1;
    (offset + 1, offset + end, literal[1..end].to_string())
}

fn add_wire(text: &str, a: &str, b: &str) -> String {
    let at = text.find("\n  \"ground\"").expect("the scene's ground");
    let close = text[..at].rfind(']').expect("the wires array");
    let mut out = String::new();
    out.push_str(&text[..close]);
    out.push_str(&format!(", [\"{a}\", \"{b}\"]"));
    out.push_str(&text[close..]);
    out
}

/// Apply `forced` in place of each printed hint, or the printed hint when `forced` is none.
/// A printed check's hints name different holes.
fn follow(dir: &Path, forced: Option<&str>) -> Result<(), Vec<Diagnostic>> {
    let path = dir.join("scene.jsonc");
    let text = fs::read_to_string(&path).unwrap();
    let found = findings(dir);
    let layout: Vec<Diagnostic> = found
        .into_iter()
        .filter(|d| d.code == Code::Layout)
        .collect();
    assert!(!layout.is_empty(), "no shared hole in {}", scene_text(dir));
    if forced.is_none() {
        let mut seen = BTreeSet::new();
        for diag in &layout {
            for hole in holes_named(&diag.hint) {
                assert!(seen.insert(hole.clone()), "{hole} is named by two hints");
            }
        }
    }
    let mut edits = Vec::new();
    let mut moves = Vec::new();
    let mut wires = Vec::new();
    for diag in &layout {
        let (_, _, span) = quoted_span(&text, diag);
        let hint = forced.unwrap_or(diag.hint.as_str());
        let steps = steps_of(hint, &span);
        let (start, end, _) = quoted_span(&text, diag);
        edits.push((start, end, steps.span_to));
        if let Some(move_from) = steps.move_from {
            moves.push(move_from);
        }
        if let Some(wire) = steps.wire {
            wires.push(wire);
        }
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));
    let mut text = text;
    for (start, end, hole) in edits {
        text.replace_range(start..end, &hole);
    }
    for (from, to) in moves {
        let needle = format!("\"{from}\"");
        let count = text.matches(&needle).count();
        assert_eq!(
            count, 1,
            "the lead in {from} appears {count} times:\n{text}"
        );
        text = text.replacen(&needle, &format!("\"{to}\""), 1);
    }
    for (a, b) in wires {
        text = add_wire(&text, &a, &b);
    }
    fs::write(&path, text).unwrap();
    orbe_scene::read(&Files::disk(), &path).map(|_| ())
}
