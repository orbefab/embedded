use crate::diagnostic::Diagnostic;
use crate::diagnostic::ValueRule;
use crate::netlist::NetId;
use crate::netlist::Rating;
use crate::netlist::Value;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireJunctionRatings;
use crate::netlist::json::WireValue;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_rating;
use crate::netlist::json::resolve_value;
use serde::Deserialize;
use std::f64::consts::SQRT_2;

use orbe_model::Time;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{GMIN, Node, Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::ParseCtx;
use crate::netlist::{Conduction, Kind};
use crate::newton::{ABSTOL, RELTOL, Side};

/// `exp` overflows near 709.78. Stay under that.
const ARG_MAX: f64 = 700.0;
/// A conductance no operating point reaches. The cap keeps `Is*exp(arg)/nvt` under this.
const G_CAP: f64 = 1e100;

/// One junction's current law. A diode has one, and a transistor one per junction.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shockley {
    is: f64,
    pub(crate) nvt: f64,
    pub(crate) vcrit: f64,
}

impl Shockley {
    pub(crate) fn new(emission: f64, is: f64) -> Shockley {
        let scale = junction_scale(emission, is).expect("parse accepts only a finite junction");
        Shockley {
            is,
            nvt: scale.nvt,
            vcrit: scale.vcrit,
        }
    }

    /// A junction whose saturation current, `n*VT` and `vcrit` are already known.
    pub(crate) fn from_parts(is: f64, nvt: f64, vcrit: f64) -> Shockley {
        Shockley { is, nvt, vcrit }
    }

    /// Largest `exp` argument whose current and slope stay finite and under `G_CAP`.
    fn arg_cap(&self) -> f64 {
        // In log space, so a tiny `Is` cannot overflow the quotient.
        ARG_MAX.min(libm::log(G_CAP) + libm::log(self.nvt) - libm::log(self.is))
    }

    /// `Is * (exp(vj / nvt) - 1)`. Current, slope, whether the argument was clamped, and the
    /// voltage that was evaluated.
    fn exponential(&self, vj: f64) -> (f64, f64, bool, f64) {
        let cap = self.arg_cap();
        let arg = vj / self.nvt;
        let limited = arg > cap;
        let used = if limited { cap } else { arg };
        let e = libm::exp(used);
        let voltage = if limited { used * self.nvt } else { vj };
        (
            self.is * (e - 1.0),
            self.is * e / self.nvt,
            limited,
            voltage,
        )
    }

    /// ngspice-42's junction law, `dioload` with no breakdown voltage and each junction of
    /// `bjtload`. Below `-3 * nvt` the current is `-Is * (1 + (3 * nvt / (e * vj))^3)`, which
    /// meets the exponential there in current and slope. Returns what `exponential` does.
    pub(crate) fn evaluate(&self, vj: f64) -> (f64, f64, bool, f64) {
        if vj >= -3.0 * self.nvt {
            return self.exponential(vj);
        }
        let arg = 3.0 * self.nvt / (vj * std::f64::consts::E);
        let arg = arg * arg * arg;
        (-self.is * (1.0 + arg), self.is * 3.0 * arg / vj, false, vj)
    }
}

fn shockley(junction: &Junction) -> Shockley {
    Shockley::new(
        junction.emission_coefficient.si.get(),
        junction.saturation_current.si.get(),
    )
}

/// Limits the solved junction voltage against `prior` and checks the
/// linearised current against the device equation at the solved voltage.
pub(crate) fn update(junction: &Junction, probe: &Probe<'_>, prior: f64) -> (f64, Update) {
    let params = shockley(junction);
    let solved = probe.internal(0) - probe.v(junction.pins[1]);
    let (next, mut limited) = pnjlim(solved, prior, params.nvt, params.vcrit);
    let (id, gd, cap_prior, voltage) = params.evaluate(prior);
    let linear = id + gd * (solved - voltage);
    let (actual, _, cap_solved, _) = params.evaluate(solved);
    limited |= cap_prior || cap_solved;
    let mismatch = mismatch(linear, actual);
    (next, Update { limited, mismatch })
}

