use std::collections::BTreeMap;

use sim_core::diagnostic::Diagnostic;
use sim_core::finite::Finite;
use sim_core::name::Ref;
use sim_core::netlist::{Edge, Model, Netlist, Transient};
use sim_core::run_folder::{Observations, OpPoint};
use sim_core::time::Time;
use sim_core::trace::Trace;

use crate::newton::{
    Circuit, Conditions, Converged, Failure, Integration, Iterate, Method, Offender, newton,
};
use crate::observe::{self, Columns};
use crate::op;
use crate::ratings::{Ratings, Timing};
use crate::step::{EDGE_STEP, Plan};

pub(crate) const STEP_BUDGET: u32 = 50;

/// After an error `observations` is empty, and the trace keeps the rows before it.
pub(crate) struct Outcome {
    pub(crate) observations: Observations,
    pub(crate) trace: Option<Trace>,
}

/// The operating point, then steps to `stop`. Rating findings and any solver
/// error go to `diagnostics`.
pub(crate) fn run(
    netlist: &Netlist,
    transient: &Transient,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    let columns = Columns::new(netlist);
    run_with(
        netlist,
        transient,
        STEP_BUDGET,
        Trace::row_limit(columns.refs().len()),
        &columns,
        diagnostics,
    )
}

fn run_with(
    netlist: &Netlist,
    transient: &Transient,
    budget: u32,
    max_rows: usize,
    columns: &Columns,
    diagnostics: &mut Vec<Diagnostic>,
) -> Outcome {
    let mut record = Record {
        trace: Trace::new(columns.refs().to_vec(), transient.stop),
        max_rows,
        ratings: Ratings::new(netlist, columns, Timing::Run),
        columns,
        op: None,
    };
    match steps(netlist, transient, budget, &mut record) {
        Ok(()) => {
            diagnostics.extend(record.ratings.finish());
            Outcome {
                observations: Observations {
                    op: record.op,
                    switches: Some(switches(netlist, transient.stop)),
                },
                trace: Some(record.trace),
            }
        }
        Err(diagnostic) => {
            diagnostics.extend(record.ratings.finish());
            diagnostics.push(diagnostic);
            Outcome {
                observations: Observations::default(),
                trace: (record.trace.rows() > 0).then_some(record.trace),
            }
        }
    }
}

/// Everything an accepted row feeds.
struct Record<'c> {
    columns: &'c Columns,
    trace: Trace,
    max_rows: usize,
    ratings: Ratings,
    op: Option<OpPoint>,
}

impl Record<'_> {
    fn accept(&mut self, at: Time, row: &[Finite]) -> Result<(), Diagnostic> {
        if self.trace.rows() == self.max_rows {
            return Err(Diagnostic::TraceTooLarge {
                at,
                limit: self.max_rows,
                columns: self.columns.refs().len(),
            });
        }
        self.trace.push(at, row);
        if at == Time::ZERO {
            self.op = Some(observe::op_point(self.columns, row));
        }
        self.ratings.sample(at, row);
        Ok(())
    }
}

/// The whole state a step starts from. Replaced, never mutated, on accept.
struct Accepted {
    at: Time,
    iterate: Iterate,
}

fn steps(
    netlist: &Netlist,
    transient: &Transient,
    budget: u32,
    record: &mut Record<'_>,
) -> Result<(), Diagnostic> {
    let circuit = Circuit::new(netlist)?;
    let start = op::operating_point(&circuit)?;
    let row = observe::observe(&circuit, record.columns, &start.solution, Time::ZERO)?;
    record.accept(Time::ZERO, &row)?;
    let mut breakpoints = Breakpoints::new(netlist, transient.stop);
    let mut accepted = Accepted {
        at: Time::ZERO,
        iterate: start,
    };
    let mut plan = Plan::RESTART;
    // Newton iterations over every failed attempt since the last accepted step.
    let mut iterations = 0;
    while accepted.at < transient.stop {
        let h = plan.clip(accepted.at, breakpoints.next_after(accepted.at));
        let at = Time::from_ps(accepted.at.as_ps() + h);
        match attempt(&circuit, &accepted, h, plan.method, budget) {
            Ok(converged) => {
                accepted = Accepted {
                    at,
                    iterate: converged.iterate,
                };
                let row =
                    observe::observe(&circuit, record.columns, &accepted.iterate.solution, at)?;
                record.accept(at, &row)?;
                iterations = 0;
                // An open ramp restarts at 1 ns. Growing through it leaves the exit step at `max_step` and trapezoidal.
                plan = if breakpoints.ramp_open(at) {
                    Plan::RESTART
                } else {
                    plan.grow(transient.max_step)
                };
            }
            Err(Failure::Singular { row }) => {
                return Err(Diagnostic::SolverSingular {
                    at,
                    unknown: circuit.layout.describe(netlist, row),
                });
            }
            Err(Failure::Stalled(stall)) => {
                // Without a junction the system is linear, and a shorter step does not make it finite.
                if let Offender::NonFinite(_) = stall.offender
                    && circuit.layout.junctions() == 0
                {
                    return Err(Diagnostic::SolverNonFinite {
                        at,
                        unknown: stall.offender.describe(&circuit),
                    });
                }
                iterations += stall.iterations;
                plan = plan.halve(h).ok_or_else(|| Diagnostic::SolverConvergence {
                    at: accepted.at,
                    unknown: stall.offender.describe(&circuit),
                    iterations,
                    gmin: None,
                    source_scale: 1.0,
                    step_ps: Some(h),
                })?;
            }
        }
    }
    Ok(())
}

