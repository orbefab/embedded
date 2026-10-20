//! The ATmega328P through the seam, running the repo's firmware in `firmware/`.

use std::sync::Arc;

use sim_avr::{FuseBytes, Port};
use sim_core::netlist::Netlist;

use super::active::{US, at, capacitor, circuit, net, resistor, restarted, run_active, volts};
use crate::active::avr::{
    self, Atmega328p, NotACrystal, PACKAGE, PULL_UP, Pad, RESET_PULL_UP, RESET_THRESHOLD, STAGE,
    THRESHOLD, port_index,
};
use crate::active::{self, Drive};

const BLINK: &str = include_str!("../../../../../firmware/blink.hex");
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

/// The net of each package pin that is not a supply: the pin's name in lowercase.
fn pin_nets() -> impl Iterator<Item = (String, Pad)> {
    PACKAGE
        .iter()
        .filter(|(_, pad)| matches!(pad, Pad::Port(..) | Pad::Xtal))
        .map(|&(name, pad)| (name.to_lowercase(), pad))
}

fn atmega(netlist: &Netlist, vdd: &str, hex: &str, clock_hz: u32, fuses: FuseBytes) -> Atmega328p {
    let pins = PACKAGE.map(|(name, pad)| match pad {
        Pad::Vcc | Pad::Avcc | Pad::Aref => net(netlist, vdd),
        Pad::Gnd | Pad::Agnd => net(netlist, "gnd"),
        Pad::Port(..) | Pad::Xtal => net(netlist, &name.to_lowercase()),
    });
    Atmega328p {
        pins,
        high: STAGE,
        low: STAGE,
        pull_up: PULL_UP,
        reset_pull_up: RESET_PULL_UP,
        threshold: THRESHOLD,
        reset_threshold: RESET_THRESHOLD,
        clock_hz,
        fuses,
        flash: Arc::from(sim_avr::parse_hex(hex).unwrap().bytes()),
    }
}

fn mcu(netlist: &Netlist, vdd: &str, hex: &str, clock_hz: u32) -> active::Chip {
    avr::chip(&atmega(netlist, vdd, hex, clock_hz, FUSES)).unwrap()
}

#[test]
fn the_factory_internal_rc_fuses_are_no_chip() {
    let netlist = board(US, &[], &[resistor("rl", "pb5", "gnd", 1000.0)]);
    let factory = FuseBytes {
        low: 0x62,
        high: 0xD9,
        extended: 0xFF,
    };
    let chip = avr::chip(&atmega(&netlist, "vcc", TOGGLE, MHZ_16, factory));
    assert_eq!(chip.err(), Some(NotACrystal { cksel: 0b0010 }));
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
    );
    let fuses = FuseBytes { low: 0xDF, ..FUSES };
    let chip = avr::chip(&atmega(&netlist, "vcc", TOGGLE, MHZ_16, fuses)).unwrap();
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pc6 = volts(&trace, "pc6").last().unwrap().1;
    assert!(
        pc6 > RESET_THRESHOLD.high * 5.0 && pc6 < THRESHOLD.high * 5.0,
        "{pc6}"
    );
    let pb5 = volts(&trace, "pb5");
    assert!(
        pb5.iter().filter(|&&(t, _)| t < US).all(|&(_, v)| v > 4.9),
        "held in reset"
    );
    assert!(
        pb5.iter().any(|&(t, v)| t > US && v < 0.1),
        "released and running"
    );
}

fn board(stop_ps: u64, nets: &[&str], parts: &[String]) -> Netlist {
    let pins: Vec<String> = pin_nets().map(|(net, _)| net).collect();
    let nets: Vec<&str> = pins
        .iter()
        .map(String::as_str)
        .chain(nets.iter().copied())
        .collect();
    circuit(stop_ps, &nets, parts)
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
    );
    let chip = mcu(&netlist, "vcc", TOGGLE, MHZ_14_7456);
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
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
    let first_high = edges[1];
    assert!(
        cap.iter()
            .filter(|&&(t, _)| t <= first_high)
            .all(|&(_, v)| v.abs() < 1e-9)
    );
    assert!(at(&cap, edges[2]) > 1.0, "{}", at(&cap, edges[2]));
}

