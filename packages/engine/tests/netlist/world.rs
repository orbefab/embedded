use orbe_model::Time;

use orbe_engine::netlist::{self, Model};
use orbe_engine::world::{Curve, Param, Quantity};

const TRANSIENT: &str =
    r#"{ "transient": { "stop_ps": 1000000, "max_step_ps": 1000, "grid_ps": 1000 } }"#;

fn value(value: f64, unit: &str) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "{unit}" }}"#)
}

fn document(world: &str, parts: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {TRANSIENT},
      "ground": "gnd",
      "nets": ["gnd", "a"],
      "world": {world},
      "parts": [{parts}]
    }}"#
    )
}

fn resistor(resistance: &str) -> String {
    format!(
        r#"{{ "name": "r1", "kind": "resistor", "pins": {{ "a": "a", "b": "gnd" }},
          "resistance": {resistance}, "ratings": {{ "power": null }} }}"#
    )
}

fn driven(quantity: &str, curve: &str) -> String {
    format!(r#"{{ "kind": "world", "quantity": "{quantity}", "curve": {curve} }}"#)
}

fn power() -> String {
    format!(
        r#"{{ "kind": "power", "x": {}, "y": {}, "exponent": {}, "max": {} }}"#,
        value(10.0, "lx"),
        value(8000.0, "ohm"),
        value(-0.7, "1"),
        value(1e6, "ohm")
    )
}

/// Each diagnostic as its code and message.
fn messages(json: &str) -> Vec<(String, String)> {
    let found = netlist::parse(json.as_bytes()).unwrap_err();
    found
        .iter()
        .map(|d| {
            let json = serde_json::to_value(d).unwrap();
            (
                d.code().to_string(),
                json["message"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn one(json: &str) -> (String, String) {
    let found = messages(json);
    assert_eq!(found.len(), 1, "{found:#?}");
    found.into_iter().next().unwrap()
}

#[test]
fn a_world_driven_resistance_parses_and_follows_the_schedule() {
    let schedule = format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": 5000, "level": {} }}] }}"#,
        value(10.0, "lx"),
        value(1000.0, "lx")
    );
    let json = document(
        &format!(r#"{{ "light": {schedule} }}"#),
        &resistor(&driven("light", &power())),
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let world = netlist.world();
    assert_eq!(world.declared().count(), 1);
    assert_eq!(
        world.before(Quantity::Light, Time::from_ps(5000)),
        Some(10.0)
    );
    assert_eq!(
        world.before(Quantity::Light, Time::from_ps(5001)),
        Some(1000.0)
    );
    assert_eq!(world.before(Quantity::Temperature, Time::ZERO), None);
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Resistor(resistor) = &part.model else {
        panic!("a resistor");
    };
    let Param::World(driven) = &resistor.resistance else {
        panic!("a world-driven resistance");
    };
    assert_eq!(driven.quantity, Quantity::Light);
    assert!(matches!(driven.curve, Curve::Power { .. }));
    let values = driven.values();
    assert_eq!(values.initial().get(), 8000.0, "y at x");
    // 8000 * 100^-0.7 = 8000 / 10^1.4
    let at_1000 = values.events()[0].to.get();
    assert!(
        (at_1000 - 8000.0 / 25.118864315095795).abs() < 1e-9,
        "{at_1000}"
    );
}

#[test]
fn the_three_curves_give_their_closed_forms() {
    let beta = Curve::Beta {
        x: si(25.0),
        y: si(10_000.0),
        beta: si(3950.0),
    };
    assert!((beta.at(25.0) - 10_000.0).abs() < 1e-9);
    // 10000 * exp(3950 * (1/323.15 - 1/298.15)) = 10000 * exp(-1.024939) = 3588.18 ohm.
    assert!((beta.at(50.0) - 3588.18).abs() < 0.01, "{}", beta.at(50.0));
    let lm35 = Curve::Linear {
        x0: si(0.0),
        y0: si(0.0),
        x1: si(100.0),
        y1: si(1.0),
    };
    assert!((lm35.at(25.0) - 0.25).abs() < 1e-15);
    assert!((lm35.at(-40.0) + 0.4).abs() < 1e-15);
    let ldr = Curve::Power {
        x: si(10.0),
        y: si(10_000.0),
        exponent: si(-1.0),
        max: si(1e6),
    };
    assert_eq!(ldr.at(50.0), 2000.0);
    assert_eq!(ldr.at(0.0), 1e6, "the dark reads max");

    // Vishay NTCLE100E3103JB0 (Doc 29049 p4 row 9, Mat A, B25/85 = 3977 K) vs p10 R/T table.
    let ntc = Curve::SteinhartHart {
        y: si(10_000.0),
        a: si(-14.6337),
        b: si(4791.842),
        c: si(-115_334.0),
        d: si(-3_730_535.0),
    };
    for (temp_c, table_ohm) in [
        (-40.0, 332_094.0),
        (-20.0, 96_358.0),
        (-10.0, 55_046.0),
        (0.0, 32_554.0),
        (25.0, 10_000.0),
        (85.0, 1_070.0),
        (125.0, 338.7),
    ] {
        let got = ntc.at(temp_c);
        let rel = (got - table_ohm).abs() / table_ohm;
        assert!(rel < 0.001, "at {temp_c} C: got {got}, table {table_ohm}");
    }
}

#[test]
fn a_steinhart_hart_curve_parses_and_rejects_non_temperature() {
    let sh = format!(
        r#"{{ "kind": "steinhart_hart", "y": {}, "a": {}, "b": {}, "c": {}, "d": {} }}"#,
        value(10_000.0, "ohm"),
        value(-14.6337, "1"),
        value(4791.842, "K"),
        value(-115_334.0, "K^2"),
        value(-3_730_535.0, "K^3")
    );
    let json = document(
        &format!(r#"{{ "temperature": {} }}"#, value(-40.0, "degC")),
        &resistor(&driven("temperature", &sh)),
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Resistor(r) = &part.model else {
        panic!("a resistor");
    };
    let Param::World(field) = &r.resistance else {
        panic!("a world-driven resistance");
    };
    assert_eq!(field.curve.kind(), "steinhart_hart");
    assert!((field.values().initial().get() - 332_094.0).abs() < 10.0);

    let wrong_qty = document(
        &format!(r#"{{ "light": {} }}"#, value(300.0, "lx")),
        &resistor(&driven("light", &sh)),
    );
    assert_eq!(one(&wrong_qty).0, "netlist.world");
}

fn si(value: f64) -> orbe_engine::netlist::Value {
    orbe_engine::netlist::Value {
        si: orbe_engine::finite::Finite::new(value).unwrap(),
    }
}

#[test]
fn a_value_outside_its_quantity_s_range_is_netlist_value_range() {
    let json = document(&format!(r#"{{ "light": {} }}"#, value(200_000.0, "lx")), "");
    assert_eq!(
        one(&json),
        (
            "netlist.value".into(),
            "#world.light value is 200000.0 lx; light must be from 0.0 to 100000.0 lx".into()
        )
    );
}

#[test]
fn an_unknown_quantity_is_netlist_world() {
    let json = document(&format!(r#"{{ "lite": {} }}"#, value(300.0, "lx")), "");
    assert_eq!(
        one(&json),
        (
            "netlist.world".into(),
            "world names `lite`, which is not a world quantity; the quantities are temperature, humidity, light, pressure".into()
        )
    );
    let json = document(
        &format!(r#"{{ "light": {} }}"#, value(300.0, "lx")),
        &resistor(&driven("lite", &power())),
    );
    assert_eq!(
        one(&json).1,
        "#r1 resistance.quantity names `lite`, which is not a world quantity; the quantities are temperature, humidity, light, pressure"
    );
}

#[test]
fn a_malformed_schedule_is_netlist_schedule() {
    let schedule = format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [
          {{ "t_ps": 0, "level": {} }},
          {{ "t_ps": 5000, "level": {} }},
          {{ "t_ps": 5000, "level": {} }},
          {{ "t_ps": 6000, "level": {} }},
          {{ "t_ps": 9000, "level": {} }}
        ] }}"#,
        value(300.0, "lx"),
        value(10.0, "lx"),
        value(20.0, "lx"),
        value(30.0, "lx"),
        value(40.0, "lx"),
        value(40.0, "lx")
    );
    let json = document(&format!(r#"{{ "light": {schedule} }}"#), "");
    let found: Vec<String> = messages(&json).into_iter().map(|(_, m)| m).collect();
    assert_eq!(
        found,
        [
            "#world.light events[0] is at 0 ps; an event must come after t = 0",
            "#world.light events[2] is not after the previous event",
            "#world.light events[3] is less than 2000 ps after the previous event",
            "#world.light events[4] does not change the level",
        ]
    );
    let flat = document(r#"{ "light": [300] }"#, "");
    let (code, message) = one(&flat);
    assert_eq!(code, "netlist.syntax");
    assert!(message.contains("world"), "{message}");
}

#[test]
fn a_part_that_reads_an_undeclared_quantity_is_netlist_world() {
    let json = document("{}", &resistor(&driven("light", &power())));
    assert_eq!(
        one(&json),
        (
            "netlist.world".into(),
            "#r1 resistance.quantity reads light, which the netlist's `world` does not declare"
                .into()
        )
    );
}

#[test]
fn a_curve_that_breaks_its_field_at_an_end_of_the_range_is_netlist_value() {
    let falling = format!(
        r#"{{ "kind": "linear", "x0": {}, "y0": {}, "x1": {}, "y1": {} }}"#,
        value(0.0, "degC"),
        value(10_000.0, "ohm"),
        value(100.0, "degC"),
        value(1000.0, "ohm")
    );
    let json = document(
        &format!(r#"{{ "temperature": {} }}"#, value(25.0, "degC")),
        &resistor(&driven("temperature", &falling)),
    );
    assert_eq!(
        one(&json),
        (
            "netlist.value".into(),
            "#r1 resistance.curve at 125.0 degC is -1250.0; it must be positive".into()
        )
    );
    let beta = format!(
        r#"{{ "kind": "beta", "x": {}, "y": {}, "beta": {} }}"#,
        value(25.0, "lx"),
        value(10_000.0, "ohm"),
        value(3950.0, "K")
    );
    let json = document(
        &format!(r#"{{ "light": {} }}"#, value(300.0, "lx")),
        &resistor(&driven("light", &beta)),
    );
    assert_eq!(
        one(&json).1,
        "#r1 resistance.curve.kind is a B equation, which reads temperature, not light"
    );
}

#[test]
fn a_world_value_in_another_unit_is_netlist_unit() {
    let json = document(&format!(r#"{{ "temperature": {} }}"#, value(25.0, "K")), "");
    assert_eq!(one(&json).0, "netlist.unit");
}

fn temperature_power(exponent: f64) -> String {
    format!(
        r#"{{ "kind": "power", "x": {}, "y": {}, "exponent": {}, "max": {} }}"#,
        value(10.0, "degC"),
        value(1000.0, "ohm"),
        value(exponent, "1"),
        value(1e6, "ohm")
    )
}

#[test]
fn a_power_curve_on_a_quantity_that_goes_below_0_is_netlist_world() {
    let to_zero = format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": 5000, "level": {} }}] }}"#,
        value(25.0, "degC"),
        value(0.0, "degC")
    );
    let cold = format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [] }}"#,
        value(-40.0, "degC")
    );
    for (schedule, exponent) in [(to_zero, 2.0), (cold, -0.7)] {
        let json = document(
            &format!(r#"{{ "temperature": {schedule} }}"#),
            &resistor(&driven("temperature", &temperature_power(exponent))),
        );
        assert_eq!(
            one(&json),
            (
                "netlist.world".into(),
                "#r1 resistance.curve.kind is a power curve, which reads a quantity that does not go below 0, not temperature".into()
            ),
            "exponent {exponent}"
        );
    }
}

#[test]
fn a_power_curve_that_is_not_a_number_at_an_end_is_netlist_value_finite() {
    let zero = format!(
        r#"{{ "kind": "power", "x": {}, "y": {}, "exponent": {}, "max": {} }}"#,
        value(10.0, "lx"),
        value(0.0, "ohm"),
        value(-1.0, "1"),
        value(1e6, "ohm")
    );
    let json = document(
        &format!(r#"{{ "light": {} }}"#, value(300.0, "lx")),
        &resistor(&driven("light", &zero)),
    );
    let found: Vec<String> = messages(&json).into_iter().map(|(_, m)| m).collect();
    assert_eq!(
        found,
        [
            "#r1 resistance.curve at 0.0 lx is NaN; it must be finite",
            "#r1 resistance.curve at 100000.0 lx is 0.0; it must be positive",
        ]
    );
}

#[test]
fn a_rejected_world_value_is_the_only_diagnostic_for_a_part_that_reads_it() {
    let same = format!(
        r#"{{ "kind": "steps", "initial": {}, "events": [{{ "t_ps": 5000, "level": {} }}] }}"#,
        value(300.0, "lx"),
        value(300.0, "lx")
    );
    let json = document(
        &format!(r#"{{ "light": {same} }}"#),
        &resistor(&driven("light", &power())),
    );
    assert_eq!(
        one(&json),
        (
            "netlist.schedule".into(),
            "#world.light events[0] does not change the level".into()
        )
    );
    let json = document(
        &format!(r#"{{ "light": {} }}"#, value(-5.0, "lx")),
        &resistor(&driven("light", &power())),
    );
    assert_eq!(
        one(&json),
        (
            "netlist.value".into(),
            "#world.light value is -5.0 lx; light must be from 0.0 to 100000.0 lx".into()
        )
    );
}
