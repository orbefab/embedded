//! The first-target netlist runs in the engine with the button pressed from 50 ms to 150 ms,
//! and lights the LED to the values measured with the Nano's 5V pin one USB diode drop under 5 V.

use orbe_model::Time;

use orbe_engine::netlist::Contact;

mod common;

use orbe_engine::blob::Blobs;
use orbe_engine::name::{Name, Ref};

use orbe_engine::{Input, Session, Setting, Status};
use orbe_scene::Files;

/// The values while pressed, with the Nano's 5V pin at 4.75 V behind its USB diode, to the
/// digits recorded then.
const D13_PRESSED_V: f64 = 4.4007135;
const LED1_PRESSED_A: f64 = 0.0112925044;

fn ms(ms: u64) -> Time {
    Time::from_ps(ms * 1_000_000_000)
}

fn read(session: &Session, r: &str) -> f64 {
    session
        .probe()
        .get(&r.parse::<Ref>().unwrap())
        .unwrap_or_else(|| panic!("no column {r}"))
        .get()
}

#[test]
fn pressing_the_button_lights_led1() {
    let project = common::project("nano-button");
    let netlist = orbe_scene::read(&Files::disk(), &project.path().join("scene.jsonc")).unwrap();
    let mut session = Session::open_json(netlist.json.as_bytes(), &Blobs::new()).unwrap();
    for (at, contact) in [(50, Contact::Closed), (150, Contact::Open)] {
        let input = Input {
            part: Name::new("btn1").unwrap(),
            at: ms(at),
            setting: Setting::Contact(contact),
        };
        session.apply(&input).unwrap();
    }

    assert_eq!(session.advance_to(ms(40)), Status::Running);
    let (d13_open, led1_open) = (read(&session, "#net.nano__D13"), read(&session, "#led1.i"));
    assert_eq!(session.advance_to(ms(100)), Status::Running);
    let (d13, led1, d2) = (
        read(&session, "#net.nano__D13"),
        read(&session, "#led1.i"),
        read(&session, "#net.nano__D2"),
    );
    assert_eq!(session.advance_to(ms(200)), Status::Running);
    let (d13_released, led1_released) =
        (read(&session, "#net.nano__D13"), read(&session, "#led1.i"));
    let diagnostics: Vec<String> = session
        .diagnostics()
        .iter()
        .map(|d| serde_json::to_string(d).unwrap())
        .collect();

    println!("40 ms: D13 {d13_open} V, led1 {led1_open} A");
    println!("100 ms: D13 {d13} V, D2 {d2} V, led1 {led1} A");
    println!("200 ms: D13 {d13_released} V, led1 {led1_released} A");
    println!("diagnostics: {diagnostics:#?}");

    assert!(
        session.fault().is_none(),
        "the run faulted: {:?}",
        session.fault()
    );
    assert!(
        (d13 - D13_PRESSED_V).abs() < 5e-8,
        "D13 while pressed is {d13} V"
    );
    assert!(
        (led1 - LED1_PRESSED_A).abs() < 5e-11,
        "led1 while pressed is {led1} A"
    );
    assert!(
        d13_open < 1e-3 && led1_open < 1e-9,
        "the LED is lit before the press"
    );
    assert!(
        d13_released < 1e-3 && led1_released < 1e-9,
        "the LED is lit after the release"
    );
    let errors: Vec<&String> = diagnostics
        .iter()
        .filter(|d| d.contains("\"severity\":\"error\""))
        .collect();
    assert!(errors.is_empty(), "errors: {errors:?}");
}

/// The reader writes every finite whole number as a JSON integer. Each engine example netlist
/// that reads as written still reads with its whole numbers made integers, so every float field
/// those examples cover takes an integer.
#[test]
fn the_engine_reads_whole_numbers_as_integers() {
    fn integers(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(n) => {
                if let Some(f) = n
                    .as_f64()
                    .filter(|f| n.is_f64() && f.fract() == 0.0 && f.abs() < 9e15)
                {
                    *value = serde_json::json!(f as i64);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(integers),
            serde_json::Value::Object(map) => map.values_mut().for_each(integers),
            _ => {}
        }
    }
    let examples = common::repo().join("packages/engine/tests/circuits");
    let mut kinds = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(examples).unwrap() {
        let path = entry.unwrap().path().join("netlist.json");
        let Ok(text) = std::fs::read(&path) else {
            continue;
        };
        if orbe_engine::netlist::parse(&text).is_err() {
            continue;
        }
        let mut json: serde_json::Value = serde_json::from_slice(&text).unwrap();
        integers(&mut json);
        let changed = serde_json::to_vec(&json).unwrap();
        if let Err(found) = orbe_engine::netlist::parse(&changed) {
            panic!("{}: {found:?}", path.display());
        }
        for part in json["parts"].as_array().unwrap() {
            kinds.insert(part["kind"].as_str().unwrap().to_string());
        }
    }
    for kind in [
        "resistor",
        "capacitor",
        "led",
        "switch",
        "atmega328p",
        "voltage_source",
    ] {
        assert!(kinds.contains(kind), "no example covers {kind}: {kinds:?}");
    }
    println!("kinds read with integers: {kinds:?}");
}
