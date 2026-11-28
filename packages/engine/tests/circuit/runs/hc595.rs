//! A 74HC595 sources an LED through 220 ohm, and eight outputs on 22 ohm go past
//! the sheet's pin and supply currents.

use std::str::FromStr;

use orbe_engine::Session;
use orbe_engine::blob::Blobs;
use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::name::Ref;
use orbe_model::Time;

fn val(value: f64, unit: &str) -> serde_json::Value {
    serde_json::json!({ "value": value, "unit": unit })
}

fn stage(high: f64, low: f64) -> serde_json::Value {
    serde_json::json!({
        "high_drop": val(0.0, "V"),
        "high_resistance": val(high, "ohm"),
        "low_drop": val(0.0, "V"),
        "low_resistance": val(low, "ohm"),
    })
}

fn source(name: &str, net: &str, volts: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "gnd" },
        "voltage": val(volts, "V"),
    })
}

fn steps(name: &str, net: &str, events: &[(u64, f64)]) -> serde_json::Value {
    let events: Vec<_> = events
        .iter()
        .map(|(t, volts)| serde_json::json!({ "t_ps": t, "level": val(*volts, "V") }))
        .collect();
    serde_json::json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "gnd" },
        "voltage": {
            "kind": "steps",
            "initial": val(0.0, "V"),
            "events": events,
        },
    })
}

fn register(
    loads: Vec<serde_json::Value>,
    clocks: serde_json::Value,
    latch: serde_json::Value,
) -> serde_json::Value {
    let mut nets = [
        "gnd", "vcc", "ser", "srclk", "rclk", "qa", "qb", "qc", "qd", "qe", "qf", "qg", "qh", "qhp",
    ]
    .map(str::to_string)
    .to_vec();
    let mut parts = vec![
        source("vcc", "vcc", 5.0),
        source("ser", "ser", 5.0),
        clocks,
        latch,
        serde_json::json!({
            "name": "u1",
            "kind": "hc595",
            "pins": {
                "QB": "qb", "QC": "qc", "QD": "qd", "QE": "qe", "QF": "qf", "QG": "qg", "QH": "qh",
                "GND": "gnd", "QHp": "qhp", "SRCLR": "vcc", "SRCLK": "srclk", "RCLK": "rclk",
                "OE": "gnd", "SER": "ser", "QA": "qa", "VCC": "vcc",
            },
            "output": stage(33.333333333333336, 28.333333333333332),
            "serial_output": stage(50.0, 42.5),
            "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
            "ratings": {
                "pin_current": val(0.035, "A"),
                "supply_current": val(0.07, "A"),
                "input_margin": val(0.5, "V"),
                "output_margin": val(0.5, "V"),
            },
        }),
    ];
    for load in loads {
        if let Some(net) = load.get("net").and_then(|net| net.as_str()) {
            nets.push(net.to_string());
        }
        parts.push(load["part"].clone());
    }
    serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": 50_000_000_000u64, "max_step_ps": 1_000_000_000u64, "grid_ps": 1_000_000_000u64 } },
        "ground": "gnd",
        "nets": nets,
        "parts": parts,
    })
}

fn led(name: &str, output: &str) -> serde_json::Value {
    let mid = format!("{name}_a");
    serde_json::json!({
        "net": mid,
        "part": {
            "name": name,
            "kind": "led",
            "pins": { "anode": mid, "cathode": "gnd" },
            "saturation_current": val(6.711e-10, "A"),
            "emission_coefficient": val(4.427, "1"),
            "series_resistance": val(2.115, "ohm"),
            "ratings": { "current": val(0.03, "A"), "power": val(0.075, "W") },
        },
        "series": {
            "name": format!("r_{name}"),
            "kind": "resistor",
            "pins": { "a": output, "b": mid },
            "resistance": val(220.0, "ohm"),
            "ratings": { "power": null },
        },
    })
}

