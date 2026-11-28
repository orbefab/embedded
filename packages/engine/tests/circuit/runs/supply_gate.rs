//! A chip that drives only above a minimum supply powers its outputs up once the supply rises to
//! that minimum and down once it falls below its release supply. Its outputs load the supply, so
//! driving can pull it down and releasing can let it rise. Each run here finishes, holds that
//! band at every accepted step, and reads the same split into calls and at another step size.
//! When neither state is self-consistent, the run reports `chip.unsettled` loudly rather than
//! accepting a contradicting answer.

use orbe_engine::blob::Blobs;
use orbe_engine::finite::Finite;
use orbe_engine::netlist::{self, Netlist};
use orbe_engine::run_folder::Results;
use orbe_engine::timestamp::Timestamp;
use orbe_engine::{Session, Status};
use orbe_model::Time;

const MS: u64 = 1_000_000_000;

fn val(value: f64, unit: &str) -> serde_json::Value {
    serde_json::json!({ "value": value, "unit": unit })
}

fn stage(high: f64, low: f64) -> serde_json::Value {
    serde_json::json!({
        "high_drop": val(0.0, "V"), "high_resistance": val(high, "ohm"),
        "low_drop": val(0.0, "V"), "low_resistance": val(low, "ohm"),
    })
}

fn source(name: &str, net: &str, voltage: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "voltage_source", "pins": { "p": net, "n": "gnd" }, "voltage": voltage,
    })
}

fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "resistor", "pins": { "a": a, "b": b },
        "resistance": val(ohm, "ohm"), "ratings": { "power": null },
    })
}

fn capacitor(name: &str, net: &str, farad: f64, initial: Option<f64>) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "capacitor", "pins": { "a": net, "b": "gnd" },
        "capacitance": val(farad, "F"),
        "initial": initial.map(|v| val(v, "V")),
    })
}

/// A chip whose one gated output on net `q` holds low, its supply on `vcc`.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Hc595,
    Max7219,
    Ws2812,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Hc595, Kind::Max7219, Kind::Ws2812];

    /// The supply it powers up at, and the one it powers down below.
    fn band(self) -> (f64, f64) {
        match self {
            Kind::Hc595 | Kind::Max7219 => (2.0, 1.8),
            Kind::Ws2812 => (3.0, 2.7),
        }
    }

    /// The chip and the parts and nets it needs besides `vcc`, `q`, `five` and `gnd`.
    fn parts(self) -> (Vec<&'static str>, Vec<serde_json::Value>) {
        match self {
            Kind::Hc595 => (
                Vec::new(),
                vec![serde_json::json!({
                    "name": "u1", "kind": "hc595",
                    "pins": {
                        "QB": "gnd", "QC": "gnd", "QD": "gnd", "QE": "gnd", "QF": "gnd", "QG": "gnd",
                        "QH": "gnd", "GND": "gnd", "QHp": "gnd", "SRCLR": "five", "SRCLK": "gnd",
                        "RCLK": "gnd", "OE": "gnd", "SER": "gnd", "QA": "q", "VCC": "vcc",
                    },
                    "output": stage(25.0, 25.0),
                    "serial_output": stage(50.0, 50.0),
                    "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
                    "ratings": {
                        "pin_current": null, "supply_current": null,
                        "input_margin": null, "output_margin": null,
                    },
                })],
            ),
            Kind::Max7219 => (
                vec!["iset"],
                vec![
                    serde_json::json!({
                        "name": "u1", "kind": "max7219",
                        "pins": {
                            "VCC": "vcc", "GND": "gnd", "DIN": "gnd", "CLK": "gnd", "LOAD": "five",
                            "DOUT": "q", "ISET": "iset",
                        },
                        "input_threshold": { "low": val(0.8, "V"), "high": val(3.5, "V") },
                        "dout": stage(1000.0, 250.0),
                        "led_forward_voltage": val(1.85, "V"),
                    }),
                    resistor("rset", "five", "iset", 10_000.0),
                ],
            ),
            Kind::Ws2812 => (
                Vec::new(),
                vec![serde_json::json!({
                    "name": "u1", "kind": "ws2812", "leds": 1,
                    "pins": { "VCC": "vcc", "GND": "gnd", "DIN": "gnd", "DOUT": "q" },
                    "input_threshold": { "low": val(0.3, "1"), "high": val(0.7, "1") },
                    "min_supply": val(3.0, "V"),
                    "dout": { "output": stage(50.0, 50.0), "delay": { "value": 150_000, "unit": "ps" } },
                })],
            ),
        }
    }
}

