use crate::diagnostic::ValueRule;
use crate::netlist::NetId;
use crate::netlist::Rating;
use crate::netlist::Value;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireRating;
use crate::netlist::json::WireValue;
use crate::netlist::json::bounded;
use crate::netlist::json::resolve_limit;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_rating;
use crate::netlist::json::resolve_value;
use serde::Deserialize;

use orbe_model::Time;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, Update};
use crate::mna::{Node, Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::ParseCtx;
use crate::netlist::{Conduction, Kind};
use crate::newton::{RELTOL, Side, VNTOL};

/// Dropout knee, volt. The smooth minimum is within a millivolt of the corner.
const KNEE_V: f64 = 1e-3;
/// Ohm. `b` is the smooth maximum of the dropout line and `-FLOOR_OHM * i`.
/// Below the ground pin the softplus of the dropout line underflows, and this
/// resistance keeps the slope. A 10 us trapezoidal run of the load step stops
/// at 0.15 ohm and finishes at 0.2 ohm. 0.25 ohm also finishes a 100 uF output,
/// a 4 V input step and a 1 ohm dropout resistance.
const FLOOR_OHM: f64 = 0.25;
/// Current-limit knee as a fraction of `current_limit`.
const LIMIT_FRACTION: f64 = 1e-3;
/// The short-circuit current stays within about a percent of `current_limit`.
const LIMIT_HEADROOM: f64 = 100.0;
/// Reverse current stays under a nanoamp at a tenth of a volt of back-feed.
const BLOCK_OHM: f64 = 1e9;
const BLOCK_A: f64 = 1e-12;
/// Volt. Pin `in` warns `rating.voltage` below this far under `gnd`. The same margin a chip
/// pin uses below its ground. LP2985 SLVS522 states -0.3 V. The AMS1117 sheet this repo cites
/// states 15 V and no negative floor, and the SGM2212 sheet states 22 V and no negative floor.
pub(crate) const INPUT_FLOOR_V: f64 = 0.3;
/// Fraction of `output_voltage` dropped by the current-limit term before the regulator is treated
/// as actively clamped (0.5%, i.e. 16.5 mV on 3.3 V or 25 mV on 5 V), and also the drop below
/// the pre-overload baseline that confirms a pull-down rather than a charging rail. At 99.5% of
/// `current_limit` the softplus term is 0.07% of `output_voltage` and an ideal 100% nominal load
/// is 0.35%; at 0.5% the pass current is 99.7% of `current_limit`, catching 105% overloads even
/// on regulators whose own `output_resistance` already takes 3.5% of `output_voltage` (LP2985,
/// XC6206).
pub(crate) const LIMIT_SAG_FRACTION: f64 = 0.005;
/// Below this fraction of `output_voltage` the regulator is clear of the current-limit knee, so
/// its unclamped output voltage updates the pre-overload baseline.
pub(crate) const LIMIT_KNEE_FRACTION: f64 = 0.001;
/// Minimum picoseconds a pulled-down clamp must persist before warning, and the minimum
/// continuous clamped hold for an output voltage to count as the lowest held voltage (1 us). Real
/// linear regulators' error-amplifier and current-limit loops and required output bypass
/// capacitors respond on microsecond-to-hundreds-of-microsecond timescales (TI SLVS522S LP2985
/// load transient curves at 20 us/div with 1 A/us steps and >= 2.2 uF Cout; AMS1117 20 us/div with
/// 22 uF Cout; SGM2212 200 us/div with >= 1 uF Cout; Torex XC6206 1 ms/div with 1 uF CL;
/// TI SNOSBT0L LM7805 0.1 uF output bypass; CH340G 0.1 uF V3 bypass), so picosecond to
/// tens-of-nanosecond capacitive switching spikes do not trip or set the reported voltage, while
/// microsecond brownouts (such as the 1.7 us Nano 3V3 NeoPixel reset) and PWM overloads warn.
pub(crate) const LIMIT_HOLD_PS: u64 = 1_000_000;
/// Minimum fractional rate (per second) at which a charging clamped output starting below target
/// must close its remaining current-limit gap `drop = vo_min - vo` (0.5/s, a 2 s RC time
/// constant). Any capacitive load in parallel with a resistor at or under the limit closes `drop`
/// at least as fast as `1 / (R * C)` all the way into regulation (1.93/s even for 4700 uF at 100%
/// load on the 30 mA CH340G), while a rail stalled below target slows exponentially and must
/// warn within at most `ln(1 / LIMIT_SAG_FRACTION) / MIN_CHARGE_RATE` (~10.6 s), and in practice
/// within a few `R * C` time constants or [`STALL_PS`] with no capacitor.
pub(crate) const MIN_CHARGE_RATE: f64 = 0.5;
/// Minimum picoseconds over which a charging clamped rail is checked before declaring it stalled
/// (500 us). At `MIN_CHARGE_RATE` and the smallest clamped `drop` (`LIMIT_SAG_FRACTION * vout`),
/// the required rise is `1.25e-6 * vout`, above Newton's `RELTOL * vo`, while an immediate
/// no-capacitor overload still warns well inside a 1 ms run.
pub(crate) const STALL_PS: u64 = 500_000_000;

pub(crate) struct Law {
    vo: f64,
    dvo_di: f64,
    dvo_dvi: f64,
    ig: f64,
    dig_di: f64,
    dig_dvi: f64,
}

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

/// `h` and `h'` of the ground-current step. `h'` is zero outside `(0, 1)`.
fn step(x: f64) -> (f64, f64) {
    if x <= 0.0 || x >= 1.0 {
        (if x >= 1.0 { 1.0 } else { 0.0 }, 0.0)
    } else {
        (x * x * (3.0 - 2.0 * x), 6.0 * x * (1.0 - x))
    }
}

pub(crate) fn evaluate(reg: &LinearRegulator, i: f64, vi: f64) -> Law {
    let vout = reg.output_voltage.si.get();
    let k = reg.line_regulation.si.get();
    let ro = reg.output_resistance.si.get();
    let vdo = reg.dropout_voltage.si.get();
    let rdo = reg.dropout_resistance.si.get();
    let ilim = reg.current_limit.si.get();
    let iq = reg.quiescent_current.si.get();
    let ratio = reg.ground_current_ratio.si.get();
    let d = LIMIT_FRACTION * ilim;
    let rlim = LIMIT_HEADROOM * vout / ilim;

    let vset = vout + k * (vi - vout);
    let a = vset - ro * i;
    let raw = vi - vdo - rdo * i;
    let floor = -FLOOR_OHM * i;
    let blend = sigmoid((raw - floor) / KNEE_V);
    let b = floor + KNEE_V * softplus((raw - floor) / KNEE_V);
    let share = sigmoid((a - b) / KNEE_V);
    let vo_min = a - KNEE_V * softplus((a - b) / KNEE_V);
    let ds_da = 1.0 - share;
    let ds_db = share;
    let db_di = -FLOOR_OHM + blend * (FLOOR_OHM - rdo);
    let db_dvi = blend;

    let over = (i - ilim) / d;
    let back = -i / BLOCK_A;
    let vo = vo_min - rlim * d * softplus(over) + BLOCK_OHM * BLOCK_A * softplus(back);
    let dvo_di = -ro * ds_da + db_di * ds_db - rlim * sigmoid(over) - BLOCK_OHM * sigmoid(back);
    let dvo_dvi = k * ds_da + db_dvi * ds_db;

    let x = vi / vdo;
    let (h, hp) = step(x);
    // `h(-x)` is 0 for `vi` at least 0, so this term and its derivatives are exactly 0 there.
    // Below 0 it loads the input: `-Iq` at `-Vdo`, and `Iq * vi / Vdo` further below.
    let (hoff, hpoff) = step(-x);
    let ig = (iq + ratio * i) * h + iq * x * hoff;
    Law {
        vo,
        dvo_di,
        dvo_dvi,
        ig,
        dig_di: ratio * h,
        dig_dvi: (iq + ratio * i) * hp / vdo + iq / vdo * hoff - iq * vi / (vdo * vdo) * hpoff,
    }
}

/// `(vo_min, limit_drop)` at pass current `i` and input voltage `vi`: the output voltage the
/// regulating and dropout lines alone ask for, and the drop subtracted by the current-limit clamp.
pub(crate) fn limit_drop(reg: &LinearRegulator, i: f64, vi: f64) -> (f64, f64) {
    let vout = reg.output_voltage.si.get();
    let k = reg.line_regulation.si.get();
    let ro = reg.output_resistance.si.get();
    let vdo = reg.dropout_voltage.si.get();
    let rdo = reg.dropout_resistance.si.get();
    let ilim = reg.current_limit.si.get();
    let d = LIMIT_FRACTION * ilim;
    let rlim = LIMIT_HEADROOM * vout / ilim;

    let vset = vout + k * (vi - vout);
    let a = vset - ro * i;
    let raw = vi - vdo - rdo * i;
    let floor = -FLOOR_OHM * i;
    let b = floor + KNEE_V * softplus((raw - floor) / KNEE_V);
    let vo_min = a - KNEE_V * softplus((a - b) / KNEE_V);
    let drop = rlim * d * softplus((i - ilim) / d);
    (vo_min, drop)
}

#[cfg(test)]
mod tests {
    use crate::netlist::{self, Model, NetId};

    use crate::name::Ref;

    use crate::mna::{GMIN, Node, Solution, Stamp, System};
    use crate::newton::{Circuit, Conditions, Iterate};
    use crate::op::OP_BUDGET;

    use super::{FLOOR_OHM, evaluate};

    fn fields(ratio: f64) -> String {
        format!(
            r#"
              "output_voltage": {{ "value": 3.3, "unit": "V" }},
              "line_regulation": {{ "value": 9.75609756097561e-05, "unit": "1" }},
              "output_resistance": {{ "value": 0.00125, "unit": "ohm" }},
              "dropout_voltage": {{ "value": 1.0857142857142859, "unit": "V" }},
              "dropout_resistance": {{ "value": 0.14285714285714288, "unit": "ohm" }},
              "current_limit": {{ "value": 1.2, "unit": "A" }},
              "quiescent_current": {{ "value": 0.005, "unit": "A" }},
              "ground_current_ratio": {{ "value": {ratio}, "unit": "1" }},
              "ratings": {{
                "current": {{ "value": 0.8, "unit": "A" }},
                "power": {{ "value": 1.6233766233766234, "unit": "W" }}
              }}
            "#
        )
    }

    fn source(name: &str, net: &str, volts: f64) -> String {
        format!(
            r#"{{
              "name": "{name}",
              "kind": "voltage_source",
              "pins": {{ "p": "{net}", "n": "gnd" }},
              "voltage": {{ "value": {volts}, "unit": "V" }}
            }}"#
        )
    }

    fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> String {
        format!(
            r#"{{
              "name": "{name}",
              "kind": "resistor",
              "pins": {{ "a": "{a}", "b": "{b}" }},
              "resistance": {{ "value": {ohm}, "unit": "ohm" }},
              "ratings": {{ "power": null }}
            }}"#
        )
    }

    fn capacitor(name: &str, a: &str, b: &str) -> String {
        format!(
            r#"{{
              "name": "{name}",
              "kind": "capacitor",
              "pins": {{ "a": "{a}", "b": "{b}" }},
              "capacitance": {{ "value": 1e-5, "unit": "F" }},
              "initial": null
            }}"#
        )
    }

    fn regulator(inn: &str, out: &str, gnd: &str, ratio: f64) -> String {
        format!(
            r#"{{
              "name": "u1",
              "kind": "linear_regulator",
              "pins": {{ "in": "{inn}", "out": "{out}", "gnd": "{gnd}" }},
              {fields}
            }}"#,
            fields = fields(ratio)
        )
    }

    fn netlist(nets: &str, parts: &[&str]) -> String {
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": [{nets}],
              "parts": [{parts}]
            }}"#,
            parts = parts.join(",")
        )
    }

    fn device(ratio: f64) -> crate::chip::LinearRegulator {
        let json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                &resistor("rl", "vout", "gnd", 33.0),
                &regulator("vin", "vout", "gnd", ratio),
            ],
        );
        let parsed = netlist::parse(json.as_bytes()).unwrap();
        owned(&parsed)
    }

    fn owned(parsed: &crate::netlist::Netlist) -> crate::chip::LinearRegulator {
        let Model::LinearRegulator(reg) = parsed
            .parts()
            .find(|(_, part)| matches!(part.model, Model::LinearRegulator(_)))
            .unwrap()
            .1
            .model
            .clone()
        else {
            panic!("not a regulator");
        };
        reg
    }

    fn regulator_id(circuit: &Circuit) -> crate::netlist::PartId {
        circuit
            .netlist
            .parts()
            .find(|(_, part)| matches!(part.model, Model::LinearRegulator(_)))
            .unwrap()
            .0
    }

    fn circuit(json: &str) -> Circuit {
        Circuit::new(netlist::parse(json.as_bytes()).unwrap()).unwrap()
    }

    fn regulating() -> Circuit {
        circuit(&netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                &resistor("rl", "vout", "gnd", 33.0),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        ))
    }

    fn solve(circuit: &Circuit) -> (crate::newton::Iterate, u32) {
        let converged = crate::newton::newton(
            circuit,
            &mut crate::mna::Solver::new(&circuit.layout),
            &Conditions::NOMINAL,
            Iterate::initial(circuit),
            OP_BUDGET,
        )
        .expect("plain Newton");
        (converged.iterate, converged.iterations)
    }

    fn net(circuit: &Circuit, name: &str) -> NetId {
        circuit
            .netlist
            .nets()
            .find(|(_, net)| net.as_str() == name)
            .map(|(id, _)| id)
            .unwrap()
    }

    fn voltage(circuit: &Circuit, solution: &Solution, name: &str) -> f64 {
        solution.node(net(circuit, name).into())
    }

    fn near(got: f64, want: f64, tol: f64) -> bool {
        (got - want).abs() <= tol * want.abs() + 1e-6
    }

    #[test]
    fn regulator_law_partials_match_a_difference() {
        let reg = device(0.05);
        let vdo = reg.dropout_voltage.si.get();
        let ro = reg.output_resistance.si.get();
        let rdo = reg.dropout_resistance.si.get();
        let k = reg.line_regulation.si.get();
        let knee = |i: f64| {
            let vout = reg.output_voltage.si.get();
            vout + (vdo + i * (rdo - ro)) / (1.0 - k)
        };
        let points = [
            (0.01, 5.0),
            (0.1, 5.0),
            (0.8, 5.0),
            (0.4, 15.0),
            (0.05, 2.5),
            (0.2, 3.0),
            (0.5, 3.5),
            (0.8, 4.0),
            (0.05, knee(0.05)),
            (0.2, knee(0.2)),
            (0.5, knee(0.5)),
            (0.8, knee(0.8)),
            (1.2, 5.0),
            (1.3, 5.0),
            (1.5, 8.0),
            (1.25, 12.0),
            (-1e-6, 5.0),
            (-1e-7, 5.0),
            (-1e-8, 8.0),
            (-1e-9, 8.0),
            (0.0, -1.0),
            (0.01, 0.0),
            (0.01, 0.5 * vdo),
            (0.01, vdo),
            (1.21, 5.0),
            (0.0, 5.0),
            (1e-4, 4.75),
            (0.8, 4.75),
            (1e-12, 5.0),
            (-1e-12, 5.0),
        ];
        assert_eq!(points.len(), 30);
        for (n, (i, vi)) in points.iter().copied().enumerate() {
            let law = evaluate(&reg, i, vi);
            let di = if i == 0.0 {
                1e-15
            } else if i.abs() < 1e-4 {
                (1e-3 * i.abs()).max(1e-15)
            } else {
                1e-7
            };
            let gi = 1e-6;
            let dvi = 1e-6;
            let d_i = (evaluate(&reg, i + di, vi).vo - evaluate(&reg, i - di, vi).vo) / (2.0 * di);
            let d_vi =
                (evaluate(&reg, i, vi + dvi).vo - evaluate(&reg, i, vi - dvi).vo) / (2.0 * dvi);
            let g_i = (evaluate(&reg, i + gi, vi).ig - evaluate(&reg, i - gi, vi).ig) / (2.0 * gi);
            let g_vi =
                (evaluate(&reg, i, vi + dvi).ig - evaluate(&reg, i, vi - dvi).ig) / (2.0 * dvi);
            assert!(law.dvo_di < 0.0, "point {n} slope {}", law.dvo_di);
            if i == 0.01 && vi == 0.0 {
                assert!(
                    law.dvo_di <= -FLOOR_OHM,
                    "point {n} floor slope {}",
                    law.dvo_di
                );
            }
            assert!(
                near(d_i, law.dvo_di, 1e-4),
                "point {n} dvo/di {d_i} vs {}",
                law.dvo_di
            );
            assert!(
                near(d_vi, law.dvo_dvi, 1e-4),
                "point {n} dvo/dvi {d_vi} vs {}",
                law.dvo_dvi
            );
            assert!(
                near(g_i, law.dig_di, 1e-4),
                "point {n} dig/di {g_i} vs {}",
                law.dig_di
            );
            assert!(
                near(g_vi, law.dig_dvi, 1e-4),
                "point {n} dig/dvi {g_vi} vs {}",
                law.dig_dvi
            );
        }
    }

    #[test]
    fn regulator_regions_are_continuous() {
        let reg = device(0.0);
        let vdo = reg.dropout_voltage.si.get();
        let ro = reg.output_resistance.si.get();
        let rdo = reg.dropout_resistance.si.get();
        let k = reg.line_regulation.si.get();
        let vout = reg.output_voltage.si.get();
        let ilim = reg.current_limit.si.get();
        let i = 0.2;
        let knee = vout + (vdo + i * (rdo - ro)) / (1.0 - k);
        let floor_i = 0.01;
        let floor_vi = vdo + rdo * floor_i;
        let samples = [
            (i, knee, 1e-6),
            (ilim, 5.0, 1e-6),
            (0.0, 5.0, 1e-15),
            (floor_i, floor_vi, 1e-6),
        ];
        for (i, vi, di) in samples {
            let law = evaluate(&reg, i, vi);
            let left = evaluate(&reg, i - di, vi).vo;
            let right = evaluate(&reg, i + di, vi).vo;
            let central = (right - left) / (2.0 * di);
            assert!(
                (law.vo - 0.5 * (left + right)).abs() < 1e-6,
                "value jumps at {i} {vi}"
            );
            assert!(
                near(central, law.dvo_di, 1e-3),
                "slope {central} vs {} at {i} {vi}",
                law.dvo_di
            );
        }
    }

    #[test]
    fn the_branch_row_carries_line_regulation() {
        let built = regulating();
        let id = regulator_id(&built);
        let Model::LinearRegulator(reg) = &built.netlist.part(id).model else {
            panic!("not a regulator");
        };
        let mut iterate = Iterate::initial(&built);
        let mut x = vec![0.0; built.layout.size()];
        let Node::Row(in_row) = Node::from(reg.pins[0]) else {
            panic!("in is ground");
        };
        let Node::Row(out_row) = Node::from(reg.pins[1]) else {
            panic!("out is ground");
        };
        let branch = built.layout.branch_row(id, 0);
        x[in_row] = 5.0;
        x[out_row] = 3.3;
        x[branch] = 0.1;
        iterate.solution = Solution::from(x);
        let mut system = System::new(&built.layout);
        system.reset(&built.layout, GMIN);
        let conditions = Conditions::NOMINAL;
        let mut stamp = Stamp::new(&mut system, &built.layout, id, &iterate, &conditions);
        crate::element::Device::stamp(&built.netlist.part(id).model, &mut stamp);
        let law = evaluate(reg, 0.1, 5.0);
        let got = system.term(branch, in_row);
        assert!(law.dvo_dvi.abs() > 1e-6, "line regulation dropped");
        assert!(
            (got - (-law.dvo_dvi)).abs() <= 1e-12,
            "column {got} is not {}",
            -law.dvo_dvi
        );
    }

    #[test]
    fn the_ground_current_column_follows_the_pass_current() {
        let built = circuit(&netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                &resistor("rl", "vout", "gnd", 6.6),
                &regulator("vin", "vout", "gnd", 0.02),
            ],
        ));
        let id = regulator_id(&built);
        let Model::LinearRegulator(reg) = &built.netlist.part(id).model else {
            panic!("not a regulator");
        };
        let mut iterate = Iterate::initial(&built);
        let mut x = vec![0.0; built.layout.size()];
        let Node::Row(in_row) = Node::from(reg.pins[0]) else {
            panic!("in is ground");
        };
        let branch = built.layout.branch_row(id, 0);
        x[in_row] = 5.0;
        x[branch] = 0.5;
        iterate.solution = Solution::from(x);
        let mut system = System::new(&built.layout);
        system.reset(&built.layout, GMIN);
        let conditions = Conditions::NOMINAL;
        let mut stamp = Stamp::new(&mut system, &built.layout, id, &iterate, &conditions);
        crate::element::Device::stamp(&built.netlist.part(id).model, &mut stamp);
        let law = evaluate(reg, 0.5, 5.0);
        let got = system.term(in_row, branch);
        assert!(law.dig_di > 0.01, "ratio dropped {}", law.dig_di);
        assert!(
            (got - (1.0 + law.dig_di)).abs() <= 1e-12,
            "column {got} is not {}",
            1.0 + law.dig_di
        );
    }

    #[test]
    fn the_first_stamp_is_not_the_regulating_point() {
        let built = regulating();
        let iterate = Iterate::initial(&built);
        let mut system = System::new(&built.layout);
        system.reset(&built.layout, GMIN);
        let conditions = Conditions::NOMINAL;
        for (id, part) in built.netlist.parts() {
            let mut stamp = Stamp::new(&mut system, &built.layout, id, &iterate, &conditions);
            crate::element::Device::stamp(&part.model, &mut stamp);
        }
        let once = system.solve().unwrap();
        let first = voltage(&built, &once, "vout");
        let (solved, iterations) = solve(&built);
        let got = voltage(&built, &solved.solution, "vout");
        assert!(iterations > 1, "{iterations} iterations");
        assert!((got - first).abs() > 1.0, "first {first} solved {got}");
        assert!((got - 3.3).abs() < 1e-3, "{got}");
    }

    fn op_value(json: &str, reference: &str) -> f64 {
        let results = crate::simulate(&netlist::parse(json.as_bytes()).unwrap());
        assert!(
            results
                .diagnostics()
                .iter()
                .all(|d| d.code() != "solver.convergence" && d.code() != "solver.singular"),
            "{:?}",
            results.diagnostics()
        );
        let op = results.observations().op().unwrap();
        op.values[&reference.parse::<Ref>().unwrap()].get()
    }

    #[test]
    fn regulator_does_not_sink() {
        let json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                &source("v2", "vout", 3.4),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let iout = op_value(&json, "#u1.iout");
        let iin = op_value(&json, "#u1.iin");
        assert!(iout.abs() < 1e-9, "{iout}");
        assert!((iin - 0.005).abs() < 1e-6, "{iin}");
    }

    #[test]
    fn a_capacitor_only_rail_is_not_floating() {
        let json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &capacitor("cin", "vin", "gnd"),
                &capacitor("cout", "vout", "gnd"),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let results = crate::simulate(&netlist::parse(json.as_bytes()).unwrap());
        assert!(
            results.observations().op().is_some(),
            "{:?}",
            results.diagnostics()
        );
        let floating: Vec<_> = results
            .diagnostics()
            .iter()
            .filter(|d| d.code() == "net.floating")
            .map(|d| serde_json::to_string(d).unwrap())
            .collect();
        assert!(floating.is_empty(), "{floating:?}");
    }

    #[test]
    fn a_sinking_load_on_an_off_input_follows_the_floor() {
        let json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 0.0),
                r#"{
                      "name": "i1",
                      "kind": "current_source",
                      "pins": { "p": "gnd", "n": "vout" },
                      "current": { "value": 0.5, "unit": "A" }
                    }"#,
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let vout = op_value(&json, "#net.vout");
        assert!((vout - (-0.125)).abs() < 1e-9, "{vout}");
    }

    #[test]
    fn an_off_input_does_not_drive_a_capacitor_below_ground() {
        let json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 0.0),
                &capacitor("cout", "vout", "gnd"),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let vout = op_value(&json, "#net.vout");
        assert!(vout >= 0.0, "{vout}");
        assert!(vout < 1e-3, "{vout}");
    }

    #[test]
    fn ground_current_adds_the_ratio_times_the_load() {
        let json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                r#"{
                      "name": "i1",
                      "kind": "current_source",
                      "pins": { "p": "gnd", "n": "vout" },
                      "current": { "value": 0.5, "unit": "A" }
                    }"#,
                &regulator("vin", "vout", "gnd", 0.02),
            ],
        );
        let iin = op_value(&json, "#u1.iin");
        let iout = op_value(&json, "#u1.iout");
        let ground = iin + iout;
        let expect = 0.005 + 0.02 * iout.abs();
        assert!((ground - expect).abs() <= 1e-9, "{ground} vs {expect}");
    }

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/circuit/fixtures")
            .join(name)
            .join("netlist.json");
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn regulator_fixtures_report_their_newton_iterations() {
        let names = [
            "regulator-open",
            "regulator-short",
            "regulator-backfeed",
            "regulator-off",
            "regulator-off-cap",
            "regulator-ground",
            "regulator-sag",
            "regulator-limit",
            "regulator-hot",
        ];
        for name in names {
            let built = circuit(&fixture(name));
            let (solved, iterations) = solve(&built);
            let vout = voltage(&built, &solved.solution, "vout");
            eprintln!("{name} iterations {iterations} vout {vout}");
            assert!(iterations < OP_BUDGET, "{name} {iterations}");
            if name == "regulator-off-cap" {
                assert!(vout >= 0.0, "{name} {vout}");
                assert!(vout < 1e-3, "{name} {vout}");
            }
        }
    }

    #[test]
    fn an_elevated_ground_pin_is_the_reference() {
        let json = netlist(
            r#""gnd", "ref", "vin", "vout""#,
            &[
                &source("vref", "ref", 1.0),
                r#"{
                      "name": "vin_src",
                      "kind": "voltage_source",
                      "pins": { "p": "vin", "n": "ref" },
                      "voltage": { "value": 5.0, "unit": "V" }
                    }"#,
                &resistor("rl", "vout", "ref", 33.0),
                &regulator("vin", "vout", "ref", 0.0),
            ],
        );
        let out = op_value(&json, "#net.vout");
        let reference = op_value(&json, "#net.ref");
        assert!(
            (out - reference - 3.3).abs() < 1e-3,
            "{out} against {reference}"
        );
    }

    #[test]
    fn the_current_rating_names_iout_and_power_names_p() {
        let limited_json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                &resistor("rl", "vout", "gnd", 1.0),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let hot_json = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 12.0),
                &resistor("rl", "vout", "gnd", 6.6),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let text = |json: &str| {
            let results = crate::simulate(&netlist::parse(json.as_bytes()).unwrap());
            results
                .diagnostics()
                .iter()
                .map(|d| serde_json::to_string(d).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let limited = text(&limited_json);
        assert!(limited.contains("rating.current"), "{limited}");
        assert!(limited.contains("#u1.iout"), "{limited}");
        assert!(!limited.contains("#u1.iin"), "{limited}");
        let hot = text(&hot_json);
        assert!(hot.contains("rating.power"), "{hot}");
        assert!(hot.contains("#u1.p"), "{hot}");
        assert!(!hot.contains("rating.current"), "{hot}");
        let p = op_value(&hot_json, "#u1.p");
        let iin = op_value(&hot_json, "#u1.iin");
        let iout = op_value(&hot_json, "#u1.iout");
        let vin = op_value(&hot_json, "#net.vin");
        let vout = op_value(&hot_json, "#net.vout");
        assert!((p - (vin * iin + vout * iout)).abs() <= 1e-12, "{p}");
    }

    #[test]
    fn ground_current_loads_a_reversed_input_and_stays_put_while_the_input_is_up() {
        let reg = device(0.0);
        let iq = reg.quiescent_current.si.get();
        let vdo = reg.dropout_voltage.si.get();
        let on = evaluate(&reg, 0.0, 5.0);
        assert!((on.ig - iq).abs() < 1e-15, "{}", on.ig);
        let loaded = evaluate(&reg, 0.2, 5.0);
        assert!((loaded.ig - iq).abs() < 1e-15, "{}", loaded.ig);
        let at_zero = evaluate(&reg, 0.01, 0.0);
        assert_eq!(at_zero.ig, 0.0);
        assert_eq!(at_zero.dig_dvi, 0.0);
        let reverse = evaluate(&reg, 0.0, -vdo);
        assert!((reverse.ig + iq).abs() < 1e-15, "{}", reverse.ig);
        assert!(
            (reverse.dig_dvi - iq / vdo).abs() < 1e-12,
            "{}",
            reverse.dig_dvi
        );
        let deep = evaluate(&reg, 0.0, -2.0 * vdo);
        assert!((deep.ig + 2.0 * iq).abs() < 1e-15, "{}", deep.ig);
    }

    fn diode(anode: &str, cathode: &str) -> String {
        format!(
            r#"{{
              "name": "d1",
              "kind": "diode",
              "pins": {{ "anode": "{anode}", "cathode": "{cathode}" }},
              "saturation_current": {{ "value": 9.4334e-08, "unit": "A" }},
              "emission_coefficient": {{ "value": 2.0844, "unit": "1" }},
              "series_resistance": {{ "value": 0.0317, "unit": "ohm" }},
              "ratings": {{ "current": {{ "value": 1.0, "unit": "A" }}, "power": null }}
            }}"#
        )
    }

    fn diagnostics_of(json: &str) -> String {
        let results = crate::simulate(&netlist::parse(json.as_bytes()).unwrap());
        results
            .diagnostics()
            .iter()
            .map(|d| serde_json::to_string(d).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_reversed_input_warns_and_a_series_diode_holds_it_near_zero() {
        let reversed = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", -9.0),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let text = diagnostics_of(&reversed);
        assert!(text.contains("rating.voltage"), "{text}");
        assert!(text.contains("#u1.pin.in"), "{text}");
        assert!(text.contains("-0.3"), "{text}");
        assert!((op_value(&reversed, "#net.vin") + 9.0).abs() < 1e-9);

        let inside = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", -0.2),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        assert!(
            !diagnostics_of(&inside).contains("rating.voltage"),
            "{}",
            diagnostics_of(&inside)
        );

        let protected = netlist(
            r#""gnd", "sleeve", "vin", "vout""#,
            &[
                r#"{
                  "name": "v1",
                  "kind": "voltage_source",
                  "pins": { "p": "gnd", "n": "sleeve" },
                  "voltage": { "value": 9.0, "unit": "V" }
                }"#,
                &diode("sleeve", "vin"),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let vin = op_value(&protected, "#net.vin");
        assert!(vin.abs() < 0.05, "{vin}");
        assert!(
            !diagnostics_of(&protected).contains("rating.voltage"),
            "{}",
            diagnostics_of(&protected)
        );
    }

    #[test]
    fn current_limit_warns_only_when_clamped_and_not_in_dropout_or_under_limit() {
        let just_under = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                // 3.3 V / 1.18 A = 2.797 ohm: under the 1.2 A current limit.
                &resistor("rl", "vout", "gnd", 2.8),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        assert!(
            !diagnostics_of(&just_under).contains("regulator.current_limit"),
            "{}",
            diagnostics_of(&just_under)
        );

        let dropout = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 3.5),
                &resistor("rl", "vout", "gnd", 33.0),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        assert!(
            !diagnostics_of(&dropout).contains("regulator.current_limit"),
            "{}",
            diagnostics_of(&dropout)
        );

        let shorted = netlist(
            r#""gnd", "vin", "vout""#,
            &[
                &source("v1", "vin", 5.0),
                &resistor("rl", "vout", "gnd", 0.5),
                &regulator("vin", "vout", "gnd", 0.0),
            ],
        );
        let text = diagnostics_of(&shorted);
        assert!(text.contains("regulator.current_limit"), "{text}");
        assert!(text.contains("#u1.pin.out"), "{text}");
        assert!(text.contains("can give 1.2 A at 3.3 V"), "{text}");
    }

    #[test]
    fn brief_power_on_capacitor_charge_stays_quiet_while_sustained_overload_warns_once() {
        let step_source = r#"{
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": {
            "kind": "step",
            "initial": { "value": 0.0, "unit": "V" },
            "at_ps": 1000,
            "final": { "value": 5.0, "unit": "V" }
          }
        }"#;
        let tran = |parts: &[&str]| {
            format!(
                r#"{{
                  "version": 1,
                  "seed": 0,
                  "analysis": {{ "transient": {{ "stop_ps": 5000000000, "max_step_ps": 100000000, "grid_ps": 100000000 }} }},
                  "ground": "gnd",
                  "nets": ["gnd", "vin", "vout"],
                  "parts": [{}]
                }}"#,
                parts.join(",")
            )
        };
        // 10 uF charges monotonically to 3.3 V at 1.2 A in ~27.5 us and must stay quiet.
        let inrush = tran(&[
            step_source,
            &capacitor("c1", "vout", "gnd"),
            &resistor("rl", "vout", "gnd", 330.0),
            &regulator("vin", "vout", "gnd", 0.0),
        ]);
        let results = crate::simulate(&netlist::parse(inrush.as_bytes()).unwrap());
        let limits: Vec<_> = results
            .diagnostics()
            .iter()
            .filter(|d| d.code() == "regulator.current_limit")
            .collect();
        assert!(limits.is_empty(), "{limits:?}");

        // With a 0.5 ohm short on the output, the rail stalls near 0.6 V and emits one warning.
        let shorted = tran(&[
            step_source,
            &capacitor("c1", "vout", "gnd"),
            &resistor("rl", "vout", "gnd", 0.5),
            &regulator("vin", "vout", "gnd", 0.0),
        ]);
        let results = crate::simulate(&netlist::parse(shorted.as_bytes()).unwrap());
        let limits: Vec<_> = results
            .diagnostics()
            .iter()
            .filter(|d| d.code() == "regulator.current_limit")
            .collect();
        assert_eq!(limits.len(), 1, "{limits:?}");
    }
}

