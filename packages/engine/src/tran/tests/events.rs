use crate::netlist::Netlist;

use super::super::*;
use super::fixtures::*;
use crate::session::Budget;
use crate::session::State;

/// 100 kohm and 1 uF from 5 V hold an NE555's RESET low at power-on, and its release at
/// about 15.1 ms sets the latch. The NE555 has `packages/engine/tests/circuits/ne555-led`'s values, as in
/// `tests/chip.rs`. `cr` starts at `rst` V. `step_ps` adds a 0 to 1 V step into 1 kohm and
/// 1 nF at that time.
fn reset_release(step_ps: Option<u64>, rst: f64) -> Netlist {
    let value =
        |v: f64, unit: &str| format!(r#"{{ "value": {v:?}, "unit": "{unit}", "source": "user" }}"#);
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": 40000000000, "max_step_ps": 1000000000, "grid_ps": 1000000000 }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "rst", "out", "cont", "dis"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {} }},
            {{ "name": "rr", "kind": "resistor", "pins": {{ "a": "vcc", "b": "rst" }},
              "resistance": {}, "ratings": {{ "power": null }} }},
            {{ "name": "cr", "kind": "capacitor", "pins": {{ "a": "rst", "b": "gnd" }},
              "capacitance": {}, "initial": {} }},
            {{ "name": "u1", "kind": "ne555",
              "pins": {{ "gnd": "gnd", "trig": "gnd", "out": "out", "reset": "rst",
                        "cont": "cont", "thres": "gnd", "disch": "dis", "vcc": "vcc" }},
              "ladder_resistance": {}, "reset_threshold": {},
              "discharge": {{ "on_resistance": {}, "off_resistance": {} }},
              "supply_resistance": {},
              "output": {{ "high_drop": {}, "high_resistance": {}, "low_drop": {}, "low_resistance": {} }} }}
          ]
        }}"#,
        value(5.0, "V"),
        value(100e3, "ohm"),
        value(1e-6, "F"),
        value(rst, "V"),
        value(5000.0, "ohm"),
        value(0.7, "V"),
        value(18.75, "ohm"),
        value(2.5e8, "ohm"),
        value(1875.0, "ohm"),
        value(0.9, "V"),
        value(8.0, "ohm"),
        value(1.0 / 60.0, "V"),
        value(50.0 / 3.0, "ohm"),
    );
    let json = match step_ps {
        None => json,
        Some(at) => {
            let json = json.replacen(r#""dis"]"#, r#""dis", "src", "vs"]"#, 1);
            let side = format!(
                r#",{{ "name": "vs", "kind": "voltage_source", "pins": {{ "p": "src", "n": "gnd" }},
                  "voltage": {{ "kind": "step", "initial": {}, "final": {}, "at_ps": {at} }} }},
                {{ "name": "rs", "kind": "resistor", "pins": {{ "a": "src", "b": "vs" }},
                  "resistance": {}, "ratings": {{ "power": null }} }},
                {{ "name": "cs", "kind": "capacitor", "pins": {{ "a": "vs", "b": "gnd" }},
                  "capacitance": {}, "initial": {} }}"#,
                value(0.0, "V"),
                value(1.0, "V"),
                value(1000.0, "ohm"),
                value(1e-9, "F"),
                value(0.0, "V"),
            );
            let close = json.rfind(']').unwrap();
            let mut json = json;
            json.insert_str(close, &side);
            json
        }
    };
    crate::netlist::parse(json.as_bytes()).unwrap()
}

/// One Newton iteration converges only where nothing moves, and a probe is a shorter step.
#[test]
fn a_stalled_probe_ends_the_run_and_names_the_probe() {
    let netlist = reset_release(None, 0.0);
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget { probe: 1, ..BUDGET },
        Rows::Grid,
        &mut diagnostics,
    );
    let end = reached(&outcome).as_ps();
    assert!(outcome.observations.logic().is_some());
    let [failed] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let at = failed.at().unwrap().as_ps();
    assert!(
        at > end,
        "the probe at {at} ps is after the last row at {end} ps"
    );
    let text = serde_json::to_string(failed).unwrap();
    assert_eq!(
        text,
        format!(
            r##"{{"code":"solver.convergence","severity":"error","refs":["#net.rst"],"t_ps":{at},"message":"Newton did not converge at #net.rst after 1 iterations in the probe at {at} ps locating a chip event","data":{{"gmin":null,"iterations":1,"source_scale":1.0,"step_ps":{}}}}}"##,
            at - end
        )
    );
}

