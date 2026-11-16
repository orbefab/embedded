use orbe_model::Time;

use orbe_engine::netlist::{
    self, Analysis, Appended, Baud, Contact, Document, Frame, Model, SerialSend,
};

use crate::common;

use common::with_parts;
use orbe_engine::finite::Finite;

const ANALYSIS: &str =
    r#"{ "transient": { "stop_ps": 10000000, "max_step_ps": 100000, "grid_ps": 5000000 } }"#;

fn document() -> String {
    with_parts(
        ANALYSIS,
        r#"{
          "name": "sw2",
          "kind": "switch",
          "pins": { "a": "a", "b": "gnd" },
          "on_resistance": { "value": 0.1, "unit": "ohm", "source": "user" },
          "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
          "initial": "open",
          "events": [{ "t_ps": 1000, "state": "closed" }],
          "bounce": { "window": { "value": 8000, "unit": "ps", "source": "user" },
                      "max_bounces": { "value": 2, "unit": "1", "source": "user" } }
        },
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "a", "b": "gnd" },
          "resistance": { "value": 0.30000000000000004, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        },
        {
          "name": "sw1",
          "kind": "switch",
          "pins": { "a": "a", "b": "gnd" },
          "on_resistance": { "value": 1, "unit": "ohm", "source": "user" },
          "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
          "initial": "closed",
          "events": [],
          "bounce": null
        }"#,
    )
}

fn parts(netlist: &netlist::Netlist) -> Vec<netlist::Part> {
    netlist.parts().map(|(_, part)| part.clone()).collect()
}

fn switch_id(netlist: &netlist::Netlist, name: &str) -> netlist::PartId {
    netlist
        .part_id(&orbe_engine::name::Name::new(name).unwrap())
        .unwrap()
}

fn transient(netlist: &netlist::Netlist) -> netlist::Transient {
    let Analysis::Transient(transient) = netlist.analysis() else {
        panic!("not a transient");
    };
    *transient
}

#[test]
fn an_export_with_nothing_appended_parses_to_the_same_netlist() {
    let (document, netlist) = Document::parse(document().as_bytes()).unwrap();
    let export = document.export(&netlist, Time::from_ps(10_000_000));
    let exported = netlist::parse(&export).unwrap();
    assert_eq!(parts(&exported), parts(&netlist));
    assert_eq!(exported.analysis(), netlist.analysis());
    assert_eq!(exported.seed(), netlist.seed());
}

