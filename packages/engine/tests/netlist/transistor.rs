use orbe_engine::name::Quantity;
use orbe_engine::netlist::{self, Model, RatingKind};

/// Every field the `npn` and `pnp` kinds take, with its unit and a value that parses.
const FIELDS: [(&str, &str, f64); 11] = [
    ("saturation_current", "A", 8.095e-15),
    ("forward_beta", "1", 173.2),
    ("reverse_beta", "1", 0.2704),
    ("forward_emission", "1", 1.0),
    ("reverse_emission", "1", 1.0),
    ("early_voltage", "V", 100.0),
    ("base_resistance", "ohm", 5.707),
    ("collector_resistance", "ohm", 1.531),
    ("emitter_resistance", "ohm", 0.867),
    ("base_emitter_capacitance", "F", 8e-12),
    ("base_collector_capacitance", "F", 4e-12),
];

/// `q1` of `kind`, with one field's unit and value replaced when `changed` names it.
fn transistor(kind: &str, changed: Option<(&str, &str, &str)>) -> String {
    let fields: Vec<String> = FIELDS
        .iter()
        .map(|&(field, unit, value)| {
            let (unit, value) = match changed {
                Some((name, unit, value)) if name == field => (unit.to_string(), value.to_string()),
                _ => (unit.to_string(), format!("{value:?}")),
            };
            format!(r#""{field}": {{ "value": {value}, "unit": "{unit}", "source": "user" }}"#)
        })
        .collect();
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "b", "c"],
      "parts": [
        {{
          "name": "q1",
          "kind": "{kind}",
          "pins": {{ "collector": "c", "base": "b", "emitter": "gnd" }},
          {},
          "ratings": {{
            "current": {{ "value": 0.2, "unit": "A", "source": "user" }},
            "power": {{ "value": 0.625, "unit": "W", "source": "user" }}
          }}
        }}
      ]
    }}"#,
        fields.join(",\n          ")
    )
}

/// `code`, `field` and `rule` of each diagnostic.
fn rejected(json: &str) -> Vec<(String, String, String)> {
    netlist::parse(json.as_bytes())
        .unwrap_err()
        .iter()
        .map(|d| {
            let d = serde_json::to_value(d).unwrap();
            let text = |key: &str| d["data"][key].as_str().unwrap_or("").to_string();
            (
                d["code"].as_str().unwrap().to_string(),
                text("field"),
                text("rule"),
            )
        })
        .collect()
}

#[test]
fn an_npn_keeps_every_field_and_rates_ic_and_p() {
    let netlist = netlist::parse(transistor("npn", None).as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Npn(bjt) = &part.model else {
        panic!("{:?}", part.model);
    };
    let got = [
        &bjt.saturation_current,
        &bjt.forward_beta,
        &bjt.reverse_beta,
        &bjt.forward_emission,
        &bjt.reverse_emission,
        &bjt.early_voltage,
        &bjt.base_resistance,
        &bjt.collector_resistance,
        &bjt.emitter_resistance,
        &bjt.base_emitter_capacitance,
        &bjt.base_collector_capacitance,
    ]
    .map(|value| value.si.get());
    assert_eq!(got, FIELDS.map(|(_, _, value)| value));
    assert_eq!(part.model.pin_names(), ["collector", "base", "emitter"]);
    assert_eq!(
        part.model.quantities(),
        [Quantity::Ib, Quantity::Ic, Quantity::P]
    );
    let ratings: Vec<_> = part
        .model
        .ratings()
        .into_iter()
        .map(|(kind, value)| (kind, part.model.rated(kind), value.si.get()))
        .collect();
    assert_eq!(
        ratings,
        [
            (RatingKind::Current, Quantity::Ic, 0.2),
            (RatingKind::Power, Quantity::P, 0.625)
        ]
    );
}

#[test]
fn a_pnp_takes_the_same_fields_as_an_npn() {
    let npn = netlist::parse(transistor("npn", None).as_bytes()).unwrap();
    let pnp = netlist::parse(transistor("pnp", None).as_bytes()).unwrap();
    let (Model::Npn(n), Model::Pnp(p)) = (
        &npn.parts().next().unwrap().1.model,
        &pnp.parts().next().unwrap().1.model,
    ) else {
        panic!("not an npn and a pnp");
    };
    assert_eq!(n, p);
    for (field, _, _) in FIELDS {
        assert_eq!(
            rejected(&transistor("pnp", Some((field, "mA", "1.0")))),
            rejected(&transistor("npn", Some((field, "mA", "1.0")))),
            "{field}"
        );
    }
}

#[test]
fn each_field_names_its_unit_and_must_be_positive_with_a_finite_reciprocal() {
    for (field, unit, _) in FIELDS {
        assert_eq!(
            rejected(&transistor("npn", Some((field, "mA", "1.0")))),
            [("netlist.unit".to_string(), field.to_string(), String::new())],
            "{field}"
        );
        for (value, rule) in [
            ("0.0", "positive"),
            ("-1.0", "positive"),
            ("1e-320", "finite_reciprocal"),
        ] {
            let found = rejected(&transistor("npn", Some((field, unit, value))));
            assert_eq!(
                found,
                [(
                    "netlist.value".to_string(),
                    field.to_string(),
                    rule.to_string()
                )],
                "{field} {value}"
            );
        }
    }
}

#[test]
fn finite_junction_names_the_emission_of_the_junction_that_breaks_it() {
    let junction = |field: &str| {
        (
            "netlist.value".to_string(),
            field.to_string(),
            "finite_junction".to_string(),
        )
    };
    for field in ["forward_emission", "reverse_emission"] {
        let found = rejected(&transistor("npn", Some((field, "1", "1e308"))));
        assert_eq!(found, [junction(field)], "{field}");
    }
    let both = transistor("npn", Some(("forward_emission", "1", "1e308"))).replace(
        r#""reverse_emission": { "value": 1.0"#,
        r#""reverse_emission": { "value": 1e308"#,
    );
    assert_eq!(
        rejected(&both),
        [junction("forward_emission"), junction("reverse_emission")]
    );
    let tiny = transistor("npn", Some(("saturation_current", "A", "1e-308")))
        .replace(
            r#""forward_emission": { "value": 1.0"#,
            r#""forward_emission": { "value": 100.0"#,
        )
        .replace(
            r#""reverse_emission": { "value": 1.0"#,
            r#""reverse_emission": { "value": 100.0"#,
        );
    assert_eq!(rejected(&tiny), [junction("saturation_current")]);
}

const MOS_FIELDS: [(&str, &str, f64); 6] = [
    ("transconductance_parameter", "A/V^2", 0.0468),
    ("threshold_voltage", "V", 1.08),
    ("channel_length_modulation", "1/V", 0.0),
    ("gate_source_capacitance", "F", 55e-12),
    ("gate_drain_capacitance", "F", 5e-12),
    ("drain_source_capacitance", "F", 20e-12),
];

fn mosfet(changed: Option<(&str, &str, &str)>) -> String {
    let fields: Vec<String> = MOS_FIELDS
        .iter()
        .map(|&(field, unit, value)| {
            let (unit, value) = match changed {
                Some((name, unit, value)) if name == field => (unit.to_string(), value.to_string()),
                _ => (unit.to_string(), format!("{value:?}")),
            };
            format!(r#""{field}": {{ "value": {value}, "unit": "{unit}", "source": "user" }}"#)
        })
        .collect();
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "d", "g"],
      "parts": [
        {{
          "name": "m1",
          "kind": "nmos",
          "pins": {{ "drain": "d", "gate": "g", "source": "gnd" }},
          {},
          "ratings": {{
            "current": {{ "value": 0.2, "unit": "A", "source": "user" }},
            "power": {{ "value": 0.35, "unit": "W", "source": "user" }}
          }}
        }}
      ]
    }}"#,
        fields.join(",\n          ")
    )
}

