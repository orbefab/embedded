//! An `atmega328p`'s `ratings` on the documented example of `docs/engine/netlist.md`: each port
//! pin's current, the sums through `VCC` and `GND_8`, and each pin's voltage window.

use std::fs;

use crate::runs::common::repo;
use orbe_engine::Session;
use orbe_engine::diagnostic::{Diagnostic, Excursion, VoltageExcursion};
use orbe_engine::netlist;
use orbe_engine::timestamp::Timestamp;
use orbe_model::Time;

/// The documented `leakage_resistance`, to each rail.
const LEAKAGE: f64 = 5.5e6;
/// The documented `output.*_resistance`.
const STAGE: f64 = 25.0;

fn section() -> String {
    let doc = fs::read_to_string(repo("docs/engine/netlist.md")).unwrap();
    let start = doc.find("### ATmega328P").unwrap();
    let end = start + doc[start..].find("\n### SPI replay").unwrap();
    doc[start..end].to_string()
}

/// The section's code block that starts with `start`.
fn block(start: &str) -> String {
    let section = section();
    let from = section.find(&format!("```json\n{start}")).unwrap() + "```json\n".len();
    let to = from + section[from..].find("\n```").unwrap();
    section[from..to].to_string()
}

/// The documented example, its chip rated with the documented `ratings` object.
fn rated() -> serde_json::Value {
    let mut netlist: serde_json::Value = serde_json::from_str(&block("{")).unwrap();
    let ratings: serde_json::Value =
        serde_json::from_str(&format!("{{{}}}", block("\"ratings\""))).unwrap();
    chip(&mut netlist)["ratings"] = ratings["ratings"].clone();
    netlist
}

fn chip(netlist: &mut serde_json::Value) -> &mut serde_json::Value {
    part(netlist, "u1")
}

fn part<'n>(netlist: &'n mut serde_json::Value, name: &str) -> &'n mut serde_json::Value {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == name)
        .unwrap()
}

fn add(netlist: &mut serde_json::Value, nets: &[&str], parts: serde_json::Value) {
    for net in nets {
        netlist["nets"].as_array_mut().unwrap().push((*net).into());
    }
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .extend(parts.as_array().unwrap().iter().cloned());
}

fn source(name: &str, p: &str, n: &str, volts: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "voltage_source", "pins": { "p": p, "n": n },
        "voltage": { "value": volts, "unit": "V", "source": "user" }
    })
}

fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "resistor", "pins": { "a": a, "b": b },
        "resistance": { "value": ohm, "unit": "ohm", "source": "user" },
        "ratings": { "power": null }
    })
}

fn run(netlist: &serde_json::Value) -> Vec<Diagnostic> {
    orbe_engine::run(
        netlist.to_string().as_bytes(),
        Timestamp::from_unix_seconds(0),
    )
    .results
    .diagnostics()
    .to_vec()
}

fn currents(found: &[Diagnostic]) -> Vec<&Excursion> {
    found
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingCurrent(excursion) => Some(excursion),
            _ => None,
        })
        .collect()
}

fn windows(found: &[Diagnostic]) -> Vec<&VoltageExcursion> {
    found
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingVoltage(excursion) => Some(excursion),
            _ => None,
        })
        .collect()
}

fn refs(found: &[Diagnostic]) -> Vec<String> {
    found
        .iter()
        .map(|d| format!("{} {}", d.code(), d.refs()[0]))
        .collect()
}

/// PB5 high into `load` to ground: the stage and the two leakages against the load.
fn high_into(load: f64) -> f64 {
    let up = 1.0 / STAGE + 1.0 / LEAKAGE;
    let down = 1.0 / LEAKAGE + 1.0 / load;
    5.0 * up / (up + down) / load
}

#[test]
fn the_documented_ratings_are_the_part_files() {
    let ratings = block("\"ratings\"");
    for part in ["atmega328p-dip28", "atmega328p-tqfp32"] {
        let file = fs::read_to_string(repo(&format!("parts/{part}/part.jsonc"))).unwrap();
        for line in ratings
            .lines()
            .skip(1)
            .filter(|line| line.contains("\"value\""))
        {
            let line = line.trim().trim_end_matches(',');
            assert!(file.contains(line), "{part} lacks {line}");
        }
    }
}

