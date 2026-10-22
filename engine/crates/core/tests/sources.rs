mod common;

use common::{errors, with_parts};
use sim_core::netlist::{self, Model, Netlist, Shape, Waveform};
use sim_core::time::Time;

fn level(value: f64, unit: &str) -> String {
    format!(r#"{{ "value": {value:?}, "unit": "{unit}", "source": "user" }}"#)
}

fn source(kind: &str, key: &str, drive: &str) -> String {
    with_parts(
        r#"{ "transient": { "stop_ps": 100000, "max_step_ps": 1000, "grid_ps": 1000 } }"#,
        &format!(
            r#"{{ "name": "s1", "kind": "{kind}", "pins": {{ "p": "a", "n": "gnd" }}, "{key}": {drive} }}"#
        ),
    )
}

fn step(unit: &str, at_ps: &str) -> String {
    format!(
        r#"{{ "kind": "step", "initial": {}, "final": {}, "at_ps": {at_ps} }}"#,
        level(0.0, unit),
        level(5.0, unit)
    )
}

fn pulse(unit: &str, delay: &str, width: &str, period: &str) -> String {
    format!(
        r#"{{ "kind": "pulse", "initial": {}, "pulsed": {},
             "delay_ps": {delay}, "width_ps": {width}, "period_ps": {period} }}"#,
        level(0.0002, unit),
        level(0.001, unit)
    )
}

fn parse(json: &str) -> Netlist {
    netlist::parse(json.as_bytes()).unwrap()
}

fn drive(model: &Model) -> &Waveform {
    match model {
        Model::VoltageSource(source) => &source.voltage,
        Model::CurrentSource(source) => &source.current,
        other => panic!("not a source: {other:?}"),
    }
}

fn jumps(netlist: &Netlist) -> Vec<u64> {
    let (_, part) = netlist.parts().next().unwrap();
    let wave = drive(&part.model);
    std::iter::successors(wave.next_change_after(Time::ZERO), |&at| {
        wave.next_change_after(at)
    })
    .take_while(|&at| at < Time::from_ps(100_000))
    .map(Time::as_ps)
    .collect()
}

#[test]
fn a_dc_current_source_has_pins_p_and_n_and_no_ratings() {
    let netlist = parse(&source("current_source", "current", &level(0.001, "A")));
    let (_, s1) = netlist.parts().next().unwrap();
    let Model::CurrentSource(current) = &s1.model else {
        panic!("{:?}", s1.model);
    };
    assert_eq!(s1.model.pin_names(), ["p", "n"]);
    assert_eq!(
        s1.model.pins(),
        [netlist.nets().nth(1).unwrap().0, netlist::NetId::GROUND]
    );
    assert!(s1.model.ratings().is_empty());
    assert!(matches!(
        current.current.shape(),
        Shape::Steps(schedule) if schedule.events().is_empty() && schedule.initial().si.get() == 0.001
    ));
    assert!(jumps(&netlist).is_empty());
}

#[test]
fn a_step_and_a_pulse_resolve_on_both_kinds() {
    for (kind, key, unit) in [
        ("voltage_source", "voltage", "V"),
        ("current_source", "current", "A"),
    ] {
        let stepped = parse(&source(kind, key, &step(unit, "3000")));
        assert_eq!(jumps(&stepped), [3000], "{kind}");
        let (_, part) = stepped.parts().next().unwrap();
        let Shape::Step(step) = drive(&part.model).shape() else {
            panic!("{kind}");
        };
        assert_eq!(
            (
                step.initial().si.get(),
                step.final_level().si.get(),
                step.at()
            ),
            (0.0, 5.0, Time::from_ps(3000))
        );

        let pulsed = parse(&source(kind, key, &pulse(unit, "10000", "5000", "40000")));
        assert_eq!(jumps(&pulsed), [10000, 15000, 50000, 55000, 90000, 95000]);
        let (_, part) = pulsed.parts().next().unwrap();
        let Shape::Pulse(pulse) = drive(&part.model).shape() else {
            panic!("{kind}");
        };
        assert_eq!(
            (pulse.delay(), pulse.width(), pulse.period()),
            (Time::from_ps(10000), 5000, 40000)
        );
        assert_eq!(
            (pulse.initial().si.get(), pulse.pulsed().si.get()),
            (0.0002, 0.001)
        );
    }
}