/// `netlist` on a grid of `grid` ps with each row's solve held to `row` Newton iterations.
fn with_row_budget(netlist: &Netlist, grid: u64, row: u32) -> (Outcome, Vec<Diagnostic>) {
    let transient = Transient {
        grid,
        ..transient(netlist)
    };
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        netlist,
        &transient,
        Vec::new(),
        Budget { row, ..BUDGET },
        Rows::Grid,
        &mut diagnostics,
    );
    (outcome, diagnostics)
}

/// In this fixture no row is within Newton's tolerance of either start after one iteration, so
/// every row no step lands on fails twice. Each is the line between the accepted steps around
/// it, with a warning, and the run reaches `stop`.
/// A grid with no row inside a step takes the same steps and warns of nothing.
/// The latch's set feeds no capacitor, so the step from it is long and one row falls inside. That
/// line starts at the restart solve, which is the operating point with the latch set and `cr`
/// held where the step left it.
#[test]
fn a_row_that_fails_twice_is_the_line_between_its_steps() {
    let netlist = reset_release(None, 0.0);
    let committed = transient(&netlist);
    let (gridded, lines) = with_row_budget(&netlist, committed.grid, 1);
    let (gridless, none) = with_row_budget(&netlist, committed.stop.as_ps(), 1);
    assert_eq!(none, []);
    assert_eq!(gridded.stats, gridless.stats);
    assert_eq!(
        serde_json::to_value(&gridded.observations).unwrap(),
        serde_json::to_value(&gridless.observations).unwrap()
    );
    assert_eq!(reached(&gridded), committed.stop);

    let steps = run_steps(&netlist);
    assert_eq!(gridded.stats.landings, steps.rows() as u64 - 1);
    let column = |name: &str| {
        steps
            .columns()
            .iter()
            .position(|r| r.to_string() == name)
            .unwrap()
    };
    let (out, rst) = (column("#net.out"), column("#net.rst"));
    let trace = gridded.trace.unwrap();
    let off_steps = trace
        .time()
        .iter()
        .filter(|at| steps.time().binary_search(at).is_err())
        .count();
    assert_eq!(lines.len(), off_steps);
    let mut from_restart = 0;
    for line in &lines {
        let at = line.at().unwrap();
        let k = steps.time().partition_point(|&t| t < at) - 1;
        let (from, to) = (steps.time()[k], steps.time()[k + 1]);
        assert_eq!(
            serde_json::to_value(line).unwrap(),
            serde_json::json!({
                "code": "trace.row_interpolated",
                "severity": "warning",
                "refs": [],
                "t_ps": at,
                "message": format!(
                    "the row at {} ps did not solve; it is the line between the steps at {} ps and {} ps",
                    at.as_ps(),
                    from.as_ps(),
                    to.as_ps()
                ),
                "data": { "from_ps": from, "to_ps": to },
            })
        );
        let share = (at.as_ps() - from.as_ps()) as f64 / (to.as_ps() - from.as_ps()) as f64;
        let row = trace.time().binary_search(&at).unwrap();
        let set = steps.column(out)[k] < 2.5 && steps.column(out)[k + 1] > 2.5;
        let restart = set.then(|| run_steps(&reset_release(None, steps.column(rst)[k])));
        from_restart += usize::from(set);
        for (j, quantity) in trace.columns().iter().enumerate() {
            let a = match &restart {
                Some(restart) => restart.column(j)[0],
                None => steps.column(j)[k],
            };
            let b = steps.column(j)[k + 1];
            let want = a + (b - a) * share;
            let got = trace.column(j)[row];
            assert!(
                (got - want).abs() <= 1e-15 * a.abs().max(b.abs()),
                "{quantity} at {at:?}: {got}, not {want}"
            );
        }
    }
    assert_eq!(from_restart, 1);
}

