//! The development board fragment in `packages/engine/tests/circuits/atmega-board-blink`.
//! The LED, D0, the reset release, and the pin-13 pull-up are closed forms.
//! `packages/engine/tests/circuits/atmega-board-button` adds a switch and a pull-down. Its LED lags are recorded.
//! A live session advances, then sets `usb_dtr` to 0 V, and the blink sketch restarts.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use orbe_model::Time;

use crate::runs::common;
use common::examples::examples;
use common::read_vcd;
use common::{records, repo};
use orbe_engine::finite::Finite;
use orbe_engine::name::{Name, Ref};
use orbe_engine::run_folder::RunFolder;
use orbe_engine::timestamp::Timestamp;
use orbe_engine::{Input, Session, Setting, Status};

/// `k * T / q` at 298.15 K, with the exact SI values.
const VT: f64 = 1.380649e-23 * 298.15 / 1.602176634e-19;
/// The engine's conductance from every node to ground, and across a junction.
const GMIN: f64 = 1e-12;
/// Larger measured gap at `grid_ps` of 1e9 is 69612 ps. This is that gap plus 130 ns.
const RELEASE_WINDOW_PS: u64 = 200_000;
/// External-reset extra cycles, from the crystal row of the fuse tables.
const RESET_CYCLES: u64 = 14;
/// Recorded from the engine. First PB5 rise of `packages/engine/tests/firmware/sketch/blink.hex`, in cycles. Not derived.
const SKETCH_CYCLES: u64 = 233;
/// Recorded from the engine at these switch times. Cycles from this example's switch close to the PB5 rise. Not derived.
const PRESS_LAG_CYCLES: u64 = 137;
/// Recorded from the engine at these switch times. Cycles from this example's switch open to the PB5 fall. Not derived.
const RELEASE_LAG_CYCLES: u64 = 85;
/// Recorded from the engine on the DTR rise. Not derived. The run peaked at 6.001514 V.
const CLAMP_V: f64 = 6.002;
const CLAMP_V_WINDOW: f64 = 0.002;
/// Recorded diode current at that peak. Not derived. The run peaked at 0.109632 A.
const CLAMP_A: f64 = 0.110;
const CLAMP_A_WINDOW: f64 = 0.002;

/// Cited fragment numbers. The release formula reads these, not the netlist a test just edited.
const CITED: &[(&str, &str, f64)] = &[
    ("usb_vbus", "voltage.value", 5.0),
    ("usb_fuse", "resistance.value", 0.15),
    ("usb_switch", "resistance.value", 0.06),
    ("reg_3v3", "output_voltage.value", 3.3),
    ("reg_3v3", "line_regulation.value", 0.000231),
    ("reg_3v3", "output_resistance.value", 0.3322147651006711),
    ("reg_3v3", "dropout_voltage.value", 0.0051677852348993285),
    ("reg_3v3", "dropout_resistance.value", 1.8322147651006713),
    ("reg_3v3", "current_limit.value", 0.35),
    ("reg_3v3", "quiescent_current.value", 6.5e-5),
    (
        "reg_3v3",
        "ground_current_ratio.value",
        0.005233333333333334,
    ),
    ("reg_3v3", "ratings.current.value", 0.15),
    ("reg_3v3", "ratings.power.value", 0.48685491723466406),
    ("c3", "capacitance.value", 1e-6),
    ("mcu", "clock_hz.value", 16_000_000.0),
    ("mcu", "fuses.low", 255.0),
    ("mcu", "fuses.high", 223.0),
    ("mcu", "fuses.extended", 253.0),
    ("mcu", "output.high_drop.value", 0.0),
    ("mcu", "output.high_resistance.value", 25.0),
    ("mcu", "output.low_drop.value", 0.0),
    ("mcu", "output.low_resistance.value", 25.0),
    ("mcu", "pull_up_resistance.value", 35_714.0),
    ("mcu", "reset_pull_up_resistance.value", 46_296.0),
    ("mcu", "leakage_resistance.value", 5_500_000.0),
    ("mcu", "input_threshold.low.value", 0.42),
    ("mcu", "input_threshold.high.value", 0.52),
    ("mcu", "reset_threshold.low.value", 0.42),
    ("mcu", "reset_threshold.high.value", 0.44),
    ("res_xtal1", "capacitance.value", 1.5e-11),
    ("res_xtal1", "initial.value", 0.0),
    ("res_xtal2", "capacitance.value", 1.5e-11),
    ("res_xtal2", "initial.value", 0.0),
    ("c_aref", "capacitance.value", 1e-7),
    ("c_aref", "initial.value", 0.0),
    ("usb_tx", "voltage.mark.value", 5.0),
    ("usb_tx", "voltage.space.value", 0.0),
    ("r_rx", "resistance.value", 1000.0),
    ("usb_dtr", "voltage.initial.value", 5.0),
    ("dtr_out", "resistance.value", 36.0),
    ("r_dtr_boot", "resistance.value", 1000.0),
    ("c_reset", "capacitance.value", 1e-7),
    ("r_reset", "resistance.value", 10_000.0),
    ("d_reset", "saturation_current.value", 1.341e-12),
    ("d_reset", "emission_coefficient.value", 1.391),
    ("d_reset", "series_resistance.value", 0.7523),
    ("d_reset", "ratings.current.value", 4.0),
    ("d_reset", "ratings.power.value", 0.4),
    ("sw_reset", "on_resistance.value", 0.1),
    ("sw_reset", "off_resistance.value", 1e8),
    ("r_l", "resistance.value", 1000.0),
    ("led_l", "saturation_current.value", 6.711e-10),
    ("led_l", "emission_coefficient.value", 4.427),
    ("led_l", "series_resistance.value", 2.115),
    ("led_l", "ratings.current.value", 0.03),
    ("led_l", "ratings.power.value", 0.075),
    ("r_on_a", "resistance.value", 1000.0),
    ("r_on_b", "resistance.value", 1000.0),
    ("led_on", "saturation_current.value", 6.711e-10),
    ("led_on", "emission_coefficient.value", 4.427),
    ("led_on", "series_resistance.value", 2.115),
    ("led_on", "ratings.current.value", 0.03),
    ("led_on", "ratings.power.value", 0.075),
];

