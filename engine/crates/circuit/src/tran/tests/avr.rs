//! The ATmega328P through the seam, running the repo's firmware in `firmware/`.

use sim_avr::{FuseBytes, Port};
use sim_core::diagnostic::Severity;
use sim_core::netlist::{Atmega328p, Netlist};

use super::active::{
    NS, US, at, capacitor, circuit, resistor, restarted, run_active, scripted, volts,
};
use crate::active::avr::{self, PACKAGE, Pad, port_index};
use crate::active::{self, Drive};
use crate::newton::Circuit;
use sim_core::diagnostic::Diagnostic;
use sim_core::logic::Level;
use sim_core::name::Ref;
use sim_core::trace::Trace;

const BLINK: &str = include_str!("../../../../../firmware/blink.hex");
const BUTTON: &str = include_str!("../../../../../firmware/button.hex");
const TOGGLE: &str = include_str!("../../../../../firmware/toggle.hex");
const PULL: &str = include_str!("../../../../../firmware/pullup.hex");
const ECHO: &str = include_str!("../../../../../firmware/echo.hex");
const ADC: &str = include_str!("../../../../../firmware/adc.hex");
const CLKPR: &str = include_str!("../../../../../firmware/clkpr.hex");
const SPEED_UP: &str = include_str!("../../../../../firmware/speedup.hex");
const PORTS: &str = include_str!("../../../../../firmware/ports.hex");

const MHZ_16: u32 = 16_000_000;
/// A crystal whose cycle is no whole number of picoseconds, so every stop rounds up.
const MHZ_14_7456: u32 = 14_745_600;

/// A crystal, `RESET` on `PC6`, and brown-out at 2.7 V.
const FUSES: FuseBytes = FuseBytes {
    low: 0xFF,
    high: 0xD9,
    extended: 0xFD,
};

const PART: &str = include_str!("../../../../core/tests/fixtures/atmega328p.json");

fn documented(pointer: &str) -> f64 {
    let part: serde_json::Value = serde_json::from_str(PART).unwrap();
    part.pointer(pointer)
        .and_then(serde_json::Value::as_f64)
        .unwrap()
}

/// The volts a pin sits at with `up` siemens to `supply` and `down` to ground.
fn share(supply: f64, up: f64, down: f64) -> f64 {
    supply * up / (up + down)
}

/// The net of each package pin that is not a supply: the pin's name in lowercase.
fn pin_nets() -> impl Iterator<Item = (String, Pad)> {
    Atmega328p::PINS
        .iter()
        .zip(PACKAGE)
        .filter(|(_, pad)| matches!(pad, Pad::Port(..) | Pad::Xtal))
        .map(|(name, pad)| (name.to_lowercase(), pad))
}

fn atmega(vdd: &str, hex: &str, clock_hz: u32, fuses: FuseBytes) -> String {
    let mut part: serde_json::Value = serde_json::from_str(PART).unwrap();
    for (name, pad) in Atmega328p::PINS.iter().zip(PACKAGE) {
        part["pins"][*name] = match pad {
            Pad::Vcc | Pad::Avcc | Pad::Aref => vdd.to_string(),
            Pad::Gnd | Pad::Agnd => "gnd".to_string(),
            Pad::Port(..) | Pad::Xtal => name.to_lowercase(),
        }
        .into();
    }
    part["firmware"] = hex.into();
    part["clock_hz"]["value"] = clock_hz.into();
    part["fuses"] =
        serde_json::json!({ "low": fuses.low, "high": fuses.high, "extended": fuses.extended });
    part.to_string()
}

#[test]
fn each_pad_is_the_pin_its_name_says() {
    for (name, pad) in Atmega328p::PINS.iter().zip(PACKAGE) {
        let names = match pad {
            Pad::Port(port, bit) => vec![format!("P{port:?}{bit}")],
            Pad::Xtal => vec!["PB6".to_string(), "PB7".to_string()],
            Pad::Vcc => vec!["VCC".to_string()],
            Pad::Gnd => vec!["GND_8".to_string()],
            Pad::Avcc => vec!["AVCC".to_string()],
            Pad::Aref => vec!["AREF".to_string()],
            Pad::Agnd => vec!["GND_22".to_string()],
        };
        assert!(names.iter().any(|n| n == name), "{name} {pad:?}");
    }
}