/// A static linear regulator. The pass branch's voltage falls with its own current.
/// `in` feeds the pass element and the ground current. `out` is the regulated pin.
/// `gnd` is the reference for both voltages and the return for the ground current.
///
/// `vset = output_voltage + line_regulation * (V(in) - V(gnd) - output_voltage)`.
/// The regulating line is `vset - output_resistance * i`. The dropout line is
/// `V(in) - V(gnd) - dropout_voltage - dropout_resistance * i`. The pass voltage
/// uses the smooth maximum of that line and `-Rf * i`, where `Rf` is a constant
/// of the law, then the smooth minimum against the regulating line, a current
/// limit and a reverse block. At no load the floor is the ground pin. A load
/// that draws current with the input at the pin pulls the output below it.
/// `i` is the current in the pass branch, from `in` to `out`. The ground current is
/// `(quiescent_current + ground_current_ratio * i)` times a smooth step of
/// `vi / dropout_voltage`, plus `quiescent_current * vi / dropout_voltage` times a
/// smooth step of `-vi / dropout_voltage`. The second term is 0 while `vi` is at
/// least 0, and it loads a reversed input. The constants of that law are in the
/// engine, not fields. Pin `in` warns `rating.voltage` below -0.3 V over `gnd`.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearRegulator {
    /// Nets of pins `in`, `out`, `gnd`.
    pub pins: [NetId; 3],
    /// Volt. The output at the line-regulation reference input, at no load.
    pub output_voltage: Value,
    /// Unit `1`. The change in the set voltage per volt of input, at least 0 and below 1.
    pub line_regulation: Value,
    /// Ohm. The fall of the regulating line per ampere.
    pub output_resistance: Value,
    /// Volt. The dropout line's intercept.
    pub dropout_voltage: Value,
    /// Ohm. The fall of the dropout line per ampere.
    pub dropout_resistance: Value,
    /// Ampere. The pass current the limit holds.
    pub current_limit: Value,
    /// Ampere. The ground current at no load, once the input is above `dropout_voltage`.
    pub quiescent_current: Value,
    /// Unit `1`. The extra ground current per ampere of pass current. Zero is a constant ground current.
    pub ground_current_ratio: Value,
    /// Ampere, greater than 0. Bounds `|iout|`. `None` is an explicit `"current": null`.
    pub max_current: Option<Rating>,
    /// Watt, greater than 0. `None` is an explicit `"power": null`.
    pub max_power: Option<Rating>,
    /// Volt, greater than 0. `V(in) - V(gnd)` above this warns `rating.voltage`. `None` when
    /// the part rates none.
    pub max_input: Option<Value>,
}

