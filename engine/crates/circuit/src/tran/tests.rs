use sim_core::netlist::{Analysis, Netlist};

use super::*;

/// A 5 V source, 220 ohm and an LED onto a switch that closes at 1 us.
fn led_switch() -> Netlist {
    sim_core::netlist::parse(
        br#"{
          "version": 0,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 2000000, "max_step_ps": 100000 } },
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

fn transient(netlist: &Netlist) -> Transient {
    let Analysis::Transient(transient) = netlist.analysis() else {
        panic!("not a transient");
    };
    *transient
}

/// 1 V through 1 kohm into 1 pF, and a 1 kohm switch across it that closes at 1 us.
/// The capacitor then discharges with a 0.5 ns time constant. `bleed` adds 10 ohm
/// across the source, rated 1 mW.
fn fast_discharge(bleed: bool) -> Netlist {
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
          "analysis": {{ "transient": {{ "stop_ps": 2000000, "max_step_ps": 100000 }} }},
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

/// One Newton iteration per step converges only where nothing moves: before the edge.
#[test]
fn a_step_that_fails_down_to_one_picosecond_ends_the_run_at_the_last_accepted_time() {
    let netlist = fast_discharge(false);
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
    assert!(outcome.observations.op.is_none() && outcome.observations.switches.is_none());
    let trace = outcome.trace.unwrap();
    assert_eq!(trace.end(), Some(Time::from_ps(1_000_000)));
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
          "analysis": { "transient": { "stop_ps": 2000000, "max_step_ps": 100000 } },
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
        Budget { step: 2, ..BUDGET },
        Trace::MAX_ROWS,
        &columns,
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
    let outcome = run(&netlist, &transient(&netlist), &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(outcome.trace.unwrap().end(), Some(Time::from_ps(2_000_000)));
}

#[test]
fn the_same_circuit_converges_with_the_step_budget() {
    let netlist = fast_discharge(false);
    let mut diagnostics = Vec::new();
    let outcome = run(&netlist, &transient(&netlist), &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(outcome.observations.op.is_some());
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
        Budget {
            restart: 1,
            ..BUDGET
        },
        Trace::MAX_ROWS,
        &columns,
        &mut diagnostics,
    );
    assert!(outcome.observations.op.is_none());
    let trace = outcome.trace.unwrap();
    assert_eq!(trace.end(), Some(Time::from_ps(1_000_000)));
    let [failed] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    let text = serde_json::to_string(failed).unwrap();
    assert_eq!(
        text,
        r##"{"code":"solver.convergence","severity":"error","refs":["#led1"],"t_ps":1000000,"message":"Newton did not converge at #led1 after 3 iterations in the restart solve at 1000000 ps; source stepping reached 0","data":{"gmin":null,"iterations":3,"source_scale":0.0}}"##
    );
}

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

#[test]
fn breakpoints_list_each_edge_the_nanosecond_after_it_and_after_the_start() {
    let netlist = led_switch();
    let mut breakpoints = Breakpoints::new(&netlist, Time::from_ps(2_000_000));
    assert_eq!(breakpoints.next_after(Time::ZERO), Time::from_ps(EDGE_STEP));
    assert_eq!(
        breakpoints.next_after(Time::from_ps(EDGE_STEP)),
        Time::from_ps(1_000_000)
    );
    assert_eq!(breakpoints.edges, vec![Time::from_ps(1_000_000)]);
    assert_eq!(
        breakpoints.next_after(Time::from_ps(1_000_000)),
        Time::from_ps(1_000_000 + EDGE_STEP)
    );
    assert_eq!(
        breakpoints.next_after(Time::from_ps(1_000_000 + EDGE_STEP)),
        Time::from_ps(2_000_000)
    );
    assert!(breakpoints.is_edge(Time::from_ps(1_000_000)));
    assert!(!breakpoints.is_edge(Time::from_ps(1_000_000 + EDGE_STEP)));
    let mut early = Breakpoints::new(&netlist, Time::from_ps(1_000_000));
    assert!(early.edges.is_empty());
    assert_eq!(early.next_after(Time::ZERO), Time::from_ps(EDGE_STEP));
}

/// A 0 to 5 V pulse into 1 kohm, rising every 10 ns from 10 ns and 5 ns wide, over 1 s:
/// 2e8 jumps.
fn train_100mhz() -> Netlist {
    sim_core::netlist::parse(
        br#"{
          "version": 0,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 1000000000000, "max_step_ps": 20000000 } },
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
                "pulsed": { "value": 5.0, "unit": "V", "source": "user" },
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
fn breakpoints_list_source_jumps_as_edges_and_stop_at_the_row_cap() {
    let netlist = train_100mhz();
    let mut breakpoints = Breakpoints::new(&netlist, transient(&netlist).stop);
    assert_eq!(breakpoints.edges.len(), Trace::MAX_ROWS);
    assert_eq!(
        breakpoints.edges[..3],
        [10_000, 15_000, 20_000].map(Time::from_ps)
    );
    let last = 10_000 + (Trace::MAX_ROWS as u64 - 1) * 5_000;
    assert_eq!(breakpoints.edges.last(), Some(&Time::from_ps(last)));
    assert_eq!(breakpoints.points.len(), 2 * Trace::MAX_ROWS + 1);
    assert!(breakpoints.is_edge(Time::from_ps(15_000)));
    assert_eq!(
        breakpoints.next_after(Time::from_ps(10_000)),
        Time::from_ps(11_000)
    );
}

/// The first source jumps only after the second has already filled the cap.
#[test]
fn breakpoints_keep_the_earliest_jumps_across_sources() {
    let late = 3_000_000_000_u64;
    let pulse = |name: &str, net: &str, delay: u64| {
        format!(
            r#"{{"name":"{name}","kind":"voltage_source","pins":{{"p":"{net}","n":"gnd"}},"voltage":{{"kind":"pulse","initial":{{"value":0.0,"unit":"V","source":"user"}},"pulsed":{{"value":1.0,"unit":"V","source":"user"}},"delay_ps":{delay},"width_ps":2000,"period_ps":4000}}}}"#
        )
    };
    let resistor = |name: &str, net: &str| {
        format!(
            r#"{{"name":"{name}","kind":"resistor","pins":{{"a":"{net}","b":"gnd"}},"resistance":{{"value":1000.0,"unit":"ohm","source":"user"}},"ratings":{{"power":null}}}}"#
        )
    };
    let json = format!(
        r#"{{"version":0,"seed":0,"analysis":{{"transient":{{"stop_ps":1000000000000,"max_step_ps":20000000}}}},"ground":"gnd","nets":["gnd","a","b"],"parts":[{},{},{},{}]}}"#,
        pulse("vlate", "a", late),
        resistor("ra", "a"),
        pulse("vearly", "b", 2_000),
        resistor("rb", "b"),
    );
    let netlist = sim_core::netlist::parse(json.as_bytes()).unwrap();
    let breakpoints = Breakpoints::new(&netlist, transient(&netlist).stop);
    let last = 2_000 * Trace::MAX_ROWS as u64;
    assert_eq!(breakpoints.edges.len(), Trace::MAX_ROWS);
    assert_eq!(breakpoints.edges[0], Time::from_ps(2_000));
    assert_eq!(breakpoints.edges[1], Time::from_ps(4_000));
    assert_eq!(*breakpoints.edges.last().unwrap(), Time::from_ps(last));
    assert!(breakpoints.edges.iter().all(|at| at.as_ps() < late));
}

#[test]
fn a_dense_pulse_train_ends_at_the_row_cap() {
    let netlist = train_100mhz();
    let mut diagnostics = Vec::new();
    let columns = Columns::new(&netlist);
    let outcome = run_with(
        &netlist,
        &transient(&netlist),
        BUDGET,
        1_000,
        &columns,
        &mut diagnostics,
    );
    assert_eq!(outcome.trace.unwrap().rows(), 1_000);
    let codes: Vec<_> = diagnostics.iter().map(|d| d.code()).collect();
    assert_eq!(codes, ["trace.too_large"]);
}

fn run_clean(netlist: &Netlist) -> Trace {
    let mut diagnostics = Vec::new();
    let outcome = run(netlist, &transient(netlist), &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    outcome.trace.unwrap()
}

fn times(netlist: &Netlist) -> Vec<u64> {
    run_clean(netlist)
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
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {max_step} }} }},
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
    sim_core::netlist::parse(json.as_bytes()).unwrap()
}

