//! The parallel 16x2 LCD (`lcd1602`) on a Nano: LiquidCrystal's eight-pin bus, and V0 left open.

use std::path::{Path, PathBuf};

use orbe_session::{Command, Files, Live};
use serde_json::{Value, json};

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

/// The `lcd1602`'s supply on a Nano: VSS on ground and VDD on 5V.
const SUPPLY: [[&str; 2]; 2] = [["lcd.VSS", "nano.GND"], ["lcd.VDD", "nano.5V"]];

/// `D4` to `D7` on `D5` to `D2`, the HelloWorld's 4-bit bus.
const BUS4: [[&str; 2]; 4] = [
    ["lcd.D4", "nano.D5"],
    ["lcd.D5", "nano.D4"],
    ["lcd.D6", "nano.D3"],
    ["lcd.D7", "nano.D2"],
];

/// A Nano and an `lcd1602` with R/W, VSS and K on ground, VDD on 5V, A on 5V through 220 ohm
/// and RS and E on D12 and D11, plus `wires`. `contrast` puts a 10 kohm trimmer's wiper on V0
/// near 0.9 V.
fn open(hex: &str, wires: &[[&str; 2]], contrast: bool) -> Live {
    open_with(hex, wires, contrast, &SUPPLY, json!({}))
}

/// [`open`] with `supply` for the module's VSS and VDD wires and `extra` parts.
fn open_with(
    hex: &str,
    wires: &[[&str; 2]],
    contrast: bool,
    supply: &[[&str; 2]],
    extra: Value,
) -> Live {
    let mut parts = json!({
        "nano": { "type": "nano-board", "firmware": "build/lcd.hex" },
        "lcd": { "type": "lcd1602" },
        "r1": { "type": "resistor-axial", "resistance": "220" },
    });
    for (name, part) in extra.as_object().unwrap() {
        parts[name] = part.clone();
    }
    let mut all = vec![
        ["lcd.RW", "nano.GND"],
        ["lcd.K", "nano.GND"],
        ["r1.a", "nano.5V"],
        ["r1.b", "lcd.A"],
        ["lcd.RS", "nano.D12"],
        ["lcd.E", "nano.D11"],
    ];
    all.extend_from_slice(supply);
    if contrast {
        parts["trim"] = json!({ "type": "trimmer-3362p", "resistance": "10k", "position": 0.1 });
        all.extend([
            ["trim.a", "nano.GND"],
            ["trim.b", "nano.5V"],
            ["trim.wiper", "lcd.V0"],
        ]);
    }
    all.extend_from_slice(wires);
    let scene = json!({ "format": 1, "parts": parts, "wires": all });
    let block = json!({
        "format": 1,
        "block": "test",
        "version": "0.0.0",
        "title": "test",
        "board": "nano-board",
        "license": "MIT OR Apache-2.0"
    });
    let hex = std::fs::read(repo().join(hex)).unwrap();
    let files = Files::memory([
        ("block.jsonc".to_string(), block.to_string().into_bytes()),
        ("build/lcd.hex".to_string(), hex),
        ("scene.jsonc".to_string(), scene.to_string().into_bytes()),
    ])
    .with_catalog(catalog());
    let mut live = Live::new("t", files);
    let reply = live.run(&Command::Open(PathBuf::from("scene.jsonc")));
    assert!(reply.ok(), "{}", reply.text());
    live
}

const HELLO: &str = "blocks/nano-lcd1602/build/hello.ino.hex";
const BLINK: &str = "packages/engine/tests/firmware/sketch/liquidcrystal_blink.hex";

fn ok(live: &mut Live, line: &str) -> String {
    let reply = live.run(&Command::parse(line, Path::new("")).unwrap().unwrap());
    assert!(reply.ok(), "{line}: {}", reply.text());
    reply.text()
}

/// The JSON of `orbe get <target>`.
fn get(live: &mut Live, target: &str) -> Value {
    let reply = live.run(
        &Command::parse(&format!("get {target}"), Path::new(""))
            .unwrap()
            .unwrap(),
    );
    assert!(reply.ok(), "get {target}: {}", reply.text());
    reply.json()["data"].clone()
}

/// The rows of the glass as `orbe get @lcd.display.screen` prints them.
fn screen(live: &mut Live) -> Vec<String> {
    let data = get(live, "@lcd.display.screen");
    let rows = data["value"]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{data}"));
    rows.iter()
        .map(|row| row.as_str().unwrap().to_string())
        .collect()
}

/// The volts `orbe get @lcd.<pin>` prints.
fn volts(live: &mut Live, pin: &str) -> f64 {
    let text = ok(live, &format!("get @lcd.{pin}"));
    let words: Vec<&str> = text.split_whitespace().collect();
    words[words.len() - 2].parse().unwrap()
}