fn cited(name: &str, path: &str) -> f64 {
    CITED
        .iter()
        .find(|(part, key, _)| *part == name && *key == path)
        .unwrap_or_else(|| panic!("missing cited {name} {path}"))
        .2
}

fn number_at(value: &serde_json::Value, path: &str) -> f64 {
    let got = path.split('.').fold(value, |node, key| &node[key]);
    got.as_f64()
        .unwrap_or_else(|| panic!("{path} is not a number"))
}

fn fragment_path() -> std::path::PathBuf {
    repo("packages/engine/tests/circuits/atmega-board-blink/netlist.json")
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn fragment() -> serde_json::Value {
    read_json(&fragment_path())
}

fn part<'a>(netlist: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn part_mut<'a>(netlist: &'a mut serde_json::Value, name: &str) -> &'a mut serde_json::Value {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn board_examples() -> Vec<std::path::PathBuf> {
    examples()
        .into_iter()
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("atmega-board-")
        })
        .collect()
}

/// Drop the four fields a board example is allowed to change.
fn stripped(part: &serde_json::Value) -> serde_json::Value {
    let mut part = part.clone();
    match part["name"].as_str() {
        Some("mcu") => part["firmware"] = serde_json::Value::Null,
        Some("usb_tx") | Some("usb_dtr") => part["voltage"]["events"] = serde_json::Value::Null,
        Some("sw_reset") => part["events"] = serde_json::Value::Null,
        _ => {}
    }
    part
}

fn walk_values(value: &serde_json::Value, at: &str) {
    match value {
        serde_json::Value::Object(map) => {
            if map.contains_key("value") && map.contains_key("unit") && map.contains_key("source") {
                assert_ne!(map["source"], "user", "{at}");
            }
            for (key, child) in map {
                walk_values(child, &format!("{at}.{key}"));
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                walk_values(child, &format!("{at}[{index}]"));
            }
        }
        _ => {}
    }
}

fn run(netlist: &serde_json::Value) -> orbe_engine::Completed {
    orbe_engine::run(
        &serde_json::to_vec(netlist).unwrap(),
        Timestamp::from_unix_seconds(0),
    )
}

fn column_index(trace: &orbe_engine::trace::Trace, name: &str) -> usize {
    trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn row_at(trace: &orbe_engine::trace::Trace, ps: u64) -> usize {
    trace
        .time()
        .iter()
        .position(|time| time.as_ps() == ps)
        .unwrap_or_else(|| panic!("missing row at {ps}"))
}

fn sample(trace: &orbe_engine::trace::Trace, name: &str, ps: u64) -> f64 {
    trace.column(column_index(trace, name))[row_at(trace, ps)]
}

/// Within Newton's 1e-6 plus the 1 pA floor.
fn near(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-6 * want.abs() + 1e-12
}

fn cycle_ps(netlist: &serde_json::Value) -> u64 {
    let hz = part(netlist, "mcu")["clock_hz"]["value"].as_u64().unwrap();
    assert_eq!(1_000_000_000_000 % hz, 0);
    1_000_000_000_000 / hz
}

fn logic(folder: &RunFolder, key: &str) -> Vec<(u64, bool)> {
    let text = std::str::from_utf8(folder.file("traces/logic.vcd").unwrap()).unwrap();
    let vcd = read_vcd(text);
    vcd[key]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| (edge["t_ps"].as_u64().unwrap(), edge["level"] == "high"))
        .collect()
}

struct Release {
    tau: f64,
    vstep: f64,
    vinf: f64,
    vlogic: f64,
    couple: f64,
    vr_closed: f64,
}

/// First-order release. The diode stays reverse-biased until `0.44 * v5`, so its current is
/// `Is` into `reset` and its `GMIN` joins the pull-up. `Rs * Is` is about 1e-12 V and the
/// reverse cubic is a few parts in 1e6 on this swing, so both stay out of the time constant.
/// `C`, the resistors, and `0.44` come from [`CITED`], not from the netlist under test.
fn release_network(v5: f64) -> Release {
    let rd = cited("dtr_out", "resistance.value");
    let rb = cited("r_dtr_boot", "resistance.value");
    let c = cited("c_reset", "capacitance.value");
    let r_ext = cited("r_reset", "resistance.value");
    let r_pu = cited("mcu", "reset_pull_up_resistance.value");
    let leak = cited("mcu", "leakage_resistance.value");
    let ron = cited("sw_reset", "on_resistance.value");
    let roff = cited("sw_reset", "off_resistance.value");
    let is = cited("d_reset", "saturation_current.value");
    let frac = cited("mcu", "reset_threshold.high.value");
    let vs = cited("usb_dtr", "voltage.initial.value");
    let g_to_v5 = 1.0 / r_ext + 1.0 / r_pu + 1.0 / leak + GMIN;
    let g_to_gnd = 1.0 / leak + 1.0 / roff + GMIN;
    let g = g_to_v5 + g_to_gnd;
    let rth = 1.0 / g;
    let gp = 1.0 / rd + 1.0 / rb + GMIN;
    let rp = 1.0 / gp;
    let couple = rth / (rth + rp);
    Release {
        tau: c * (rth + rp),
        vstep: vs * couple * (rp / rd),
        vinf: (g_to_v5 * v5 + is) / g,
        vlogic: frac * v5,
        couple,
        vr_closed: (g_to_v5 * v5 + is) / (g_to_v5 + 1.0 / leak + 1.0 / ron + GMIN),
    }
}

fn dtr_release_ps(v5: f64) -> u64 {
    let net = release_network(v5);
    (net.tau * libm::log(net.vstep / (net.vinf - net.vlogic)) * 1e12).round() as u64
}

fn button_release_ps(v5: f64) -> u64 {
    let net = release_network(v5);
    let arg = net.couple * (net.vinf - net.vr_closed) / (net.vinf - net.vlogic);
    (net.tau * libm::log(arg) * 1e12).round() as u64
}

fn same_json(got: &serde_json::Value, want: &serde_json::Value) -> bool {
    match (got, want) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => a.as_f64() == b.as_f64(),
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(left, right)| same_json(left, right))
        }
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same_json(value, other)))
        }
        _ => got == want,
    }
}

