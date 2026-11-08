use orbe_engine_core::netlist::Zener;

use crate::element::Update;
use crate::element::junction::{Shockley, mismatch, pnjlim};
use crate::mna::{GMIN, Probe, Stamp};

fn forward(zener: &Zener) -> Shockley {
    Shockley::new(
        zener.emission_coefficient.si.get(),
        zener.saturation_current.si.get(),
    )
}

fn breakdown(zener: &Zener) -> Shockley {
    let derived = &zener.breakdown;
    Shockley::from_parts(derived.saturation_current, derived.nvt, derived.vcrit)
}

/// Rs from the anode to internal node 0, then the forward junction to the cathode.
/// Rb from internal node 1 to the cathode, then the breakdown junction back to node 0.
/// Each junction has its own `GMIN`. Neither internal node has node gmin.
pub(super) fn stamp(zener: &Zener, stamp: &mut Stamp<'_>) {
    let jf = forward(zener);
    let jb = breakdown(zener);
    let [anode, cathode] = zener.pins;
    let n0 = stamp.internal(0);
    let n1 = stamp.internal(1);
    stamp.conductance(anode, n0, 1.0 / zener.series_resistance.si.get());
    let forward_voltage = stamp.junction(0);
    let (id, gd, _, voltage) = jf.evaluate(forward_voltage);
    stamp.conductance(n0, cathode, gd + GMIN);
    stamp.current(n0, cathode, id - gd * voltage);
    stamp.conductance(n1, cathode, 1.0 / zener.breakdown_resistance.si.get());
    let breakdown_voltage = stamp.junction(1);
    let (ib, gb, _, voltage) = jb.evaluate(breakdown_voltage);
    stamp.conductance(n1, n0, gb + GMIN);
    stamp.current(n1, n0, ib - gb * voltage);
}

pub(super) fn initial(zener: &Zener) -> [f64; 2] {
    [forward(zener).vcrit, breakdown(zener).vcrit]
}

/// Limits each solved junction voltage against its own prior and its own `vcrit`.
pub(super) fn update(zener: &Zener, probe: &Probe<'_>, prior: &[f64]) -> ([f64; 2], Update) {
    let jf = forward(zener);
    let jb = breakdown(zener);
    let cathode = zener.pins[1];
    let solved_f = probe.internal(0) - probe.v(cathode);
    let solved_b = probe.internal(1) - probe.internal(0);
    let (next_f, limited_f) = pnjlim(solved_f, prior[0], jf.nvt, jf.vcrit);
    let (next_b, limited_b) = pnjlim(solved_b, prior[1], jb.nvt, jb.vcrit);
    let (mismatch_f, clamped_f) = junction_mismatch(&jf, prior[0], solved_f);
    let (mismatch_b, clamped_b) = junction_mismatch(&jb, prior[1], solved_b);
    (
        [next_f, next_b],
        Update {
            limited: limited_f || limited_b || clamped_f || clamped_b,
            mismatch: mismatch_f.max(mismatch_b),
        },
    )
}

fn junction_mismatch(params: &Shockley, prior: f64, solved: f64) -> (f64, bool) {
    let (id, gd, cap_prior, voltage) = params.evaluate(prior);
    let linear = id + gd * (solved - voltage);
    let (actual, _, cap_solved, _) = params.evaluate(solved);
    (mismatch(linear, actual), cap_prior || cap_solved)
}

/// The current through Rs, entering the anode.
pub(super) fn current(zener: &Zener, probe: &Probe<'_>) -> f64 {
    (probe.v(zener.pins[0]) - probe.internal(0)) / zener.series_resistance.si.get()
}

#[cfg(test)]
mod tests {
    use orbe_engine_core::netlist::{self, Model};

    use crate::element::junction::pnjlim;
    use crate::mna::{GMIN, Node, Probe, Solution, Stamp, System};
    use crate::newton::{Circuit, Conditions, Iterate};

    use super::update;