/// One step of `h` ps from `from`. `from` is only read, so a retry calls this again.
fn attempt(
    circuit: &Circuit,
    from: &Accepted,
    h: u64,
    method: Method,
    budget: u32,
) -> Result<Converged, Failure> {
    let conditions = Conditions {
        at: Time::from_ps(from.at.as_ps() + h),
        step: Some(Integration {
            method,
            h_ps: h,
            previous: &from.iterate.solution,
        }),
        ..Conditions::NOMINAL
    };
    newton(circuit, &conditions, from.iterate.clone(), budget)
}

/// Where a step must land. Contact edges, the 1 ns ramp point after t = 0,
/// and the 1 ns ramp point after every edge, then `stop`. The cursor only moves forward.
struct Breakpoints {
    /// Contact edges strictly before `stop`, ascending.
    edges: Vec<Time>,
    /// Landing times strictly before `stop`, ascending.
    points: Vec<Time>,
    stop: Time,
    cursor: usize,
}

impl Breakpoints {
    fn new(netlist: &Netlist, stop: Time) -> Breakpoints {
        let mut edges: Vec<Time> = netlist
            .parts()
            .filter_map(|(_, part)| match &part.model {
                Model::Switch(switch) => Some(switch.schedule.edges()),
                _ => None,
            })
            .flatten()
            .map(|edge| edge.at)
            .filter(|&at| at < stop)
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mut points = Vec::with_capacity(edges.len() * 2 + 1);
        push_point(&mut points, EDGE_STEP, stop);
        for &edge in &edges {
            push_point(&mut points, edge.as_ps(), stop);
            if let Some(end) = edge.as_ps().checked_add(EDGE_STEP) {
                push_point(&mut points, end, stop);
            }
        }
        points.sort_unstable();
        points.dedup();
        Breakpoints {
            edges,
            points,
            stop,
            cursor: 0,
        }
    }

    fn next_after(&mut self, now: Time) -> Time {
        while self
            .points
            .get(self.cursor)
            .is_some_and(|&point| point <= now)
        {
            self.cursor += 1;
        }
        self.points.get(self.cursor).copied().unwrap_or(self.stop)
    }

    /// True while `at` is before 1 ns after t = 0, or before 1 ns after the last edge at or before `at`.
    /// Edges are sorted, so that last edge owns the horizon.
    fn ramp_open(&self, at: Time) -> bool {
        let end = self.edges.partition_point(|&edge| edge <= at);
        let horizon = if end == 0 {
            EDGE_STEP
        } else {
            self.edges[end - 1].as_ps().saturating_add(EDGE_STEP)
        };
        at.as_ps() < horizon
    }
}

/// A landing time strictly before `stop`. `stop` itself is the cursor's end.
fn push_point(points: &mut Vec<Time>, at: u64, stop: Time) {
    if at > 0 && at < stop.as_ps() {
        points.push(Time::from_ps(at));
    }
}

