use sim_core::netlist::{Junction, junction_scale};

use crate::element::Update;
use crate::mna::{Probe, Stamp};
use crate::newton::{ABSTOL, RELTOL};

/// `exp` overflows near 709.78. Stay under that.
const ARG_MAX: f64 = 700.0;
/// A conductance no operating point reaches. The cap keeps `Is*exp(arg)/nvt` under this.
const G_CAP: f64 = 1e100;

struct Params {
    is: f64,
    nvt: f64,
    rs: f64,
    vcrit: f64,
}

impl Params {
    fn new(junction: &Junction) -> Params {
        let is = junction.saturation_current.si.get();
        let scale = junction_scale(junction.emission_coefficient.si.get(), is)
            .expect("parse accepts only a finite junction");
        Params {
            is,
            nvt: scale.nvt,
            rs: junction.series_resistance.si.get(),
            vcrit: scale.vcrit,
        }
    }

    /// Largest `exp` argument whose current and slope stay finite and under `G_CAP`.
    fn arg_cap(&self) -> f64 {
        // In log space, so a tiny `Is` cannot overflow the quotient.
        ARG_MAX.min(G_CAP.ln() + self.nvt.ln() - self.is.ln())
    }

    /// Current, slope, whether the argument was clamped, and the voltage that was evaluated.
    fn evaluate(&self, vj: f64) -> (f64, f64, bool, f64) {
        let cap = self.arg_cap();
        let arg = vj / self.nvt;
        let limited = arg > cap;
        let used = if limited { cap } else { arg };
        let e = used.exp();
        let voltage = if limited { used * self.nvt } else { vj };
        (
            self.is * (e - 1.0),
            self.is * e / self.nvt,
            limited,
            voltage,
        )
    }
}

/// Rs from the anode to internal node 0, then the junction from there to the
/// cathode, linearised at the voltage `evaluate` used.
pub(super) fn stamp(junction: &Junction, stamp: &mut Stamp<'_>) {
    let params = Params::new(junction);
    let [anode, cathode] = junction.pins;
    let inner = stamp.internal(0);
    stamp.conductance(anode, inner, 1.0 / params.rs);
    let vj = stamp.junction(0);
    let (id, gd, _, voltage) = params.evaluate(vj);
    stamp.conductance(inner, cathode, gd);
    // The companion is the derivative at the clamped voltage, not at an argument `exp` never saw.
    stamp.current(inner, cathode, id - gd * voltage);
}

pub(super) fn initial(junction: &Junction) -> f64 {
    Params::new(junction).vcrit
}

/// Limits the solved junction voltage against `prior` and checks the
/// linearised current against the device equation at the solved voltage.
pub(super) fn update(junction: &Junction, probe: &Probe<'_>, prior: f64) -> (f64, Update) {
    let params = Params::new(junction);
    let solved = probe.internal(0) - probe.v(junction.pins[1]);
    let (next, mut limited) = pnjlim(solved, prior, params.nvt, params.vcrit);
    let (id, gd, cap_prior, voltage) = params.evaluate(prior);
    let linear = id + gd * (solved - voltage);
    let (actual, _, cap_solved, _) = params.evaluate(solved);
    limited |= cap_prior || cap_solved;
    let tolerance = RELTOL * linear.abs().max(actual.abs()) + ABSTOL;
    let mismatch = (linear - actual).abs() / tolerance;
    let mismatch = if mismatch.is_finite() {
        mismatch
    } else {
        f64::INFINITY
    };
    (next, Update { limited, mismatch })
}

/// The current through Rs, entering the anode.
pub(super) fn current(junction: &Junction, probe: &Probe<'_>) -> f64 {
    let params = Params::new(junction);
    (probe.v(junction.pins[0]) - probe.internal(0)) / params.rs
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
                vold + nvt * arg.ln()
            } else {
                vcrit
            }
        } else {
            nvt * (vnew / nvt).ln()
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
    use sim_core::netlist::{Model, Netlist};

    use super::{Params, pnjlim, update};
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
        assert_eq!(v, 2.0 + NVT * (1.0 + 3.0 / NVT).ln());
        assert!(v < 2.5, "{v}");
    }

    #[test]
    fn a_long_forward_step_from_reverse_lands_on_the_log_curve() {
        let (v, limited) = pnjlim(5.0, -1.0, NVT, VCRIT);
        assert!(limited);
        assert_eq!(v, NVT * (5.0 / NVT).ln());
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
    fn the_cap_holds_the_slope_bound_for_a_tiny_saturation_current() {
        let params = Params {
            is: 1e-308,
            nvt: 1e-300,
            rs: 1.0,
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
        let params = Params {
            is: 1e-14,
            nvt: 0.026,
            rs: 1.0,
            vcrit: 0.7,
        };
        let cap = params.arg_cap();
        assert!(cap > 200.0, "{cap}");
        let (below, _, open, voltage) = params.evaluate(params.nvt * 100.0);
        assert!(!open);
        assert_eq!(voltage, params.nvt * 100.0);
        let full = params.is * (100.0_f64.exp() - 1.0);
        assert!((below - full).abs() <= full * 1e-12);

        let over_v = params.nvt * (cap + 1.0);
        let (current, slope, over, clamped) = params.evaluate(over_v);
        assert!(over);
        assert_eq!(clamped, cap * params.nvt);
        let expected = params.is * (cap.exp() - 1.0);
        assert!((current - expected).abs() <= expected * 1e-12);
        assert!(slope.is_finite() && slope > 0.0);

        let netlist = diode();
        let circuit = Circuit::new(&netlist).unwrap();
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

    fn diode() -> Netlist {
        sim_core::netlist::parse(
            br#"{
              "version": 0,
              "seed": 0,
              "analysis": { "op": {} },
              "ground": "gnd",
              "nets": ["gnd", "a"],
              "parts": [
                {
                  "name": "d1",
                  "kind": "diode",
                  "pins": { "anode": "a", "cathode": "gnd" },
                  "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
                  "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
                  "series_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
                  "ratings": { "current": null, "power": null }
                }
              ]
            }"#,
        )
        .unwrap()
    }
}