/// Every probe is the run's row at its time, and only a probe between steps can be the line.
#[test]
fn a_probe_that_fails_twice_is_the_line_and_says_so() {
    let netlist = reset_release(None, 0.0);
    let (outcome, lines) = with_row_budget(&netlist, transient(&netlist).grid, 1);
    let trace = outcome.trace.unwrap();
    let mut session = open(&netlist, Vec::new(), Budget { row: 1, ..BUDGET });
    let mut interpolated = 0;
    for (k, &at) in trace.time().iter().enumerate() {
        session.advance_to(at);
        let probe = session.probe();
        let line = lines.iter().any(|line| line.at() == Some(at));
        assert_eq!(probe.interpolated(), line, "{at:?}");
        interpolated += usize::from(line);
        for (j, value) in probe.values().iter().enumerate() {
            assert_eq!(
                value.get().to_bits(),
                trace.column(j)[k].to_bits(),
                "{} at {at:?}",
                trace.columns()[j]
            );
        }
    }
    assert!(interpolated > 0 && interpolated < trace.rows());
}

/// `voltage` through 1 kohm into 1 nF from `initial` V, with `current` into the capacitor
/// and a switch `sw` across it, 1 kohm closed, on `contact`. Over 20 ns with a row at 10.5 ns.
fn source_into_rc(voltage: &str, current: &str, contact: &str, initial: f64) -> Netlist {
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": 20000, "max_step_ps": 20000, "grid_ps": 10500 }} }},
          "ground": "gnd",
          "nets": ["gnd", "src", "vout"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "src", "n": "gnd" }},
              "voltage": {voltage} }},
            {{ "name": "i1", "kind": "current_source", "pins": {{ "p": "vout", "n": "gnd" }},
              "current": {current} }},
            {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "src", "b": "vout" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }},
            {{ "name": "c1", "kind": "capacitor", "pins": {{ "a": "vout", "b": "gnd" }},
              "capacitance": {{ "value": 1e-9, "unit": "F", "source": "user" }},
              "initial": {{ "value": {initial:?}, "unit": "V", "source": "user" }} }},
            {{ "name": "sw", "kind": "switch", "pins": {{ "a": "vout", "b": "gnd" }},
              "on_resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
              {contact}, "bounce": null }}
          ]
        }}"#
    );
    crate::netlist::parse(json.as_bytes()).unwrap()
}

/// A row inside the step after an edge that fails twice is the line from the restart, with the
/// current source and the switch read after the edge. The restart is the operating point of the
/// circuit after the edge with the capacitor held at its voltage at the edge.
#[test]
fn a_row_after_a_restart_is_the_line_from_the_restart() {
    let step = |unit: &str, last: f64| {
        format!(
            r#"{{ "kind": "step", "initial": {{ "value": 0.0, "unit": "{unit}", "source": "user" }},
            "final": {{ "value": {last:?}, "unit": "{unit}", "source": "user" }}, "at_ps": 10000 }}"#
        )
    };
    let dc = |unit: &str, value: f64| {
        format!(r#"{{ "value": {value:?}, "unit": "{unit}", "source": "user" }}"#)
    };
    let netlist = source_into_rc(
        &step("V", 5.0),
        &step("A", 1e-3),
        r#""initial": "open", "events": [{ "t_ps": 10000, "state": "closed" }]"#,
        2.0,
    );
    let steps = run_steps(&netlist);
    let edge = steps.time().binary_search(&Time::from_ps(10_000)).unwrap();
    let (edge_at, to) = (steps.time()[edge], steps.time()[edge + 1]);
    assert_eq!(to, Time::from_ps(10_000 + EDGE_STEP));
    let column = |name: &str| {
        steps
            .columns()
            .iter()
            .position(|r| r.to_string() == name)
            .unwrap()
    };
    let vout = steps.column(column("#net.vout"))[edge];
    let restart = run_steps(&source_into_rc(
        &dc("V", 5.0),
        &dc("A", 1e-3),
        r#""initial": "closed", "events": []"#,
        vout,
    ));

    let (outcome, lines) = with_row_budget(&netlist, 10_500, 1);
    let [line] = lines.as_slice() else {
        panic!("{lines:?}");
    };
    let at = Time::from_ps(10_500);
    assert_eq!(line.at(), Some(at));
    assert_eq!(
        serde_json::to_value(line).unwrap()["data"],
        serde_json::json!({ "from_ps": edge_at, "to_ps": to })
    );
    let trace = outcome.trace.unwrap();
    let row = trace.time().binary_search(&at).unwrap();
    assert_eq!(trace.column(column("#net.src"))[row], 5.0);
    assert_eq!(trace.column(column("#i1.i"))[row], -1e-3);
    for (j, quantity) in trace.columns().iter().enumerate() {
        let want = (restart.column(j)[0] + steps.column(j)[edge + 1]) / 2.0;
        assert_eq!(trace.column(j)[row], want, "{quantity}");
    }
}

