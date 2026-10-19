use sim_core::netlist::Netlist;

use super::super::*;
use super::fixtures::*;

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
