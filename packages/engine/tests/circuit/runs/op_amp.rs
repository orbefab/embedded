//! The `op-amp-*` examples run the LMV358's values, TI SLOS263Y p7 section 6.8 at VCC 5 V.
//! Each check holds the run to a band from that table and to the law in
//! `docs/engine/netlist.md`. The circuits themselves are [`super::op_amp_sheet`].

use super::op_amp_sheet::{LoadBand, Sheet, Values};

fn lmv358() -> Sheet {
    Sheet {
        values: Values {
            gain_bandwidth: 1e6,
            slew_rate: 1e6,
            open_loop_gain: 1e5,
            input_bias_current: 1.5e-8,
            swing_high: 0.0025,
            swing_low: 0.051,
            output_resistance: 30.0,
            short_circuit_current: 0.04,
            supply_current: 1.05e-4,
        },
        write_values: false,
        swing_to_ground: false,
        swing_return: 2.5,
        loads: &[
            LoadBand {
                ohm: 10e3,
                voh_min: 4.900,
                vol_max: 0.180,
                high: "#net.h10k",
                low: "#net.l10k",
            },
            LoadBand {
                ohm: 2e3,
                voh_min: 4.700,
                vol_max: 0.300,
                high: "#net.h2k",
                low: "#net.l2k",
            },
        ],
        slew_typical: 1.0,
        slew_width: 0.20,
        gain_width_10k: 0.02,
        gain_width_200k: 0.10,
        isc_min: 0.036,
        isc_max: 0.044,
        ib_max: 250e-9,
        edges_of_high_stop: false,
    }
}

/// Circuit 1. A follower whose `in_p` sits behind 1 Mohm from 2.5 V. The bias current leaves `in_p`,
/// so `in_p` sits 15 mV above 2.5 V. The datasheet bounds it by 250 nA. The node gmin of
/// 1e-12 S takes `V(in_p)` times that from it, so 1 Mohm carries the rest. The supply carries
/// one channel's supply current, both bias currents, the gmin on `vcc`, and the share of the
/// output current the law draws from `vcc`. `in_n` is the output net, so its bias current
/// feeds the 10 kohm load too, and the output sources the rest of the load's current. Newton
/// settles that sum to `RELTOL` of it plus `ABSTOL`.
#[test]
fn the_bias_current_leaves_each_input() {
    super::op_amp_sheet::bias(&lmv358());
}

/// Circuit 1, second channel. `u2` is a follower on its own 5 V supply whose `in_p` sits behind
/// 1 Mohm from 4.95 V, within the 0.8 V headroom of `vcc`. The bias gate is
/// `sg((5 - 0.8 - 4.95) / 0.05)`, about 3e-7, so the bias current falls below a thousandth of
/// `Ib`, and 1 Mohm carries little more than the node gmin.
#[test]
fn the_bias_current_turns_off_near_the_positive_rail() {
    super::op_amp_sheet::bias_off(&lmv358());
}

/// Circuit 2. Overdriven open loop into 10 kohm and 2 kohm to 2.5 V. The datasheet's bands are VOH at
/// least 4.900 V and VOL at most 0.180 V into 10 kohm, and 4.700 V and 0.300 V into 2 kohm.
#[test]
fn the_output_swings_to_its_rails() {
    super::op_amp_sheet::swing(&lmv358());
}

/// Circuit 3. A follower stepped from 0.5 V to 4.0 V at 10 us and back at 30 us. The datasheet's slew
/// rate is 1 V/us, and the band is 0.8 to 1.2 V/us. The integrator slews at `slew_rate`, and
/// the output follows it through `Ro` into 10 kohm, so it moves at `SR / (1 + Ro / 10 kohm)`.
#[test]
fn the_output_slews_at_the_slew_rate() {
    super::op_amp_sheet::slew(&lmv358());
}

/// Circuit 4. Non-inverting gain 10, 10 kohm and 90 kohm, driven by a 64-level staircase of a 10 mV
/// sine on 0.25 V for 12 periods. The gain is the ratio of the output's and the input's
/// amplitudes at `f` over the last 6 periods, so the staircase's droop cancels. One pole with
/// DC gain `A0` and unity-gain frequency `GBW` gives `A0 / sqrt((1 + A0 / 10)^2 + (f A0 /
/// GBW)^2)`. The bands are 2% at 10 kHz and 10% at 200 kHz around the single pole's value with
/// an infinite DC gain.
#[test]
fn the_closed_loop_gain_falls_with_one_pole() {
    super::op_amp_sheet::gain(&lmv358());
}

/// Circuit 5. The output held at 0 V while driven high, and at 5 V while driven low. The datasheet's
/// short-circuit current is 40 mA typical each way, and the band is 36 to 44 mA. The law
/// holds it a little past `Isc`, where its slope meets the swing. The sourced current comes
/// from `vcc` and the sunk current goes to `gnd`.
#[test]
fn the_output_current_is_limited_both_ways() {
    super::op_amp_sheet::short(&lmv358());
}

/// Circuit 6. Open-loop comparators against 2.5 V, run on every accepted step. A 1 mA current into
/// 2 uF ramps the input at 500 V/s from 1 ms to 11 ms and back down to 21 ms. The square is
/// 0 V and 5 V at 10 kHz. The hysteresis circuit puts the ramp through 10 kohm into `in_p`,
/// with 100 kohm from the output, so `in_p` reaches 2.5 V when the ramp is
/// `2.75 - 0.1 * V(out)`. Each run has no diagnostic, no accepted step shorter than 100 ps, and
/// no transition over 2000 steps.
///
/// Each output crosses 2.5 V after `in_p` does by what one pole allows. The integrator waits
/// past a rail, at most where the clamp carries the whole slew current,
/// `0.1 + 0.01 * ln(exp(SR / 1e5) - 1)`, 0.2 V at 1 V/us. On the square that is a slew at
/// 1 V/us. On the ramp the difference grows at 500 V/s and stays in the linear range. With
/// hysteresis `in_p` ramps above 400 V/s, and the output's rise feeds it, so the delay is under
/// the ramp delay at 400 V/s and over the slew.
#[test]
fn a_comparator_switches_in_few_steps_at_its_threshold() {
    super::op_amp_sheet::comparators(&lmv358());
}