/// 5 V through 220 ohm into 1 uF from 0 V, with an LED across the capacitor, over 2 ms in
/// steps of up to 100 us, with a row every 1 us.
fn led_charge() -> Netlist {
    let json = r#"{
      "version": 0,
      "seed": 0,
      "analysis": { "transient": { "stop_ps": 2000000000, "max_step_ps": 100000000, "grid_ps": 1000000 } },
      "ground": "gnd",
      "nets": ["gnd", "vcc", "x"],
      "parts": [
        { "name": "v1", "kind": "voltage_source", "pins": { "p": "vcc", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" } },
        { "name": "r1", "kind": "resistor", "pins": { "a": "vcc", "b": "x" },
          "resistance": { "value": 220.0, "unit": "ohm", "source": "user" }, "ratings": { "power": null } },
        { "name": "c1", "kind": "capacitor", "pins": { "a": "x", "b": "gnd" },
          "capacitance": { "value": 1e-6, "unit": "F", "source": "user" },
          "initial": { "value": 0.0, "unit": "V", "source": "user" } },
        { "name": "led1", "kind": "led", "pins": { "anode": "x", "cathode": "gnd" },
          "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
          "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
          "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
          "ratings": { "current": null, "power": null } }
      ]
    }"#;
    crate::netlist::parse(json.as_bytes()).unwrap()
}

/// With three iterations some rows do not converge from the last accepted step and do from the
/// step's end, and the trace stores that second solve.
#[test]
fn a_row_whose_second_solve_converges_is_solved() {
    let netlist = led_charge();
    let mut session = open(&netlist, Vec::new(), Budget { row: 3, ..BUDGET });
    let mut retried = Vec::new();
    while let State::Running { accepted, next, .. } = &session.state {
        let span = session.span(accepted, next);
        for at in Rows::Grid.rows_in(transient(&netlist), accepted.at, next.at) {
            if span.solve(at, &accepted.iterate).is_err() {
                retried.push((at, span.solve(at, &next.converged.iterate)));
            }
        }
        let landing = next.at;
        session.advance_to(landing);
    }
    let mut none = Vec::new();
    let trace = session.outcome(&mut none).trace.unwrap();
    assert_eq!(none, []);
    assert!(!retried.is_empty());
    for (at, from_step_end) in retried {
        let Ok(solved) = from_step_end else {
            panic!("the row at {at:?} did not solve from the step's end: {from_step_end:?}");
        };
        let row = trace.time().binary_search(&at).unwrap();
        for (j, quantity) in trace.columns().iter().enumerate() {
            assert_eq!(
                trace.column(j)[row].to_bits(),
                solved[j].get().to_bits(),
                "{quantity} at {at:?}"
            );
        }
    }
}

/// 1 V to 2 V at `at`, through 1 kohm into 1 nF, and a 1 kohm switch across the capacitor
/// that closes at the same picosecond.
fn source_and_switch_at(at: u64) -> Netlist {
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": 30000, "max_step_ps": 20000, "grid_ps": 20000 }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vout"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{
                "kind": "step",
                "initial": {{ "value": 1.0, "unit": "V", "source": "user" }},
                "final": {{ "value": 2.0, "unit": "V", "source": "user" }},
                "at_ps": {at}
              }}
            }},
            {{
              "name": "r1",
              "kind": "resistor",
              "pins": {{ "a": "vcc", "b": "vout" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "ratings": {{ "power": null }}
            }},
            {{
              "name": "c1",
              "kind": "capacitor",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "capacitance": {{ "value": 1e-9, "unit": "F", "source": "user" }},
              "initial": null
            }},
            {{
              "name": "s1",
              "kind": "switch",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "on_resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
              "initial": "open",
              "events": [{{ "t_ps": {at}, "state": "closed" }}],
              "bounce": null
            }}
          ]
        }}"#
    );
    crate::netlist::parse(json.as_bytes()).unwrap()
}

