use crate::diagnostic::Diagnostic;
use crate::diagnostic::ValueRule;
use crate::netlist::NetId;
use crate::netlist::Value;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireJunctionRatings;
use crate::netlist::json::WireValue;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_rating;
use crate::netlist::json::resolve_value;
use crate::semiconductor::junction_scale;
use serde::Deserialize;

use orbe_model::Time;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, StoreEnd, Update};
use crate::mna::{GMIN, Node, Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::ParseCtx;
use crate::netlist::{Conduction, Kind};
use crate::newton::Side;
use crate::passive::capacitor::stamp_capacitor;
use crate::semiconductor::junction::{Shockley, mismatch, pnjlim};

/// A `Bjt`'s kind. It sets the sign of every junction voltage and pin current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Polarity {
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
    let internal = |a, b| [StoreEnd::Internal(a), StoreEnd::Internal(b)];
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

pub(crate) fn charges(bjt: &Bjt) -> Vec<OwnedCapacitor> {
    capacitors(bjt).to_vec()
}

/// Each series resistance, the substrate's `GMIN`, the two junctions linearised at the voltages
/// `point` evaluated, and the two capacitors.
pub(crate) fn stamp(bjt: &Bjt, polarity: Polarity, stamp: &mut Stamp<'_>) {
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
pub(crate) fn initial(bjt: &Bjt, out: &mut [f64]) {
    out[BE] = Params::new(bjt).be.vcrit;
    out[BC] = 0.0;
}

/// Limits each junction against its own `vcrit`, and checks both terminal currents.
pub(crate) fn update(
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
pub(crate) fn observe(bjt: &Bjt, probe: &Probe<'_>) -> [f64; 3] {
    let [c, b, e] = bjt.pins.map(|pin| probe.v(pin));
    let ib = (b - probe.internal(B)) / bjt.base_resistance.si.get();
    let ic = (c - probe.internal(C)) / bjt.collector_resistance.si.get();
    [ib, ic, (c - e) * ic + (b - e) * ib]
}

/// Which square-law region `forward` is in. `vds` is non-negative there.
enum Region {
    Cutoff,
    Linear,
    Saturation,
}

fn region(vov: f64, vds: f64) -> Region {
    if vov <= 0.0 {
        Region::Cutoff
    } else if vds >= vov {
        Region::Saturation
    } else {
        Region::Linear
    }
}

/// Level-1 parameters. `vds < 0` is a terminal swap in `id`, then one `GMIN` from drain to source.
struct Level1 {
    kp: f64,
    vto: f64,
    lambda: f64,
}

impl Level1 {
    fn new(mosfet: &Mosfet) -> Level1 {
        Level1 {
            kp: mosfet.transconductance_parameter.si.get(),
            vto: mosfet.threshold_voltage.si.get(),
            lambda: mosfet.channel_length_modulation.si.get(),
        }
    }

    /// Drain current and its partials on `vgs` and `vds`, for `vds >= 0`, before `GMIN`.
    fn forward(&self, vgs: f64, vds: f64) -> (f64, f64, f64) {
        let vov = vgs - self.vto;
        match region(vov, vds) {
            Region::Cutoff => (0.0, 0.0, 0.0),
            Region::Saturation => {
                let half = self.kp * vov * vov / 2.0;
                let k = 1.0 + self.lambda * vds;
                (half * k, self.kp * vov * k, half * self.lambda)
            }
            Region::Linear => {
                let k = 1.0 + self.lambda * vds;
                let core = self.kp * vds * (vov - vds / 2.0);
                (
                    core * k,
                    self.kp * vds * k,
                    self.kp * (vov - vds) * k + core * self.lambda,
                )
            }
        }
    }

    /// Current into the drain, `gm = d/dvgs` and `gds = d/dvds`.
    ///
    /// ngspice-42's `mos1load` puts `CKTgmin` on the drain-bulk diode. With the bulk on the
    /// source and `is = 0` that diode is `GMIN` from drain to source, and the source-bulk term
    /// cancels. There is no internal node, so nothing else is added.
    fn id(&self, vgs: f64, vds: f64) -> (f64, f64, f64) {
        let (id, gm, gds) = if vds >= 0.0 {
            self.forward(vgs, vds)
        } else {
            let (f, gm_s, gds_s) = self.forward(vgs - vds, -vds);
            (-f, -gm_s, gm_s + gds_s)
        };
        (id + GMIN * vds, gm, gds + GMIN)
    }
}

/// Slots `vgs`, then `vds`.
const VGS: usize = 0;
const VDS: usize = 1;

/// `CGS` on branch 0, `CGD` on branch 1, `CDS` on branch 2. The branch current enters the first end.
fn mos_capacitors(mosfet: &Mosfet) -> [OwnedCapacitor; 3] {
    let [d, g, s] = mosfet.pins;
    let between = |k, a, b, farad| OwnedCapacitor {
        k,
        ends: [StoreEnd::Net(a), StoreEnd::Net(b)],
        farad,
    };
    [
        between(0, g, s, mosfet.gate_source_capacitance.si.get()),
        between(1, g, d, mosfet.gate_drain_capacitance.si.get()),
        between(2, d, s, mosfet.drain_source_capacitance.si.get()),
    ]
}

/// Writes the solved terminal voltages through, and checks the drain current.
///
/// No limiter. A pass that rebuilt `vds` as `vgs - vgd` moved the last bit and never converged.
pub(crate) fn mos_update(
    mosfet: &Mosfet,
    probe: &Probe<'_>,
    prior: &[f64],
    next: &mut [f64],
) -> Update {
    let model = Level1::new(mosfet);
    let vs = probe.v(mosfet.pins[2]);
    let vgs = probe.v(mosfet.pins[1]) - vs;
    let vds = probe.v(mosfet.pins[0]) - vs;
    next[VGS] = vgs;
    next[VDS] = vds;
    let (id, gm, gds) = model.id(prior[VGS], prior[VDS]);
    let linear = id + gm * (vgs - prior[VGS]) + gds * (vds - prior[VDS]);
    let (actual, _, _) = model.id(vgs, vds);
    Update {
        limited: false,
        mismatch: mismatch(linear, actual),
    }
}

#[cfg(test)]
mod tests {
    use crate::netlist::{self, Model, Netlist, PartId};
    use crate::semiconductor::{Junction, junction_scale};

    use crate::name::{Name, Ref};

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
            Model::Npn(Npn(bjt)) | Model::Pnp(Pnp(bjt)) => (id, bjt),
            other => panic!("{other:?}"),
        }
    }

    fn npn() -> Netlist {
        circuit("npn", ["c", "b", "gnd"], 1.0, &[])
    }

    /// Active, saturated, reverse active, cut off and in between, and each junction alone below
    /// `-3 * VT`.
    const BIASES: [(f64, f64); 14] = [
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
        (-0.13, 0.0),
        (0.0, -0.13),
    ];

    /// One junction's current at `IS`. The exponential down to `-3 * nvt`, and ngspice-42's
    /// `bjtload` cubic below it.
    fn law(v: f64, nvt: f64) -> f64 {
        if v >= -3.0 * nvt {
            IS * (libm::exp(v / nvt) - 1.0)
        } else {
            let arg = 3.0 * nvt / (v * std::f64::consts::E);
            -IS * (1.0 + arg * arg * arg)
        }
    }

    /// `ic` and `ib` of the closed form, with `NF` at 1 and `NR` at `nr`.
    fn closed_form(vbe: f64, vbc: f64, nr: f64) -> (f64, f64) {
        let ibe = law(vbe, Junction::VT);
        let ibc = law(vbc, nr * Junction::VT);
        (
            (ibe - ibc) * (1.0 - vbc / VAF) - ibc / BR - GMIN * vbc,
            ibe / BF + ibc / BR + GMIN * (vbe + vbc),
        )
    }

    #[test]
    fn currents_match_the_closed_form() {
        for nr in [1.0, 2.0] {
            let netlist = circuit("npn", ["c", "b", "gnd"], nr, &[]);
            let params = Params::new(q1(&netlist).1);
            for (vbe, vbc) in BIASES {
                let p = params.point(vbe, vbc);
                assert_eq!(
                    (p.ic, p.ib),
                    closed_form(vbe, vbc, nr),
                    "at vbe {vbe} V, vbc {vbc} V, nr {nr}"
                );
            }
        }
    }

    /// `point` at `vbe` and `vbc` against the closed form, at a bias where the cubic is at least
    /// 1e-3 away from the exponential, so the exponential would fail.
    fn follows_the_reverse_law(vbe: f64, vbc: f64, nr: f64, junction: f64, nvt: f64) {
        let netlist = circuit("npn", ["c", "b", "gnd"], nr, &[]);
        let p = Params::new(q1(&netlist).1).point(vbe, vbc);
        let exponential = IS * (libm::exp(junction / nvt) - 1.0);
        let cubic = law(junction, nvt);
        assert!(
            (cubic - exponential).abs() > 1e-3 * cubic.abs(),
            "at {junction} V the laws agree"
        );
        assert_eq!(
            (p.ic, p.ib),
            closed_form(vbe, vbc, nr),
            "at vbe {vbe} V, vbc {vbc} V, nr {nr}"
        );
    }

    /// `NR` is 2, so `-0.1 V` to `-0.13 V` are below `-3 * NF*VT` and above `-3 * NR*VT`. A
    /// junction on the other's emission coefficient fails here or in the next test.
    #[test]
    fn a_reversed_base_emitter_junction_follows_ngspice_below_three_nf_vt() {
        for vbe in [-0.1, -0.12, -0.13, -0.2] {
            follows_the_reverse_law(vbe, 0.0, 2.0, vbe, Junction::VT);
        }
    }

    #[test]
    fn a_reversed_base_collector_junction_follows_ngspice_below_three_nr_vt() {
        let nvt = 2.0 * Junction::VT;
        for vbc in [-0.2, -0.25, -0.3, -0.4] {
            follows_the_reverse_law(0.0, vbc, 2.0, vbc, nvt);
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
        // `0.2 - (1.08 - 0.0)` and `vgs - (vgs - vds)` at `(0.01, 1.08, 0.1)` each move a bit.
        let mosfet = Circuit::new(open_mos(0.0)).unwrap();
        for v in [
            [0.2, 1.08, 0.0],
            [0.01, 1.08, 0.1],
            [5.0, 3.3, 0.0],
            [4.95, 2.53, 0.01],
        ] {
            let prior = [v[1] - v[2], v[0] - v[2]];
            let (next, report) = mos_update_at(&mosfet, v, prior);
            assert_eq!(next.map(f64::to_bits), prior.map(f64::to_bits), "{v:?}");
            assert!(!report.limited, "{v:?}");
            assert_eq!(report.mismatch, 0.0, "{v:?}");
        }
    }

    #[test]
    fn off_state_current_is_gmin() {
        let netlist = npn();
        let params = Params::new(q1(&netlist).1);
        let (vbe, vbc) = (0.0, -5.0);
        let ibc = law(vbc, Junction::VT);
        let p = params.point(vbe, vbc);
        let junctions = -ibc * (1.0 - vbc / VAF) - ibc / BR;
        assert!(junctions < 1e-13, "{junctions}");
        assert!((p.ic - junctions - 5.0 * GMIN).abs() <= 1e-25, "{}", p.ic);
        assert!((p.ib - ibc / BR + 5.0 * GMIN).abs() <= 1e-25, "{}", p.ib);
        let model = Level1::new(m1(&open_mos(0.0)).1);
        for vds in [5.0, -0.2] {
            let (id, gm, gds) = model.id(0.0, vds);
            assert!((id - GMIN * vds).abs() <= 1e-25, "{id} at vds {vds}");
            assert_eq!(gm, 0.0, "{vds}");
            assert!((gds - GMIN).abs() <= 1e-25, "{gds} at vds {vds}");
        }
    }

    #[test]
    fn an_nmos_iterate_whose_id_misses_its_linearisation_is_not_converged() {
        let circuit = Circuit::new(open_mos(0.0)).unwrap();
        // Saturation. The solved gate is 0.1 V above the prior, so the square law
        // misses the linearised drain current by more than the tolerance.
        let (_, report) = mos_update_at(&circuit, [5.0, 3.1, 0.0], [3.0, 5.0]);
        assert!(!report.limited);
        assert!(report.mismatch > 1.0, "{}", report.mismatch);
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

    const KP: f64 = 0.0468;
    const VTO: f64 = 1.08;
    const CGS: f64 = 55e-12;
    const CGD: f64 = 5e-12;
    const CDS: f64 = 20e-12;

    fn mos_part(pins: [&str; 3], lambda: f64) -> String {
        let [d, g, s] = pins;
        format!(
            r#"{{ "name": "m1", "kind": "nmos",
                "pins": {{ "drain": "{d}", "gate": "{g}", "source": "{s}" }},
                "transconductance_parameter": {kp}, "threshold_voltage": {vto},
                "channel_length_modulation": {lambda},
                "gate_source_capacitance": {cgs}, "gate_drain_capacitance": {cgd},
                "drain_source_capacitance": {cds},
                "ratings": {{ "current": null, "power": null }} }}"#,
            kp = value(KP, "A/V^2"),
            vto = value(VTO, "V"),
            lambda = value(lambda, "1/V"),
            cgs = value(CGS, "F"),
            cgd = value(CGD, "F"),
            cds = value(CDS, "F"),
        )
    }

    fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> String {
        format!(
            r#"{{ "name": "{name}", "kind": "resistor", "pins": {{ "a": "{a}", "b": "{b}" }},
                "resistance": {}, "ratings": {{ "power": null }} }}"#,
            value(ohm, "ohm")
        )
    }

    fn capacitor(name: &str, a: &str, b: &str, farad: f64) -> String {
        format!(
            r#"{{ "name": "{name}", "kind": "capacitor", "pins": {{ "a": "{a}", "b": "{b}" }},
                "capacitance": {}, "initial": null }}"#,
            value(farad, "F")
        )
    }

    fn source(name: &str, net: &str, volts: f64) -> String {
        format!(
            r#"{{ "name": "{name}", "kind": "voltage_source",
                "pins": {{ "p": "{net}", "n": "gnd" }}, "voltage": {} }}"#,
            value(volts, "V")
        )
    }

    /// `d`, `g` and `s` each have a DC path, so the gate is not floating.
    fn open_mos(lambda: f64) -> Netlist {
        let parts = [
            mos_part(["d", "g", "s"], lambda),
            resistor("rd", "d", "gnd", 1e6),
            resistor("rg", "g", "gnd", 1e6),
            resistor("rs", "s", "gnd", 1e6),
        ];
        assemble(r#"{ "op": {} }"#, &["d", "g", "s"], &parts)
    }

    fn m1(netlist: &Netlist) -> (PartId, &Mosfet) {
        let id = netlist.part_id(&Name::new("m1").unwrap()).unwrap();
        let (_, part) = netlist.parts().nth(id.index()).unwrap();
        match &part.model {
            Model::Nmos(mosfet) => (id, mosfet),
            other => panic!("{other:?}"),
        }
    }

    fn mos_update_at(circuit: &Circuit, v: [f64; 3], prior: [f64; 2]) -> ([f64; 2], Update) {
        let (id, mosfet) = m1(&circuit.netlist);
        let mut x = Solution::zeros(circuit.layout.size());
        for (pin, volts) in mosfet.pins.into_iter().zip(v) {
            if let Node::Row(row) = Node::from(pin) {
                x.add(row, volts);
            }
        }
        let probe = Probe {
            layout: &circuit.layout,
            part: id,
            x: &x,
        };
        let mut next = [f64::NAN; 2];
        let report = mos_update(mosfet, &probe, &prior, &mut next);
        (next, report)
    }

    fn within(got: f64, want: f64, slack: f64, what: &str) {
        assert!(
            (got - want).abs() <= slack,
            "{what}: {got} vs {want}, off by {}",
            (got - want).abs()
        );
    }

    /// `id`, `gm` and `gds` meet across cutoff, across saturation, and across `vds = 0`.
    /// Reverse mode is the terminal swap, so it equals `-f(vgs - vds, -vds)` plus `GMIN`.
    #[test]
    fn regions_meet_at_their_boundaries() {
        let lambda = 0.05;
        let model = Level1::new(m1(&open_mos(lambda)).1);
        let meet = |left: (f64, f64, f64), right: (f64, f64, f64), what: &str| {
            for (name, a, b) in [
                ("id", left.0, right.0),
                ("gm", left.1, right.1),
                ("gds", left.2, right.2),
            ] {
                within(a, b, 1e-6, &format!("{what} {name}"));
            }
        };
        for vds in [0.0, 0.4, 2.0, 8.0] {
            meet(
                model.id(VTO - 1e-6, vds),
                model.id(VTO + 1e-6, vds),
                "vgs = VTO",
            );
            let (id, gm, gds) = model.id(VTO, vds);
            within(id, GMIN * vds, 1e-18, "cutoff id");
            assert_eq!(gm, 0.0);
            within(gds, GMIN, 1e-18, "cutoff gds");
        }
        let vgs = 3.0;
        let vov = vgs - VTO;
        meet(
            model.id(vgs, vov - 1e-6),
            model.id(vgs, vov + 1e-6),
            "vds = vgs - VTO",
        );
        meet(model.id(vgs, -1e-6), model.id(vgs, 1e-6), "vds = 0");
        let (id, gm, gds) = model.id(vgs, 0.0);
        assert_eq!(id, 0.0);
        assert_eq!(gm, 0.0);
        within(gds, KP * vov + GMIN, 1e-12, "gds at vds = 0");

        for (vgs, vds) in [(3.0, 1.5), (2.0, 0.2), (0.5, 4.0), (4.0, 0.05)] {
            let neg = -vds;
            let (got_id, got_gm, got_gds) = model.id(vgs, neg);
            let (f, fg, fd) = model.forward(vgs - neg, -neg);
            assert_eq!(
                (got_id, got_gm, got_gds),
                (-f + GMIN * neg, -fg, fg + fd + GMIN),
                "reverse at vgs {vgs}, vds {neg}"
            );
        }

        let h = 1e-6;
        for (vgs, vds) in [
            (0.0, 4.0),
            (3.0, 0.4),
            (3.0, 6.0),
            (1.0, -2.0),
            (2.2, -0.3),
            (0.0, -0.2),
        ] {
            let (id, gm, gds) = model.id(vgs, vds);
            let gm_d = (model.id(vgs + h, vds).0 - model.id(vgs - h, vds).0) / (2.0 * h);
            let gds_d = (model.id(vgs, vds + h).0 - model.id(vgs, vds - h).0) / (2.0 * h);
            let rounding = 4.0 * f64::EPSILON * id.abs() / h;
            within(gm, gm_d, 1e-6 * gm_d.abs() + rounding + 1e-9, "gm");
            within(gds, gds_d, 1e-6 * gds_d.abs() + rounding + 1e-9, "gds");
        }
    }

    fn solved_op(netlist: &Netlist) -> std::collections::BTreeMap<String, f64> {
        let results = crate::simulate(netlist);
        assert!(
            results.diagnostics().is_empty(),
            "{:?}",
            results.diagnostics()
        );
        results
            .observations()
            .op()
            .unwrap()
            .values
            .iter()
            .map(|(reference, value)| (reference.to_string(), value.get()))
            .collect()
    }

    /// A voltage source on the drain sinks `id` plus the node `GMIN`.
    #[test]
    fn drain_current_matches_the_square_law() {
        let model = Level1::new(m1(&open_mos(0.0)).1);
        for (vd, vg, name) in [
            (5.0, 0.0, "cutoff"),
            (1.0, 5.0, "linear"),
            (5.0, 5.0, "saturation"),
            (-2.0, 0.0, "reverse"),
        ] {
            let want = model.id(vg, vd).0;
            let parts = [
                mos_part(["d", "g", "gnd"], 0.0),
                source("vd", "d", vd),
                source("vg", "g", vg),
            ];
            let op = solved_op(&assemble(r#"{ "op": {} }"#, &["d", "g"], &parts));
            let (id, p) = (op["#m1.id"], op["#m1.p"]);
            within(id, want, 1e-9 * want.abs() + 1e-15, name);
            within(p, vd * id, 1e-15, name);
            within(op["#vd.i"] + id + GMIN * vd, 0.0, 1e-12, name);
            within(op["#vg.i"] + GMIN * vg, 0.0, 1e-15, name);
        }
    }

    /// Drain at 12 V, gate at 5 V, 1 kohm from the source to ground.
    /// Those two readings are ngspice-42's operating point for this fit.
    #[test]
    fn a_source_follower_holds_the_source_off_ground() {
        let parts = [
            mos_part(["d", "g", "s"], 0.0),
            source("vd", "d", 12.0),
            source("vg", "g", 5.0),
            resistor("rs", "s", "gnd", 1e3),
        ];
        let op = solved_op(&assemble(r#"{ "op": {} }"#, &["d", "g", "s"], &parts));
        within(op["#net.s"], 3.531516, 1e-5, "source");
        within(op["#m1.id"], 3.531516e-3, 1e-8, "id");
    }

    fn floating_nets(netlist: &Netlist) -> Vec<String> {
        crate::simulate(netlist)
            .diagnostics()
            .iter()
            .filter(|d| d.code() == "net.floating")
            .map(|d| d.refs()[0].to_string())
            .collect()
    }

    /// A gate on a capacitor and on nothing else has no DC path. A resistor to ground closes it.
    #[test]
    fn a_gate_reached_only_through_a_capacitor_is_floating() {
        let open = [
            mos_part(["d", "g", "gnd"], 0.0),
            source("vdd", "vdd", 5.0),
            resistor("rd", "vdd", "d", 1e3),
            capacitor("cg", "g", "gnd", CGS),
        ];
        assert_eq!(
            floating_nets(&assemble(r#"{ "op": {} }"#, &["d", "g", "vdd"], &open)),
            ["#net.g"]
        );
        let held = [
            mos_part(["d", "g", "gnd"], 0.0),
            source("vdd", "vdd", 5.0),
            resistor("rd", "vdd", "d", 1e3),
            capacitor("cg", "g", "gnd", CGS),
            resistor("rg", "g", "gnd", 1e6),
        ];
        assert_eq!(
            floating_nets(&assemble(r#"{ "op": {} }"#, &["d", "g", "vdd"], &held)),
            Vec::<String>::new()
        );
    }

    /// Drain tied to the gate leaves `CGS + CDS`. Source tied to the gate leaves `CGD + CDS`.
    /// A swap of `CGS` and `CGD` moves each time constant to the other.
    fn gate_step(drain_tied: bool) -> Netlist {
        let pins = if drain_tied {
            ["g", "g", "gnd"]
        } else {
            ["gnd", "g", "g"]
        };
        let parts = [
            mos_part(pins, 0.0),
            format!(
                r#"{{ "name": "vin", "kind": "voltage_source", "pins": {{ "p": "in", "n": "gnd" }},
                    "voltage": {{ "kind": "pulse", "initial": {}, "pulsed": {},
                        "delay_ps": 1000000, "width_ps": 100000000, "period_ps": 200000000 }} }}"#,
                value(0.0, "V"),
                value(-1.0, "V")
            ),
            resistor("rs", "in", "g", 100e3),
        ];
        let analysis = r#"{ "transient": { "stop_ps": 40000000, "max_step_ps": 1000000, "grid_ps": 100000 } }"#;
        assemble(analysis, &["g", "in"], &parts)
    }

    #[test]
    fn a_gate_step_charges_cgs_and_cgd_each_with_its_own_time_constant() {
        let ohm = 100e3;
        for (drain_tied, tau) in [(true, ohm * (CGS + CDS)), (false, ohm * (CGD + CDS))] {
            let results = crate::simulate(&gate_step(drain_tied));
            assert!(
                results.diagnostics().is_empty(),
                "{:?}",
                results.diagnostics()
            );
            let trace = results.trace().unwrap();
            let g = "#net.g".parse::<Ref>().unwrap();
            let column = trace.columns().iter().position(|r| *r == g).unwrap();
            let mut worst = 0.0f64;
            for (at, v) in trace.time().iter().zip(trace.column(column)) {
                let since = (at.as_ps() as f64 - 1e6).max(0.0) * 1e-12;
                let want = -(1.0 - libm::exp(-since / tau));
                worst = worst.max((v - want).abs());
            }
            assert!(
                worst <= 1e-3,
                "drain tied {drain_tied}: {worst} V off, tau {tau}"
            );
        }
    }

    /// `p` keeps the gate current. The trace has no `ig` column, so the resistor supplies it.
    #[test]
    fn pin_power_includes_the_gate_current() {
        let ohm = 1e6;
        let parts = [
            mos_part(["d", "g", "gnd"], 0.0),
            format!(
                r#"{{ "name": "vin", "kind": "voltage_source", "pins": {{ "p": "in", "n": "gnd" }},
                    "voltage": {{ "kind": "pulse", "initial": {}, "pulsed": {},
                        "delay_ps": 1000000, "width_ps": 500000000, "period_ps": 1000000000 }} }}"#,
                value(0.0, "V"),
                value(5.0, "V")
            ),
            resistor("rg", "in", "g", ohm),
            source("vdd", "vdd", 5.0),
            resistor("rd", "vdd", "d", 1e3),
        ];
        let analysis = r#"{ "transient": { "stop_ps": 200000000, "max_step_ps": 1000000, "grid_ps": 1000000 } }"#;
        let results = crate::simulate(&assemble(analysis, &["d", "g", "in", "vdd"], &parts));
        assert!(
            results.diagnostics().is_empty(),
            "{:?}",
            results.diagnostics()
        );
        let trace = results.trace().unwrap();
        let column = |key: &str| {
            let reference = key.parse::<Ref>().unwrap();
            let index = trace
                .columns()
                .iter()
                .position(|r| *r == reference)
                .unwrap();
            trace.column(index)
        };
        let (id, p) = (column("#m1.id"), column("#m1.p"));
        let (vd, vg, vin) = (column("#net.d"), column("#net.g"), column("#net.in"));
        let mut gate_power = 0.0f64;
        for row in 0..id.len() {
            let ig = (vin[row] - vg[row]) / ohm - GMIN * vg[row];
            gate_power = gate_power.max((vg[row] * ig).abs());
            within(p[row], vd[row] * id[row] + vg[row] * ig, 1e-15, "p");
        }
        assert!(gate_power > 1e-9, "{gate_power}");
    }
}

/// A bipolar transistor. `npn` and `pnp` share it, and the kind is the polarity.
///
/// SPICE's Gummel-Poon model at its defaults, which is the transport form of Ebers-Moll with a
/// forward Early voltage, at 298.15 K with no temperature scaling. `RB`, `RC` and `RE` join
/// each pin to its internal node `b'`, `c'` or `e'`. With `vbe = s * (V(b') - V(e'))`,
/// `vbc = s * (V(b') - V(c'))`, `s` +1 for `npn` and -1 for `pnp`, and `VT` the junction's:
///
/// ```text
/// ibe = IS * (exp(vbe / (NF*VT)) - 1)
/// ibc = IS * (exp(vbc / (NR*VT)) - 1)
/// ic  = (ibe - ibc) * (1 - vbc/VAF) - ibc/BR - GMIN*vbc
/// ib  = ibe/BF + ibc/BR + GMIN*(vbe + vbc)
/// ```
///
/// Below `vbe = -3 * NF*VT`, `ibe` takes a `Junction`'s reverse law.
/// Below `vbc = -3 * NR*VT`, `ibc` does.
///
/// `s * ic` enters `c'` and `s * ib` enters `b'`. `CJE` is a constant capacitance from `b'` to
/// `e'`, and `CJC` from `b'` to `c'`. Every value is positive, as in a SPICE model card.
#[derive(Clone, Debug, PartialEq)]
pub struct Bjt {
    /// Nets of pins `collector`, `base`, `emitter`.
    pub pins: [NetId; 3],
    /// Ampere. `IS`.
    pub saturation_current: Value,
    /// Unit `1`. `BF`.
    pub forward_beta: Value,
    /// Unit `1`. `BR`.
    pub reverse_beta: Value,
    /// Unit `1`. `NF`.
    pub forward_emission: Value,
    /// Unit `1`. `NR`.
    pub reverse_emission: Value,
    /// Volt. `VAF`.
    pub early_voltage: Value,
    /// Ohm. `RB`.
    pub base_resistance: Value,
    /// Ohm. `RC`.
    pub collector_resistance: Value,
    /// Ohm. `RE`.
    pub emitter_resistance: Value,
    /// Farad. `CJE`.
    pub base_emitter_capacitance: Value,
    /// Farad. `CJC`.
    pub base_collector_capacitance: Value,
    /// Ampere, greater than 0. Bounds `|ic|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl Bjt {
    pub const PINS: [&'static str; 3] = ["collector", "base", "emitter"];
}

/// A [`Bjt`] as an `npn`.
#[derive(Clone, Debug, PartialEq)]
pub struct Npn(pub Bjt);

/// A [`Bjt`] as a `pnp`. Every junction voltage and pin current is an `npn`'s negated.
#[derive(Clone, Debug, PartialEq)]
pub struct Pnp(pub Bjt);

/// An N-channel MOSFET. Level 1 with the bulk tied to the source, so `W/L` is folded into `KP`
/// and there is no body effect. No temperature scaling.
///
/// With `vgs = V(gate) - V(source)` and `vds = V(drain) - V(source)`, and `vds >= 0`:
///
/// ```text
/// cutoff, vgs <= VTO:                 id = 0
/// saturation, vds >= vgs - VTO:       id = KP/2 * (vgs - VTO)^2 * (1 + LAMBDA*vds)
/// linear:                             id = KP * vds * (vgs - VTO - vds/2) * (1 + LAMBDA*vds)
/// ```
///
/// `vds < 0` swaps the terminals, `id = -f(vgs - vds, -vds)`. `id` then gains `GMIN * vds`,
/// which is ngspice-42's drain-bulk `gmin` once the bulk sits on the source and `is` is 0.
/// The gate draws no DC current. `CGS`, `CGD` and `CDS` are constant capacitances between the pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Mosfet {
    /// Nets of pins `drain`, `gate`, `source`.
    pub pins: [NetId; 3],
    /// Ampere per volt squared. `KP`. Greater than 0.
    pub transconductance_parameter: Value,
    /// Volt. `VTO`. Any finite value, so a depletion device can state a negative threshold.
    pub threshold_voltage: Value,
    /// Per volt. `LAMBDA`. Zero is a flat saturation current.
    pub channel_length_modulation: Value,
    /// Farad. From gate to source.
    pub gate_source_capacitance: Value,
    /// Farad. From gate to drain.
    pub gate_drain_capacitance: Value,
    /// Farad. From drain to source.
    pub drain_source_capacitance: Value,
    /// Ampere, greater than 0. Bounds `|id|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

impl Mosfet {
    pub const PINS: [&'static str; 3] = ["drain", "gate", "source"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireBjt {
    pub(crate) name: String,
    pins: PinMap,
    saturation_current: WireValue,
    forward_beta: WireValue,
    reverse_beta: WireValue,
    forward_emission: WireValue,
    reverse_emission: WireValue,
    early_voltage: WireValue,
    base_resistance: WireValue,
    collector_resistance: WireValue,
    emitter_resistance: WireValue,
    base_emitter_capacitance: WireValue,
    base_collector_capacitance: WireValue,
    ratings: WireJunctionRatings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireMosfet {
    pub(crate) name: String,
    pins: PinMap,
    transconductance_parameter: WireValue,
    threshold_voltage: WireValue,
    channel_length_modulation: WireValue,
    gate_source_capacitance: WireValue,
    gate_drain_capacitance: WireValue,
    drain_source_capacitance: WireValue,
    ratings: WireJunctionRatings,
}

fn resolve_bjt(ctx: ParseCtx<'_>, wire: WireBjt) -> Option<Bjt> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Bjt::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let is = positive("saturation_current", "A", wire.saturation_current);
    let bf = positive("forward_beta", "1", wire.forward_beta);
    let br = positive("reverse_beta", "1", wire.reverse_beta);
    let nf = positive("forward_emission", "1", wire.forward_emission);
    let nr = positive("reverse_emission", "1", wire.reverse_emission);
    let vaf = positive("early_voltage", "V", wire.early_voltage);
    let rb = positive("base_resistance", "ohm", wire.base_resistance);
    let rc = positive("collector_resistance", "ohm", wire.collector_resistance);
    let re = positive("emitter_resistance", "ohm", wire.emitter_resistance);
    let cje = positive(
        "base_emitter_capacitance",
        "F",
        wire.base_emitter_capacitance,
    );
    let cjc = positive(
        "base_collector_capacitance",
        "F",
        wire.base_collector_capacitance,
    );
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    let mut faults = Vec::new();
    if let Some(is) = &is {
        for (field, n) in [("forward_emission", &nf), ("reverse_emission", &nr)] {
            if let Some(n) = n
                && let Err(fault) = junction_scale(n.si.get(), is.si.get())
            {
                let at = (fault.field(field), fault.value(n.si.get(), is.si.get()));
                // Both junctions share `saturation_current`, which breaks the rule once.
                if !faults.contains(&at) {
                    faults.push(at);
                }
            }
        }
    }
    let junctions = faults.is_empty();
    for (field, value) in faults {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: field.into(),
            value,
            rule: ValueRule::FiniteJunction,
        });
    }
    if !junctions {
        return None;
    }
    Some(Bjt {
        pins: pins?,
        saturation_current: is?,
        forward_beta: bf?,
        reverse_beta: br?,
        forward_emission: nf?,
        reverse_emission: nr?,
        early_voltage: vaf?,
        base_resistance: rb?,
        collector_resistance: rc?,
        emitter_resistance: re?,
        base_emitter_capacitance: cje?,
        base_collector_capacitance: cjc?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

pub(crate) fn resolve_npn(ctx: ParseCtx<'_>, wire: WireBjt) -> Option<Npn> {
    resolve_bjt(ctx, wire).map(Npn)
}

pub(crate) fn resolve_pnp(ctx: ParseCtx<'_>, wire: WireBjt) -> Option<Pnp> {
    resolve_bjt(ctx, wire).map(Pnp)
}

pub(crate) fn resolve_mosfet(ctx: ParseCtx<'_>, wire: WireMosfet) -> Option<Mosfet> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Mosfet::PINS, &wire.pins.0, nets, out);
    let lambda_number = wire.channel_length_modulation.value.get();
    let (kp, cgs, cgd, cds) = {
        let mut positive = |field: &'static str, unit, value| {
            resolve_value(part_ref, field, unit, true, value, out)
        };
        (
            positive(
                "transconductance_parameter",
                "A/V^2",
                wire.transconductance_parameter,
            ),
            positive("gate_source_capacitance", "F", wire.gate_source_capacitance),
            positive("gate_drain_capacitance", "F", wire.gate_drain_capacitance),
            positive(
                "drain_source_capacitance",
                "F",
                wire.drain_source_capacitance,
            ),
        )
    };
    let vto = resolve_value(
        part_ref,
        "threshold_voltage",
        "V",
        false,
        wire.threshold_voltage,
        out,
    );
    let lambda = resolve_value(
        part_ref,
        "channel_length_modulation",
        "1/V",
        false,
        wire.channel_length_modulation,
        out,
    )
    .and_then(|value| {
        if lambda_number < 0.0 {
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "channel_length_modulation".into(),
                value: lambda_number,
                rule: ValueRule::NonNegative,
            });
            None
        } else {
            Some(value)
        }
    });
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    Some(Mosfet {
        pins: pins?,
        transconductance_parameter: kp?,
        threshold_voltage: vto?,
        channel_length_modulation: lambda?,
        gate_source_capacitance: cgs?,
        gate_drain_capacitance: cgd?,
        drain_source_capacitance: cds?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

macro_rules! bjt_kind {
    ($kind:ident, $polarity:expr) => {
        impl Kind for $kind {
            fn pins(&self) -> &[NetId] {
                &self.0.pins
            }
            fn unknowns(&self) -> Unknowns {
                Unknowns {
                    internal_nodes: 3,
                    bare_internal: true,
                    branches: 2,
                    junctions: 2,
                }
            }
            fn charges(&self) -> Vec<OwnedCapacitor> {
                charges(&self.0)
            }
            fn inductors(&self) -> Vec<OwnedInductor> {
                Vec::new()
            }
            #[inline]
            fn stamp(&self, stamp: &mut Stamp<'_>) {
                self::stamp(&self.0, $polarity, stamp);
            }
            fn reads_iterate(&self) -> bool {
                true
            }
            fn initial(&self, out: &mut [f64]) {
                initial(&self.0, out);
            }
            fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
                update(&self.0, $polarity, probe, prior, next)
            }
            /// The base to each other pin, because each junction conducts.
            fn dc_paths(&self) -> &'static [(usize, usize)] {
                &[(0, 1), (1, 2)]
            }
            fn quantities(&self) -> &'static [Quantity] {
                &[Quantity::Ib, Quantity::Ic, Quantity::P]
            }
            fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
                let current = self.0.max_current.as_ref().map(|v| (Quantity::Ic, v));
                (current, self.0.max_power.as_ref())
            }
            fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
                Some(Observed::values(observe(&self.0, probe), 3))
            }
            fn conduction(&self) -> Conduction {
                Conduction::Switches(0, 2)
            }
        }
    };
}

bjt_kind!(Npn, Polarity::Npn);
bjt_kind!(Pnp, Polarity::Pnp);

impl Kind for Mosfet {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 0,
            bare_internal: false,
            branches: 3,
            junctions: 2,
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        mos_capacitors(self).to_vec()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    /// The channel linearised at the slot voltages, then the three pin capacitors.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let model = Level1::new(self);
        let (vgs, vds) = (stamp.junction(VGS), stamp.junction(VDS));
        let (id, gm, gds) = model.id(vgs, vds);
        let [d, g, s] = self.pins.map(Node::from);
        stamp.add(d, g, gm);
        stamp.add(d, s, -gm - gds);
        stamp.add(d, d, gds);
        stamp.add(s, g, -gm);
        stamp.add(s, s, gm + gds);
        stamp.add(s, d, -gds);
        stamp.current(d, s, id - gm * vgs - gds * vds);
        for cap in mos_capacitors(self) {
            stamp_capacitor(stamp, cap.k, cap.ends, cap.farad, None);
        }
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    /// Both slots start at 0. The square law needs no `vcrit`.
    fn initial(&self, out: &mut [f64]) {
        out[VGS] = 0.0;
        out[VDS] = 0.0;
    }
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
        mos_update(self, probe, prior, next)
    }
    /// The drain to the source. The gate joins nothing.
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 2)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        &[Quantity::Id, Quantity::P]
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        let current = self.max_current.as_ref().map(|v| (Quantity::Id, v));
        (current, self.max_power.as_ref())
    }
    /// `id` and `p`. `id` is the channel current plus the capacitor currents into the drain pin.
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        let [vd, vg, vs] = self.pins.map(|pin| probe.v(pin));
        let id_channel = Level1::new(self).id(vg - vs, vd - vs).0;
        let i_cgs = probe.branch(0);
        let i_cgd = probe.branch(1);
        let i_cds = probe.branch(2);
        let id = id_channel + i_cds - i_cgd;
        let ig = i_cgs + i_cgd;
        Some(Observed::values(
            [id, (vd - vs) * id + (vg - vs) * ig, 0.0],
            2,
        ))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Switches(0, 2)
    }
}
