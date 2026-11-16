use orbe_engine::netlist::{self, Model};

fn op_amp() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/circuits/op-amp-bias/netlist.json");
    std::fs::read_to_string(path).unwrap()
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

fn edited(field: &str, value: &str, unit: &str) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(&op_amp()).unwrap();
    let part = (doc["parts"].as_array_mut().unwrap().iter_mut())
        .find(|part| part["kind"] == "op_amp")
        .unwrap();
    part[field] = serde_json::json!({
        "value": serde_json::from_str::<serde_json::Value>(value).unwrap(),
        "unit": unit,
        "source": "user",
    });
    doc.to_string()
}

#[test]
fn an_op_amp_parses_its_nine_fields_with_no_quantities_or_ratings() {
    let netlist = netlist::parse(op_amp().as_bytes()).unwrap();
    let model = &netlist
        .parts()
        .find(|(_, part)| part.name.as_str() == "u1")
        .unwrap()
        .1
        .model;
    assert_eq!(model.pin_names(), ["in_p", "in_n", "out", "vcc", "gnd"]);
    assert!(model.quantities().is_empty());
    assert!(model.ratings().is_empty());
    let Model::OpAmp(op) = model else {
        panic!("not an op-amp");
    };
    let fields = [
        &op.gain_bandwidth,
        &op.slew_rate,
        &op.open_loop_gain,
        &op.input_bias_current,
        &op.swing_high,
        &op.swing_low,
        &op.output_resistance,
        &op.short_circuit_current,
        &op.supply_current,
    ]
    .map(|value| value.si.get());
    assert_eq!(
        fields,
        [1e6, 1e6, 1e5, 1.5e-8, 0.0025, 0.051, 30.0, 0.04, 1.05e-4]
    );
}

#[test]
fn each_field_rejects_a_bad_value_and_the_wrong_unit() {
    let positive = &[("0.0", "positive"), ("-1.0", "positive")][..];
    let non_negative = &[("-1.0", "non_negative")][..];
    let cases = [
        ("gain_bandwidth", "Hz", "kHz", positive),
        ("slew_rate", "V/s", "V/us", positive),
        ("open_loop_gain", "1", "V/mV", positive),
        ("output_resistance", "ohm", "kohm", positive),
        ("short_circuit_current", "A", "mA", positive),
        ("input_bias_current", "A", "nA", non_negative),
        ("swing_high", "V", "mV", non_negative),
        ("swing_low", "V", "mV", non_negative),
        ("supply_current", "A", "uA", non_negative),
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
    for field in [
        "input_bias_current",
        "swing_high",
        "swing_low",
        "supply_current",
    ] {
        netlist::parse(
            edited(
                field,
                "0.0",
                match field {
                    "swing_high" | "swing_low" => "V",
                    _ => "A",
                },
            )
            .as_bytes(),
        )
        .unwrap_or_else(|errors| panic!("{field} 0: {errors:?}"));
    }
}
