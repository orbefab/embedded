use orbe_model::Time;

use orbe_engine::finite::Finite;
use orbe_engine::name::Ref;
use orbe_engine::netlist;
use orbe_engine::run_folder::{RunFolder, render};
use orbe_engine::timestamp::Timestamp;
use orbe_engine::world::Quantity;
use orbe_engine::{Rejected, Session, Status, WorldChange};

use crate::common::{column, simulate, simulate_steps, trace};

const VOLTS: f64 = 5.0;
const TOP_OHM: f64 = 10_000.0;

/// `10000 * (10 / light)` ohm, at most 1 Mohm: 333.33 ohm at 300 lx, 2 kohm at 50 lx.
fn ldr(name: &str, a: &str, b: &str) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "resistor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "resistance": {{ "kind": "world", "quantity": "light", "curve": {{ "kind": "power",
            "x": {{ "value": 10.0, "unit": "lx", "source": "user" }},
            "y": {{ "value": 10000.0, "unit": "ohm", "source": "user" }},
            "exponent": {{ "value": -1.0, "unit": "1", "source": "user" }},
            "max": {{ "value": 1000000.0, "unit": "ohm", "source": "user" }} }} }},
          "ratings": {{ "power": null }} }}"#
    )
}

fn lux(value: f64) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "lx", "source": "user" }}"#)
}

/// A 5 V source over 10 kohm on top and the LDR below, `mid` between them.
fn divider(light: &str, stop_ps: u64, max_step_ps: u64) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps}, "grid_ps": {max_step_ps} }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "mid"],
      "world": {{ "light": {light} }},
      "parts": [
        {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
           "voltage": {{ "value": {VOLTS:?}, "unit": "V", "source": "user" }} }},
        {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "vcc", "b": "mid" }},
           "resistance": {{ "value": {TOP_OHM:?}, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }},
        {}
      ]
    }}"#,
        ldr("rl", "mid", "gnd")
    )
}

/// The divider's middle with the LDR at `ohm`.
fn mid(ohm: f64) -> f64 {
    VOLTS * ohm / (TOP_OHM + ohm)
}

fn close(found: f64, expected: f64) -> bool {
    (found - expected).abs() <= 1e-8 * expected.abs()
}

fn light(value: f64) -> WorldChange {
    WorldChange {
        quantity: Quantity::Light,
        at: Time::ZERO,
        value: Finite::new(value).unwrap(),
    }
}

#[test]
fn a_divider_reaches_the_curve_after_set_light() {
    let json = divider(&lux(300.0), 10_000_000_000, 100_000_000);
    let mut session = Session::open(netlist::parse(json.as_bytes()).unwrap()).unwrap();
    let net: Ref = "#net.mid".parse().unwrap();
    let world: Ref = "#world.light".parse().unwrap();
    session.advance_to(Time::from_ps(1_000_000_000));
    let probe = session.probe();
    assert!(close(
        probe.get(&net).unwrap().get(),
        mid(10_000.0 * 10.0 / 300.0)
    ));
    assert_eq!(probe.get(&world).unwrap().get(), 300.0);

    let applied = session.set(&light(50.0)).unwrap();
    assert_eq!(applied.at, Time::from_ps(1_000_000_000));
    assert_eq!(applied.status, Status::Running);
    session.advance_to(Time::from_ps(2_000_000_000));
    let probe = session.probe();
    assert!(
        close(probe.get(&net).unwrap().get(), mid(2_000.0)),
        "mid {} at 50 lx",
        probe.get(&net).unwrap().get()
    );
    assert_eq!(probe.get(&world).unwrap().get(), 50.0);

    assert_eq!(session.set(&light(0.0)).unwrap().status, Status::Running);
    session.advance_to(Time::from_ps(3_000_000_000));
    let dark = session.probe().get(&net).unwrap().get();
    assert!(close(dark, mid(1_000_000.0)), "the dark reads max: {dark}");
}

#[test]
fn a_set_the_session_cannot_take_is_refused() {
    let json = divider(&lux(300.0), 10_000_000_000, 100_000_000);
    let mut session = Session::open(netlist::parse(json.as_bytes()).unwrap()).unwrap();
    assert_eq!(
        session.set(&light(100_001.0)),
        Err(Rejected::Value(orbe_engine::diagnostic::ValueRule::Range))
    );
    assert_eq!(session.set(&light(300.0)), Err(Rejected::NoChange));
    let temperature = WorldChange {
        quantity: Quantity::Temperature,
        ..light(30.0)
    };
    assert_eq!(session.set(&temperature), Err(Rejected::Undeclared));
}

