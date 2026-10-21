use sim_core::diagnostic::{Diagnostic, Solve};
use sim_core::finite::Finite;
use sim_core::netlist::Netlist;
use sim_core::run_folder::OpPoint;
use sim_core::time::Time;

use sim_core::name::Ref;

use crate::active::{self, Actives};
use crate::chip;
use crate::ladder::{self, Next, Rung};
use crate::newton::{Circuit, Conditions, Failure, Iterate, Offender, Side, newton};
use crate::observe::{self, Columns};
use crate::ratings::{Ratings, Timing};
use crate::watch::Levels;

pub(crate) const OP_BUDGET: u32 = 100;

/// The most solves `settle` runs at one time before the chips are `chip.unsettled`. Chips
/// have no propagation delay, so two that keep flipping each other at one time never settle.
const SETTLE_ROUNDS: u32 = 8;

/// The operating point, with a finding per rating it breaks.
pub(crate) fn run(netlist: &Netlist, diagnostics: &mut Vec<Diagnostic>) -> Option<OpPoint> {
    let columns = Columns::new(netlist);
    let row = match solve(netlist, &columns) {
        Ok(row) => row,
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            return None;
        }
    };
    let mut ratings = Ratings::new(netlist, &columns, Timing::Point);
    ratings.sample(Time::ZERO, &row);
    diagnostics.extend(ratings.finish());
    Some(observe::op_point(&columns, &row))
}

fn solve(netlist: &Netlist, columns: &Columns) -> Result<Vec<Finite>, Diagnostic> {
    let circuit = Circuit::new(netlist)?;
    let start = Iterate::initial(&circuit);
    let settled = settle(
        &circuit,
        Conditions::NOMINAL,
        start,
        circuit.power_on(),
        &Actives::power_on(&circuit),
        None,
        OP_BUDGET,
    )?;
    observe::observe(
        &circuit,
        columns,
        &settled.iterate.solution,
        Time::ZERO,
        Side::Before,
    )
}

/// A solve the chips agree with: its chip states, and the levels they read on it.
pub(crate) struct Settled {
    pub(crate) iterate: Iterate,
    pub(crate) chips: Vec<chip::State>,
    pub(crate) levels: Levels,
}

/// The ladder at `base` with `chips` in force, from `start`, with `budget` per rung. Under
/// `History::Start` that is the operating point, and under `History::Restart` the operating
/// point with every capacitor held. Then every chip reacts to the levels on that solve, and
/// while any state changed it solves again. `prior` is the levels just before, `None` at
/// t = 0. Levels are read only on the solve that converged at `base`, never on a fallback
/// rung. Without a chip it is one climb.
pub(crate) fn settle(
    circuit: &Circuit,
    base: Conditions<'_>,
    start: Iterate,
    chips: Vec<chip::State>,
    cores: &[active::Core],
    prior: Option<&Levels>,
    budget: u32,
) -> Result<Settled, Diagnostic> {
    let mut chips = chips;
    let mut start = start;
    let mut prior = prior.cloned();
    let mut changed = Vec::new();
    for _ in 0..SETTLE_ROUNDS {
        let conditions = Conditions {
            chips: &chips,
            cores,
            ..base
        };
        let iterate = climb(circuit, conditions, start, |_| budget)?;
        let levels = circuit.watches.levels(&iterate.solution, prior.as_ref());
        let next = circuit.react(&chips, prior.as_ref().unwrap_or(&levels), &levels);
        if next == chips {
            return Ok(Settled {
                iterate,
                chips,
                levels,
            });
        }
        changed = circuit
            .chips()
            .zip(next.iter().zip(&chips))
            .filter(|(_, (new, old))| new != old)
            .map(|((id, _), _)| Ref::Part(circuit.netlist.part(id).name.clone()))
            .collect();
        chips = next;
        start = iterate;
        prior = Some(levels);
    }
    Err(Diagnostic::ChipUnsettled {
        at: base.at,
        chips: changed,
        rounds: SETTLE_ROUNDS,
    })
}

/// The ladder driver. `budget` gives each rung's Newton budget, so a test can make a rung fail.
pub(crate) fn climb(
    circuit: &Circuit,
    base: Conditions<'_>,
    start: Iterate,
    budget: impl Fn(Rung) -> u32,
) -> Result<Iterate, Diagnostic> {
    let mut rung = Rung::Plain;
    let mut warm: Option<Iterate> = None;
    let mut iterations = 0;
    let mut gmin = None;
    let mut source_scale = 0.0;
    let mut offender = None;
    loop {
        let from = match &warm {
            Some(iterate) if rung.warm() => iterate.clone(),
            _ => start.clone(),
        };
        let converged = match newton(circuit, &rung.conditions(base), from, budget(rung)) {
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
                    at: base.at,
                    unknown: circuit.layout.describe(circuit.netlist, row),
                });
            }
            Err(Failure::Stalled(stall)) => {
                // Without a junction the system is linear, and no rung makes a non-finite answer finite.
                if let Offender::NonFinite(_) = stall.offender
                    && circuit.layout.junctions() == 0
                {
                    return Err(Diagnostic::SolverNonFinite {
                        at: base.at,
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
                    at: base.at,
                    unknown: offender.describe(circuit),
                    iterations,
                    gmin,
                    source_scale,
                    solve: Solve::Ladder,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sim_core::netlist::{self, Netlist};

    use super::*;
    use crate::mna::Solution;

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

    fn climb_op(circuit: &Circuit, budget: impl Fn(Rung) -> u32) -> Result<Iterate, Diagnostic> {
        climb(
            circuit,
            Conditions::NOMINAL,
            Iterate::initial(circuit),
            budget,
        )
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
            let plain = climb_op(&circuit, |_| OP_BUDGET).unwrap();
            let stepped = climb_op(&circuit, |rung| match rung {
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
            let plain = climb_op(&circuit, |_| OP_BUDGET).unwrap();
            let stepped = climb_op(&circuit, |rung| match rung {
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
        let failed = climb_op(&circuit, |_| 1).unwrap_err();
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
        let failed = climb_op(&circuit, |_| OP_BUDGET).unwrap_err();
        assert_eq!(failed.code(), "solver.singular");
        assert_eq!(failed.refs(), vec!["#v2".parse().unwrap()]);
    }

    #[test]
    fn source_scale_zero_turns_every_source_off() {
        let divider = String::from_utf8(example("divider")).unwrap();
        let netlist = parse(
            divider
                .replace(
                    r#""parts": ["#,
                    r#""parts": [
    {
      "name": "i1",
      "kind": "current_source",
      "pins": { "p": "vout", "n": "gnd" },
      "current": { "value": 0.001, "unit": "A", "source": "user" }
    },"#,
                )
                .as_bytes(),
        );
        let circuit = Circuit::new(&netlist).unwrap();
        let conditions = Conditions {
            source_scale: 0.0,
            ..Conditions::NOMINAL
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
            ..Conditions::NOMINAL
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
