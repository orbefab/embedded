//! The development board fragment in `examples/atmega-board-blink`.
//! The LED, D0, the reset release, and the pin-13 pull-up are closed forms.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use common::examples::examples;
use common::read_vcd;
use common::{records, repo};
use sim_core::timestamp::Timestamp;

/// `k * T / q` at 298.15 K, with the exact SI values.
const VT: f64 = 1.380649e-23 * 298.15 / 1.602176634e-19;
/// The engine's conductance from every node to ground, and across a junction.
const GMIN: f64 = 1e-12;
/// Larger measured gap at `grid_ps` of 1e9 is 69612 ps. This is that gap plus 130 ns.
const RELEASE_WINDOW_PS: u64 = 200_000;
/// External-reset extra cycles, from the crystal row of the fuse tables.
const RESET_CYCLES: u64 = 14;
/// Recorded from the engine. First PB5 rise of `firmware/sketch/blink.hex`, in cycles. Not derived.
const SKETCH_CYCLES: u64 = 233;
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
    ("reg_3v3", "voltage.value", 3.3),
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
    repo("examples/atmega-board-blink/netlist.json")
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

fn run(netlist: &serde_json::Value) -> sim_circuit::Completed {
    sim_cli::run(
        &serde_json::to_vec(netlist).unwrap(),
        Timestamp::from_unix_seconds(0),
    )
}

fn column_index(trace: &sim_core::trace::Trace, name: &str) -> usize {
    trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn row_at(trace: &sim_core::trace::Trace, ps: u64) -> usize {
    trace
        .time()
        .iter()
        .position(|time| time.as_ps() == ps)
        .unwrap_or_else(|| panic!("missing row at {ps}"))
}

fn sample(trace: &sim_core::trace::Trace, name: &str, ps: u64) -> f64 {
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

fn logic(done: &sim_circuit::Completed, key: &str) -> Vec<(u64, bool)> {
    let text = std::str::from_utf8(done.folder.file("traces/logic.vcd").unwrap()).unwrap();
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

fn assert_quiet(done: &sim_circuit::Completed) {
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
}

/// PC6 falls at `fall_ps`, then rises `want_ps` after `charge_ps`, within the window.
/// The next PB5 rise is the fuse delay plus the sketch offset, within one cycle.
fn assert_restart(
    done: &sim_circuit::Completed,
    fall_ps: u64,
    charge_ps: u64,
    cycle: u64,
    want_ps: u64,
) {
    let pc6 = logic(done, "#mcu.logic.PC6");
    assert_eq!(pc6[0], (fall_ps, false), "PC6 did not fall at the input");
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
    let pb5 = logic(done, "#mcu.logic.PB5");
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
        records(&fs::read_to_string(repo("firmware/sketch/blink.hex")).unwrap())
    );
    let done = run(&netlist);
    assert_quiet(&done);
    let trace = done.results.trace().unwrap();
    let high_ps = 500_000_000_000;
    let v5 = sample(trace, "#net.v5", high_ps);
    let leak = 1.0 / 5.5e6;
    let stage = 1.0 / 25.0;
    let got = sample(trace, "#led_l.i", high_ps);
    let want = led_current(v5, leak + stage, leak + GMIN);
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
    assert_restart(&done, fall, fall, cycle, want);
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
    assert_restart(&done, 1_500_000_000_000, open, cycle, want);
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
    part_mut(&mut netlist, "mcu")["firmware"] =
        serde_json::json!(fs::read_to_string(repo("firmware/pullup.hex")).unwrap());
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
    part_mut(&mut netlist, "mcu")["firmware"] =
        serde_json::json!(fs::read_to_string(repo("firmware/serial_echo.hex")).unwrap());
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
