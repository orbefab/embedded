use std::collections::BTreeMap;

use sim_core::diagnostic::Diagnostic;
use sim_core::finite::Finite;
use sim_core::name::Ref;
use sim_core::netlist::Netlist;
use sim_core::run_folder::OpPoint;
use sim_core::time::Time;

use crate::element;
use crate::ladder::{self, Next, Rung};
use crate::mna::{Probe, Solution};
use crate::newton::{Circuit, Failure, Iterate, Offender, newton};
use crate::ratings::Ratings;

pub(crate) const OP_BUDGET: u32 = 100;

/// The operating point, with a finding per rating it breaks.
pub(crate) fn run(netlist: &Netlist, diagnostics: &mut Vec<Diagnostic>) -> Option<OpPoint> {
    match solve(netlist) {
        Ok(point) => {
            let mut ratings = Ratings::new(netlist);
            ratings.sample(Time::ZERO, &point.values);
            diagnostics.extend(ratings.finish());
            Some(point)
        }
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            None
        }
    }
}

fn solve(netlist: &Netlist) -> Result<OpPoint, Diagnostic> {
    let circuit = Circuit::new(netlist)?;
    let iterate = operating_point(&circuit)?;
    observe(&circuit, &iterate.solution)
}

/// Newton, then the fallback ladder. What comes back converged at `Conditions::NOMINAL`.
fn operating_point(circuit: &Circuit) -> Result<Iterate, Diagnostic> {
    climb(circuit, |_| OP_BUDGET)
}

/// The ladder driver. `budget` gives each rung's Newton budget, so a test can make a rung fail.
fn climb(circuit: &Circuit, budget: impl Fn(Rung) -> u32) -> Result<Iterate, Diagnostic> {
    let mut rung = Rung::Plain;
    let mut warm: Option<Iterate> = None;
    let mut iterations = 0;
    let mut gmin = None;
    let mut source_scale = 0.0;
    let mut offender = None;
    loop {
        let start = match &warm {
            Some(iterate) if rung.warm() => iterate.clone(),
            _ => Iterate::initial(circuit),
        };
        let converged = match newton(circuit, &rung.conditions(), start, budget(rung)) {
            Ok(converged) => {
                iterations += converged.iterations;
                match rung {
                    Rung::Gmin { extra, .. } if extra > 0.0 => gmin = Some(extra),
                    Rung::Source { scale, .. } => source_scale = scale,
                    _ => {}
                }
                warm = Some(converged.iterate);
                true
            }
            Err(Failure::Singular { row }) => {
                return Err(Diagnostic::SolverSingular {
                    at: Time::ZERO,
                    unknown: circuit.layout.describe(circuit.netlist, row),
                });
            }
            Err(Failure::Stalled(stall)) => {
                // Without a junction the system is linear, and no rung makes a non-finite answer finite.
                if let Offender::NonFinite(_) = stall.offender
                    && circuit.layout.junctions() == 0
                {
                    return Err(Diagnostic::SolverNonFinite {
                        at: Time::ZERO,
                        unknown: stall.offender.describe(circuit),
                    });
                }
                iterations += stall.iterations;
                offender = Some(stall.offender);
                false
            }
        };
        match ladder::next(rung, converged) {
            Next::Try(next) => rung = next,
            Next::Done => return Ok(warm.expect("the last rung converged")),
            Next::Exhausted => {
                let offender = offender.expect("an exhausted ladder had a stalled rung");
                return Err(Diagnostic::SolverConvergence {
                    at: Time::ZERO,
                    unknown: offender.describe(circuit),
                    iterations,
                    gmin,
                    source_scale,
                });
            }
        }
    }
}