/// `1e-50`, `4.99995000345103e-09` and a 17-digit value are spellings the fast parser reads
/// one ULP away from the decimal. The parsed resistance is the correctly rounded `f64`, and
/// the export parses back to that same netlist.
#[test]
fn an_export_round_trips_a_literal_the_fast_parser_misreads() {
    let json = with_parts(
        ANALYSIS,
        r#"{
          "name": "r_tiny",
          "kind": "resistor",
          "pins": { "a": "a", "b": "gnd" },
          "resistance": { "value": 1e-50, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        },
        {
          "name": "r_exp",
          "kind": "resistor",
          "pins": { "a": "a", "b": "gnd" },
          "resistance": { "value": 4.99995000345103e-09, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        },
        {
          "name": "r_wide",
          "kind": "resistor",
          "pins": { "a": "a", "b": "gnd" },
          "resistance": { "value": 9241.7580362847343, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        }"#,
    );
    let (document, netlist) = Document::parse(json.as_bytes()).unwrap();
    assert_eq!(ohms(&netlist, "r_tiny"), 1e-50);
    assert_eq!(ohms(&netlist, "r_exp"), 4.99995000345103e-9);
    // Rust and the exact parser read these 17 digits alike. The fast parser misread their reprint.
    #[allow(clippy::excessive_precision)]
    let wide = 9241.7580362847343_f64;
    assert_eq!(ohms(&netlist, "r_wide"), wide);
    let export = document.export(&netlist, Time::from_ps(10_000_000));
    let exported = netlist::parse(&export).unwrap();
    assert_eq!(parts(&exported), parts(&netlist));
}

fn ohms(netlist: &netlist::Netlist, name: &str) -> f64 {
    let id = netlist
        .part_id(&orbe_engine::name::Name::new(name).unwrap())
        .unwrap();
    let Model::Resistor(resistor) = &netlist.part(id).model else {
        panic!("{name} is not a resistor");
    };
    resistor.resistance.fixed().unwrap().si.get()
}

/// The appended events follow each switch's own, in the caller's part order. `stop_ps` is the
/// export's end, and the grid is cut to it.
#[test]
fn an_export_appends_each_switch_event_and_ends_at_its_stop() {
    let (document, mut netlist) = Document::parse(document().as_bytes()).unwrap();
    let (sw1, sw2) = (switch_id(&netlist, "sw1"), switch_id(&netlist, "sw2"));
    let earliest = |netlist: &netlist::Netlist, id| {
        let Model::Switch(switch) = &netlist.part(id).model else {
            panic!("not a switch");
        };
        switch.earliest(Time::ZERO).unwrap()
    };
    for (id, to) in [
        (sw2, Contact::Open),
        (sw1, Contact::Open),
        (sw2, Contact::Closed),
    ] {
        let at = earliest(&netlist, id);
        netlist.append(id, at, Appended::Contact(to));
    }
    let stop = Time::from_ps(3_000_000);
    let export = document.export(&netlist, stop);
    let exported = netlist::parse(&export).unwrap();
    assert_eq!(parts(&exported), parts(&netlist));
    let transient = transient(&exported);
    assert_eq!((transient.stop, transient.grid), (stop, 3_000_000));
    assert_eq!(transient.max_step, 100_000);

    let json: serde_json::Value = serde_json::from_slice(&export).unwrap();
    let events = |k: usize| json["parts"][k]["events"].clone();
    assert_eq!(json["parts"][0]["name"], "sw2");
    assert_eq!(
        events(0),
        serde_json::json!([
            { "t_ps": 1000, "state": "closed" },
            { "t_ps": 11000, "state": "open" },
            { "t_ps": 21000, "state": "closed" }
        ])
    );
    assert_eq!(
        events(2),
        serde_json::json!([{ "t_ps": 1, "state": "open" }])
    );
}

fn levels() -> String {
    with_parts(
        ANALYSIS,
        r#"{
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "a", "n": "gnd" },
          "voltage": {
            "value": 4.99995000345103e-09,
            "unit": "V",
            "source": { "closed_form": { "note": "kept" } }
          }
        },
        {
          "name": "i0",
          "kind": "current_source",
          "pins": { "p": "a", "n": "gnd" },
          "current": { "value": 0.0002, "unit": "A", "source": "user" }
        },
        {
          "name": "i1",
          "kind": "current_source",
          "pins": { "p": "a", "n": "gnd" },
          "current": {
            "kind": "steps",
            "initial": { "value": 0.0002, "unit": "A", "source": "user" },
            "events": [
              { "t_ps": 5000, "level": { "value": 0.0001, "unit": "A", "source": "user" } }
            ]
          }
        },
        {
          "name": "rv1",
          "kind": "potentiometer",
          "pins": { "a": "a", "wiper": "a", "b": "gnd" },
          "resistance": { "value": 10000.0, "unit": "ohm", "source": "user" },
          "position": { "value": 0.49951171875, "unit": "1", "source": "user" }
        },
        {
          "name": "r1",
          "kind": "resistor",
          "pins": { "a": "a", "b": "gnd" },
          "resistance": { "value": 0.30000000000000004, "unit": "ohm", "source": "user" },
          "ratings": { "power": null }
        },
        {
          "name": "sw1",
          "kind": "switch",
          "pins": { "a": "a", "b": "gnd" },
          "on_resistance": { "value": 1, "unit": "ohm", "source": "user" },
          "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
          "initial": "closed",
          "events": [],
          "bounce": null
        }"#,
    )
}

fn part_json<'a>(json: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    json["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("no {name}"))
}

fn earliest_level(netlist: &netlist::Netlist, name: &str) -> Time {
    let id = switch_id(netlist, name);
    let Some(level) = netlist.part(id).model.level() else {
        panic!("{name} takes no level");
    };
    level.schedule.earliest(Time::ZERO, 0).unwrap()
}

fn append_level(netlist: &mut netlist::Netlist, name: &str, si: f64) -> Time {
    let at = earliest_level(netlist, name);
    let id = switch_id(netlist, name);
    netlist.append(id, at, Appended::Level(Finite::new(si).unwrap()));
    at
}

fn round_trip(document: &Document, netlist: &netlist::Netlist) -> serde_json::Value {
    let export = document.export(netlist, Time::from_ps(10_000_000));
    let exported = netlist::parse(&export).unwrap();
    assert_eq!(parts(&exported), parts(netlist));
    assert_eq!(exported.analysis(), netlist.analysis());
    assert_eq!(exported.seed(), netlist.seed());
    serde_json::from_slice(&export).unwrap()
}