/// `#led_l.i` while the pin sources. The stage and the leakage come from [`CITED`], not the netlist under test.
fn led_on_current(v5: f64) -> f64 {
    let leak = 1.0 / cited("mcu", "leakage_resistance.value");
    let stage = 1.0 / cited("mcu", "output.high_resistance.value");
    led_current(v5, leak + stage, leak + GMIN)
}

fn led_current(v5: f64, up: f64, down: f64) -> f64 {
    let r = 1000.0;
    let (is, n, rs) = (6.711e-10, 4.427, 2.115);
    let (source, behind) = (v5 * up / (up + down), 1.0 / (up + down));
    let current = |vj: f64| is * libm::expm1(vj / (n * VT)) + GMIN * vj;
    let (mut low, mut high) = (0.0, source);
    for _ in 0..200 {
        let vj = (low + high) / 2.0;
        let anode = vj + rs * current(vj);
        if anode + (r + behind) * (current(vj) + GMIN * anode) < source {
            low = vj;
        } else {
            high = vj;
        }
    }
    current(low)
}

fn pin_with_pullup(v5: f64) -> f64 {
    let (is, n, rs) = (6.711e-10, 4.427, 2.115);
    let pull = 35_714.0;
    let series = 1000.0;
    let leak = 5.5e6;
    let (mut low, mut high) = (0.0, v5);
    for _ in 0..200 {
        let vj = (low + high) / 2.0;
        let ij = is * libm::expm1(vj / (n * VT)) + GMIN * vj;
        let anode = vj + rs * ij;
        let branch = ij + GMIN * anode;
        let pin = anode + series * branch;
        let from_chip = (v5 - pin) / pull + (v5 - pin) / leak - pin / leak - GMIN * pin;
        if from_chip > branch {
            low = vj;
        } else {
            high = vj;
        }
    }
    let vj = low;
    let ij = is * libm::expm1(vj / (n * VT)) + GMIN * vj;
    let anode = vj + rs * ij;
    anode + series * (ij + GMIN * anode)
}

