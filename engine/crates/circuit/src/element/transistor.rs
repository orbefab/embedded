use sim_core::netlist::Bjt;

use super::capacitor::stamp_capacitor;
use super::junction::{Shockley, mismatch, pnjlim};
use super::{CapacitorEnd, OwnedCapacitor, Update};
use crate::mna::{GMIN, Node, Probe, Stamp};

/// A `Bjt`'s kind. It sets the sign of every junction voltage and pin current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Polarity {
    Npn,
    Pnp,
}

impl Polarity {
    fn sign(self) -> f64 {
        match self {
            Polarity::Npn => 1.0,
            Polarity::Pnp => -1.0,
        }
    }

    /// The internal node ngspice-42's default substrate junction joins to ground, through
    /// `GMIN`. It is vertical for an `npn` and lateral for a `pnp`.
    fn substrate(self) -> usize {
        match self {
            Polarity::Npn => C,
            Polarity::Pnp => B,
        }
    }
}

/// Internal nodes `c'`, `b'` and `e'`, behind `RC`, `RB` and `RE`.
const C: usize = 0;
const B: usize = 1;
const E: usize = 2;

/// Junction slots, in the `npn` frame.
const BE: usize = 0;
const BC: usize = 1;

struct Params {
    be: Shockley,
    bc: Shockley,
    bf: f64,
    br: f64,
    vaf: f64,
}

/// The terminal currents in the `npn` frame at one bias, and their slopes on `vbe` and `vbc`.
#[derive(Debug)]
struct Point {
    ic: f64,
    ib: f64,
    ic_be: f64,
    ic_bc: f64,
    ib_be: f64,
    ib_bc: f64,
    /// Either exponent was clamped.
    capped: bool,
    /// The voltages evaluated. A clamp moves them.
    vbe: f64,
    vbc: f64,
}

impl Params {
    fn new(bjt: &Bjt) -> Params {
        let is = bjt.saturation_current.si.get();
        Params {
            be: Shockley::new(bjt.forward_emission.si.get(), is),
            bc: Shockley::new(bjt.reverse_emission.si.get(), is),
            bf: bjt.forward_beta.si.get(),
            br: bjt.reverse_beta.si.get(),
            vaf: bjt.early_voltage.si.get(),
        }
    }

    /// The `Bjt` equations at `vbe` and `vbc`, with `GMIN` across each junction.
    fn point(&self, vbe: f64, vbc: f64) -> Point {
        let (ibe, gbe, be_capped, vbe) = self.be.evaluate(vbe);
        let (ibc, gbc, bc_capped, vbc) = self.bc.evaluate(vbc);
        let early = 1.0 - vbc / self.vaf;
        Point {
            ic: (ibe - ibc) * early - ibc / self.br - GMIN * vbc,
            ib: ibe / self.bf + ibc / self.br + GMIN * (vbe + vbc),
            ic_be: gbe * early,
            ic_bc: -gbc * early - (ibe - ibc) / self.vaf - gbc / self.br - GMIN,
            ib_be: gbe / self.bf + GMIN,
            ib_bc: gbc / self.br + GMIN,
            capped: be_capped || bc_capped,
            vbe,
            vbc,
        }
    }
}

/// `CJE` from `b'` to `e'`, then `CJC` from `b'` to `c'`, on branches 0 and 1.
fn capacitors(bjt: &Bjt) -> [OwnedCapacitor; 2] {
    let internal = |a, b| [CapacitorEnd::Internal(a), CapacitorEnd::Internal(b)];
    [
        OwnedCapacitor {
            k: 0,
            ends: internal(B, E),
            farad: bjt.base_emitter_capacitance.si.get(),
        },
        OwnedCapacitor {
            k: 1,
            ends: internal(B, C),
            farad: bjt.base_collector_capacitance.si.get(),
        },
    ]
}

pub(super) fn charges(bjt: &Bjt) -> Vec<OwnedCapacitor> {
    capacitors(bjt).to_vec()
}

