use orbe_model::Time;

use crate::netlist::Netlist;

use crate::diagnostic::{Diagnostic, Solve};
use crate::finite::Finite;

use crate::run_folder::OpPoint;

use crate::name::Ref;

use crate::active::{self, Actives};
use crate::chip;
use crate::ladder::{self, Next, Rung};
use crate::mna::Solver;
use crate::newton::{Circuit, Conditions, Failure, Iterate, Side, newton};
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
    let (circuit, cores, row) = match solve(netlist, &columns) {
        Ok(solved) => solved,
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            return None;
        }
    };
    let mut ratings = Ratings::new(netlist, &circuit.actives, &columns, Timing::Point);
    ratings.sample(Time::ZERO, &row, &cores);
    diagnostics.extend(ratings.findings());
    Some(observe::op_point(&columns, &row))
}

/// The operating point's row, with the circuit and the power-on cores it was solved with.
fn solve(
    netlist: &Netlist,
    columns: &Columns,
) -> Result<(Circuit, Vec<active::Core>, Vec<Finite>), Diagnostic> {
    let circuit = Circuit::new(netlist.clone())?;
    let start = Iterate::initial(&circuit);
    let cores = Actives::power_on(&circuit);
    let settled = settle(
        &circuit,
        &mut Solver::new(&circuit.layout),
        Conditions::NOMINAL,
        start,
        None,
        circuit.power_on(),
        &cores,
        None,
        OP_BUDGET,
    )?;
    let row = observe::observe(
        &circuit,
        columns,
        &settled.iterate.solution,
        Time::ZERO,
        Side::Before,
    )?;
    Ok((circuit, cores, row))
}

/// A solve the chips agree with: its chip states, and the levels they read on it.
pub(crate) struct Settled {
    pub(crate) iterate: Iterate,
    pub(crate) chips: Vec<chip::State>,
    pub(crate) levels: Levels,
    /// Whether the first solve converged from the guess.
    pub(crate) guessed: bool,
}