fn assert_quiet(done: &orbe_engine::Completed) {
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
}

/// PC6 falls at `fall_ps`, then rises `want_ps` after `charge_ps`, within the window.
/// The next PB5 rise is the fuse delay plus the sketch offset, within one cycle.
fn assert_restart(folder: &RunFolder, fall_ps: u64, charge_ps: u64, cycle: u64, want_ps: u64) {
    let pc6 = logic(folder, "#mcu.logic.PC6");
    assert_eq!(
        pc6.first().copied(),
        Some((fall_ps, false)),
        "PC6 did not fall at the input"
    );
    let release = pc6
        .iter()
        .find(|(at, high)| *high && *at >= charge_ps)
        .unwrap()
        .0;
    let got = release - charge_ps;
    assert!(
        got.abs_diff(want_ps) <= RELEASE_WINDOW_PS,
        "release {got} ps, closed form {want_ps} ps"
    );
    let pb5 = logic(folder, "#mcu.logic.PB5");
    let offset = pb5.iter().find(|(_, high)| *high).unwrap().0;
    assert_eq!(offset, SKETCH_CYCLES * cycle);
    let (restart, high) = pb5.iter().find(|(at, _)| *at > release).copied().unwrap();
    assert!(high, "PB5 fell after reset instead of rising");
    let expect = release + 65_000_000_000 + (RESET_CYCLES + SKETCH_CYCLES) * cycle;
    assert!(
        restart.abs_diff(expect) <= cycle,
        "PB5 at {restart} ps, want {expect} ps, one cycle is {cycle} ps"
    );
}

