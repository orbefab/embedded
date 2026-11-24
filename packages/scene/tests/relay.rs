//! The Songle coil and the 1-channel module.
//!
//! The contact closes above the must-operate voltage and opens below must-release.
//! A voltage between the two leaves it. The module's input current is the base
//! resistor, and the coil current comes from VCC.

mod common;

use std::fs;
use std::os::unix::fs::symlink;

use orbe_engine::diagnostic::Severity;
use orbe_engine::logic::Level;
use orbe_scene::Files;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    symlink(common::repo().join("parts"), dir.path().join("parts")).unwrap();
    dir
}

fn run(mut json: serde_json::Value, stop_ps: u64) -> orbe_engine::run_folder::Results {
    json["analysis"]["transient"]["stop_ps"] = serde_json::json!(stop_ps);
    json["analysis"]["transient"]["max_step_ps"] = serde_json::json!(1_000_000_000u64);
    json["analysis"]["transient"]["grid_ps"] = serde_json::json!(1_000_000_000u64);
    let bytes = serde_json::to_vec(&json).unwrap();
    let parsed = orbe_engine::netlist::parse(&bytes).expect("the netlist parses");
    orbe_engine::simulate(&parsed)
}

fn column(results: &orbe_engine::run_folder::Results, name: &str) -> Vec<f64> {
    let trace = results.trace().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|column| column.to_string() == name)
        .unwrap_or_else(|| panic!("no column {name}; have {:?}", trace.columns()));
    trace.column(index).to_vec()
}

fn at(results: &orbe_engine::run_folder::Results, name: &str, t_ps: u64) -> f64 {
    let trace = results.trace().unwrap();
    let values = column(results, name);
    let row = trace
        .time()
        .iter()
        .position(|time| time.as_ps() >= t_ps)
        .unwrap_or_else(|| panic!("{name} has no row at {t_ps}"));
    values[row]
}

fn contact_edges(results: &orbe_engine::run_folder::Results, part: &str) -> Vec<(u64, Level)> {
    let trace = results.trace().unwrap();
    let logic = trace.logic().expect("the relay has a contact");
    let want = format!("#{part}.logic.contact");
    let index = logic
        .signals()
        .iter()
        .position(|signal| signal.to_string() == want)
        .unwrap_or_else(|| panic!("no {want}"));
    logic
        .changes()
        .iter()
        .filter(|change| change.signal == index)
        .map(|change| (change.at.as_ps(), change.level))
        .collect()
}

fn errors(results: &orbe_engine::run_folder::Results) -> Vec<String> {
    results
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.severity() == Severity::Error)
        .map(|diagnostic| diagnostic.code().to_string())
        .collect()
}

const COIL: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "model": {
    "kind": "voltage_source",
    "pins": { "p": "pos", "n": "neg" },
    "voltage": {
      "kind": "steps",
      "initial": { "value": 5, "unit": "V", "source": "user" },
      "events": [
        { "t_ps": 50000000000, "level": { "value": 2, "unit": "V", "source": "user" } },
        { "t_ps": 100000000000, "level": { "value": 0.3, "unit": "V", "source": "user" } }
      ]
    }
  }
}"#;

const FIVE: &str = r#"{
  "format": 1,
  "pins": { "pos": [1], "neg": [2] },
  "model": {
    "kind": "voltage_source",
    "pins": { "p": "pos", "n": "neg" },
    "voltage": { "value": 5, "unit": "V", "source": "user" }
  }
}"#;

/// 5 V closes the contact, 2 V leaves it closed, and 0.3 V opens it. None of those is an error.
#[test]
fn the_contact_follows_the_coil_voltage_with_hysteresis() {
    let dir = project();
    fs::write(dir.path().join("coil.jsonc"), COIL).unwrap();
    fs::write(dir.path().join("five.jsonc"), FIVE).unwrap();
    let scene = r#"{
      "format": 1,
      "parts": {
        "coil": { "type": "./coil.jsonc" },
        "load": { "type": "./five.jsonc" },
        "k": { "type": "relay-srd-05vdc-sl-c" },
        "rno": { "type": "resistor-axial", "resistance": "1k" }
      },
      "ground": "coil.neg",
      "wires": [
        ["coil.pos", "k.coil_a"],
        ["coil.neg", "k.coil_b"],
        ["load.neg", "coil.neg"],
        ["load.pos", "k.COM"],
        ["k.NO", "rno.a"],
        ["rno.b", "coil.neg"]
      ]
    }"#;
    let path = dir.path().join("scene.jsonc");
    fs::write(&path, scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &path).expect("the relay scene checks");
    let json = serde_json::from_str(&netlist.json).unwrap();
    let results = run(json, 150_000_000_000);
    assert!(errors(&results).is_empty(), "{:?}", results.diagnostics());
    let edges = contact_edges(&results, "k");
    // 5 V is on at t = 0, so the operating point closes the contact and writes no edge.
    assert!(
        !edges.iter().any(|(at, _)| *at < 100_000_000_000),
        "the contact moved at 5 V or at 2 V: {edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|(at, level)| *at > 100_000_000_000 && *level == Level::Low),
        "0.3 V did not open the contact: {edges:?}"
    );
    let closed = at(&results, "#rno.i", 40_000_000_000);
    let held = at(&results, "#rno.i", 80_000_000_000);
    let open = at(&results, "#rno.i", 130_000_000_000);
    assert!(closed > 0.004, "closed load {closed}");
    assert!(held > 0.004, "held load {held}");
    assert!(open.abs() < 1e-4, "open load {open}");
}

