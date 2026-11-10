use crate::netlist::LinearRegulator;

use crate::element::Update;
use crate::mna::{Node, Probe, Stamp};
use crate::newton::{RELTOL, VNTOL};

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

pub(super) struct Law {
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

pub(super) fn evaluate(reg: &LinearRegulator, i: f64, vi: f64) -> Law {
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

    let (h, hp) = step(vi / vdo);
    let ig = (iq + ratio * i) * h;
    Law {
        vo,
        dvo_di,
        dvo_dvi,
        ig,
        dig_di: ratio * h,
        dig_dvi: (iq + ratio * i) * hp / vdo,
    }
}

/// The row is linearised at the last solution. The zero start sits on the reverse-block
/// slope, and a stamp that does not re-read it repeats that row. A linear circuit accepts it.
pub(super) fn stamp(reg: &LinearRegulator, stamp: &mut Stamp<'_>) {
    let [pin_in, pin_out, pin_gnd] = reg.pins;
    let prior = stamp.prior();
    let i = prior.branch(0);
    let vi = prior.v(pin_in) - prior.v(pin_gnd);
    let law = evaluate(reg, i, vi);
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

/// The branch equation is a voltage. The tolerance is the voltage one, so the check does not
/// divide a milliohm slope into the ampere floor.
pub(super) fn update(reg: &LinearRegulator, probe: &Probe<'_>) -> Update {
    let [pin_in, pin_out, pin_gnd] = reg.pins;
    let i = probe.branch(0);
    let vi = probe.v(pin_in) - probe.v(pin_gnd);
    let vo = probe.v(pin_out) - probe.v(pin_gnd);
    let law = evaluate(reg, i, vi);
    let finite = vo.is_finite() && law.vo.is_finite() && law.ig.is_finite();
    let scale = RELTOL * vo.abs().max(law.vo.abs()) + VNTOL;
    let voltage = (vo - law.vo).abs() / scale;
    Update {
        limited: false,
        mismatch: if finite { voltage } else { f64::INFINITY },
    }
}

/// `iin` enters at `in`, `iout` enters at `out`, and `p` is the power entering at every pin.
pub(super) fn observe(reg: &LinearRegulator, probe: &Probe<'_>) -> [f64; 3] {
    let [pin_in, pin_out, pin_gnd] = reg.pins;
    let i = probe.branch(0);
    let vi = probe.v(pin_in) - probe.v(pin_gnd);
    let vo = probe.v(pin_out) - probe.v(pin_gnd);
    let ig = evaluate(reg, i, vi).ig;
    let iin = i + ig;
    let iout = -i;
    [iin, iout, vi * iin + vo * iout]
}

#[cfg(test)]
mod tests {
    use crate::name::Ref;
    use crate::netlist::{self, Model, NetId};

    use crate::mna::{GMIN, Node, Solution, Stamp, System};
    use crate::newton::{Circuit, Conditions, Iterate};
    use crate::op::OP_BUDGET;

    use super::{FLOOR_OHM, evaluate};

    fn fields(ratio: f64) -> String {
        format!(
            r#"
              "output_voltage": {{ "value": 3.3, "unit": "V", "source": "user" }},
              "line_regulation": {{ "value": 9.75609756097561e-05, "unit": "1", "source": "user" }},
              "output_resistance": {{ "value": 0.00125, "unit": "ohm", "source": "user" }},
              "dropout_voltage": {{ "value": 1.0857142857142859, "unit": "V", "source": "user" }},
              "dropout_resistance": {{ "value": 0.14285714285714288, "unit": "ohm", "source": "user" }},
              "current_limit": {{ "value": 1.2, "unit": "A", "source": "user" }},
              "quiescent_current": {{ "value": 0.005, "unit": "A", "source": "user" }},
              "ground_current_ratio": {{ "value": {ratio}, "unit": "1", "source": "user" }},
              "ratings": {{
                "current": {{ "value": 0.8, "unit": "A", "source": "user" }},
                "power": {{ "value": 1.6233766233766234, "unit": "W", "source": "user" }}
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
              "voltage": {{ "value": {volts}, "unit": "V", "source": "user" }}
            }}"#
        )
    }

    fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> String {
        format!(
            r#"{{
              "name": "{name}",
              "kind": "resistor",
              "pins": {{ "a": "{a}", "b": "{b}" }},
              "resistance": {{ "value": {ohm}, "unit": "ohm", "source": "user" }},
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
              "capacitance": {{ "value": 1e-5, "unit": "F", "source": "user" }},
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
              "version": 0,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": [{nets}],
              "parts": [{parts}]
            }}"#,
            parts = parts.join(",")
        )
    }

    fn device(ratio: f64) -> crate::netlist::LinearRegulator {
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

    fn owned(parsed: &crate::netlist::Netlist) -> crate::netlist::LinearRegulator {
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
            None,
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
        crate::element::stamp(&built.netlist.part(id).model, &mut stamp);
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
        crate::element::stamp(&built.netlist.part(id).model, &mut stamp);
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
            crate::element::stamp(&part.model, &mut stamp);
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
                      "current": { "value": 0.5, "unit": "A", "source": "user" }
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
                      "current": { "value": 0.5, "unit": "A", "source": "user" }
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
                      "voltage": { "value": 5.0, "unit": "V", "source": "user" }
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
}
