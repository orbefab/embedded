use sim_core::netlist::Netlist;

use super::super::*;
use super::fixtures::*;

/// 100 kohm and 1 uF from 5 V hold an NE555's RESET low at power-on, and its release at
/// about 15.1 ms sets the latch. The NE555 has `examples/ne555-led`'s values, as in
/// `tests/chip.rs`. `step_ps` adds a 0 to 1 V step into 1 kohm and 1 nF at that time.
fn reset_release(step_ps: Option<u64>) -> Netlist {
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
        value(0.0, "V"),
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
    sim_core::netlist::parse(json.as_bytes()).unwrap()
}

/// One Newton iteration converges only where nothing moves, and a probe is a shorter step.
#[test]
fn a_stalled_probe_ends_the_run_and_names_the_probe() {
    let netlist = reset_release(None);
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Budget { probe: 1, ..BUDGET },
        Rows::Grid,
        &columns,
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
    sim_core::netlist::parse(json.as_bytes()).unwrap()
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
    let plain = run_steps(&reset_release(Some(30_000_000_000)));
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
            .find(|change| change.signal == out && change.level == sim_core::logic::Level::High)
            .unwrap()
            .at
            .as_ps()
    };
    // The jump is a breakpoint, so it moves the located picosecond.
    let mut at = edge(&plain);
    let mut trace = plain;
    let mut landed = false;
    for _ in 0..6 {
        trace = run_steps(&reset_release(Some(at)));
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