/// IN draws the base current. The coil current comes from VCC, and VCC at 0 V leaves the contact open.
#[test]
fn the_module_input_draws_the_driver_and_an_unpowered_coil_stays_open() {
    let dir = project();
    fs::write(dir.path().join("five.jsonc"), FIVE).unwrap();
    let vcc = r#"{
      "format": 1,
      "pins": { "pos": [1], "neg": [2] },
      "model": {
        "kind": "voltage_source",
        "pins": { "p": "pos", "n": "neg" },
        "voltage": {
          "kind": "steps",
          "initial": { "value": 0, "unit": "V", "source": "user" },
          "events": [
            { "t_ps": 20000000000, "level": { "value": 5, "unit": "V", "source": "user" } }
          ]
        }
      }
    }"#;
    fs::write(dir.path().join("vcc.jsonc"), vcc).unwrap();
    let scene = r#"{
      "format": 1,
      "parts": {
        "vcc": { "type": "./vcc.jsonc" },
        "sig": { "type": "./five.jsonc" },
        "mod": { "type": "relay-module-1ch" },
        "rno": { "type": "resistor-axial", "resistance": "1k" }
      },
      "ground": "vcc.neg",
      "wires": [
        ["vcc.neg", "sig.neg"],
        ["vcc.neg", "mod.GND"],
        ["vcc.pos", "mod.VCC"],
        ["sig.pos", "mod.IN"],
        ["vcc.pos", "mod.COM"],
        ["mod.NO", "rno.a"],
        ["rno.b", "vcc.neg"]
      ]
    }"#;
    let path = dir.path().join("scene.jsonc");
    fs::write(&path, scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &path).expect("the module scene checks");
    let json = serde_json::from_str(&netlist.json).unwrap();
    let results = run(json, 50_000_000_000);
    assert!(errors(&results).is_empty(), "{:?}", results.diagnostics());
    let edges = contact_edges(&results, "mod__k");
    assert!(
        edges.iter().all(|(at, _)| *at >= 20_000_000_000),
        "the contact moved while VCC was off: {edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|(at, level)| *at >= 20_000_000_000 && *level == Level::High),
        "5 V on VCC did not close the contact: {edges:?}"
    );
    let coil_off = at(&results, "#mod__k.i", 10_000_000_000).abs();
    let coil_on = at(&results, "#mod__k.i", 40_000_000_000).abs();
    let base = at(&results, "#mod__rb.i", 40_000_000_000).abs();
    assert!(coil_off < 0.001, "unpowered coil {coil_off}");
    assert!(coil_on > 0.05, "coil current {coil_on}");
    assert!(base < 0.01, "input current {base} is the coil");
    assert!(base > 0.001, "the driver draws nothing: {base}");
    let load = at(&results, "#rno.i", 40_000_000_000);
    assert!(load > 0.004, "the contact did not switch the load: {load}");
}

/// COM is 1.4 mm inside the 19 mm body, and the body is 15.3 mm tall. The rows stay
/// 2 mm and then 12.2 mm apart, at ±6 mm.
#[test]
fn the_srd_outline_matches_the_drilling_drawing() {
    let dir = common::repo().join("parts/relay-srd-05vdc-sl-c");
    let anchors: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("relay-srd-05vdc-sl-c.anchors.json")).unwrap(),
    )
    .unwrap();
    let asset: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("asset.json")).unwrap()).unwrap();
    let at = |lead: u64| -> [f64; 3] {
        let pin = anchors["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|pin| pin["lead"] == lead)
            .unwrap();
        let nums = pin["at"].as_array().unwrap();
        [0, 1, 2].map(|i| nums[i].as_f64().unwrap())
    };
    let com = at(1);
    let coil = at(2);
    let no = at(3);
    let nc = at(4);
    let coil_b = at(5);
    let edge = asset["box"]["min"][0].as_f64().unwrap();
    assert!((edge - (-9.5)).abs() < 1e-6, "body edge {edge}");
    assert!(
        (com[0] - edge - 1.4).abs() < 1e-6,
        "COM inset {}",
        com[0] - edge
    );
    assert!((asset["box"]["max"][2].as_f64().unwrap() - 15.3).abs() < 1e-6);
    assert!((coil[0] - com[0] - 2.0).abs() < 1e-6 && (coil[1] - 6.0).abs() < 1e-6);
    assert!((coil_b[0] - coil[0]).abs() < 1e-6 && (coil_b[1] + 6.0).abs() < 1e-6);
    assert!((no[0] - coil[0] - 12.2).abs() < 1e-6 && (no[1] - 6.0).abs() < 1e-6);
    assert!((nc[0] - no[0]).abs() < 1e-6 && (nc[1] + 6.0).abs() < 1e-6);
    for pin in [com, coil, no, nc, coil_b] {
        assert!(pin[2].abs() < 1e-6);
    }
}

