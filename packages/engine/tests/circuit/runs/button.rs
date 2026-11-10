//! packages/engine/tests/circuits/atmega-button, `packages/engine/tests/firmware/button.S` on an ATmega328P at 16 MHz. PD2 reads a push
//! button to ground with its pull-up on, and PB5 drives an LED with the inverse of PIND2.
//! The edges follow the synchronizer and the loop's cycle counts, and the LED current follows
//! the closed form.

use orbe_engine::netlist::{self, Contact, Model};

use crate::runs::common;
use common::atmega::{
    assert_led_follows, chip_matches_fixture, cycle_ps, logic_edges, logic_vcd, run_beside_steps,
    serial_csv,
};

/// Cycles from a pad change until `PINx` shows it. `PIN_SYNC_DELAY` in
/// `packages/socs/microchip/src/mux.rs` is that count, from DS40002061B §14.2.4. A change at
/// cycle T is first visible to an instruction that starts at T + 2.
const SYNC_CYCLES: u64 = 2;

/// `button.S`, DS40002198B, AVRe column. `CBI` and each `SBI` take 2. The loop is `IN` 1,
/// `COM` 1, `BST` 1, `IN` 1, `BLD` 1, `OUT` 1, `RJMP` 2.
const LOOP_CYCLES: u64 = 1 + 1 + 1 + 1 + 1 + 1 + 2;
const IN_TO_OUT: u64 = 1 + 1 + 1 + 1 + 1 + 1;
/// `SBI PORTD,2` retires at cycle 4.
const PULLUP_CYCLES: u64 = 2 + 2;
/// `SBI DDRB,5` retires at cycle 6. The loop's first `IN` starts on that cycle.
const DDRB_CYCLES: u64 = PULLUP_CYCLES + 2;
const FIRST_IN: u64 = DDRB_CYCLES;

#[test]
fn the_chip_is_the_tests_fixture_running_button_hex() {
    chip_matches_fixture("atmega-button", "packages/engine/tests/firmware/button.hex");
}

/// A contact level no `IN` samples. `cycles` is its visible span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Missed {
    at_ps: u64,
    high: bool,
    cycles: u64,
}

struct Sampled {
    pb5: Vec<(u64, bool)>,
    missed: Vec<Missed>,
}

struct Pad {
    cycle: u64,
    high: bool,
}

fn switch_edges(parsed: &netlist::Netlist) -> Vec<(u64, bool)> {
    let (_, part) = parsed
        .parts()
        .find(|(_, part)| part.name.as_str() == "sw1")
        .expect("sw1");
    let Model::Switch(switch) = &part.model else {
        panic!("sw1 is a switch");
    };
    assert_eq!(*switch.schedule.initial(), Contact::Open);
    switch
        .schedule
        .events()
        .iter()
        .map(|edge| (edge.at.as_ps(), edge.to == Contact::Open))
        .collect()
}

fn pullup_edge(cycle_ps: u64) -> (u64, bool) {
    (PULLUP_CYCLES * cycle_ps, true)
}

fn next_in_at_or_after(visible: u64) -> u64 {
    let phase = FIRST_IN % LOOP_CYCLES;
    let mut sample = visible.max(FIRST_IN);
    let rem = sample % LOOP_CYCLES;
    if rem != phase {
        sample += (phase + LOOP_CYCLES - rem) % LOOP_CYCLES;
    }
    sample
}

