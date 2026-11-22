//! An LM7805 fed by a barrel jack, and an IRF9540N as a high-side switch.
//!
//! The regulator's 9 V loads sit inside the datasheet's 4.8 V to 5.2 V window. Six volts in
//! is below the 2 V dropout, so the output follows the input. A 3 ohm load draws more than
//! the 1.5 A rating and the part warns, while the output is still near 5 V.
//!
//! The MOSFET's gate at 0 V against a 10 V source is the datasheet's on point, VGS = -10 V.
//! The load there is (10 - 1.287) / 11 ohm, so the channel sits on the 0.117 ohm, 11 A point
//! the fit uses. The same gate with 10 ohm is near 1 A, and the on-resistance is a little
//! under 0.117 ohm because of the vds/2 term. The gate tied to the source leaves the channel
//! off. A Nano pin through a 2N2222 pulls that gate down and turns the channel on.

mod common;

use std::fs;

use orbe_engine::RunStats;
use orbe_engine::diagnostic::{Diagnostic, Severity};
use orbe_engine::run_folder::Results;
use orbe_scene::Files;

/// Long enough for the capacitances to settle, and for `gate_hold.hex` to finish setup.
const STOP_PS: u64 = 50_000_000_000;

/// From `parts/lm7805-to220/part.jsonc`.
const VOUT: f64 = 5.0;
const K_LINE: f64 = 0.00016666666666666666;
const RO: f64 = 0.006688963210702341;
const VDO: f64 = 1.999;
const RDO: f64 = 0.001;

/// From `parts/pmos-irf9540n-to220/part.jsonc`.
const KP: f64 = 1.3446092263051281;
const VTO: f64 = 3.0;
const LAMBDA: f64 = 0.0;

/// 11 A times the 0.117 ohm maximum, the |vds| of the datasheet point.
const FIT_VDS: f64 = 11.0 * 0.117;
/// The load that puts a 10 V source on that point.
const FIT_OHMS: f64 = (10.0 - FIT_VDS) / 11.0;

fn regulator(volts: u32, ohms: &str) -> String {
    format!(
        r#"{{ "format": 1, "ground": "jack.sleeve", "parts": {{ "jack": {{ "type": "barrel-jack-dc21", "voltage": {volts} }}, "u1": {{ "type": "lm7805-to220" }}, "rl": {{ "type": "resistor-axial", "resistance": "{ohms}" }} }}, "wires": [ ["jack.center", "u1.IN"], ["jack.sleeve", "u1.GND"], ["u1.OUT", "rl.a"], ["rl.b", "jack.sleeve"] ], "nets": {{ "VIN": "u1.IN", "VOUT": "u1.OUT" }} }}"#
    )
}

fn mosfet(gate: &str, ohms: &str) -> String {
    let gate_wire = match gate {
        "low" => r#"["m1.gate", "jack.sleeve"]"#,
        "high" => r#"["m1.gate", "m1.source"]"#,
        _ => panic!("gate {gate}"),
    };
    // The gate shares the ground net, or the source net, so it is not named on its own.
    format!(
        r#"{{ "format": 1, "ground": "jack.sleeve", "parts": {{ "jack": {{ "type": "barrel-jack-dc21", "voltage": 10 }}, "m1": {{ "type": "pmos-irf9540n-to220" }}, "rl": {{ "type": "resistor-axial", "resistance": "{ohms}" }} }}, "wires": [ ["jack.center", "m1.source"], ["m1.drain", "rl.a"], ["rl.b", "jack.sleeve"], {gate_wire} ], "nets": {{ "source": "m1.source", "drain": "m1.drain" }} }}"#
    )
}

/// D9 stays high, so the 2N2222 holds the gate near ground and the channel is on.
fn nano_high_side() -> String {
    r#"{ "format": 1, "parts": { "nano": { "type": "nano-board", "firmware": "build/gate_hold.hex" }, "jack": { "type": "barrel-jack-dc21", "voltage": 10 }, "m1": { "type": "pmos-irf9540n-to220" }, "q1": { "type": "npn-2n2222-to92" }, "rpu": { "type": "resistor-axial", "resistance": "10k" }, "rb": { "type": "resistor-axial", "resistance": "1k" }, "rl": { "type": "resistor-axial", "resistance": "470" } }, "wires": [ ["jack.center", "m1.source"], ["jack.sleeve", "nano.GND"], ["m1.drain", "rl.a"], ["rl.b", "nano.GND"], ["rpu.a", "m1.source"], ["rpu.b", "m1.gate"], ["q1.collector", "m1.gate"], ["q1.emitter", "nano.GND"], ["nano.D9", "rb.a"], ["rb.b", "q1.base"] ], "nets": { "GND": "nano.GND", "source": "m1.source", "drain": "m1.drain", "gate": "m1.gate" } }"#.to_string()
}