/// The screw terminal, the relay, the diode and the small parts each keep their own space.
/// The diode lies on the board. The relay and the header are the leads through it.
#[test]
fn the_module_bodies_do_not_share_space() {
    let repo = common::repo();
    let text = fs::read_to_string(repo.join("parts/relay-module-1ch/part.jsonc")).unwrap();
    let text: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let part: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut boxes: Vec<(&str, Aabb)> = vec![
        (
            "terminal",
            Aabb {
                min: [-1.0, 7.5, 4.14],
                max: [15.0, 15.5, 12.14],
            },
        ),
        (
            "pcb",
            Aabb {
                min: [-17.0, -11.5, 2.54],
                max: [17.0, 15.5, 4.14],
            },
        ),
    ];
    let layout = part["layout"].as_object().unwrap();
    for (name, place) in layout {
        let kind = part["parts"][name]["type"].as_str().unwrap();
        let asset: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(repo.join("parts").join(kind).join("asset.json")).unwrap(),
        )
        .unwrap();
        let num = |side: &str, i: usize| asset["box"][side][i].as_f64().unwrap();
        let local = Aabb {
            min: [num("min", 0), num("min", 1), num("min", 2)],
            max: [num("max", 0), num("max", 1), num("max", 2)],
        };
        let at = [0, 1, 2].map(|i| place["at"][i].as_f64().unwrap());
        let rotate = match place.get("rotate") {
            Some(rotate) => [0, 1, 2].map(|i| rotate[i].as_f64().unwrap()),
            None => [0.0; 3],
        };
        boxes.push((name.as_str(), world_box(local, at, rotate)));
    }
    let owned: Vec<(String, Aabb)> = boxes
        .iter()
        .map(|(name, aabb)| ((*name).to_string(), *aabb))
        .collect();
    for (i, (a_name, a)) in owned.iter().enumerate() {
        for (b_name, b) in owned.iter().skip(i + 1) {
            let through = matches!(
                (a_name.as_str(), b_name.as_str()),
                ("pcb", "k" | "hdr") | ("k" | "hdr", "pcb")
            );
            if through {
                continue;
            }
            let hit = overlap(a, b);
            assert!(
                hit.is_none(),
                "{a_name} overlaps {b_name} by {hit:?}\n{a_name} {a:?}\n{b_name} {b:?}"
            );
        }
    }
    let diode = owned.iter().find(|(name, _)| name == "d").unwrap().1;
    assert!(
        diode.min[2] >= 4.14 - 1e-3,
        "the diode enters the board: {diode:?}"
    );
}

#[derive(Clone, Copy, Debug)]
struct Aabb {
    min: [f64; 3],
    max: [f64; 3],
}

fn world_box(local: Aabb, at: [f64; 3], rotate: [f64; 3]) -> Aabb {
    let m = rotation(rotate);
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for x in [local.min[0], local.max[0]] {
        for y in [local.min[1], local.max[1]] {
            for z in [local.min[2], local.max[2]] {
                let p = turn(&m, [x, y, z]);
                for i in 0..3 {
                    let w = p[i] + at[i];
                    min[i] = min[i].min(w);
                    max[i] = max[i].max(w);
                }
            }
        }
    }
    Aabb { min, max }
}

fn overlap(a: &Aabb, b: &Aabb) -> Option<[f64; 3]> {
    let mut depth = [0.0; 3];
    for (i, slot) in depth.iter_mut().enumerate() {
        let lo = a.min[i].max(b.min[i]);
        let hi = a.max[i].min(b.max[i]);
        if hi - lo <= 1e-4 {
            return None;
        }
        *slot = hi - lo;
    }
    Some(depth)
}

/// Quarter turns only, so the matrix does not depend on the host math library.
fn sincos(deg: f64) -> (f64, f64) {
    match deg as i32 {
        0 => (0.0, 1.0),
        90 => (1.0, 0.0),
        -90 | 270 => (-1.0, 0.0),
        180 | -180 => (0.0, -1.0),
        _ => panic!("the module test turns on a quarter degree, got {deg}"),
    }
}

/// Fixed X, then Y, then Z, matching the scene's placement.
fn rotation([rx, ry, rz]: [f64; 3]) -> [[f64; 3]; 3] {
    let (sx, cx) = sincos(rx);
    let (sy, cy) = sincos(ry);
    let (sz, cz) = sincos(rz);
    [
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ]
}

fn turn(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|row| m[row][0] * v[0] + m[row][1] * v[1] + m[row][2] * v[2])
}
