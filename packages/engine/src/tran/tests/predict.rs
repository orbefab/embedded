//! A restart starts from the change the same image change made before. It lands within
//! Newton's tolerance of a restart from the landing, a wrong guess falls back to the landing,
//! and a session advanced in any chunks guesses as one advanced at once.

use orbe_model::Drive;

use crate::netlist::{Appended, Contact, Netlist};

use super::super::*;
use super::active::{NS, US, capacitor, circuit, net, resistor};
use super::fixtures::*;
use super::inputs::without_events;
use super::session::{AVR_STOP_PS, Rng, batch, example, finish};
use crate::active;
use crate::active::script::{self, Op};
use crate::name::Name;
use crate::run_folder::Results;
use crate::session::predict::Mode;
use crate::session::{Input, Setting, State, Status};

const LED: &str = r#"{ "name": "led1", "kind": "led", "pins": { "anode": "x", "cathode": "gnd" },
      "saturation_current": { "value": 6.711e-10, "unit": "A" },
      "emission_coefficient": { "value": 4.427, "unit": "1" },
      "series_resistance": { "value": 2.115, "unit": "ohm" },
      "ratings": { "current": null, "power": null } }"#;

/// A pin powered from `rail`, a 10 ohm and 1 uF supply that sags as it drives, into an LED
/// through 330 ohm. It turns high and low every 3 us from 1 us, so each restart has an LED's
/// junction to settle and a rail a little lower than the time before.
fn pumped(stop_ps: u64) -> (Netlist, active::Chip) {
    let netlist = circuit(
        stop_ps,
        &["rail", "out", "x"],
        &[
            resistor("rs", "vcc", "rail", 10.0),
            capacitor("cs", "rail", 1e-6),
            resistor("r1", "out", "x", 330.0),
            LED.to_string(),
        ],
    );
    let pins = (
        net(&netlist, "rail"),
        net(&netlist, "gnd"),
        net(&netlist, "out"),
        net(&netlist, "x"),
    );
    let script = (0..)
        .map(|k| US + k * 3 * US)
        .take_while(|&at| at < stop_ps)
        .enumerate()
        .map(|(k, at)| {
            let drive = if k % 2 == 0 { Drive::High } else { Drive::Low };
            (Time::from_ps(at), Op::Drive(drive))
        })
        .collect();
    (netlist, script::chip(pins, script, None))
}

/// `netlist` with `chip`, guessing as `mode` says, advanced to its stop in one call, or in
/// chunks seeded by `seed`.
fn run(
    netlist: &Netlist,
    chip: &active::Chip,
    mode: Mode,
    seed: Option<u64>,
) -> (Results, RunStats) {
    let mut session = open(netlist, vec![chip.clone()], BUDGET);
    session.predictor.mode = mode;
    let stop = transient(netlist).stop;
    match seed {
        None => {
            session.advance_to(stop);
        }
        Some(seed) => {
            let mut rng = Rng(seed);
            while let State::Running { now, next, .. } = &session.state {
                let (now, landing) = (now.as_ps(), next.at.as_ps());
                let to = match rng.below(4) {
                    0 => landing,
                    1 => landing - 1,
                    2 => now + 1 + rng.below(NS),
                    _ => now + 1 + rng.below(stop.as_ps() / 50),
                };
                session.advance_to(Time::from_ps(to));
            }
        }
    }
    let stats = session.stats.clone();
    (finish(session), stats)
}

/// Every row of every column of `got` within 1e-3 of the column's span in `want`, plus 1 nV,
/// 1 pA or 1 fW, whichever the column holds.
fn assert_within_bound(got: &Results, want: &Results) {
    let (got, want) = (got.trace().unwrap(), want.trace().unwrap());
    assert_eq!(got.time(), want.time());
    for (j, column) in want.columns().iter().enumerate() {
        let (a, b) = (want.column(j), got.column(j));
        let span = a.iter().fold(f64::NEG_INFINITY, |m, &v| m.max(v))
            - a.iter().fold(f64::INFINITY, |m, &v| m.min(v));
        let floor = match column.to_string() {
            name if name.ends_with(".p") => 1e-15,
            name if name.ends_with(".i") => 1e-12,
            _ => 1e-9,
        };
        let worst = a
            .iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(
            worst <= 1e-3 * span + floor,
            "{column}: off by {worst:e} on a span of {span:e}"
        );
    }
}

fn same_bits(got: &Results, want: &Results) {
    let (got, want) = (got.trace().unwrap(), want.trace().unwrap());
    assert_eq!(got.time(), want.time());
    for j in 0..want.columns().len() {
        let bits = |column: &[f64]| column.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(
            bits(got.column(j)),
            bits(want.column(j)),
            "{}",
            want.columns()[j]
        );
    }
}