impl LinearRegulator {
    pub const PINS: [&'static str; 3] = ["in", "out", "gnd"];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireLinearRegulator {
    pub(crate) name: String,
    pins: PinMap,
    output_voltage: WireValue,
    line_regulation: WireValue,
    output_resistance: WireValue,
    dropout_voltage: WireValue,
    dropout_resistance: WireValue,
    current_limit: WireValue,
    quiescent_current: WireValue,
    ground_current_ratio: WireValue,
    ratings: WireRegulatorRatings,
}

/// `current` and `power` are required, and `null` leaves either out. `input_voltage` is
/// optional.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRegulatorRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    current: Option<WireRating>,
    #[serde(deserialize_with = "Option::deserialize")]
    power: Option<WireRating>,
    #[serde(default)]
    input_voltage: Option<WireValue>,
}

pub(crate) fn resolve_regulator(
    ctx: ParseCtx<'_>,
    wire: WireLinearRegulator,
) -> Option<LinearRegulator> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, LinearRegulator::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let output_voltage = positive("output_voltage", "V", wire.output_voltage);
    let output_resistance = positive("output_resistance", "ohm", wire.output_resistance);
    let dropout_voltage = positive("dropout_voltage", "V", wire.dropout_voltage);
    let dropout_resistance = positive("dropout_resistance", "ohm", wire.dropout_resistance);
    let current_limit = positive("current_limit", "A", wire.current_limit);
    let line_regulation = bounded(
        part_ref,
        "line_regulation",
        "1",
        wire.line_regulation,
        |number| (number < 0.0 || number >= 1.0).then_some(ValueRule::BelowOne),
        out,
    );
    let quiescent_current = bounded(
        part_ref,
        "quiescent_current",
        "A",
        wire.quiescent_current,
        |number| (number < 0.0).then_some(ValueRule::NonNegative),
        out,
    );
    let ground_current_ratio = bounded(
        part_ref,
        "ground_current_ratio",
        "1",
        wire.ground_current_ratio,
        |number| (number < 0.0).then_some(ValueRule::NonNegative),
        out,
    );
    let ratings = wire.ratings;
    let max_current = resolve_rating(part_ref, "ratings.current", "A", ratings.current, out);
    let max_power = resolve_rating(part_ref, "ratings.power", "W", ratings.power, out);
    let max_input = resolve_limit(
        part_ref,
        "ratings.input_voltage",
        "V",
        ratings.input_voltage,
        out,
    );
    Some(LinearRegulator {
        pins: pins?,
        output_voltage: output_voltage?,
        line_regulation: line_regulation?,
        output_resistance: output_resistance?,
        dropout_voltage: dropout_voltage?,
        dropout_resistance: dropout_resistance?,
        current_limit: current_limit?,
        quiescent_current: quiescent_current?,
        ground_current_ratio: ground_current_ratio?,
        max_current: max_current?,
        max_power: max_power?,
        max_input: max_input?,
    })
}