/// `pullup.S` turns PB5's pull-up on at cycle 2. Into its own resistance to ground, the pin
/// sits at half the supply.
#[test]
fn a_pull_up_pin_is_read_by_the_circuit() {
    let netlist = board(2 * US, &[], &[resistor("rg", "pb5", "gnd", PULL_UP)]);
    let chip = mcu(&netlist, "vcc", PULL, MHZ_16);
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pb5 = volts(&trace, "pb5");
    assert!(at(&pb5, 125_000).abs() < 1e-6, "{}", at(&pb5, 125_000));
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
    );
    let chip = mcu(&netlist, "vcc", PULL, MHZ_16);
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pb5 = volts(&trace, "pb5");
    let before = 5.0 * 1e6 / 1.1e6;
    assert!(before > THRESHOLD.high * 5.0);
    assert!((at(&pb5, 125_000) - before).abs() < 1e-6);
    assert!(restarted(&pb5, 125_000));
    let up = 1.0 / (1.0 / 100e3 + 1.0 / PULL_UP);
    let after = 5.0 * 1e6 / (1e6 + up);
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
    );
    let chip = mcu(&netlist, "vcc", ECHO, MHZ_16);
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pd2 = volts(&trace, "pd2");
    let crossing = pd2
        .iter()
        .find(|&&(_, v)| v > THRESHOLD.high * 5.0)
        .unwrap()
        .0;
    let expected = US as f64 * (1.0 - (1.0 - THRESHOLD.high).ln());
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
    );
    let chip = mcu(&netlist, "vcc", ADC, MHZ_16);
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
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
    let netlist = board(40 * US, &[], &[resistor("rl", "pb5", "gnd", 1000.0)]);
    let chip = avr::chip(&atmega(&netlist, "vcc", hex, MHZ_16, fuses)).unwrap();
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
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

/// `ports.S` drives every port pin high, and 2 V is under `VBOT-`.
#[test]
fn a_brown_out_holds_every_pin_off() {
    let supply = r#"{ "name": "vd", "kind": "voltage_source", "pins": { "p": "vdd", "n": "gnd" },
          "voltage": { "kind": "step", "initial": { "value": 5.0, "unit": "V", "source": "user" },
                       "final": { "value": 2.0, "unit": "V", "source": "user" }, "at_ps": 20000000 } }"#;
    let netlist = board(
        30 * US,
        &["vdd"],
        &[supply.to_string(), resistor("rl", "pb5", "gnd", 10e3)],
    );
    let chip = mcu(&netlist, "vdd", PORTS, MHZ_16);
    let (trace, diagnostics) = run_active(&netlist, vec![chip]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let pb5 = volts(&trace, "pb5");
    assert!(at(&pb5, 20 * US) > 4.9, "{}", at(&pb5, 20 * US));
    assert!(
        restarted(&pb5, 20 * US + 62_500),
        "off one cycle after VCC fell"
    );
    for (name, pad) in pin_nets() {
        let v = volts(&trace, &name).last().unwrap().1;
        match pad {
            Pad::Port(Port::C, 6) => assert!((v - 2.0).abs() < 1e-3, "RESET pulls up: {v}"),
            Pad::Xtal => {}
            _ => assert!(v.abs() < 1e-6, "{name} is off: {v}"),
        }
    }
}

fn core(chip: active::Chip) -> avr::Core {
    let active::Core::Avr(core) = chip.core else {
        unreachable!("an ATmega328P");
    };
    *core
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
    let netlist = board(US, &[], &[resistor("rl", "pb5", "gnd", 1000.0)]);
    let ckdiv8 = FuseBytes { low: 0x7F, ..FUSES };
    for (hex, fuses, edges) in [(TOGGLE, FUSES, 4), (CLKPR, FUSES, 3), (SPEED_UP, ckdiv8, 4)] {
        let chip = avr::chip(&atmega(&netlist, "vcc", hex, MHZ_14_7456, fuses)).unwrap();
        let start = core(chip);
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
    let netlist = board(US, &[], &[resistor("rl", "pb5", "gnd", 1000.0)]);
    let mut core = core(mcu(&netlist, "vcc", BLINK, MHZ_16));
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
    let netlist = board(US, &[], &[resistor("rl", "pb5", "gnd", 1000.0)]);
    let mut core = core(mcu(&netlist, "vcc", BLINK, MHZ_16));
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