/// The diagnostics' messages.
fn messages(live: &mut Live) -> Vec<String> {
    let reply = live.run(&Command::parse("snapshot", Path::new("")).unwrap().unwrap());
    let diagnostics = reply.json()["data"]["diagnostics"].clone();
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["message"].as_str().unwrap().to_string())
        .collect()
}

/// The rule each diagnostic names, in the order the snapshot lists them.
fn rules(live: &mut Live) -> Vec<String> {
    let reply = live.run(&Command::parse("snapshot", Path::new("")).unwrap().unwrap());
    let diagnostics = reply.json()["data"]["diagnostics"].clone();
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d: &Value| {
            let message = d["message"].as_str().unwrap();
            let rule = message.split("breaking rule ").nth(1).unwrap_or(message);
            format!(
                "{} {}",
                d["code"].as_str().unwrap(),
                rule.split_whitespace().next().unwrap()
            )
        })
        .collect()
}

#[test]
fn eight_bit_constructor_prints_hello_world() {
    let bus = [
        ["lcd.D0", "nano.D9"],
        ["lcd.D1", "nano.D8"],
        ["lcd.D2", "nano.D7"],
        ["lcd.D3", "nano.D6"],
        ["lcd.D4", "nano.D5"],
        ["lcd.D5", "nano.D4"],
        ["lcd.D6", "nano.D3"],
        ["lcd.D7", "nano.D2"],
    ];
    let mut live = open(
        "packages/engine/tests/firmware/sketch/liquidcrystal_8bit.hex",
        &bus,
        true,
    );
    ok(&mut live, "step 1500ms");
    ok(&mut live, "expect @lcd.text line 0 \"hello, world!\"");
    ok(&mut live, "expect @lcd.text line 1 \"1\"");
    ok(&mut live, "step 1000ms");
    ok(&mut live, "expect @lcd.text line 1 \"2\"");
    // The constructor's own begin lands inside the reset busy time and then the reset wait, as
    // in the 4-bit Block.
    assert_eq!(
        rules(&mut live),
        ["chip.protocol init_busy", "chip.protocol init_wait"]
    );
}

#[test]
fn an_open_v0_leaves_the_glass_blank_and_warns_contrast() {
    let mut live = open("blocks/nano-lcd1602/build/hello.ino.hex", &BUS4, false);
    ok(&mut live, "step 1500ms");
    let (v0, vdd) = (volts(&mut live, "V0"), volts(&mut live, "VDD"));
    assert!(
        vdd - v0 < 0.01,
        "the bias ladder holds an open V0 at VDD, {v0} of {vdd} V"
    );
    ok(&mut live, "expect @lcd.backlight lit");
    ok(&mut live, "expect @lcd.text line 0 \"\"");
    ok(&mut live, "expect @lcd.text line 1 \"\"");
    assert_eq!(
        rules(&mut live),
        [
            "chip.protocol init_busy",
            "chip.protocol init_wait",
            "chip.protocol contrast"
        ]
    );
}

/// The glass's pixel origin of the character cell at `row` and `col`, and its dot pitch: 2 by 2
/// pixel dots on a 3 px pitch, 14 by 23 px cells 4 px apart across and 6 px apart down, from an
/// 18 px and 8 px margin.
fn cell(row: usize, col: usize) -> (usize, usize) {
    (18 + col * 18, 8 + row * 29)
}

/// Whether the screen art shows dot `dot_row`, `dot_col` of the cell at `row`, `col`.
fn dot(rows: &[String], row: usize, col: usize, dot_row: usize, dot_col: usize) -> bool {
    let (x, y) = cell(row, col);
    rows[y + dot_row * 3].as_bytes()[x + dot_col * 3] == b'#'
}

#[test]
fn the_screen_reading_shows_the_glyphs_on_the_lit_glass() {
    let mut live = open(HELLO, &BUS4, true);
    ok(&mut live, "step 1500ms");
    ok(&mut live, "expect @lcd.text line 0 \"hello, world!\"");
    let rows = screen(&mut live);
    // The A00 ROM's `h` (Table 4, p. 17), five dots a row, top row first.
    let h = [
        0b10000, 0b10000, 0b10110, 0b11001, 0b10001, 0b10001, 0b10001, 0b00000,
    ];
    for (dot_row, bits) in h.iter().enumerate() {
        for dot_col in 0..5 {
            let on = bits >> (4 - dot_col) & 1 != 0;
            assert_eq!(
                dot(&rows, 0, 0, dot_row, dot_col),
                on,
                "dot {dot_row},{dot_col} of h:\n{}",
                rows.join("\n")
            );
        }
    }
    // A space shows no dot, and neither does the glass between the cells.
    assert!((0..8).all(|r| (0..5).all(|c| !dot(&rows, 0, 15, r, c))));
    assert_eq!(rows[0].as_bytes()[0], b'.');
}

