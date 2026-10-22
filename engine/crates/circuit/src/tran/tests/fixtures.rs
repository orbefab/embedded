use sim_core::netlist::{Analysis, Netlist};

use super::super::*;

/// A 5 V source, 220 ohm and an LED onto a switch that closes at 1 us.
pub(super) fn led_switch() -> Netlist {
    sim_core::netlist::parse(
        br#"{
          "version": 0,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 2000000, "max_step_ps": 100000, "grid_ps": 100000 } },
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vk", "vbtn"],
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
              "pins": { "a": "vcc", "b": "vk" },
              "resistance": { "value": 220.0, "unit": "ohm", "source": "user" },
              "ratings": { "power": null }
            },
            {
              "name": "led1",
              "kind": "led",
              "pins": { "anode": "vk", "cathode": "vbtn" },
              "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
              "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
              "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
              "ratings": { "current": null, "power": null }
            },
            {
              "name": "sw1",
              "kind": "switch",
              "pins": { "a": "vbtn", "b": "gnd" },
              "on_resistance": { "value": 0.1, "unit": "ohm", "source": "user" },
              "off_resistance": { "value": 1e9, "unit": "ohm", "source": "user" },
              "initial": "open",
              "events": [{ "t_ps": 1000000, "state": "closed" }],
              "bounce": null
            }
          ]
        }"#,
    )
    .unwrap()
}

pub(super) fn transient(netlist: &Netlist) -> Transient {
    let Analysis::Transient(transient) = netlist.analysis() else {
        panic!("not a transient");
    };
    *transient
}

/// 1 V through 1 kohm into 1 pF, and a 1 kohm switch across it that closes at 1 us.
/// The capacitor then discharges with a 0.5 ns time constant. `bleed` adds 10 ohm
/// across the source, rated 1 mW.
pub(super) fn fast_discharge(bleed: bool) -> Netlist {
    let bleeder = if bleed {
        r#"{
              "name": "rbleed",
              "kind": "resistor",
              "pins": { "a": "vcc", "b": "gnd" },
              "resistance": { "value": 10.0, "unit": "ohm", "source": "user" },
              "ratings": { "power": { "value": 0.001, "unit": "W", "source": "user" } }
            },"#
    } else {
        ""
    };
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": 2000000, "max_step_ps": 100000, "grid_ps": 100000 }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vout"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 1.0, "unit": "V", "source": "user" }}
            }},
            {bleeder}
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
              "capacitance": {{ "value": 1e-12, "unit": "F", "source": "user" }},
              "initial": null
            }},
            {{
              "name": "s1",
              "kind": "switch",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "on_resistance": {{ "value": 1000.0, "unit": "ohm", "source": "user" }},
              "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
              "initial": "open",
              "events": [{{ "t_ps": 1000000, "state": "closed" }}],
              "bounce": null
            }}
          ]
        }}"#
    );
    sim_core::netlist::parse(json.as_bytes()).unwrap()
}

/// The observations of a run that ended early hold what it reached: `end` is the last row,
/// `op` is row 0, every switch edge is before `end`, and the logic summary is the trace's.
/// Returns `end`.
pub(super) fn reached(outcome: &Outcome) -> Time {
    let trace = outcome.trace.as_ref().unwrap();
    let Observations::Transient {
        end,
        op,
        switches,
        logic,
    } = &outcome.observations
    else {
        panic!("{:?}", outcome.observations);
    };
    assert_eq!(Some(*end), trace.end());
    for (column, quantity) in trace.columns().iter().enumerate() {
        assert_eq!(
            op.values[quantity].get(),
            trace.column(column)[0],
            "{quantity}"
        );
    }
    assert!(switches.values().flatten().all(|edge| edge.at < *end));
    assert_eq!(logic.as_ref(), trace.logic().map(Logic::summary).as_ref());
    *end
}

/// A clean run with a row at every accepted step.
pub(super) fn run_steps(netlist: &Netlist) -> Trace {
    let mut diagnostics = Vec::new();
    let outcome = run(netlist, &transient(netlist), Rows::Steps, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    outcome.trace.unwrap()
}

/// Backward Euler for 1 kohm into 1 nF, from `v0`, source at `v_src`. `g_load` is the
/// load on the capacitor, including gmin.
pub(super) fn backward_euler(v_src: f64, v0: f64, h_ps: u64, g_load: f64) -> f64 {
    let h = h_ps as f64 * 1e-12;
    let g_series = 1.0 / 1000.0;
    let c_over_h = 1e-9 / h;
    (g_series * v_src + c_over_h * v0) / (g_series + g_load + c_over_h)
}

/// The trapezoidal rule for that RC, from `v0` and the capacitor current `i0`.
pub(super) fn trapezoidal(v_src: f64, v0: f64, i0: f64, h_ps: u64, g_load: f64) -> f64 {
    let h = h_ps as f64 * 1e-12;
    let g_series = 1.0 / 1000.0;
    let two_c_over_h = 2e-9 / h;
    (g_series * v_src + two_c_over_h * v0 + i0) / (g_series + g_load + two_c_over_h)
}

/// A session on the grid, opened with `chips` beside the netlist's.
pub(super) fn open(netlist: &Netlist, chips: Vec<active::Chip>, budget: Budget) -> Session<'_> {
    let columns = Columns::new(netlist);
    Session::open_recorded(
        netlist,
        &transient(netlist),
        chips,
        budget,
        columns,
        Rows::Grid,
    )
    .unwrap()
}
