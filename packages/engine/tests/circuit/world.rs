use orbe_model::Time;

use orbe_engine::finite::Finite;
use orbe_engine::name::Ref;
use orbe_engine::netlist;
use orbe_engine::run_folder::render;
use orbe_engine::timestamp::Timestamp;
use orbe_engine::world::Quantity;
use orbe_engine::{EdgeCursor, Rejected, SerialCursor, Session, Status, WorldChange};

use crate::common::{column, simulate, simulate_steps, trace};

const VOLTS: f64 = 5.0;
const TOP_OHM: f64 = 10_000.0;

/// `10000 * (10 / light)` ohm, at most 1 Mohm: 333.33 ohm at 300 lx, 2 kohm at 50 lx.
fn ldr(name: &str, a: &str, b: &str) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "resistor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "resistance": {{ "kind": "world", "quantity": "light", "curve": {{ "kind": "power",
            "x": {{ "value": 10.0, "unit": "lx" }},
            "y": {{ "value": 10000.0, "unit": "ohm" }},
            "exponent": {{ "value": -1.0, "unit": "1" }},
            "max": {{ "value": 1000000.0, "unit": "ohm" }} }} }},
          "ratings": {{ "power": null }} }}"#
    )
}

fn lux(value: f64) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "lx" }}"#)
}

fn deg(value: f64) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "degC" }}"#)
}