#[test]
fn stock_blink_alternates_the_cursor_cell_at_the_sheet_rate() {
    let mut live = open(BLINK, &BUS4, true);
    // The loop runs noBlink(), waits 3 s, runs blink() and waits 3 s, so B = 1 from about 3 s to
    // 6 s. The cursor sits after "hello, world!", on row 0, column 13.
    ok(&mut live, "step 4000ms");
    ok(&mut live, "expect @lcd.text line 0 \"hello, world!\"");
    let mut glasses = vec![screen(&mut live)];
    for step in [120, 130, 130] {
        ok(&mut live, &format!("step {step}ms"));
        glasses.push(screen(&mut live));
    }
    assert!(
        glasses.windows(2).any(|pair| pair[0] != pair[1]),
        "the glass is the same at 4.0, 4.12, 4.25 and 4.38 s"
    );
    // Sampled every 10 ms from 4.5 s to 5.9 s, the cell switches between all dots and its
    // character, a space, every 409.6 ms x 250 / 270 = 379.3 ms (p. 26).
    ok(&mut live, "step 120ms");
    let mut switches = Vec::new();
    let mut last = None;
    for k in 0..140 {
        let rows = screen(&mut live);
        let block = (0..8).all(|r| (0..5).all(|c| dot(&rows, 0, 13, r, c)));
        let blank = (0..8).all(|r| (0..5).all(|c| !dot(&rows, 0, 13, r, c)));
        assert!(block || blank, "the cell shows a block or a space");
        if last.is_some_and(|was| was != block) {
            switches.push(4500 + k * 10);
        }
        last = Some(block);
        ok(&mut live, "step 10ms");
    }
    assert!(switches.len() >= 3, "{switches:?}");
    for pair in switches.windows(2) {
        let gap = pair[1] - pair[0];
        assert!((370..=390).contains(&gap), "{switches:?}");
    }
    // Once blink() is undone, at about 6 s, the cell holds its space.
    ok(&mut live, "step 700ms");
    let rows = screen(&mut live);
    assert!((0..8).all(|r| (0..5).all(|c| !dot(&rows, 0, 13, r, c))));
}

#[test]
fn the_backlight_reads_lit_from_its_led_with_vdd_open_or_reversed() {
    let open_vdd: &[[&str; 2]] = &[["lcd.VSS", "nano.GND"]];
    let reversed: &[[&str; 2]] = &[["lcd.VSS", "nano.5V"], ["lcd.VDD", "nano.GND"]];
    for supply in [open_vdd, reversed] {
        let mut live = open_with(HELLO, &BUS4, true, supply, json!({}));
        ok(&mut live, "step 1500ms");
        let amps = get(&mut live, "@r1.i");
        let amps = amps["value"].as_f64().unwrap_or_else(|| panic!("{amps}"));
        assert!(amps > 2e-3, "{supply:?}: the LED path carries {amps} A");
        ok(&mut live, "expect @lcd.backlight lit");
        ok(&mut live, "expect @lcd.text line 0 \"\"");
    }
}

#[test]
fn a_supply_under_the_module_minimum_still_runs_and_warns_low_supply() {
    // 220 ohm in VDD drops the 5V rail to about 4.65 V: past the controller's 4.5 V, under the
    // module's 4.7 V minimum (Vishay LCD-016N002B p. 1).
    let supply: &[[&str; 2]] = &[
        ["lcd.VSS", "nano.GND"],
        ["r2.a", "nano.5V"],
        ["r2.b", "lcd.VDD"],
    ];
    let extra = json!({ "r2": { "type": "resistor-axial", "resistance": "220" } });
    let mut live = open_with(HELLO, &BUS4, true, supply, extra);
    ok(&mut live, "step 1500ms");
    let vdd = volts(&mut live, "VDD") - volts(&mut live, "VSS");
    assert!((4.5..4.7).contains(&vdd), "VDD is {vdd} V");
    ok(&mut live, "expect @lcd.text line 0 \"hello, world!\"");
    let rules = rules(&mut live);
    assert!(
        rules.iter().any(|rule| rule == "chip.protocol low_supply"),
        "{rules:?}"
    );
}

#[test]
fn the_init_warnings_say_which_writes_ran_and_which_were_dropped() {
    let mut live = open(HELLO, &BUS4, true);
    ok(&mut live, "step 1500ms");
    // The constructor's begin starts about 1 ms after power-on: its first writes land inside the
    // 10 ms reset busy time and are dropped, and the ones after 10 ms run before the 15 ms wait.
    let messages = messages(&mut live);
    let busy = messages
        .iter()
        .find(|m| m.contains("breaking rule init_busy"))
        .unwrap_or_else(|| panic!("{messages:?}"));
    assert!(busy.contains("dropped"), "{busy}");
    let wait = messages
        .iter()
        .find(|m| m.contains("breaking rule init_wait"))
        .unwrap_or_else(|| panic!("{messages:?}"));
    assert!(!wait.contains("dropped") && wait.contains("ran"), "{wait}");
}