#[test]
fn an_nmos_keeps_every_field_and_rates_id_and_p() {
    let netlist = netlist::parse(mosfet(None).as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Nmos(mosfet) = &part.model else {
        panic!("{:?}", part.model);
    };
    let got = [
        mosfet.transconductance_parameter.si.get(),
        mosfet.threshold_voltage.si.get(),
        mosfet.channel_length_modulation.si.get(),
        mosfet.gate_source_capacitance.si.get(),
        mosfet.gate_drain_capacitance.si.get(),
        mosfet.drain_source_capacitance.si.get(),
    ];
    assert_eq!(got, MOS_FIELDS.map(|(_, _, value)| value));
    assert_eq!(part.model.pin_names(), ["drain", "gate", "source"]);
    assert_eq!(part.model.quantities(), [Quantity::Id, Quantity::P]);
    let ratings: Vec<_> = part
        .model
        .ratings()
        .into_iter()
        .map(|(kind, value)| (kind, part.model.rated(kind), value.si.get()))
        .collect();
    assert_eq!(
        ratings,
        [
            (RatingKind::Current, Quantity::Id, 0.2),
            (RatingKind::Power, Quantity::P, 0.35)
        ]
    );
}

#[test]
fn mosfet_fields_name_their_units_and_rules() {
    for (field, _, _) in MOS_FIELDS {
        assert_eq!(
            rejected(&mosfet(Some((field, "mA", "1.0")))),
            [("netlist.unit".to_string(), field.to_string(), String::new())],
            "{field}"
        );
    }
    for field in [
        "transconductance_parameter",
        "gate_source_capacitance",
        "gate_drain_capacitance",
        "drain_source_capacitance",
    ] {
        let unit = MOS_FIELDS
            .iter()
            .find(|(name, _, _)| *name == field)
            .unwrap()
            .1;
        for (value, rule) in [
            ("0.0", "positive"),
            ("-1.0", "positive"),
            ("1e-320", "finite_reciprocal"),
        ] {
            assert_eq!(
                rejected(&mosfet(Some((field, unit, value)))),
                [(
                    "netlist.value".to_string(),
                    field.to_string(),
                    rule.to_string()
                )],
                "{field} {value}"
            );
        }
    }
    let negative =
        netlist::parse(mosfet(Some(("threshold_voltage", "V", "-1.5"))).as_bytes()).unwrap();
    let Model::Nmos(device) = &negative.parts().next().unwrap().1.model else {
        panic!("not an nmos");
    };
    assert_eq!(device.threshold_voltage.si.get(), -1.5);
    assert!(netlist::parse(mosfet_text_with_infinite_threshold().as_bytes()).is_err());
    assert_eq!(
        rejected(&mosfet(Some(("channel_length_modulation", "1/V", "-0.1")))),
        [(
            "netlist.value".to_string(),
            "channel_length_modulation".to_string(),
            "non_negative".to_string()
        )]
    );
    assert!(
        netlist::parse(mosfet(Some(("channel_length_modulation", "1/V", "0.0"))).as_bytes())
            .is_ok()
    );
    let ratings = mosfet(None).replace(
        r#""current": { "value": 0.2, "unit": "A""#,
        r#""current": { "value": 0.2, "unit": "mA""#,
    );
    assert_eq!(
        rejected(&ratings),
        [(
            "netlist.unit".to_string(),
            "ratings.current".to_string(),
            String::new()
        )]
    );
}

fn mosfet_text_with_infinite_threshold() -> String {
    mosfet(None).replace(
        r#""threshold_voltage": { "value": 1.08"#,
        r#""threshold_voltage": { "value": 1e309"#,
    )
}