fn steps(initial: &str, events: &[(u64, &str)]) -> String {
    let events = events
        .iter()
        .map(|(t_ps, level)| format!(r#"{{ "t_ps": {t_ps}, "level": {level} }}"#))
        .collect::<Vec<_>>()
        .join(", ");
    format!(r#"{{ "kind": "steps", "initial": {initial}, "events": [{events}] }}"#)
}

/// A 5 V source over 10 kohm on top and the LDR below, `mid` between them.
fn divider(light: &str, stop_ps: u64, max_step_ps: u64) -> String {
    divider_in(&format!(r#""light": {light}"#), stop_ps, max_step_ps)
}

fn divider_in(world: &str, stop_ps: u64, max_step_ps: u64) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps}, "grid_ps": {max_step_ps} }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "mid"],
      "world": {{ {world} }},
      "parts": [
        {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
           "voltage": {{ "value": {VOLTS:?}, "unit": "V" }} }},
        {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "vcc", "b": "mid" }},
           "resistance": {{ "value": {TOP_OHM:?}, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
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

/// Light steps at 1 ms, 5 ms and 10 ms, and temperature steps at 5 ms. A `set` at 500 µs
/// lands there, drops only light's later steps, and the divider follows at once.
#[test]
fn a_set_drops_later_steps_and_leaves_the_other_quantity() {
    let (lx300, lx100, lx50, lx10) = (lux(300.0), lux(100.0), lux(50.0), lux(10.0));
    let light_steps = steps(
        &lx300,
        &[
            (1_000_000_000, lx100.as_str()),
            (5_000_000_000, lx50.as_str()),
            (10_000_000_000, lx10.as_str()),
        ],
    );
    let (deg25, deg40) = (deg(25.0), deg(40.0));
    let temperature = steps(&deg25, &[(5_000_000_000, deg40.as_str())]);
    let json = divider_in(
        &format!(r#""light": {light_steps}, "temperature": {temperature}"#),
        20_000_000_000,
        100_000_000,
    );
    let net: Ref = "#net.mid".parse().unwrap();
    let world: Ref = "#world.light".parse().unwrap();
    let room: Ref = "#world.temperature".parse().unwrap();
    let asked = Time::from_ps(500_000_000);

    let mut session = Session::open(netlist::parse(json.as_bytes()).unwrap()).unwrap();
    session.advance_to(asked);
    assert_eq!(
        session.set(&light(100_001.0)),
        Err(Rejected::Value(orbe_engine::diagnostic::ValueRule::Range)),
        "a refused set drops nothing"
    );
    assert_eq!(event_times(&session, Quantity::Light).len(), 3);

    let applied = session.set(&light(1000.0)).unwrap();
    assert_eq!(applied.status, Status::Running);
    assert_eq!(applied.dropped, 3);
    assert_eq!(applied.at, asked, "the set lands at the session's time");
    assert_eq!(session.now(), asked);
    assert_eq!(
        session.next_at(),
        Some(Time::from_ps(asked.as_ps() + 1_000)),
        "the new edge is taken, and the ramp after it is the next step"
    );
    assert_eq!(event_times(&session, Quantity::Light), vec![asked.as_ps()]);
    assert_eq!(
        event_times(&session, Quantity::Temperature),
        vec![5_000_000_000]
    );

    session.advance_to(Time::from_ps(asked.as_ps() + 1_000));
    let probe = session.probe();
    assert_eq!(probe.get(&world).unwrap().get(), 1000.0);
    assert!(
        close(probe.get(&net).unwrap().get(), mid(100.0)),
        "mid {} at 1000 lx",
        probe.get(&net).unwrap().get()
    );
    assert_eq!(probe.get(&room).unwrap().get(), 25.0);

    session.advance_to(Time::from_ps(12_000_000_000));
    let probe = session.probe();
    assert_eq!(probe.get(&world).unwrap().get(), 1000.0);
    assert_eq!(probe.get(&room).unwrap().get(), 40.0);
    assert!(
        close(probe.get(&net).unwrap().get(), mid(100.0)),
        "a later light step must not return: {}",
        probe.get(&net).unwrap().get()
    );

    let mut session = Session::open(netlist::parse(json.as_bytes()).unwrap()).unwrap();
    session.advance_to(asked);
    let applied = session.set(&light(300.0)).unwrap();
    assert_eq!(applied.dropped, 3);
    assert!(applied.already);
    assert_eq!(applied.at, asked);
    assert!(
        session
            .world()
            .schedule(Quantity::Light)
            .unwrap()
            .events()
            .is_empty(),
        "the value already in force drops the later steps and appends none"
    );
    assert_eq!(
        event_times(&session, Quantity::Temperature),
        vec![5_000_000_000]
    );
    session.advance_to(Time::from_ps(12_000_000_000));
    let probe = session.probe();
    assert_eq!(probe.get(&world).unwrap().get(), 300.0);
    assert_eq!(probe.get(&room).unwrap().get(), 40.0);
    assert!(close(
        probe.get(&net).unwrap().get(),
        mid(10_000.0 * 10.0 / 300.0)
    ));
}

/// A second `set` of a value that is pending and not yet in force changes nothing.
#[test]
fn a_repeated_set_of_a_pending_value_is_no_change() {
    let json = divider(&lux(300.0), 10_000_000_000, 100_000_000);
    let mut session = Session::open(netlist::parse(json.as_bytes()).unwrap()).unwrap();
    let first = session.set(&light(1000.0)).unwrap();
    assert!(!first.already);
    assert_eq!(session.set(&light(1000.0)), Err(Rejected::NoChange));
    assert_eq!(
        event_times(&session, Quantity::Light),
        vec![first.at.as_ps()]
    );
}

fn event_times(session: &Session, quantity: Quantity) -> Vec<u64> {
    session
        .world()
        .schedule(quantity)
        .unwrap()
        .events()
        .iter()
        .map(|event| event.at.as_ps())
        .collect()
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
/// ahead reads what a run of the netlist with that change scheduled holds: its serial bytes,
/// its logic changes, and its last row, bit for bit.
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
          "resistance": {{ "value": {TOP_OHM:?}, "unit": "ohm" }}, "ratings": {{ "power": null }} }}"#
    ))
    .unwrap();
    parts.push(serde_json::from_str(&ldr("rl", "pc0", "gnd")).unwrap());
    doc["world"] = serde_json::json!({ "light": serde_json::from_str::<serde_json::Value>(&lux(300.0)).unwrap() });
    let live = serde_json::to_vec(&doc).unwrap();

    let mut session = Session::open(netlist::parse(&live).unwrap()).unwrap();
    let asked = Time::from_ps(6_000_000_000);
    session.advance_to(asked);
    assert!(
        session.next_at().unwrap() > asked,
        "the session holds a step past its time"
    );
    let applied = session.set(&light(50.0)).unwrap();
    assert!(applied.at >= asked);
    assert_eq!(session.advance_to(Time::from_ps(u64::MAX)), Status::Stopped);

    doc["world"]["light"] = serde_json::json!({
        "kind": "steps",
        "initial": serde_json::from_str::<serde_json::Value>(&lux(300.0)).unwrap(),
        "events": [{ "t_ps": applied.at.as_ps(), "level": serde_json::from_str::<serde_json::Value>(&lux(50.0)).unwrap() }],
    });
    let scheduled = serde_json::to_vec(&doc).unwrap();
    let netlist = netlist::parse(&scheduled).unwrap();
    let results = orbe_engine::simulate(&netlist);
    let batch = trace(&results);
    let last = batch.time().len() - 1;
    let probe = session.probe();
    assert_eq!(probe.at(), batch.time()[last]);
    for (j, column) in batch.columns().iter().enumerate() {
        assert_eq!(
            probe.get(column).unwrap().get().to_bits(),
            batch.column(j)[last].to_bits(),
            "{column}"
        );
    }
    let (edges, _) = session.edges_since(EdgeCursor::default()).unwrap();
    assert_eq!(edges, batch.logic().unwrap().changes());
    let folder = render(
        &scheduled,
        Some(&netlist),
        &results,
        Timestamp::from_unix_seconds(0),
    );
    let csv = String::from_utf8(folder.file("traces/serial.csv").unwrap().to_vec()).unwrap();
    let usarts = session.usarts();
    let (bytes, _) = session.serial_since(SerialCursor::default()).unwrap();
    let read: Vec<String> = bytes
        .iter()
        .map(|byte| {
            let (part, usart) = &usarts[usize::from(byte.usart)];
            format!(
                "{},{},{usart},{}",
                byte.at.as_ps(),
                part.as_str(),
                byte.byte
            )
        })
        .collect();
    assert_eq!(read, csv.lines().skip(1).collect::<Vec<_>>());
    let text: String = csv
        .lines()
        .skip(1)
        .map(|line| char::from(line.rsplit(',').next().unwrap().parse::<u8>().unwrap()))
        .collect();
    // floor(1024 * R / (10 kohm + R)): 33 at 300 lx, where R is 333 ohm, and 170 at 50 lx.
    assert!(text.starts_with("33\r\n33\r\n"), "{text:?}");
    assert!(text.contains("\n170\r\n"), "{text:?}");
}
