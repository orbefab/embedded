use crate::netlist::{Analysis, Netlist};

use super::super::*;
use crate::active;
use crate::session::Budget;

/// A 5 V source, 220 ohm and an LED onto a switch that closes at 1 us.
pub(super) fn led_switch() -> Netlist {
    crate::netlist::parse(
        br#"{
          "version": 1,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 2000000, "max_step_ps": 100000, "grid_ps": 100000 } },
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vk", "vbtn"],
          "parts": [
            {
              "name": "v1",
              "kind": "voltage_source",
              "pins": { "p": "vcc", "n": "gnd" },
              "voltage": { "value": 5.0, "unit": "V" }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "vcc", "b": "vk" },
              "resistance": { "value": 220.0, "unit": "ohm" },
              "ratings": { "power": null }
            },
            {
              "name": "led1",
              "kind": "led",
              "pins": { "anode": "vk", "cathode": "vbtn" },
              "saturation_current": { "value": 6.711e-10, "unit": "A" },
              "emission_coefficient": { "value": 4.427, "unit": "1" },
              "series_resistance": { "value": 2.115, "unit": "ohm" },
              "ratings": { "current": null, "power": null }
            },
            {
              "name": "sw1",
              "kind": "switch",
              "pins": { "a": "vbtn", "b": "gnd" },
              "on_resistance": { "value": 0.1, "unit": "ohm" },
              "off_resistance": { "value": 1e9, "unit": "ohm" },
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
/// across the source, rated 1 uW. Its 0.1 W crosses that over the window by 0.102 us.
pub(super) fn fast_discharge(bleed: bool) -> Netlist {
    let bleeder = if bleed {
        r#"{
              "name": "rbleed",
              "kind": "resistor",
              "pins": { "a": "vcc", "b": "gnd" },
              "resistance": { "value": 10.0, "unit": "ohm" },
              "ratings": { "power": { "value": 1e-6, "unit": "W" } }
            },"#
    } else {
        ""
    };
    let json = format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": 2000000, "max_step_ps": 100000, "grid_ps": 100000 }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "vout"],
          "parts": [
            {{
              "name": "v1",
              "kind": "voltage_source",
              "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 1.0, "unit": "V" }}
            }},
            {bleeder}
            {{
              "name": "r1",
              "kind": "resistor",
              "pins": {{ "a": "vcc", "b": "vout" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }},
              "ratings": {{ "power": null }}
            }},
            {{
              "name": "c1",
              "kind": "capacitor",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "capacitance": {{ "value": 1e-12, "unit": "F" }},
              "initial": null
            }},
            {{
              "name": "s1",
              "kind": "switch",
              "pins": {{ "a": "vout", "b": "gnd" }},
              "on_resistance": {{ "value": 1000.0, "unit": "ohm" }},
              "off_resistance": {{ "value": 1e9, "unit": "ohm" }},
              "initial": "open",
              "events": [{{ "t_ps": 1000000, "state": "closed" }}],
              "bounce": null
            }}
          ]
        }}"#
    );
    crate::netlist::parse(json.as_bytes()).unwrap()
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
        ..
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

/// `run` with `actives` beside the netlist's chips and each solve held to `budget`.
/// It checks no row limit and reports no floating net, because a test chip drives nets the
/// netlist leaves floating.
pub(super) fn run_with(
    netlist: &Netlist,
    transient: &Transient,
    actives: Vec<active::Chip>,
    budget: Budget,
    rows: Rows,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    let recorded = match Session::start(
        netlist.clone(),
        transient,
        actives,
        budget,
        Columns::new(netlist),
        Some(rows),
    ) {
        Ok(mut session) => {
            session.warnings.clear();
            Ok(session)
        }
        Err(failure) => Err(Unopened {
            error: failure.error,
            warnings: Vec::new(),
        }),
    };
    to_stop(recorded, transient, diagnostics)
}

/// A clean run with a row at every accepted step.
pub(super) fn run_steps(netlist: &Netlist) -> Trace {
    let mut diagnostics = Vec::new();
    let outcome = run_with(
        netlist,
        &transient(netlist),
        Vec::new(),
        BUDGET,
        Rows::Steps,
        &mut diagnostics,
    );
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

/// Variable-step BDF2 for that RC, from `v1` at the step's start and `v0` one step of `h0_ps`
/// before it.
pub(super) fn bdf2(v_src: f64, (v0, v1): (f64, f64), h0_ps: u64, h_ps: u64, g_load: f64) -> f64 {
    let h = h_ps as f64 * 1e-12;
    let w = h_ps as f64 / h0_ps as f64;
    let [a0, a1, a2] = [(1.0 + 2.0 * w) / (1.0 + w), -(1.0 + w), w * w / (1.0 + w)];
    let g_series = 1.0 / 1000.0;
    let c_over_h = 1e-9 / h;
    (g_series * v_src - c_over_h * (a1 * v1 + a2 * v0)) / (g_series + g_load + a0 * c_over_h)
}

/// A session on the grid, opened with `chips` beside the netlist's.
pub(super) fn open(netlist: &Netlist, chips: Vec<active::Chip>, budget: Budget) -> Session {
    Session::start(
        netlist.clone(),
        &transient(netlist),
        chips,
        budget,
        Columns::new(netlist),
        Some(Rows::Grid),
    )
    .unwrap()
}