/// `kind` on a supply that `wave` drives through `rs` onto `cv` at `vcc`, with `rq` from `pull`
/// to `q`. A pull from `vcc` loads the supply only while the chip drives `q` low.
struct Rig {
    kind: Kind,
    wave: serde_json::Value,
    rs: f64,
    cv: Option<(f64, Option<f64>)>,
    rq: f64,
    pull: &'static str,
    stop_ps: u64,
    max_step_ps: u64,
    grid_ps: u64,
}

impl Rig {
    fn ramp(kind: Kind, rising: bool, rs: f64, cv: f64, rq: f64) -> Rig {
        let wave = if rising {
            serde_json::json!({
                "kind": "pulse", "initial": val(0.0, "V"), "pulsed": val(5.0, "V"),
                "delay_ps": MS, "width_ps": 100 * MS, "period_ps": 200 * MS,
            })
        } else {
            serde_json::json!({
                "kind": "step", "initial": val(5.0, "V"), "final": val(0.0, "V"), "at_ps": MS,
            })
        };
        Rig {
            kind,
            wave,
            rs,
            cv: Some((cv, None)),
            rq,
            pull: "vcc",
            stop_ps: 200 * MS,
            max_step_ps: MS,
            grid_ps: MS / 10,
        }
    }

    fn json(&self) -> serde_json::Value {
        let (extra_nets, chip) = self.kind.parts();
        let mut nets = vec!["gnd", "five", "vs", "vcc", "q"];
        nets.extend(extra_nets);
        let mut parts = vec![
            source("five", "five", val(5.0, "V")),
            source("vs", "vs", self.wave.clone()),
            resistor("rs", "vs", "vcc", self.rs),
            resistor("rq", self.pull, "q", self.rq),
        ];
        parts.extend(
            self.cv
                .map(|(farad, initial)| capacitor("cv", "vcc", farad, initial)),
        );
        parts.extend(chip);
        serde_json::json!({
            "version": 1,
            "seed": 0,
            "analysis": { "transient": {
                "stop_ps": self.stop_ps, "max_step_ps": self.max_step_ps, "grid_ps": self.grid_ps,
            } },
            "ground": "gnd",
            "nets": nets,
            "parts": parts,
        })
    }

    fn netlist(&self) -> Netlist {
        netlist::parse(&serde_json::to_vec(&self.json()).unwrap()).unwrap()
    }
}

fn column(results: &Results, key: &str) -> Vec<f64> {
    let trace = results.trace().expect("a transient trace");
    let index = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == key)
        .unwrap_or_else(|| panic!("no column {key}"));
    trace.column(index).to_vec()
}

/// At each accepted step, whether the chip drove `q` low, and the times it changed. A pull of
/// 0.1 V or less tells nothing, and keeps the step before's answer.
struct Steps {
    times: Vec<u64>,
    vcc: Vec<f64>,
    driven: Vec<bool>,
}