/// The ladder at `base` with `chips` in force, from `start`, with `budget` per rung. Under
/// `History::Start` that is the operating point, and under `History::Restart` the operating
/// point with every capacitor held. Then every chip reacts to the levels on that solve, and
/// while any state changed it solves again. `prior` is the levels just before, `None` at
/// t = 0. Levels are read only on the solve that converged at `base`, never on a fallback
/// rung. Without a chip it is one climb.
///
/// A `guess` starts the first solve instead of `start`, Newton alone with
/// [`GUESS_BUDGET`](crate::session::predict::GUESS_BUDGET). When that does not converge the
/// first solve is the climb from `start`, as without a guess.
#[allow(clippy::too_many_arguments)]
pub(crate) fn settle(
    circuit: &Circuit,
    solver: &mut Solver,
    base: Conditions<'_>,
    start: Iterate,
    guess: Option<Iterate>,
    chips: Vec<chip::State>,
    cores: &[active::Core],
    prior: Option<&Levels>,
    budget: u32,
) -> Result<Settled, Diagnostic> {
    let mut chips = chips;
    let mut start = start;
    let mut guess = guess;
    let mut guessed = false;
    let mut prior = prior.cloned();
    let mut changed = Vec::new();
    for _ in 0..SETTLE_ROUNDS {
        let conditions = Conditions {
            chips: &chips,
            cores,
            ..base
        };
        let budget_from_guess = crate::session::predict::GUESS_BUDGET.min(budget);
        let iterate = match guess
            .take()
            .map(|guess| newton(circuit, solver, &conditions, guess, budget_from_guess))
        {
            Some(Ok(converged)) => {
                guessed = true;
                converged.iterate
            }
            Some(Err(_)) | None => climb(circuit, solver, conditions, start, |_| budget)?,
        };
        let levels = circuit.watches.levels(&iterate.solution, prior.as_ref());
        let next = circuit.react(&chips, prior.as_ref().unwrap_or(&levels), &levels);
        if next == chips {
            return Ok(Settled {
                iterate,
                chips,
                levels,
                guessed,
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
    solver: &mut Solver,
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
        let converged = match newton(circuit, solver, &rung.conditions(base), from, budget(rung)) {
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
                    unknown: circuit.layout.describe(&circuit.netlist, row),
                });
            }
            Err(Failure::NonFinite { row }) => {
                return Err(Diagnostic::SolverNonFinite {
                    at: base.at,
                    unknown: circuit.layout.describe(&circuit.netlist, row),
                });
            }
            Err(Failure::Stalled(stall)) => {
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
    use crate::netlist::{self, Appended, Netlist};

    use std::path::PathBuf;

    use crate::finite::Finite;
    use crate::name::Name;

    use super::*;
    use crate::mna::{SOLVES, Solution};
    use crate::newton::{Converged, History, Integration};

    fn example(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/circuits")
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
            &mut Solver::new(&circuit.layout),
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
            let circuit = Circuit::new(netlist.clone()).unwrap();
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
            let circuit = Circuit::new(netlist.clone()).unwrap();
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
        let circuit = Circuit::new(netlist.clone()).unwrap();
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
      "voltage": { "value": 3.0, "unit": "V" }
    },"#,
                )
                .into_bytes(),
        );
        let circuit = Circuit::new(netlist.clone()).unwrap();
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
      "current": { "value": 0.001, "unit": "A" }
    },"#,
                )
                .as_bytes(),
        );
        let circuit = Circuit::new(netlist.clone()).unwrap();
        let conditions = Conditions {
            source_scale: 0.0,
            ..Conditions::NOMINAL
        };
        let converged = newton(
            &circuit,
            &mut Solver::new(&circuit.layout),
            &conditions,
            Iterate::initial(&circuit),
            OP_BUDGET,
        )
        .unwrap();
        for (id, name) in netlist.nets() {
            let volts = converged.iterate.solution.voltage(id);
            assert!(volts.abs() <= 1e-12, "{} is {volts}", name.as_str());
        }
    }

    fn solves() -> u64 {
        SOLVES.with(|solves| solves.get())
    }

    /// Newton from zeros, and the systems it solved.
    fn newton_from_zeros(name: &str) -> (Converged, u64) {
        let circuit = Circuit::new(parse(&example(name))).unwrap();
        let start = Iterate::initial(&circuit);
        let before = solves();
        let converged = newton(
            &circuit,
            &mut Solver::new(&circuit.layout),
            &Conditions::NOMINAL,
            start,
            OP_BUDGET,
        )
        .unwrap();
        (converged, solves() - before)
    }

    #[test]
    fn newton_on_a_circuit_without_a_junction_solves_once() {
        let (converged, solves) = newton_from_zeros("divider");
        assert_eq!((converged.iterations, solves), (2, 1));
    }

    #[test]
    fn newton_on_a_circuit_with_a_junction_solves_every_iteration() {
        let (converged, solves) = newton_from_zeros("led-5v");
        assert!(converged.iterations > 2, "{}", converged.iterations);
        assert_eq!(solves, u64::from(converged.iterations));
    }

    fn bits(converged: &Converged) -> Vec<u64> {
        let x = converged.iterate.solution.components();
        x.iter().map(|v| v.to_bits()).collect()
    }

    /// An RC on a steps source, stepped by backward Euler and held after each step. Every
    /// solve with the memo matches a solve without it to the bit, across the source's step at
    /// 9.9995 ms and a level appended at 14.95 ms the way a live `set` appends it.
    #[test]
    fn the_memo_answers_with_the_bits_of_a_fresh_solve() {
        let netlist = String::from_utf8(example("v-steps"))
            .unwrap()
            .replace(r#""t_ps": 9999500,"#, r#""t_ps": 9999500000,"#)
            .replace(r#""stop_ps": 20000000,"#, r#""stop_ps": 20000000000,"#);
        let mut circuit = Circuit::new(parse(netlist.as_bytes())).unwrap();
        let v1 = circuit.netlist.part_id(&Name::new("v1").unwrap()).unwrap();
        let h_ps = 100_000_000;
        let mut memo = Solver::new(&circuit.layout);
        let mut previous = Solution::zeros(circuit.layout.size());
        let mut vout = Vec::new();
        let mut memo_solves = [0; 3];
        for k in 1..=200 {
            if k == 150 {
                circuit.netlist.append(
                    v1,
                    Time::from_ps(14_950_000_000),
                    Appended::Level(Finite::new(3.0).unwrap()),
                );
            }
            let at = Time::from_ps(k * h_ps);
            let step = Conditions {
                at,
                history: History::Step(Integration {
                    h_ps,
                    coefficients: [1.0, -1.0, 0.0],
                    previous: &previous,
                    before: None,
                }),
                ..Conditions::NOMINAL
            };
            let mut solve = |conditions: &Conditions| {
                let start = Iterate {
                    solution: previous.clone(),
                    junctions: Vec::new(),
                };
                let before = solves();
                let warm =
                    newton(&circuit, &mut memo, conditions, start.clone(), OP_BUDGET).unwrap();
                memo_solves[usize::from(k >= 100) + usize::from(k >= 150)] += solves() - before;
                let cold = newton(
                    &circuit,
                    &mut Solver::new(&circuit.layout),
                    conditions,
                    start,
                    OP_BUDGET,
                )
                .unwrap();
                assert_eq!(bits(&warm), bits(&cold), "at step {k}");
                assert_eq!(warm.iterations, cold.iterations, "at step {k}");
                warm
            };
            let stepped = solve(&step);
            let held = solve(&Conditions {
                at,
                history: History::Restart {
                    previous: &stepped.iterate.solution,
                },
                ..Conditions::NOMINAL
            });
            vout.push(held.iterate.solution.get(1));
            previous = stepped.iterate.solution;
        }
        for (k, volts) in [(99, 5.0), (149, 1.0), (200, 3.0)] {
            assert!(
                (vout[k - 1] - volts).abs() < 1e-6,
                "{} at step {k}",
                vout[k - 1]
            );
        }
        // Each window makes 100 calls or more, and once the RC settles its systems repeat.
        assert!(memo_solves.iter().all(|&n| n < 50), "{memo_solves:?}");
    }

    #[test]
    fn gmin_pulls_a_divider_toward_ground() {
        let netlist = parse(&example("divider"));
        let circuit = Circuit::new(netlist.clone()).unwrap();
        let gmin = 1e-2;
        let conditions = Conditions {
            gmin,
            ..Conditions::NOMINAL
        };
        let converged = newton(
            &circuit,
            &mut Solver::new(&circuit.layout),
            &conditions,
            Iterate::initial(&circuit),
            OP_BUDGET,
        )
        .unwrap();
        let vout = voltage(&netlist, &converged.iterate.solution, "vout");
        let g1 = 1.0 / 1000.0;
        let g2 = 1.0 / 2000.0;
        let expected = g1 * 5.0 / (g1 + g2 + gmin);
        let unloaded = g1 * 5.0 / (g1 + g2);
        assert!((vout - expected).abs() <= 1e-9, "{vout} vs {expected}");
        assert!(vout < unloaded, "{vout} was not pulled below {unloaded}");
    }
}
