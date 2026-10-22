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
    assert!(matches!(current.current.shape(), Shape::Dc(dc) if dc.si.get() == 0.001));
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
