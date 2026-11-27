//! A three-pin analog temperature sensor (`vs`, `vout`, `gnd`) such as the TMP36.
//!
//! Its open-circuit output voltage follows `output_voltage` (a fixed value or a curve driven by
//! `world.temperature`), gated by the supply `V(vs) - V(gnd)` and smoothly clamped between `0`
//! and `V(vs) - V(gnd)`, in series with `output_resistance` (a fixed value or a world-driven
//! curve). While powered it draws `quiescent_current` from `vs` to `gnd`, plus any current
//! sourced out of `vout`; current sunk into `vout` returns to `gnd`. When unpowered
//! (`V(vs) - V(gnd) <= 0`), the output sits at `0 V` over `gnd` and the supply draws nothing.
//!
//! Operating outside `ratings.operating_supply` while powered (`V(vs) - V(gnd) >= UNPOWERED_V`)
//! keeps working (with the output still clamped to `vs`) and warns `rating.voltage`. Supply
//! voltages above `ratings.supply_voltage` warn `rating.voltage` past the absolute maximum
//! ceiling.

use orbe_model::Time;
use serde::Deserialize;

use crate::diagnostic::{Diagnostic, ValueRule};
use crate::element::{Observed, OwnedCapacitor, OwnedInductor, Update};
use crate::mna::{Node, Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::{
    ParseCtx, PinMap, WireValue, resolve_limit, resolve_pins, resolve_value,
};
use crate::netlist::{Conduction, Kind, NetId, Rating, Schedule, Value, broken_positive};
use crate::newton::{RELTOL, Side, VNTOL};
use crate::world::json::{FieldRule, WireParam, resolve_param};
use crate::world::{Param, Quantity as WorldQuantity};

/// A three-pin analog temperature sensor: `vs`, `vout`, `gnd`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalogTempSensor {
    /// `[vs, vout, gnd]`.
    pub pins: [NetId; 3],
    pub output_voltage: Param,
    pub output_resistance: Param,
    pub quiescent_current: Value,
    /// Operating supply range `(min, max)` in volts (`vs` over `gnd`). `None` rates no operating
    /// supply window.
    pub operating_supply: Option<(f64, f64)>,
    /// Absolute maximum supply voltage (`vs` over `gnd`). `None` rates no ceiling.
    pub max_supply: Option<Value>,
}

