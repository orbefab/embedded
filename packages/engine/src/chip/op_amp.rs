use orbe_model::Time;
use serde::Deserialize;

use crate::diagnostic::ValueRule;
use crate::element::{Observed, OwnedCapacitor, OwnedInductor, StoreEnd, Update};
use crate::mna::{Node, Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::{ParseCtx, PinMap, WireValue, bounded, resolve_pins, resolve_value};
use crate::netlist::{Conduction, Kind, NetId, Value};
use crate::newton::{RELTOL, Side, VNTOL};
use crate::passive::capacitor::stamp_capacitor;

/// The integrator's capacitance, farad. The slew current and the transconductance scale with
/// it, so its size moves no answer. It is large enough that Newton's current floor stays far
/// below its truncation tolerance at a 10 ms step.
const C_INT: f64 = 1e-9;
/// How far past each rail the integrator may run before its clamp, volt.
const OVERRUN: f64 = 0.1;
const CLAMP_KNEE: f64 = 0.01;
/// The clamp's conductance per farad of `C_INT`, so its time constant does not move with it.
const CLAMP_SIEMENS: f64 = 1e7;
/// The output's swing knee, volt.
const SWING_KNEE: f64 = 1e-3;
/// The current limit's knee as a share of the limit.
const LIMIT_FRACTION: f64 = 1e-3;
/// The current limit's slope past its knee times the limit, volt.
const LIMIT_HEADROOM: f64 = 500.0;
/// The output current over which the supply share moves from `gnd` to `vcc`, ampere.
const SPLIT_A: f64 = 1e-6;
/// The input pair turns off above `vcc` less this, volt.
const INPUT_HEADROOM: f64 = 0.8;
const INPUT_KNEE: f64 = 0.05;
/// The supply at which the supply conductance draws `supply_current`, volt.
const SUPPLY_NOMINAL: f64 = 5.0;

/// `sp(x) = ln(1 + e^x)`, branched so `exp` is never of a positive overflow.
fn softplus(x: f64) -> f64 {
    if x > 0.0 {
        x + libm::log1p(libm::exp(-x))
    } else {
        libm::log1p(libm::exp(x))
    }
}

fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + libm::exp(-x))
    } else {
        let e = libm::exp(x);
        e / (1.0 + e)
    }
}

const IN_P: usize = 0;
const IN_N: usize = 1;
const OUT: usize = 2;
const VCC: usize = 3;
const GND: usize = 4;

/// One op-amp channel, a behavioural element with one pole. Internal node 0 is an integrator
/// with `C_INT` to `gnd`, branch 0 is that capacitor, and branch 1 is the output's current.
/// A dual part maps one channel per body.
///
/// A transconductance with a tanh limit charges the integrator, so the slew rate is
/// `slew_rate` and the unity-gain frequency is `gain_bandwidth`. A resistor to mid-supply sets
/// the DC gain to `open_loop_gain`, and a soft clamp holds the integrator within `OVERRUN` of
/// each rail. The output is a voltage law of the integrator, smoothly clamped to the swing,
/// less `output_resistance` times the output current, less a current limit. It sources from
/// `vcc` and sinks to `gnd`. A bias current leaves each input, drawn from `vcc`, and turns off
/// as the input nears `vcc`. A conductance from `vcc` to `gnd` draws `supply_current` at 5 V.
/// Every current enters and leaves at a pin. `docs/engine/netlist.md` gives the law.
#[derive(Clone, Debug, PartialEq)]
pub struct OpAmp {
    /// Nets of pins `in_p`, `in_n`, `out`, `vcc`, `gnd`.
    pub pins: [NetId; 5],
    /// Hz, greater than 0. The unity-gain frequency.
    pub gain_bandwidth: Value,
    /// V/s, greater than 0. Rising and falling.
    pub slew_rate: Value,
    /// Unit `1`, greater than 0. The DC gain with no load.
    pub open_loop_gain: Value,
    /// Ampere, at least 0. Leaves each input.
    pub input_bias_current: Value,
    /// Volt, at least 0. How far below `vcc` the output stops with no load.
    pub swing_high: Value,
    /// Volt, at least 0. How far above `gnd` the output stops with no load.
    pub swing_low: Value,
    /// Ohm, at least 0. The output's fall per ampere it sources, and rise per ampere it sinks.
    pub output_resistance: Value,
    /// Ampere, greater than 0. The output current the limit holds, sourcing and sinking.
    pub short_circuit_current: Value,
    /// Ampere, at least 0. One channel's supply current at a 5 V supply.
    pub supply_current: Value,
}

