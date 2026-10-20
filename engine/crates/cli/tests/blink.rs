//! examples/atmega-blink, `firmware/blink.S` on an ATmega328P at 16 MHz with an LED on PB5,
//! against the firmware's cycle counts and the closed form of the LED's current.

mod common;

use std::fs;
use std::process::Command;

use common::{read_vcd, records, repo, summary, transient_folder};

/// `k * T / q` at 298.15 K, with the exact SI values.
const VT: f64 = 1.380649e-23 * 298.15 / 1.602176634e-19;
/// The engine's conductance from every node to ground.
const GMIN: f64 = 1e-12;
/// The engine's first step after a stop.
const RESTART_PS: u64 = 1_000;

/// Within Newton's 1e-6 plus the 1 pA floor of docs/reference.md.
fn near(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-6 * want + 1e-12
}

fn netlist() -> serde_json::Value {
    serde_json::from_slice(&fs::read(repo("examples/atmega-blink/netlist.json")).unwrap()).unwrap()
}

fn part<'a>(netlist: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap()
}

fn value(field: &serde_json::Value) -> f64 {
    field["value"].as_f64().unwrap()
}

#[test]
fn the_chip_is_the_tests_fixture_running_blink_hex() {
    let netlist = netlist();
    let mut chip = part(&netlist, "u1").clone();
    let mut fixture: serde_json::Value = serde_json::from_slice(
        &fs::read(repo("crates/core/tests/fixtures/atmega328p.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        records(chip["firmware"].as_str().unwrap()),
        records(&fs::read_to_string(repo("firmware/blink.hex")).unwrap())
    );
    chip["firmware"] = serde_json::Value::Null;
    fixture["firmware"] = serde_json::Value::Null;
    assert_eq!(chip, fixture);
}

fn cycle_ps(netlist: &serde_json::Value) -> u64 {
    let hz = part(netlist, "u1")["clock_hz"]["value"].as_u64().unwrap();
    assert_eq!(1_000_000_000_000 % hz, 0);
    1_000_000_000_000 / hz
}

/// `SBI DDRB,5` takes two cycles (DS40002198B, AVRe column).
fn driven_ps(netlist: &serde_json::Value) -> u64 {
    2 * cycle_ps(netlist)
}

/// `(t_ps, high)` of every PB5 edge before `stop_ps`. In `blink.S`, `SBI DDRB,5` and the first
/// `SBI PINB,5` take two cycles each, and a pass of its loop takes
/// 2 + 3 + 5 * 1,599,998 - 1 + 4 + 2 (DS40002198B, AVRe column).
fn edges(netlist: &serde_json::Value) -> Vec<(u64, bool)> {
    let cycle_ps = cycle_ps(netlist);
    let stop_ps = netlist["analysis"]["transient"]["stop_ps"]
        .as_u64()
        .unwrap();
    let (first, pass) = (2 + 2, 2 + 3 + 5 * 1_599_998 - 1 + 4 + 2);
    (0..)
        .map(|k| ((first + pass * k) * cycle_ps, k % 2 == 0))
        .take_while(|&(at, _)| at < stop_ps)
        .collect()
}

/// The LED's current, through its `Rs`, while PB5's stage pulls up through `up` siemens and down
/// through `down`. The stage is a Thevenin source into `r1`, the LED's `Rs`, and its junction,
/// with gmin from the anode and from the junction's inner node to ground. Bisects on the
/// junction's voltage.
fn led_current(netlist: &serde_json::Value, up: f64, down: f64) -> f64 {
    let vcc = value(&part(netlist, "v1")["voltage"]);
    let r = value(&part(netlist, "r1")["resistance"]);
    let led = part(netlist, "led1");
    let (is, n, rs) = (
        value(&led["saturation_current"]),
        value(&led["emission_coefficient"]),
        value(&led["series_resistance"]),
    );
    let (source, behind) = (vcc * up / (up + down), 1.0 / (up + down));
    let current = |vj: f64| is * (vj / (n * VT)).exp_m1() + GMIN * vj;
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

/// The LED's current in each state of PB5, with the pin's leakage to both rails and gmin on PB5.
struct Levels {
    off: f64,
    low: f64,
    high: f64,
}

/// Both drops are 0 V, so each stage is a resistance to its rail.
fn led_levels(netlist: &serde_json::Value) -> Levels {
    let chip = part(netlist, "u1");
    assert_eq!(value(&chip["output"]["low_drop"]), 0.0);
    assert_eq!(value(&chip["output"]["high_drop"]), 0.0);
    let leak = 1.0 / value(&chip["leakage_resistance"]);
    let low = 1.0 / value(&chip["output"]["low_resistance"]);
    let high = 1.0 / value(&chip["output"]["high_resistance"]);
    Levels {
        off: led_current(netlist, leak, leak + GMIN),
        low: led_current(netlist, leak, leak + GMIN + low),
        high: led_current(netlist, leak + high, leak + GMIN),
    }
}

/// `sim run` and a steps run of the same netlist side by side, so the test costs one run. The
/// step at a stop's picosecond holds the old level, and the restart's step after it has the new
/// one.
#[test]
fn sim_run_writes_pb5_and_the_led_at_the_picoseconds_blink_adds_up_to() {
    let netlist = netlist();
    let out = std::env::temp_dir().join(format!("sim-cli-blink-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let path = repo("examples/atmega-blink/netlist.json");
    let parsed = sim_core::netlist::parse(&fs::read(&path).unwrap()).unwrap();
    let (output, steps) = std::thread::scope(|scope| {
        let steps = scope.spawn(|| sim_circuit::simulate_steps(&parsed));
        let output = Command::new(env!("CARGO_BIN_EXE_sim"))
            .args(["run".as_ref(), path.as_os_str()])
            .args(["--out".as_ref(), out.as_os_str()])
            .output()
            .unwrap();
        (output, steps.join().unwrap())
    });
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["status"], "ok");
    assert_eq!(stdout["diagnostics"], serde_json::json!([]));

    let folder = transient_folder("atmega-blink", &out);
    let file =
        |name: &str| -> serde_json::Value { serde_json::from_slice(&folder.files[name]).unwrap() };
    let observations = file("observations.json");
    assert_eq!(
        file("diagnostics.json"),
        serde_json::to_value(steps.diagnostics()).unwrap()
    );
    let step_observations = serde_json::to_value(steps.observations()).unwrap();
    for key in ["end_ps", "switches", "logic"] {
        assert_eq!(observations[key], step_observations[key], "{key}");
    }

    let step_trace = steps.trace().unwrap();
    let step_refs: Vec<String> = step_trace.columns().iter().map(|r| r.to_string()).collect();
    assert_eq!(step_refs, folder.refs);
    let step_t: Vec<u64> = step_trace.time().iter().map(|at| at.as_ps()).collect();
    assert!(step_t.len() > folder.time.len());
    for (j, reference) in folder.refs.iter().enumerate() {
        let (mins, maxs) = folder.bounds(reference);
        let step_values = step_trace.column(j);
        for k in 0..folder.time.len() - 1 {
            let from = step_t.partition_point(|&at| at < folder.time[k]);
            let to = step_t.partition_point(|&at| at <= folder.time[k + 1]);
            let spanned = &step_values[from..to];
            let low = spanned.iter().copied().fold(f64::INFINITY, f64::min);
            let high = spanned.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert_eq!((mins[k], maxs[k]), (low, high), "{reference} interval {k}");
        }
    }

    let vcd = read_vcd(std::str::from_utf8(&folder.files["traces/logic.vcd"]).unwrap());
    assert_eq!(summary(&vcd), observations["logic"]);
    let pb5: Vec<(u64, bool)> = vcd["#u1.logic.PB5"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| (edge["t_ps"].as_u64().unwrap(), edge["level"] == "high"))
        .collect();
    let edges = edges(&netlist);
    assert_eq!(edges.len(), 8);
    assert_eq!(pb5, edges);
    for (key, signal) in observations["logic"].as_object().unwrap() {
        if key != "#u1.logic.PB5" {
            assert_eq!(signal["count"], 0, "{key} holds its level");
        }
    }

    let Levels { off, low, high } = led_levels(&netlist);
    let (mins, maxs) = folder.bounds("#led1.i");
    for &(edge, _) in &edges {
        let k = folder.time.partition_point(|&at| at <= edge) - 1;
        assert!(
            near(mins[k], low) && near(maxs[k], high),
            "the interval holding the edge at {edge} ps spans both currents"
        );
    }

    let column = step_refs.iter().position(|r| r == "#led1.i").unwrap();
    let rows: Vec<(u64, f64)> = step_t
        .iter()
        .copied()
        .zip(step_trace.column(column).iter().copied())
        .collect();
    let driven = driven_ps(&netlist);
    for &(at, i) in &rows {
        let want = match edges.iter().rfind(|&&(edge, _)| edge < at) {
            Some(&(_, true)) => high,
            Some(&(_, false)) => low,
            None if at <= driven => off,
            None => low,
        };
        assert!(near(i, want), "{i} A at {at} ps, not {want} A");
    }
    for &(edge, _) in &edges {
        let k = rows.iter().position(|&(at, _)| at == edge).unwrap();
        assert_eq!(
            rows[k + 1].0,
            edge + RESTART_PS,
            "the restart after {edge} ps"
        );
    }
    fs::remove_dir_all(&out).unwrap();
}