impl AnalogTempSensor {
    pub const PINS: [&'static str; 3] = ["vs", "vout", "gnd"];
}

/// Supply ramp width over which the sensor turns on from 0 to full drive, in volts.
const V_ON: f64 = 0.2;
/// Smooth clamp knee at 0 V and `V(vs) - V(gnd)`, in volts.
const KNEE_V: f64 = 1e-3;
/// Smooth sourcing/sinking split width on output current, in amperes.
const SPLIT_A: f64 = 1e-6;
/// Below this `V(vs) - V(gnd)` the sensor is treated as unpowered, so under-supply warnings stay
/// quiet.
pub(crate) const UNPOWERED_V: f64 = 0.1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireAnalogTempSensor {
    pub(crate) name: String,
    pins: PinMap,
    output_voltage: WireParam,
    output_resistance: WireParam,
    quiescent_current: WireValue,
    #[serde(default)]
    ratings: WireAnalogTempSensorRatings,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAnalogTempSensorRatings {
    #[serde(default)]
    operating_supply: Option<WireOperatingSupply>,
    #[serde(default)]
    supply_voltage: Option<WireValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireOperatingSupply {
    min: WireValue,
    max: WireValue,
}

pub(crate) fn resolve_analog_temp_sensor(
    ctx: ParseCtx<'_>,
    wire: WireAnalogTempSensor,
) -> Option<AnalogTempSensor> {
    let pins = resolve_pins(
        ctx.name,
        AnalogTempSensor::PINS,
        &wire.pins.0,
        ctx.nets,
        ctx.out,
    );
    let output_voltage = resolve_param(
        ctx.part,
        FieldRule {
            field: "output_voltage",
            unit: "V",
            broken: |v| (!v.is_finite()).then_some(ValueRule::Finite),
        },
        ctx.world,
        wire.output_voltage,
        ctx.out,
    );
    let output_resistance = resolve_param(
        ctx.part,
        FieldRule {
            field: "output_resistance",
            unit: "ohm",
            broken: broken_positive,
        },
        ctx.world,
        wire.output_resistance,
        ctx.out,
    );
    let quiescent_current = resolve_non_negative(
        ctx.part,
        "quiescent_current",
        "A",
        wire.quiescent_current,
        ctx.out,
    );
    let operating_supply =
        resolve_operating_supply(ctx.part, wire.ratings.operating_supply, ctx.out);
    let max_supply = resolve_limit(
        ctx.part,
        "ratings.supply_voltage",
        "V",
        wire.ratings.supply_voltage,
        ctx.out,
    );
    Some(AnalogTempSensor {
        pins: pins?,
        output_voltage: output_voltage?,
        output_resistance: output_resistance?,
        quiescent_current: quiescent_current?,
        operating_supply: operating_supply?,
        max_supply: max_supply?,
    })
}

fn resolve_non_negative(
    part: &crate::name::Ref,
    field: &'static str,
    expected: &'static str,
    wire: WireValue,
    out: &mut Vec<Diagnostic>,
) -> Option<Value> {
    let number = wire.value.get();
    let value = resolve_value(part, field, expected, false, wire, out);
    if number < 0.0 {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: std::borrow::Cow::Borrowed(field),
            value: number,
            rule: ValueRule::NonNegative,
        });
        return None;
    }
    value
}

fn resolve_operating_supply(
    part: &crate::name::Ref,
    wire: Option<WireOperatingSupply>,
    out: &mut Vec<Diagnostic>,
) -> Option<Option<(f64, f64)>> {
    let Some(WireOperatingSupply { min, max }) = wire else {
        return Some(None);
    };
    let lo = resolve_value(part, "ratings.operating_supply.min", "V", true, min, out);
    let hi = resolve_value(part, "ratings.operating_supply.max", "V", true, max, out);
    match (lo, hi) {
        (Some(lo), Some(hi)) if lo.si.get() < hi.si.get() => Some(Some((lo.si.get(), hi.si.get()))),
        (Some(lo), Some(_)) => {
            out.push(Diagnostic::NetlistValue {
                part: part.clone(),
                field: std::borrow::Cow::Borrowed("ratings.operating_supply.min"),
                value: lo.si.get(),
                rule: ValueRule::AboveHigh,
            });
            None
        }
        _ => None,
    }
}

struct Law {
    vo: f64,
    dvo_di: f64,
    dvo_dvs: f64,
    i_vs: f64,
    di_vs_di: f64,
    di_vs_dvs: f64,
}

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

/// Smoothstep `h(x)` and `h'(x)` on `[0, 1]`. Both are exact `0` for `x <= 0`.
fn step(x: f64) -> (f64, f64) {
    if x <= 0.0 {
        (0.0, 0.0)
    } else if x >= 1.0 {
        (1.0, 0.0)
    } else {
        (x * x * (3.0 - 2.0 * x), 6.0 * x * (1.0 - x))
    }
}

fn evaluate(iq: f64, vt: f64, ro: f64, i: f64, vs: f64) -> Law {
    // Clamp open-circuit target smoothly to [0, vs].
    let v_pos = KNEE_V * softplus(vt / KNEE_V);
    let u = (v_pos - vs) / KNEE_V;
    let v_hi = v_pos - KNEE_V * softplus(u);
    let dv_hi_dvs = sigmoid(u);

    let x = vs / V_ON;
    let (h, hp) = step(x);
    let dh_dvs = hp / V_ON;

    let v_oc = h * v_hi;
    let dv_oc_dvs = dh_dvs * v_hi + h * dv_hi_dvs;

    let vo = v_oc - ro * i;
    let dvo_di = -ro;
    let dvo_dvs = dv_oc_dvs;

    // Quiescent supply current: 0 at vs = 0, iq for vs >= V_ON, passive ohmic load for vs < 0.
    let (hoff, hpoff) = step(-x);
    let ig = iq * h + iq * x * hoff;
    let dig_dvs = iq * dh_dvs + (iq / V_ON) * hoff - (iq * vs / (V_ON * V_ON)) * hpoff;

    // Sourced output current (i > 0) draws from vs; sunk current (i < 0) returns to gnd.
    let s = sigmoid(i / SPLIT_A);
    let ds_di = s * (1.0 - s) / SPLIT_A;
    let i_vs = ig + h * i * s;
    let di_vs_dvs = dig_dvs + dh_dvs * i * s;
    let di_vs_di = h * (s + i * ds_di);

    Law {
        vo,
        dvo_di,
        dvo_dvs,
        i_vs,
        di_vs_di,
        di_vs_dvs,
    }
}

impl Kind for AnalogTempSensor {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let [pin_vs, pin_vout, pin_gnd] = self.pins;
        let conditions = stamp.conditions;
        let side = conditions.side();
        let vt = self.output_voltage.read(side, conditions.at);
        let ro = self.output_resistance.read(side, conditions.at);
        let prior = stamp.prior();
        let i = prior.branch(0);
        let vs = prior.v(pin_vs) - prior.v(pin_gnd);
        let law = evaluate(self.quiescent_current.si.get(), vt, ro, i, vs);
        let row = Node::Row(stamp.layout.branch_row(stamp.part, 0));