#[test]
fn a_scheduled_change_lands_at_its_exact_time_and_restarts() {
    let at = 1_234_567_891;
    let schedule = format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": {at}, "level": {} }}] }}"#,
        lux(300.0),
        lux(50.0)
    );
    let results = simulate_steps(&divider(&schedule, 2_000_000_000, 100_000_000));
    let traced = trace(&results);
    let times: Vec<u64> = traced.time().iter().map(|t| t.as_ps()).collect();
    let k = times
        .iter()
        .position(|&t| t == at)
        .expect("a step lands on the change");
    let after: Vec<u64> = times[k..].windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        after.iter().all(|&gap| gap >= 100),
        "a step under 100 ps follows the change: {:?}",
        &after[..after.len().min(8)]
    );
    let world = column(traced, "#world.light");
    assert_eq!(world[k], 300.0, "the row at the change reads before it");
    assert_eq!(world[k + 1], 50.0);
    let net = column(traced, "#net.mid");
    assert!(close(net[k], mid(10_000.0 * 10.0 / 300.0)));
    assert!(close(net[k + 1], mid(2_000.0)), "{}", net[k + 1]);
}

#[test]
fn a_netlist_without_a_world_writes_no_world_column() {
    let json = crate::common::transient(
        1_000_000,
        1_000_000,
        r#""gnd","vcc""#,
        &[
            &crate::common::source(5.0),
            &crate::common::resistor("r1", "vcc", "gnd", 1000.0),
        ],
    );
    let results = simulate(&json);
    assert!(
        trace(&results)
            .columns()
            .iter()
            .all(|r| !matches!(r, Ref::World(_)))
    );
}

/// The ADC example's pot becomes 10 kohm over the LDR. A `set light` after the session ran
/// ahead writes the run folder a netlist with that change scheduled writes: trace, serial,
/// logic and diagnostics.
#[test]
fn a_set_after_the_session_ran_ahead_matches_the_change_scheduled() {
    let path = format!(
        "{}/tests/circuits/atmega-analog-serial/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut doc: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let parts = doc["parts"].as_array_mut().unwrap();
    let pot = parts.iter().position(|p| p["name"] == "rv1").unwrap();
    parts[pot] = serde_json::from_str(&format!(
        r#"{{ "name": "rv1", "kind": "resistor", "pins": {{ "a": "vcc", "b": "pc0" }},
          "resistance": {{ "value": {TOP_OHM:?}, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }}"#
    ))
    .unwrap();
    parts.push(serde_json::from_str(&ldr("rl", "pc0", "gnd")).unwrap());
    doc["world"] = serde_json::json!({ "light": serde_json::from_str::<serde_json::Value>(&lux(300.0)).unwrap() });
    let live = serde_json::to_vec(&doc).unwrap();

    let mut session = Session::record_document(&live).unwrap();
    let asked = Time::from_ps(6_000_000_000);
    session.advance_to(asked);
    assert!(
        session.next_at().unwrap() > asked,
        "the session holds a step past its time"
    );
    let applied = session.set(&light(50.0)).unwrap();
    assert!(applied.at >= asked);
    let created = Timestamp::from_unix_seconds(1_700_000_000);
    let set = session.finish(created).unwrap();

    doc["world"]["light"] = serde_json::json!({
        "kind": "steps",
        "initial": serde_json::from_str::<serde_json::Value>(&lux(300.0)).unwrap(),
        "events": [{ "t_ps": applied.at.as_ps(), "level": serde_json::from_str::<serde_json::Value>(&lux(50.0)).unwrap() }],
    });
    let scheduled = serde_json::to_vec(&doc).unwrap();
    let netlist = netlist::parse(&scheduled).unwrap();
    let batch = render(
        &scheduled,
        Some(&netlist),
        &orbe_engine::simulate(&netlist),
        created,
    );
    same_files(&set, &batch);

    let csv = String::from_utf8(set.file("traces/serial.csv").unwrap().to_vec()).unwrap();
    let text: String = csv
        .lines()
        .skip(1)
        .map(|line| char::from(line.rsplit(',').next().unwrap().parse::<u8>().unwrap()))
        .collect();
    // floor(1024 * R / (10 kohm + R)): 33 at 300 lx, where R is 333 ohm, and 170 at 50 lx.
    assert!(text.starts_with("33\r\n33\r\n"), "{text:?}");
    assert!(text.contains("\n170\r\n"), "{text:?}");
}

fn same_files(left: &RunFolder, right: &RunFolder) {
    let names: Vec<_> = left.files().map(|(name, _)| name.to_string()).collect();
    let other: Vec<_> = right.files().map(|(name, _)| name.to_string()).collect();
    assert_eq!(names, other);
    for name in names {
        if name == "netlist.json" || name == "manifest.json" {
            continue;
        }
        assert!(left.file(&name) == right.file(&name), "{name} differs");
    }
}
