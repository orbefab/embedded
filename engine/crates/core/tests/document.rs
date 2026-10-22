// Each test crate uses its own part of `common`.
#[allow(dead_code)]
mod common;

use common::with_parts;
use sim_core::netlist::{self, Analysis, Contact, Document, Edge, Model};
use sim_core::time::Time;

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
        .part_id(&sim_core::name::Name::new(name).unwrap())
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
        .part_id(&sim_core::name::Name::new(name).unwrap())
        .unwrap();
    let Model::Resistor(resistor) = &netlist.part(id).model else {
        panic!("{name} is not a resistor");
    };
    resistor.resistance.si.get()
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
        netlist.append(id, Edge { at, to });
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
