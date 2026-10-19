use sim_core::netlist::Netlist;

use super::super::*;
use super::fixtures::*;

/// 5 V pulsed onto 10 ohm rated 1 mW, from 3 ns for 2 ns, on a 10 ns grid. No row sees it.
fn pulse_between_rows() -> Netlist {
    sim_core::netlist::parse(
        br#"{
          "version": 0,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 20000, "max_step_ps": 10000, "grid_ps": 10000 } },
          "ground": "gnd",
          "nets": ["gnd", "vcc"],
          "parts": [
            {
              "name": "v1",
              "kind": "voltage_source",
              "pins": { "p": "vcc", "n": "gnd" },
              "voltage": {
                "kind": "pulse",
                "initial": { "value": 0.0, "unit": "V", "source": "user" },
                "pulsed": { "value": 5.0, "unit": "V", "source": "user" },
                "delay_ps": 3000, "width_ps": 2000, "period_ps": 1000000
              }
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
fn a_rating_samples_every_step_between_the_rows() {
    let netlist = pulse_between_rows();
    let rated = |rows| {
        let mut diagnostics = Vec::new();
        let outcome = run(&netlist, &transient(&netlist), rows, &mut diagnostics);
        let times: Vec<u64> = outcome
            .trace
            .unwrap()
            .time()
            .iter()
            .map(|t| t.as_ps())
            .collect();
        (diagnostics, times)
    };
    let (grid, rows) = rated(Rows::Grid);
    let (steps, _) = rated(Rows::Steps);
    assert_eq!(rows, [0, 10_000, 20_000]);
    assert_eq!(grid, steps);
    let [Diagnostic::RatingPower(excursion)] = grid.as_slice() else {
        panic!("{grid:?}");
    };
    // The jump at 3 ns holds the old level, and the step 1 ns later the new one, through the
    // fall at 5 ns.
    assert_eq!(excursion.first, Time::from_ps(4_000));
    assert_eq!(excursion.peak_at, Time::from_ps(4_000));
    assert_eq!(excursion.over_ps, Some(2_000));
    assert!((excursion.peak - 2.5).abs() < 1e-9, "{}", excursion.peak);
}

#[test]
fn the_value_cap_binds_before_the_row_cap() {
    const RESISTORS: usize = 4_000;
    let columns = 2 + 3 * (RESISTORS + 1);
    let limit = Trace::row_limit(columns);
    assert!(limit < Trace::MAX_ROWS, "{limit}");
    assert_eq!(limit, Trace::MAX_VALUES / columns);
    let netlist = |rows: usize| {
        let stop = (rows as u64 - 1) * 1_000;
        let mut json = String::with_capacity(RESISTORS * 160 + 800);
        json.push_str(&format!(
            r#"{{"version":0,"seed":0,"analysis":{{"transient":{{"stop_ps":{stop},"max_step_ps":1000,"grid_ps":1000}}}},"ground":"gnd","nets":["gnd","vcc"],"parts":[{{"name":"v1","kind":"voltage_source","pins":{{"p":"vcc","n":"gnd"}},"voltage":{{"value":1.0,"unit":"V","source":"user"}}}}"#
        ));
        for index in 0..RESISTORS {
            json.push_str(&format!(
                r#",{{"name":"r{index}","kind":"resistor","pins":{{"a":"vcc","b":"gnd"}},"resistance":{{"value":1000.0,"unit":"ohm","source":"user"}},"ratings":{{"power":null}}}}"#
            ));
        }
        json.push_str("]}");
        sim_core::netlist::parse(json.as_bytes()).unwrap()
    };
    let at_limit = netlist(limit);
    let mut diagnostics = Vec::new();
    let outcome = run(
        &at_limit,
        &transient(&at_limit),
        Rows::Grid,
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(outcome.trace.unwrap().rows(), limit);
    let over = netlist(limit + 1);
    let outcome = run(&over, &transient(&over), Rows::Grid, &mut diagnostics);
    assert!(outcome.trace.is_none());
    let [refused] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let found = serde_json::to_value(refused).unwrap();
    assert_eq!(found["code"], "trace.too_large");
    assert_eq!(found["t_ps"], serde_json::Value::Null);
    assert_eq!(found["data"]["rows"], limit + 1);
    assert_eq!(found["data"]["limit"], limit);
    assert_eq!(found["data"]["columns"], columns);
}

/// The value cap binds. The grid is one interval, so the up-front row check passes.
/// `stop` is `max_step` times the row limit.
fn wide_source(resistors: usize, stop: u64) -> Netlist {
    let mut json = String::with_capacity(resistors * 160 + 800);
    json.push_str(&format!(
        r#"{{"version":0,"seed":0,"analysis":{{"transient":{{"stop_ps":{stop},"max_step_ps":1000,"grid_ps":{stop}}}}},"ground":"gnd","nets":["gnd","vcc"],"parts":[{{"name":"v1","kind":"voltage_source","pins":{{"p":"vcc","n":"gnd"}},"voltage":{{"value":1.0,"unit":"V","source":"user"}}}}"#
    ));
    for index in 0..resistors {
        json.push_str(&format!(
            r#",{{"name":"r{index}","kind":"resistor","pins":{{"a":"vcc","b":"gnd"}},"resistance":{{"value":1000.0,"unit":"ohm","source":"user"}},"ratings":{{"power":null}}}}"#
        ));
    }
    json.push_str("]}");
    sim_core::netlist::parse(json.as_bytes()).unwrap()
}

#[test]
fn steps_past_the_row_limit_keep_the_rows_and_report_trace_too_large() {
    const RESISTORS: usize = 4_000;
    let columns = 2 + 3 * (RESISTORS + 1);
    let limit = Trace::row_limit(columns);
    assert!(limit < Trace::MAX_ROWS, "{limit}");
    let stop = limit as u64 * 1_000;
    let netlist = wide_source(RESISTORS, stop);
    assert!(transient(&netlist).rows() <= limit as u64);
    let mut diagnostics = Vec::new();
    let outcome = run(
        &netlist,
        &transient(&netlist),
        Rows::Steps,
        &mut diagnostics,
    );
    let end = reached(&outcome);
    assert_eq!(end, Time::from_ps(stop - 1_000));
    let trace = outcome.trace.unwrap();
    assert_eq!(trace.rows(), limit);
    let [refused] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let found = serde_json::to_value(refused).unwrap();
    assert_eq!(found["code"], "trace.too_large");
    assert_eq!(found["t_ps"], serde_json::json!(stop));
    assert_eq!(
        found["message"],
        format!("the trace holds {limit} rows; the step at {stop} ps did not fit")
    );
    assert_eq!(found["refs"], serde_json::json!([]));
    assert_eq!(found["data"]["rows"], limit + 1);
    assert_eq!(found["data"]["limit"], limit);
    assert_eq!(found["data"]["columns"], columns);
}

/// 100 MHz over 1 s on a 20 ms grid. The grid is 50,001 rows, under the row cap.
fn dense_pulse() -> Netlist {
    sim_core::netlist::parse(
        br#"{
          "version": 0,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 1000000000000, "max_step_ps": 20000000, "grid_ps": 20000000 } },
          "ground": "gnd",
          "nets": ["gnd", "vin"],
          "parts": [
            {
              "name": "v1",
              "kind": "voltage_source",
              "pins": { "p": "vin", "n": "gnd" },
              "voltage": {
                "kind": "pulse",
                "initial": { "value": 0.0, "unit": "V", "source": "user" },
                "pulsed": { "value": 1.0, "unit": "V", "source": "user" },
                "delay_ps": 10000, "width_ps": 5000, "period_ps": 10000
              }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "vin", "b": "gnd" },
              "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
              "ratings": { "power": null }
            }
          ]
        }"#,
    )
    .unwrap()
}

#[test]
fn a_dense_pulse_stops_at_the_step_limit_and_keeps_what_it_reached() {
    let netlist = dense_pulse();
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Budget {
            steps: 30,
            ..BUDGET
        },
        Rows::Grid,
        &columns,
        &mut diagnostics,
    );
    let end = reached(&outcome);
    assert!(end > Time::ZERO);
    assert!(end < transient(&netlist).stop);
    let [failed] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let text = serde_json::to_string(failed).unwrap();
    assert_eq!(
        text,
        format!(
            r#"{{"code":"solver.step_limit","severity":"error","refs":[],"t_ps":{},"message":"the run accepted 30 steps, its limit; dense switch or source schedules cost steps, not rows","data":{{"limit":30}}}}"#,
            end.as_ps()
        )
    );
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
        Rows::Grid,
        &columns,
        &mut diagnostics,
    );
    assert!(reached(&outcome) < transient(&netlist).stop);
    let codes: Vec<_> = diagnostics.iter().map(|d| d.code()).collect();
    assert_eq!(codes, ["rating.power", "solver.convergence"]);
}