#[test]
fn every_board_example_keeps_the_fragment() {
    let fragment = fragment();
    let want_nets: BTreeSet<_> = fragment["nets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|net| net.as_str().unwrap().to_string())
        .collect();
    let want_parts: BTreeMap<_, _> = fragment["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| (part["name"].as_str().unwrap().to_string(), stripped(part)))
        .collect();
    let boards = board_examples();
    assert!(
        boards
            .iter()
            .any(|path| path.ends_with("atmega-board-blink"))
    );
    for path in boards {
        let netlist = read_json(&path.join("netlist.json"));
        let name = path.file_name().unwrap().to_str().unwrap();
        let nets: BTreeSet<_> = netlist["nets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|net| net.as_str().unwrap().to_string())
            .collect();
        assert!(want_nets.is_subset(&nets), "{name}");
        for (part_name, want) in &want_parts {
            let got = stripped(part(&netlist, part_name));
            assert!(same_json(&got, want), "{name} {part_name}\n{got}\n{want}");
        }
    }
}

#[test]
fn no_fragment_value_has_source_user() {
    let fragment = fragment();
    let names: Vec<_> = fragment["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part["name"].as_str().unwrap().to_string())
        .collect();
    for path in board_examples() {
        let netlist = read_json(&path.join("netlist.json"));
        let example = path.file_name().unwrap().to_str().unwrap();
        for name in &names {
            walk_values(part(&netlist, name), &format!("{example}.{name}"));
        }
    }
}

#[test]
fn the_blink_run_matches_the_led_and_d0() {
    let netlist = fragment();
    assert_eq!(
        records(part(&netlist, "mcu")["firmware"].as_str().unwrap()),
        records(
            &fs::read_to_string(repo("packages/engine/tests/firmware/sketch/blink.hex")).unwrap()
        )
    );
    let done = run(&netlist);
    assert_quiet(&done);
    let trace = done.results.trace().unwrap();
    let high_ps = 500_000_000_000;
    let v5 = sample(trace, "#net.v5", high_ps);
    let got = sample(trace, "#led_l.i", high_ps);
    let want = led_on_current(v5);
    assert!(
        near(got, want),
        "LED {got} A, closed form {want} A at {v5} V"
    );
    let idle_v5 = sample(trace, "#net.v5", 0);
    let mark = 5.0;
    let series = 1000.0;
    let leak_r = 5.5e6;
    let d0 = (mark / series + idle_v5 / leak_r) / (1.0 / series + 2.0 / leak_r + GMIN);
    let got = sample(trace, "#net.d0", 0);
    assert!(
        (got - d0).abs() <= 1e-8 * d0,
        "D0 {got} V, closed form {d0} V"
    );
}

#[test]
fn a_dtr_fall_resets_within_the_window() {
    let mut netlist = fragment();
    let source = part(&netlist, "usb_dtr")["voltage"]["initial"]["source"].clone();
    let level = |volts: f64| {
        serde_json::json!({
            "value": volts,
            "unit": "V",
            "source": source.clone()
        })
    };
    part_mut(&mut netlist, "usb_dtr")["voltage"]["events"] = serde_json::json!([
        { "t_ps": 1_500_000_000_000_u64, "level": level(0.0) },
        { "t_ps": 2_500_000_000_000_u64, "level": level(5.0) }
    ]);
    netlist["analysis"]["transient"]["stop_ps"] = serde_json::Value::from(3_000_000_000_000_u64);
    netlist["analysis"]["transient"]["grid_ps"] = serde_json::Value::from(1_000_000_000_u64);
    let cycle = cycle_ps(&netlist);
    let done = run(&netlist);
    assert_quiet(&done);
    let fall = 1_500_000_000_000;
    let trace = done.results.trace().unwrap();
    let want = dtr_release_ps(sample(trace, "#net.v5", fall));
    assert_restart(&done.folder, fall, fall, cycle, want);
    let rise = 2_500_000_000_000;
    let interval = row_at(trace, rise);
    let volts = trace.max_column(column_index(trace, "#net.reset"))[interval];
    let amps = trace.max_column(column_index(trace, "#d_reset.i"))[interval];
    assert!(
        (volts - CLAMP_V).abs() <= CLAMP_V_WINDOW,
        "RESET peaked at {volts} V"
    );
    assert!(
        (amps - CLAMP_A).abs() <= CLAMP_A_WINDOW,
        "diode peaked at {amps} A"
    );
}

#[test]
fn the_reset_button_restarts_within_the_window() {
    let mut netlist = fragment();
    part_mut(&mut netlist, "sw_reset")["events"] = serde_json::json!([
        { "t_ps": 1_500_000_000_000_u64, "state": "closed" },
        { "t_ps": 1_600_000_000_000_u64, "state": "open" }
    ]);
    netlist["analysis"]["transient"]["stop_ps"] = serde_json::Value::from(2_000_000_000_000_u64);
    netlist["analysis"]["transient"]["grid_ps"] = serde_json::Value::from(1_000_000_000_u64);
    let cycle = cycle_ps(&netlist);
    let done = run(&netlist);
    assert_quiet(&done);
    let open = 1_600_000_000_000;
    let trace = done.results.trace().unwrap();
    let want = button_release_ps(sample(trace, "#net.v5", open));
    assert_restart(&done.folder, 1_500_000_000_000, open, cycle, want);
}

#[test]
fn the_fragment_matches_the_cited_values() {
    let netlist = fragment();
    for (name, path, want) in CITED {
        let got = number_at(part(&netlist, name), path);
        assert_eq!(got, *want, "{name} {path}");
    }
}

#[test]
fn pin_13_with_its_pullup_reads_low() {
    let mut netlist = fragment();
    part_mut(&mut netlist, "mcu")["firmware"] = serde_json::json!(
        fs::read_to_string(repo("packages/engine/tests/firmware/pullup.hex")).unwrap()
    );
    netlist["analysis"]["transient"]["stop_ps"] = serde_json::Value::from(200_000_000_000_u64);
    netlist["analysis"]["transient"]["grid_ps"] = serde_json::Value::from(1_000_000_000_u64);
    let done = run(&netlist);
    assert_quiet(&done);
    let text = std::str::from_utf8(done.folder.file("traces/logic.vcd").unwrap()).unwrap();
    let vcd = read_vcd(text);
    assert_eq!(vcd["#mcu.logic.PB5"]["initial"], "low");
    assert!(
        vcd["#mcu.logic.PB5"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["level"] == "low")
    );
    let trace = done.results.trace().unwrap();
    let at = 150_000_000_000;
    let v5 = sample(trace, "#net.v5", at);
    let got = sample(trace, "#net.d13", at);
    let want = pin_with_pullup(v5);
    assert!(near(got, want), "D13 {got} V, closed form {want} V");
    assert!(
        got < 0.42 * v5,
        "D13 {got} V reads high against 0.42 of {v5} V"
    );
}

#[test]
fn the_serial_host_is_echoed() {
    let mut netlist = fragment();
    part_mut(&mut netlist, "mcu")["firmware"] = serde_json::json!(
        fs::read_to_string(repo("packages/engine/tests/firmware/serial_echo.hex")).unwrap()
    );
    part_mut(&mut netlist, "usb_tx")["voltage"]["events"] = serde_json::json!([
        { "t_ps": 1_000_000_000_u64, "baud": 9600, "frame": "8N1", "bytes": [49] },
        { "t_ps": 5_000_000_000_u64, "baud": 9600, "frame": "8N1", "bytes": [48, 104, 105] }
    ]);
    netlist["analysis"]["transient"]["stop_ps"] = serde_json::Value::from(20_000_000_000_u64);
    netlist["analysis"]["transient"]["grid_ps"] = serde_json::Value::from(1_000_000_000_u64);
    let done = run(&netlist);
    assert_quiet(&done);
    let text = std::str::from_utf8(done.folder.file("traces/serial.csv").unwrap()).unwrap();
    let bytes: Vec<u8> = text
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut cols = line.split(',');
            let _at = cols.next();
            assert_eq!(cols.next(), Some("mcu"));
            assert_eq!(cols.next(), Some("USART0"));
            cols.next().unwrap().parse().unwrap()
        })
        .collect();
    assert_eq!(bytes, vec![49, 48, 104, 105]);
}

#[test]
fn the_led_follows_the_button() {
    let netlist = read_json(&repo(
        "packages/engine/tests/circuits/atmega-board-button/netlist.json",
    ));
    assert_eq!(
        records(part(&netlist, "mcu")["firmware"].as_str().unwrap()),
        records(&fs::read_to_string(repo("blocks/nano-button-led/firmware/button.hex")).unwrap())
    );
    assert_eq!(
        netlist["analysis"]["transient"]["stop_ps"],
        20_000_000_000_u64
    );
    assert_eq!(
        netlist["analysis"]["transient"]["grid_ps"],
        1_000_000_000_u64
    );
    let events = part(&netlist, "sw1")["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "sw1 has no closed interval");
    assert_eq!(events[0]["state"], "closed");
    assert_eq!(events[1]["state"], "open");
    let close = events[0]["t_ps"].as_u64().unwrap();
    let open = events[1]["t_ps"].as_u64().unwrap();
    assert!(close < open, "sw1 has no closed interval");
    let cycle = cycle_ps(&netlist);
    let done = run(&netlist);
    assert_quiet(&done);
    assert_eq!(
        logic(&done.folder, "#mcu.logic.PB5"),
        vec![
            (close + PRESS_LAG_CYCLES * cycle, true),
            (open + RELEASE_LAG_CYCLES * cycle, false),
        ]
    );
    assert_eq!(
        logic(&done.folder, "#mcu.logic.PD2"),
        vec![(close, true), (open, false)]
    );
    let trace = done.results.trace().unwrap();
    let on_ps = 10_000_000_000;
    let off_ps = 18_000_000_000;
    assert!(
        close < on_ps && on_ps < open,
        "{on_ps} is outside the press"
    );
    assert!(open < off_ps, "{off_ps} is not after the release");
    let v5 = sample(trace, "#net.v5", on_ps);
    let got = sample(trace, "#led_l.i", on_ps);
    let want = led_on_current(v5);
    assert!(
        near(got, want),
        "LED {got} A, closed form {want} A at {v5} V"
    );
    let off = sample(trace, "#led_l.i", off_ps);
    assert!(off < 1e-9, "LED {off} A while the button is open");
}

/// The apply lands at the time the session already stands at. A future apply is only a scheduled event.
fn live_dtr_fall(target_ps: u64, grid_ps: u64, on_row: bool) {
    assert_eq!(
        target_ps.is_multiple_of(grid_ps),
        on_row,
        "fall {target_ps} grid {grid_ps}"
    );
    let mut json = fragment();
    json["analysis"]["transient"]["stop_ps"] = serde_json::Value::from(2_000_000_000_000_u64);
    json["analysis"]["transient"]["grid_ps"] = serde_json::Value::from(grid_ps);
    let bytes = serde_json::to_vec(&json).unwrap();
    let mut session = Session::record_document(&bytes).unwrap();
    let fall = Time::from_ps(target_ps);
    assert_eq!(
        session.advance_to(fall),
        Status::Running,
        "the session did not advance"
    );
    assert_eq!(session.now(), fall, "the session was not at the fall");
    let applied = session
        .apply(&Input {
            part: Name::new("usb_dtr").unwrap(),
            at: session.now(),
            setting: Setting::Level(Finite::new(0.0).unwrap()),
        })
        .unwrap();
    assert_eq!(applied.at, fall, "DTR did not land at the current time");
    let probe = session.probe();
    assert_eq!(probe.at(), fall, "v5 is not the row at the fall");
    assert!(!probe.interpolated(), "v5 was interpolated");
    let column: Ref = "#net.v5".parse().unwrap();
    let v5 = probe.get(&column).expect("missing #net.v5").get();
    let folder = session.finish(Timestamp::from_unix_seconds(0)).unwrap();
    let diagnostics: serde_json::Value =
        serde_json::from_slice(folder.file("diagnostics.json").unwrap()).unwrap();
    assert_eq!(diagnostics.as_array().unwrap().len(), 0);
    assert_restart(
        &folder,
        fall.as_ps(),
        fall.as_ps(),
        cycle_ps(&json),
        dtr_release_ps(v5),
    );
}

#[test]
fn a_live_dtr_fall_restarts_the_sketch() {
    let grid = 1_000_000_000;
    live_dtr_fall(1_500_000_000_000, grid, true);
    live_dtr_fall(1_234_567_890_123, grid, false);
}

/// `sp(x) = ln(1 + e^x)`, branched so `exp` is never of a positive overflow.
fn softplus(x: f64) -> f64 {
    if x > 0.0 {
        x + libm::log1p(libm::exp(-x))
    } else {
        libm::log1p(libm::exp(x))
    }
}

/// Open-circuit output of the regulator law at `vi`, the current that `GMIN` draws there,
/// and `dvo/di` at that current. The constants are the law's.
fn regulator_open(reg: &orbe_engine::chip::LinearRegulator, vi: f64) -> (f64, f64, f64) {
    const KNEE_V: f64 = 1e-3;
    const FLOOR_OHM: f64 = 0.25;
    const LIMIT_FRACTION: f64 = 1e-3;
    const LIMIT_HEADROOM: f64 = 100.0;
    const BLOCK_OHM: f64 = 1e9;
    const BLOCK_A: f64 = 1e-12;
    let vout = reg.output_voltage.si.get();
    let k = reg.line_regulation.si.get();
    let ro = reg.output_resistance.si.get();
    let vdo = reg.dropout_voltage.si.get();
    let rdo = reg.dropout_resistance.si.get();
    let ilim = reg.current_limit.si.get();
    let law = |i: f64| {
        let d = LIMIT_FRACTION * ilim;
        let rlim = LIMIT_HEADROOM * vout / ilim;
        let a = vout + k * (vi - vout) - ro * i;
        let raw = vi - vdo - rdo * i;
        let floor = -FLOOR_OHM * i;
        let b = floor + KNEE_V * softplus((raw - floor) / KNEE_V);
        let vo_min = a - KNEE_V * softplus((a - b) / KNEE_V);
        let over = (i - ilim) / d;
        let back = -i / BLOCK_A;
        vo_min - rlim * d * softplus(over) + BLOCK_OHM * BLOCK_A * softplus(back)
    };
    let mut i = GMIN * vout;
    for _ in 0..20 {
        i = GMIN * law(i);
    }
    let volts = law(i);
    let step = 1e-15;
    let slope = (law(i + step) - law(i - step)) / (2.0 * step);
    (volts, i, slope)
}

/// `v3v3` stays between the lowest and highest open-circuit setpoints the run visits.
///
/// The output cannot sink when its only load is `GMIN`, so a fall in the setpoint
/// settles with a time constant of about 36 s.
#[test]
fn v3v3_stays_between_the_setpoints_the_run_visits() {
    let bytes = fs::read(fragment_path()).unwrap();
    let netlist = orbe_engine::netlist::parse(&bytes).unwrap();
    let (results, stats) = orbe_engine::simulate_stats(&netlist);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let folder = orbe_engine::run_folder::render(
        &bytes,
        Some(&netlist),
        &results,
        Timestamp::from_unix_seconds(0),
    );
    let text = std::str::from_utf8(folder.file("traces/logic.vcd").unwrap()).unwrap();
    let vcd = read_vcd(text);
    let mut pin_edges = 0_u64;
    let mut pb5 = 0_u64;
    for (key, body) in vcd.as_object().unwrap() {
        let count = body["edges"]
            .as_array()
            .map(|edges| edges.len())
            .unwrap_or(0) as u64;
        if key.starts_with("#mcu.logic.") {
            pin_edges += count;
        }
        if key == "#mcu.logic.PB5" {
            pb5 = count;
        }
    }
    let trace = results.trace().unwrap();
    eprintln!("landings {}", stats.landings);
    eprintln!("stops {}", stats.stops);
    eprintln!("pb5_edges {pb5}");
    eprintln!("pin_edges {pin_edges}");
    assert_eq!(stats.landings, 778, "accepted steps after t = 0");
    assert_eq!(pb5, 4, "PB5 edges");
    assert_eq!(pin_edges, 4, "pin edges");
    let name = Name::new("reg_3v3").unwrap();
    let id = netlist.part_id(&name).unwrap();
    let orbe_engine::netlist::Model::LinearRegulator(reg) = &netlist.part(id).model else {
        panic!("reg_3v3 is not a linear regulator");
    };
    let high = 500_000_000_000;
    let host = sample(trace, "#usb_vbus.i", high);
    assert!((host + 0.009555890962038169).abs() <= 1e-12, "host {host}");
    let v5_col = trace.column(column_index(trace, "#net.v5"));
    let v3_col = trace.column(column_index(trace, "#net.v3v3"));
    let mut low = f64::INFINITY;
    let mut high_set = f64::NEG_INFINITY;
    for &v5 in v5_col {
        let (law, _, _) = regulator_open(reg, v5);
        low = low.min(law);
        high_set = high_set.max(law);
    }
    let (law0, _, slope0) = regulator_open(reg, v5_col[0]);
    let tau = 1e-6 * slope0.abs();
    eprintln!("setpoint_low {low} setpoint_high {high_set}");
    eprintln!("t0_law {law0} open_slope_ohm {slope0} tau_s {tau}");
    eprintln!("high_ic {}", sample(trace, "#c3.i", high));
    for (tag, ps) in [
        ("t0", 0_u64),
        ("high", 500_000_000_000),
        ("low", 1_500_000_000_000),
    ] {
        let v5 = sample(trace, "#net.v5", ps);
        let v3 = sample(trace, "#net.v3v3", ps);
        let (law, _, _) = regulator_open(reg, v5);
        eprintln!(
            "{tag} v5 {v5} v3v3 {v3} law {law} host {} led {} d0 {} dtr {} iin {} ic {}",
            sample(trace, "#usb_vbus.i", ps),
            sample(trace, "#led_l.i", ps),
            sample(trace, "#net.d0", ps),
            sample(trace, "#net.dtr", ps),
            sample(trace, "#reg_3v3.iin", ps),
            sample(trace, "#c3.i", ps),
        );
    }
    assert!(
        (v3_col[0] - law0).abs() <= 1e-9,
        "t0 v3v3 {} law {law0}",
        v3_col[0]
    );
    for (index, (&v3, time)) in v3_col.iter().zip(trace.time()).enumerate() {
        assert!(
            (low - 1e-9) <= v3 && v3 <= (high_set + 1e-9),
            "row {index} at {} ps v3 {v3} outside {low} to {high_set}",
            time.as_ps()
        );
    }
}