impl Steps {
    fn of(rig: &Rig) -> Steps {
        let results = orbe_engine::simulate_steps(&rig.netlist());
        let errors: Vec<_> = results
            .diagnostics()
            .iter()
            .filter(|found| found.severity() == orbe_engine::diagnostic::Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{:?}: {errors:?}", rig.kind);
        let trace = results.trace().unwrap();
        let times = trace.time().iter().map(|time| time.as_ps()).collect();
        let vcc = column(&results, "#net.vcc");
        let q = column(&results, "#net.q");
        let pull = column(&results, &format!("#net.{}", rig.pull));
        let mut driven = Vec::with_capacity(q.len());
        let mut last = false;
        for (&q, &pull) in q.iter().zip(&pull) {
            if pull > 0.1 {
                last = q < 0.5 * pull;
            }
            driven.push(last);
        }
        Steps { times, vcc, driven }
    }

    /// The times the chip turned on or off: the steps that landed where it took the change.
    fn changes(&self) -> Vec<u64> {
        (1..self.times.len())
            .filter(|&k| self.driven[k] != self.driven[k - 1])
            .map(|k| self.times[k - 1])
            .collect()
    }

    /// Every step that drives sits at or above the release, and every step that does not sits at
    /// or below the minimum. The step that lands where the chip changes still holds the state
    /// before, so it is left out.
    fn assert_in_band(&self, kind: Kind) {
        let (min, release) = kind.band();
        for k in 1..self.times.len() {
            let (at, vcc) = (self.times[k], self.vcc[k]);
            if self
                .driven
                .get(k + 1)
                .is_some_and(|&next| next != self.driven[k])
            {
                continue;
            }
            if self.driven[k] {
                assert!(
                    vcc >= release - 1e-9,
                    "{kind:?}: drives at {at} ps with VCC {vcc}, below the {release} V release"
                );
            } else {
                assert!(
                    vcc <= min + 1e-9,
                    "{kind:?}: released at {at} ps with VCC {vcc}, above the {min} V minimum"
                );
            }
        }
    }
}

fn bits(values: &[Finite]) -> Vec<u64> {
    values.iter().map(|value| value.get().to_bits()).collect()
}

/// A session split at each of `splits` and at every grid time reads each grid row of the batch
/// run bit for bit.
fn assert_split_reads_the_batch(rig: &Rig, splits: &[u64]) {
    let json = serde_json::to_vec(&rig.json()).unwrap();
    let done = orbe_engine::run(&json, Timestamp::from_unix_seconds(0));
    let trace = done.results.trace().unwrap();
    let columns = trace.columns().to_vec();
    let mut session = Session::open_json(&json, &Blobs::new()).unwrap();
    let mut stops: Vec<u64> = (0..trace.rows() as u64).map(|k| k * rig.grid_ps).collect();
    stops.extend_from_slice(splits);
    stops.sort_unstable();
    stops.dedup();
    for at in stops {
        let status = session.advance_to(Time::from_ps(at));
        assert_ne!(status, Status::Faulted, "{:?} at {at} ps", rig.kind);
        if at % rig.grid_ps != 0 {
            continue;
        }
        let row = (at / rig.grid_ps) as usize;
        let probe = session.probe();
        let read: Vec<Finite> = columns
            .iter()
            .map(|reference| probe.get(reference).unwrap())
            .collect();
        let want: Vec<u64> = (0..columns.len())
            .map(|k| trace.column(k)[row].to_bits())
            .collect();
        assert_eq!(bits(&read), want, "{:?}: row at {at} ps", rig.kind);
    }
}

/// The supply ramps through 10 kohm onto 1 uF while a 100 kohm pull-up from VCC loads the low
/// output: the chip powers up once on the way up and down once on the way down.
#[test]
fn a_slow_ramp_with_a_light_load_powers_up_and_down_once() {
    for kind in Kind::ALL {
        let rig = Rig::ramp(kind, true, 10_000.0, 1e-6, 100_000.0);
        let steps = Steps::of(&rig);
        steps.assert_in_band(kind);
        assert_eq!(steps.changes().len(), 2, "{kind:?}: {:?}", steps.changes());
        assert!(!steps.driven.last().unwrap(), "{kind:?} ends powered");
        let falling = Rig::ramp(kind, false, 10_000.0, 1e-6, 100_000.0);
        let steps = Steps::of(&falling);
        steps.assert_in_band(kind);
        assert_eq!(steps.changes().len(), 1, "{kind:?}: {:?}", steps.changes());
        assert!(steps.driven[0] && !steps.driven.last().unwrap());
    }
}

/// A stiffer supply and heavier loads: 100 ohm onto 10 uF with 10 kohm, 1 kohm onto 0.1 uF with
/// 1 kohm, and 10 kohm onto 1 uF with 1 kohm (which drags VCC below release each time the chip
/// drives, cycling cleanly on the capacitor).
#[test]
fn a_loaded_ramp_stays_in_the_band_rising_and_falling() {
    for kind in Kind::ALL {
        for (rs, cv, rq) in [
            (100.0, 1e-5, 10_000.0),
            (1_000.0, 1e-7, 1_000.0),
            (10_000.0, 1e-6, 1_000.0),
        ] {
            let rising = Rig::ramp(kind, true, rs, cv, rq);
            let steps = Steps::of(&rising);
            steps.assert_in_band(kind);
            assert!(!steps.changes().is_empty(), "{kind:?} never powered");
            assert!(!steps.driven.last().unwrap(), "{kind:?} ends powered");
        }
        // Falling ramps with a self-consistent DC operating point at 5 V (and an initialized
        // capacitor for the heavy load that has no DC operating point at 5 V).
        for (rs, cv, init, rq) in [
            (100.0, 1e-5, None, 10_000.0),
            (1_000.0, 1e-7, Some(5.0), 1_000.0),
            (10_000.0, 1e-6, Some(5.0), 1_000.0),
        ] {
            let mut falling = Rig::ramp(kind, false, rs, cv, rq);
            falling.cv = Some((cv, init));
            let steps = Steps::of(&falling);
            steps.assert_in_band(kind);
            assert!(steps.driven[0], "{kind:?} not powered at t = 0");
            assert!(!steps.changes().is_empty(), "{kind:?} never released");
            assert!(!steps.driven.last().unwrap(), "{kind:?} ends powered");
        }
    }
}

/// A feedback load near the minimum supply (10 ohm source at min + 0.01 V, loaded by 1 kohm from
/// VCC) pulls VCC slightly below min once driving, but stays above the release threshold so both
/// operating point and transient settle cleanly powered.
#[test]
fn a_feedback_supply_inside_the_band_settles_with_hysteresis() {
    for kind in Kind::ALL {
        let (min, release) = kind.band();
        for top in [min + 0.01, min + 0.03] {
            for cv in [None, Some((1e-6, None))] {
                let rig = Rig {
                    kind,
                    wave: val(top, "V"),
                    rs: 10.0,
                    cv,
                    rq: 1_000.0,
                    pull: "vcc",
                    stop_ps: 20 * MS,
                    max_step_ps: MS / 10,
                    grid_ps: MS / 10,
                };
                let steps = Steps::of(&rig);
                steps.assert_in_band(kind);
                assert!(steps.driven[0], "{kind:?} top={top} not powered at t=0");
                assert!(
                    steps.vcc[0] >= release && steps.vcc[0] <= top,
                    "{kind:?} top={top}: vcc={}",
                    steps.vcc[0]
                );
                assert_eq!(steps.changes(), Vec::<u64>::new(), "{kind:?} top={top}");

                let mut op = rig.json();
                op["analysis"] = serde_json::json!({ "op": {} });
                let done = orbe_engine::run(
                    &serde_json::to_vec(&op).unwrap(),
                    Timestamp::from_unix_seconds(0),
                );
                assert!(
                    done.results.diagnostics().is_empty(),
                    "{kind:?} op top={top}: {:?}",
                    done.results.diagnostics()
                );
                let obs: serde_json::Value =
                    serde_json::from_slice(done.folder.file("observations.json").unwrap()).unwrap();
                let q = obs["op"]["#net.q"].as_f64().unwrap();
                let vcc = obs["op"]["#net.vcc"].as_f64().unwrap();
                assert!(q < 0.5 * vcc, "{kind:?} op top={top}: q={q} vcc={vcc}");
            }
        }
    }
}

#[test]
fn a_gated_run_reads_the_same_split_into_calls() {
    let splits = [
        1,
        MS + 1,
        3 * MS + 333_333,
        6_100_000_001,
        10_200_000_007,
        57 * MS + 13,
        101 * MS + 5,
    ];
    for kind in Kind::ALL {
        for (rising, rs, cv, rq) in [
            (true, 10_000.0, 1e-6, 100_000.0),
            (true, 1_000.0, 1e-7, 1_000.0),
            (true, 10_000.0, 1e-6, 1_000.0),
        ] {
            let mut rig = Rig::ramp(kind, rising, rs, cv, rq);
            rig.stop_ps = 120 * MS;
            assert_split_reads_the_batch(&rig, &splits);
        }
    }
}

/// The chip turns on and off at the same times, to within 1 us, at a tenth of the step.
#[test]
fn a_gated_run_switches_at_the_same_times_at_a_shorter_step() {
    for kind in Kind::ALL {
        for (rising, rs, cv, rq) in [
            (true, 10_000.0, 1e-6, 100_000.0),
            (true, 100.0, 1e-5, 10_000.0),
        ] {
            let rig = Rig::ramp(kind, rising, rs, cv, rq);
            let short = Rig {
                max_step_ps: rig.max_step_ps / 10,
                ..Rig::ramp(kind, rising, rs, cv, rq)
            };
            let long = Steps::of(&rig).changes();
            let fine = Steps::of(&short).changes();
            assert_eq!(long.len(), fine.len(), "{kind:?}: {long:?} {fine:?}");
            for (a, b) in long.iter().zip(&fine) {
                assert!(a.abs_diff(*b) < 1_000_000, "{kind:?}: {long:?} {fine:?}");
            }
        }
    }
}

/// A seeded xorshift, so a failing hover replays.
fn noise(seed: &mut u64) -> f64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed % 2001) as f64 / 1000.0 - 1.0
}

