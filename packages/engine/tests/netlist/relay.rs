use orbe_engine::netlist::{self, Model};

use orbe_engine::name::Quantity;

const FIELDS: [(&str, &str, &str); 6] = [
    ("inductance", "H", "0.505"),
    ("coil_resistance", "ohm", "70.0"),
    ("pull_in_current", "A", "0.0536"),
    ("drop_out_current", "A", "0.00714"),
    ("on_resistance", "ohm", "0.1"),
    ("off_resistance", "ohm", "100000000.0"),
];

/// `k1` with its coil from `vcc` to ground and its contact on three nets, with each field in
/// `changed` given that unit and value.
fn relay(changed: &[(&str, &str, &str)]) -> String {
    let fields: Vec<String> = FIELDS
        .iter()
        .map(|&(name, unit, value)| {
            let (unit, value) = changed
                .iter()
                .find(|(field, ..)| *field == name)
                .map_or((unit, value), |&(_, unit, value)| (unit, value));
            format!(r#""{name}": {{ "value": {value}, "unit": "{unit}" }}"#)
        })
        .collect();
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "no", "nc"],
      "parts": [
        {{
          "name": "k1",
          "kind": "relay",
          "pins": {{ "coil_a": "vcc", "coil_b": "gnd", "com": "vcc", "no": "no", "nc": "nc" }},
          {}
        }}
      ]
    }}"#,
        fields.join(",\n          ")
    )
}

/// `code`, `field`, `rule` and `message` of each diagnostic.
fn rejected(json: &str) -> Vec<[String; 4]> {
    netlist::parse(json.as_bytes())
        .unwrap_err()
        .iter()
        .map(|d| {
            let d = serde_json::to_value(d).unwrap();
            let text = |value: &serde_json::Value| value.as_str().unwrap_or("").to_string();
            [
                text(&d["code"]),
                text(&d["data"]["field"]),
                text(&d["data"]["rule"]),
                text(&d["message"]),
            ]
        })
        .collect()
}

fn diagnostic(code: &str, field: &str, rule: &str, message: &str) -> [String; 4] {
    [code, field, rule, message].map(String::from)
}

#[test]
fn a_relay_keeps_its_fields_and_has_its_coil_s_i_v_and_p() {
    let netlist = netlist::parse(relay(&[]).as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Relay(k) = &part.model else {
        panic!("{:?}", part.model);
    };
    assert_eq!(k.inductance.si.get(), 0.505);
    assert_eq!(k.coil_resistance.si.get(), 70.0);
    assert_eq!(k.pull_in_current.si.get(), 0.0536);
    assert_eq!(k.drop_out_current.si.get(), 0.00714);
    assert_eq!(k.on_resistance.si.get(), 0.1);
    assert_eq!(k.off_resistance.si.get(), 1e8);
    assert_eq!(
        part.model.pin_names(),
        ["coil_a", "coil_b", "com", "no", "nc"]
    );
    assert_eq!(
        part.model.quantities(),
        [Quantity::I, Quantity::V, Quantity::P]
    );
    assert!(part.model.ratings().is_empty());
    assert!(part.model.level().is_none());
}

#[test]
fn each_field_names_its_unit_and_must_be_positive_with_a_finite_reciprocal() {
    for (field, unit, _) in FIELDS {
        assert_eq!(
            rejected(&relay(&[(field, "mA", "1.0")])),
            [diagnostic(
                "netlist.unit",
                field,
                "",
                &format!("#k1 {field} is in `mA`; it must be in `{unit}` (SI base units)")
            )],
            "{field}"
        );
        for (value, rule) in [
            ("0.0", "positive"),
            ("-1.0", "positive"),
            ("1e-320", "finite_reciprocal"),
        ] {
            let found = rejected(&relay(&[(field, unit, value)]));
            assert_eq!(found.len(), 1, "{field} {value}: {found:?}");
            assert_eq!(found[0][..3], ["netlist.value", field, rule], "{field}");
        }
    }
}

#[test]
fn drop_out_at_or_above_pull_in_breaks_hysteresis() {
    for drop_out in ["0.0536", "0.06"] {
        assert_eq!(
            rejected(&relay(&[("drop_out_current", "A", drop_out)])),
            [diagnostic(
                "netlist.value",
                "drop_out_current",
                "hysteresis",
                &format!("#k1 drop_out_current is {drop_out}; it must be below pull_in_current")
            )],
            "{drop_out}"
        );
    }
    assert!(netlist::parse(relay(&[("drop_out_current", "A", "0.0535")]).as_bytes()).is_ok());
}

#[test]
fn a_missing_key_an_unknown_key_or_a_wrong_type_is_a_syntax_error() {
    let missing = relay(&[]).replace(
        r#""off_resistance": { "value": 100000000.0, "unit": "ohm" }"#,
        r#""bounce": null"#,
    );
    let wrong_type = relay(&[("inductance", "H", r#""0.505""#)]);
    let missing_pin = relay(&[]).replace(r#", "nc": "nc""#, "");
    for json in [missing, wrong_type] {
        let found = rejected(&json);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0][0], "netlist.syntax", "{found:?}");
    }
    let found = rejected(&missing_pin);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0][0], "netlist.pins", "{found:?}");
}
