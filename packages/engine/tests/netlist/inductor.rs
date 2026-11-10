use orbe_engine::netlist::{self, Model};

use orbe_engine::name::Quantity;

/// `l1` from `vin` to ground, with one field's unit and value replaced when `changed` names it.
fn inductor(changed: Option<(&str, &str, &str)>) -> String {
    let field = |name: &str, unit: &str, value: &str| {
        let (unit, value) = match changed {
            Some((field, unit, value)) if field == name => (unit, value),
            _ => (unit, value),
        };
        format!(r#""{name}": {{ "value": {value}, "unit": "{unit}", "source": "user" }}"#)
    };
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {{
          "name": "l1",
          "kind": "inductor",
          "pins": {{ "a": "vin", "b": "gnd" }},
          {},
          {}
        }}
      ]
    }}"#,
        field("inductance", "H", "0.505"),
        field("series_resistance", "ohm", "70.0"),
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
fn an_inductor_keeps_its_fields_and_has_i_v_and_p() {
    let netlist = netlist::parse(inductor(None).as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Inductor(l) = &part.model else {
        panic!("{:?}", part.model);
    };
    assert_eq!(l.inductance.si.get(), 0.505);
    assert_eq!(l.series_resistance.si.get(), 70.0);
    assert_eq!(part.model.pin_names(), ["a", "b"]);
    assert_eq!(
        part.model.quantities(),
        [Quantity::I, Quantity::V, Quantity::P]
    );
    assert!(part.model.ratings().is_empty());
    assert!(part.model.level().is_none());
}

#[test]
fn each_field_names_its_unit_and_must_be_positive_with_a_finite_reciprocal() {
    for (field, unit) in [("inductance", "H"), ("series_resistance", "ohm")] {
        assert_eq!(
            rejected(&inductor(Some((field, "mH", "1.0")))),
            [("netlist.unit".to_string(), field.to_string(), String::new())],
            "{field}"
        );
        for (value, rule) in [
            ("0.0", "positive"),
            ("-1.0", "positive"),
            ("1e-320", "finite_reciprocal"),
        ] {
            assert_eq!(
                rejected(&inductor(Some((field, unit, value)))),
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
fn a_missing_or_unknown_key_is_a_syntax_error() {
    let missing = inductor(None).replace(
        r#""series_resistance": { "value": 70.0, "unit": "ohm", "source": "user" }"#,
        r#""ratings": { "current": null }"#,
    );
    let wrong_type = inductor(Some(("inductance", "H", r#""0.505""#)));
    for json in [missing, wrong_type] {
        let found = rejected(&json);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "netlist.syntax", "{found:?}");
    }
}
