mod common;

use common::{errors, with_parts};
use sim_core::netlist::{self, Analysis, Transient};
use sim_core::time::Time;

fn transient(stop_ps: &str, max_step_ps: &str) -> String {
    with_parts(
        &format!(r#"{{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps} }} }}"#),
        "",
    )
}

#[test]
fn a_transient_analysis_keeps_its_stop_and_max_step() {
    let netlist = netlist::parse(transient("81000000000", "20000000").as_bytes()).unwrap();
    assert_eq!(
        netlist.analysis(),
        &Analysis::Transient(Transient {
            stop: Time::from_ps(81_000_000_000),
            max_step: 20_000_000,
        })
    );
    assert!(netlist::parse(transient("1000", "1000").as_bytes()).is_ok());
}

#[test]
fn a_transient_analysis_breaks_each_rule_with_its_field() {
    let cases = [
        (
            "0",
            "1000",
            vec![
                ("stop_ps", "positive", "0"),
                ("max_step_ps", "step_over_stop", "1000"),
            ],
        ),
        (
            "5000",
            "999",
            vec![("max_step_ps", "step_under_1ns", "999")],
        ),
        ("5000", "0", vec![("max_step_ps", "step_under_1ns", "0")]),
        (
            "5000",
            "5001",
            vec![("max_step_ps", "step_over_stop", "5001")],
        ),
    ];
    for (stop, max_step, expected) in cases {
        let expected: Vec<String> = expected
            .iter()
            .map(|(field, rule, value)| {
                format!(
                    r#""netlist.analysis" [] {{"field":"{field}","rule":"{rule}","value":{value}}}"#
                )
            })
            .collect();
        assert_eq!(
            errors(&transient(stop, max_step)),
            expected,
            "{stop} {max_step}"
        );
    }
    for shape in [
        r#"{ "transient": { "stop_ps": 5000 } }"#,
        r#"{ "transient": { "stop_ps": 5000, "max_step_ps": 1000.5 } }"#,
        r#"{ "transient": { "stop_ps": 5000, "max_step_ps": 1000, "min_step_ps": 1 } }"#,
        r#"{ "op": {}, "transient": { "stop_ps": 5000, "max_step_ps": 1000 } }"#,
    ] {
        let err = netlist::parse(with_parts(shape, "").as_bytes()).unwrap_err();
        assert_eq!(err[0].code(), "netlist.syntax", "{shape}");
    }
}