/// `RESET` holds the chip from power-on, then rises to about 2.45 V at 1 us: above its own
/// 2.2 V, under an I/O pin's 2.6 V. With `SUT` 01 the reset delay is 14 cycles, and `toggle.S`
/// then pulls PB5 low against 10 kohm to VCC.
#[test]
fn reset_releases_at_its_own_threshold() {
    let step = r#"{ "name": "vr", "kind": "voltage_source", "pins": { "p": "src", "n": "gnd" },
          "voltage": { "kind": "step", "initial": { "value": 0.0, "unit": "V", "source": "user" },
                       "final": { "value": 2.4, "unit": "V", "source": "user" }, "at_ps": 1000000 } }"#;
    let netlist = board(
        3 * US,
        &["src"],
        &[
            step.to_string(),
            resistor("rs", "src", "pc6", 1000.0),
            resistor("rl", "vcc", "pb5", 10e3),
        ],
        atmega("vcc", TOGGLE, MHZ_16, FuseBytes { low: 0xDF, ..FUSES }),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pc6 = volts(&trace, "pc6").last().unwrap().1;
    let reset_high = documented("/reset_threshold/high/value");
    let input_high = documented("/input_threshold/high/value");
    assert!(pc6 > reset_high * 5.0 && pc6 < input_high * 5.0, "{pc6}");
    let pb5 = volts(&trace, "pb5");
    assert!(
        pb5.iter().filter(|&&(t, _)| t < US).all(|&(_, v)| v > 4.9),
        "held in reset"
    );
    assert!(
        pb5.iter().any(|&(t, v)| t > US && v < 0.1),
        "released and running"
    );
    let logic = trace.logic().unwrap();
    assert_eq!(logic.initial()[signal(&trace, "PC6")], Level::Low);
    assert_eq!(edges(&trace, "PC6"), [(US, true)]);
}

fn board(stop_ps: u64, nets: &[&str], parts: &[String], mcu: String) -> Netlist {
    let pins: Vec<String> = pin_nets().map(|(net, _)| net).collect();
    let nets: Vec<&str> = pins
        .iter()
        .map(String::as_str)
        .chain(nets.iter().copied())
        .collect();
    let mut parts = parts.to_vec();
    parts.push(mcu);
    circuit(stop_ps, &nets, &parts)
}

fn run(netlist: &Netlist) -> (Trace, Vec<Diagnostic>) {
    run_active(netlist, Vec::new())
}

fn core(netlist: &Netlist) -> avr::Core {
    let circuit = Circuit::new(netlist).unwrap();
    let [chip] = &circuit.actives[..] else {
        unreachable!("one ATmega328P");
    };
    let active::Core::Avr(core) = &chip.core else {
        unreachable!("an ATmega328P");
    };
    (**core).clone()
}

fn ps(cycle: u64, clock_hz: u32) -> u64 {
    (u128::from(cycle) * 1_000_000_000_000).div_ceil(u128::from(clock_hz)) as u64
}