/// How far a linearised current is from the device equation, over Newton's tolerance on it.
/// At most 1 passes. A ratio that is not finite is infinite.
pub(crate) fn mismatch(linear: f64, actual: f64) -> f64 {
    let tolerance = RELTOL * linear.abs().max(actual.abs()) + ABSTOL;
    let mismatch = (linear - actual).abs() / tolerance;
    if mismatch.is_finite() {
        mismatch
    } else {
        f64::INFINITY
    }
}

/// Junction voltage limiting. Returns the voltage to stamp next and whether it differs from `vnew`.
///
/// Above `vcrit`, a forward step longer than `2 * nvt` is cut to a logarithmic
/// one, so `exp` stays in range. Below 0, a reverse step past `-1 - vold`
/// (or `2 * vold - 1` from a reverse `vold`) is clamped there. The reverse
/// clamp is a damping addition to the textbook forward rule.
pub(crate) fn pnjlim(vnew: f64, vold: f64, nvt: f64, vcrit: f64) -> (f64, bool) {
    if vnew > vcrit && (vnew - vold).abs() > 2.0 * nvt {
        let limited = if vold > 0.0 {
            let arg = 1.0 + (vnew - vold) / nvt;
            if arg > 0.0 {
                vold + nvt * libm::log(arg)
            } else {
                vcrit
            }
        } else {
            nvt * libm::log(vnew / nvt)
        };
        return (limited, true);
    }
    if vnew < 0.0 {
        let floor = if vold > 0.0 {
            -1.0 - vold
        } else {
            2.0 * vold - 1.0
        };
        if vnew < floor {
            return (floor, true);
        }
    }
    (vnew, false)
}

#[cfg(test)]
mod tests {
    use crate::netlist::{Model, Netlist};

    use super::{Shockley, pnjlim, update};
    use crate::mna::{Probe, Solution};
    use crate::newton::Circuit;

    const NVT: f64 = 0.1;
    const VCRIT: f64 = 2.0;

    #[test]
    fn a_small_step_passes_through() {
        assert_eq!(pnjlim(2.15, 2.1, NVT, VCRIT), (2.15, false));
        assert_eq!(pnjlim(-0.5, 0.0, NVT, VCRIT), (-0.5, false));
    }

    #[test]
    fn a_step_below_vcrit_is_not_limited_however_long() {
        assert_eq!(pnjlim(1.9, -0.5, NVT, VCRIT), (1.9, false));
    }

    #[test]
    fn a_long_forward_step_from_a_forward_voltage_grows_logarithmically() {
        let (v, limited) = pnjlim(5.0, 2.0, NVT, VCRIT);
        assert!(limited);
        assert_eq!(v, 2.0 + NVT * libm::log(1.0 + 3.0 / NVT));
        assert!(v < 2.5, "{v}");
    }

    #[test]
    fn a_long_forward_step_from_reverse_lands_on_the_log_curve() {
        let (v, limited) = pnjlim(5.0, -1.0, NVT, VCRIT);
        assert!(limited);
        assert_eq!(v, NVT * libm::log(5.0 / NVT));
    }

    #[test]
    fn a_long_backward_step_that_overshoots_falls_back_to_vcrit() {
        assert_eq!(pnjlim(2.5, 4.0, NVT, VCRIT), (VCRIT, true));
    }

    #[test]
    fn a_long_reverse_step_is_clamped_from_a_forward_voltage() {
        assert_eq!(pnjlim(-10.0, 2.0, NVT, VCRIT), (-3.0, true));
        assert_eq!(pnjlim(-2.5, 2.0, NVT, VCRIT), (-2.5, false));
    }

    #[test]
    fn a_long_reverse_step_from_a_reverse_voltage_doubles_at_most() {
        assert_eq!(pnjlim(-10.0, -3.0, NVT, VCRIT), (-7.0, true));
        assert_eq!(pnjlim(-10.0, -7.0, NVT, VCRIT), (-10.0, false));
    }

    #[test]
    fn the_reverse_law_meets_the_exponential_at_three_nvt() {
        let params = Shockley {
            is: 6.711e-10,
            nvt: 0.114,
            vcrit: 2.0,
        };
        let edge = -3.0 * params.nvt;
        let (above, above_slope, _, _) = params.evaluate(edge);
        let (below, below_slope, _, _) = params.evaluate(edge * (1.0 + 1e-12));
        assert!(
            (above - below).abs() <= 1e-9 * above.abs(),
            "{above} {below}"
        );
        assert!(
            (above_slope - below_slope).abs() <= 1e-9 * above_slope,
            "{above_slope} {below_slope}"
        );
    }