/// Each series resistance, the substrate's `GMIN`, the two junctions linearised at the voltages
/// `point` evaluated, and the two capacitors.
pub(super) fn stamp(bjt: &Bjt, polarity: Polarity, stamp: &mut Stamp<'_>) {
    let params = Params::new(bjt);
    let [c, b, e] = bjt.pins;
    let (ci, bi, ei) = (stamp.internal(C), stamp.internal(B), stamp.internal(E));
    stamp.conductance(c, ci, 1.0 / bjt.collector_resistance.si.get());
    stamp.conductance(b, bi, 1.0 / bjt.base_resistance.si.get());
    stamp.conductance(e, ei, 1.0 / bjt.emitter_resistance.si.get());
    let substrate = stamp.internal(polarity.substrate());
    stamp.conductance(substrate, Node::Ground, GMIN);
    let p = params.point(stamp.junction(BE), stamp.junction(BC));
    let s = polarity.sign();
    // The current into `c'` or `b'` is `s * k + d_be * (V(b') - V(e')) + d_bc * (V(b') - V(c'))`.
    // The slopes are the `npn` frame's, because both voltage and current change sign.
    let collector = (
        ci,
        p.ic_be,
        p.ic_bc,
        p.ic - p.ic_be * p.vbe - p.ic_bc * p.vbc,
    );
    let base = (
        bi,
        p.ib_be,
        p.ib_bc,
        p.ib - p.ib_be * p.vbe - p.ib_bc * p.vbc,
    );
    for (row, d_be, d_bc, k) in [collector, base] {
        stamp.add(row, bi, d_be + d_bc);
        stamp.add(row, ei, -d_be);
        stamp.add(row, ci, -d_bc);
        stamp.add(ei, bi, -(d_be + d_bc));
        stamp.add(ei, ei, d_be);
        stamp.add(ei, ci, d_bc);
        stamp.current(row, ei, s * k);
    }
    for cap in capacitors(bjt) {
        stamp_capacitor(stamp, cap.k, cap.ends, cap.farad, None);
    }
}

/// `vbe` starts at its `vcrit` and `vbc` at 0.
pub(super) fn initial(bjt: &Bjt, out: &mut [f64]) {
    out[BE] = Params::new(bjt).be.vcrit;
    out[BC] = 0.0;
}

/// Limits each junction against its own `vcrit`, and checks both terminal currents.
pub(super) fn update(
    bjt: &Bjt,
    polarity: Polarity,
    probe: &Probe<'_>,
    prior: &[f64],
    next: &mut [f64],
) -> Update {
    let params = Params::new(bjt);
    let s = polarity.sign();
    let vb = probe.internal(B);
    let vbe = s * (vb - probe.internal(E));
    let vbc = s * (vb - probe.internal(C));
    let (be, be_limited) = pnjlim(vbe, prior[BE], params.be.nvt, params.be.vcrit);
    let (bc, bc_limited) = pnjlim(vbc, prior[BC], params.bc.nvt, params.bc.vcrit);
    next[BE] = be;
    next[BC] = bc;
    let old = params.point(prior[BE], prior[BC]);
    let new = params.point(vbe, vbc);
    let linear =
        |at: f64, d_be: f64, d_bc: f64| at + d_be * (vbe - old.vbe) + d_bc * (vbc - old.vbc);
    let collector = mismatch(linear(old.ic, old.ic_be, old.ic_bc), new.ic);
    let base = mismatch(linear(old.ib, old.ib_be, old.ib_bc), new.ib);
    Update {
        limited: be_limited || bc_limited || old.capped || new.capped,
        mismatch: collector.max(base),
    }
}

/// `ib`, `ic` and `p`. Each current enters its pin through its series resistance, so it
/// includes the capacitors' current.
pub(super) fn observe(bjt: &Bjt, probe: &Probe<'_>) -> [f64; 3] {
    let [c, b, e] = bjt.pins.map(|pin| probe.v(pin));
    let ib = (b - probe.internal(B)) / bjt.base_resistance.si.get();
    let ic = (c - probe.internal(C)) / bjt.collector_resistance.si.get();
    [ib, ic, (c - e) * ic + (b - e) * ib]
}