impl OpAmp {
    pub const PINS: [&'static str; 5] = ["in_p", "in_n", "out", "vcc", "gnd"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireOpAmp {
    pub(crate) name: String,
    pins: PinMap,
    gain_bandwidth: WireValue,
    slew_rate: WireValue,
    open_loop_gain: WireValue,
    input_bias_current: WireValue,
    swing_high: WireValue,
    swing_low: WireValue,
    output_resistance: WireValue,
    short_circuit_current: WireValue,
    supply_current: WireValue,
}

pub(crate) fn resolve_op_amp(ctx: ParseCtx<'_>, wire: WireOpAmp) -> Option<OpAmp> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, OpAmp::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let gain_bandwidth = positive("gain_bandwidth", "Hz", wire.gain_bandwidth);
    let slew_rate = positive("slew_rate", "V/s", wire.slew_rate);
    let open_loop_gain = positive("open_loop_gain", "1", wire.open_loop_gain);
    let short_circuit_current = positive("short_circuit_current", "A", wire.short_circuit_current);
    let output_resistance = positive("output_resistance", "ohm", wire.output_resistance);
    let mut non_negative = |field: &'static str, unit, value| {
        bounded(
            part_ref,
            field,
            unit,
            value,
            |number| (number < 0.0).then_some(ValueRule::NonNegative),
            out,
        )
    };
    let input_bias_current = non_negative("input_bias_current", "A", wire.input_bias_current);
    let swing_high = non_negative("swing_high", "V", wire.swing_high);
    let swing_low = non_negative("swing_low", "V", wire.swing_low);
    let supply_current = non_negative("supply_current", "A", wire.supply_current);
    Some(OpAmp {
        pins: pins?,
        gain_bandwidth: gain_bandwidth?,
        slew_rate: slew_rate?,
        open_loop_gain: open_loop_gain?,
        input_bias_current: input_bias_current?,
        swing_high: swing_high?,
        swing_low: swing_low?,
        output_resistance: output_resistance?,
        short_circuit_current: short_circuit_current?,
        supply_current: supply_current?,
    })
}

/// The output law at integrator voltage `u`, supply `s` and output current `i`, all from
/// `gnd`, with its slopes.
struct Output {
    vo: f64,
    du: f64,
    ds: f64,
    di: f64,
}

fn output(op: &OpAmp, u: f64, s: f64, i: f64) -> Output {
    let lo = op.swing_low.si.get();
    let hi = s - op.swing_high.si.get();
    let k = SWING_KNEE;
    let t = lo + k * softplus((u - lo) / k) - k * softplus((u - hi) / k);
    let ro = op.output_resistance.si.get();
    let isc = op.short_circuit_current.si.get();
    let d = LIMIT_FRACTION * isc;
    let rl = LIMIT_HEADROOM / isc;
    let over = (i - isc) / d;
    let under = (-i - isc) / d;
    Output {
        vo: t - ro * i - rl * d * softplus(over) + rl * d * softplus(under),
        du: sigmoid((u - lo) / k) - sigmoid((u - hi) / k),
        ds: sigmoid((u - hi) / k),
        di: -ro - rl * sigmoid(over) - rl * sigmoid(under),
    }
}

/// A current `i0 + sum(c * (V(node) - V0(node)))` from `from` through the part to `to`,
/// linearised at the prior solution.
fn flow(stamp: &mut Stamp<'_>, from: Node, to: Node, i0: f64, terms: &[(Node, f64, f64)]) {
    let mut constant = i0;
    for &(node, c, v0) in terms {
        stamp.add(from, node, c);
        stamp.add(to, node, -c);
        constant -= c * v0;
    }
    stamp.current(from, to, constant);
}

impl Kind for OpAmp {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 1,
            bare_internal: true,
            branches: 2,
            junctions: 0,
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        vec![OwnedCapacitor {
            k: 0,
            ends: [StoreEnd::Internal(0), StoreEnd::Net(self.pins[GND])],
            farad: C_INT,
            sized_by_kind: true,
        }]
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    /// Every nonlinear term is linearised at the last solution.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let pin = |k: usize| Node::from(self.pins[k]);
        let (inp, inn, out, vcc, gnd) = (pin(IN_P), pin(IN_N), pin(OUT), pin(VCC), pin(GND));
        let x = stamp.internal(0);
        stamp_capacitor(
            stamp,
            0,
            [StoreEnd::Internal(0), StoreEnd::Net(self.pins[GND])],
            C_INT,
            None,
        );