fn run(scene: &str) -> (Results, RunStats) {
    let project = common::project("nano-button");
    fs::copy(
        common::repo().join("packages/engine/tests/firmware/sketch/gate_hold.hex"),
        project.path().join("build/gate_hold.hex"),
    )
    .unwrap();
    let path = project.path().join("power.jsonc");
    fs::write(&path, scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &path).unwrap_or_else(|error| {
        panic!("{error:?}\n{scene}");
    });
    let mut json: serde_json::Value = serde_json::from_str(&netlist.json).unwrap();
    json["analysis"]["transient"]["stop_ps"] = STOP_PS.into();
    let netlist = orbe_engine::netlist::parse(json.to_string().as_bytes()).unwrap();
    orbe_engine::simulate_stats(&netlist)
}

fn last(results: &Results, key: &str) -> f64 {
    let trace = results.trace().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|column| column.to_string() == key)
        .unwrap_or_else(|| panic!("no column {key}"));
    *trace.column(index).last().unwrap()
}

fn warns(results: &Results, code: &str, quantity: &str) -> bool {
    results.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == code
            && diagnostic
                .refs()
                .iter()
                .any(|r#ref| r#ref.to_string() == quantity)
    })
}

/// `vset - Ro * i` while the input is far above dropout.
fn regulated(vin: f64, ohms: f64) -> f64 {
    let vset = VOUT + K_LINE * (vin - VOUT);
    vset / (1.0 + RO / ohms)
}

/// `vi - Vdo - Rdo * i` while dropout holds the output down.
fn dropped(vin: f64, ohms: f64) -> f64 {
    (vin - VDO) / (1.0 + RDO / ohms)
}

/// Level 1's drain current at `vgs` and `vds`, both positive magnitudes.
fn square_law(vgs: f64, vds: f64) -> f64 {
    let vov = vgs - VTO;
    let k = 1.0 + LAMBDA * vds;
    if vds >= vov {
        KP / 2.0 * vov * vov * k
    } else {
        KP * vds * (vov - vds / 2.0) * k
    }
}

#[test]
fn lm7805_holds_five_volts_from_a_nine_volt_jack() {
    for ohms in ["1000", "470", "220"] {
        let (results, _) = run(&regulator(9, ohms));
        assert_eq!(
            results.count(Severity::Error),
            0,
            "{ohms}: {:?}",
            results.diagnostics()
        );
        let vo = last(&results, "#net.VOUT");
        let law = regulated(9.0, ohms.parse().unwrap());
        assert!(
            (4.8..=5.2).contains(&vo),
            "{ohms} ohm: {vo} V, outside 4.8 V to 5.2 V"
        );
        assert!((vo - law).abs() < 1e-4, "{ohms} ohm: {vo} V, law {law} V");
        assert!(
            !warns(&results, "rating.current", "#u1.iout"),
            "{ohms}: {:?}",
            results.diagnostics()
        );
        assert!(
            !warns(&results, "rating.power", "#u1.p"),
            "{ohms}: {:?}",
            results.diagnostics()
        );
    }
    let at_9 = last(&run(&regulator(9, "1000")).0, "#net.VOUT");
    let at_15 = last(&run(&regulator(15, "1000")).0, "#net.VOUT");
    let rise = regulated(15.0, 1000.0) - regulated(9.0, 1000.0);
    assert!(
        ((at_15 - at_9) - rise).abs() < 1e-4,
        "9 V to 15 V moved the output by {} V, law {rise} V",
        at_15 - at_9
    );
}

