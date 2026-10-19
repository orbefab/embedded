use sim_core::netlist::Netlist;

use super::super::*;
use super::fixtures::*;

#[test]
fn a_trace_past_its_row_limit_ends_the_run_and_keeps_the_rows() {
    let netlist = led_switch();
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        BUDGET,
        4,
        &columns,
        &mut diagnostics,
    );
    let trace = outcome.trace.unwrap();
    assert_eq!(trace.rows(), 4);
    let times: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    assert_eq!(times, [0, 1_000, 3_000, 7_000]);
    assert!(outcome.observations.op.is_none());
    let codes: Vec<_> = diagnostics.iter().map(|d| (d.code(), d.at())).collect();
    assert_eq!(trace.end(), Some(Time::from_ps(7_000)));
    assert_eq!(codes, [("trace.too_large", Some(Time::from_ps(15_000)))]);
}

fn overloaded_source() -> Netlist {
    sim_core::netlist::parse(
        br#"{
          "version": 0,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 100000, "max_step_ps": 1000 } },
          "ground": "gnd",
          "nets": ["gnd", "vcc"],
          "parts": [
            {
              "name": "v1",
              "kind": "voltage_source",
              "pins": { "p": "vcc", "n": "gnd" },
              "voltage": { "value": 5.0, "unit": "V", "source": "user" }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "vcc", "b": "gnd" },
              "resistance": { "value": 10.0, "unit": "ohm", "source": "user" },
              "ratings": { "power": { "value": 0.001, "unit": "W", "source": "user" } }
            }
          ]
        }"#,
    )
    .unwrap()
}

#[test]
fn a_rating_survives_the_row_cap() {
    let netlist = overloaded_source();
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        BUDGET,
        4,
        &columns,
        &mut diagnostics,
    );
    assert!(outcome.observations.op.is_none());
    assert_eq!(outcome.trace.unwrap().rows(), 4);
    let codes: Vec<_> = diagnostics.iter().map(|d| d.code()).collect();
    assert_eq!(codes, ["rating.power", "trace.too_large"]);
    assert_eq!(diagnostics[0].at(), Some(Time::ZERO));
}

#[test]
fn the_value_cap_binds_before_the_row_cap() {
    const RESISTORS: usize = 4_000;
    let columns = 2 + 3 * (RESISTORS + 1);
    let limit = Trace::row_limit(columns);
    assert!(limit < Trace::MAX_ROWS, "{limit}");
    assert_eq!(limit, Trace::MAX_VALUES / columns);
    let stop = limit as u64 * 1_000;
    let mut json = String::with_capacity(RESISTORS * 160 + 800);
    json.push_str(&format!(
        r#"{{"version":0,"seed":0,"analysis":{{"transient":{{"stop_ps":{stop},"max_step_ps":1000}}}},"ground":"gnd","nets":["gnd","vcc"],"parts":[{{"name":"v1","kind":"voltage_source","pins":{{"p":"vcc","n":"gnd"}},"voltage":{{"value":1.0,"unit":"V","source":"user"}}}}"#
    ));
    for index in 0..RESISTORS {
        json.push_str(&format!(
            r#",{{"name":"r{index}","kind":"resistor","pins":{{"a":"vcc","b":"gnd"}},"resistance":{{"value":1000.0,"unit":"ohm","source":"user"}},"ratings":{{"power":null}}}}"#
        ));
    }
    json.push_str("]}");
    let netlist = sim_core::netlist::parse(json.as_bytes()).unwrap();
    let mut diagnostics = Vec::new();
    let outcome = run(&netlist, &transient(&netlist), &mut diagnostics);
    assert!(outcome.observations.op.is_none());
    assert_eq!(outcome.trace.unwrap().rows(), limit);
    assert_eq!(diagnostics.len(), 1);
    let found = serde_json::to_value(&diagnostics[0]).unwrap();
    assert_eq!(found["code"], "trace.too_large");
    assert_eq!(found["data"]["limit"], limit);
    assert_eq!(found["data"]["columns"], columns);
}

#[test]
fn a_rating_survives_a_convergence_failure() {
    let netlist = fast_discharge(true);
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Budget { step: 1, ..BUDGET },
        Trace::MAX_ROWS,
        &columns,
        &mut diagnostics,
    );
    assert!(outcome.observations.op.is_none());
    assert!(outcome.trace.unwrap().rows() > 0);
    let codes: Vec<_> = diagnostics.iter().map(|d| d.code()).collect();
    assert_eq!(codes, ["rating.power", "solver.convergence"]);
}
