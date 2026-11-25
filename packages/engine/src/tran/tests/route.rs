//! Record routing: a `Pulses` or `Spi` record stops the run with `chip.link_unsupported` on the
//! first record whose active net reaches any element other than the sender's pin, a part that
//! takes that record kind on that pin, or a pull-up or pull-down resistor to a fixed rail.

use std::sync::Arc;

use orbe_model::Time;

use crate::blob::Blobs;
use crate::diagnostic::{Diagnostic, LinkKind};
use crate::netlist::Netlist;
use crate::session::{BUDGET, Session, Status};

use super::active::{US, capacitor, circuit, resistor};
use super::fixtures::open;
use super::spi::jedec_id_program;

const STOP: u64 = 30 * US;

const STAGE: &str = r#"{
    "high_drop": { "value": 0.0, "unit": "V" },
    "high_resistance": { "value": 25.0, "unit": "ohm" },
    "low_drop": { "value": 0.0, "unit": "V" },
    "low_resistance": { "value": 25.0, "unit": "ohm" }
}"#;

const THRESHOLD: &str = r#"{
    "low": { "value": 0.3, "unit": "1" },
    "high": { "value": 0.7, "unit": "1" }
}"#;

fn pulses_replay(name: &str, out: &str, at: u64) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "pulses_replay",
          "pins": {{ "VCC": "vcc", "GND": "gnd", "OUT": "{out}" }},
          "idle": false, "output": {STAGE},
          "events": [{{ "t_ps": {at}, "pulses": [{{ "level": true, "ps": 400000 }}, {{ "level": false, "ps": 850000 }}] }}] }}"#
    )
}

fn ws2812(name: &str, din: &str, dout: Option<&str>) -> String {
    let pins = match dout {
        Some(dout) => format!(r#""VCC": "vcc", "GND": "gnd", "DIN": "{din}", "DOUT": "{dout}""#),
        None => format!(r#""VCC": "vcc", "GND": "gnd", "DIN": "{din}""#),
    };
    let dout_field = match dout {
        Some(_) => format!(
            r#", "dout": {{ "delay": {{ "value": 100000, "unit": "ps" }}, "output": {STAGE} }}"#
        ),
        None => String::new(),
    };
    format!(
        r#"{{ "name": "{name}", "kind": "ws2812", "pins": {{ {pins} }},
          "leds": 1, "input_threshold": {THRESHOLD}{dout_field} }}"#
    )
}

fn run_session(netlist: &Netlist) -> Session {
    let mut session = open(netlist, Vec::new(), BUDGET);
    session.advance_to(Time::from_ps(STOP));
    session
}

#[test]
fn pulses_through_a_series_resistor_faults_at_the_first_record() {
    let netlist = circuit(
        STOP,
        &["out", "din"],
        &[
            pulses_replay("rp1", "out", 5 * US),
            resistor("r1", "out", "din", 330.0),
            ws2812("px1", "din", None),
        ],
    );
    let session = run_session(&netlist);
    assert_eq!(session.status(), Status::Faulted);
    assert_eq!(
        session.fault(),
        Some(&Diagnostic::ChipLinkUnsupported {
            pin: "#rp1.pin.OUT".parse().unwrap(),
            other: "#r1.pin.a".parse().unwrap(),
            at: Time::from_ps(5 * US + 1_250_000),
            peripheral: "pulses_replay",
            kind: LinkKind::Pulses,
        })
    );
}

#[test]
fn pulses_with_pull_up_and_pull_down_resistors_still_succeeds() {
    let netlist = circuit(
        STOP,
        &["din"],
        &[
            pulses_replay("rp1", "din", 5 * US),
            resistor("rpu", "din", "vcc", 10_000.0),
            resistor("rpd", "din", "gnd", 10_000.0),
            ws2812("px1", "din", None),
        ],
    );
    let session = run_session(&netlist);
    assert_eq!(session.status(), Status::Stopped);
    assert_eq!(session.fault(), None);
}

#[test]
fn pulses_with_a_capacitor_to_ground_faults() {
    let netlist = circuit(
        STOP,
        &["din"],
        &[
            pulses_replay("rp1", "din", 5 * US),
            capacitor("c1", "din", 1e-9),
            ws2812("px1", "din", None),
        ],
    );
    let session = run_session(&netlist);
    assert_eq!(session.status(), Status::Faulted);
    assert!(matches!(
        session.fault(),
        Some(Diagnostic::ChipLinkUnsupported {
            other,
            kind: LinkKind::Pulses,
            ..
        }) if other.to_string() == "#c1.pin.a"
    ));
}

