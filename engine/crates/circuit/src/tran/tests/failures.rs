use sim_core::netlist::Netlist;

use super::super::*;
use super::fixtures::*;

/// One Newton iteration per step converges only where nothing moves: before the edge.
#[test]
fn a_step_that_fails_down_to_one_picosecond_ends_the_run_at_the_last_accepted_time() {
    let netlist = fast_discharge(false);
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget { step: 1, ..BUDGET },
        Rows::Grid,
        columns,
        &mut diagnostics,
    );
    assert_eq!(reached(&outcome), Time::from_ps(1_000_000));
    let [failed] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let text = serde_json::to_string(failed).unwrap();
    assert_eq!(
        text,
        r##"{"code":"solver.convergence","severity":"error","refs":["#v1"],"t_ps":1000000,"message":"Newton did not converge at #v1 after 10 iterations; the time step halved to 1 ps","data":{"gmin":null,"iterations":10,"source_scale":1.0,"step_ps":1}}"##
    );
}

/// `led_switch` with 10 fF from `vk` to ground, which discharges through the LED
/// after the edge.
fn led_discharge() -> Netlist {
    let json = r#"{
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
              "name": "c1",
              "kind": "capacitor",
              "pins": { "a": "vk", "b": "gnd" },
              "capacitance": { "value": 1e-14, "unit": "F", "source": "user" },
              "initial": null
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
        }"#;
    sim_core::netlist::parse(json.as_bytes()).unwrap()
}

/// The edge dumps 10 fF at 5 V into the LED in femtoseconds, so even a 1 ps step moves
/// the junction far. Two Newton iterations per step do not reach it.
#[test]
fn a_junction_step_that_fails_down_to_one_picosecond_names_the_junction() {
    let netlist = led_discharge();
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget { step: 2, ..BUDGET },
        Rows::Grid,
        columns,
        &mut diagnostics,
    );
    assert_eq!(outcome.trace.unwrap().end(), Some(Time::from_ps(1_000_000)));
    let [failed] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let text = serde_json::to_string(failed).unwrap();
    assert_eq!(
        text,
        r##"{"code":"solver.convergence","severity":"error","refs":["#led1"],"t_ps":1000000,"message":"Newton did not converge at #led1 after 20 iterations; the time step halved to 1 ps","data":{"gmin":null,"iterations":20,"source_scale":1.0,"step_ps":1}}"##
    );
    let mut diagnostics = Vec::new();
    let outcome = run(&netlist, &transient(&netlist), Rows::Grid, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(outcome.trace.unwrap().end(), Some(Time::from_ps(2_000_000)));
}

#[test]
fn the_same_circuit_converges_with_the_step_budget() {
    let netlist = fast_discharge(false);
    let mut diagnostics = Vec::new();
    let outcome = run(&netlist, &transient(&netlist), Rows::Grid, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(outcome.observations.end(), Some(Time::from_ps(2_000_000)));
    assert_eq!(outcome.trace.unwrap().end(), Some(Time::from_ps(2_000_000)));
}

#[test]
fn a_restart_that_fails_keeps_the_edge_row_and_names_the_edge() {
    let netlist = led_switch();
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        Vec::new(),
        Budget {
            restart: 1,
            ..BUDGET
        },
        Rows::Grid,
        columns,
        &mut diagnostics,
    );
    assert_eq!(reached(&outcome), Time::from_ps(1_000_000));
    // The edge at the end never took effect, so it is not listed.
    let switches = outcome.observations.switches().unwrap();
    assert_eq!(switches[&"#sw1".parse().unwrap()], []);
    let [failed] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let text = serde_json::to_string(failed).unwrap();
    assert_eq!(
        text,
        r##"{"code":"solver.convergence","severity":"error","refs":["#led1"],"t_ps":1000000,"message":"Newton did not converge at #led1 after 3 iterations in the restart solve at 1000000 ps; source stepping reached 0","data":{"gmin":null,"iterations":3,"source_scale":0.0}}"##
    );
}

/// A source step from 0 V to 1e200 V at 5000 ps. The grid is 10000 and the stop is
/// 20000, so 5000 is neither a row time nor the stop.
fn huge_step() -> (Netlist, &'static [u8]) {
    let json = br#"{
      "version": 0,
      "seed": 0,
      "analysis": { "transient": { "stop_ps": 20000, "max_step_ps": 10000, "grid_ps": 10000 } },
      "ground": "gnd",
      "nets": ["gnd", "vin"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": {
            "kind": "step",
            "initial": { "value": 0.0, "unit": "V", "source": "user" },
            "final": { "value": 1e200, "unit": "V", "source": "user" },
            "at_ps": 5000
          }
        },
        {
          "name": "c1",
          "kind": "capacitor",
          "pins": { "a": "vin", "b": "gnd" },
          "capacitance": { "value": 1e-9, "unit": "F", "source": "user" },
          "initial": null
        }
      ]
    }"#;
    (sim_core::netlist::parse(json).unwrap(), json)
}

#[test]
fn an_off_grid_failure_keeps_that_accepted_step_as_its_last_row() {
    let (netlist, json) = huge_step();
    let run_one = |rows| {
        let mut diagnostics = Vec::new();
        let outcome = run(&netlist, &transient(&netlist), rows, &mut diagnostics);
        (outcome, diagnostics)
    };
    let (grid, grid_diagnostics) = run_one(Rows::Grid);
    let (steps, step_diagnostics) = run_one(Rows::Steps);
    let end = reached(&grid);
    assert_eq!(end, Time::from_ps(5_000));
    assert_eq!(reached(&steps), end);
    assert_ne!(end.as_ps() % transient(&netlist).grid, 0);
    assert_ne!(end, transient(&netlist).stop);
    let [failed] = grid_diagnostics.as_slice() else {
        panic!("{grid_diagnostics:?}");
    };
    assert_eq!(failed.code(), "solver.nonfinite");
    assert_eq!(failed.refs(), vec!["#c1".parse().unwrap()]);
    assert_eq!(failed.at(), Some(end));
    assert_eq!(step_diagnostics[0].code(), failed.code());
    assert_eq!(step_diagnostics[0].at(), failed.at());
    let trace = grid.trace.unwrap();
    assert_eq!(
        trace.time().iter().map(|t| t.as_ps()).collect::<Vec<_>>(),
        [0, 5_000]
    );
    let vin = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == "#net.vin")
        .unwrap();
    assert_eq!(trace.column(vin).last().copied(), Some(0.0));
    let results =
        sim_core::run_folder::Results::new(grid.observations, Some(trace), grid_diagnostics);
    let folder = sim_core::run_folder::render(
        json,
        Some(&netlist),
        &results,
        sim_core::timestamp::Timestamp::from_unix_seconds(0),
    );
    let index: serde_json::Value =
        serde_json::from_slice(folder.file("traces/index.json").unwrap()).unwrap();
    assert_eq!(index["end_ps"], end.as_ps());
    let time = folder.file("traces/time.u64").unwrap();
    let last = time.len() - 8;
    let written = u64::from_le_bytes(time[last..].try_into().unwrap());
    assert_eq!(written, end.as_ps());
    let observations: serde_json::Value =
        serde_json::from_slice(folder.file("observations.json").unwrap()).unwrap();
    assert_eq!(observations["end_ps"], end.as_ps());
}