    #[test]
    fn the_cap_holds_the_slope_bound_for_a_tiny_saturation_current() {
        let params = Shockley {
            is: 1e-308,
            nvt: 1e-300,
            vcrit: 1e-299,
        };
        let cap = params.arg_cap();
        assert!(
            (cap - 108.0 * std::f64::consts::LN_10).abs() < 1e-9,
            "{cap}"
        );
        let (_, slope, _, _) = params.evaluate(params.nvt * 1000.0);
        assert!(slope <= super::G_CAP * (1.0 + 1e-9), "{slope}");
    }

    #[test]
    fn evaluate_caps_only_past_the_overflow_bound_and_the_iteration_is_limited() {
        let params = Shockley {
            is: 1e-14,
            nvt: 0.026,
            vcrit: 0.7,
        };
        let cap = params.arg_cap();
        assert!(cap > 200.0, "{cap}");
        let (below, _, open, voltage) = params.evaluate(params.nvt * 100.0);
        assert!(!open);
        assert_eq!(voltage, params.nvt * 100.0);
        let full = params.is * (libm::exp(100.0) - 1.0);
        assert!((below - full).abs() <= full * 1e-12);

        let over_v = params.nvt * (cap + 1.0);
        let (current, slope, over, clamped) = params.evaluate(over_v);
        assert!(over);
        assert_eq!(clamped, cap * params.nvt);
        let expected = params.is * (libm::exp(cap) - 1.0);
        assert!((current - expected).abs() <= expected * 1e-12);
        assert!(slope.is_finite() && slope > 0.0);

        let netlist = diode();
        let circuit = Circuit::new(netlist.clone()).unwrap();
        let solution = Solution::zeros(circuit.layout.size());
        let (id, part) = netlist
            .parts()
            .find(|(_, part)| part.name.as_str() == "d1")
            .unwrap();
        let Model::Diode(junction) = &part.model else {
            panic!("{:?}", part.model);
        };
        let probe = Probe {
            layout: &circuit.layout,
            part: id,
            x: &solution,
        };
        // 40 V is above vcrit and above the exp cap. The solved step back to 0 is not a pnjlim hit.
        let (_, report) = update(junction, &probe, 40.0);
        assert!(report.limited);
    }

    /// Up to eight segments of a common-anode display whose anode is on no other net, each
    /// cathode through 330 ohm to an input pin that leaks 5.5 Mohm to the supply and to
    /// ground. The anode settles a little below the pins, where node gmin balances the
    /// reverse junctions. Stamped as two conductances, six segments at 3.3 V never settled.
    #[test]
    fn a_floating_common_anode_settles_on_plain_newton() {
        for segments in 1..=8 {
            for supply in [3.3, 5.0] {
                floating_common_anode(segments, supply);
            }
        }
    }

