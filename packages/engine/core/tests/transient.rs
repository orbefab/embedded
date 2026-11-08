mod common;

use common::{errors, with_parts};
use orbe_engine_core::netlist::{self, Analysis, Transient};
use orbe_engine_core::time::Time;

fn transient(stop_ps: &str, max_step_ps: &str, grid_ps: &str) -> String {
    with_parts(
        &format!(
            r#"{{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": {max_step_ps}, "grid_ps": {grid_ps} }} }}"#
        ),
        "",
    )
}

#[test]
fn a_transient_analysis_keeps_its_stop_max_step_and_grid() {
    let netlist =
        netlist::parse(transient("81000000000", "20000000", "1000000000").as_bytes()).unwrap();
    assert_eq!(
        netlist.analysis(),
        &Analysis::Transient(Transient {
            stop: Time::from_ps(81_000_000_000),
            max_step: 20_000_000,
            grid: 1_000_000_000,
        })
    );
    assert!(netlist::parse(transient("1000", "1000", "1000").as_bytes()).is_ok());
}

#[test]
fn a_transient_analysis_breaks_each_rule_with_its_field() {
    let cases = [
        (
            ("0", "1000", "1000"),
            vec![
                ("stop_ps", "positive", "0"),
                ("max_step_ps", "step_over_stop", "1000"),
                ("grid_ps", "grid_over_stop", "1000"),
            ],
        ),
        (
            ("5000", "999", "1000"),
            vec![("max_step_ps", "step_under_1ns", "999")],
        ),
        (
            ("5000", "0", "1000"),
            vec![("max_step_ps", "step_under_1ns", "0")],
        ),
        (
            ("5000", "5001", "1000"),
            vec![("max_step_ps", "step_over_stop", "5001")],
        ),
        (
            ("5000", "1000", "999"),
            vec![("grid_ps", "grid_under_1ns", "999")],
        ),
        (
            ("5000", "1000", "5001"),
            vec![("grid_ps", "grid_over_stop", "5001")],
        ),
    ];
    for ((stop, max_step, grid), expected) in cases {
        let expected: Vec<String> = expected
            .iter()
            .map(|(field, rule, value)| {
                format!(
                    r#""netlist.analysis" [] {{"field":"{field}","rule":"{rule}","value":{value}}}"#
                )
            })
            .collect();
        assert_eq!(
            errors(&transient(stop, max_step, grid)),
            expected,
            "{stop} {max_step} {grid}"
        );
    }
    for shape in [
        r#"{ "transient": { "stop_ps": 5000, "max_step_ps": 1000 } }"#,
        r#"{ "transient": { "stop_ps": 5000, "grid_ps": 1000 } }"#,
        r#"{ "transient": { "stop_ps": 5000, "max_step_ps": 1000, "grid_ps": 1000.5 } }"#,
        r#"{ "transient": { "stop_ps": 5000, "max_step_ps": 1000, "grid_ps": 1000, "min_step_ps": 1 } }"#,
        r#"{ "op": {}, "transient": { "stop_ps": 5000, "max_step_ps": 1000, "grid_ps": 1000 } }"#,
    ] {
        let err = netlist::parse(with_parts(shape, "").as_bytes()).unwrap_err();
        assert_eq!(err[0].code(), "netlist.syntax", "{shape}");
    }
}

#[test]
fn rows_are_every_multiple_of_the_grid_before_stop_and_stop() {
    let transient = |stop, grid| Transient {
        stop: Time::from_ps(stop),
        max_step: 1_000,
        grid,
    };
    let odd = transient(5_500, 2_000);
    assert_eq!(odd.rows(), 4);
    let rows: Vec<u64> = (0..=5_500)
        .filter(|&at| odd.is_row(Time::from_ps(at)))
        .collect();
    assert_eq!(rows, [0, 2_000, 4_000, 5_500]);
    assert_eq!(odd.next_row_after(Time::ZERO), Time::from_ps(2_000));
    assert_eq!(
        odd.next_row_after(Time::from_ps(2_000)),
        Time::from_ps(4_000)
    );
    assert_eq!(
        odd.next_row_after(Time::from_ps(4_001)),
        Time::from_ps(5_500)
    );
    let even = transient(6_000, 2_000);
    assert_eq!(even.rows(), 4);
    assert_eq!(
        even.next_row_after(Time::from_ps(4_000)),
        Time::from_ps(6_000)
    );
    assert_eq!(transient(1_000, 1_000).rows(), 2);
}