        let (vp, vn, vx, vs, vg, i_prev) = {
            let prior = stamp.prior();
            let v = |n: Node| prior.v(n);
            (v(inp), v(inn), v(x), v(vcc), v(gnd), prior.branch(1))
        };
        let u = vx - vg;
        let s = vs - vg;

        stamp.conductance(vcc, gnd, self.supply_current.si.get() / SUPPLY_NOMINAL);

        let ib = self.input_bias_current.si.get();
        for (input, v_in) in [(inp, vp), (inn, vn)] {
            let z = (s - INPUT_HEADROOM - (v_in - vg)) / INPUT_KNEE;
            let on = sigmoid(z);
            let slope = ib * on * (1.0 - on) / INPUT_KNEE;
            flow(
                stamp,
                vcc,
                input,
                ib * on,
                &[(input, -slope, v_in), (vcc, slope, vs)],
            );
        }

        let is = self.slew_rate.si.get() * C_INT;
        let gm = 2.0 * std::f64::consts::PI * self.gain_bandwidth.si.get() * C_INT;
        let r = self.open_loop_gain.si.get() / gm;
        let th = libm::tanh(gm * (vp - vn) / is);
        let g = gm * (1.0 - th * th);
        flow(stamp, gnd, x, is * th, &[(inp, g, vp), (inn, -g, vn)]);
        flow(
            stamp,
            x,
            gnd,
            (u - s / 2.0) / r,
            &[(x, 1.0 / r, vx), (vcc, -0.5 / r, vs), (gnd, -0.5 / r, vg)],
        );
        let siemens = CLAMP_SIEMENS * C_INT;
        let top = (u - s - OVERRUN) / CLAMP_KNEE;
        let bottom = (-OVERRUN - u) / CLAMP_KNEE;
        let clamp = siemens * CLAMP_KNEE * (softplus(top) - softplus(bottom));
        let du = siemens * (sigmoid(top) + sigmoid(bottom));
        let ds = -siemens * sigmoid(top);
        flow(
            stamp,
            x,
            gnd,
            clamp,
            &[(x, du, vx), (vcc, ds, vs), (gnd, -du - ds, vg)],
        );

        let law = output(self, u, s, i_prev);
        let row = Node::Row(stamp.layout.branch_row(stamp.part, 1));
        stamp.branch_row(
            out,
            gnd,
            1,
            -law.di,
            1.0,
            law.vo - law.du * u - law.ds * s - law.di * i_prev,
        );
        stamp.add(row, x, -law.du);
        stamp.add(row, vcc, -law.ds);
        stamp.add(row, gnd, law.du + law.ds);

        let share = sigmoid(i_prev / SPLIT_A);
        let w = i_prev * share;
        let dw = share + i_prev / SPLIT_A * share * (1.0 - share);
        stamp.add(vcc, row, dw);
        stamp.add(gnd, row, 1.0 - dw);
        stamp.add(out, row, -1.0);
        stamp.current(vcc, gnd, w - dw * i_prev);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, _out: &mut [f64]) {}
    /// The output row is a voltage, so the check uses the voltage tolerance, as the
    /// regulator's does.
    fn update(&self, probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        let pin = |k: usize| probe.v(self.pins[k]);
        let u = probe.internal(0) - pin(GND);
        let s = pin(VCC) - pin(GND);
        let vo = pin(OUT) - pin(GND);
        let law = output(self, u, s, probe.branch(1));
        let finite = vo.is_finite() && law.vo.is_finite();
        let scale = RELTOL * vo.abs().max(law.vo.abs()) + VNTOL;
        Update {
            limited: false,
            mismatch: if finite {
                (vo - law.vo).abs() / scale
            } else {
                f64::INFINITY
            },
        }
    }
    /// `out` reaches `vcc` and `gnd`, and the supply conductance joins those two. An input
    /// carries only its bias current.
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(OUT, VCC), (OUT, GND), (VCC, GND)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        &[]
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, _probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        None
    }
    fn conduction(&self) -> Conduction {
        Conduction::Other
    }
}
