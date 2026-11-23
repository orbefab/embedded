//! An SG90 on a stiff 5 V rail follows a 3.3 V signal and ignores one under half the supply.
//! On a 3.3 V rail limited to 30 mA, the same pulses collapse the rail and the shaft stays put.

use orbe_engine::blob::Blobs;
use orbe_engine::name::{Name, Ref};
use orbe_engine::{Session, Status};
use orbe_model::Time;

const MS: u64 = 1_000_000_000;
const STOP: u64 = 400 * MS;

fn pulse(high: f64) -> String {
    format!(
        r#"{{ "kind": "pulse", "initial": {{ "value": 0.0, "unit": "V" }},
             "pulsed": {{ "value": {high}, "unit": "V" }},
             "delay_ps": {delay}, "width_ps": 2400000000, "period_ps": 20000000000 }}"#,
        delay = 20 * MS
    )
}

fn servo() -> &'static str {
    r#"{ "name": "servo", "kind": "sg90",
        "pins": { "VCC": "vservo", "GND": "gnd", "SIGNAL": "sig" },
        "input_threshold": {
          "low": { "value": 0.3, "unit": "1" },
          "high": { "value": 0.5, "unit": "1" }
        } }"#
}

fn stiff(volts: f64) -> String {
    format!(
        r#"{{ "name": "vservo", "kind": "voltage_source",
             "pins": {{ "p": "vservo", "n": "gnd" }},
             "voltage": {{ "value": {volts}, "unit": "V" }} }}"#
    )
}

/// The CH340 V3 stand-in: 3.3 V, 30 mA, fed from 5 V. A moving SG90 pulls it down.
fn collapsed() -> &'static str {
    r#"{ "name": "v5", "kind": "voltage_source",
        "pins": { "p": "v5", "n": "gnd" },
        "voltage": { "value": 5.0, "unit": "V" } },
      { "name": "rail", "kind": "linear_regulator",
        "pins": { "in": "v5", "out": "vservo", "gnd": "gnd" },
        "output_voltage": { "value": 3.3, "unit": "V" },
        "line_regulation": { "value": 0.0, "unit": "1" },
        "output_resistance": { "value": 1.0, "unit": "ohm" },
        "dropout_voltage": { "value": 0.7, "unit": "V" },
        "dropout_resistance": { "value": 1.0, "unit": "ohm" },
        "current_limit": { "value": 0.03, "unit": "A" },
        "quiescent_current": { "value": 0.0, "unit": "A" },
        "ground_current_ratio": { "value": 0.0, "unit": "1" },
        "ratings": { "current": null, "power": null } }"#
}

fn netlist(nets: &str, supply: &str, signal: f64) -> String {
    format!(
        r#"{{ "version": 1, "seed": 0,
             "analysis": {{ "transient": {{ "stop_ps": {STOP}, "max_step_ps": {MS}, "grid_ps": {MS} }} }},
             "ground": "gnd", "nets": [{nets}],
             "parts": [ {supply},
               {{ "name": "vsig", "kind": "voltage_source",
                  "pins": {{ "p": "sig", "n": "gnd" }}, "voltage": {pulse} }},
               {servo} ] }}"#,
        pulse = pulse(signal),
        servo = servo()
    )
}

fn run(supply: &str, signal: f64) -> Session {
    let json = netlist(r#""gnd", "vservo", "sig""#, supply, signal);
    let mut session = Session::open_json(json.as_bytes(), &Blobs::new()).unwrap_or_else(|err| {
        panic!("the netlist did not open: {err:?}\n{json}");
    });
    let status = session.advance_to(Time::from_ps(STOP));
    assert_eq!(status, Status::Stopped, "{:?}", session.diagnostics());
    session
}

fn angle(session: &Session) -> f64 {
    session
        .angle(&Name::new("servo").unwrap())
        .expect("the servo shows an angle")
}

#[test]
fn a_three_point_three_volt_signal_sweeps_the_shaft() {
    let session = run(&stiff(5.0), 3.3);
    let horn = angle(&session);
    assert!(horn > 170.0, "{horn}");
}

#[test]
fn a_signal_under_half_the_supply_does_not_move_the_shaft() {
    let session = run(&stiff(5.0), 2.4);
    let horn = angle(&session);
    assert!((horn - 90.0).abs() < 0.1, "{horn}");
}

#[test]
fn a_thirty_milliamp_rail_collapses_and_the_shaft_stays() {
    let json = netlist(r#""gnd", "vservo", "v5", "sig""#, collapsed(), 5.0);
    let mut session = Session::open_json(json.as_bytes(), &Blobs::new()).unwrap_or_else(|err| {
        panic!("the netlist did not open: {err:?}\n{json}");
    });
    let rail: Ref = "#net.vservo".parse().unwrap();
    assert_eq!(session.advance_to(Time::from_ps(25 * MS)), Status::Running);
    let sag = session.probe().get(&rail).unwrap().get();
    assert!(sag < 2.0, "the rail did not collapse: {sag}");
    assert_eq!(session.advance_to(Time::from_ps(STOP)), Status::Stopped);
    let horn = angle(&session);
    assert!(
        (horn - 90.0).abs() < 10.0,
        "the shaft kept turning on the collapsed rail: {horn} at {sag} V"
    );
}