fn open(netlist: serde_json::Value) -> Session {
    Session::open_json(netlist.to_string().as_bytes(), &Blobs::new())
        .unwrap_or_else(|err| panic!("{err:?}\n{netlist}"))
}

fn amps(session: &Session, reference: &str) -> f64 {
    session
        .probe()
        .get(&Ref::from_str(reference).unwrap())
        .unwrap_or_else(|| panic!("{reference} is not a column"))
        .get()
}

#[test]
fn one_latched_bit_lights_qa_and_leaves_qh_dark() {
    let qa = led("qa_led", "qa");
    let qh = led("qh_led", "qh");
    let netlist = register(
        vec![
            serde_json::json!({ "net": null, "part": qa["series"] }),
            qa,
            serde_json::json!({ "net": null, "part": qh["series"] }),
            qh,
        ],
        steps("srclk", "srclk", &[(1_000_000, 5.0), (20_000_000, 0.0)]),
        steps("rclk", "rclk", &[(200_000_000, 5.0)]),
    );
    let mut session = open(netlist);
    session.advance_to(Time::from_ps(1_000_000_000));
    let qa_i = amps(&session, "#qa_led.i");
    let qh_i = amps(&session, "#qh_led.i");
    assert!(
        (8.0e-3..16.0e-3).contains(&qa_i),
        "QA LED current is {qa_i}"
    );
    assert!(qh_i < 1.0e-4, "QH LED current is {qh_i}");
    assert!(
        session
            .diagnostics()
            .iter()
            .all(|d| !matches!(d, Diagnostic::RatingCurrent(_))),
        "{:?}",
        session.diagnostics()
    );
}

#[test]
fn eight_outputs_on_22_ohm_warn_on_the_pin_and_the_supply() {
    let mut loads = Vec::new();
    for (name, net) in [
        ("ra", "qa"),
        ("rb", "qb"),
        ("rc", "qc"),
        ("rd", "qd"),
        ("re", "qe"),
        ("rf", "qf"),
        ("rg", "qg"),
        ("rh", "qh"),
    ] {
        loads.push(serde_json::json!({
            "net": null,
            "part": {
                "name": name,
                "kind": "resistor",
                "pins": { "a": net, "b": "gnd" },
                "resistance": val(22.0, "ohm"),
                "ratings": { "power": null },
            },
        }));
    }
    let mut clocks = Vec::new();
    for n in 0..8 {
        let t = 1_000_000 + n * 100_000_000;
        clocks.push((t, 5.0));
        clocks.push((t + 20_000_000, 0.0));
    }
    let netlist = register(
        loads,
        steps("srclk", "srclk", &clocks),
        steps("rclk", "rclk", &[(900_000_000, 5.0)]),
    );
    let mut session = open(netlist);
    session.advance_to(Time::from_ps(2_000_000_000));
    let names: Vec<String> = session
        .diagnostics()
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingCurrent(excursion) => Some(excursion.quantity.to_string()),
            _ => None,
        })
        .collect();
    assert!(
        names.iter().any(|name| name.contains("QA")),
        "pin warning missing in {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains("VCC")),
        "supply warning missing in {names:?}"
    );
}

fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": "resistor",
        "pins": { "a": a, "b": b },
        "resistance": val(ohm, "ohm"),
        "ratings": { "power": null },
    })
}