#[test]
fn lm7805_drops_out_at_six_volts_and_warns_past_one_and_a_half_amps() {
    let (results, _) = run(&regulator(6, "1000"));
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    let vo = last(&results, "#net.VOUT");
    let law = dropped(6.0, 1000.0);
    assert!((vo - law).abs() < 1e-3, "dropout {vo} V, law {law} V");
    assert!(vo < 4.5, "6 V in still looks regulated at {vo} V");
    assert!(!warns(&results, "rating.current", "#u1.iout"));

    let (results, _) = run(&regulator(9, "3"));
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    let vo = last(&results, "#net.VOUT");
    let iout = last(&results, "#u1.iout");
    assert!(
        (vo - regulated(9.0, 3.0)).abs() < 1e-3,
        "over current the output is {vo} V"
    );
    assert!(
        iout < -1.5,
        "load current is {} A, under the 1.5 A rating",
        -iout
    );
    assert!(
        warns(&results, "rating.current", "#u1.iout"),
        "{:?}",
        results.diagnostics()
    );
    assert!(
        warns(&results, "rating.power", "#u1.p"),
        "{:?}",
        results.diagnostics()
    );
}

#[test]
fn irf9540n_on_resistance_matches_the_datasheet_and_it_turns_off() {
    let ohms = format!("{FIT_OHMS}");
    let (results, _) = run(&mosfet("low", &ohms));
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    let (vs, vd, id) = (
        last(&results, "#net.source"),
        last(&results, "#net.drain"),
        last(&results, "#m1.id"),
    );
    // The gate is on the ground net, so its voltage is 0.
    let vds = vs - vd;
    let vgs = vs;
    let rds = vds / -id;
    let law = square_law(vgs, vds);
    assert!(
        (id + law).abs() <= 1e-6 * law,
        "id {id} A at |vgs| {vgs} V and |vds| {vds} V, law {law} A"
    );
    assert!(
        (rds - 0.117).abs() < 0.001,
        "RDS {rds} ohm at the 0.117 ohm point"
    );
    assert!(
        (-id - 11.0).abs() < 0.05,
        "id {} A, datasheet point is 11 A",
        -id
    );
    assert!(
        warns(&results, "rating.power", "#m1.p"),
        "11 A on a bare TO-220 should warn: {:?}",
        results.diagnostics()
    );
    assert!(!warns(&results, "rating.current", "#m1.id"));

    let (results, _) = run(&mosfet("low", "10"));
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    let (vs, vd, id) = (
        last(&results, "#net.source"),
        last(&results, "#net.drain"),
        last(&results, "#m1.id"),
    );
    let rds = (vs - vd) / -id;
    assert!(
        (0.10..=0.117).contains(&rds),
        "10 ohm load RDS {rds} ohm, near the 0.117 ohm maximum and under it"
    );
    assert!((-id - 0.99).abs() < 0.02, "id {} A", -id);
    assert!(!warns(&results, "rating.power", "#m1.p"));
    assert!(!warns(&results, "rating.current", "#m1.id"));

    let (results, _) = run(&mosfet("high", "10"));
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    let id = last(&results, "#m1.id");
    let vd = last(&results, "#net.drain");
    assert!(id.abs() < 1e-8, "off current {id} A");
    assert!(vd.abs() < 1e-4, "off drain {vd} V");
}

#[test]
fn irf9540n_high_side_from_a_nano_pin() {
    let (results, _) = run(&nano_high_side());
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    let (vs, vd, vg, id) = (
        last(&results, "#net.source"),
        last(&results, "#net.drain"),
        last(&results, "#net.gate"),
        last(&results, "#m1.id"),
    );
    let vgs = vs - vg;
    let vds = vs - vd;
    let law = square_law(vgs, vds);
    assert!(
        vgs > 9.0,
        "gate should be pulled down, |Vgs| is {vgs} V (gate {vg} V, source {vs} V)"
    );
    assert!(
        (id + law).abs() <= 1e-4 * law.abs().max(1e-6),
        "id {id} A, law {law} A"
    );
    let rds = vds / -id;
    assert!(
        (0.10..=0.12).contains(&rds),
        "RDS {rds} ohm with the gate driven low"
    );
    assert!(-id > 0.02, "load current {} A", -id);
    let power = last(&results, "#m1.p");
    assert!(power < 2.42, "settled power {power} W");
    assert!(!warns(&results, "rating.current", "#m1.id"));
    // The gate edge moves the drain in a few nanoseconds. That sample's capacitive current
    // can clear the power rating, and the part is back under it for the rest of the run.
    for diagnostic in results.diagnostics() {
        let Diagnostic::RatingPower(excursion) = diagnostic else {
            continue;
        };
        if excursion.quantity.to_string() != "#m1.p" {
            continue;
        }
        let over = excursion.over_ps.unwrap_or(u64::MAX);
        assert!(
            over < 1_000_000,
            "power stays over the rating for {over} ps, peak {} W",
            excursion.peak
        );
    }
}
