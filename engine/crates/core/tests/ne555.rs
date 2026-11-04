use sim_core::diagnostic::Diagnostic;
use sim_core::netlist::{self, Chip, Model, Ne555, Netlist, Source};

/// One NE555 with every field, and `edit` applied to its JSON object.
fn chip(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Netlist, Vec<Diagnostic>> {
    let value =
        |v: f64, unit: &str| serde_json::json!({ "value": v, "unit": unit, "source": "user" });
    let mut part = serde_json::json!({
        "name": "u1",
        "kind": "ne555",
        "pins": {
            "gnd": "gnd", "trig": "thr", "out": "out", "reset": "vcc",
            "cont": "cont", "thres": "thr", "disch": "dis", "vcc": "vcc"
        },
        "ladder_resistance": value(5000.0, "ohm"),
        "reset_threshold": value(0.7, "V"),
        "discharge": {
            "on_resistance": value(18.75, "ohm"),
            "off_resistance": value(2.5e8, "ohm")
        },
        "supply_resistance": value(1875.0, "ohm"),
        "output": {
            "high_drop": value(0.9, "V"),
            "high_resistance": value(8.0, "ohm"),
            "low_drop": value(0.0, "V"),
            "low_resistance": value(16.0, "ohm")
        }
    });
    edit(&mut part);
    let document = serde_json::json!({
        "version": 0,
        "seed": 0,
        "analysis": { "op": {} },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "thr", "out", "cont", "dis"],
        "parts": [part]
    });
    netlist::parse(&serde_json::to_vec(&document).unwrap())
}

fn values(netlist: &Netlist) -> Ne555 {
    let (_, part) = netlist.parts().next().unwrap();
    match &part.model {
        Model::Chip(Chip::Ne555(chip)) => (**chip).clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_ne555_parses_with_its_pins_in_package_order() {
    let netlist = chip(|_| {}).unwrap();
    let chip = values(&netlist);
    let nets: Vec<_> = chip
        .pins
        .iter()
        .map(|&id| netlist.net_name(id).as_str())
        .collect();
    assert_eq!(
        nets,
        ["gnd", "thr", "out", "vcc", "cont", "thr", "dis", "vcc"]
    );
    assert_eq!(
        Ne555::PINS,
        [
            "gnd", "trig", "out", "reset", "cont", "thres", "disch", "vcc"
        ]
    );
    assert_eq!(chip.discharge.off_resistance.si.get(), 2.5e8);
    assert_eq!(chip.output.low_drop.si.get(), 0.0);
    assert_eq!(chip.reset_threshold.source, Source::User);
    let (_, part) = netlist.parts().next().unwrap();
    assert!(part.model.ratings().is_empty());
}

fn errors(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<String> {
    chip(edit)
        .unwrap_err()
        .iter()
        .map(|d| serde_json::to_string(d).unwrap())
        .collect()
}

#[test]
fn a_negative_drop_is_non_negative_and_a_zero_resistance_is_positive() {
    let found = errors(|part| {
        part["output"]["low_drop"]["value"] = (-0.1).into();
        part["discharge"]["off_resistance"]["value"] = 0.0.into();
    });
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found.iter().any(|d| d.contains(
            r##""message":"#u1 output.low_drop is -0.1; it must not be negative","data":{"field":"output.low_drop","rule":"non_negative","value":-0.1}"##
        )),
        "{found:?}"
    );
    assert!(
        found.iter().any(|d| d.contains(
            r#""data":{"field":"discharge.off_resistance","rule":"positive","value":0.0}"#
        )),
        "{found:?}"
    );
}

#[test]
fn reset_threshold_must_be_positive_volts() {
    let found = errors(|part| {
        part["reset_threshold"]["value"] = 0.0.into();
        part["ladder_resistance"]["unit"] = "kohm".into();
    });
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .any(|d| d.contains(r#""field":"reset_threshold","rule":"positive""#))
    );
    assert!(
        found
            .iter()
            .any(|d| d.contains(r#""expected":"ohm","field":"ladder_resistance","found":"kohm""#))
    );
}

#[test]
fn a_missing_pin_or_value_is_reported() {
    let pins = errors(|part| {
        part["pins"].as_object_mut().unwrap().remove("cont");
    });
    assert_eq!(pins.len(), 1, "{pins:?}");
    assert!(pins[0].contains(r#""data":{"missing":["cont"],"unexpected":[]}"#));
    let field = chip(|part| {
        part["output"].as_object_mut().unwrap().remove("high_drop");
    })
    .unwrap_err();
    assert_eq!(field[0].code(), "netlist.syntax");
}
