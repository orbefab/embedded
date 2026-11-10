use std::fs;

use crate::runs::common;
use common::{read_vcd, records, repo, summary};
use orbe_engine::name::Ref;
use orbe_engine::timestamp::Timestamp;

/// A cycle of the document's 16 MHz crystal.
const CYCLE_PS: u64 = 62_500;

fn read(path: &str) -> String {
    fs::read_to_string(repo(path)).unwrap()
}

fn documented() -> String {
    let doc = read("docs/engine/netlist.md");
    let section = &doc[doc.find("### ATmega328P").unwrap()..];
    let start = section.find("```json\n").unwrap() + "```json\n".len();
    let end = start + section[start..].find("\n```").unwrap();
    section[start..end].to_string()
}

#[test]
fn the_documented_example_runs_with_no_diagnostics() {
    let done = orbe_engine::run(documented().as_bytes(), Timestamp::from_unix_seconds(0));
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
    assert_eq!(done.results.trace().unwrap().time().len(), 21);
}

fn chip(netlist: &mut serde_json::Value) -> &mut serde_json::Value {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["kind"] == "atmega328p")
        .unwrap()
}

fn leakage() -> f64 {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    chip(&mut netlist)["leakage_resistance"]["value"]
        .as_f64()
        .unwrap()
}

fn value(results: &orbe_engine::run_folder::Results, reference: &str) -> f64 {
    let op = results.observations().op().unwrap();
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

/// The documented example at its operating point, with `parts` added on `nets`.
fn probe(
    nets: &[&str],
    parts: serde_json::Value,
    edit: impl FnOnce(&mut serde_json::Value),
) -> orbe_engine::Completed {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    netlist["analysis"] = serde_json::json!({ "op": {} });
    for net in nets {
        netlist["nets"].as_array_mut().unwrap().push((*net).into());
    }
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .extend(parts.as_array().unwrap().iter().cloned());
    edit(chip(&mut netlist));
    orbe_engine::run(
        netlist.to_string().as_bytes(),
        Timestamp::from_unix_seconds(0),
    )
}

#[test]
fn the_documented_part_is_the_tests_fixture() {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    let fixture: serde_json::Value = serde_json::from_str(&read(
        "packages/engine/tests/netlist/fixtures/atmega328p.json",
    ))
    .unwrap();
    assert_eq!(*chip(&mut netlist), fixture);
}

#[test]
fn the_documented_firmware_is_toggle_hex() {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    let chip = chip(&mut netlist);
    assert_eq!(
        records(chip["firmware"].as_str().unwrap()),
        records(&read("packages/engine/tests/firmware/toggle.hex"))
    );
}

/// Every accepted step of the documented example, with PB5's volts.
fn pb5_steps() -> Vec<(u64, f64)> {
    let netlist = orbe_engine::netlist::parse(documented().as_bytes()).unwrap();
    let results = orbe_engine::simulate_steps(&netlist);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let trace = results.trace().unwrap();
    let pb5: Ref = "#net.pb5".parse().unwrap();
    let column = trace.columns().iter().position(|r| *r == pb5).unwrap();
    trace
        .time()
        .iter()
        .map(|at| at.as_ps())
        .zip(trace.column(column).iter().copied())
        .collect()
}

/// `toggle.S` drives PB5 low at cycle 2, onto a pin its load already holds at 0 V, then toggles
/// it at cycle 4 and every 100 cycles. Each edge is a row on the old level at its cycle's
/// picosecond, and the step after it holds the new one.
#[test]
fn pb5_switches_at_the_picosecond_of_each_cycle_the_firmware_implies() {
    let rows = pb5_steps();
    let edges: Vec<(u64, bool)> = rows
        .windows(2)
        .filter(|pair| (pair[0].1 > 2.5) != (pair[1].1 > 2.5))
        .map(|pair| (pair[0].0, pair[1].1 > 2.5))
        .collect();
    let leak = 1.0 / leakage();
    let share = |up: f64, down: f64| 5.0 * up / (up + down);
    let levels = [
        share(leak, leak + 1e-3),
        share(leak, leak + 1e-3 + 1.0 / 25.0),
        share(leak + 1.0 / 25.0, leak + 1e-3),
    ];
    assert!(
        rows.iter()
            .all(|&(_, v)| levels.iter().any(|level| (v - level).abs() < 1e-9)),
        "off, low, or high on 1 kohm, each with the pin's leakage"
    );
    assert_eq!(
        edges,
        [
            (4 * CYCLE_PS, true),
            (104 * CYCLE_PS, false),
            (204 * CYCLE_PS, true),
            (304 * CYCLE_PS, false),
        ]
    );
}

/// The run folder's `logic.vcd` read back, and its `observations.json`.
fn logic(done: &orbe_engine::Completed) -> (serde_json::Value, serde_json::Value) {
    let vcd = done.folder.file("traces/logic.vcd").unwrap();
    let vcd = read_vcd(std::str::from_utf8(vcd).unwrap());
    let observations = done.folder.file("observations.json").unwrap();
    (vcd, serde_json::from_slice(observations).unwrap())
}

/// `(t_ps, high)` of each change `read_vcd` found on `signal`.
fn vcd_edges(vcd: &serde_json::Value, signal: &str) -> Vec<(u64, bool)> {
    vcd[signal]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| (edge["t_ps"].as_u64().unwrap(), edge["level"] == "high"))
        .collect()
}