/// 1 V through 1 kohm into 1 nF, with two switches across the capacitor.
fn rc_two_switches(edge_a: u64, edge_b: u64, stop: u64, max_step: u64) -> Netlist {
    let json = format!(
        r#"{{
          "version": 0,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {max_step} }} }},
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
    sim_core::netlist::parse(json.as_bytes()).unwrap()
}

/// Backward Euler for 1 kohm into 1 nF, from `v0`, source at `v_src`. `g_load` is the
/// load on the capacitor, including gmin.
fn backward_euler(v_src: f64, v0: f64, h_ps: u64, g_load: f64) -> f64 {
    let h = h_ps as f64 * 1e-12;
    let g_series = 1.0 / 1000.0;
    let c_over_h = 1e-9 / h;
    (g_series * v_src + c_over_h * v0) / (g_series + g_load + c_over_h)
}

/// The trapezoidal rule for that RC, from `v0` and the capacitor current `i0`.
fn trapezoidal(v_src: f64, v0: f64, i0: f64, h_ps: u64, g_load: f64) -> f64 {
    let h = h_ps as f64 * 1e-12;
    let g_series = 1.0 / 1000.0;
    let two_c_over_h = 2e-9 / h;
    (g_series * v_src + two_c_over_h * v0 + i0) / (g_series + g_load + two_c_over_h)
}

#[test]
fn each_edge_restarts_on_backward_euler_and_the_next_step_is_trapezoidal() {
    let netlist = rc_two_switches(10_000, 10_500, 20_000, EDGE_STEP);
    let trace = run_clean(&netlist);
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
    let trace = run_clean(&netlist);
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
          "analysis": {{ "transient": {{ "stop_ps": 40000000000, "max_step_ps": 1000000000 }} }},
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
        Trace::MAX_ROWS,
        &columns,
        &mut diagnostics,
    );
    assert!(outcome.observations.logic.is_none());
    let end = outcome.trace.unwrap().end().unwrap().as_ps();
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
          "analysis": {{ "transient": {{ "stop_ps": 30000, "max_step_ps": 20000 }} }},
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
    let trace = run_clean(&netlist);
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
    let plain = run_clean(&reset_release(Some(30_000_000_000)));
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
        trace = run_clean(&reset_release(Some(at)));
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
