//! An SG90 on a stiff 5 V rail follows a 3.3 V signal and ignores one under half the supply.
//! On a 3.3 V rail limited to 30 mA the moving draw pulls the rail down, the shaft stops, and the rail recovers.

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
    // The moving draw collapses the rail within a nanosecond. The servo stops, the draw
    // falls, and by 25 ms the rail is back. The shaft never leaves centre.
    assert_eq!(session.advance_to(Time::from_ps(25 * MS)), Status::Running);
    let sag = session.probe().get(&rail).unwrap().get();
    assert!(
        sag > 3.0,
        "the servo was still pulling the rail down: {sag}"
    );
    assert_eq!(session.advance_to(Time::from_ps(STOP)), Status::Stopped);
    let horn = angle(&session);
    assert!(
        (horn - 90.0).abs() < 1.0,
        "the shaft turned on the 30 mA rail: {horn}"
    );
}

#[test]
fn a_series_resistor_sags_and_the_shaft_moves_slower_and_warns() {
    let stiff = {
        let mut session = open_supply(&stiff(5.0), 200 * MS, MS);
        assert_eq!(session.advance_to(Time::from_ps(120 * MS)), Status::Running);
        angle(&session)
    };
    let supply = r#"{ "name": "v5", "kind": "voltage_source",
             "pins": { "p": "v5", "n": "gnd" },
             "voltage": { "value": 5.0, "unit": "V" } },
           { "name": "rs", "kind": "resistor",
              "pins": { "a": "v5", "b": "vservo" },
              "resistance": { "value": 10.0, "unit": "ohm" },
              "ratings": { "power": null } }"#;
    let json = format!(
        r#"{{ "version": 1, "seed": 0,
             "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {MS}, "grid_ps": {MS} }} }},
             "ground": "gnd", "nets": ["gnd", "v5", "vservo", "sig"],
             "parts": [ {supply},
               {{ "name": "vsig", "kind": "voltage_source",
                  "pins": {{ "p": "sig", "n": "gnd" }}, "voltage": {pulse} }},
               {servo} ] }}"#,
        stop = 200 * MS,
        pulse = pulse(5.0),
        servo = servo()
    );
    let mut session = Session::open_json(json.as_bytes(), &Blobs::new()).unwrap_or_else(|err| {
        panic!("the netlist did not open: {err:?}\n{json}");
    });
    assert_eq!(session.advance_to(Time::from_ps(120 * MS)), Status::Running);
    let horn = angle(&session);
    assert!(horn > 90.0, "the sagged shaft did not move: {horn}");
    assert!(
        horn < stiff,
        "the sag did not slow the shaft: {horn} vs {stiff}"
    );
    let warnings = format!("{:?}", session.diagnostics());
    assert!(
        warnings.contains("Supply"),
        "a sagged rail did not warn: {warnings}"
    );
}

fn steps_supply(initial: f64, events: &[(u64, f64)]) -> String {
    let events: String = events
        .iter()
        .map(|(t, volts)| {
            format!(r#"{{ "t_ps": {t}, "level": {{ "value": {volts}, "unit": "V" }} }}"#)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"{{ "name": "vservo", "kind": "voltage_source",
             "pins": {{ "p": "vservo", "n": "gnd" }},
             "voltage": {{ "kind": "steps", "initial": {{ "value": {initial}, "unit": "V" }},
                           "events": [{events}] }} }}"#
    )
}

fn open_supply(supply: &str, stop: u64, step: u64) -> Session {
    let json = format!(
        r#"{{ "version": 1, "seed": 0,
             "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {step}, "grid_ps": {MS} }} }},
             "ground": "gnd", "nets": ["gnd", "vservo", "sig"],
             "parts": [ {supply},
               {{ "name": "vsig", "kind": "voltage_source",
                  "pins": {{ "p": "sig", "n": "gnd" }}, "voltage": {pulse} }},
               {servo} ] }}"#,
        pulse = pulse(5.0),
        servo = servo()
    );
    Session::open_json(json.as_bytes(), &Blobs::new()).unwrap_or_else(|err| {
        panic!("the netlist did not open: {err:?}\n{json}");
    })
}

#[test]
fn the_angle_moves_on_every_millisecond_of_a_slew() {
    let mut session = open_supply(&stiff(5.0), 80 * MS, 10 * MS);
    let mut previous = None;
    let mut stuck = 0u32;
    let mut jump = 0.0f64;
    for ms in 40..=60 {
        assert_eq!(session.advance_to(Time::from_ps(ms * MS)), Status::Running);
        let horn = angle(&session);
        if let Some(before) = previous {
            let step = horn - before;
            jump = jump.max(step);
            if step < 0.05 {
                stuck += 1;
            }
        }
        previous = Some(horn);
    }
    assert_eq!(
        stuck, 0,
        "the angle sat still across a 1 ms read, last {previous:?}"
    );
    assert!(jump < 1.0, "a 1 ms read jumped {jump}°, last {previous:?}");
}

#[test]
fn a_rail_that_starts_at_zero_and_steps_to_four_volts_moves() {
    let supply = steps_supply(0.0, &[(50 * MS, 4.0)]);
    let mut session = open_supply(&supply, 1_000 * MS, MS);
    assert_eq!(session.advance_to(Time::from_ps(900 * MS)), Status::Running);
    let horn = angle(&session);
    assert!(
        horn > 120.0,
        "4 V after a start at 0 V left the shaft at {horn}"
    );
}

#[test]
fn a_five_millisecond_dip_slows_the_shaft_off_the_sample_grid() {
    let undipped = {
        let mut session = open_supply(&stiff(5.0), 200 * MS, 10 * MS);
        assert_eq!(session.advance_to(Time::from_ps(150 * MS)), Status::Running);
        angle(&session)
    };
    let mut dipped = Vec::new();
    for start in [25 * MS, 28 * MS] {
        let supply = steps_supply(5.0, &[(start, 2.0), (start + 5 * MS, 5.0)]);
        let mut session = open_supply(&supply, 200 * MS, 10 * MS);
        assert_eq!(session.advance_to(Time::from_ps(150 * MS)), Status::Running);
        dipped.push(angle(&session));
    }
    let lost: Vec<f64> = dipped.iter().map(|horn| undipped - horn).collect();
    assert!(
        lost.iter().all(|lost| *lost > 1.0),
        "a 5 ms dip to 2 V did not hold the shaft: undipped {undipped}, lost {lost:?}"
    );
    assert!(
        (lost[0] - lost[1]).abs() < 2.5,
        "the dip's effect depends on where it sits: lost {lost:?}"
    );
}