/// Every contact edge before `stop`, per switch.
fn switches(netlist: &Netlist, stop: Time) -> BTreeMap<Ref, Vec<Edge>> {
    netlist
        .parts()
        .filter_map(|(_, part)| match &part.model {
            Model::Switch(switch) => Some((
                Ref::Part(part.name.clone()),
                switch
                    .schedule
                    .edges()
                    .iter()
                    .filter(|edge| edge.at < stop)
                    .copied()
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_step_that_fails_down_to_one_picosecond_ends_the_run_at_the_last_accepted_time() {
        let netlist = led_switch();
        let mut diagnostics = Vec::new();
        let columns = Columns::new(&netlist);
        let outcome = run_with(
            &netlist,
            &transient(&netlist),
            5,
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
            r##"{"code":"solver.convergence","severity":"error","refs":["#led1"],"t_ps":1000000,"message":"Newton did not converge at #led1 after 50 iterations; the time step halved to 1 ps","data":{"gmin":null,"iterations":50,"source_scale":1.0,"step_ps":1}}"##
        );
    }

    #[test]
    fn the_same_circuit_converges_with_the_step_budget() {
        let netlist = led_switch();
        let mut diagnostics = Vec::new();
        let outcome = run(&netlist, &transient(&netlist), &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(outcome.observations.op.is_some());
        assert_eq!(outcome.trace.unwrap().end(), Some(Time::from_ps(2_000_000)));
    }

    #[test]
    fn a_trace_past_its_row_limit_ends_the_run_and_keeps_the_rows() {
        let netlist = led_switch();
        let mut diagnostics = Vec::new();
        let columns = Columns::new(&netlist);
        let outcome = run_with(
            &netlist,
            &transient(&netlist),
            STEP_BUDGET,
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
            STEP_BUDGET,
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
        let netlist = led_with_bleed();
        let mut diagnostics = Vec::new();
        let columns = Columns::new(&netlist);
        let outcome = run_with(
            &netlist,
            &transient(&netlist),
            5,
            Trace::MAX_ROWS,
            &columns,
            &mut diagnostics,
        );
        assert!(outcome.observations.op.is_none());
        assert!(outcome.trace.unwrap().rows() > 0);
        let codes: Vec<_> = diagnostics.iter().map(|d| d.code()).collect();
        assert_eq!(codes, ["rating.power", "solver.convergence"]);
    }

    fn led_with_bleed() -> Netlist {
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
                  "name": "rbleed",
                  "kind": "resistor",
                  "pins": { "a": "vcc", "b": "gnd" },
                  "resistance": { "value": 10.0, "unit": "ohm", "source": "user" },
                  "ratings": { "power": { "value": 0.001, "unit": "W", "source": "user" } }
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

    #[test]
    fn breakpoints_list_each_edge_its_ramp_and_the_start_ramp() {
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
        assert!(breakpoints.ramp_open(Time::from_ps(1_000_000)));
        assert!(!breakpoints.ramp_open(Time::from_ps(1_000_000 + EDGE_STEP)));
        let mut early = Breakpoints::new(&netlist, Time::from_ps(1_000_000));
        assert!(early.edges.is_empty());
        assert_eq!(early.next_after(Time::ZERO), Time::from_ps(EDGE_STEP));
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
                  "capacitance": {{ "value": 1e-9, "unit": "F", "source": "user" }}
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

    /// Backward Euler for the RC above. `g_load` is both switches plus gmin.
    fn backward_euler(v0: f64, h_ps: u64, g_load: f64) -> f64 {
        let h = h_ps as f64 * 1e-12;
        let g_series = 1.0 / 1000.0;
        let c_over_h = 1e-9 / h;
        (g_series + c_over_h * v0) / (g_series + g_load + c_over_h)
    }

    #[test]
    fn two_switches_500_ps_apart_keep_both_ramps_on_backward_euler() {
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
        let index = trace
            .columns()
            .iter()
            .position(|r| r.to_string() == "#net.vout")
            .unwrap();
        let vout = trace.column(index);
        let at = |t: u64| vout[rows.binary_search(&t).unwrap()];
        // s1 closes at 10 ns. s2 closes at 10.5 ns. gmin is 1e-12 S.
        let one_closed = 1.0 / 1000.0 + 1e-9 + 1e-12;
        let both_closed = 2.0 / 1000.0 + 1e-12;
        for (start, end, g_load) in [
            (10_000, 10_500, one_closed),
            (10_500, 11_000, both_closed),
            (11_000, 11_500, both_closed),
        ] {
            let got = at(end);
            let want = backward_euler(at(start), end - start, g_load);
            assert!(
                (got - want).abs() < 1e-6,
                "{start} -> {end}: {got} vs backward Euler {want}"
            );
        }
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
    fn an_edge_at_1_ps_keeps_the_start_ramp_and_the_edge_ramp() {
        let netlist = divider_switch(1, 10_000, 10_000);
        assert_eq!(times(&netlist), [0, 1, 1_000, 1_001, 3_001, 7_001, 10_000]);
    }
}