#[test]
fn a_chip_with_no_ratings_object_is_unrated() {
    let mut netlist = rated();
    chip(&mut netlist)
        .as_object_mut()
        .unwrap()
        .remove("ratings");
    part(&mut netlist, "r1")["resistance"]["value"] = 47.0.into();
    assert_eq!(run(&netlist), []);
}

#[test]
fn the_documented_example_is_inside_every_rating() {
    assert_eq!(run(&rated()), []);
}

/// `toggle.S` drives PB5 high from 250 ns to 6.5 us and from 12.75 us to 19 us of the 20 us run.
/// Into 47 ohm that is 69 mA, the stage's closed form. Each sample holds until the next, so the
/// time over is those 12.5 us, give or take a step at each edge.
#[test]
fn a_port_pin_over_its_current_is_named_with_the_stage_s_current() {
    let mut netlist = rated();
    part(&mut netlist, "r1")["resistance"]["value"] = 47.0.into();
    let found = run(&netlist);
    assert_eq!(refs(&found), ["rating.current #u1.pin.PB5"]);
    let [pin] = currents(&found)[..] else {
        unreachable!()
    };
    assert_eq!(pin.limit, 0.04);
    assert!(
        (pin.peak - high_into(47.0)).abs() < 1e-9,
        "{} against {}",
        pin.peak,
        high_into(47.0)
    );
    // The first step after the rise at cycle 4, 250 ns.
    assert!(pin.first > Time::from_ps(250_000) && pin.first < Time::from_ps(1_250_000));
    let over = pin.over_ps.unwrap();
    assert!(over.abs_diff(12_500_000) < 2_000_000, "{over}");
}

/// PB5 driving low on the supply's net sinks 5 V across 25 ohm, the 200 mA rating of the
/// ground pin, and the leakages of the other pins put `GND_8` just over it.
#[test]
fn a_pin_driving_low_on_the_supply_is_over_and_so_is_its_ground() {
    let mut netlist = rated();
    chip(&mut netlist)["pins"]["PB5"] = "vcc".into();
    part(&mut netlist, "r1")["pins"]["a"] = "vcc".into();
    netlist["nets"]
        .as_array_mut()
        .unwrap()
        .retain(|net| net != "pb5");
    let found = run(&netlist);
    assert_eq!(
        refs(&found),
        ["rating.current #u1.pin.GND_8", "rating.current #u1.pin.PB5"]
    );
    let [ground, pin] = currents(&found)[..] else {
        unreachable!()
    };
    assert!(
        (pin.peak - (5.0 / STAGE + 5.0 / LEAKAGE)).abs() < 1e-9,
        "{}",
        pin.peak
    );
    assert!(ground.peak > 0.2 && ground.peak < 0.2 + 21.0 * 5.0 / LEAKAGE);
}

/// `ports.S` drives every port pin high. Six of them into 120 ohm each carry 34.5 mA, inside
/// the pin rating. With PB5's 4.9 mA into the example's 1 kohm, `VCC` carries 212 mA, plus at
/// most 5 V across the leakage of each of the other pins.
#[test]
fn six_pins_inside_their_rating_put_vcc_over_its_own() {
    let mut netlist = rated();
    chip(&mut netlist)["firmware"] =
        fs::read_to_string(repo("packages/engine/tests/firmware/ports.hex"))
            .unwrap()
            .into();
    let loads: Vec<_> = (2..8)
        .map(|k| resistor(&format!("rd{k}"), &format!("pd{k}"), "gnd", 120.0))
        .collect();
    add(&mut netlist, &[], serde_json::Value::Array(loads));
    let found = run(&netlist);
    assert_eq!(refs(&found), ["rating.current #u1.pin.VCC"]);
    let [vcc] = currents(&found)[..] else {
        unreachable!()
    };
    let sum = 6.0 * high_into(120.0) + high_into(1000.0);
    assert!(
        vcc.peak > sum && vcc.peak < sum + 23.0 * 5.0 / LEAKAGE,
        "{} against {sum}",
        vcc.peak
    );
    assert!(high_into(120.0) < 0.04);
}