#[test]
fn a_restart_from_its_guess_lands_within_the_bound_of_one_from_the_landing() {
    let (netlist, chip) = pumped(200 * US);
    let (guessed, with) = run(&netlist, &chip, Mode::On, None);
    let (plain, without) = run(&netlist, &chip, Mode::Off, None);
    assert_eq!(without.guesses, 0);
    // The first high and the first low have nothing to go on, and while the rail sags from
    // pulse to pulse a landing is too far from the last to guess from.
    assert!(with.guesses > with.restarts / 2, "{with:?}");
    assert!(with.guesses < with.restarts - 2, "{with:?}");
    assert_eq!(with.guessed, with.guesses, "{with:?}");
    assert!(
        with.restart_iterations * 2 < without.restart_iterations,
        "{with:?} {without:?}"
    );
    assert_within_bound(&guessed, &plain);
}

/// A guess a kilovolt off on every row, its junctions a volt off, never converges within its
/// budget. Each restart then solves from the landing as it would with no guess, and costs the
/// two iterations of the guess more.
#[test]
fn a_wrong_guess_falls_back_to_the_landing() {
    let (netlist, chip) = pumped(200 * US);
    let (spoiled, with) = run(&netlist, &chip, Mode::Spoiled, None);
    let (plain, without) = run(&netlist, &chip, Mode::Off, None);
    assert!(with.guesses > 0, "{with:?}");
    assert_eq!(with.guessed, 0, "{with:?}");
    assert_eq!(with.restarts, without.restarts);
    assert_eq!(
        with.restart_iterations,
        without.restart_iterations + 2 * with.guesses,
        "{with:?} {without:?}"
    );
    same_bits(&spoiled, &plain);
}

#[test]
fn a_session_in_chunks_guesses_as_one_call() {
    let (netlist, chip) = pumped(200 * US);
    let (whole, stats) = run(&netlist, &chip, Mode::On, None);
    assert!(stats.guessed > 0, "{stats:?}");
    for seed in [0x5eed_2001, 0x5eed_2002, 0x5eed_2003] {
        for mode in [Mode::On, Mode::Spoiled] {
            let (one, one_stats) = run(&netlist, &chip, mode, None);
            let (chunked, chunked_stats) = run(&netlist, &chip, mode, Some(seed));
            same_bits(&chunked, &one);
            assert_eq!(chunked_stats.guesses, one_stats.guesses, "{mode:?}");
            assert_eq!(chunked_stats.guessed, one_stats.guessed, "{mode:?}");
            assert_eq!(
                chunked_stats.restart_iterations, one_stats.restart_iterations,
                "{mode:?}"
            );
        }
    }
    same_bits(&run(&netlist, &chip, Mode::On, Some(0x5eed_2004)).0, &whole);
}

/// Live presses drop the held step and run the cores again from a landing. The session guesses
/// as a batch run with each press in the netlist from the start.
#[test]
fn live_presses_guess_as_a_batch_with_them_scheduled() {
    let input = without_events(&example("atmega-button", AVR_STOP_PS));
    let netlist = crate::netlist::parse(&input).unwrap();
    let stop = transient(&netlist).stop.as_ps();
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let mut landings = netlist.clone();
    let sw1 = Name::new("sw1").unwrap();
    let mut rng = Rng(0x5eed_2005);
    let mut contact = Contact::Open;
    while let State::Running { now, .. } = &session.state {
        let now = now.as_ps();
        session.advance_to(Time::from_ps(now + 1 + rng.below(stop / 40)));
        if session.status() != Status::Running || rng.below(3) != 0 {
            continue;
        }
        contact = contact.flipped();
        let press = Input {
            part: sw1.clone(),
            at: session.now(),
            setting: Setting::Contact(contact),
        };
        let at = session.apply(&press).unwrap().at;
        let id = landings.part_id(&sw1).unwrap();
        landings.append(id, at, Appended::Contact(contact));
    }
    let stats = session.stats.clone();
    assert!(stats.guessed > 0, "{stats:?}");
    same_bits(&finish(session), &batch(&landings, BUDGET));
}

/// The chunked and live-input tests in `session` and `inputs` run these circuits at these
/// stops, so they check sessions that guess against the batch.
#[test]
fn the_examples_run_in_chunks_guess() {
    for (name, stop) in [
        ("atmega-analog-serial", u64::MAX),
        ("atmega-fade", AVR_STOP_PS),
        ("ne555-led", u64::MAX),
    ] {
        let netlist = crate::netlist::parse(&example(name, stop)).unwrap();
        let mut session = open(&netlist, Vec::new(), BUDGET);
        session.advance_to(transient(&netlist).stop);
        let stats = &session.stats;
        assert!(stats.guessed > 0, "{name}: {stats:?}");
    }
}