    fn part(body: &str) -> String {
        format!(
            r#"{{
              "version": 0,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": ["gnd", "c"],
              "parts": [
                {{
                  "name": "v1",
                  "kind": "voltage_source",
                  "pins": {{ "p": "c", "n": "gnd" }},
                  "voltage": {{ "value": 1.0, "unit": "V", "source": "user" }}
                }},
                {{
                  "name": "dz1",
                  "kind": "zener",
                  "pins": {{ "anode": "gnd", "cathode": "c" }},
                  {body}
                  "ratings": {{ "current": null, "power": null }}
                }}
              ]
            }}"#
        )
    }

    fn circuit() -> Circuit {
        let json = part(
            r#"
                  "saturation_current": { "value": 1e-20, "unit": "A", "source": "user" },
                  "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
                  "series_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
                  "zener_voltage": { "value": 5.0, "unit": "V", "source": "user" },
                  "zener_current": { "value": 0.05, "unit": "A", "source": "user" },
                  "zener_impedance": { "value": 10.0, "unit": "ohm", "source": "user" },
                  "breakdown_resistance": { "value": 4.0, "unit": "ohm", "source": "user" },
            "#,
        );
        Circuit::new(netlist::parse(json.as_bytes()).unwrap()).unwrap()
    }

    fn zener_id(circuit: &Circuit) -> orbe_engine_core::netlist::PartId {
        circuit
            .netlist
            .parts()
            .find(|(_, part)| part.name.as_str() == "dz1")
            .unwrap()
            .0
    }

    /// Currents leaving the two internal nodes, with `g_forward` and `g_breakdown` the
    /// linearised junction conductances, cathode held at `vc`, anode at ground.
    fn linear_current(gs: f64, gf: f64, gb: f64, gr: f64, vc: f64) -> f64 {
        let a00 = gs + gf + gb;
        let a01 = -gb;
        let a10 = -gb;
        let a11 = gr + gb;
        let b0 = gf * vc;
        let b1 = gr * vc;
        let det = a00 * a11 - a01 * a10;
        let v0 = (b0 * a11 - a01 * b1) / det;
        -gs * v0
    }

    #[test]
    fn zener_junctions_have_their_own_gmin_and_vcrit() {
        let circuit = circuit();
        let id = zener_id(&circuit);
        let Model::Zener(zener) = &circuit.netlist.part(id).model else {
            panic!("not a zener");
        };
        let jf = super::forward(zener);
        let jb = super::breakdown(zener);
        assert_ne!(jf.vcrit, jb.vcrit);

        let mut iterate = Iterate::initial(&circuit);
        let slots = circuit.layout.junction_slots(id);
        assert_eq!(iterate.junctions[slots.start], jf.vcrit);
        assert_eq!(iterate.junctions[slots.start + 1], jb.vcrit);
        iterate.junctions.fill(0.0);

        let mut system = System::new(&circuit.layout);
        system.reset(&circuit.layout, GMIN);
        let conditions = Conditions::NOMINAL;
        for (part_id, part) in circuit.netlist.parts() {
            let mut stamp =
                Stamp::new(&mut system, &circuit.layout, part_id, &iterate, &conditions);
            crate::element::stamp(&part.model, &mut stamp);
        }
        let solution = system.solve().unwrap();
        let probe = Probe {
            layout: &circuit.layout,
            part: id,
            x: &solution,
        };
        let got = super::current(zener, &probe);
        let (_, gf, _, _) = jf.evaluate(0.0);
        let (_, gb, _, _) = jb.evaluate(0.0);
        let rs = zener.series_resistance.si.get();
        let rb = zener.breakdown_resistance.si.get();
        let both = linear_current(1.0 / rs, gf + GMIN, gb + GMIN, 1.0 / rb, 1.0);
        let one = linear_current(1.0 / rs, gf + GMIN, gb, 1.0 / rb, 1.0);
        assert!(
            (both - one).abs() > 1e-15,
            "one GMIN is not distinguishable"
        );
        assert!(
            (got - both).abs() < 1e-18,
            "current {got} is not the two-GMIN current {both}"
        );

        let mut x = vec![0.0; circuit.layout.size()];
        let solved = jb.vcrit + 0.01;
        let Node::Row(row) = circuit.layout.internal(id, 1) else {
            panic!("internal node 1 is a row");
        };
        x[row] = solved;
        let solution = Solution::from(x);
        let probe = Probe {
            layout: &circuit.layout,
            part: id,
            x: &solution,
        };
        let (next, _) = update(zener, &probe, &[0.0, 100.0]);
        let (own, _) = pnjlim(solved, 100.0, jb.nvt, jb.vcrit);
        let (forward_limit, _) = pnjlim(solved, 100.0, jb.nvt, jf.vcrit);
        assert_ne!(own, forward_limit);
        assert_eq!(next[1], own);
    }

    #[test]
    fn zener_overload_converges_on_plain_newton() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../cli/tests/fixtures/zener-overload/netlist.json");
        let bytes = std::fs::read(path).unwrap();
        let circuit = Circuit::new(netlist::parse(&bytes).unwrap()).unwrap();
        let converged = crate::newton::newton(
            &circuit,
            None,
            &Conditions::NOMINAL,
            Iterate::initial(&circuit),
            crate::op::OP_BUDGET,
        )
        .expect("plain Newton");
        eprintln!("zener-overload iterations {}", converged.iterations);
        assert_eq!(converged.iterations, 6);
    }
}
