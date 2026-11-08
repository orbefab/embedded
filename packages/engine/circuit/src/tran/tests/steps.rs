use orbe_engine_core::netlist::Netlist;

use super::super::*;
use super::fixtures::*;

fn times(netlist: &Netlist) -> Vec<u64> {
    run_steps(netlist)
        .time()
        .iter()
        .map(|t| t.as_ps())
        .collect()
}

/// 5 V across two 1 kohm resistors, and a switch to ground that closes at `edge`.
fn divider_switch(edge: u64, stop: u64, max_step: u64) -> Netlist {
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {max_step}, "grid_ps": {stop} }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vout"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }}
            }},
            {{
              "name": "r1",
              "kind": "resistor",
              "pins": {{ "a": "vcc", "b": "vout" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "ratings": {{ "power": null }}
            }},
            {{
              "name": "r2",
              "kind": "resistor",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "ratings": {{ "power": null }}
            }},
            {{
              "name": "s1",
              "kind": "switch",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "on_resistance": {{ "value": 0.1, "unit": "ohm", "source": "user" }},
              "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
              "initial": "open",
              "events": [{{ "t_ps": {edge}, "state": "closed" }}],
              "bounce": null
            }}
          ]
        }}"#
    );
    orbe_engine_core::netlist::parse(json.as_bytes()).unwrap()
}

/// 1 V through 1 kohm into 1 nF, with two switches across the capacitor.
fn rc_two_switches(edge_a: u64, edge_b: u64, stop: u64, max_step: u64) -> Netlist {
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {max_step}, "grid_ps": {stop} }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vout"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 1.0, "unit": "V", "source": "user" }}
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
              "events": [{{ "t_ps": {edge_a}, "state": "closed" }}],
              "bounce": null
            }},
            {{
              "name": "s2",
              "kind": "switch",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "on_resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
              "initial": "open",
              "events": [{{ "t_ps": {edge_b}, "state": "closed" }}],
              "bounce": null
            }}
          ]
        }}"#
    );
    orbe_engine_core::netlist::parse(json.as_bytes()).unwrap()
}

#[test]
fn each_edge_restarts_on_backward_euler_and_the_next_step_is_trapezoidal() {
    let netlist = rc_two_switches(10_000, 10_500, 20_000, EDGE_STEP);
    let trace = run_steps(&netlist);
    let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    assert_eq!(
        rows,
        [
            0, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10_000, 10_500, 11_000,
            11_500, 12_500, 13_500, 14_500, 15_500, 16_500, 17_500, 18_500, 19_500, 20_000,
        ]
    );
    let column = |name: &str| {
        let index = trace
            .columns()
            .iter()
            .position(|r| r.to_string() == name)
            .unwrap();
        trace.column(index)
    };
    let (vout, current) = (column("#net.vout"), column("#c1.i"));
    let at = |values: &[f64], t: u64| values[rows.binary_search(&t).unwrap()];
    // s1 closes at 10 ns. s2 closes at 10.5 ns. gmin is 1e-12 S.
    let one_closed = 1.0 / 1000.0 + 1e-9 + 1e-12;
    let both_closed = 2.0 / 1000.0 + 1e-12;
    // The restart holds c1 through 1e-6 ohm, so it starts within 1e-9 V of the row at the edge.
    for (start, end, g_load) in [(10_000, 10_500, one_closed), (10_500, 11_000, both_closed)] {
        let got = at(vout, end);
        let want = backward_euler(1.0, at(vout, start), end - start, g_load);
        assert!(
            (got - want).abs() < 1e-8,
            "{start} -> {end}: {got} vs backward Euler {want}"
        );
    }
    let (start, end) = (11_000, 11_500);
    let got = at(vout, end);
    let want = trapezoidal(
        1.0,
        at(vout, start),
        at(current, start),
        end - start,
        both_closed,
    );
    let euler = backward_euler(1.0, at(vout, start), end - start, both_closed);
    assert!((want - euler).abs() > 1e-7, "{want} vs {euler}");
    assert!(
        (got - want).abs() < 1e-12,
        "{start} -> {end}: {got} vs trapezoidal {want}"
    );
}