/// `QA` and `QHp` each have 100 kΩ to 5 V and 10 kΩ to ground. `vcc` is a source when
/// `supply` is set, and open when it is not.
fn loaded(supply: Option<f64>) -> serde_json::Value {
    let mut parts = vec![
        source("five", "five", 5.0),
        resistor("qa_up", "five", "qa", 100_000.0),
        resistor("qa_dn", "qa", "gnd", 10_000.0),
        resistor("qhp_up", "five", "qhp", 100_000.0),
        resistor("qhp_dn", "qhp", "gnd", 10_000.0),
        serde_json::json!({
            "name": "u1",
            "kind": "hc595",
            "pins": {
                "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd", "QH": "gnd",
                "GND": "gnd", "QHp": "qhp", "SRCLR": "five", "SRCLK": "gnd", "RCLK": "gnd",
                "OE": "gnd", "SER": "gnd", "QA": "qa", "VCC": "vcc",
            },
            "output": stage(33.333333333333336, 28.333333333333332),
            "serial_output": stage(50.0, 42.5),
            "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
            "ratings": {
                "pin_current": val(0.035, "A"),
                "supply_current": val(0.07, "A"),
                "input_margin": serde_json::Value::Null,
                "output_margin": serde_json::Value::Null,
            },
        }),
    ];
    if let Some(volts) = supply {
        parts.insert(0, source("vcc", "vcc", volts));
    }
    serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": 1_000_000u64, "max_step_ps": 1_000_000u64, "grid_ps": 1_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "five", "qa", "qhp"],
        "parts": parts,
    })
}

fn pin_volts(supply: Option<f64>, net: &str) -> f64 {
    let mut session = open(loaded(supply));
    session.advance_to(Time::from_ps(1_000_000));
    amps(&session, &format!("#net.{net}"))
}

/// Released, 100 kΩ to 5 V against 10 kΩ to ground.
const DIVIDER_V: f64 = 5.0 * 10_000.0 / 110_000.0;

#[test]
fn below_2v_qa_and_qhp_sit_on_their_dividers_and_at_5v_they_drive_low() {
    for supply in [Some(0.0), None, Some(1.9)] {
        let qa = pin_volts(supply, "qa");
        let qhp = pin_volts(supply, "qhp");
        assert!(
            (qa - DIVIDER_V).abs() < 1e-3 && (qhp - DIVIDER_V).abs() < 1e-3,
            "supply {supply:?}: QA {qa}, QHp {qhp}, divider {DIVIDER_V}"
        );
    }
    let qa = pin_volts(Some(5.0), "qa");
    let qhp = pin_volts(Some(5.0), "qhp");
    assert!(qa < 0.01 && qhp < 0.01, "driven low: QA {qa}, QHp {qhp}");
}

/// One rising `SRCLK` while `SER` is high, then `RCLK`. `vcc_at_5us` is the supply after 5 ms;
/// before that it is `vcc0`.
fn shifted(vcc0: f64, vcc_at_5ms: f64) -> f64 {
    let vcc = if (vcc0 - vcc_at_5ms).abs() < 1e-9 {
        source("vcc", "vcc", vcc0)
    } else {
        steps("vcc", "vcc", &[(5_000_000_000, vcc_at_5ms)])
    };
    let netlist = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": 8_000_000_000u64, "max_step_ps": 1_000_000_000u64, "grid_ps": 1_000_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "five", "srclk", "rclk", "qa"],
        "parts": [
            source("five", "five", 5.0),
            vcc,
            steps("srclk", "srclk", &[(1_000_000_000, 5.0), (1_020_000_000, 0.0)]),
            steps("rclk", "rclk", &[(6_000_000_000, 5.0)]),
            resistor("qa_dn", "qa", "gnd", 10_000.0),
            serde_json::json!({
                "name": "u1",
                "kind": "hc595",
                "pins": {
                    "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd", "QH": "gnd",
                    "GND": "gnd", "QHp": "gnd", "SRCLR": "five", "SRCLK": "srclk", "RCLK": "rclk",
                    "OE": "gnd", "SER": "five", "QA": "qa", "VCC": "vcc",
                },
                "output": stage(33.333333333333336, 28.333333333333332),
                "serial_output": stage(50.0, 42.5),
                "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
                "ratings": {
                    "pin_current": val(0.035, "A"),
                    "supply_current": val(0.07, "A"),
                    "input_margin": serde_json::Value::Null,
                    "output_margin": serde_json::Value::Null,
                },
            }),
        ],
    });
    let mut session = open(netlist);
    session.advance_to(Time::from_ps(7_000_000_000));
    amps(&session, "#net.qa")
}