/// In `toggle.S`, `SBI DDRB,5` and the first `SBI PINB,5` take two cycles each, and a pass of
/// its loop takes 2 + 1 + 3 * 31 - 1 + 3 + 2 (DS40002198B, AVRe column).
#[test]
fn the_documented_logic_vcd_has_pb5_change_where_its_node_crosses() {
    let (first, pass) = (2 + 2, 2 + 1 + 3 * 31 - 1 + 3 + 2);
    let edges: Vec<(u64, bool)> = (0..4)
        .map(|k| (first + pass * k) * CYCLE_PS)
        .zip([true, false, true, false])
        .collect();
    let crossings: Vec<(u64, bool)> = pb5_steps()
        .windows(2)
        .filter(|pair| (pair[0].1 > 2.5) != (pair[1].1 > 2.5))
        .map(|pair| (pair[0].0, pair[1].1 > 2.5))
        .collect();
    assert_eq!(crossings, edges);

    let done = orbe_engine::run(documented().as_bytes(), Timestamp::from_unix_seconds(0));
    let (vcd, observations) = logic(&done);
    assert_eq!(vcd_edges(&vcd, "#u1.logic.PB5"), edges);
    assert_eq!(summary(&vcd), observations["logic"]);
    let quiet = observations["logic"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, signal)| signal["count"] == 0)
        .count();
    assert_eq!(quiet, 21, "every signal but PB5 holds its level");
    let index: serde_json::Value =
        serde_json::from_slice(done.folder.file("traces/index.json").unwrap()).unwrap();
    let signals: Vec<&str> = index["logic"]["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|signal| signal["ref"].as_str().unwrap())
        .collect();
    let keys: Vec<&str> = observations["logic"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(signals, keys);
    assert_eq!(signals.len(), 22);
}

/// packages/engine/tests/circuits/ne555-led beside the documented ATmega328P, each of its parts and nets but ground
/// renamed with an `x` in front, and its timing capacitor 1e4 times smaller, so its OUT turns
/// within the run. `xu1` sorts after `u1`, so the passive chip's signals follow the active one's.
#[test]
fn an_ne555_and_an_atmega328p_in_one_netlist_both_write_their_signals() {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    netlist["analysis"]["transient"]["stop_ps"] = 100_000_000.into();
    let astable: serde_json::Value = serde_json::from_str(&read(
        "packages/engine/tests/circuits/ne555-led/netlist.json",
    ))
    .unwrap();
    let rename = |net: &serde_json::Value| -> serde_json::Value {
        match net.as_str().unwrap() {
            "gnd" => "gnd".into(),
            net => format!("x{net}").into(),
        }
    };
    for net in astable["nets"].as_array().unwrap() {
        if net != "gnd" {
            netlist["nets"].as_array_mut().unwrap().push(rename(net));
        }
    }
    for part in astable["parts"].as_array().unwrap() {
        let mut part = part.clone();
        part["name"] = format!("x{}", part["name"].as_str().unwrap()).into();
        for net in part["pins"].as_object_mut().unwrap().values_mut() {
            *net = rename(net);
        }
        if part["name"] == "xc1" {
            part["capacitance"]["value"] = 1e-9.into();
        }
        netlist["parts"].as_array_mut().unwrap().push(part);
    }
    let done = orbe_engine::run(
        netlist.to_string().as_bytes(),
        Timestamp::from_unix_seconds(0),
    );
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
    let (vcd, observations) = logic(&done);
    assert_eq!(summary(&vcd), observations["logic"]);
    let keys: Vec<&String> = observations["logic"].as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 26);
    assert_eq!(
        keys[21..],
        [
            "#u1.logic.VCC",
            "#xu1.logic.out",
            "#xu1.logic.reset",
            "#xu1.logic.thres",
            "#xu1.logic.trig"
        ]
    );
    assert_eq!(vcd_edges(&vcd, "#u1.logic.PB5").len(), 16);
    assert!(vcd_edges(&vcd, "#xu1.logic.out").len() >= 2);
    let text = std::str::from_utf8(done.folder.file("traces/logic.vcd").unwrap()).unwrap();
    let u1 = text.find("$scope module u1 $end").unwrap();
    let xu1 = text.find("$scope module xu1 $end").unwrap();
    assert!(u1 < xu1);
}

/// 1 uA, the leakage bound of Table 30-1, into an off pin through 1 kohm. The pin's leakage to
/// each rail holds it, so the net solves at 2.5 V plus 1 uA through half the leakage
/// resistance and the 1 kohm.
#[test]
fn a_net_held_only_by_an_off_pin_solves_on_its_leakage() {
    let parts = serde_json::json!([
        { "name": "i1", "kind": "current_source", "pins": { "p": "x", "n": "gnd" },
          "current": { "value": 1e-6, "unit": "A", "source": "user" } },
        { "name": "rx", "kind": "resistor", "pins": { "a": "x", "b": "pd3" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null } }
    ]);
    let done = probe(&["x"], parts, |_| {});
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
    let expected = 2.5 + 1e-6 * (leakage() / 2.0 + 1000.0);
    let x = value(&done.results, "#net.x");
    assert!(
        (x / expected - 1.0).abs() < 1e-5,
        "{x} within gmin's share of {expected}"
    );
}

#[test]
fn a_ground_pin_on_a_net_of_its_own_is_an_error() {
    let parts = serde_json::json!([
        { "name": "ra", "kind": "resistor", "pins": { "a": "vcc", "b": "agnd" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null } }
    ]);
    let done = probe(&["agnd"], parts, |chip| {
        chip["pins"]["GND_22"] = "agnd".into()
    });
    let codes: Vec<&str> = done
        .results
        .diagnostics()
        .iter()
        .map(|d| d.code())
        .collect();
    assert_eq!(codes, ["netlist.same_net"]);
}
