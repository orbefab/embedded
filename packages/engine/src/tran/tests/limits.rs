use crate::netlist::Netlist;

use super::super::*;
use super::fixtures::*;
use crate::session::Budget;

/// 5 V pulsed onto 10 ohm from 3 ns for 2 ns, on a 10 ns grid. No row sees it. 2.5 W for
/// 2 ns averages about 4.9e-7 W over 10.2 ms, and the rating sits under that share.
fn pulse_between_rows() -> Netlist {
    crate::netlist::parse(
        br#"{
          "version": 1,
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
                "initial": { "value": 0.0, "unit": "V" },
                "pulsed": { "value": 5.0, "unit": "V" },
                "delay_ps": 3000, "width_ps": 2000, "period_ps": 1000000
              }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "vcc", "b": "gnd" },
              "resistance": { "value": 10.0, "unit": "ohm" },
              "ratings": { "power": { "value": 1e-7, "unit": "W" } }
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
    // fall at 5 ns. The rows stay at 0 V, so this average is the pulse the steps between them
    // folded in: 2.5 W for 2 ns over the 10.2 ms window.
    assert_eq!(excursion.window_ps, Some(10_200_000_000));
    let share = 2.5 * 2_000.0 / 10_200_000_000.0;
    assert!(
        (excursion.peak - share).abs() <= 0.05 * share,
        "{}",
        excursion.peak
    );
    assert!(excursion.peak > excursion.limit);
    assert!(excursion.first.as_ps() > 3_000, "{:?}", excursion.first);
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
            r#"{{"version":1,"seed":0,"analysis":{{"transient":{{"stop_ps":{stop},"max_step_ps":1000,"grid_ps":1000}}}},"ground":"gnd","nets":["gnd","vcc"],"parts":[{{"name":"v1","kind":"voltage_source","pins":{{"p":"vcc","n":"gnd"}},"voltage":{{"value":1.0,"unit":"V"}}}}"#
        ));
        for index in 0..RESISTORS {
            json.push_str(&format!(
                r#",{{"name":"r{index}","kind":"resistor","pins":{{"a":"vcc","b":"gnd"}},"resistance":{{"value":1000.0,"unit":"ohm"}},"ratings":{{"power":null}}}}"#
            ));
        }
        json.push_str("]}");
        crate::netlist::parse(json.as_bytes()).unwrap()
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
        r#"{{"version":1,"seed":0,"analysis":{{"transient":{{"stop_ps":{stop},"max_step_ps":1000,"grid_ps":{stop}}}}},"ground":"gnd","nets":["gnd","vcc"],"parts":[{{"name":"v1","kind":"voltage_source","pins":{{"p":"vcc","n":"gnd"}},"voltage":{{"value":1.0,"unit":"V"}}}}"#
    ));
    for index in 0..resistors {
        json.push_str(&format!(
            r#",{{"name":"r{index}","kind":"resistor","pins":{{"a":"vcc","b":"gnd"}},"resistance":{{"value":1000.0,"unit":"ohm"}},"ratings":{{"power":null}}}}"#
        ));
    }
    json.push_str("]}");
    crate::netlist::parse(json.as_bytes()).unwrap()
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
    crate::netlist::parse(
        br#"{
          "version": 1,
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
                "initial": { "value": 0.0, "unit": "V" },
                "pulsed": { "value": 1.0, "unit": "V" },
                "delay_ps": 10000, "width_ps": 5000, "period_ps": 10000
              }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "vin", "b": "gnd" },
              "resistance": { "value": 1000.0, "unit": "ohm" },
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
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget {
            steps: 30,
            ..BUDGET
        },
        Rows::Grid,
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

/// Eight steps of the dense pulse end off the 20 ms grid, on a low level after interior
/// highs. The failed run stores that last accepted step as its last row, so the interval
/// ending there covers the steps folded after the previous row, and only those steps reach 1 V.
#[test]
fn a_failed_runs_last_interval_ends_at_its_off_grid_last_step() {
    let netlist = dense_pulse();
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget { steps: 8, ..BUDGET },
        Rows::Grid,
        &mut diagnostics,
    );
    let end = reached(&outcome);
    let trace = outcome.trace.expect("the run kept its rows");
    assert_eq!(end, trace.end().unwrap());
    assert_ne!(end.as_ps() % 20_000_000, 0, "the last step is off the grid");
    assert_eq!(trace.intervals(), trace.rows() - 1);
    for j in 0..trace.columns().len() {
        let (values, mins, maxs) = (trace.column(j), trace.min_column(j), trace.max_column(j));
        for k in 0..trace.intervals() {
            assert!(mins[k] <= values[k].min(values[k + 1]));
            assert!(maxs[k] >= values[k].max(values[k + 1]));
        }
    }
    // The steps run takes the same steps, so its rows are this run's samples.
    let mut step_diagnostics = Vec::new();
    let steps = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget { steps: 8, ..BUDGET },
        Rows::Steps,
        &mut step_diagnostics,
    );
    assert_eq!(
        step_diagnostics, diagnostics,
        "both runs stop at the same step"
    );
    let step_trace = steps.trace.expect("the steps run kept its rows");
    assert!(step_trace.rows() > trace.rows());
    let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    let step_t: Vec<u64> = step_trace.time().iter().map(|t| t.as_ps()).collect();
    for j in 0..trace.columns().len() {
        let step_values = step_trace.column(j);
        for (k, &at) in rows.iter().enumerate().skip(1) {
            let from = step_t.partition_point(|&t| t < rows[k - 1]);
            let to = step_t.partition_point(|&t| t <= at);
            let spanned = &step_values[from..to];
            let low = spanned.iter().copied().fold(f64::INFINITY, f64::min);
            let high = spanned.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(trace.min_column(j)[k - 1], low, "interval {k}");
            assert_eq!(trace.max_column(j)[k - 1], high, "interval {k}");
        }
    }
    // The rows hold the level before each jump. The last interval's samples include the pulse.
    let last = trace.intervals() - 1;
    let vin = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == "#net.vin")
        .expect("#net.vin");
    let (last_min, last_max) = (trace.min_column(vin)[last], trace.max_column(vin)[last]);
    assert!(last_max - last_min >= 1.0 - 1e-12, "{last_min} {last_max}");
}

/// In step-rows mode every accepted step is a row, so each interval's samples are exactly
/// its two rows.
#[test]
fn in_step_rows_mode_each_interval_holds_exactly_its_two_rows() {
    let netlist = pulse_between_rows();
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        BUDGET,
        Rows::Steps,
        &mut diagnostics,
    );
    let trace = outcome.trace.expect("the steps run keeps its rows");
    assert_eq!(trace.intervals(), trace.rows() - 1);
    for j in 0..trace.columns().len() {
        let (values, mins, maxs) = (trace.column(j), trace.min_column(j), trace.max_column(j));
        for (k, (low, high)) in values.iter().zip(&values[1..]).enumerate() {
            assert_eq!(mins[k], low.min(*high));
            assert_eq!(maxs[k], low.max(*high));
        }
    }
}

#[test]
fn a_rating_survives_a_convergence_failure() {
    let netlist = fast_discharge(true);
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget { step: 1, ..BUDGET },
        Rows::Grid,
        &mut diagnostics,
    );
    assert!(reached(&outcome) < transient(&netlist).stop);
    let codes: Vec<_> = diagnostics.iter().map(|d| d.code()).collect();
    assert_eq!(codes, ["rating.power", "solver.convergence"]);
}
