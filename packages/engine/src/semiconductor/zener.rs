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
use crate::semiconductor::Junction;
use crate::semiconductor::junction_scale;
use serde::Deserialize;
use std::f64::consts::SQRT_2;

use orbe_model::Time;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{GMIN, Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::ParseCtx;
use crate::netlist::{Conduction, Kind};
use crate::newton::Side;
use crate::semiconductor::junction::{Shockley, mismatch, pnjlim};

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

/// Limits each solved junction voltage against its own prior and its own `vcrit`.
pub(crate) fn update(zener: &Zener, probe: &Probe<'_>, prior: &[f64]) -> ([f64; 2], Update) {
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

/// The current through Rs, entering the anode.
fn current(zener: &Zener, probe: &Probe<'_>) -> f64 {
    (probe.v(zener.pins[0]) - probe.internal(0)) / zener.series_resistance.si.get()
}

fn junction_mismatch(params: &Shockley, prior: f64, solved: f64) -> (f64, bool) {
    let (id, gd, cap_prior, voltage) = params.evaluate(prior);
    let linear = id + gd * (solved - voltage);
    let (actual, _, cap_solved, _) = params.evaluate(solved);
    (mismatch(linear, actual), cap_prior || cap_solved)
}

#[cfg(test)]
mod tests {
    use crate::netlist::{self, Model};

    use crate::mna::{GMIN, Node, Probe, Solution, Stamp, System};
    use crate::newton::{Circuit, Conditions, Iterate};
    use crate::semiconductor::junction::pnjlim;

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

    fn zener_id(circuit: &Circuit) -> crate::netlist::PartId {
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
            crate::element::Device::stamp(&part.model, &mut stamp);
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
            .join("tests/circuit/fixtures/zener-overload/netlist.json");
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

/// A forward junction and a breakdown junction, each with its own series resistance.
///
/// `series_resistance` joins the anode to internal node 0. The forward junction runs from
/// there to the cathode, with the diode's law. `breakdown_resistance` joins internal node 1
/// to the cathode, and the breakdown junction runs from internal node 1 to internal node 0.
/// Breakdown current does not flow in the forward junction.
///
/// `breakdown` is derived at parse time:
/// `nbvt = (zener_impedance - series_resistance - breakdown_resistance) * zener_current`,
/// `bvj = zener_voltage - zener_current * (series_resistance + breakdown_resistance)`,
/// `Isb = zener_current / expm1(bvj / nbvt)`.
/// At `zener_current` the pin voltage is `zener_voltage`. The slope there is
/// `series_resistance + breakdown_resistance + nbvt / (zener_current + Isb)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Zener {
    /// Nets of pins `anode`, `cathode`.
    pub pins: [NetId; 2],
    /// Ampere. The forward junction's `Is`.
    pub saturation_current: Value,
    /// Unit `1`. The forward junction's `N`.
    pub emission_coefficient: Value,
    /// Ohm. From the anode to internal node 0.
    pub series_resistance: Value,
    /// Volt. `VZ`, the pin voltage at `zener_current` in breakdown.
    pub zener_voltage: Value,
    /// Ampere. `IZT`.
    pub zener_current: Value,
    /// Ohm. `ZZT`, the slope of the pin voltage at `zener_current`.
    pub zener_impedance: Value,
    /// Ohm. In series with the breakdown junction only.
    pub breakdown_resistance: Value,
    /// The breakdown junction derived from the row and the two resistances.
    pub breakdown: Breakdown,
    /// Ampere, greater than 0. Bounds `|i|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Value>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Value>,
}

/// `Isb`, `nbvt` and `vcrit` of a zener's breakdown junction.
#[derive(Clone, Debug, PartialEq)]
pub struct Breakdown {
    /// Ampere.
    pub saturation_current: f64,
    /// Volt. `NBV * VT`.
    pub nvt: f64,
    /// Volt. The breakdown junction's start voltage.
    pub vcrit: f64,
}

/// Which derived breakdown value breaks `finite_junction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakdownFault {
    /// `nbvt` is not normal.
    Impedance,
    /// `Isb` is not normal, or `vcrit` is not finite and positive.
    Voltage,
}

impl BreakdownFault {
    pub fn field(self) -> &'static str {
        match self {
            BreakdownFault::Impedance => "zener_impedance",
            BreakdownFault::Voltage => "zener_voltage",
        }
    }
}

/// The breakdown junction of a row whose impedance and voltage already clear the two resistances.
///
/// `nbvt` must be normal. `Isb` must be normal and positive, and its `vcrit` must be finite and
/// positive. A failure names [`BreakdownFault::Impedance`] or [`BreakdownFault::Voltage`].
pub fn breakdown_scale(
    series_resistance: f64,
    breakdown_resistance: f64,
    zener_voltage: f64,
    zener_current: f64,
    zener_impedance: f64,
) -> Result<Breakdown, BreakdownFault> {
    let nbvt = (zener_impedance - series_resistance - breakdown_resistance) * zener_current;
    if !nbvt.is_normal() {
        return Err(BreakdownFault::Impedance);
    }
    let bvj = zener_voltage - zener_current * (series_resistance + breakdown_resistance);
    let isb = zener_current / libm::expm1(bvj / nbvt);
    if !isb.is_normal() || isb <= 0.0 {
        return Err(BreakdownFault::Voltage);
    }
    let vcrit = nbvt * libm::log(nbvt / (SQRT_2 * isb));
    if !(vcrit.is_finite() && vcrit > 0.0) {
        return Err(BreakdownFault::Voltage);
    }
    Ok(Breakdown {
        saturation_current: isb,
        nvt: nbvt,
        vcrit,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireZener {
    pub(crate) name: String,
    pins: PinMap,
    saturation_current: WireValue,
    emission_coefficient: WireValue,
    series_resistance: WireValue,
    zener_voltage: WireValue,
    zener_current: WireValue,
    zener_impedance: WireValue,
    breakdown_resistance: WireValue,
    ratings: WireJunctionRatings,
}

pub(crate) fn resolve_zener(ctx: ParseCtx<'_>, wire: WireZener) -> Option<Zener> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Junction::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let is = positive("saturation_current", "A", wire.saturation_current);
    let n = positive("emission_coefficient", "1", wire.emission_coefficient);
    let rs = positive("series_resistance", "ohm", wire.series_resistance);
    let vz = positive("zener_voltage", "V", wire.zener_voltage);
    let izt = positive("zener_current", "A", wire.zener_current);
    let zzt = positive("zener_impedance", "ohm", wire.zener_impedance);
    let rb = positive("breakdown_resistance", "ohm", wire.breakdown_resistance);
    let max_current = resolve_rating(part_ref, "ratings.current", "A", wire.ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", wire.ratings.power, out);
    let mut rejected = false;
    let mut breakdown = None;
    if let (Some(is), Some(n), Some(rs), Some(vz), Some(izt), Some(zzt), Some(rb)) =
        (&is, &n, &rs, &vz, &izt, &zzt, &rb)
    {
        if let Err(fault) = junction_scale(n.si.get(), is.si.get()) {
            rejected = true;
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: fault.field("emission_coefficient").into(),
                value: fault.value(n.si.get(), is.si.get()),
                rule: ValueRule::FiniteJunction,
            });
        }
        let series = rs.si.get() + rb.si.get();
        let resistance_ok = zzt.si.get() > series;
        let drop_ok = vz.si.get() > izt.si.get() * series;
        if !resistance_ok {
            rejected = true;
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "zener_impedance".into(),
                value: zzt.si.get(),
                rule: ValueRule::AboveResistance,
            });
        }
        if !drop_ok {
            rejected = true;
            out.push(Diagnostic::NetlistValue {
                part: part_ref.clone(),
                field: "zener_voltage".into(),
                value: vz.si.get(),
                rule: ValueRule::AboveDrop,
            });
        }
        if resistance_ok && drop_ok {
            match breakdown_scale(
                rs.si.get(),
                rb.si.get(),
                vz.si.get(),
                izt.si.get(),
                zzt.si.get(),
            ) {
                Ok(found) => breakdown = Some(found),
                Err(fault) => {
                    rejected = true;
                    let value = match fault {
                        BreakdownFault::Impedance => zzt.si.get(),
                        BreakdownFault::Voltage => vz.si.get(),
                    };
                    out.push(Diagnostic::NetlistValue {
                        part: part_ref.clone(),
                        field: fault.field().into(),
                        value,
                        rule: ValueRule::FiniteJunction,
                    });
                }
            }
        }
    }
    if rejected {
        return None;
    }
    Some(Zener {
        pins: pins?,
        saturation_current: is?,
        emission_coefficient: n?,
        series_resistance: rs?,
        zener_voltage: vz?,
        zener_current: izt?,
        zener_impedance: zzt?,
        breakdown_resistance: rb?,
        breakdown: breakdown?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

impl Kind for Zener {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 2,
            bare_internal: true,
            branches: 0,
            junctions: 2,
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    /// Rs from the anode to internal node 0, then the forward junction to the cathode.
    /// Rb from internal node 1 to the cathode, then the breakdown junction back to node 0.
    /// Each junction has its own `GMIN`. Neither internal node has node gmin.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let jf = forward(self);
        let jb = breakdown(self);
        let [anode, cathode] = self.pins;
        let n0 = stamp.internal(0);
        let n1 = stamp.internal(1);
        stamp.conductance(anode, n0, 1.0 / self.series_resistance.si.get());
        let forward_voltage = stamp.junction(0);
        let (id, gd, _, voltage) = jf.evaluate(forward_voltage);
        stamp.conductance(n0, cathode, gd + GMIN);
        stamp.current(n0, cathode, id - gd * voltage);
        stamp.conductance(n1, cathode, 1.0 / self.breakdown_resistance.si.get());
        let breakdown_voltage = stamp.junction(1);
        let (ib, gb, _, voltage) = jb.evaluate(breakdown_voltage);
        stamp.conductance(n1, n0, gb + GMIN);
        stamp.current(n1, n0, ib - gb * voltage);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, out: &mut [f64]) {
        out[..2].copy_from_slice(&[forward(self).vcrit, breakdown(self).vcrit]);
    }
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
        let (limited, update) = update(self, probe, prior);
        next[..2].copy_from_slice(&limited);
        update
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        let current = self.max_current.as_ref().map(|v| (Quantity::I, v));
        (current, self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(Observed::two_pin(self.pins, current(self, probe), probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Passes
    }
}
