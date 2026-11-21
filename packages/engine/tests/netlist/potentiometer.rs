use orbe_model::Time;

use orbe_engine::netlist::{self, Model};

use crate::common;

use common::errors;

fn document(body: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 0,
      "analysis": {{ "op": {{}} }},
      "ground": "gnd",
      "nets": ["gnd", "top", "mid"],
      "parts": [{{
        "name": "rv1",
        "kind": "potentiometer",
        {body}
      }}]
    }}"#
    )
}

fn fields(resistance: &str, position: &str) -> String {
    format!(
        r#""pins": {{ "a": "top", "wiper": "mid", "b": "gnd" }},
        "resistance": {resistance},
        "position": {position}"#
    )
}

fn value(number: &str, unit: &str) -> String {
    format!(r#"{{ "value": {number}, "unit": "{unit}" }}"#)
}

fn pot(resistance: &str, position: &str) -> String {
    document(&fields(&value(resistance, "ohm"), &value(position, "1")))
}

#[test]
fn a_potentiometer_parses_its_pins_resistance_and_position() {
    let netlist = netlist::parse(pot("10000.0", "0.3").as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Potentiometer(pot) = &part.model else {
        panic!("{:?}", part.model);
    };
    assert_eq!(part.model.pin_names(), ["a", "wiper", "b"]);
    assert!(part.model.ratings().is_empty());
    assert!(part.model.next_change_after(Time::ZERO).is_none());
    assert_eq!(netlist.net_name(pot.pins[0]).as_str(), "top");
    assert_eq!(netlist.net_name(pot.pins[1]).as_str(), "mid");
    assert_eq!(netlist.net_name(pot.pins[2]).as_str(), "gnd");
    assert_eq!(pot.resistance.si.get(), 10000.0);
    assert!(pot.position.events().is_empty());
    assert_eq!(pot.position.initial().si.get(), 0.3);
}

#[test]
fn resistance_follows_the_resistor_rules_and_its_unit() {
    assert_eq!(
        errors(&pot("0", "0.3")),
        [r##""netlist.value" ["#rv1"] {"field":"resistance","rule":"positive","value":0.0}"##]
    );
    assert_eq!(
        errors(&pot("-4.0", "0.3")),
        [r##""netlist.value" ["#rv1"] {"field":"resistance","rule":"positive","value":-4.0}"##]
    );
    let tiny = errors(&pot("1e-320", "0.3"));
    assert_eq!(tiny.len(), 1, "{tiny:?}");
    assert!(
        tiny[0].contains(r#""field":"resistance","rule":"finite_reciprocal""#),
        "{tiny:?}"
    );
    assert_eq!(
        errors(&document(&fields(
            &value("10000.0", "kohm"),
            &value("0.3", "1"),
        ))),
        [r##""netlist.unit" ["#rv1"] {"expected":"ohm","field":"resistance","found":"kohm"}"##]
    );
}

#[test]
fn position_is_an_open_fraction_in_unit_1() {
    for (number, text) in [("0", "0.0"), ("1", "1.0"), ("-1", "-1.0"), ("2", "2.0")] {
        assert_eq!(
            errors(&pot("10000.0", number)),
            [format!(
                r##""netlist.value" ["#rv1"] {{"field":"position","rule":"fraction","value":{text}}}"##
            )],
            "{number}"
        );
    }
    assert_eq!(
        errors(&document(&fields(
            &value("10000.0", "ohm"),
            &value("0.3", "V"),
        ))),
        [r##""netlist.unit" ["#rv1"] {"expected":"1","field":"position","found":"V"}"##]
    );
    assert_eq!(
        errors(&document(&fields(
            &value("10000.0", "ohm"),
            &value("0", "V"),
        ))),
        [
            r##""netlist.unit" ["#rv1"] {"expected":"1","field":"position","found":"V"}"##,
            r##""netlist.value" ["#rv1"] {"field":"position","rule":"fraction","value":0.0}"##,
        ]
    );
}

#[test]
fn pins_are_exactly_a_wiper_and_b_and_every_field_is_required() {
    assert_eq!(
        errors(&document(
            r#""pins": { "a": "top", "b": "gnd" },
            "resistance": { "value": 10000.0, "unit": "ohm" },
            "position": { "value": 0.3, "unit": "1" }"#
        )),
        [r##""netlist.pins" ["#rv1"] {"missing":["wiper"],"unexpected":[]}"##]
    );
    assert_eq!(
        errors(&document(
            r#""pins": { "a": "top", "wiper": "mid", "b": "gnd", "c": "top" },
            "resistance": { "value": 10000.0, "unit": "ohm" },
            "position": { "value": 0.3, "unit": "1" }"#
        )),
        [r##""netlist.pins" ["#rv1"] {"missing":[],"unexpected":["c"]}"##]
    );
    for body in [
        r#""pins": { "a": "top", "wiper": "mid", "b": "gnd" },
          "resistance": { "value": 10000.0, "unit": "ohm" }"#,
        r#""pins": { "a": "top", "wiper": "mid", "b": "gnd" },
          "resistance": { "value": 10000.0, "unit": "ohm" },
          "position": { "value": 0.3, "unit": "1" },
          "ratings": { "power": null }"#,
    ] {
        let found = errors(&document(body));
        assert_eq!(found.len(), 1, "{body} {found:?}");
        assert!(
            found[0].starts_with(r#""netlist.syntax""#),
            "{body} {found:?}"
        );
    }
}

#[test]
fn each_leg_uses_the_resistor_rule_on_field_resistance() {
    let tiny = netlist::parse(pot("1.0", "1e-320").as_bytes()).unwrap_err();
    assert_eq!(tiny.len(), 1, "{tiny:?}");
    let text = serde_json::to_value(&tiny[0]).unwrap();
    assert_eq!(text["code"], "netlist.value");
    assert_eq!(text["data"]["field"], "resistance");
    assert_eq!(text["data"]["rule"], "finite_reciprocal");
    assert_eq!(text["data"]["value"].as_f64().unwrap(), 1e-320);

    assert_eq!(
        errors(&pot("1e-10", "1e-320")),
        [r##""netlist.value" ["#rv1"] {"field":"resistance","rule":"positive","value":0.0}"##]
    );
}

fn steps_position(initial: &str, events: &str) -> String {
    format!(
        r#"{{ "kind": "steps", "initial": {initial}, "events": [{events}] }}"#,
        initial = value(initial, "1"),
        events = events,
    )
}

fn level_event(t_ps: &str, number: &str, unit: &str) -> String {
    format!(r#"{{ "t_ps": {t_ps}, "level": {} }}"#, value(number, unit))
}

fn pot_position(resistance: &str, position: &str) -> String {
    document(&fields(&value(resistance, "ohm"), position))
}

#[test]
fn a_steps_position_moves_at_each_event_and_an_empty_list_stays_put() {
    let moving = pot_position(
        "10000.0",
        &steps_position("0.25", &level_event("5000", "0.75", "1")),
    );
    let netlist = netlist::parse(moving.as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Potentiometer(pot) = &part.model else {
        panic!("{:?}", part.model);
    };
    assert_eq!(pot.position.initial().si.get(), 0.25);
    assert_eq!(pot.position.events().len(), 1);
    assert_eq!(pot.position.events()[0].at, Time::from_ps(5_000));
    assert_eq!(pot.position.events()[0].to.si.get(), 0.75);
    assert_eq!(
        part.model.next_change_after(Time::ZERO),
        Some(Time::from_ps(5_000))
    );

    let fixed = pot_position("10000.0", &steps_position("0.3", ""));
    let netlist = netlist::parse(fixed.as_bytes()).unwrap();
    let (_, part) = netlist.parts().next().unwrap();
    let Model::Potentiometer(pot) = &part.model else {
        panic!("{:?}", part.model);
    };
    assert!(pot.position.events().is_empty());
    assert_eq!(pot.position.initial().si.get(), 0.3);
    assert!(part.model.next_change_after(Time::ZERO).is_none());
}

#[test]
fn each_position_rule_names_its_event_and_the_largest_time_does_not_overflow() {
    let schedule = |event: usize, rule: &str| {
        format!(r##""netlist.schedule" ["#rv1"] {{"event":{event},"rule":"{rule}"}}"##)
    };
    let cases = [
        (
            steps_position("0.25", &level_event("0", "0.75", "1")),
            vec![schedule(0, "at_zero")],
        ),
        (
            steps_position("0.25", &level_event("2000", "0.25", "1")),
            vec![schedule(0, "same_state")],
        ),
        (
            steps_position(
                "0.25",
                &format!(
                    "{}, {}",
                    level_event("4000", "0.5", "1"),
                    level_event("4000", "0.75", "1")
                ),
            ),
            vec![schedule(1, "not_increasing")],
        ),
        (
            steps_position(
                "0.25",
                &format!(
                    "{}, {}",
                    level_event("2000", "0.5", "1"),
                    level_event("3999", "0.75", "1")
                ),
            ),
            vec![schedule(1, "too_close")],
        ),
    ];
    for (position, want) in cases {
        assert_eq!(
            errors(&pot_position("10000.0", &position)),
            want,
            "{position}"
        );
    }

    let largest = steps_position("0.25", &level_event("18446744073709551615", "0.75", "1"));
    let netlist = netlist::parse(pot_position("10000.0", &largest).as_bytes()).unwrap();
    let Model::Potentiometer(pot) = &netlist.parts().next().unwrap().1.model else {
        panic!("not a potentiometer");
    };
    assert_eq!(pot.position.events()[0].at, Time::from_ps(u64::MAX));
}

#[test]
fn a_position_level_message_says_the_level_does_not_change() {
    let json = pot_position(
        "10000.0",
        &steps_position("0.25", &level_event("2000", "0.25", "1")),
    );
    let doc = serde_json::to_value(&netlist::parse(json.as_bytes()).unwrap_err()[0]).unwrap();
    assert_eq!(doc["data"]["rule"], "same_state");
    assert_eq!(doc["data"]["event"], 0);
    assert!(doc["data"].get("field").is_none());
    assert_eq!(
        doc["message"],
        "#rv1 position.events[0] does not change the level"
    );
}

#[test]
fn a_position_too_close_names_no_bounce_window() {
    let json = pot_position(
        "10000.0",
        &steps_position(
            "0.25",
            &format!(
                "{}, {}",
                level_event("2000", "0.5", "1"),
                level_event("3999", "0.75", "1")
            ),
        ),
    );
    let doc = serde_json::to_value(&netlist::parse(json.as_bytes()).unwrap_err()[0]).unwrap();
    assert_eq!(doc["data"]["event"], 1);
    assert_eq!(doc["data"]["rule"], "too_close");
    assert!(doc["data"].get("field").is_none());
    assert_eq!(
        doc["message"],
        "#rv1 position.events[1] is less than 2000 ps after the previous event"
    );
}

#[test]
fn a_position_level_uses_its_own_field_for_unit_and_fraction() {
    assert_eq!(
        errors(&pot_position(
            "10000.0",
            &steps_position("0.3", &level_event("2000", "0.6", "V")),
        )),
        [
            r##""netlist.unit" ["#rv1"] {"expected":"1","field":"position.events[0].level","found":"V"}"##
        ]
    );
    assert_eq!(
        errors(&pot_position("10000.0", &steps_position("0", ""),)),
        [
            r##""netlist.value" ["#rv1"] {"field":"position.initial","rule":"fraction","value":0.0}"##
        ]
    );
    for (number, text) in [("0", "0.0"), ("1", "1.0"), ("2", "2.0")] {
        assert_eq!(
            errors(&pot_position(
                "10000.0",
                &steps_position("0.3", &level_event("2000", number, "1")),
            )),
            [format!(
                r##""netlist.value" ["#rv1"] {{"field":"position.events[0].level","rule":"fraction","value":{text}}}"##
            )],
            "{number}"
        );
    }
    let at_zero = steps_position("0.3", &level_event("0", "0.6", "V"));
    assert_eq!(
        errors(&pot_position("10000.0", &at_zero)),
        [
            r##""netlist.unit" ["#rv1"] {"expected":"1","field":"position.events[0].level","found":"V"}"##,
            r##""netlist.schedule" ["#rv1"] {"event":0,"rule":"at_zero"}"##,
        ]
    );
}

#[test]
fn a_wrong_position_unit_reports_no_leg() {
    let unit = |field: &str| {
        format!(r##""netlist.unit" ["#rv1"] {{"expected":"1","field":"{field}","found":"V"}}"##)
    };
    let plain = |resistance: &str, position: &str| {
        document(&fields(&value(resistance, "ohm"), &value(position, "V")))
    };
    let cases = [
        (plain("10000.0", "1e-320"), vec![unit("position")]),
        (plain("1e-300", "1e-30"), vec![unit("position")]),
        (
            pot_position(
                "10000.0",
                &format!(
                    r#"{{ "kind": "steps", "initial": {}, "events": [] }}"#,
                    value("1e-320", "V")
                ),
            ),
            vec![unit("position.initial")],
        ),
        (
            pot_position(
                "1e-300",
                &format!(
                    r#"{{ "kind": "steps", "initial": {}, "events": [{}] }}"#,
                    value("0.5", "1"),
                    level_event("2000", "1e-30", "V"),
                ),
            ),
            vec![unit("position.events[0].level")],
        ),
    ];
    for (json, want) in cases {
        assert_eq!(errors(&json), want, "{json}");
    }
}

#[test]
fn a_steps_level_leg_uses_the_resistor_rule_on_field_resistance() {
    let tiny = steps_position("0.5", &level_event("2000", "1e-320", "1"));
    let reciprocal = netlist::parse(pot_position("1.0", &tiny).as_bytes()).unwrap_err();
    assert_eq!(reciprocal.len(), 1, "{reciprocal:?}");
    let text = serde_json::to_value(&reciprocal[0]).unwrap();
    assert_eq!(text["code"], "netlist.value");
    assert_eq!(text["data"]["field"], "resistance");
    assert_eq!(text["data"]["rule"], "finite_reciprocal");
    assert_eq!(text["data"]["value"].as_f64().unwrap(), 1e-320);

    assert_eq!(
        errors(&pot_position("1e-10", &tiny)),
        [r##""netlist.value" ["#rv1"] {"field":"resistance","rule":"positive","value":0.0}"##]
    );
}

#[test]
fn a_step_or_pulse_position_is_a_syntax_error() {
    let level = value("0.3", "1");
    let cases = [
        (
            format!(
                r#"{{ "kind": "step", "initial": {level}, "final": {level}, "at_ps": 2000 }}"#,
                level = level,
            ),
            "unknown variant `step`",
        ),
        (
            format!(
                r#"{{ "kind": "pulse", "initial": {level}, "pulsed": {level}, "delay_ps": 2000, "width_ps": 2000, "period_ps": 5000 }}"#,
                level = level,
            ),
            "unknown variant `pulse`",
        ),
        (
            format!(
                r#"{{ "value": 0.3, "kind": "steps", "initial": {level}, "events": [] }}"#,
                level = level,
            ),
            "unknown field `value`",
        ),
        (
            steps_position(
                "0.3",
                r#"{ "t_ps": 2000.5, "level": { "value": 0.4, "unit": "1" } }"#,
            ),
            "invalid type: floating point `2000.5`",
        ),
    ];
    for (position, detail) in cases {
        let found = netlist::parse(pot_position("10000.0", &position).as_bytes()).unwrap_err();
        let [diagnostic] = found.as_slice() else {
            panic!("{position} {found:?}");
        };
        let doc = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(doc["code"], "netlist.syntax", "{position}");
        let found = doc["data"]["detail"].as_str().unwrap();
        assert!(found.contains(detail), "{found} lacks {detail}");
    }
}
