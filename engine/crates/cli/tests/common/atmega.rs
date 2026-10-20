//! The LED closed form, the potentiometer wiper, the fixture check, the side-by-side run,
//! the check that the LED follows the pin that drives it, and `assert_holds`, shared by the
//! ATmega328P firmware examples.

use std::fs;
use std::process::Command;

use super::{TransientFolder, records, repo, transient_folder};

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

fn read_netlist(example: &str) -> serde_json::Value {
    let path = repo(&format!("examples/{example}/netlist.json"));
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
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

pub fn chip_matches_fixture(example: &str, hex: &str) {
    let netlist = read_netlist(example);
    let mut chip = part(&netlist, "u1").clone();
    let mut fixture: serde_json::Value = serde_json::from_slice(
        &fs::read(repo("crates/core/tests/fixtures/atmega328p.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        records(chip["firmware"].as_str().unwrap()),
        records(&fs::read_to_string(repo(hex)).unwrap())
    );
    chip["firmware"] = serde_json::Value::Null;
    fixture["firmware"] = serde_json::Value::Null;
    assert_eq!(chip, fixture);
}

pub fn cycle_ps(netlist: &serde_json::Value) -> u64 {
    let hz = part(netlist, "u1")["clock_hz"]["value"].as_u64().unwrap();
    assert_eq!(1_000_000_000_000 % hz, 0);
    1_000_000_000_000 / hz
}

/// The LED's current, through its `Rs`, while the pin's stage pulls up through `up` siemens and down
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

/// The LED's current in each state of the pin, with the pin's leakage to both rails and gmin on the pin.
struct Levels {
    pub off: f64,
    pub low: f64,
    pub high: f64,
}

/// The wiper voltage of `rv1` across `v1`, and that supply.
/// Leakage of `u1` loads the wiper to both rails, and gmin loads the lower leg.
pub fn wiper_volts(netlist: &serde_json::Value) -> (f64, f64) {
    let pot = part(netlist, "rv1");
    assert_eq!(pot["pins"]["a"], "vcc");
    assert_eq!(pot["pins"]["wiper"], "pc0");
    assert_eq!(pot["pins"]["b"], "gnd");
    let x = value(&pot["position"]);
    let ohms = value(&pot["resistance"]);
    let leak = value(&part(netlist, "u1")["leakage_resistance"]);
    let vcc = value(&part(netlist, "v1")["voltage"]);
    let g_hi = 1.0 / (x * ohms) + 1.0 / leak;
    let g_lo = 1.0 / ((1.0 - x) * ohms) + 1.0 / leak + GMIN;
    (vcc * g_hi / (g_hi + g_lo), vcc)
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

/// One `sim run` beside `simulate_steps` of the same netlist, so a test pays for one run.
pub struct Beside {
    pub netlist: serde_json::Value,
    pub parsed: sim_core::netlist::Netlist,
    folder: TransientFolder,
    pub observations: serde_json::Value,
    steps: sim_core::run_folder::Results,
}

pub fn run_beside_steps(example: &str) -> Beside {
    let path = repo(&format!("examples/{example}/netlist.json"));
    let bytes = fs::read(&path).unwrap();
    let netlist: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let parsed = sim_core::netlist::parse(&bytes).unwrap();
    let out = std::env::temp_dir().join(format!("sim-cli-{example}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let (output, steps) = std::thread::scope(|scope| {
        let steps = scope.spawn(|| sim_circuit::simulate_steps(&parsed));
        let output = Command::new(env!("CARGO_BIN_EXE_sim"))
            .args(["run".as_ref(), path.as_os_str()])
            .args(["--out".as_ref(), out.as_os_str()])
            .output()
            .unwrap();
        (output, steps.join().unwrap())
    });
    assert_eq!(output.status.code(), Some(0), "{example}");
    assert!(output.stderr.is_empty(), "{example}");
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["status"], "ok");
    assert_eq!(stdout["diagnostics"], serde_json::json!([]));

    let folder = transient_folder(example, &out);
    fs::remove_dir_all(&out).unwrap();
    let observations: serde_json::Value =
        serde_json::from_slice(&folder.files["observations.json"]).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&folder.files["diagnostics.json"]).unwrap(),
        serde_json::to_value(steps.diagnostics()).unwrap()
    );
    let step_observations = serde_json::to_value(steps.observations()).unwrap();
    for key in ["end_ps", "switches", "logic"] {
        assert_eq!(observations[key], step_observations[key], "{example} {key}");
    }

    let step_trace = steps.trace().unwrap();
    let step_refs: Vec<String> = step_trace.columns().iter().map(|r| r.to_string()).collect();
    assert_eq!(step_refs, folder.refs);
    let step_t: Vec<u64> = step_trace.time().iter().map(|at| at.as_ps()).collect();
    assert!(step_t.len() > folder.time.len(), "{example}");
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
    Beside {
        netlist,
        parsed,
        folder,
        observations,
        steps,
    }
}

/// `(t_ps, high)` of one signal in `logic.vcd`.
pub fn logic_edges(vcd: &serde_json::Value, key: &str) -> Vec<(u64, bool)> {
    vcd[key]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| (edge["t_ps"].as_u64().unwrap(), edge["level"] == "high"))
        .collect()
}