    fn floating_common_anode(segments: usize, supply: f64) {
        let mut nets = vec!["gnd".to_string(), "vcc".to_string(), "com".to_string()];
        let mut parts = vec![serde_json::json!({
            "name": "vcc", "kind": "voltage_source", "pins": { "p": "vcc", "n": "gnd" },
            "voltage": { "value": supply, "unit": "V" }
        })];
        for k in 0..segments {
            let (cathode, pin) = (format!("k{k}"), format!("p{k}"));
            nets.extend([cathode.clone(), pin.clone()]);
            parts.extend([
                serde_json::json!({
                    "name": format!("seg{k}"), "kind": "led",
                    "pins": { "anode": "com", "cathode": cathode },
                    "saturation_current": { "value": 3.272e-14, "unit": "A" },
                    "emission_coefficient": { "value": 2.509, "unit": "1" },
                    "series_resistance": { "value": 9.532, "unit": "ohm" },
                    "ratings": { "current": null, "power": null }
                }),
                serde_json::json!({
                    "name": format!("r{k}"), "kind": "resistor",
                    "pins": { "a": cathode, "b": pin },
                    "resistance": { "value": 330.0, "unit": "ohm" }, "ratings": { "power": null }
                }),
                serde_json::json!({
                    "name": format!("up{k}"), "kind": "resistor",
                    "pins": { "a": pin, "b": "vcc" },
                    "resistance": { "value": 5.5e6, "unit": "ohm" }, "ratings": { "power": null }
                }),
                serde_json::json!({
                    "name": format!("down{k}"), "kind": "resistor",
                    "pins": { "a": pin, "b": "gnd" },
                    "resistance": { "value": 5.5e6, "unit": "ohm" }, "ratings": { "power": null }
                }),
            ]);
        }
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1, "seed": 0, "analysis": { "op": {} }, "ground": "gnd",
            "nets": nets, "parts": parts
        }))
        .unwrap();
        let circuit = Circuit::new(crate::netlist::parse(&bytes).unwrap()).unwrap();
        let converged = crate::newton::newton(
            &circuit,
            &mut crate::mna::Solver::new(&circuit.layout),
            &crate::newton::Conditions::NOMINAL,
            crate::newton::Iterate::initial(&circuit),
            crate::op::OP_BUDGET,
        );
        assert!(
            converged.is_ok(),
            "{segments} segments at {supply} V: {converged:?}"
        );
    }

    fn diode() -> Netlist {
        crate::netlist::parse(
            br#"{
              "version": 1,
              "seed": 0,
              "analysis": { "op": {} },
              "ground": "gnd",
              "nets": ["gnd", "a"],
              "parts": [
                {
                  "name": "d1",
                  "kind": "diode",
                  "pins": { "anode": "a", "cathode": "gnd" },
                  "saturation_current": { "value": 1e-14, "unit": "A" },
                  "emission_coefficient": { "value": 1.0, "unit": "1" },
                  "series_resistance": { "value": 1.0, "unit": "ohm" },
                  "ratings": { "current": null, "power": null }
                }
              ]
            }"#,
        )
        .unwrap()
    }
}

/// A Shockley junction in series with a resistance, at 298.15 K with no temperature scaling.
/// `i = Is * (exp(vj / (N * kT/q)) - 1)` down to `vj = -3 * N * kT/q`, ngspice-42's reverse law
/// `i = -Is * (1 + (3 * N * kT/q / (e * vj))^3)` below it, and `V(anode) - V(cathode) = vj + i * Rs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Junction {
    /// Nets of pins `anode`, `cathode`.
    pub pins: [NetId; 2],
    /// Ampere, greater than 0.
    pub saturation_current: Value,
    /// Unit `1`, greater than 0.
    pub emission_coefficient: Value,
    /// Ohm, greater than 0.
    pub series_resistance: Value,
    /// Ampere, greater than 0. `None` is an explicit `"current": null`.
    pub max_current: Option<Rating>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Rating>,
}

impl Junction {
    pub const PINS: [&'static str; 2] = ["anode", "cathode"];

    /// `k*T/q` at 298.15 K, from the exact SI values of `k` and `q`.
    pub const VT: f64 = 1.380649e-23 * 298.15 / 1.602176634e-19;
}

/// `n*VT` and `vcrit` for a junction whose parameters are inside the domain.
pub struct JunctionScale {
    pub nvt: f64,
    pub vcrit: f64,
}

/// Which junction field breaks `finite_junction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JunctionFault {
    EmissionCoefficient,
    SaturationCurrent,
}