/// `toggle.S` drives PB5 low at cycle 2 and toggles it at 4 and every 100 cycles after.
#[test]
fn a_pin_edge_lands_at_its_cycles_picosecond_into_an_rc() {
    let netlist = board(
        30 * US,
        &["cap"],
        &[
            resistor("r1", "pb5", "cap", 1000.0),
            capacitor("c1", "cap", 1e-9),
        ],
        atmega("vcc", TOGGLE, MHZ_14_7456, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pb5 = volts(&trace, "pb5");
    let cap = volts(&trace, "cap");
    let edges: Vec<u64> = [2, 4]
        .into_iter()
        .chain((1..).map(|k| 4 + 100 * k))
        .map(|cycle| ps(cycle, MHZ_14_7456))
        .take_while(|&at| at < 30 * US)
        .collect();
    assert_eq!(&edges[..3], [135_634, 271_268, 7_052_952]);
    for &edge in &edges {
        assert!(restarted(&pb5, edge), "a restart at {edge} ps");
    }
    assert!(
        (at(&cap, 0) - 2.5).abs() < 1e-4,
        "an off pin and its open RC rest on the leakage"
    );
    assert!(
        cap.iter()
            .filter(|&&(t, _)| t <= edges[0])
            .all(|&(_, v)| (v - 2.5).abs() < 1e-3),
        "held until PB5 drives low"
    );
    assert!(at(&cap, edges[1]) < 2.4, "{}", at(&cap, edges[1]));
    assert!(at(&cap, edges[2]) > 4.9, "{}", at(&cap, edges[2]));
}

/// 5_760_000 puts the crossing in a step that has already run the core to the 6.5 µs stop.
#[test]
fn toggle_stops_follow_the_firmware_when_a_watch_is_located() {
    let edge_ps = 5_760_000;
    let step = format!(
        r#"{{ "name": "vs", "kind": "voltage_source", "pins": {{ "p": "src", "n": "gnd" }},
          "voltage": {{ "kind": "step", "initial": {{ "value": 0.0, "unit": "V", "source": "user" }},
                       "final": {{ "value": 5.0, "unit": "V", "source": "user" }}, "at_ps": {edge_ps} }} }}"#
    );
    let stop = 20 * US;
    let netlist = board(
        stop,
        &["src"],
        &[
            step,
            resistor("rs", "src", "pd2", 1000.0),
            capacitor("cs", "pd2", 1e-9),
            resistor("rl", "pb5", "gnd", 1000.0),
        ],
        atmega("vcc", TOGGLE, MHZ_16, FUSES),
    );
    let (results, stats) = crate::simulate_stats(&netlist);
    assert_eq!(results.count(Severity::Error), 0);
    let firmware: Vec<u64> = [2u64, 4]
        .into_iter()
        .chain((1..).map(|k| 4 + 100 * k))
        .map(|cycle| cycle * 62_500)
        .take_while(|&at| at <= stop)
        .collect();
    assert_eq!(stats.stops, firmware.len() as u64, "{firmware:?}");
    let trace = results.trace().expect("a transient keeps a trace");
    let pd2 = edges(trace, "PD2");
    assert_eq!(pd2.len(), 1, "{pd2:?}");
    let crossing = pd2[0].0;
    assert!(
        !firmware.contains(&crossing),
        "the located input is not a firmware stop: {crossing}"
    );
    let input_high = documented("/input_threshold/high/value");
    let leak = 1.0 / documented("/leakage_resistance/value");
    let tau_ps = 1e-9 / (1e-3 + 2.0 * leak) * 1e12;
    let start = share(5.0, leak, leak + 1e-3);
    let end = share(5.0, 1e-3 + leak, leak);
    let expected = edge_ps as f64 + tau_ps * ((end - start) / (end - input_high * 5.0)).ln();
    assert!(
        (crossing as f64 - expected).abs() < 200.0,
        "located at {crossing}, formula {expected}"
    );
    let next_stop = firmware
        .iter()
        .copied()
        .find(|&at| at > crossing)
        .expect("a firmware stop after the crossing");
    let (step_trace, step_diagnostics) = run(&netlist);
    assert!(step_diagnostics.is_empty(), "{step_diagnostics:?}");
    let times: Vec<u64> = volts(&step_trace, "pd2")
        .into_iter()
        .map(|(at, _)| at)
        .collect();
    let located = times
        .iter()
        .position(|&at| at == crossing)
        .unwrap_or_else(|| panic!("crossing {crossing} is not an accepted step"));
    assert!(located >= 2, "the crossing needs a full step before it");
    assert_eq!(
        times[located + 1],
        next_stop,
        "the accepted step after the crossing is the firmware stop"
    );
    let predecessor_step = times[located - 1] - times[located - 2];
    let crossing_step_start = times[located - 1];
    let start_to_stop = next_stop - crossing_step_start;
    assert!(
        start_to_stop <= predecessor_step,
        "the crossing's step starts {start_to_stop} ps before {next_stop}, past the {predecessor_step} ps predecessor"
    );
    assert!(stats.landings > stats.stops);
}

