use orbe_engine::name::Quantity;
use orbe_engine::netlist::{self, Model, RatingKind};

fn regulator(fields: &str) -> String {
    format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "op": {{}} }},
          "ground": "gnd",
          "nets": ["gnd", "vin", "vout"],
          "parts": [
            {{
              "name": "u1",
              "kind": "linear_regulator",
              "pins": {{ "in": "vin", "out": "vout", "gnd": "gnd" }},
              {fields}
            }}
          ]
        }}"#
    )
}

fn fitted() -> String {
    regulator(
        r#"
          "output_voltage": { "value": 3.3, "unit": "V", "source": "user" },
          "line_regulation": { "value": 0.0, "unit": "1", "source": "user" },
          "output_resistance": { "value": 0.00125, "unit": "ohm", "source": "user" },
          "dropout_voltage": { "value": 1.1, "unit": "V", "source": "user" },
          "dropout_resistance": { "value": 0.14, "unit": "ohm", "source": "user" },
          "current_limit": { "value": 1.2, "unit": "A", "source": "user" },
          "quiescent_current": { "value": 0.0, "unit": "A", "source": "user" },
          "ground_current_ratio": { "value": 0.0, "unit": "1", "source": "user" },
          "ratings": { "current": null, "power": null }
        "#,
    )
}

fn codes(json: &str) -> Vec<String> {
    netlist::parse(json.as_bytes())
        .unwrap_err()
        .iter()
        .map(|d| {
            let doc: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(d).unwrap()).unwrap();
            format!(
                "{} {} {}",
                doc["code"].as_str().unwrap(),
                doc["data"]["field"].as_str().unwrap_or(""),
                doc["data"]["rule"].as_str().unwrap_or("")
            )
        })
        .collect()
}

#[test]
fn a_regulator_parses_and_rates_iout() {
    let netlist = netlist::parse(fitted().as_bytes()).unwrap();
    let model = &netlist.parts().next().unwrap().1.model;
    assert_eq!(model.pin_names(), ["in", "out", "gnd"]);
    assert_eq!(
        model.quantities(),
        [Quantity::Iin, Quantity::Iout, Quantity::P]
    );
    assert_eq!(model.rated(RatingKind::Current), Quantity::Iout);
    assert_eq!(model.rated(RatingKind::Power), Quantity::P);
    let Model::LinearRegulator(reg) = model else {
        panic!("not a regulator");
    };
    assert_eq!(reg.line_regulation.si.get(), 0.0);
    assert_eq!(reg.quiescent_current.si.get(), 0.0);
}

fn edited(field: &str, value: &str, unit: &str) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(&fitted()).unwrap();
    let part = &mut doc["parts"][0];
    let target = if let Some(name) = field.strip_prefix("ratings.") {
        &mut part["ratings"][name]
    } else {
        &mut part[field]
    };
    *target = serde_json::json!({
        "value": serde_json::from_str::<serde_json::Value>(value).unwrap(),
        "unit": unit,
        "source": "user",
    });
    doc.to_string()
}

#[test]
fn each_field_rejects_a_bad_value_and_the_wrong_unit() {
    let cases = [
        (
            "output_voltage",
            "V",
            "mV",
            &[("0.0", "positive"), ("-1.0", "positive")][..],
        ),
        (
            "output_resistance",
            "ohm",
            "kohm",
            &[("0.0", "positive"), ("-1.0", "positive")][..],
        ),
        (
            "dropout_voltage",
            "V",
            "mV",
            &[("0.0", "positive"), ("-1.0", "positive")][..],
        ),
        (
            "dropout_resistance",
            "ohm",
            "kohm",
            &[("0.0", "positive"), ("-1.0", "positive")][..],
        ),
        (
            "current_limit",
            "A",
            "mA",
            &[("0.0", "positive"), ("-1.0", "positive")][..],
        ),
        (
            "line_regulation",
            "1",
            "V",
            &[("1.0", "below_one"), ("-0.1", "below_one")][..],
        ),
        (
            "quiescent_current",
            "A",
            "mA",
            &[("-0.001", "non_negative")][..],
        ),
        (
            "ground_current_ratio",
            "1",
            "A",
            &[("-0.2", "non_negative")][..],
        ),
        (
            "ratings.current",
            "A",
            "mA",
            &[("0.0", "positive"), ("-1.0", "positive")][..],
        ),
        (
            "ratings.power",
            "W",
            "mW",
            &[("0.0", "positive"), ("-1.0", "positive")][..],
        ),
    ];
    for (field, unit, wrong, rejects) in cases {
        assert_eq!(
            codes(&edited(field, "1.0", wrong)),
            [format!("netlist.unit {field} ")],
            "{field}"
        );
        for (value, rule) in rejects {
            assert_eq!(
                codes(&edited(field, value, unit)),
                [format!("netlist.value {field} {rule}")],
                "{field} {value}"
            );
        }
    }
}