        // Branch 0 carries `i` from gnd into the sensor and out of vout.
        stamp.branch_current(pin_gnd, pin_vout, 0);
        // Branch equation: (V(vout) - V(gnd)) - dvo_di * i - dvo_dvs * (V(vs) - V(gnd)) = rhs.
        stamp.branch_row(
            pin_vout,
            pin_gnd,
            0,
            -law.dvo_di,
            1.0,
            law.vo - law.dvo_di * i - law.dvo_dvs * vs,
        );
        stamp.add(row, pin_vs, -law.dvo_dvs);
        stamp.add(row, pin_gnd, law.dvo_dvs);

        // Total current leaving `pin_vs` into the sensor and returning to `pin_gnd`.
        stamp.current(
            pin_vs,
            pin_gnd,
            law.i_vs - law.di_vs_di * i - law.di_vs_dvs * vs,
        );
        stamp.conductance(pin_vs, pin_gnd, law.di_vs_dvs);
        stamp.add(pin_vs, row, law.di_vs_di);
        stamp.add(pin_gnd, row, -law.di_vs_di);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        unreachable!("AnalogTempSensor uses update_at")
    }
    fn update_at(
        &self,
        probe: &Probe<'_>,
        at: Time,
        side: Side,
        _prior: &[f64],
        _next: &mut [f64],
    ) -> Update {
        let [pin_vs, pin_vout, pin_gnd] = self.pins;
        let vt = self.output_voltage.read(side, at);
        let ro = self.output_resistance.read(side, at);
        let i = probe.branch(0);
        let vs = probe.v(pin_vs) - probe.v(pin_gnd);
        let vo = probe.v(pin_vout) - probe.v(pin_gnd);
        let law = evaluate(self.quiescent_current.si.get(), vt, ro, i, vs);
        let finite = vo.is_finite() && law.vo.is_finite() && law.i_vs.is_finite();
        let scale = RELTOL * vo.abs().max(law.vo.abs()) + VNTOL;
        let voltage = (vo - law.vo).abs() / scale;
        Update {
            limited: false,
            mismatch: if finite { voltage } else { f64::INFINITY },
        }
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 2), (1, 2)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        &[Quantity::Iin, Quantity::Iout, Quantity::P]
    }
    fn ratings(&self) -> (Option<(Quantity, &Rating)>, Option<&Rating>) {
        (None, None)
    }
    /// `iin` enters at `vs`, `iout` enters at `vout`, and `p` is the power entering at every pin.
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed> {
        let [pin_vs, pin_vout, pin_gnd] = self.pins;
        let vt = self.output_voltage.read(side, at);
        let ro = self.output_resistance.read(side, at);
        let i = probe.branch(0);
        let vs = probe.v(pin_vs) - probe.v(pin_gnd);
        let vo = probe.v(pin_vout) - probe.v(pin_gnd);
        let iin = evaluate(self.quiescent_current.si.get(), vt, ro, i, vs).i_vs;
        let iout = -i;
        Some(Observed::values([iin, iout, vs * iin + vo * iout], 3))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Other
    }
    fn follow_world(&mut self, quantity: WorldQuantity, schedule: &Schedule<Value>) {
        self.output_voltage.follow_world(quantity, schedule);
        self.output_resistance.follow_world(quantity, schedule);
    }
}