#[test]
fn a_script_chip_runs_beside_an_atmega() {
    let netlist = board(
        20 * US,
        &["echo"],
        &[
            resistor("rl", "pb5", "gnd", 1000.0),
            resistor("re", "echo", "gnd", 1000.0),
        ],
        atmega("vcc", TOGGLE, MHZ_16, FUSES),
    );
    let script = scripted(&netlist, "echo", "pb5", &[], Some(US));
    let (trace, diagnostics) = run_active(&netlist, vec![script]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let (pb5, echo) = (volts(&trace, "pb5"), volts(&trace, "echo"));
    let high = 5.0 * 1000.0 / 1025.0;
    for (cycle, level) in [(4, high), (104, 0.0), (204, high)] {
        let edge = cycle * 62_500;
        assert!(restarted(&pb5, edge), "PB5 at cycle {cycle}");
        assert!(restarted(&echo, edge + US), "the echo of cycle {cycle}");
        assert!((at(&echo, edge + US + NS) - level).abs() < 1e-6);
    }
}

/// `pullup.S` turns PB5's pull-up on at cycle 2. Into its own resistance to ground, the pin
/// sits at half the supply.
#[test]
fn a_pull_up_pin_is_read_by_the_circuit() {
    let pull_up = documented("/pull_up_resistance/value");
    let leak = 1.0 / documented("/leakage_resistance/value");
    let netlist = board(
        2 * US,
        &[],
        &[resistor("rg", "pb5", "gnd", pull_up)],
        atmega("vcc", PULL, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pb5 = volts(&trace, "pb5");
    let off = share(5.0, leak, leak + 1.0 / pull_up);
    assert!(
        (at(&pb5, 125_000) - off).abs() < 1e-6,
        "{}",
        at(&pb5, 125_000)
    );
    assert!(restarted(&pb5, 125_000));
    assert!((pb5.last().unwrap().1 - 2.5).abs() < 1e-6);
}

#[test]
fn a_pull_up_that_leaves_the_level_still_stops() {
    let netlist = board(
        2 * US,
        &[],
        &[
            resistor("ru", "vcc", "pb5", 100e3),
            resistor("rd", "pb5", "gnd", 1e6),
        ],
        atmega("vcc", PULL, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pb5 = volts(&trace, "pb5");
    let leak = 1.0 / documented("/leakage_resistance/value");
    let before = share(5.0, 1.0 / 100e3 + leak, 1.0 / 1e6 + leak);
    assert!(before > documented("/input_threshold/high/value") * 5.0);
    assert!((at(&pb5, 125_000) - before).abs() < 1e-6);
    assert!(restarted(&pb5, 125_000));
    let up = 1.0 / 100e3 + 1.0 / documented("/pull_up_resistance/value") + leak;
    let after = share(5.0, up, 1.0 / 1e6 + leak);
    assert!((pb5.last().unwrap().1 - after).abs() < 1e-6);
}

#[test]
fn an_input_flip_reaches_the_core_and_comes_back_on_an_output() {
    let step = r#"{ "name": "vs", "kind": "voltage_source", "pins": { "p": "src", "n": "gnd" },
          "voltage": { "kind": "step", "initial": { "value": 0.0, "unit": "V", "source": "user" },
                       "final": { "value": 5.0, "unit": "V", "source": "user" }, "at_ps": 1000000 } }"#;
    let netlist = board(
        4 * US,
        &["src"],
        &[
            step.to_string(),
            resistor("rs", "src", "pd2", 1000.0),
            capacitor("cs", "pd2", 1e-9),
            resistor("rl", "pb5", "gnd", 1000.0),
        ],
        atmega("vcc", ECHO, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pd2 = volts(&trace, "pd2");
    let input_high = documented("/input_threshold/high/value");
    let crossing = pd2.iter().find(|&&(_, v)| v > input_high * 5.0).unwrap().0;
    let leak = 1.0 / documented("/leakage_resistance/value");
    let tau_ps = 1e-9 / (1e-3 + 2.0 * leak) * 1e12;
    let start = share(5.0, leak, leak + 1e-3);
    let end = share(5.0, 1e-3 + leak, leak);
    let expected = US as f64 + tau_ps * ((end - start) / (end - input_high * 5.0)).ln();
    assert!((crossing as f64 - expected).abs() < 200.0, "{crossing}");
    let pb5 = volts(&trace, "pb5");
    let rise = pb5.iter().find(|&&(_, v)| v > 2.5).unwrap().0;
    let landed = pb5.iter().rev().find(|&&(t, _)| t < rise).unwrap().0;
    assert_eq!(landed % 62_500, 0, "a stop on a cycle: {landed}");
    assert!(restarted(&pb5, landed));
    assert!(
        landed > crossing && landed <= crossing + 12 * 62_500,
        "{crossing} {landed}"
    );
}

/// `adc.S` converts ADC0 against AVCC and drives the code's top eight bits on port D. ADC0
/// sits on 6.8 kohm under 3.3 kohm from 5 V, 3.3663 V, code 689, and 689 >> 2 is 0b10101100.
#[test]
fn an_adc_sample_reads_a_known_divider() {
    let netlist = board(
        300 * US,
        &[],
        &[
            resistor("rt", "vcc", "pc0", 3300.0),
            resistor("rb", "pc0", "gnd", 6800.0),
        ],
        atmega("vcc", ADC, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let code: u8 = (0..8)
        .map(|bit| {
            let v = volts(&trace, &format!("pd{bit}")).last().unwrap().1;
            u8::from(v > 2.5) << bit
        })
        .sum();
    assert_eq!(code, 0b1010_1100);
}

/// PB5's edges at 16 MHz, one oscillator cycle every 62,500 ps.
fn clkpr_edges(hex: &str, fuses: FuseBytes) -> Vec<(u64, f64)> {
    let netlist = board(
        40 * US,
        &[],
        &[resistor("rl", "pb5", "gnd", 1000.0)],
        atmega("vcc", hex, MHZ_16, fuses),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    volts(&trace, "pb5")
}

/// §9.11 puts a new division in force `T1 + T2` to `T1 + 2 * T2` after the `STS` that writes
/// `CLKPR` retires. The first toggle is the `SBI` after it, those two edges, and each next one
/// is 100 CPU cycles later. Every edge is at its oscillator cycle.
///
/// `clkpr.S` goes from /1 to /2 with its store retiring at oscillator cycle 8: the window is 11
/// to 13 and the toggle at 11. `speedup.S`, under `CKDIV8`, goes from /8 to /1 with its store
/// retiring at oscillator cycle 64: the window is 73 to 74 and the toggle at 73.
#[test]
fn a_clkpr_division_keeps_edges_on_the_crystals_cycles() {
    let slower = clkpr_edges(CLKPR, FUSES);
    for osc in [2, 11, 211, 411, 611] {
        assert!(
            restarted(&slower, osc * 62_500),
            "an edge at oscillator cycle {osc}"
        );
    }
    assert!(
        slower.iter().all(|&(ps, _)| ps != 111 * 62_500),
        "no edge at the undivided rate"
    );
    let faster = clkpr_edges(SPEED_UP, FuseBytes { low: 0x7F, ..FUSES });
    for osc in [16, 73, 173, 273, 373] {
        assert!(
            restarted(&faster, osc * 62_500),
            "an edge at oscillator cycle {osc}"
        );
    }
    assert!(
        faster.iter().all(|&(ps, _)| ps != 66 * 62_500),
        "no edge two new periods after the store"
    );
}

/// `ports.S` drives every port pin high, and 2 V is under `VBOT-`. An off pin then sits where
/// its leakage and its load put it.
#[test]
fn a_brown_out_holds_every_pin_off() {
    let supply = r#"{ "name": "vd", "kind": "voltage_source", "pins": { "p": "vdd", "n": "gnd" },
          "voltage": { "kind": "step", "initial": { "value": 5.0, "unit": "V", "source": "user" },
                       "final": { "value": 2.0, "unit": "V", "source": "user" }, "at_ps": 20000000 } }"#;
    let netlist = board(
        30 * US,
        &["vdd"],
        &[supply.to_string(), resistor("rl", "pb5", "gnd", 10e3)],
        atmega("vdd", PORTS, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(edges(&trace, "VCC"), [(20 * US, false)]);
    let pb5 = volts(&trace, "pb5");
    assert!(at(&pb5, 20 * US) > 4.9, "{}", at(&pb5, 20 * US));
    assert!(
        restarted(&pb5, 20 * US + 62_500),
        "off one cycle after VCC fell"
    );
    let leak = 1.0 / documented("/leakage_resistance/value");
    let reset_up = 1.0 / documented("/reset_pull_up_resistance/value");
    for (name, pad) in pin_nets() {
        let v = volts(&trace, &name).last().unwrap().1;
        let expected = match pad {
            Pad::Port(Port::C, 6) => share(2.0, reset_up + leak, leak),
            Pad::Port(Port::B, 5) => share(2.0, leak, leak + 1.0 / 10e3),
            Pad::Xtal => continue,
            _ => 1.0,
        };
        assert!((v - expected).abs() < 1e-5, "{name} is off: {v}");
    }
}

fn signal(trace: &Trace, pin: &str) -> usize {
    let signal: Ref = format!("#u1.logic.{pin}").parse().unwrap();
    let logic = trace.logic().expect("an ATmega328P has logic signals");
    logic.signals().iter().position(|s| *s == signal).unwrap()
}

/// Each change of `#u1.logic.<pin>` after t = 0: its picosecond, and whether it reads high.
fn edges(trace: &Trace, pin: &str) -> Vec<(u64, bool)> {
    let index = signal(trace, pin);
    trace
        .logic()
        .unwrap()
        .changes()
        .iter()
        .filter(|change| change.signal == index)
        .map(|change| (change.at.as_ps(), change.level == Level::High))
        .collect()
}

/// Every port pin but the crystal's, then `VCC` while brown-out is fused on.
#[test]
fn each_watched_pin_is_a_signal_in_name_order() {
    let signals = |fuses: FuseBytes| {
        let netlist = board(US, &[], &[], atmega("vcc", TOGGLE, MHZ_16, fuses));
        let (trace, diagnostics) = run(&netlist);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let logic = trace.logic().unwrap();
        logic
            .signals()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let pins: Vec<String> = [("B", 6), ("C", 7), ("D", 8)]
        .into_iter()
        .flat_map(|(port, bits)| (0..bits).map(move |bit| format!("#u1.logic.P{port}{bit}")))
        .collect();
    let mut with_vcc = pins.clone();
    with_vcc.push("#u1.logic.VCC".to_string());
    assert_eq!(signals(FUSES), with_vcc);
    let no_brown_out = FuseBytes {
        extended: 0xFF,
        ..FUSES
    };
    assert_eq!(signals(no_brown_out), pins);
}

/// `toggle.S` drives PB5 low at cycle 2, onto the low its load already holds, then high at 4,
/// and toggles it every 100 cycles. The pin reads each level its drive makes at the drive's
/// stop, which the restart's solve holds.
#[test]
fn a_driven_pin_reads_its_own_drive_at_its_stop() {
    let netlist = board(
        40 * US,
        &[],
        &[resistor("rl", "pb5", "gnd", 1000.0)],
        atmega("vcc", TOGGLE, MHZ_14_7456, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let expected: Vec<(u64, bool)> = (0..)
        .map(|k| ps(4 + 100 * k, MHZ_14_7456))
        .take_while(|&at| at < 40 * US)
        .zip([true, false].into_iter().cycle())
        .collect();
    assert_eq!(expected[..2], [(271_268, true), (7_052_952, false)]);
    assert_eq!(edges(&trace, "PB5"), expected);
    let pb5 = volts(&trace, "pb5");
    for &(at, _) in &expected {
        assert!(restarted(&pb5, at), "a restart at {at} ps");
    }
    let pb5 = signal(&trace, "PB5");
    let logic = trace.logic().unwrap();
    assert!(logic.changes().iter().all(|change| change.signal == pb5));
}

/// A pulse straight onto PD2 reads high and low at its jumps, which the restart's solve holds.
#[test]
fn an_input_on_a_source_reads_the_sources_jumps() {
    let pulse = r#"{ "name": "vs", "kind": "voltage_source", "pins": { "p": "pd2", "n": "gnd" },
          "voltage": { "kind": "pulse", "initial": { "value": 0.0, "unit": "V", "source": "user" },
                       "pulsed": { "value": 5.0, "unit": "V", "source": "user" },
                       "delay_ps": 2000000, "width_ps": 3000000, "period_ps": 10000000 } }"#;
    let netlist = board(
        24 * US,
        &[],
        &[pulse.to_string()],
        atmega("vcc", TOGGLE, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        edges(&trace, "PD2"),
        [
            (2 * US, true),
            (5 * US, false),
            (12 * US, true),
            (15 * US, false),
            (22 * US, true),
        ]
    );
}

/// A pulse through 10 kohm into 1 nF ramps PD2 up and down over about 10 us. It reads high at
/// the first picosecond above the high fraction of VCC and low at the first at or under the
/// low one, once each. `echo.S` copies PD2 to PB5, which follows each flip on a cycle.
#[test]
fn a_slow_input_flips_once_at_each_threshold() {
    let pulse = r#"{ "name": "vs", "kind": "voltage_source", "pins": { "p": "src", "n": "gnd" },
          "voltage": { "kind": "pulse", "initial": { "value": 0.0, "unit": "V", "source": "user" },
                       "pulsed": { "value": 5.0, "unit": "V", "source": "user" },
                       "delay_ps": 1000000, "width_ps": 40000000, "period_ps": 200000000 } }"#;
    let netlist = board(
        80 * US,
        &["src"],
        &[
            pulse.to_string(),
            resistor("rs", "src", "pd2", 10e3),
            capacitor("cs", "pd2", 1e-9),
            resistor("rl", "pb5", "gnd", 1000.0),
        ],
        atmega("vcc", ECHO, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pd2 = edges(&trace, "PD2");
    let [(rise, true), (fall, false)] = pd2[..] else {
        panic!("{pd2:?}");
    };
    let v = volts(&trace, "pd2");
    let high = documented("/input_threshold/high/value") * 5.0;
    let low = documented("/input_threshold/low/value") * 5.0;
    let slope = 5.0 / 10e6;
    assert!(
        at(&v, rise) > high && at(&v, rise) - high < slope,
        "{}",
        at(&v, rise)
    );
    assert!(
        at(&v, fall) <= low && low - at(&v, fall) < slope,
        "{}",
        at(&v, fall)
    );
    let pb5 = edges(&trace, "PB5");
    let [(up, true), (down, false)] = pb5[..] else {
        panic!("{pb5:?}");
    };
    for (flip, follow) in [(rise, up), (fall, down)] {
        assert_eq!(follow % 62_500, 0, "a stop on a cycle: {follow}");
        assert!(
            follow > flip && follow <= flip + 12 * 62_500,
            "{flip} {follow}"
        );
    }
}

fn supply(initial: f64, pulsed: f64) -> String {
    format!(
        r#"{{ "name": "vd", "kind": "voltage_source", "pins": {{ "p": "vdd", "n": "gnd" }},
          "voltage": {{ "kind": "pulse", "initial": {{ "value": {initial:?}, "unit": "V", "source": "user" }},
                       "pulsed": {{ "value": {pulsed:?}, "unit": "V", "source": "user" }},
                       "delay_ps": 10000000, "width_ps": 10000000, "period_ps": 100000000 }} }}"#
    )
}

#[test]
fn vcc_reads_the_supply_against_the_brown_out_window() {
    let vcc = |initial: f64, pulsed: f64| {
        let netlist = board(
            30 * US,
            &["vdd"],
            &[supply(initial, pulsed)],
            atmega("vdd", TOGGLE, MHZ_16, FUSES),
        );
        let (trace, diagnostics) = run(&netlist);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let initial = trace.logic().unwrap().initial()[signal(&trace, "VCC")];
        (initial, edges(&trace, "VCC"))
    };
    assert_eq!(
        vcc(5.0, 2.0),
        (Level::High, vec![(10 * US, false), (20 * US, true)])
    );
    assert_eq!(
        vcc(2.0, 5.0),
        (Level::Low, vec![(10 * US, true), (20 * US, false)])
    );
}

#[test]
fn a_flip_the_restart_undoes_returns_at_the_next_step() {
    let (start, stop, jump) = (NS, 9 * NS, 6_201);
    let volts_per_ps = 1.0 / 2e-9 * 1e-12;
    let high = documented("/input_threshold/high/value") * 5.0;
    let low = documented("/input_threshold/low/value") * 5.0;
    let crossing = jump as f64 - 0.5;
    let initial = high - volts_per_ps * (crossing - start as f64);
    let ramp = format!(
        r#"{{ "name": "is", "kind": "current_source", "pins": {{ "p": "cap", "n": "gnd" }},
          "current": {{ "kind": "pulse", "initial": {{ "value": 0.0, "unit": "A", "source": "user" }},
                       "pulsed": {{ "value": 1.0, "unit": "A", "source": "user" }},
                       "delay_ps": {start}, "width_ps": {}, "period_ps": {} }} }}"#,
        stop - start,
        2 * US,
    );
    let cap = format!(
        r#"{{ "name": "cs", "kind": "capacitor", "pins": {{ "a": "cap", "b": "gnd" }},
          "capacitance": {{ "value": 2e-9, "unit": "F", "source": "user" }},
          "initial": {{ "value": {initial:?}, "unit": "V", "source": "user" }} }}"#
    );
    let back = format!(
        r#"{{ "name": "vb", "kind": "voltage_source", "pins": {{ "p": "pd2", "n": "cap" }},
          "voltage": {{ "kind": "step", "initial": {{ "value": 0.0, "unit": "V", "source": "user" }},
                       "final": {{ "value": -2.0, "unit": "V", "source": "user" }}, "at_ps": {jump} }} }}"#
    );
    let netlist = board(
        US,
        &["cap"],
        &[ramp, cap, back],
        atmega("vcc", TOGGLE, MHZ_16, FUSES),
    );
    let (trace, diagnostics) = run(&netlist);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pd2 = volts(&trace, "pd2");
    let before = pd2.iter().rfind(|&&(t, _)| t < jump).unwrap();
    assert!(before.1 < high, "{before:?}");
    assert!(at(&pd2, jump) > high, "{}", at(&pd2, jump));
    assert!(restarted(&pd2, jump), "a restart at the jump");
    assert!(at(&pd2, US) < low, "{}", at(&pd2, US));
    assert_eq!(edges(&trace, "PD2"), [(jump, true), (jump + NS, false)]);
}

#[test]
fn a_drive_at_stop_ps_is_not_in_the_run() {
    let pb5 = |stop_ps: u64| {
        let netlist = board(
            stop_ps,
            &[],
            &[resistor("rl", "pb5", "gnd", 1000.0)],
            atmega("vcc", TOGGLE, MHZ_16, FUSES),
        );
        let (trace, diagnostics) = run(&netlist);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        edges(&trace, "PB5")
    };
    assert_eq!(pb5(6_500_000), [(250_000, true)]);
    assert_eq!(pb5(6_500_001), [(250_000, true), (6_500_000, false)]);
}

fn stops(core: &mut avr::Core, limit: u64) -> Vec<(u64, Drive)> {
    let pb5 = port_index(Pad::Port(Port::B, 5));
    let mut stops = Vec::new();
    while let Some(stop) = core.run_until(sim_core::time::Time::from_ps(limit)) {
        stops.push((stop.at.as_ps(), core.drive()[pb5]));
    }
    stops
}

/// Across `CLKPR` switches too: `clkpr.S` slows down and `speedup.S` under `CKDIV8` speeds up.
#[test]
fn a_run_split_at_any_picosecond_is_the_same_run() {
    let ckdiv8 = FuseBytes { low: 0x7F, ..FUSES };
    for (hex, fuses, edges) in [(TOGGLE, FUSES, 4), (CLKPR, FUSES, 3), (SPEED_UP, ckdiv8, 4)] {
        let load = [resistor("rl", "pb5", "gnd", 1000.0)];
        let start = core(&board(
            US,
            &[],
            &load,
            atmega("vcc", hex, MHZ_14_7456, fuses),
        ));
        let limit = ps(300, MHZ_14_7456);
        let run = |split: Option<u64>| {
            let mut core = start.clone();
            let mut all = split.map_or_else(Vec::new, |split| stops(&mut core, split));
            all.extend(stops(&mut core, limit));
            (all, format!("{core:?}"))
        };
        let whole = run(None);
        assert_eq!(whole.0.len(), edges);
        for cycle in 0..300 {
            for split in [
                ps(cycle, MHZ_14_7456).saturating_sub(1),
                ps(cycle, MHZ_14_7456) + 1,
            ] {
                assert_eq!(run(Some(split)), whole, "split at {split} ps");
            }
        }
    }
}

/// The first stop, at cycle 2, is `SBI DDRB,5` turning PB5 from off to driven low. That
/// changes the image but not the level its load already holds.
#[test]
fn blink_stops_once_for_its_output_and_once_per_edge() {
    let load = [resistor("rl", "pb5", "gnd", 1000.0)];
    let mut core = core(&board(US, &[], &load, atmega("vcc", BLINK, MHZ_16, FUSES)));
    let stops = stops(&mut core, ps(24_000_004, MHZ_16));
    let cycles: Vec<(u64, Drive)> = stops
        .iter()
        .map(|&(at, drive)| (at / 62_500, drive))
        .collect();
    assert_eq!(
        cycles,
        [
            (2, Drive::Low),
            (4, Drive::High),
            (8_000_004, Drive::Low),
            (16_000_004, Drive::High),
            (24_000_004, Drive::Low),
        ]
    );
    let edges = cycles.windows(2).filter(|w| w[0].1 != w[1].1).count();
    assert_eq!((stops.len(), edges), (5, 4));
}

/// Wall time of ten seconds of Blink, 160e6 cycles, on the bare core and through the seam.
/// `cargo test --release -p sim-circuit --lib blink_throughput -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
#[expect(clippy::disallowed_methods, reason = "the clock is the measurement")]
fn blink_throughput() {
    const CYCLES: u64 = 160_000_000;
    let mut cpu = sim_avr::Cpu::new();
    cpu.set_fuses(FUSES.low, FUSES.high, FUSES.extended);
    cpu.load_image(&sim_avr::parse_hex(BLINK).unwrap()).unwrap();
    let start = std::time::Instant::now();
    let mut edges = 0;
    while cpu.cycles() < CYCLES {
        cpu.run_until(CYCLES);
        while cpu.take_pin().is_some() {
            edges += 1;
        }
    }
    let raw = start.elapsed().as_secs_f64();
    let load = [resistor("rl", "pb5", "gnd", 1000.0)];
    let mut core = core(&board(US, &[], &load, atmega("vcc", BLINK, MHZ_16, FUSES)));
    let start = std::time::Instant::now();
    let stops = stops(&mut core, ps(CYCLES, MHZ_16)).len();
    let seam = start.elapsed().as_secs_f64();
    println!(
        "bare core {raw:.3} s, {:.1}e6 cycles/s, {edges} reports; through the seam {seam:.3} s, \
         {:.1}e6 cycles/s, {stops} stops",
        CYCLES as f64 / raw / 1e6,
        CYCLES as f64 / seam / 1e6,
    );
}

/// Wall time of 32e6 cycles of `button.S` on the bare core and through the seam.
/// The example runs 16e6 cycles.
/// `cargo test --release -p sim-circuit --lib button_throughput -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
#[expect(clippy::disallowed_methods, reason = "the clock is the measurement")]
fn button_throughput() {
    const CYCLES: u64 = 32_000_000;
    let mut cpu = sim_avr::Cpu::new();
    cpu.set_fuses(FUSES.low, FUSES.high, FUSES.extended);
    cpu.load_image(&sim_avr::parse_hex(BUTTON).unwrap())
        .unwrap();
    let start = std::time::Instant::now();
    let mut edges = 0;
    while cpu.cycles() < CYCLES {
        cpu.run_until(CYCLES);
        while cpu.take_pin().is_some() {
            edges += 1;
        }
    }
    let raw = start.elapsed().as_secs_f64();
    let load = [resistor("rl", "pb5", "gnd", 1000.0)];
    let mut core = core(&board(US, &[], &load, atmega("vcc", BUTTON, MHZ_16, FUSES)));
    let start = std::time::Instant::now();
    let stops = stops(&mut core, ps(CYCLES, MHZ_16)).len();
    let seam = start.elapsed().as_secs_f64();
    println!(
        "bare core {raw:.3} s, {:.1}e6 cycles/s, {edges} reports; through the seam {seam:.3} s, \
         {:.1}e6 cycles/s, {stops} stops",
        CYCLES as f64 / raw / 1e6,
        CYCLES as f64 / seam / 1e6,
    );
}