#[test]
fn a_clock_below_2v_does_not_shift_and_the_same_clock_at_5v_does() {
    let ignored = shifted(0.0, 5.0);
    assert!(
        ignored < 0.1,
        "QA after a clock while unpowered is {ignored}"
    );
    let latched = shifted(5.0, 5.0);
    assert!(latched > 4.5, "QA after a clock at 5 V is {latched}");
}

fn rated() -> serde_json::Value {
    serde_json::json!({
        "pin_current": val(0.035, "A"),
        "supply_current": val(0.07, "A"),
        "input_margin": val(0.5, "V"),
        "output_margin": val(0.5, "V"),
    })
}

fn voltage_pins(session: &Session) -> Vec<String> {
    session
        .diagnostics()
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingVoltage(excursion) => Some(excursion.pin.to_string()),
            _ => None,
        })
        .collect()
}

/// One `SRCLK` with `SER` at `high`, then `RCLK`. `SRCLR` sits on `VCC`.
fn shifted_from(supply: f64, high: f64) -> (f64, Vec<String>) {
    let netlist = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": 4_000_000_000u64, "max_step_ps": 1_000_000_000u64, "grid_ps": 1_000_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "ser", "srclk", "rclk", "qa"],
        "parts": [
            source("vcc", "vcc", supply),
            source("ser", "ser", high),
            steps("srclk", "srclk", &[(1_000_000_000, high), (1_020_000_000, 0.0)]),
            steps("rclk", "rclk", &[(2_000_000_000, high)]),
            resistor("qa_dn", "qa", "gnd", 10_000.0),
            serde_json::json!({
                "name": "u1",
                "kind": "hc595",
                "pins": {
                    "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd", "QH": "gnd",
                    "GND": "gnd", "QHp": "gnd", "SRCLR": "vcc", "SRCLK": "srclk", "RCLK": "rclk",
                    "OE": "gnd", "SER": "ser", "QA": "qa", "VCC": "vcc",
                },
                "output": stage(33.333333333333336, 28.333333333333332),
                "serial_output": stage(50.0, 42.5),
                "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
                "ratings": rated(),
            }),
        ],
    });
    let mut session = open(netlist);
    session.advance_to(Time::from_ps(3_000_000_000));
    (amps(&session, "#net.qa"), voltage_pins(&session))
}

#[test]
fn a_3v3_part_with_5v_inputs_warns_and_still_shifts() {
    let (qa, names) = shifted_from(3.3, 5.0);
    assert!(qa > 3.0, "QA is {qa}");
    for pin in ["SER", "SRCLK", "RCLK"] {
        assert!(
            names.iter().any(|name| name.contains(pin)),
            "{pin} missing from {names:?}"
        );
    }
}

#[test]
fn a_5v_part_with_5v_inputs_stays_quiet() {
    let (qa, names) = shifted_from(5.0, 5.0);
    assert!(qa > 4.5, "QA is {qa}");
    assert!(names.is_empty(), "{names:?}");
}