impl Kind for LinearRegulator {
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
    /// The row is linearised at the last solution. The zero start sits on the reverse-block
    /// slope, and a stamp that does not re-read it repeats that row. A linear circuit accepts it.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let [pin_in, pin_out, pin_gnd] = self.pins;
        let prior = stamp.prior();
        let i = prior.branch(0);
        let vi = prior.v(pin_in) - prior.v(pin_gnd);
        let law = evaluate(self, i, vi);
        let row = Node::Row(stamp.layout.branch_row(stamp.part, 0));
        stamp.branch_current(pin_in, pin_out, 0);
        stamp.branch_row(
            pin_out,
            pin_gnd,
            0,
            -law.dvo_di,
            1.0,
            law.vo - law.dvo_di * i - law.dvo_dvi * vi,
        );
        stamp.add(row, pin_in, -law.dvo_dvi);
        stamp.add(row, pin_gnd, law.dvo_dvi);

        stamp.current(pin_in, pin_gnd, law.ig - law.dig_di * i - law.dig_dvi * vi);
        stamp.conductance(pin_in, pin_gnd, law.dig_dvi);
        stamp.add(pin_in, row, law.dig_di);
        stamp.add(pin_gnd, row, -law.dig_di);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, _out: &mut [f64]) {}
    /// The branch equation is a voltage. The tolerance is the voltage one, so the check does not
    /// divide a milliohm slope into the ampere floor.
    fn update(&self, probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        let [pin_in, pin_out, pin_gnd] = self.pins;
        let i = probe.branch(0);
        let vi = probe.v(pin_in) - probe.v(pin_gnd);
        let vo = probe.v(pin_out) - probe.v(pin_gnd);
        let law = evaluate(self, i, vi);
        let finite = vo.is_finite() && law.vo.is_finite() && law.ig.is_finite();
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
        let current = self.max_current.as_ref().map(|v| (Quantity::Iout, v));
        (current, self.max_power.as_ref())
    }
    /// `iin` enters at `in`, `iout` enters at `out`, and `p` is the power entering at every pin.
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        let [pin_in, pin_out, pin_gnd] = self.pins;
        let i = probe.branch(0);
        let vi = probe.v(pin_in) - probe.v(pin_gnd);
        let vo = probe.v(pin_out) - probe.v(pin_gnd);
        let ig = evaluate(self, i, vi).ig;
        let iin = i + ig;
        let iout = -i;
        Some(Observed::values([iin, iout, vi * iin + vo * iout], 3))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Fixes
    }
}