#[cfg(test)]
mod tests {
    use orbe_model::Time;

    use super::*;
    use crate::finite::Finite;
    use crate::name::Ref;
    use crate::netlist::{self, Model};
    use crate::run_folder::{Observations, OpPoint};
    use crate::session::{Session, WorldChange};
    use crate::world::Quantity;

    fn op_of(results: &crate::run_folder::Results) -> &OpPoint {
        match results.observations() {
            Observations::Op(op) => op,
            other => panic!("expected Op observations, got {other:?}"),
        }
    }

    fn tmp36_netlist(supply_v: f64, temp_c: f64, load_ohm: Option<f64>) -> String {
        let load = match load_ohm {
            Some(ohm) => format!(
                r#",{{ "name": "rl", "kind": "resistor", "pins": {{ "a": "vout", "b": "gnd" }},
                  "resistance": {{ "value": {ohm}, "unit": "ohm" }},
                  "ratings": {{ "power": null }} }}"#
            ),
            None => String::new(),
        };
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": ["gnd", "vs", "vout"],
              "world": {{ "temperature": {{ "value": {temp_c}, "unit": "degC" }} }},
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vs", "n": "gnd" }},
                  "voltage": {{ "value": {supply_v}, "unit": "V" }} }},
                {{ "name": "u1", "kind": "analog_temp_sensor",
                  "pins": {{ "vs": "vs", "vout": "vout", "gnd": "gnd" }},
                  "output_voltage": {{
                    "kind": "world",
                    "quantity": "temperature",
                    "curve": {{
                      "kind": "linear",
                      "x0": {{ "value": 25.0, "unit": "degC" }},
                      "y0": {{ "value": 0.75, "unit": "V" }},
                      "x1": {{ "value": 125.0, "unit": "degC" }},
                      "y1": {{ "value": 1.75, "unit": "V" }}
                    }}
                  }},
                  "output_resistance": {{
                    "kind": "world",
                    "quantity": "temperature",
                    "curve": {{
                      "kind": "step",
                      "x": {{ "value": 105.0, "unit": "degC" }},
                      "below": {{ "value": 60.0, "unit": "ohm" }},
                      "above": {{ "value": 250.0, "unit": "ohm" }}
                    }}
                  }},
                  "quiescent_current": {{ "value": 5e-5, "unit": "A" }},
                  "ratings": {{
                    "operating_supply": {{
                      "min": {{ "value": 2.7, "unit": "V" }},
                      "max": {{ "value": 5.5, "unit": "V" }}
                    }},
                    "supply_voltage": {{ "value": 7.0, "unit": "V" }}
                  }}
                }}{load}
              ]
            }}"#
        )
    }

    fn sensor() -> AnalogTempSensor {
        let parsed = netlist::parse(tmp36_netlist(5.0, 25.0, None).as_bytes()).unwrap();
        let Model::AnalogTempSensor(sensor) = parsed
            .parts()
            .find(|(_, p)| p.name.as_str() == "u1")
            .unwrap()
            .1
            .model
            .clone()
        else {
            panic!("expected AnalogTempSensor");
        };
        sensor
    }

    #[test]
    fn analytic_derivatives_match_finite_differences() {
        let s = sensor();
        let iq = s.quiescent_current.si.get();
        let eps_v = 1e-7;
        let eps_i = 1e-9;
        for &ro in &[60.0, 250.0] {
            for &vt in &[-0.05, 0.0, 0.1, 0.75, 1.75] {
                for &vs in &[-1.0, -0.1, 0.0, 0.05, 0.1, 0.15, 0.25, 0.75, 2.0, 5.0] {
                    for &i in &[-2e-4, -1e-6, 0.0, 1e-6, 2e-4] {
                        let law = evaluate(iq, vt, ro, i, vs);
                        let dv_plus = evaluate(iq, vt, ro, i, vs + eps_v);
                        let dv_minus = evaluate(iq, vt, ro, i, vs - eps_v);
                        let di_plus = evaluate(iq, vt, ro, i + eps_i, vs);
                        let di_minus = evaluate(iq, vt, ro, i - eps_i, vs);

                        let num_dvo_dvs = (dv_plus.vo - dv_minus.vo) / (2.0 * eps_v);
                        let num_dvo_di = (di_plus.vo - di_minus.vo) / (2.0 * eps_i);
                        let num_divs_dvs = (dv_plus.i_vs - dv_minus.i_vs) / (2.0 * eps_v);
                        let num_divs_di = (di_plus.i_vs - di_minus.i_vs) / (2.0 * eps_i);

                        assert!(
                            (law.dvo_dvs - num_dvo_dvs).abs() < 1e-4,
                            "dvo_dvs mismatch at ro={ro}, vt={vt}, vs={vs}, i={i}: {} vs {num_dvo_dvs}",
                            law.dvo_dvs
                        );
                        assert!(
                            (law.dvo_di - num_dvo_di).abs() < 1e-4,
                            "dvo_di mismatch at ro={ro}, vt={vt}, vs={vs}, i={i}: {} vs {num_dvo_di}",
                            law.dvo_di
                        );
                        assert!(
                            (law.di_vs_dvs - num_divs_dvs).abs() < 1e-5,
                            "di_vs_dvs mismatch at ro={ro}, vt={vt}, vs={vs}, i={i}: {} vs {num_divs_dvs}",
                            law.di_vs_dvs
                        );
                        assert!(
                            (law.di_vs_di - num_divs_di).abs() < 1e-4,
                            "di_vs_di mismatch at ro={ro}, vt={vt}, vs={vs}, i={i}: {} vs {num_divs_di}",
                            law.di_vs_di
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn tmp36_transfer_function_quiescent_and_load_regulation() {
        for (temp, expected_v) in [
            (-40.0, 0.10),
            (0.0, 0.50),
            (25.0, 0.75),
            (40.0, 0.90),
            (105.0, 1.55),
            (125.0, 1.75),
        ] {
            let netlist = netlist::parse(tmp36_netlist(5.0, temp, None).as_bytes()).unwrap();
            let results = crate::simulate(&netlist);
            assert!(
                results.diagnostics().is_empty(),
                "temp={temp}: {:?}",
                results.diagnostics()
            );
            let op = op_of(&results);
            let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
            let iin = op.values[&"#u1.iin".parse::<Ref>().unwrap()].get();
            assert!(
                (vout - expected_v).abs() < 1e-6,
                "temp={temp}: got {vout} V, want {expected_v} V"
            );
            assert!((iin - 50e-6).abs() < 1e-9, "temp={temp}: iin={iin}");
        }

        // Load regulation (-40 to +105 C): 15 kOhm load at 25 C and 105 C sees 60 ohm.
        for (temp, voc) in [(25.0, 0.75), (105.0, 1.55)] {
            let netlist =
                netlist::parse(tmp36_netlist(5.0, temp, Some(15_000.0)).as_bytes()).unwrap();
            let results = crate::simulate(&netlist);
            assert!(results.diagnostics().is_empty());
            let op = op_of(&results);
            let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
            let expected = voc * 15_000.0 / 15_060.0;
            assert!(
                (vout - expected).abs() < 1e-6,
                "temp={temp}: got {vout}, want {expected}"
            );
            let iin = op.values[&"#u1.iin".parse::<Ref>().unwrap()].get();
            let iload = expected / 15_000.0;
            assert!(
                (iin - (50e-6 + iload)).abs() < 1e-7,
                "temp={temp}: iin={iin}, expected {}",
                50e-6 + iload
            );
        }

        // Load regulation (+105 to +125 C): 15 kOhm load at 120 C sees 250 ohm.
        let netlist = netlist::parse(tmp36_netlist(5.0, 120.0, Some(15_000.0)).as_bytes()).unwrap();
        let results = crate::simulate(&netlist);
        assert!(results.diagnostics().is_empty());
        let op = op_of(&results);
        let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
        let expected = 1.70 * 15_000.0 / 15_250.0;
        assert!(
            (vout - expected).abs() < 1e-6,
            "120 C: got {vout}, want {expected}"
        );
    }

    #[test]
    fn unpowered_gives_zero_output_zero_draw_and_no_warning() {
        let netlist = netlist::parse(tmp36_netlist(0.0, 25.0, Some(10_000.0)).as_bytes()).unwrap();
        let results = crate::simulate(&netlist);
        assert!(
            results.diagnostics().is_empty(),
            "unpowered warned: {:?}",
            results.diagnostics()
        );
        let op = op_of(&results);
        let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
        let iin = op.values[&"#u1.iin".parse::<Ref>().unwrap()].get();
        assert!(vout.abs() < 1e-9, "vout={vout}");
        assert!(iin.abs() < 1e-12, "iin={iin}");
    }

    #[test]
    fn supply_boundaries_clamp_and_warn() {
        // Exact operating boundaries 2.7 V and 5.5 V stay quiet.
        for supply in [2.7, 5.0, 5.5] {
            let netlist = netlist::parse(tmp36_netlist(supply, 25.0, None).as_bytes()).unwrap();
            let results = crate::simulate(&netlist);
            assert!(
                results.diagnostics().is_empty(),
                "supply={supply}: {:?}",
                results.diagnostics()
            );
        }

        // Below 2.7 V: keeps working (clamped to VS when needed) and warns rating.voltage.
        let low = netlist::parse(tmp36_netlist(1.0, 100.0, None).as_bytes()).unwrap();
        let results = crate::simulate(&low);
        let op = op_of(&results);
        let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
        // Target at 100 C is 1.50 V, clamped to VS = 1.0 V.
        assert!((vout - 1.0).abs() < 2e-3, "clamped vout={vout}");
        let codes: Vec<_> = results.diagnostics().iter().map(Diagnostic::code).collect();
        assert_eq!(codes, vec!["rating.voltage"]);
        let msg = serde_json::to_value(&results.diagnostics()[0]).unwrap();
        assert!(
            msg["message"]
                .as_str()
                .unwrap()
                .contains("minimum operating supply"),
            "{msg}"
        );

        // Above 5.5 V up to 7.0 V: keeps working and warns maximum operating supply.
        for supply in [5.6, 6.0, 7.0] {
            let over_op = netlist::parse(tmp36_netlist(supply, 25.0, None).as_bytes()).unwrap();
            let results = crate::simulate(&over_op);
            let op = op_of(&results);
            let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
            assert!((vout - 0.75).abs() < 1e-6, "supply={supply} vout={vout}");
            let codes: Vec<_> = results.diagnostics().iter().map(Diagnostic::code).collect();
            assert_eq!(codes, vec!["rating.voltage"], "supply={supply}");
            let msg = serde_json::to_value(&results.diagnostics()[0]).unwrap();
            assert!(
                msg["message"]
                    .as_str()
                    .unwrap()
                    .contains("maximum operating supply"),
                "supply={supply}: {msg}"
            );
        }

        // Above 7.0 V: keeps working and warns past the 7 V absolute maximum ceiling, no -0.3 V.
        let high = netlist::parse(tmp36_netlist(8.0, 25.0, None).as_bytes()).unwrap();
        let results = crate::simulate(&high);
        let op = op_of(&results);
        let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
        assert!((vout - 0.75).abs() < 1e-6, "vout={vout}");
        let codes: Vec<_> = results.diagnostics().iter().map(Diagnostic::code).collect();
        assert_eq!(codes, vec!["rating.voltage"]);
        let msg = serde_json::to_value(&results.diagnostics()[0]).unwrap();
        let text = msg["message"].as_str().unwrap();
        assert!(text.contains("outside its 7 V ceiling"), "{msg}");
        assert!(!text.contains("-0.3"), "{msg}");

        // Reversed supply: 0 V output and no invented floor warning.
        let rev = netlist::parse(tmp36_netlist(-5.0, 25.0, None).as_bytes()).unwrap();
        let results = crate::simulate(&rev);
        let op = op_of(&results);
        let vout = op.values[&"#net.vout".parse::<Ref>().unwrap()].get();
        assert!(vout.abs() < 1e-9, "reversed vout={vout}");
        assert!(
            results.diagnostics().is_empty(),
            "reversed warned: {:?}",
            results.diagnostics()
        );
    }

    #[test]
    fn live_session_world_temperature_moves_vout_and_output_resistance() {
        let json = r#"{
          "version": 1,
          "seed": 0,
          "analysis": { "transient": { "stop_ps": 10000000000, "max_step_ps": 1000000000, "grid_ps": 1000000000 } },
          "ground": "gnd",
          "nets": ["gnd", "vs", "vout"],
          "world": { "temperature": { "value": 25.0, "unit": "degC" } },
          "parts": [
            { "name": "v1", "kind": "voltage_source", "pins": { "p": "vs", "n": "gnd" },
              "voltage": { "value": 5.0, "unit": "V" } },
            { "name": "rl", "kind": "resistor", "pins": { "a": "vout", "b": "gnd" },
              "resistance": { "value": 10000.0, "unit": "ohm" },
              "ratings": { "power": null } },
            { "name": "u1", "kind": "analog_temp_sensor",
              "pins": { "vs": "vs", "vout": "vout", "gnd": "gnd" },
              "output_voltage": {
                "kind": "world",
                "quantity": "temperature",
                "curve": {
                  "kind": "linear",
                  "x0": { "value": 25.0, "unit": "degC" },
                  "y0": { "value": 0.75, "unit": "V" },
                  "x1": { "value": 125.0, "unit": "degC" },
                  "y1": { "value": 1.75, "unit": "V" }
                }
              },
              "output_resistance": {
                "kind": "world",
                "quantity": "temperature",
                "curve": {
                  "kind": "step",
                  "x": { "value": 105.0, "unit": "degC" },
                  "below": { "value": 60.0, "unit": "ohm" },
                  "above": { "value": 250.0, "unit": "ohm" }
                }
              },
              "quiescent_current": { "value": 5e-5, "unit": "A" },
              "ratings": {
                "operating_supply": {
                  "min": { "value": 2.7, "unit": "V" },
                  "max": { "value": 5.5, "unit": "V" }
                },
                "supply_voltage": { "value": 7.0, "unit": "V" }
              }
            }
          ]
        }"#;
        let netlist = netlist::parse(json.as_bytes()).unwrap();
        let mut session = Session::open(netlist).unwrap();
        let vout_ref: Ref = "#net.vout".parse().unwrap();
        session.advance_to(Time::from_ps(1_000_000_000));
        let v_initial = session.probe().get(&vout_ref).unwrap().get();
        let exp_25 = 0.75 * 10_000.0 / 10_060.0;
        assert!(
            (v_initial - exp_25).abs() < 1e-6,
            "v_initial={v_initial}, want {exp_25}"
        );

        session
            .set(&WorldChange {
                quantity: Quantity::Temperature,
                at: Time::ZERO,
                value: Finite::new(40.0).unwrap(),
            })
            .unwrap();
        session.advance_to(Time::from_ps(2_000_000_000));
        let v_warm = session.probe().get(&vout_ref).unwrap().get();
        let exp_40 = 0.90 * 10_000.0 / 10_060.0;
        assert!(
            (v_warm - exp_40).abs() < 1e-6,
            "v_warm={v_warm}, want {exp_40}"
        );
        assert!(session.diagnostics().is_empty());

        // Move above 105 C: output resistance switches from 60 ohm to 250 ohm.
        session
            .set(&WorldChange {
                quantity: Quantity::Temperature,
                at: Time::ZERO,
                value: Finite::new(120.0).unwrap(),
            })
            .unwrap();
        session.advance_to(Time::from_ps(4_000_000_000));
        let v_hot = session.probe().get(&vout_ref).unwrap().get();
        let exp_120 = 1.70 * 10_000.0 / 10_250.0;
        assert!(
            (v_hot - exp_120).abs() < 1e-6,
            "v_hot={v_hot}, want {exp_120}"
        );
        assert!(session.diagnostics().is_empty());
    }
}
