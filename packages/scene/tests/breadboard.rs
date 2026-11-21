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

const FULL_STRIP: &str = "a hole holds one lead, and every hole in this strip is taken; free one first, for example by moving one of its leads to an empty strip and wiring that strip back to the hole it left";
const ALREADY_NAMED: &str = "a hole holds one lead; every free hole in this strip is already named by another hint in this check";
const FREE_HOLE: &str = "a hole holds one lead; move one of them to a free hole in the same strip";

/// A full column, the 400's full X rail, and both at once name no hole. A strip filled
/// with part legs, a board with no empty hole, and a full strip inside a part say the
/// same sentence. The skill and the decision record quote it.
#[test]
fn a_full_strip_says_the_strip_is_full() {
    let cases = [
        ("column", full_column(false)),
        ("rail", full_rail(false)),
        ("both", two_full_strips(false)),
        ("nowhere", no_strip_has_room()),
        ("legs", legs_fill_the_strip()),
        ("nested", nested_full_column()),
    ];
    for (name, dir) in cases {
        let found = layout_of(dir.path());
        assert!(!found.is_empty(), "{name}: {}", scene_text(dir.path()));
        for diag in &found {
            assert_eq!(diag.hint, FULL_STRIP, "{name}: {}", diag.message);
            assert!(
                holes_named(&diag.hint).is_empty(),
                "{name} names a hole: {}",
                diag.hint
            );
        }
    }
    let repo = common::repo();
    for path in ["apps/orbe/skills/core.md", "docs/decisions.md"] {
        let text = fs::read_to_string(repo.join(path)).unwrap();
        assert!(
            text.contains(FULL_STRIP),
            "{path} does not quote the full-strip hint"
        );
        assert!(
            text.contains(ALREADY_NAMED),
            "{path} does not quote the already-named hint"
        );
        assert!(
            text.contains(&format!("{FREE_HOLE}, such as bb.c1")),
            "{path} does not quote the free-hole hint"
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
    assert_eq!(
        found[0].hint,
        format!("{FREE_HOLE}, such as bb.b1"),
        "{}",
        found[0].hint
    );
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
    assert_eq!(
        found[0].hint,
        format!("{FREE_HOLE}, such as kit.bb.b1"),
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

/// The sentence a full strip used to print. Followed as it reads, the reported lead
/// moves to the next column and leaves its net.
const OLD_MOVE_ONE_LEAD: &str =
    "a hole holds one lead; the strip is full, so move one lead to another strip";

/// The printed free-hole hint, applied as written, leaves the scene clean and every lead
/// on the net `read` gave it before the extra lead shared a hole. The old "move one lead
/// to another strip" does not, on a column.
#[test]
fn following_a_free_hole_hint_keeps_each_lead_on_its_net() {
    for make in [
        column_with_room,
        rail_with_room,
        two_strips_with_room,
        part_file_hole,
        scene_file_hole,
    ] {
        let intended = make(true);
        let before = pin_nets(intended.path());
        let broken = make(false);
        let found = layout_of(broken.path());
        assert!(!found.is_empty(), "{}", scene_text(broken.path()));
        for diag in &found {
            assert!(
                diag.hint.starts_with(FREE_HOLE),
                "hint reverted: {}",
                diag.hint
            );
            assert!(
                !diag.hint.contains("move one lead to another strip"),
                "hint reverted: {}",
                diag.hint
            );
            assert_eq!(holes_named(&diag.hint).len(), 1, "{}", diag.hint);
        }
        apply_every_hint(broken.path());
        same_nets(&before, &pin_nets(broken.path()))
            .unwrap_or_else(|why| panic!("{why}\n{}", scene_text(broken.path())));
    }
    for make in [column_with_room, two_strips_with_room] {
        let intended = make(true);
        let before = pin_nets(intended.path());
        let broken = make(false);
        let followed = follow_old_move(broken.path());
        let kept = followed.is_ok() && same_nets(&before, &pin_nets(broken.path())).is_ok();
        assert!(!kept, "{OLD_MOVE_ONE_LEAD} left the leads on their nets");
    }
}

/// One free hole, two extra leads. The first hint names that hole. The second does not
/// call the strip full, and it names no hole.
#[test]
fn a_free_hole_already_named_is_not_called_taken() {
    let dir = loose(
        "breadboard-400",
        r#""r1": { "type": "resistor-axial", "resistance": "220" },
        "r2": { "type": "resistor-axial", "resistance": "220" },
        "r3": { "type": "resistor-axial", "resistance": "220" },"#,
        r#""r1": { "on": { "a": "bb.a30" }, "rotate": [0, 0, 90] }"#,
        r#"["bb.b30", "bb.j1"], ["bb.c30", "bb.j2"], ["bb.a30", "r2.a"], ["bb.a30", "r3.a"]"#,
        "r1.a",
    );
    let found = layout_of(dir.path());
    assert_eq!(found.len(), 2, "{found:#?}");
    assert_eq!(found[0].hint, format!("{FREE_HOLE}, such as bb.d30"));
    assert_eq!(found[1].hint, ALREADY_NAMED);
    assert!(holes_named(&found[1].hint).is_empty(), "{}", found[1].hint);
}

fn scene_text(dir: &Path) -> String {
    fs::read_to_string(dir.join("scene.jsonc")).unwrap_or_default()
}

/// `legal` wires the extra lead to a pin already on its net, so [`read`] can name that
/// net. The broken scene is the one the hint is printed for.
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

/// Column 30 has one free hole, `d30`.
fn column_with_room(legal: bool) -> tempfile::TempDir {
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
        &format!(r#"["bb.b30", "bb.j1"], ["bb.c30", "bb.j2"], {extra}"#,),
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

/// The X rail has one free hole, `X29`.
fn rail_with_room(legal: bool) -> tempfile::TempDir {
    let mut wires = rail_fill();
    wires.retain(|wire| !wire.contains("bb.X29"));
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

/// Column 30 and the X rail each have one free hole.
fn two_strips_with_room(legal: bool) -> tempfile::TempDir {
    let mut wires = vec![
        r#"["bb.b30", "bb.j1"]"#.to_string(),
        r#"["bb.c30", "bb.j2"]"#.to_string(),
    ];
    if legal {
        wires.push(r#"["r1.a", "r2.a"]"#.to_string());
    } else {
        wires.push(r#"["bb.a30", "r2.a"]"#.to_string());
    }
    let mut rail = rail_fill();
    rail.retain(|wire| !wire.contains("bb.X29"));
    wires.extend(rail);
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

/// Five part legs fill column 30. The other leg of each sits four columns back.
fn legs_fill_the_strip() -> tempfile::TempDir {
    let rows = ["a", "b", "c", "d", "e"];
    let mut parts = Vec::new();
    let mut layout = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let name = format!("p{}", i + 1);
        parts.push(format!(
            r#""{name}": {{ "type": "resistor-axial", "resistance": "220" }}"#
        ));
        layout.push(format!(
            r#""{name}": {{ "on": {{ "a": "bb.{row}30" }}, "rotate": [0, 0, 180] }}"#
        ));
    }
    parts.push(r#""r2": { "type": "resistor-axial", "resistance": "220" }"#.to_string());
    loose(
        "breadboard-400",
        &parts.join(",\n"),
        &layout.join(",\n"),
        r#"["bb.a30", "r2.a"]"#,
        "r2.b",
    )
}

/// Column 30 is full inside a part. The scene's extra lead is the one that shares a hole.
fn nested_full_column() -> tempfile::TempDir {
    kit(
        r#"{
  "format": 1,
  "joins_only": true,
  "parts": { "bb": { "type": "breadboard-400" } },
  "wires": [
    ["bb.a30", "bb.j1"],
    ["bb.b30", "bb.j2"],
    ["bb.c30", "bb.j3"],
    ["bb.d30", "bb.j4"],
    ["bb.e30", "bb.j5"],
  ],
}
"#,
        r#"{
  "format": 1,
  "parts": {
    "kit": { "type": "./parts/kit.jsonc" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
  },
  "wires": [["kit.bb.a30", "r1.a"]],
  "ground": "r1.a",
}
"#,
    )
}

/// The part file writes `bb.X`. `legal` already uses the free hole the hint will name.
fn part_file_hole(legal: bool) -> tempfile::TempDir {
    let second = if legal {
        r#"["bb.b1", "bb.c3"]"#
    } else {
        r#"["bb.a1", "bb.c3"]"#
    };
    kit(
        &format!(
            r#"{{
  "format": 1,
  "joins_only": true,
  "parts": {{ "bb": {{ "type": "breadboard-830" }} }},
  "wires": [
    ["bb.a1", "bb.b2"],
    {second},
  ],
}}
"#
        ),
        r#"{
  "format": 1,
  "parts": {
    "kit": { "type": "./parts/kit.jsonc" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
  },
  "ground": "r1.a",
}
"#,
    )
}

/// The scene writes `kit.bb.X`. `legal` already uses the free hole the hint will name.
fn scene_file_hole(legal: bool) -> tempfile::TempDir {
    let end = if legal { "kit.bb.b1" } else { "kit.bb.a1" };
    kit(
        r#"{
  "format": 1,
  "joins_only": true,
  "parts": { "bb": { "type": "breadboard-830" } },
  "wires": [["bb.a1", "bb.b2"]],
}
"#,
        &format!(
            r#"{{
  "format": 1,
  "parts": {{
    "kit": {{ "type": "./parts/kit.jsonc" }},
    "r1": {{ "type": "resistor-axial", "resistance": "220" }},
  }},
  "wires": [["{end}", "r1.a"]],
  "ground": "r1.a",
}}
"#
        ),
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

/// Every pin that had a net still shares it with the same pins. A hole the hint's move
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

fn holes_named(hint: &str) -> BTreeSet<String> {
    hint.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .filter(|token| {
            token.contains('.')
                && token
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && token.chars().any(|c| c.is_ascii_digit())
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

/// The old "move one lead to another strip": the reported end goes to the next column.
fn next_column(hole: &str) -> String {
    let (prefix, name) = hole.rsplit_once('.').unwrap_or_else(|| panic!("{hole}"));
    let at = name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or_else(|| panic!("{hole}"));
    let (row, col) = name.split_at(at);
    let col: u32 = col.parse().unwrap_or_else(|_| panic!("{hole}"));
    let next = if col > 1 { col - 1 } else { col + 1 };
    format!("{prefix}.{row}{next}")
}

fn follow_old_move(dir: &Path) -> Result<(), Vec<Diagnostic>> {
    let found = findings(dir);
    let layout: Vec<Diagnostic> = found
        .into_iter()
        .filter(|d| d.code == Code::Layout)
        .collect();
    assert!(!layout.is_empty(), "no shared hole in {}", scene_text(dir));
    let mut edits: BTreeMap<String, Vec<(usize, usize, String)>> = BTreeMap::new();
    for diag in &layout {
        let path = dir.join(&diag.file);
        let text = fs::read_to_string(&path).unwrap();
        let (start, end, span) = quoted_span(&text, diag);
        edits
            .entry(diag.file.clone())
            .or_default()
            .push((start, end, next_column(&span)));
    }
    for (file, mut spans) in edits {
        spans.sort_by_key(|span| std::cmp::Reverse(span.0));
        let path = dir.join(file);
        let mut text = fs::read_to_string(&path).unwrap();
        for (start, end, hole) in spans {
            text.replace_range(start..end, &hole);
        }
        fs::write(path, text).unwrap();
    }
    orbe_scene::read(&Files::disk(), &dir.join("scene.jsonc")).map(|_| ())
}

/// A hole past the board names the rows and columns the board has. A neighbor is suggested
/// only when it is one row or one column away, so `j99` is not read as `j9`.
#[test]
fn a_hole_past_the_board_names_the_range_and_only_a_near_hole() {
    let dir = on_400(
        r#""led1": { "type": "led-5mm", "color": "red" },"#,
        r#""led1": { "on": { "anode": "bb.j99" } },"#,
        r#"["bb.j99", "nano.GND"], ["bb.j31", "nano.5V"], ["bb.W6", "nano.D2"], ["bb.J9", "nano.D13"], ["bb.k31", "nano.A0"]"#,
    );
    let found = findings(dir.path());
    let hint = |needle: &str| {
        found
            .iter()
            .find(|d| d.message.contains(needle))
            .unwrap_or_else(|| panic!("no finding for {needle}: {found:#?}"))
            .hint
            .clone()
    };
    let range = "rows a to j at columns 1 to 30; rows W to Z at columns 1 to 5, 7 to 11, 13 to 17, 19 to 23, 25 to 29";
    let far: Vec<_> = found
        .iter()
        .filter(|d| d.message.contains("`j99`"))
        .collect();
    assert_eq!(
        far.len(),
        2,
        "the wire and the plug each name j99: {found:#?}"
    );
    for diag in far {
        assert!(diag.hint.contains(range), "{}", diag.hint);
        assert!(!diag.hint.contains("did you mean"), "{}", diag.hint);
        assert!(
            !diag.hint.contains("a1, "),
            "j99 lists holes instead of the range: {}",
            diag.hint
        );
    }
    assert!(
        hint("`j31`").contains("did you mean `bb.j30`?"),
        "{}",
        hint("`j31`")
    );
    assert!(
        hint("`W6`").contains("did you mean `bb.W5`?"),
        "{}",
        hint("`W6`")
    );
    assert!(
        hint("`J9`").contains("did you mean `bb.j9`?"),
        "{}",
        hint("`J9`")
    );
    let diagonal = hint("`k31`");
    assert!(diagonal.contains(range), "{diagonal}");
    assert!(!diagonal.contains("did you mean"), "{diagonal}");
}