/// 9 V through 1 kohm into PD2, and -9 V into PD3: an off pin holds what the circuit drives,
/// with no clamp, so each is outside -0.5 V to VCC + 0.5 V from the operating point on. The
/// closed form leaves out the solver's gmin, about 9 nV here.
#[test]
fn a_pin_past_either_rail_is_outside_its_window() {
    let mut netlist = rated();
    netlist["analysis"] = serde_json::json!({ "op": {} });
    add(
        &mut netlist,
        &["v9", "vm9"],
        serde_json::json!([
            source("vhi", "v9", "gnd", 9.0),
            resistor("rhi", "v9", "pd2", 1000.0),
            source("vlo", "vm9", "gnd", -9.0),
            resistor("rlo", "vm9", "pd3", 1000.0),
        ]),
    );
    let found = run(&netlist);
    assert_eq!(
        refs(&found),
        ["rating.voltage #u1.pin.PD2", "rating.voltage #u1.pin.PD3"]
    );
    let [high, low] = windows(&found)[..] else {
        unreachable!()
    };
    let held = |volts: f64| (volts / 1000.0 + 5.0 / LEAKAGE) / (1.0 / 1000.0 + 2.0 / LEAKAGE);
    for (window, volts) in [(high, 9.0), (low, -9.0)] {
        assert_eq!((window.low, window.high), (Some(-0.5), Some(5.5)));
        assert!((window.peak - held(volts)).abs() < 1e-7, "{}", window.peak);
        assert_eq!((window.first, window.over_ps), (Time::ZERO, None));
    }
    let message = serde_json::to_value(&found[0]).unwrap()["message"].clone();
    assert!(
        message
            .as_str()
            .unwrap()
            .ends_with("V over the chip's ground, outside its -0.5 V to 5.5 V rating"),
        "{message}"
    );
}

/// `RESET` may reach 13 V and `VCC` 6 V. Every other pin's ceiling follows `VCC`, so a 6.5 V
/// supply puts only `VCC` outside.
#[test]
fn reset_and_vcc_take_their_own_ceilings() {
    let at_reset = |volts: f64| {
        let mut netlist = rated();
        netlist["analysis"] = serde_json::json!({ "op": {} });
        add(
            &mut netlist,
            &[],
            serde_json::json!([source("vr", "pc6", "gnd", volts)]),
        );
        refs(&run(&netlist))
    };
    assert_eq!(at_reset(12.0), Vec::<String>::new());
    assert_eq!(at_reset(14.0), ["rating.voltage #u1.pin.PC6"]);

    let mut netlist = rated();
    netlist["analysis"] = serde_json::json!({ "op": {} });
    part(&mut netlist, "v1")["voltage"]["value"] = 6.5.into();
    let found = run(&netlist);
    assert_eq!(refs(&found), ["rating.voltage #u1.pin.VCC"]);
    assert_eq!(windows(&found)[0].high, Some(6.0));
}

/// Between accepted steps a session checks the chip's pins at the probe, with the drive in
/// force there. PB5 is high from 250 ns to 6.5 us and low from 6.5 us to 12.75 us.
#[test]
fn a_session_checks_the_pins_at_its_probe() {
    let mut netlist = rated();
    part(&mut netlist, "r1")["resistance"]["value"] = 47.0.into();
    let mut session =
        Session::open(netlist::parse(netlist.to_string().as_bytes()).unwrap()).unwrap();
    let over_at = |session: &mut Session, ps: u64| {
        session.advance_to(Time::from_ps(ps));
        let probe = session.probe();
        assert_eq!(probe.at(), Time::from_ps(ps));
        refs(&session.over_ratings(&probe))
    };
    assert_eq!(
        over_at(&mut session, 3_333_333),
        ["rating.current #u1.pin.PB5"]
    );
    assert_eq!(over_at(&mut session, 9_333_333), Vec::<String>::new());
}
