//! examples/atmega-blink, `firmware/blink.S` on an ATmega328P at 16 MHz with an LED on PB5,
//! against the firmware's cycle counts and the closed form of the LED's current.

mod common;

use common::atmega::{
    assert_led_follows, chip_matches_fixture, cycle_ps, logic_edges, logic_vcd, run_beside_steps,
};

#[test]
fn the_chip_is_the_tests_fixture_running_blink_hex() {
    chip_matches_fixture("atmega-blink", "firmware/blink.hex");
}

/// `SBI DDRB,5` takes two cycles (DS40002198B, AVRe column).
fn driven_ps(netlist: &serde_json::Value) -> u64 {
    2 * cycle_ps(netlist)
}

/// `(t_ps, high)` of every PB5 edge before `stop_ps`. In `blink.S`, `SBI DDRB,5` and the first
/// `SBI PINB,5` take two cycles each, and a pass of its loop takes
/// 2 + 3 + 5 * 1,599,998 - 1 + 4 + 2 (DS40002198B, AVRe column).
fn edges(netlist: &serde_json::Value) -> Vec<(u64, bool)> {
    let cycle_ps = cycle_ps(netlist);
    let stop_ps = netlist["analysis"]["transient"]["stop_ps"]
        .as_u64()
        .unwrap();
    let (first, pass) = (2 + 2, 2 + 3 + 5 * 1_599_998 - 1 + 4 + 2);
    (0..)
        .map(|k| ((first + pass * k) * cycle_ps, k % 2 == 0))
        .take_while(|&(at, _)| at < stop_ps)
        .collect()
}

/// `sim run` and a steps run of the same netlist side by side, so the test costs one run.
#[test]
fn sim_run_writes_pb5_and_the_led_at_the_picoseconds_blink_adds_up_to() {
    let beside = run_beside_steps("atmega-blink");
    let netlist = &beside.netlist;

    let vcd = logic_vcd(&beside, &["#u1.logic.PB5"]);
    let pb5 = logic_edges(&vcd, "#u1.logic.PB5");
    let edges = edges(netlist);
    assert_eq!(edges.len(), 8);
    assert_eq!(pb5, edges);
    assert_led_follows(&beside, &pb5, driven_ps(netlist));
}