#[test]
fn ws2812_relayed_dout_through_a_series_resistor_faults() {
    // 25 bits: 24 for px1, 1 relayed on DOUT.
    let mut pulses = Vec::new();
    for _ in 0..25 {
        pulses.push(r#"{ "level": true, "ps": 400000 }"#);
        pulses.push(r#"{ "level": false, "ps": 850000 }"#);
    }
    let rp1 = format!(
        r#"{{ "name": "rp1", "kind": "pulses_replay",
          "pins": {{ "VCC": "vcc", "GND": "gnd", "OUT": "din1" }},
          "idle": false, "output": {STAGE},
          "events": [{{ "t_ps": {}, "pulses": [{}] }}] }}"#,
        US,
        pulses.join(", ")
    );
    let netlist = circuit(
        100 * US,
        &["din1", "dout1", "din2"],
        &[
            rp1,
            ws2812("px1", "din1", Some("dout1")),
            resistor("r1", "dout1", "din2", 330.0),
            ws2812("px2", "din2", None),
        ],
    );
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.advance_to(Time::from_ps(100 * US));
    assert_eq!(session.status(), Status::Faulted);
    assert!(matches!(
        session.fault(),
        Some(Diagnostic::ChipLinkUnsupported {
            pin,
            other,
            kind: LinkKind::Pulses,
            ..
        }) if pin.to_string() == "#px1.pin.DOUT" && other.to_string() == "#r1.pin.a"
    ));
}

fn s3_spi_netlist(
    extra_nets: &[&str],
    gpio10: &str,
    gpio11: &str,
    gpio12: &str,
    gpio13: &str,
    extra_parts: &[String],
) -> Netlist {
    let elf = jedec_id_program().elf();
    let mut blobs = Blobs::new();
    let bytes = elf.len();
    let sha256 = blobs.add(Arc::from(elf));
    let ohm = |value: f64| format!(r#"{{ "value": {value:?}, "unit": "ohm" }}"#);
    let mut nets = vec!["gnd", "vdd", "cs", "mosi", "sclk", "miso", "ok"];
    nets.extend_from_slice(extra_nets);
    let nets_json: Vec<String> = nets.iter().map(|n| format!(r#""{n}""#)).collect();
    let mut parts = vec![
        r#"{ "name": "v1", "kind": "voltage_source", "pins": { "p": "vdd", "n": "gnd" },
          "voltage": { "value": 3.3, "unit": "V" } }"#
            .to_string(),
        format!(
            r#"{{ "name": "u1", "kind": "esp32s3",
              "pins": {{ "VCC": "vdd", "GND": "gnd", "GPIO10": "{gpio10}", "GPIO11": "{gpio11}",
                        "GPIO12": "{gpio12}", "GPIO13": "{gpio13}", "GPIO21": "ok" }},
              "firmware": {{ "elf": {{ "sha256": "{sha256}", "bytes": {bytes} }} }},
              "output": {{ "high_drop": {{ "value": 0.0, "unit": "V" }}, "high_resistance": {r},
                          "low_drop": {{ "value": 0.0, "unit": "V" }}, "low_resistance": {r} }},
              "pull_up_resistance": {pull},
              "leakage_resistance": {leak},
              "input_threshold": {{ "low": {{ "value": 0.25, "unit": "1" }},
                                   "high": {{ "value": 0.75, "unit": "1" }} }} }}"#,
            r = ohm(17.0),
            pull = ohm(45e3),
            leak = ohm(66e6),
        ),
    ];
    parts.extend(extra_parts.iter().cloned());
    let json = format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {US}, "grid_ps": {US} }} }},
          "ground": "gnd",
          "nets": [{nets}],
          "parts": [{parts}]
        }}"#,
        stop = 20 * US,
        nets = nets_json.join(", "),
        parts = parts.join(", "),
    );
    crate::netlist::parse_with(json.as_bytes(), &blobs).unwrap()
}

#[test]
fn s3_spi2_mosi_shorted_to_miso_faults_at_the_first_record() {
    let netlist = s3_spi_netlist(&[], "cs", "mosi", "sclk", "mosi", &[]);
    let session = run_session(&netlist);
    assert_eq!(session.status(), Status::Faulted);
    let Some(Diagnostic::ChipLinkUnsupported {
        pin,
        other,
        peripheral,
        kind,
        ..
    }) = session.fault()
    else {
        panic!("expected ChipLinkUnsupported, got {:?}", session.fault());
    };
    assert_eq!(pin.to_string(), "#u1.pin.GPIO11");
    assert_eq!(other.to_string(), "#u1.pin.GPIO13");
    assert_eq!(*peripheral, "SPI2");
    assert_eq!(*kind, LinkKind::Spi);
}