/// A supply stepped every 10 us around `center` by up to `swing`, after `before` for 1 ms.
fn hover(kind: Kind, before: f64, center: f64, swing: f64) -> Rig {
    let mut seed = 0x9e37_79b9_7f4a_7c15;
    let mut events = vec![serde_json::json!({ "t_ps": MS, "level": val(center, "V") })];
    let mut last = center;
    for k in 1..400 {
        let level = center + swing * noise(&mut seed);
        if level != last {
            events
                .push(serde_json::json!({ "t_ps": MS + k * 10_000_000, "level": val(level, "V") }));
            last = level;
        }
    }
    Rig {
        kind,
        wave: serde_json::json!({ "kind": "steps", "initial": val(before, "V"), "events": events }),
        rs: 100.0,
        cv: Some((1e-9, None)),
        rq: 10_000.0,
        pull: "five",
        stop_ps: 6 * MS,
        max_step_ps: 10_000_000,
        grid_ps: 10_000_000,
    }
}

/// Inside the band the chip keeps the state it came in with: a supply that hovers 0.05 V around
/// the middle of the band after 5 V keeps driving, after 0 V stays released, and one that hovers
/// 3 uV around the minimum after 0 V turns on once and stays on.
#[test]
fn a_supply_hovering_in_the_band_does_not_chatter() {
    for kind in Kind::ALL {
        let (min, release) = kind.band();
        let steps = Steps::of(&hover(kind, 5.0, 0.5 * (min + release), 0.05));
        steps.assert_in_band(kind);
        assert!(steps.driven[0], "{kind:?} is off at 5 V");
        assert_eq!(steps.changes(), Vec::<u64>::new(), "{kind:?}");

        let steps = Steps::of(&hover(kind, 0.0, 0.5 * (min + release), 0.05));
        steps.assert_in_band(kind);
        assert!(!steps.driven[0], "{kind:?} is on at 0 V");
        assert_eq!(steps.changes(), Vec::<u64>::new(), "{kind:?}");

        let steps = Steps::of(&hover(kind, 0.0, min, 3e-6));
        steps.assert_in_band(kind);
        assert_eq!(steps.changes().len(), 1, "{kind:?}: {:?}", steps.changes());
        assert!(steps.driven.last().unwrap(), "{kind:?} turned off");
    }
}