#[cfg(test)]
mod tests {
    use sim_core::name::{Name, Ref};
    use sim_core::netlist::{self, Junction, Model, Netlist, PartId, junction_scale};

    use super::*;
    use crate::mna::Solution;
    use crate::newton::Circuit;

    const IS: f64 = 8.095e-15;
    const BF: f64 = 173.2;
    const BR: f64 = 0.2704;
    const VAF: f64 = 100.0;
    const RB: f64 = 5.707;
    const RC: f64 = 1.531;
    const RE: f64 = 0.867;
    const CJE: f64 = 8e-12;
    const CJC: f64 = 4e-12;

    fn value(v: f64, unit: &str) -> String {
        format!(r#"{{ "value": {v:?}, "unit": "{unit}", "source": "user" }}"#)
    }

    /// `q1` with the `npn-led` example's values and `nr` for its reverse emission, its pins on
    /// `c`, `b` and `e`.
    fn q1_part(kind: &str, pins: [&str; 3], nr: f64) -> String {
        let [c, b, e] = pins;
        format!(
            r#"{{ "name": "q1", "kind": "{kind}",
                "pins": {{ "collector": "{c}", "base": "{b}", "emitter": "{e}" }},
                "saturation_current": {is}, "forward_beta": {bf}, "reverse_beta": {br},
                "forward_emission": {one}, "reverse_emission": {nr},
                "early_voltage": {vaf}, "base_resistance": {rb},
                "collector_resistance": {rc}, "emitter_resistance": {re},
                "base_emitter_capacitance": {cje}, "base_collector_capacitance": {cjc},
                "ratings": {{ "current": null, "power": null }} }}"#,
            is = value(IS, "A"),
            bf = value(BF, "1"),
            br = value(BR, "1"),
            one = value(1.0, "1"),
            nr = value(nr, "1"),
            vaf = value(VAF, "V"),
            rb = value(RB, "ohm"),
            rc = value(RC, "ohm"),
            re = value(RE, "ohm"),
            cje = value(CJE, "F"),
            cjc = value(CJC, "F"),
        )
    }

    /// `parts` on `nets` and ground.
    fn assemble(analysis: &str, nets: &[&str], parts: &[String]) -> Netlist {
        let mut nets: Vec<&str> = nets.iter().copied().filter(|net| *net != "gnd").collect();
        nets.sort_unstable();
        nets.dedup();
        let nets: Vec<String> = nets.iter().map(|net| format!("\"{net}\"")).collect();
        let json = format!(
            r#"{{ "version": 0, "seed": 0, "analysis": {analysis}, "ground": "gnd",
                "nets": ["gnd", {}], "parts": [{}] }}"#,
            nets.join(", "),
            parts.join(", ")
        );
        netlist::parse(json.as_bytes()).unwrap()
    }

    /// `q1` and a source from each of `sources` to ground.
    fn circuit(kind: &str, pins: [&str; 3], nr: f64, sources: &[(&str, f64)]) -> Netlist {
        let mut parts = vec![q1_part(kind, pins, nr)];
        for (net, volts) in sources {
            parts.push(format!(
                r#"{{ "name": "v_{net}", "kind": "voltage_source",
                    "pins": {{ "p": "{net}", "n": "gnd" }}, "voltage": {} }}"#,
                value(*volts, "V")
            ));
        }
        assemble(r#"{ "op": {} }"#, &pins, &parts)
    }

    fn q1(netlist: &Netlist) -> (PartId, &Bjt) {
        let id = netlist.part_id(&Name::new("q1").unwrap()).unwrap();
        let (_, part) = netlist.parts().nth(id.index()).unwrap();
        match &part.model {
            Model::Npn(bjt) | Model::Pnp(bjt) => (id, bjt),
            other => panic!("{other:?}"),
        }
    }

    fn npn() -> Netlist {
        circuit("npn", ["c", "b", "gnd"], 1.0, &[])
    }

    /// Active, saturated, reverse active, cut off and in between.
    const BIASES: [(f64, f64); 12] = [
        (0.65, -5.0),
        (0.72, -0.2),
        (0.75, -1.0),
        (0.7, 0.6),
        (0.8, 0.7),
        (0.5, 0.65),
        (-0.5, 0.6),
        (0.3, 0.3),
        (0.6, 0.0),
        (0.0, 0.0),
        (0.0, -5.0),
        (-1.0, -10.0),
    ];

    #[test]
    fn currents_match_the_closed_form() {
        let netlist = npn();
        let params = Params::new(q1(&netlist).1);
        let nvt = Junction::VT;
        for (vbe, vbc) in BIASES {
            let ibe = IS * (libm::exp(vbe / nvt) - 1.0);
            let ibc = IS * (libm::exp(vbc / nvt) - 1.0);
            let ic = (ibe - ibc) * (1.0 - vbc / VAF) - ibc / BR - GMIN * vbc;
            let ib = ibe / BF + ibc / BR + GMIN * (vbe + vbc);
            let p = params.point(vbe, vbc);
            assert_eq!((p.ic, p.ib), (ic, ib), "at vbe {vbe} V, vbc {vbc} V");
        }
    }

    #[test]
    fn partials_match_a_difference() {
        let netlist = npn();
        let params = Params::new(q1(&netlist).1);
        let h = 1e-6;
        for (vbe, vbc) in BIASES {
            let p = params.point(vbe, vbc);
            let slope = |current: fn(&Point) -> f64, dbe: f64, dbc: f64| {
                let up = current(&params.point(vbe + dbe, vbc + dbc));
                let down = current(&params.point(vbe - dbe, vbc - dbc));
                (up - down) / (2.0 * h)
            };
            let ic: fn(&Point) -> f64 = |p| p.ic;
            let ib: fn(&Point) -> f64 = |p| p.ib;
            // The difference rounds by about `EPSILON * |current| / h`.
            for (name, got, want, current) in [
                ("ic_be", p.ic_be, slope(ic, h, 0.0), p.ic),
                ("ic_bc", p.ic_bc, slope(ic, 0.0, h), p.ic),
                ("ib_be", p.ib_be, slope(ib, h, 0.0), p.ib),
                ("ib_bc", p.ib_bc, slope(ib, 0.0, h), p.ib),
            ] {
                let rounding = 4.0 * f64::EPSILON * current.abs() / h;
                assert!(
                    (got - want).abs() <= 1e-6 * want.abs() + rounding,
                    "{name} at vbe {vbe} V, vbc {vbc} V: {got} vs {want}"
                );
            }
        }
    }

    fn solved(netlist: &Netlist) -> [f64; 3] {
        let results = crate::simulate(netlist);
        assert!(
            results.diagnostics().is_empty(),
            "{:?}",
            results.diagnostics()
        );
        let op = results.observations().op().unwrap();
        ["#q1.ib", "#q1.ic", "#q1.p"].map(|key| op.values[&key.parse::<Ref>().unwrap()].get())
    }

    /// The substrate's `GMIN` is on `c'` in an `npn` and on `b'` in a `pnp`. So each current may
    /// differ by `GMIN` times the pin voltages, and `p` by that times `vc`, beside 1e-9 of each
    /// value for the different path Newton takes.
    #[test]
    fn pnp_is_npn_mirrored() {
        for (vb, vc) in [(0.75, 5.0), (0.8, 0.1)] {
            let bias = |kind, s: f64| {
                circuit(
                    kind,
                    ["c", "b", "gnd"],
                    1.0,
                    &[("b", s * vb), ("c", s * vc)],
                )
            };
            let [ib, ic, p] = solved(&bias("npn", 1.0));
            assert!(ic > 1e-3, "{ic}");
            let pnp = solved(&bias("pnp", -1.0));
            let leak = GMIN * (vb + vc);
            for ((got, want), slack) in
                pnp.into_iter()
                    .zip([-ib, -ic, p])
                    .zip([leak, leak, leak * vc])
            {
                assert!(
                    (got - want).abs() <= 1e-9 * want.abs() + slack,
                    "at {vb} V, {vc} V: {pnp:?} against {:?}",
                    [-ib, -ic, p]
                );
            }
        }
    }

    /// A probe of `q1` with `v` on `c'`, `b'` and `e'`.
    fn internal(circuit: &Circuit, v: [f64; 3]) -> Solution {
        let (id, _) = q1(&circuit.netlist);
        let mut x = Solution::zeros(circuit.layout.size());
        for (k, volts) in v.into_iter().enumerate() {
            let Node::Row(row) = circuit.layout.internal(id, k) else {
                panic!("an internal node has a row");
            };
            x.add(row, volts);
        }
        x
    }

    fn update_at(
        circuit: &Circuit,
        polarity: Polarity,
        v: [f64; 3],
        prior: [f64; 2],
    ) -> ([f64; 2], Update) {
        let (id, bjt) = q1(&circuit.netlist);
        let x = internal(circuit, v);
        let probe = Probe {
            layout: &circuit.layout,
            part: id,
            x: &x,
        };
        let mut next = [f64::NAN; 2];
        let report = update(bjt, polarity, &probe, &prior, &mut next);
        (next, report)
    }

    #[test]
    fn each_junction_limits_against_its_own_vcrit() {
        let circuit = Circuit::new(circuit("npn", ["c", "b", "gnd"], 2.0, &[])).unwrap();
        let be = junction_scale(1.0, IS).unwrap();
        let bc = junction_scale(2.0, IS).unwrap();
        assert!(be.vcrit < 1.0 && 1.0 < bc.vcrit && bc.vcrit < 2.0);
        let vbe = 1.0 - 0.4;
        let (next, report) = update_at(&circuit, Polarity::Npn, [0.0, 1.0, 0.4], [vbe, 0.0]);
        assert_eq!(next, [vbe, 1.0]);
        assert!(!report.limited);
        let (next, report) = update_at(&circuit, Polarity::Npn, [-1.0, 1.0, 0.4], [vbe, 0.0]);
        assert_eq!(next, [vbe, pnjlim(2.0, 0.0, bc.nvt, bc.vcrit).0]);
        assert!(report.limited);
    }

    #[test]
    fn a_converged_iterate_is_not_limited() {
        let circuit = Circuit::new(npn()).unwrap();
        for (polarity, s) in [(Polarity::Npn, 1.0), (Polarity::Pnp, -1.0)] {
            // At the second point `vbe - (vc - ve)` rounds one ulp away from `vb - vc`.
            for v in [
                [4.95, 0.65, 0.0],
                [4.95, 0.77, 0.01],
                [0.11, 0.76, 0.01],
                [-5.0, 0.0, 0.0],
            ] {
                let v = v.map(|volts| s * volts);
                let prior = [s * (v[B] - v[E]), s * (v[B] - v[C])];
                let (next, report) = update_at(&circuit, polarity, v, prior);
                assert_eq!(next.map(f64::to_bits), prior.map(f64::to_bits), "{v:?}");
                assert!(!report.limited, "{v:?}");
                assert_eq!(report.mismatch, 0.0, "{v:?}");
            }
        }
    }

    #[test]
    fn off_state_current_is_gmin() {
        let netlist = npn();
        let params = Params::new(q1(&netlist).1);
        let (vbe, vbc) = (0.0, -5.0);
        let ibc = IS * (libm::exp(vbc / Junction::VT) - 1.0);
        let p = params.point(vbe, vbc);
        let junctions = -ibc * (1.0 - vbc / VAF) - ibc / BR;
        assert!(junctions < 1e-13, "{junctions}");
        assert!((p.ic - junctions - 5.0 * GMIN).abs() <= 1e-25, "{}", p.ic);
        assert!((p.ib - ibc / BR + 5.0 * GMIN).abs() <= 1e-25, "{}", p.ib);
    }

    #[test]
    fn an_iterate_whose_ib_misses_its_linearisation_is_not_converged() {
        let circuit = Circuit::new(npn()).unwrap();
        let params = Params::new(q1(&circuit.netlist).1);
        // Forward active, `vbc` moves alone. `ib` misses by about what `ic` misses, and `ic` is
        // 170 times larger, so only `ib` is out of tolerance.
        let prior = [0.77, 0.37];
        let old = params.point(prior[BE], prior[BC]);
        let vbc = 0.77 - 0.39;
        let ic = mismatch(
            old.ic + old.ic_bc * (vbc - old.vbc),
            params.point(prior[BE], vbc).ic,
        );
        assert!(ic < 1.0, "{ic}");
        for (polarity, s) in [(Polarity::Npn, 1.0), (Polarity::Pnp, -1.0)] {
            let v = [0.39, 0.77, 0.0].map(|volts| s * volts);
            let (_, report) = update_at(&circuit, polarity, v, prior);
            assert!(!report.limited);
            assert!(report.mismatch > 1.0, "{}", report.mismatch);
        }
    }

    /// `q1`'s base stepped from 0 to -1 V through `ohm` at 1 us. `tied` is the pin wired to the
    /// base, and the third pin is on ground. Both junctions stay reverse biased.
    fn stepped(tied: usize, ohm: f64) -> Netlist {
        let mut pins = ["gnd", "b", "gnd"];
        pins[tied] = "b";
        let parts = [
            q1_part("npn", pins, 1.0),
            format!(
                r#"{{ "name": "vin", "kind": "voltage_source", "pins": {{ "p": "in", "n": "gnd" }},
                    "voltage": {{ "kind": "pulse", "initial": {}, "pulsed": {},
                        "delay_ps": 1000000, "width_ps": 10000000, "period_ps": 20000000 }} }}"#,
                value(0.0, "V"),
                value(-1.0, "V")
            ),
            format!(
                r#"{{ "name": "rs", "kind": "resistor", "pins": {{ "a": "in", "b": "b" }},
                    "resistance": {}, "ratings": {{ "power": null }} }}"#,
                value(ohm, "ohm")
            ),
        ];
        let analysis =
            r#"{ "transient": { "stop_ps": 6000000, "max_step_ps": 100000, "grid_ps": 10000 } }"#;
        assemble(analysis, &["b", "in"], &parts)
    }

    /// With the collector on the base only `CJE` sees the step, and with the emitter there only
    /// `CJC`. Each charges through the source's resistance and the series resistances on its
    /// two ends, and every row of `#net.b` follows that time constant.
    #[test]
    fn a_base_step_charges_cje_and_cjc_each_with_its_own_time_constant() {
        let ohm = 100e3;
        for (tied, tau) in [(C, (ohm + RB + RE) * CJE), (E, (ohm + RB + RC) * CJC)] {
            let results = crate::simulate(&stepped(tied, ohm));
            assert!(
                results.diagnostics().is_empty(),
                "{:?}",
                results.diagnostics()
            );
            let trace = results.trace().unwrap();
            let b = "#net.b".parse::<Ref>().unwrap();
            let column = trace.columns().iter().position(|r| *r == b).unwrap();
            let mut worst = 0.0f64;
            for (at, v) in trace.time().iter().zip(trace.column(column)) {
                let since = (at.as_ps() as f64 - 1e6).max(0.0) * 1e-12;
                let want = -(1.0 - libm::exp(-since / tau));
                worst = worst.max((v - want).abs());
            }
            assert!(worst <= 1e-3, "pin {tied} on the base: {worst} V off");
        }
    }
}