/// Eight highs, so `QHp` rises. `OE` is high and `QA` is pulled to `pull`.
fn cascaded(supply: f64, pull: f64) -> (f64, Vec<String>) {
    let mut clocks = Vec::new();
    for n in 0..8 {
        let t = 200_000_000 + n * 100_000_000;
        clocks.push((t, supply));
        clocks.push((t + 20_000_000, 0.0));
    }
    let netlist = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": 2_000_000_000u64, "max_step_ps": 100_000_000u64, "grid_ps": 100_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "srclk", "pull", "qa", "qhp"],
        "parts": [
            source("vcc", "vcc", supply),
            source("pull", "pull", pull),
            steps("srclk", "srclk", &clocks),
            resistor("qa_up", "pull", "qa", 100_000.0),
            resistor("qhp_dn", "qhp", "gnd", 10_000.0),
            serde_json::json!({
                "name": "u1",
                "kind": "hc595",
                "pins": {
                    "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd", "QH": "gnd",
                    "GND": "gnd", "QHp": "qhp", "SRCLR": "vcc", "SRCLK": "srclk", "RCLK": "gnd",
                    "OE": "vcc", "SER": "vcc", "QA": "qa", "VCC": "vcc",
                },
                "output": stage(33.333333333333336, 28.333333333333332),
                "serial_output": stage(50.0, 42.5),
                "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
                "ratings": rated(),
            }),
        ],
    });
    let mut session = open(netlist);
    session.advance_to(Time::from_ps(1_200_000_000));
    (amps(&session, "#net.qhp"), voltage_pins(&session))
}

#[test]
fn an_output_pulled_above_vcc_warns_and_qhp_still_shifts() {
    let (qhp, names) = cascaded(3.3, 5.0);
    assert!(qhp > 3.0, "QHp is {qhp}");
    assert!(names.iter().any(|name| name.contains("QA")), "{names:?}");
}

#[test]
fn a_5v_output_on_a_5v_part_stays_quiet() {
    let (qhp, names) = cascaded(5.0, 5.0);
    assert!(qhp > 4.5, "QHp is {qhp}");
    assert!(names.is_empty(), "{names:?}");
}

#[test]
fn logic_traces_name_each_input_and_keep_its_level() {
    let done = orbe_engine::run(
        &serde_json::to_vec(&loaded(Some(5.0))).unwrap(),
        orbe_engine::timestamp::Timestamp::from_unix_seconds(0),
    );
    let vcd = std::str::from_utf8(done.folder.file("traces/logic.vcd").unwrap()).unwrap();
    let vars: Vec<&str> = vcd
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("$var wire 1 ")?.strip_suffix(" $end")?;
            Some(rest.split_once(' ')?.1)
        })
        .collect();
    assert_eq!(vars, ["OE", "RCLK", "SER", "SRCLK", "SRCLR"]);
    let observations: serde_json::Value =
        serde_json::from_slice(done.folder.file("observations.json").unwrap()).unwrap();
    let logic = &observations["logic"];
    assert_eq!(
        logic.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "#u1.logic.OE",
            "#u1.logic.RCLK",
            "#u1.logic.SER",
            "#u1.logic.SRCLK",
            "#u1.logic.SRCLR",
        ]
    );
    assert_eq!(logic["#u1.logic.OE"]["initial"], "low");
    assert_eq!(logic["#u1.logic.RCLK"]["initial"], "low");
    assert_eq!(logic["#u1.logic.SER"]["initial"], "low");
    assert_eq!(logic["#u1.logic.SRCLK"]["initial"], "low");
    assert_eq!(logic["#u1.logic.SRCLR"]["initial"], "high");
}