/// Two transistors cross-coupled through 47 kohm, with a femtofarad or less on each junction,
/// set every 20 us by a 5 us pulse through `r_set` from a pin powered by a rail that falls
/// from 5 V with a 2 ms time constant, and reset 10 us later from a pin on 5 V. Near the rail
/// where the pulse stops flipping it, the set's last change is a flip the pulse may no longer
/// make.
fn latch(stop_ps: u64, r_set: f64) -> (Netlist, Vec<active::Chip>) {
    let rail = r#"{ "name": "cr", "kind": "capacitor", "pins": { "a": "rail", "b": "gnd" },
          "capacitance": { "value": 1e-6, "unit": "F" },
          "initial": { "value": 5.0, "unit": "V" } }"#;
    let parts = [
        rail.to_string(),
        resistor("rc", "gnd", "rail", 2000.0),
        resistor("rc1", "vcc", "c1", 4700.0),
        resistor("rc2", "vcc", "c2", 4700.0),
        resistor("rb1", "c2", "b1", 47000.0),
        resistor("rb2", "c1", "b2", 47000.0),
        resistor("rs", "a", "b1", r_set),
        resistor("rr", "r", "b2", 4700.0),
        resistor("rx", "x", "gnd", 1e6),
        npn("q1", "c1", "b1"),
        npn("q2", "c2", "b2"),
    ];
    let netlist = circuit(
        stop_ps,
        &["rail", "c1", "c2", "b1", "b2", "a", "r", "x"],
        &parts,
    );
    let (mut set, mut reset) = (Vec::new(), Vec::new());
    for t in (10 * US..stop_ps - 20 * US).step_by(20 * US as usize) {
        set.push((Time::from_ps(t), Op::Drive(Drive::High)));
        set.push((Time::from_ps(t + 5 * US), Op::Drive(Drive::Off)));
        reset.push((Time::from_ps(t + 10 * US), Op::Drive(Drive::High)));
        reset.push((Time::from_ps(t + 15 * US), Op::Drive(Drive::Off)));
    }
    let pins = |vcc: &str, out: &str| {
        let net = |name| net(&netlist, name);
        (net(vcc), net("gnd"), net(out), net("x"))
    };
    let set = script::chip(pins("rail", "a"), set, None);
    let reset = script::chip(pins("vcc", "r"), reset, None);
    (netlist, vec![set, reset])
}

fn npn(name: &str, c: &str, b: &str) -> String {
    format!(
        r#"{{ "name": "{name}", "kind": "npn",
      "pins": {{ "collector": "{c}", "base": "{b}", "emitter": "gnd" }},
      "saturation_current": {{ "value": 8.095e-15, "unit": "A" }},
      "forward_beta": {{ "value": 173.2, "unit": "1" }},
      "reverse_beta": {{ "value": 0.2704, "unit": "1" }},
      "forward_emission": {{ "value": 1.0, "unit": "1" }},
      "reverse_emission": {{ "value": 1.0, "unit": "1" }},
      "early_voltage": {{ "value": 100.0, "unit": "V" }},
      "base_resistance": {{ "value": 5.707, "unit": "ohm" }},
      "collector_resistance": {{ "value": 1.531, "unit": "ohm" }},
      "emitter_resistance": {{ "value": 0.867, "unit": "ohm" }},
      "base_emitter_capacitance": {{ "value": 1e-16, "unit": "F" }},
      "base_collector_capacitance": {{ "value": 1e-16, "unit": "F" }},
      "ratings": {{ "current": null, "power": null }} }}"#
    )
}

/// The set pulses stop flipping the latch on the same pulse with guesses as without.
#[test]
fn a_latch_set_from_a_falling_rail_flips_as_without_guesses() {
    let (netlist, chips) = latch(2_000 * US, 47e3);
    let go = |mode| {
        let mut session = open(&netlist, chips.clone(), BUDGET);
        session.predictor.mode = mode;
        session.advance_to(transient(&netlist).stop);
        let stats = session.stats.clone();
        (finish(session), stats)
    };
    let (guessed, with) = go(Mode::On);
    let (plain, _) = go(Mode::Off);
    assert!(with.guessed > with.restarts / 2, "{with:?}");
    let flips = |results: &Results| {
        let trace = results.trace().unwrap();
        let c1 = trace
            .columns()
            .iter()
            .position(|c| c.to_string() == "#net.c1");
        let c1 = trace.column(c1.unwrap());
        let highs: Vec<bool> = c1.iter().map(|&v| v > 2.5).collect();
        highs.windows(2).filter(|w| w[0] != w[1]).count()
    };
    let flipped = flips(&plain);
    assert!(flipped > 20, "{flipped}");
    assert_eq!(flips(&guessed), flipped);
    assert_within_bound(&guessed, &plain);
}