fn observe(circuit: &Circuit, solution: &Solution) -> Result<OpPoint, Diagnostic> {
    let netlist = circuit.netlist;
    let mut values = BTreeMap::new();
    for (id, name) in netlist.nets() {
        let finite = Finite::new(solution.voltage(id)).expect("solution components are finite");
        values.insert(Ref::Net(name.clone()), finite);
    }
    for (id, part) in netlist.parts() {
        let probe = Probe {
            layout: &circuit.layout,
            part: id,
            x: solution,
        };
        element::observe(&part.model, &part.name, &probe, &mut values).map_err(|_| {
            Diagnostic::SolverNonFinite {
                at: Time::ZERO,
                unknown: Ref::Part(part.name.clone()),
            }
        })?;
    }
    Ok(OpPoint { values })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sim_core::netlist::{self, Netlist};

    use super::*;
    use crate::mna::GMIN;
    use crate::newton::Conditions;

    fn example(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name)
            .join("netlist.json");
        std::fs::read(path).unwrap()
    }

    fn parse(bytes: &[u8]) -> Netlist {
        netlist::parse(bytes).unwrap()
    }

    fn voltage(netlist: &Netlist, solution: &Solution, name: &str) -> f64 {
        let (id, _) = netlist
            .nets()
            .find(|(_, net)| net.as_str() == name)
            .unwrap();
        solution.voltage(id)
    }

    fn assert_same_point(left: &Iterate, right: &Iterate) {
        let pairs = left
            .solution
            .components()
            .iter()
            .zip(right.solution.components());
        for (row, (&a, &b)) in pairs.enumerate() {
            assert!((a - b).abs() <= 1e-9 * a.abs(), "row {row}: {a} vs {b}");
        }
    }

    #[test]
    fn gmin_stepping_reaches_the_plain_newton_point() {
        for name in ["led-5v", "led-9v"] {
            let bytes = example(name);
            let netlist = parse(&bytes);
            let circuit = Circuit::new(&netlist).unwrap();
            let plain = climb(&circuit, |_| OP_BUDGET).unwrap();
            let stepped = climb(&circuit, |rung| match rung {
                Rung::Plain | Rung::Source { .. } => 1,
                Rung::Gmin { .. } => OP_BUDGET,
            })
            .unwrap();
            assert_same_point(&plain, &stepped);
        }
    }

    #[test]
    fn source_stepping_reaches_the_plain_newton_point() {
        for name in ["led-5v", "led-9v"] {
            let bytes = example(name);
            let netlist = parse(&bytes);
            let circuit = Circuit::new(&netlist).unwrap();
            let plain = climb(&circuit, |_| OP_BUDGET).unwrap();
            let stepped = climb(&circuit, |rung| match rung {
                Rung::Plain | Rung::Gmin { .. } => 1,
                Rung::Source { .. } => OP_BUDGET,
            })
            .unwrap();
            assert_same_point(&plain, &stepped);
        }
    }

    #[test]
    fn an_exhausted_ladder_names_the_junction() {
        let netlist = parse(&example("led-9v"));
        let circuit = Circuit::new(&netlist).unwrap();
        let failed = climb(&circuit, |_| 1).unwrap_err();
        let text = serde_json::to_string(&failed).unwrap();
        assert_eq!(
            text,
            r##"{"code":"solver.convergence","severity":"error","refs":["#led1"],"t_ps":0,"message":"Newton did not converge at #led1 after 3 iterations; source stepping reached 0","data":{"gmin":null,"iterations":3,"source_scale":0.0}}"##
        );
    }

    #[test]
    fn a_singular_rung_ends_the_ladder() {
        let netlist = parse(
            &String::from_utf8(example("led-5v"))
                .unwrap()
                .replace(
                    r#""parts": ["#,
                    r#""parts": [
    {
      "name": "v2",
      "kind": "voltage_source",
      "pins": { "p": "vin", "n": "gnd" },
      "voltage": { "value": 3.0, "unit": "V", "source": "user" }
    },"#,
                )
                .into_bytes(),
        );
        let circuit = Circuit::new(&netlist).unwrap();
        let failed = climb(&circuit, |_| OP_BUDGET).unwrap_err();
        assert_eq!(failed.code(), "solver.singular");
        assert_eq!(failed.refs(), vec!["#v2".parse().unwrap()]);
    }

    #[test]
    fn source_scale_zero_turns_every_source_off() {
        let netlist = parse(&example("divider"));
        let circuit = Circuit::new(&netlist).unwrap();
        let conditions = Conditions {
            gmin: GMIN,
            source_scale: 0.0,
        };
        let converged =
            newton(&circuit, &conditions, Iterate::initial(&circuit), OP_BUDGET).unwrap();
        for (id, name) in netlist.nets() {
            let volts = converged.iterate.solution.voltage(id);
            assert!(volts.abs() <= 1e-12, "{} is {volts}", name.as_str());
        }
    }

    #[test]
    fn gmin_pulls_a_divider_toward_ground() {
        let netlist = parse(&example("divider"));
        let circuit = Circuit::new(&netlist).unwrap();
        let gmin = 1e-2;
        let conditions = Conditions {
            gmin,
            source_scale: 1.0,
        };
        let converged =
            newton(&circuit, &conditions, Iterate::initial(&circuit), OP_BUDGET).unwrap();
        let vout = voltage(&netlist, &converged.iterate.solution, "vout");
        let g1 = 1.0 / 1000.0;
        let g2 = 1.0 / 2000.0;
        let expected = g1 * 5.0 / (g1 + g2 + gmin);
        let unloaded = g1 * 5.0 / (g1 + g2);
        assert!((vout - expected).abs() <= 1e-9, "{vout} vs {expected}");
        assert!(vout < unloaded, "{vout} was not pulled below {unloaded}");
    }
}