fn trace_samples(folder: &orbe_engine::run_folder::RunFolder, column: &str) -> Vec<f64> {
    let index: serde_json::Value =
        serde_json::from_slice(folder.file("traces/index.json").unwrap()).unwrap();
    let rows = index["rows"].as_u64().unwrap() as usize;
    let col = index["analog"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .position(|entry| entry["ref"] == column)
        .unwrap_or_else(|| panic!("{column} missing from trace index"));
    let raw = folder.file("traces/analog.f64").unwrap();
    raw.as_chunks::<8>()
        .0
        .iter()
        .skip(col * rows)
        .take(rows)
        .map(|&chunk| f64::from_le_bytes(chunk))
        .collect()
}

fn pullup_hc595_netlist(analysis: serde_json::Value, with_diode: bool) -> serde_json::Value {
    let mut nets = vec!["gnd", "vcc", "qa", "qhp"];
    let mut parts = vec![
        source("vcc", "vcc", 5.0),
        resistor("r_qa", "vcc", "qa", 10_000.0),
        resistor("r_qhp", "vcc", "qhp", 10_000.0),
        serde_json::json!({
            "name": "u1",
            "kind": "hc595",
            "pins": {
                "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd", "QH": "gnd",
                "GND": "gnd", "QHp": "qhp", "SRCLR": "vcc", "SRCLK": "gnd", "RCLK": "gnd",
                "OE": "gnd", "SER": "gnd", "QA": "qa", "VCC": "vcc",
            },
            "output": stage(25.0, 25.0),
            "serial_output": stage(50.0, 50.0),
            "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
            "ratings": {
                "pin_current": val(0.035, "A"),
                "supply_current": val(0.07, "A"),
                "input_margin": serde_json::Value::Null,
                "output_margin": serde_json::Value::Null,
            },
        }),
    ];
    if with_diode {
        nets.push("dmid");
        parts.push(resistor("r_d", "vcc", "dmid", 100_000.0));
        parts.push(serde_json::json!({
            "name": "d1",
            "kind": "diode",
            "pins": { "anode": "dmid", "cathode": "gnd" },
            "saturation_current": val(1e-9, "A"),
            "emission_coefficient": val(1.8, "1"),
            "series_resistance": val(1.0, "ohm"),
            "ratings": { "current": null, "reverse_voltage": null, "power": null },
        }));
    }
    serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": analysis,
        "ground": "gnd",
        "nets": nets,
        "parts": parts,
    })
}

#[test]
fn operating_point_and_first_sample_drive_outputs_low_without_nonlinear_parts() {
    let expected_qa = 5.0 * 25.0 / 10_025.0;
    let expected_qhp = 5.0 * 50.0 / 10_050.0;
    let transient = serde_json::json!({
        "transient": { "stop_ps": 1_000_000_000u64, "max_step_ps": 100_000_000u64, "grid_ps": 100_000_000u64 }
    });

    // Operating-point-only analysis.
    let op_run = orbe_engine::run(
        &serde_json::to_vec(&pullup_hc595_netlist(
            serde_json::json!({ "op": {} }),
            false,
        ))
        .unwrap(),
        orbe_engine::timestamp::Timestamp::from_unix_seconds(0),
    );
    let op_only: serde_json::Value =
        serde_json::from_slice(op_run.folder.file("observations.json").unwrap()).unwrap();
    let op_qa = op_only["op"]["#net.qa"].as_f64().unwrap();
    let op_qhp = op_only["op"]["#net.qhp"].as_f64().unwrap();
    assert!((op_qa - expected_qa).abs() < 1e-6, "op QA {op_qa}");
    assert!((op_qhp - expected_qhp).abs() < 1e-6, "op QHp {op_qhp}");

    // Transient operating point (t = 0) and first sample, with and without an unrelated diode.
    for with_diode in [false, true] {
        let done = orbe_engine::run(
            &serde_json::to_vec(&pullup_hc595_netlist(transient.clone(), with_diode)).unwrap(),
            orbe_engine::timestamp::Timestamp::from_unix_seconds(0),
        );
        let obs: serde_json::Value =
            serde_json::from_slice(done.folder.file("observations.json").unwrap()).unwrap();
        let qa_op = obs["op"]["#net.qa"].as_f64().unwrap();
        let qhp_op = obs["op"]["#net.qhp"].as_f64().unwrap();
        let qa_trace = trace_samples(&done.folder, "#net.qa");
        let qhp_trace = trace_samples(&done.folder, "#net.qhp");
        assert!(
            (qa_op - expected_qa).abs() < 1e-6,
            "with_diode={with_diode}: op QA {qa_op} vs {expected_qa}"
        );
        assert!(
            (qhp_op - expected_qhp).abs() < 1e-6,
            "with_diode={with_diode}: op QHp {qhp_op} vs {expected_qhp}"
        );
        assert!(
            (qa_trace[0] - expected_qa).abs() < 1e-6 && (qa_trace[1] - expected_qa).abs() < 1e-6,
            "with_diode={with_diode}: QA trace[0..2] = {:?}",
            &qa_trace[..2]
        );
        assert!(
            (qhp_trace[0] - expected_qhp).abs() < 1e-6
                && (qhp_trace[1] - expected_qhp).abs() < 1e-6,
            "with_diode={with_diode}: QHp trace[0..2] = {:?}",
            &qhp_trace[..2]
        );
    }
}