#[test]
fn a_resistor_has_no_next_change() {
    let json = with_parts(
        r#"{ "op": {} }"#,
        r#"{ "name": "r1", "kind": "resistor", "pins": { "a": "a", "b": "gnd" },
             "resistance": { "value": 1.0, "unit": "ohm", "source": "user" }, "ratings": { "power": null } }"#,
    );
    let netlist = parse(&json);
    assert!(
        netlist
            .parts()
            .next()
            .unwrap()
            .1
            .model
            .next_change_after(Time::ZERO)
            .is_none()
    );
}

#[test]
fn equal_levels_still_jump() {
    let flat = pulse("A", "10000", "5000", "40000").replace("0.001", "0.0002");
    assert_eq!(
        jumps(&parse(&source("current_source", "current", &flat))),
        [10000, 15000, 50000, 55000, 90000, 95000]
    );
}

#[test]
fn each_broken_time_names_its_field_and_rule() {
    let waveform = |field: &str, rule: &str, value: u64| {
        format!(
            r##""netlist.waveform" ["#s1"] {{"field":"{field}","rule":"{rule}","value":{value}}}"##
        )
    };
    let cases = [
        (
            source("voltage_source", "voltage", &step("V", "0")),
            vec![waveform("voltage.at_ps", "at_zero", 0)],
        ),
        (
            source(
                "current_source",
                "current",
                &pulse("A", "0", "2000", "4000"),
            ),
            vec![waveform("current.delay_ps", "at_zero", 0)],
        ),
        (
            source(
                "current_source",
                "current",
                &pulse("A", "1", "1999", "4000"),
            ),
            vec![waveform("current.width_ps", "too_close", 1999)],
        ),
        (
            source(
                "current_source",
                "current",
                &pulse("A", "1", "2000", "3999"),
            ),
            vec![waveform("current.period_ps", "too_close", 3999)],
        ),
        (
            source(
                "current_source",
                "current",
                &pulse("A", "1", "5000", "4000"),
            ),
            vec![waveform("current.period_ps", "too_close", 4000)],
        ),
        (
            source(
                "current_source",
                "current",
                &pulse("A", "18446744073709550000", "2000", "4000"),
            ),
            vec![waveform("current.width_ps", "overflow", 2000)],
        ),
        (
            source(
                "voltage_source",
                "voltage",
                &pulse("V", "0", "1000", "1500"),
            ),
            vec![
                waveform("voltage.delay_ps", "at_zero", 0),
                waveform("voltage.width_ps", "too_close", 1000),
                waveform("voltage.period_ps", "too_close", 1500),
            ],
        ),
    ];
    for (json, want) in cases {
        assert_eq!(errors(&json), want, "{json}");
    }
}