#[test]
fn an_export_of_a_plain_level_keeps_its_initial_and_writes_the_event_at_1_ps() {
    let json = levels();
    let original: serde_json::Value = serde_json::from_str(&json).unwrap();
    let (document, mut netlist) = Document::parse(json.as_bytes()).unwrap();
    let voltage_at = append_level(&mut netlist, "v1", 1.0);
    let position_at = append_level(&mut netlist, "rv1", 0.25);
    assert_eq!(
        (voltage_at, position_at),
        (Time::from_ps(1), Time::from_ps(1))
    );
    let exported = round_trip(&document, &netlist);
    for (name, unit, field) in [("v1", "V", "voltage"), ("rv1", "1", "position")] {
        let wrapped = part_json(&exported, name);
        assert_eq!(wrapped[field]["kind"], "steps");
        assert_eq!(wrapped[field]["initial"], part_json(&original, name)[field]);
        assert_eq!(
            wrapped[field]["events"],
            serde_json::json!([{
                "t_ps": 1,
                "level": { "value": if name == "v1" { 1.0 } else { 0.25 }, "unit": unit, "source": "user" }
            }])
        );
    }
}

#[test]
fn an_export_of_a_steps_level_appends_the_event_at_its_gap() {
    let json = levels();
    let original: serde_json::Value = serde_json::from_str(&json).unwrap();
    let (document, mut netlist) = Document::parse(json.as_bytes()).unwrap();
    let at = append_level(&mut netlist, "i1", 0.0003);
    assert_eq!(at, Time::from_ps(7_000));
    let exported = round_trip(&document, &netlist);
    let events = &part_json(&exported, "i1")["current"]["events"];
    assert_eq!(
        events[0],
        part_json(&original, "i1")["current"]["events"][0]
    );
    assert_eq!(
        events[1],
        serde_json::json!({
            "t_ps": 7000,
            "level": { "value": 0.0003, "unit": "A", "source": "user" }
        })
    );
}

#[test]
fn an_export_of_a_serial_source_appends_the_event_and_reparses() {
    let event = r#"{ "t_ps": 10000, "baud": 500000000, "frame": "8N1", "bytes": [0] }"#;
    let json = with_parts(
        ANALYSIS,
        &format!(
            r#"{{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "a", "n": "gnd" }},
              "voltage": {{
                "kind": "serial",
                "mark": {{ "value": 5.0, "unit": "V", "source": "user" }},
                "space": {{ "value": 0.0, "unit": "V", "source": "user" }},
                "events": [{event}]
              }}
            }}"#
        ),
    );
    let original: serde_json::Value = serde_json::from_str(&json).unwrap();
    let (document, mut netlist) = Document::parse(json.as_bytes()).unwrap();
    let untouched = document.export(&netlist, Time::from_ps(10_000_000));
    let untouched: serde_json::Value = serde_json::from_slice(&untouched).unwrap();
    assert_eq!(
        part_json(&untouched, "v1")["voltage"]["events"],
        part_json(&original, "v1")["voltage"]["events"]
    );
    netlist.append(
        switch_id(&netlist, "v1"),
        Time::from_ps(30_000),
        Appended::Serial(SerialSend {
            baud: Baud::new(500_000_000).unwrap(),
            frame: Frame::EightN1,
            bytes: vec![0xff],
        }),
    );
    let exported = round_trip(&document, &netlist);
    let events = &part_json(&exported, "v1")["voltage"]["events"];
    assert_eq!(
        events[0],
        part_json(&original, "v1")["voltage"]["events"][0]
    );
    assert_eq!(
        events[1],
        serde_json::json!({
            "t_ps": 30000,
            "baud": 500000000,
            "frame": "8N1",
            "bytes": [255]
        })
    );
    assert_eq!(
        part_json(&exported, "v1")["voltage"]["mark"],
        part_json(&original, "v1")["voltage"]["mark"]
    );
}

#[test]
fn an_export_leaves_an_unset_part_byte_for_byte() {
    let json = levels();
    let original: serde_json::Value = serde_json::from_str(&json).unwrap();
    let (document, mut netlist) = Document::parse(json.as_bytes()).unwrap();
    append_level(&mut netlist, "v1", 1.0);
    let export = document.export(&netlist, Time::from_ps(10_000_000));
    let parsed: serde_json::Value = serde_json::from_slice(&export).unwrap();
    for name in ["r1", "sw1", "i0", "rv1", "i1"] {
        assert_eq!(
            part_json(&parsed, name),
            part_json(&original, name),
            "{name}"
        );
    }
    assert!(part_json(&parsed, "i0")["current"].get("kind").is_none());
}
