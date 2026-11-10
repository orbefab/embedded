use crate::netlist::Netlist;

use super::super::*;
use super::fixtures::*;

#[test]
fn breakpoints_list_each_edge_the_nanosecond_after_it_and_after_the_start() {
    let netlist = led_switch();
    let mut breakpoints = Breakpoints::new(&netlist, Time::from_ps(2_000_000));
    assert_eq!(
        breakpoints.next_after(&netlist, Time::ZERO),
        Time::from_ps(EDGE_STEP)
    );
    assert!(!breakpoints.is_edge(Time::from_ps(EDGE_STEP)));
    assert_eq!(
        breakpoints.next_after(&netlist, Time::from_ps(EDGE_STEP)),
        Time::from_ps(1_000_000)
    );
    assert!(breakpoints.is_edge(Time::from_ps(1_000_000)));
    assert_eq!(
        breakpoints.next_after(&netlist, Time::from_ps(1_000_000)),
        Time::from_ps(1_000_000 + EDGE_STEP)
    );
    assert!(!breakpoints.is_edge(Time::from_ps(1_000_000 + EDGE_STEP)));
    assert_eq!(
        breakpoints.next_after(&netlist, Time::from_ps(1_000_000 + EDGE_STEP)),
        Time::from_ps(2_000_000)
    );
    let mut early = Breakpoints::new(&netlist, Time::from_ps(1_000_000));
    assert_eq!(
        early.next_after(&netlist, Time::ZERO),
        Time::from_ps(EDGE_STEP)
    );
    assert_eq!(
        early.next_after(&netlist, Time::from_ps(EDGE_STEP)),
        Time::from_ps(1_000_000)
    );
    assert!(!early.is_edge(Time::from_ps(1_000_000)));
}

/// A 0 to 5 V pulse into 1 kohm, rising every 10 ns from 10 ns and 5 ns wide, over 1 s:
/// 2e8 jumps.
fn train_100mhz() -> Netlist {
    crate::netlist::parse(
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

/// Every landing from `from` up to `to`, as `steps` reaches them.
fn landings(netlist: &Netlist, breakpoints: &mut Breakpoints, from: u64, to: u64) -> Vec<u64> {
    let mut now = Time::from_ps(from);
    let mut out = Vec::new();
    while now.as_ps() < to {
        now = breakpoints.next_after(netlist, now);
        out.push(now.as_ps());
    }
    out
}

#[test]
fn breakpoints_list_every_source_jump_past_a_million_without_a_cap() {
    let netlist = train_100mhz();
    let mut breakpoints = Breakpoints::new(&netlist, Time::from_ps(1_000_000_000_000));
    assert_eq!(
        landings(&netlist, &mut breakpoints, 0, 21_000),
        [1_000, 10_000, 11_000, 15_000, 16_000, 20_000, 21_000]
    );
    // The jump after the millionth: 10 ns, then every 5 ns.
    let past = 10_000 + Trace::MAX_ROWS as u64 * 5_000;
    let mut now = Time::from_ps(21_000);
    while now.as_ps() < past {
        now = breakpoints.next_after(&netlist, now);
    }
    assert_eq!(now, Time::from_ps(past));
    assert!(breakpoints.is_edge(now));
    assert_eq!(
        breakpoints.next_after(&netlist, now),
        Time::from_ps(past + EDGE_STEP)
    );
}

/// Two sources whose jumps interleave, one of them half a nanosecond off the other's grid.
#[test]
fn breakpoints_merge_jumps_across_sources_in_time_order() {
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
        r#"{{"version":0,"seed":0,"analysis":{{"transient":{{"stop_ps":1000000000000,"max_step_ps":20000000,"grid_ps":20000000}}}},"ground":"gnd","nets":["gnd","a","b"],"parts":[{},{},{},{}]}}"#,
        pulse("vlate", "a", 9_500),
        resistor("ra", "a"),
        pulse("vearly", "b", 2_000),
        resistor("rb", "b"),
    );
    let netlist = crate::netlist::parse(json.as_bytes()).unwrap();
    let mut breakpoints = Breakpoints::new(&netlist, Time::from_ps(1_000_000_000_000));
    assert_eq!(
        landings(&netlist, &mut breakpoints, 0, 9_000),
        [
            1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 7_000, 8_000, 9_000
        ]
    );
    assert_eq!(
        breakpoints.next_after(&netlist, Time::from_ps(9_000)),
        Time::from_ps(9_500)
    );
    assert!(breakpoints.is_edge(Time::from_ps(9_500)));
    assert_eq!(
        landings(&netlist, &mut breakpoints, 9_500, 12_000),
        [10_000, 10_500, 11_000, 11_500, 12_000]
    );
    assert!(breakpoints.is_edge(Time::from_ps(12_000)));
}

#[test]
fn a_grid_over_the_row_limit_is_refused_before_any_solve() {
    let netlist = train_100mhz();
    let fine = Transient {
        grid: Transient::MIN_GRID_PS,
        ..transient(&netlist)
    };
    let mut diagnostics = Vec::new();
    let outcome = run(&netlist, &fine, Rows::Grid, &mut diagnostics);
    assert!(outcome.trace.is_none());
    assert!(matches!(outcome.observations, Observations::Nothing));
    let [refused] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}");
    };
    assert_eq!(
        serde_json::to_string(refused).unwrap(),
        r#"{"code":"trace.too_large","severity":"error","refs":[],"t_ps":null,"message":"the grid gives 1000000001 rows; the trace holds 1000000 rows of 8 columns; raise grid_ps or shorten stop_ps","data":{"columns":8,"limit":1000000,"rows":1000000001}}"#
    );
}