#[test]
fn a_level_in_the_wrong_unit_names_its_field() {
    assert_eq!(
        errors(&source(
            "current_source",
            "current",
            &pulse("V", "1", "2000", "4000")
        )),
        [
            r##""netlist.unit" ["#s1"] {"expected":"A","field":"current.initial","found":"V"}"##,
            r##""netlist.unit" ["#s1"] {"expected":"A","field":"current.pulsed","found":"V"}"##,
        ]
    );
    assert_eq!(
        errors(&source("voltage_source", "voltage", &step("A", "1"))),
        [
            r##""netlist.unit" ["#s1"] {"expected":"V","field":"voltage.initial","found":"A"}"##,
            r##""netlist.unit" ["#s1"] {"expected":"V","field":"voltage.final","found":"A"}"##,
        ]
    );
    assert_eq!(
        errors(&source("current_source", "current", &level(1.0, "V"))),
        [r##""netlist.unit" ["#s1"] {"expected":"A","field":"current","found":"V"}"##]
    );
}

#[test]
fn a_malformed_waveform_is_one_syntax_error_with_serde_detail() {
    let good = pulse("A", "1000", "2000", "4000");
    let cases = [
        (
            good.replace("width_ps", "widht_ps"),
            "unknown field `widht_ps`",
        ),
        (
            good.replace(r#", "period_ps": 4000"#, ""),
            "missing field `period_ps`",
        ),
        (
            good.replace("1000,", "1000.5,"),
            "invalid type: floating point `1000.5`",
        ),
        (
            good.replace("1000,", "-1000,"),
            "invalid value: integer `-1000`",
        ),
        (
            good.replace(r#""pulse""#, r#""ramp""#),
            "unknown variant `ramp`",
        ),
        (
            good.replace(r#"{ "kind""#, r#"{ "value": 1.0, "kind""#),
            "unknown field `value`",
        ),
        (
            level(0.001, "A").replace(r#""source""#, r#""at_ps": 5, "source""#),
            "unknown field `at_ps`",
        ),
        (
            level(0.001, "A").replace(r#""unit""#, r#""value": 2.0, "unit""#),
            "duplicate field `value`",
        ),
        (
            good.replace(r#""delay_ps""#, r#""width_ps": 3000, "delay_ps""#),
            "duplicate field `width_ps`",
        ),
    ];
    for (drive, detail) in cases {
        let json = source("current_source", "current", &drive);
        let found = netlist::parse(json.as_bytes()).unwrap_err();
        let [diagnostic] = found.as_slice() else {
            panic!("{found:?}");
        };
        let doc = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(doc["code"], "netlist.syntax", "{drive}");
        let found = doc["data"]["detail"].as_str().unwrap();
        assert!(found.contains(detail), "{found} lacks {detail}");
    }
}

fn steps(unit: &str, initial: f64, events: &str) -> String {
    format!(
        r#"{{ "kind": "steps", "initial": {initial}, "events": [{events}] }}"#,
        initial = level(initial, unit),
        events = events,
    )
}

fn event(t_ps: &str, value: f64, unit: &str) -> String {
    format!(r#"{{ "t_ps": {t_ps}, "level": {} }}"#, level(value, unit))
}

#[test]
fn a_steps_source_holds_initial_then_each_event_and_an_empty_list_is_fixed() {
    for (kind, key, unit) in [
        ("voltage_source", "voltage", "V"),
        ("current_source", "current", "A"),
    ] {
        let body = steps(
            unit,
            0.0,
            &format!("{}, {}", event("4000", 5.0, unit), event("9000", 1.0, unit)),
        );
        let netlist = parse(&source(kind, key, &body));
        assert_eq!(jumps(&netlist), [4000, 9000], "{kind}");
        let (_, part) = netlist.parts().next().unwrap();
        let Shape::Steps(schedule) = drive(&part.model).shape() else {
            panic!("{kind}");
        };
        assert_eq!(schedule.initial().si.get(), 0.0);
        assert_eq!(
            schedule
                .events()
                .iter()
                .map(|event| (event.at.as_ps(), event.to.si.get()))
                .collect::<Vec<_>>(),
            [(4000, 5.0), (9000, 1.0)]
        );

        let fixed = parse(&source(kind, key, &steps(unit, 1.5, "")));
        assert!(jumps(&fixed).is_empty(), "{kind}");
        let (_, part) = fixed.parts().next().unwrap();
        let Shape::Steps(schedule) = drive(&part.model).shape() else {
            panic!("{kind}");
        };
        assert!(schedule.events().is_empty());
        assert_eq!(schedule.initial().si.get(), 1.5);
    }
}

#[test]
fn each_steps_rule_names_its_event_and_a_level_at_the_largest_time_does_not_overflow() {
    let rules = [
        (
            "voltage_source",
            "voltage",
            steps("V", 0.0, &event("0", 5.0, "V")),
            vec![schedule_line(0, "at_zero")],
        ),
        (
            "voltage_source",
            "voltage",
            steps("V", 1.0, &event("2000", 1.0, "V")),
            vec![schedule_line(0, "same_state")],
        ),
        (
            "current_source",
            "current",
            steps(
                "A",
                0.0,
                &format!("{}, {}", event("4000", 1.0, "A"), event("4000", 2.0, "A")),
            ),
            vec![schedule_line(1, "not_increasing")],
        ),
        (
            "current_source",
            "current",
            steps(
                "A",
                0.0,
                &format!("{}, {}", event("9000", 1.0, "A"), event("5000", 2.0, "A")),
            ),
            vec![schedule_line(1, "not_increasing")],
        ),
        (
            "voltage_source",
            "voltage",
            steps(
                "V",
                0.0,
                &format!("{}, {}", event("2000", 5.0, "V"), event("3999", 1.0, "V")),
            ),
            vec![schedule_line(1, "too_close")],
        ),
    ];
    for (kind, key, drive, want) in rules {
        assert_eq!(errors(&source(kind, key, &drive)), want, "{drive}");
    }

    let largest = steps("V", 0.0, &event("18446744073709551615", 5.0, "V"));
    let netlist = parse(&source("voltage_source", "voltage", &largest));
    let (_, part) = netlist.parts().next().unwrap();
    let Shape::Steps(schedule) = drive(&part.model).shape() else {
        panic!("not steps");
    };
    assert_eq!(schedule.events()[0].at, Time::from_ps(u64::MAX));
}

fn schedule_line(event: usize, rule: &str) -> String {
    format!(r##""netlist.schedule" ["#s1"] {{"event":{event},"rule":"{rule}"}}"##)
}

#[test]
fn a_steps_level_message_names_the_field_and_speaks_of_a_level() {
    let same = source(
        "voltage_source",
        "voltage",
        &steps("V", 0.0, &event("2000", -0.0, "V")),
    );
    let doc = serde_json::to_value(&netlist::parse(same.as_bytes()).unwrap_err()[0]).unwrap();
    assert_eq!(doc["code"], "netlist.schedule");
    assert_eq!(doc["data"]["event"], 0);
    assert_eq!(doc["data"]["rule"], "same_state");
    assert!(doc["data"].get("field").is_none());
    assert_eq!(
        doc["message"],
        "#s1 voltage.events[0] does not change the level"
    );

    let close = source(
        "voltage_source",
        "voltage",
        &steps(
            "V",
            0.0,
            &format!("{}, {}", event("2000", 5.0, "V"), event("3999", 1.0, "V")),
        ),
    );
    let doc = serde_json::to_value(&netlist::parse(close.as_bytes()).unwrap_err()[0]).unwrap();
    assert_eq!(
        doc["message"],
        "#s1 voltage.events[1] is less than 2000 ps after the previous event"
    );
    assert_eq!(doc["data"]["rule"], "too_close");
}

#[test]
fn a_wrong_unit_on_a_steps_level_is_reported_and_the_schedule_is_still_checked() {
    assert_eq!(
        errors(&source(
            "voltage_source",
            "voltage",
            &steps("A", 0.0, &event("2000", 5.0, "V")),
        )),
        [r##""netlist.unit" ["#s1"] {"expected":"V","field":"voltage.initial","found":"A"}"##]
    );
    let third = steps(
        "A",
        0.0,
        &format!(
            "{}, {}, {}",
            event("2000", 1.0, "A"),
            event("4000", 2.0, "A"),
            event("6000", 3.0, "V"),
        ),
    );
    assert_eq!(
        errors(&source("current_source", "current", &third)),
        [
            r##""netlist.unit" ["#s1"] {"expected":"A","field":"current.events[2].level","found":"V"}"##
        ]
    );
    let both = steps("kohm", 0.0, &event("0", 5.0, "V"));
    assert_eq!(
        errors(&source("voltage_source", "voltage", &both)),
        [
            r##""netlist.unit" ["#s1"] {"expected":"V","field":"voltage.initial","found":"kohm"}"##,
            r##""netlist.schedule" ["#s1"] {"event":0,"rule":"at_zero"}"##,
        ]
    );
}

#[test]
fn a_malformed_steps_waveform_is_one_syntax_error() {
    let good = steps(
        "V",
        0.0,
        &format!("{}, {}", event("2000", 5.0, "V"), event("4000", 1.0, "V")),
    );
    let cases = [
        (
            good.replace(r#""t_ps": 2000"#, r#""t_ps": 2000.5"#),
            "invalid type: floating point `2000.5`",
        ),
        (
            good.replace(r#""t_ps": 2000"#, r#""t_ps": -2000"#),
            "invalid value: integer `-2000`",
        ),
        (
            good.replace(r#""level""#, r#""value""#),
            "unknown field `value`",
        ),
        (
            format!(r#"{{ "value": 1.0, {} }}"#, good.trim_start_matches('{')),
            "unknown field `value`",
        ),
    ];
    for (drive, detail) in cases {
        let json = source("voltage_source", "voltage", &drive);
        let found = netlist::parse(json.as_bytes()).unwrap_err();
        let [diagnostic] = found.as_slice() else {
            panic!("{drive} {found:?}");
        };
        let doc = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(doc["code"], "netlist.syntax", "{drive}");
        let found = doc["data"]["detail"].as_str().unwrap();
        assert!(found.contains(detail), "{found} lacks {detail}");
    }
}
