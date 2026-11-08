//! packages/engine/tests/circuits/atmega-fade, `packages/engine/tests/firmware/fade.S` on an ATmega328P at 16 MHz with an LED on PB1.
//! PB1 is OC1A. The edges follow the firmware's cycle counts and docs/engine/avr.md parameter 4.

mod common;

use common::atmega::{
    assert_led_follows, chip_matches_fixture, cycle_ps, logic_edges, logic_vcd, run_beside_steps,
    serial_csv,
};

/// Countdown trips in `fade.S`.
const COUNT: u64 = 95_995;
/// Table 16-4 mode 1. TOP is 0xFF.
const TOP: u64 = 255;
/// Table 16-5, CS = 011.
const DIV: u64 = 64;
/// §16.9.4. One period is 2 × TOP timer clocks.
const PERIOD: u64 = 2 * TOP;
const SBI: u64 = 2;
const LDI: u64 = 1;
const STS: u64 = 2;
const RJMP: u64 = 2;
const UPDATE: u64 = 12;
const STORE: u64 = 5;
const NOPS: u64 = 4;
const COUNTDOWN: u64 = 5 * COUNT - 1;
const DELAY_CYCLES: u64 = 3 * LDI + COUNTDOWN + NOPS;
const FIRST_RETIRE: u64 = SBI + LDI + STS + LDI + STS + LDI + LDI + DELAY_CYCLES + UPDATE + STORE;
const STEP_CYCLES: u64 = RJMP + DELAY_CYCLES + UPDATE + STORE;
const _: () = assert!(STEP_CYCLES == 480_000);
// Stores retire off every timer clock, so `ocr_at`'s `<` and `<=` give the same edges.
const _: () = assert!(FIRST_RETIRE % DIV == 8 && STEP_CYCLES.is_multiple_of(DIV));

#[test]
fn the_chip_is_the_tests_fixture_running_fade_hex() {
    chip_matches_fixture("atmega-fade", "packages/engine/tests/firmware/fade.hex");
}

/// `SBI DDRB,1` takes two cycles (DS40002198B, AVRe column).
fn driven_ps(netlist: &serde_json::Value) -> u64 {
    2 * cycle_ps(netlist)
}

/// `(retire cycle, OCR1A)` of each store whose retire edge is before `limit_cycles`.
/// The value starts at 0. Each store adds the step, and the step negates when
/// the value lands on 0 or 255.
fn writes(limit_cycles: u64) -> Vec<(u64, u8)> {
    let mut ocr: i32 = 0;
    let mut step: i32 = 5;
    let mut out = Vec::new();
    for k in 0.. {
        let at = FIRST_RETIRE + k * STEP_CYCLES;
        if at >= limit_cycles {
            break;
        }
        ocr += step;
        if ocr == 0 || ocr == 255 {
            step = -step;
        }
        out.push((at, u8::try_from(ocr).unwrap()));
    }
    out
}

/// CPU cycle of timer clock `tick` (1-based). The `STS TCCR1B` retires at
/// cycle 8, and the first /64 tick is cycle 64 (docs/engine/avr.md, parameters 1 and 3).
fn tick_cycle(tick: u64) -> u64 {
    DIV * tick
}

/// TOP arrivals before `limit_cycles`. The first is timer clock 255.
fn tops(limit_cycles: u64) -> Vec<u64> {
    (0..)
        .map(|n| tick_cycle(TOP + n * PERIOD))
        .take_while(|&cycle| cycle < limit_cycles)
        .collect()
}

/// OCR1A copied at `top` (§16.9.4).
fn ocr_at(top: u64, writes: &[(u64, u8)]) -> u8 {
    writes
        .iter()
        .rev()
        .find(|&&(at, _)| at < top)
        .map(|&(_, ocr)| ocr)
        .unwrap_or(0)
}

/// Phase correct, non-inverting (Table 16-3).
/// The fall at TOP is Figure 15-7 (§15.7.4).
fn edges(cycle_ps: u64, limit_cycles: u64) -> Vec<(u64, bool)> {
    let writes = writes(limit_cycles);
    let mut level = false;
    let mut edges = Vec::new();
    let mut emit = |cycle: u64, high: bool| {
        if cycle >= limit_cycles || high == level {
            return;
        }
        level = high;
        edges.push((cycle * cycle_ps, high));
    };
    for top in tops(limit_cycles) {
        let ocr = u64::from(ocr_at(top, &writes));
        if ocr == TOP {
            // Figure 15-7 (§15.7.4) is the fall when OCR leaves TOP.
            emit(top, true);
            continue;
        }
        // The fall at TOP. Figure 15-7 (§15.7.4).
        emit(top, false);
        if ocr == 0 {
            continue;
        }
        emit(top + (TOP - ocr) * DIV, true);
        emit(top + (TOP + ocr) * DIV, false);
    }
    edges
}

/// `sim run` and a steps run of the same netlist side by side, so the test costs one run.
#[test]
fn sim_run_writes_pb1_and_the_led_at_the_picoseconds_fade_adds_up_to() {
    let beside = run_beside_steps("atmega-fade");
    assert_eq!(
        serial_csv(&beside),
        b"t_ps,part,usart,byte\n",
        "the USART sends nothing"
    );
    let netlist = &beside.netlist;
    let cycle_ps = cycle_ps(netlist);
    let limit = netlist["analysis"]["transient"]["stop_ps"]
        .as_u64()
        .unwrap()
        / cycle_ps;

    let vcd = logic_vcd(&beside, &["#u1.logic.PB1"]);
    let pb1 = logic_edges(&vcd, "#u1.logic.PB1");
    let edges = edges(cycle_ps, limit);
    assert_eq!(pb1, edges);

    // The turn at 0 stays outside this run. It needs about 3.1 s, double the cost,
    // and the run already starts at OCR1A = 0. The 12 cycles on every path are by
    // construction in fade.S. This run checks that the step changes sign, not the
    // top branch's length.
    let mut duty: Vec<u64> = (5..=250).step_by(5).collect();
    duty.extend([255, 250]);
    assert_eq!(
        ocr_steps(&pb1, cycle_ps),
        duty,
        "observed pulses read OCR1A up by 5 to 255, then 250"
    );

    assert_led_follows(&beside, &pb1, driven_ps(netlist));
}

fn ocr_steps(edges: &[(u64, bool)], cycle_ps: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut rise_at = None;
    for &(time, high) in edges {
        if high {
            assert!(
                rise_at.is_none(),
                "PB1 rises at {time} ps while it is already high"
            );
            rise_at = Some(time);
            continue;
        }
        assert!(rise_at.is_some(), "PB1 falls at {time} ps while it is low");
        let rise = rise_at.take().unwrap();
        let high_cycles = (time - rise) / cycle_ps;
        let period_cycles = PERIOD * DIV;
        let holds_through_top = high_cycles.is_multiple_of(period_cycles);
        let ocr = if holds_through_top {
            TOP
        } else {
            assert_eq!(
                high_cycles % (2 * DIV),
                0,
                "high time is 2 × OCR1A timer clocks"
            );
            high_cycles / (2 * DIV)
        };
        if out.last().copied() != Some(ocr) {
            out.push(ocr);
        }
    }
    out
}