#[test]
fn a_source_jump_on_a_switch_edge_restarts_once() {
    let at = 10_000;
    let netlist = source_and_switch_at(at);
    let trace = run_steps(&netlist);
    let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    let k = rows.binary_search(&at).unwrap();
    assert_eq!(rows[k + 1], at + EDGE_STEP);
    let column = |name: &str| {
        let index = trace
            .columns()
            .iter()
            .position(|r| r.to_string() == name)
            .unwrap();
        trace.column(index)
    };
    let (vout, current) = (column("#net.vout"), column("#c1.i"));
    let closed = 1.0 / 1000.0 + 1e-12;
    let open = 1e-9 + 1e-12;
    let got = vout[k + 1];
    let both = backward_euler(2.0, vout[k], EDGE_STEP, closed);
    let source_only = backward_euler(2.0, vout[k], EDGE_STEP, open);
    let switch_only = backward_euler(1.0, vout[k], EDGE_STEP, closed);
    assert!((got - both).abs() < 1e-8, "{got} vs {both}");
    assert!((both - source_only).abs() > 1e-4, "{both} vs {source_only}");
    assert!((both - switch_only).abs() > 1e-4, "{both} vs {switch_only}");
    // A second restart would take another 1 ns. This step has already left that.
    assert!(rows[k + 2] - rows[k + 1] > EDGE_STEP, "{rows:?}");
    let h = rows[k + 2] - rows[k + 1];
    let trap = trapezoidal(2.0, got, current[k + 1], h, closed);
    assert!(
        (vout[k + 2] - trap).abs() < 1e-8,
        "{} vs {trap}",
        vout[k + 2]
    );
}

#[test]
fn a_source_jump_on_a_located_chip_event_restarts_once() {
    let plain = run_steps(&reset_release(Some(30_000_000_000), 0.0));
    let logic = plain.logic().unwrap();
    let out = logic
        .signals()
        .iter()
        .position(|signal| signal.to_string() == "#u1.logic.out")
        .unwrap();
    let edge = |trace: &Trace| {
        trace
            .logic()
            .unwrap()
            .changes()
            .iter()
            .find(|change| change.signal == out && change.level == crate::logic::Level::High)
            .unwrap()
            .at
            .as_ps()
    };
    // The jump is a breakpoint, so it moves the located picosecond.
    let mut at = edge(&plain);
    let mut trace = plain;
    let mut landed = false;
    for _ in 0..6 {
        trace = run_steps(&reset_release(Some(at), 0.0));
        let found = edge(&trace);
        if found == at {
            landed = true;
            break;
        }
        at = found;
    }
    assert!(landed, "the step never landed on the OUT edge");
    let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    let k = rows.binary_search(&at).unwrap();
    let column = |name: &str| {
        let index = trace
            .columns()
            .iter()
            .position(|r| r.to_string() == name)
            .unwrap();
        trace.column(index)
    };
    let (vs, src, out_v, current) = (
        column("#net.vs"),
        column("#net.src"),
        column("#net.out"),
        column("#cs.i"),
    );
    assert!(vs[k].abs() < 1e-6, "#net.vs {vs}", vs = vs[k]);
    assert_eq!(src[k], 0.0, "old level");
    assert_eq!(src[k + 1], 1.0, "new level");
    assert!(out_v[k] < 0.1, "old drive {}", out_v[k]);
    assert!(out_v[k + 1] > 4.0, "new drive {}", out_v[k + 1]);
    let h = rows[k + 1] - at;
    let gmin = 1e-12;
    let got = vs[k + 1];
    let euler = backward_euler(1.0, vs[k], h, gmin);
    let held = backward_euler(0.0, vs[k], h, gmin);
    assert!((got - euler).abs() < 1e-8, "{got} vs {euler} over {h} ps");
    assert!((euler - held).abs() > 1e-6, "{euler} vs {held}");
    // Steps of a few tens of picoseconds do not separate the two rules.
    let j = (k + 1..rows.len() - 1)
        .find(|&j| {
            let h = rows[j + 1] - rows[j];
            (trapezoidal(1.0, vs[j], current[j], h, gmin) - backward_euler(1.0, vs[j], h, gmin))
                .abs()
                > 1e-7
        })
        .expect("no later step separates the trapezoidal rule from backward Euler");
    let h = rows[j + 1] - rows[j];
    let trap = trapezoidal(1.0, vs[j], current[j], h, gmin);
    let still = backward_euler(1.0, vs[j], h, gmin);
    let later = vs[j + 1];
    assert!((later - trap).abs() < 1e-8, "{later} vs {trap}");
    assert!((trap - still).abs() > 1e-7, "{trap} vs {still}");
}