/// `traces/logic.vcd`, checked equal to `observations.logic`. Every other signal keeps its level.
pub fn logic_vcd(beside: &Beside, changing: &[&str]) -> serde_json::Value {
    let vcd =
        super::read_vcd(std::str::from_utf8(&beside.folder.files["traces/logic.vcd"]).unwrap());
    assert_eq!(super::summary(&vcd), beside.observations["logic"]);
    for (key, signal) in beside.observations["logic"].as_object().unwrap() {
        if !changing.contains(&key.as_str()) {
            assert_eq!(signal["count"], 0, "{key} holds its level");
        }
    }
    vcd
}

/// `#led1.i` follows `pin`, the edges of the output that drives the LED. The step at an edge's
/// picosecond still holds the previous level, and the next step is `RESTART_PS` later. Before
/// any edge, steps through `driven_ps` are off and later steps are low.
pub fn assert_led_follows(beside: &Beside, pin: &[(u64, bool)], driven_ps: u64) {
    let Levels { off, low, high } = led_levels(&beside.netlist);
    let (mins, maxs) = beside.folder.bounds("#led1.i");
    for &(edge, _) in pin {
        let k = beside.folder.time.partition_point(|&at| at <= edge) - 1;
        assert!(
            near(mins[k], low) && near(maxs[k], high),
            "the interval holding the edge at {edge} ps spans both currents"
        );
    }

    let step_trace = beside.steps.trace().unwrap();
    let column = beside
        .folder
        .refs
        .iter()
        .position(|reference| reference == "#led1.i")
        .unwrap();
    let rows: Vec<(u64, f64)> = step_trace
        .time()
        .iter()
        .map(|at| at.as_ps())
        .zip(step_trace.column(column).iter().copied())
        .collect();
    for &(at, i) in &rows {
        let want = match pin.iter().rfind(|&&(edge, _)| edge < at) {
            Some(&(_, true)) => high,
            Some(&(_, false)) => low,
            None if at <= driven_ps => off,
            None => low,
        };
        assert!(near(i, want), "{i} A at {at} ps, not {want} A");
    }
    for &(edge, _) in pin {
        let k = rows.iter().position(|&(at, _)| at == edge).unwrap();
        assert_eq!(
            rows[k + 1].0,
            edge + RESTART_PS,
            "the restart after {edge} ps"
        );
    }
}

fn near_relative(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-8 * want.abs()
}

/// Every grid sample, envelope bound, and step of `reference` is `want` within 1e-8 relative,
/// the divider bound in docs/run-folder.md.
pub fn assert_holds(beside: &Beside, reference: &str, want: f64) {
    let (mins, maxs) = beside.folder.bounds(reference);
    for (k, (&low, &high)) in mins.iter().zip(maxs).enumerate() {
        assert!(
            near_relative(low, want) && near_relative(high, want),
            "{reference} interval {k}: {low}..{high}, not {want}"
        );
    }
    let rows = beside.folder.time.len();
    let j = beside
        .folder
        .refs
        .iter()
        .position(|item| item == reference)
        .unwrap();
    for (k, &got) in beside.folder.analog[j * rows..(j + 1) * rows]
        .iter()
        .enumerate()
    {
        assert!(
            near_relative(got, want),
            "{reference} row {k}: {got}, not {want}"
        );
    }
    let step_trace = beside.steps.trace().unwrap();
    for (at, &got) in step_trace.time().iter().zip(step_trace.column(j)) {
        assert!(
            near_relative(got, want),
            "{reference} at {} ps: {got}, not {want}",
            at.as_ps()
        );
    }
}