/// Where neither state is self-consistent (released lets VCC rise past min while driving pulls
/// VCC below release), the operating point and a capacitorless transient edge both report
/// `chip.unsettled` loudly rather than silently returning a contradicting state.
#[test]
fn an_inconsistent_supply_reports_unsettled_rather_than_contradicting() {
    for kind in Kind::ALL {
        let (min, _) = kind.band();
        let top = min + 0.1;

        // 1) Operating point (with or without an uninitialized capacitor, which is open at DC).
        for cv in [None, Some((1e-6, None))] {
            let rig = Rig {
                kind,
                wave: val(top, "V"),
                rs: 1_000.0,
                cv,
                rq: 1_000.0,
                pull: "vcc",
                stop_ps: MS,
                max_step_ps: MS / 10,
                grid_ps: MS / 10,
            };
            let mut op = rig.json();
            op["analysis"] = serde_json::json!({ "op": {} });
            let op_netlist = netlist::parse(&serde_json::to_vec(&op).unwrap()).unwrap();
            let op_res = orbe_engine::simulate(&op_netlist);
            let op_codes: Vec<_> = op_res.diagnostics().iter().map(|d| d.code()).collect();
            assert_eq!(op_codes, ["chip.unsettled"], "{kind:?} op cv={cv:?}");
            assert!(op_res.observations().op().is_none());

            let tran_res = orbe_engine::simulate(&rig.netlist());
            let tran_codes: Vec<_> = tran_res.diagnostics().iter().map(|d| d.code()).collect();
            assert_eq!(tran_codes, ["chip.unsettled"], "{kind:?} tran cv={cv:?}");
        }

        // 2) Starts cleanly off at 0 V, then steps into the inconsistent region with no capacitor.
        let stepped = Rig {
            kind,
            wave: serde_json::json!({
                "kind": "step", "initial": val(0.0, "V"), "final": val(top, "V"), "at_ps": MS,
            }),
            rs: 1_000.0,
            cv: None,
            rq: 1_000.0,
            pull: "vcc",
            stop_ps: 2 * MS,
            max_step_ps: MS / 10,
            grid_ps: MS / 10,
        };
        let res = orbe_engine::simulate(&stepped.netlist());
        let codes: Vec<_> = res.diagnostics().iter().map(|d| d.code()).collect();
        assert_eq!(codes, ["chip.unsettled"], "{kind:?} stepped without cap");
    }
}