/// PB5 edges `button.S` writes for these PD2 edges.
///
/// A PD2 level whose visible span holds no `IN` start makes no PB5 edge, in either polarity.
/// `missed` leaves out the level before the pull-up, which no `IN` can sample.
/// `IN` starts at `FIRST_IN` and every `LOOP_CYCLES` after. The lag from the pad change to the
/// `OUT` is 8 to 15 cycles. The sample can start on the first visible cycle, or one cycle
/// early and then a whole loop later, so the lag runs from `SYNC_CYCLES + IN_TO_OUT` to
/// `SYNC_CYCLES - 1 + LOOP_CYCLES + IN_TO_OUT`.
fn sampled_pb5(pd2: &[(u64, bool)], cycle_ps: u64, stop_ps: u64) -> Sampled {
    let mut levels = Vec::with_capacity(pd2.len() + 1);
    levels.push(Pad {
        cycle: 0,
        high: false,
    });
    for &(at_ps, high) in pd2 {
        levels.push(Pad {
            cycle: at_ps / cycle_ps,
            high,
        });
    }
    let end = stop_ps / cycle_ps;
    let mut pb5 = Vec::new();
    let mut missed = Vec::new();
    let mut driven = false;
    for i in 0..levels.len() {
        let pad = levels[i].cycle;
        let high = levels[i].high;
        let visible = pad + SYNC_CYCLES;
        let next_visible = match levels.get(i + 1) {
            Some(next) => next.cycle + SYNC_CYCLES,
            None => end + SYNC_CYCLES,
        };
        let sample = next_in_at_or_after(visible);
        if sample >= next_visible {
            if i > 0 {
                let (at_ps, _) = pd2[i - 1];
                missed.push(Missed {
                    at_ps,
                    high,
                    cycles: next_visible - visible,
                });
            }
            continue;
        }
        let want = !high;
        if want == driven {
            continue;
        }
        let at_ps = (sample + IN_TO_OUT) * cycle_ps;
        if at_ps >= stop_ps {
            break;
        }
        pb5.push((at_ps, want));
        driven = want;
    }
    Sampled { pb5, missed }
}

/// `orbe-run run` and a steps run of the same netlist side by side, so the test costs one run.
#[test]
fn a_run_writes_pd2_and_pb5_at_the_picoseconds_the_loop_adds_up_to() {
    let beside = run_beside_steps("atmega-button");
    assert_eq!(
        serial_csv(&beside),
        b"t_ps,part,usart,byte\n",
        "the USART sends nothing"
    );
    let netlist = &beside.netlist;
    let observations = &beside.observations;

    let cycle_ps = cycle_ps(netlist);
    let stop_ps = netlist["analysis"]["transient"]["stop_ps"]
        .as_u64()
        .unwrap();
    let schedule = switch_edges(&beside.parsed);
    let mut nominal = vec![pullup_edge(cycle_ps)];
    nominal.extend(schedule.iter().copied());

    let listed = observations["switches"]["#sw1"].as_array().unwrap();
    assert_eq!(listed.len(), schedule.len());
    for (got, &(at_ps, high)) in listed.iter().zip(&schedule) {
        assert_eq!(got["t_ps"].as_u64().unwrap(), at_ps);
        assert_eq!(got["state"] == "open", high);
    }

    let vcd = logic_vcd(&beside, &["#u1.logic.PD2", "#u1.logic.PB5"]);
    assert_eq!(vcd["#u1.logic.PD2"]["initial"], "low");
    assert_eq!(vcd["#u1.logic.PB5"]["initial"], "low");
    let pd2 = logic_edges(&vcd, "#u1.logic.PD2");
    let pb5 = logic_edges(&vcd, "#u1.logic.PB5");
    assert_eq!(pd2, nominal);

    let seed = netlist["seed"].as_u64().unwrap();
    let want_pb5 = sampled_pb5(&nominal, cycle_ps, stop_ps);
    let want_missed = [Missed {
        at_ps: 100_611_385_295,
        high: false,
        cycles: 5,
    }];
    let width_ps = nominal
        .iter()
        .find(|(at_ps, _)| *at_ps > want_missed[0].at_ps)
        .map(|(at_ps, _)| *at_ps)
        .unwrap()
        - want_missed[0].at_ps;
    assert_eq!(width_ps, 354_423, "seed {seed} closure");
    assert_eq!(
        want_pb5.missed.as_slice(),
        want_missed.as_slice(),
        "seed {seed} misses the 354.423 ns closure at {} ps",
        want_missed[0].at_ps
    );
    assert_eq!(pb5, want_pb5.pb5);
    assert_led_follows(&beside, &pb5, DDRB_CYCLES * cycle_ps);
}