impl JunctionFault {
    /// The field that breaks the rule. `emission` names the emission field, which is
    /// `emission_coefficient` on a diode and `forward_emission` or `reverse_emission` on a
    /// transistor.
    pub fn field(self, emission: &'static str) -> &'static str {
        match self {
            JunctionFault::EmissionCoefficient => emission,
            JunctionFault::SaturationCurrent => "saturation_current",
        }
    }

    pub(crate) fn value(self, emission: f64, saturation: f64) -> f64 {
        match self {
            JunctionFault::EmissionCoefficient => emission,
            JunctionFault::SaturationCurrent => saturation,
        }
    }
}

/// `n*VT` must be normal, and `vcrit = nvt*ln(nvt/(sqrt(2)*Is))` must be finite and positive.
///
/// `vcrit` is positive when `n*VT > sqrt(2)*Is`. A `vcrit` that fails the rule names
/// `saturation_current` when `Is` is not normal, and `emission_coefficient` otherwise.
/// `n*VT` itself names `emission_coefficient`.
pub fn junction_scale(emission: f64, saturation: f64) -> Result<JunctionScale, JunctionFault> {
    let nvt = emission * Junction::VT;
    if !nvt.is_normal() {
        return Err(JunctionFault::EmissionCoefficient);
    }
    let vcrit = nvt * libm::log(nvt / (SQRT_2 * saturation));
    if !(vcrit.is_finite() && vcrit > 0.0) {
        return Err(if saturation.is_normal() {
            JunctionFault::EmissionCoefficient
        } else {
            JunctionFault::SaturationCurrent
        });
    }
    Ok(JunctionScale { nvt, vcrit })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireJunction {
    pub(crate) name: String,
    pins: PinMap,
    saturation_current: WireValue,
    emission_coefficient: WireValue,
    series_resistance: WireValue,
    ratings: WireJunctionRatings,
}

pub(crate) fn resolve_junction(ctx: ParseCtx<'_>, wire: WireJunction) -> Option<Junction> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Junction::PINS, &wire.pins.0, nets, out);
    let is = resolve_value(
        part_ref,
        "saturation_current",
        "A",
        true,
        wire.saturation_current,
        out,
    );
    let n = resolve_value(
        part_ref,
        "emission_coefficient",
        "1",
        true,
        wire.emission_coefficient,
        out,
    );
    let rs = resolve_value(
        part_ref,
        "series_resistance",
        "ohm",
        true,
        wire.series_resistance,
        out,
    );
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    if let (Some(is), Some(n)) = (&is, &n)
        && let Err(fault) = junction_scale(n.si.get(), is.si.get())
    {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: fault.field("emission_coefficient").into(),
            value: fault.value(n.si.get(), is.si.get()),
            rule: ValueRule::FiniteJunction,
        });
        return None;
    }
    Some(Junction {
        pins: pins?,
        saturation_current: is?,
        emission_coefficient: n?,
        series_resistance: rs?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

/// Diode and led. The equations match. The variant name does not.
impl Kind for Junction {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 1,
            bare_internal: true,
            branches: 0,
            junctions: 1,
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    /// Rs from the anode to internal node 0, then the junction from there to the
    /// cathode, linearised at the voltage `evaluate` used. `GMIN` is across that
    /// junction. ngspice-42's `dioload` adds `CKTgmin` to `gd` and `CKTgmin * vd`
    /// to the current, and it puts no conductance from the internal node to ground.
    ///
    /// The internal node's row is its own current law. The anode and cathode rows carry
    /// Rs and the junction in series, `G * g / (G + g)`, which is what eliminating the
    /// internal node gives, so no other row reads the internal node. Stamped as two
    /// conductances, a node that reaches the rest only through reverse junctions, such as
    /// a floating common anode, would be left by LU as `G - G * G / (G + g)`. With `G`
    /// near 0.1 S and `g` near 1 pS that difference keeps about five digits, too few for
    /// Newton to settle on that node's voltage.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let params = shockley(self);
        let [anode, cathode] = self.pins;
        let inner = stamp.internal(0);
        let rs = self.series_resistance.si.get();
        let vj = stamp.junction(0);
        let (id, gd, _, voltage) = params.evaluate(vj);
        let g = gd + GMIN;
        // The companion is the derivative at the clamped voltage, not at an argument `exp`
        // never saw. ngspice's is `cd - gd * vd` with `GMIN` in `cd` and `gd`, and `GMIN * vd`
        // cancels there.
        let companion = id - gd * voltage;
        stamp.add(inner, inner, 1.0 / rs + g);
        stamp.add(inner, anode, -1.0 / rs);
        stamp.add(inner, cathode, -g);
        stamp.current(inner, Node::Ground, companion);
        let share = 1.0 / (1.0 + rs * g);
        stamp.conductance(anode, cathode, g * share);
        stamp.current(anode, cathode, companion * share);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, out: &mut [f64]) {
        out[0] = shockley(self).vcrit;
    }
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
        let (limited, update) = update(self, probe, prior[0]);
        next[0] = limited;
        update
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Rating)>, Option<&Rating>) {
        let current = self.max_current.as_ref().map(|v| (Quantity::I, v));
        (current, self.max_power.as_ref())
    }
    /// The current through Rs, entering the anode.
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        let i = (probe.v(self.pins[0]) - probe.internal(0)) / self.series_resistance.si.get();
        Some(Observed::two_pin(self.pins, i, probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Passes
    }
}