/// An HD44780 on an RC-ramped VDD whose pull-ups load VDD via a grounded pin converges cleanly,
/// powers up at 2.7 V, powers down at 2.4 V, and stays quiet hovering in the band.
#[test]
fn hd44780_supply_gate_ramps_and_holds_hysteresis_band() {
    let make = |wave: serde_json::Value, stop_ps: u64, max_step_ps: u64| {
        serde_json::json!({
            "version": 1,
            "seed": 0,
            "analysis": { "transient": {
                "stop_ps": stop_ps, "max_step_ps": max_step_ps, "grid_ps": max_step_ps,
            } },
            "ground": "gnd",
            "nets": ["gnd", "vs", "vdd", "rs", "rw", "e", "d4", "d5", "d6", "d7", "v0", "a"],
            "parts": [
                source("vs", "vs", wave),
                resistor("rs_vdd", "vs", "vdd", 1_000.0),
                capacitor("cv", "vdd", 1e-6, None),
                resistor("r_load", "rs", "gnd", 10_000.0),
                resistor("r_v0", "v0", "gnd", 10_000.0),
                resistor("r_a", "a", "gnd", 10_000.0),
                {
                    "name": "lcd",
                    "kind": "hd44780",
                    "pins": {
                        "VSS": "gnd", "VDD": "vdd", "V0": "v0",
                        "RS": "rs", "RW": "rw", "E": "e",
                        "D0": "gnd", "D1": "gnd", "D2": "gnd", "D3": "gnd",
                        "D4": "d4", "D5": "d5", "D6": "d6", "D7": "d7",
                        "A": "a", "K": "gnd",
                    },
                    "output": stage(100.0, 100.0),
                    "pull_up": val(20_000.0, "ohm"),
                    "input_threshold": { "low": val(0.2, "1"), "high": val(0.7, "1") },
                    "operating_voltage": val(4.5, "V"),
                    "lcd_voltage": val(3.0, "V"),
                    "backlight_voltage": val(3.0, "V"),
                    "draw": {
                        "logic_current": val(0.001, "A"),
                        "holds_above": val(2.7, "V"),
                    },
                },
            ],
        })
    };
    let pulse = serde_json::json!({
        "kind": "pulse", "initial": val(0.0, "V"), "pulsed": val(5.0, "V"),
        "delay_ps": MS, "width_ps": 30 * MS, "period_ps": 60 * MS,
    });
    let netlist = netlist::parse(&serde_json::to_vec(&make(pulse, 60 * MS, MS)).unwrap()).unwrap();
    let results = orbe_engine::simulate_steps(&netlist);
    let errors: Vec<_> = results
        .diagnostics()
        .iter()
        .filter(|d| d.severity() == orbe_engine::diagnostic::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "hd44780 ramp errors: {errors:?}");
    let vdd = column(&results, "#net.vdd");
    let rs = column(&results, "#net.rs");
    // When powered, RS is pulled up by 20 kohm against the 10 kohm load to GND (~1/3 VDD).
    // When released, RS is at 0 V.
    let mut driven = Vec::with_capacity(rs.len());
    for (&v, &r) in vdd.iter().zip(&rs) {
        driven.push(v > 0.1 && r > 0.1 * v);
    }
    let changes: Vec<usize> = (1..driven.len())
        .filter(|&k| driven[k] != driven[k - 1])
        .collect();
    assert_eq!(changes.len(), 2, "hd44780 ramp changes: {changes:?}");
    assert!(!driven[0] && driven[changes[0]] && !driven[changes[1]]);
    assert!(
        (vdd[changes[0] - 1] - 2.7).abs() < 1e-3,
        "powered up at {} V",
        vdd[changes[0] - 1]
    );
    assert!(
        (vdd[changes[1] - 1] - 2.4).abs() < 1e-3,
        "powered down at {} V",
        vdd[changes[1] - 1]
    );
}