#[test]
fn an_edge_100_ps_before_stop_rows_at_the_edge_and_at_stop() {
    let netlist = divider_switch(9_900, 10_000, 10_000);
    let trace = run_steps(&netlist);
    let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    assert_eq!(rows, [0, 1_000, 3_000, 7_000, 9_900, 10_000]);
    let index = trace
        .columns()
        .iter()
        .position(|r| r.to_string() == "#net.vout")
        .unwrap();
    let vout = trace.column(index);
    let at = |t: u64| vout[rows.binary_search(&t).unwrap()];
    assert!((at(9_900) - 2.5).abs() < 1e-3, "old contact {}", at(9_900));
    assert!(at(10_000) < 0.01, "new contact {}", at(10_000));
}

#[test]
fn an_edge_at_1_ps_keeps_the_start_point_and_the_edge_point() {
    let netlist = divider_switch(1, 10_000, 10_000);
    // The restart wants 1 ns. The cut steps to 1 ns and 1.001 ns keep that, then it doubles.
    assert_eq!(
        times(&netlist),
        [0, 1, 1_000, 1_001, 2_001, 4_001, 8_001, 10_000]
    );
}

const EDGE: u64 = 3_500;

/// A switch from `vsw` to ground that closes at `close`, with 1 kohm from `vcc` to `vsw`, and a
/// step from 0 V to 5 V at `jump` on `vp` into 1 kohm. `at_rest` adds 1 kohm from `vcc` into
/// 1 nF, which sits at 5 V. 5 V on `vcc`, over 100 ns in steps of up to 100 ns.
fn edge_beside(close: u64, jump: u64, at_rest: bool) -> Netlist {
    let rc = if at_rest {
        r#",
            { "name": "r1", "kind": "resistor", "pins": { "a": "vcc", "b": "vout" },
              "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" }, "ratings": { "power": null } },
            { "name": "c1", "kind": "capacitor", "pins": { "a": "vout", "b": "gnd" },
              "capacitance": { "value": 1e-9, "unit": "F", "source": "user" }, "initial": null }"#
    } else {
        ""
    };
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": 100000, "max_step_ps": 100000, "grid_ps": 100000 }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vsw", "vp", "vout"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 5.0, "unit": "V", "source": "user" }} }},
            {{ "name": "rs", "kind": "resistor", "pins": {{ "a": "vcc", "b": "vsw" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }},
            {{ "name": "s1", "kind": "switch", "pins": {{ "a": "vsw", "b": "gnd" }},
              "on_resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
              "initial": "open", "events": [{{ "t_ps": {close}, "state": "closed" }}], "bounce": null }},
            {{ "name": "vs", "kind": "voltage_source", "pins": {{ "p": "vp", "n": "gnd" }},
              "voltage": {{ "kind": "step", "initial": {{ "value": 0.0, "unit": "V", "source": "user" }},
                           "final": {{ "value": 5.0, "unit": "V", "source": "user" }}, "at_ps": {jump} }} }},
            {{ "name": "rp", "kind": "resistor", "pins": {{ "a": "vp", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }}, "ratings": {{ "power": null }} }}{rc}
          ]
        }}"#
    );
    orbe_engine_core::netlist::parse(json.as_bytes()).unwrap()
}

/// With no capacitor there is no error, so the step doubles from 1 ns. The edge at 3.5 ns cuts a
/// 4 ns step, and the breakpoint 1 ns after it cuts the next. A cut step keeps the step it
/// wanted, and an edge that moves no capacitor keeps the segment, so 4 ns follows. A segment
/// started at the edge would take 1 ns and then 2 ns. A capacitor at rest beside the edge
/// changes none of that. The step after the edge has the new contact or level in force.
#[test]
fn a_switch_edge_or_a_source_jump_that_moves_no_capacitor_keeps_the_segment() {
    let late = 1_000_000;
    for at_rest in [false, true] {
        for (close, jump, net, level) in
            [(EDGE, late, "#net.vsw", 2.5), (late, EDGE, "#net.vp", 5.0)]
        {
            let trace = run_steps(&edge_beside(close, jump, at_rest));
            let rows: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
            assert_eq!(
                rows,
                [
                    0,
                    1_000,
                    3_000,
                    EDGE,
                    EDGE + EDGE_STEP,
                    8_500,
                    16_500,
                    32_500,
                    64_500,
                    100_000
                ],
                "{net}, capacitor at rest: {at_rest}"
            );
            let column = trace
                .columns()
                .iter()
                .position(|r| r.to_string() == net)
                .unwrap();
            let after = trace.column(column)[4];
            assert!((after - level).abs() < 1e-6, "{net} {after}");
        }
    }
}