#[test]
fn a_nonlinear_step_crossing_min_supply_converges_on_the_matching_stamp() {
    // VCC starts 0.5 uV above the 2.0 V - 1 uV release threshold and falls 0.75 uV in one step
    // (within Newton's RELTOL * 2 V = 2 uV tolerance, and above the 2.0 V - 2 uV fall watch).
    // Newton must re-stamp with the released state rather than accepting driven-low outputs.
    let netlist = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": 1_000u64, "max_step_ps": 1_000u64, "grid_ps": 1_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "five", "vsrc", "vcc", "qa", "qhp", "dmid"],
        "parts": [
            source("five", "five", 5.0),
            source("vsrc", "vsrc", 1.999_998_0),
            resistor("r_load", "vsrc", "gnd", 100.0),
            resistor("r_rc", "vsrc", "vcc", 100.0),
            {
                "name": "c_rc",
                "kind": "capacitor",
                "pins": { "a": "vcc", "b": "gnd" },
                "capacitance": val(1e-11, "F"),
                "initial": val(1.999_999_5, "V"),
            },
            resistor("r_qa", "five", "qa", 10_000.0),
            resistor("r_qhp", "five", "qhp", 10_000.0),
            resistor("r_d", "five", "dmid", 100_000.0),
            {
                "name": "d1",
                "kind": "diode",
                "pins": { "anode": "dmid", "cathode": "gnd" },
                "saturation_current": val(1e-9, "A"),
                "emission_coefficient": val(1.8, "1"),
                "series_resistance": val(1.0, "ohm"),
                "ratings": { "current": null, "reverse_voltage": null, "power": null },
            },
            {
                "name": "u1",
                "kind": "hc595",
                "pins": {
                    "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd", "QH": "gnd",
                    "GND": "gnd", "QHp": "qhp", "SRCLR": "five", "SRCLK": "gnd", "RCLK": "gnd",
                    "OE": "gnd", "SER": "gnd", "QA": "qa", "VCC": "vcc",
                },
                "output": stage(25.0, 25.0),
                "serial_output": stage(50.0, 50.0),
                "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
                "ratings": {
                    "pin_current": val(0.035, "A"),
                    "supply_current": val(0.07, "A"),
                    "input_margin": serde_json::Value::Null,
                    "output_margin": serde_json::Value::Null,
                },
            },
        ],
    });
    let done = orbe_engine::run(
        &serde_json::to_vec(&netlist).unwrap(),
        orbe_engine::timestamp::Timestamp::from_unix_seconds(0),
    );
    let vcc = trace_samples(&done.folder, "#net.vcc");
    let qa = trace_samples(&done.folder, "#net.qa");
    let qhp = trace_samples(&done.folder, "#net.qhp");
    assert!(vcc[0] > 2.0 - 1e-6 && vcc[1] < 2.0 - 1e-6, "vcc = {vcc:?}");
    assert!(
        qa[0] < 0.05 && qhp[0] < 0.05,
        "t=0 driven low: qa={qa:?} qhp={qhp:?}"
    );
    assert!(
        (qa[1] - 5.0).abs() < 1e-4 && (qhp[1] - 5.0).abs() < 1e-4,
        "t=1ns released: qa={qa:?} qhp={qhp:?}"
    );
}