#[test]
fn s3_spi2_into_hc595_faults_at_the_first_record() {
    let hc595 = format!(
        r#"{{ "name": "sr1", "kind": "hc595",
          "pins": {{ "VCC": "vdd", "GND": "gnd", "SER": "mosi", "SRCLK": "sclk", "RCLK": "cs",
                    "OE": "gnd", "SRCLR": "vdd",
                    "QA": "qa", "QB": "qb", "QC": "qc", "QD": "qd",
                    "QE": "qe", "QF": "qf", "QG": "qg", "QH": "qh", "QHp": "qhp" }},
          "output": {STAGE}, "serial_output": {STAGE}, "input_threshold": {THRESHOLD},
          "ratings": {{ "pin_current": null, "supply_current": null, "input_margin": null, "output_margin": null }} }}"#
    );
    let netlist = s3_spi_netlist(
        &["qa", "qb", "qc", "qd", "qe", "qf", "qg", "qh", "qhp"],
        "cs",
        "mosi",
        "sclk",
        "miso",
        &[hc595],
    );
    let session = run_session(&netlist);
    assert_eq!(session.status(), Status::Faulted);
    let Some(Diagnostic::ChipLinkUnsupported {
        pin,
        other,
        peripheral,
        kind,
        ..
    }) = session.fault()
    else {
        panic!("expected ChipLinkUnsupported, got {:?}", session.fault());
    };
    assert_eq!(pin.to_string(), "#u1.pin.GPIO10");
    assert_eq!(other.to_string(), "#sr1.pin.RCLK");
    assert_eq!(*peripheral, "SPI2");
    assert_eq!(*kind, LinkKind::Spi);
}

#[test]
fn s3_spi2_wired_straight_to_st77916_with_pull_up_succeeds() {
    let panel = format!(
        r#"{{ "name": "lcd1", "kind": "st77916",
          "pins": {{ "VCC": "vdd", "GND": "gnd", "CS": "cs", "SCLK": "sclk",
                    "D0": "mosi", "D1": "miso", "D2": "d2", "D3": "d3",
                    "RST": "vdd", "TE": "te", "BLK": "gnd" }},
          "output": {STAGE}, "input_threshold": {THRESHOLD} }}"#
    );
    let pull = r#"{ "name": "rcs", "kind": "resistor", "pins": { "a": "cs", "b": "vdd" },
        "resistance": { "value": 10000.0, "unit": "ohm" }, "ratings": { "power": null } }"#
        .to_string();
    let netlist = s3_spi_netlist(
        &["d2", "d3", "te"],
        "cs",
        "mosi",
        "sclk",
        "miso",
        &[panel, pull],
    );
    let session = run_session(&netlist);
    assert_eq!(session.status(), Status::Stopped);
    assert_eq!(session.fault(), None);
}

#[test]
fn s3_spi2_miso_through_a_series_resistor_faults_on_read() {
    let panel = format!(
        r#"{{ "name": "lcd1", "kind": "st77916",
          "pins": {{ "VCC": "vdd", "GND": "gnd", "CS": "cs", "SCLK": "sclk",
                    "D0": "mosi", "D1": "d1", "D2": "d2", "D3": "d3",
                    "RST": "vdd", "TE": "te", "BLK": "gnd" }},
          "output": {STAGE}, "input_threshold": {THRESHOLD} }}"#
    );
    let series = r#"{ "name": "rmiso", "kind": "resistor", "pins": { "a": "miso", "b": "d1" },
        "resistance": { "value": 330.0, "unit": "ohm" }, "ratings": { "power": null } }"#
        .to_string();
    let netlist = s3_spi_netlist(
        &["d1", "d2", "d3", "te"],
        "cs",
        "mosi",
        "sclk",
        "miso",
        &[panel, series],
    );
    let session = run_session(&netlist);
    assert_eq!(session.status(), Status::Faulted);
    let Some(Diagnostic::ChipLinkUnsupported {
        pin,
        other,
        peripheral,
        kind,
        ..
    }) = session.fault()
    else {
        panic!("expected ChipLinkUnsupported, got {:?}", session.fault());
    };
    assert_eq!(pin.to_string(), "#u1.pin.GPIO13");
    assert_eq!(other.to_string(), "#rmiso.pin.a");
    assert_eq!(*peripheral, "SPI2");
    assert_eq!(*kind, LinkKind::Spi);
}
