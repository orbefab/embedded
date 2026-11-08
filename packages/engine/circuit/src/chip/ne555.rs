//! The NE555 as its ladder, three comparators and TI SLFS022K Table 6-1. There is no
//! propagation delay and no input current.

use orbe_engine_core::logic::Level;
use orbe_engine_core::netlist::Ne555;

use crate::mna::{Node, Stamp};
use crate::watch::{Signal, Watch};

/// OUT drives `latch`. DISCH is on while it is low.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) latch: Level,
}

/// The datasheet leaves the power-on state open. The operating point settles it.
pub(crate) const POWER_ON: State = State { latch: Level::Low };

/// Watch order. TRIG is high above the lower ladder node, THRES above CONT, and RESET above
/// GND by more than `reset_threshold`.
const TRIG: usize = 0;
const THRES: usize = 1;
const RESET: usize = 2;

pub(crate) const SIGNALS: [&str; 4] = ["out", "reset", "thres", "trig"];

pub(crate) const DC_PATHS: [(usize, usize); 6] = [
    (Ne555::VCC, Ne555::CONT),
    (Ne555::CONT, Ne555::GND),
    (Ne555::VCC, Ne555::GND),
    (Ne555::OUT, Ne555::VCC),
    (Ne555::OUT, Ne555::GND),
    (Ne555::DISCH, Ne555::GND),
];

pub(crate) fn watches(chip: &Ne555, lower: Node) -> Vec<Watch> {
    let pin = |k: usize| Node::from(chip.pins[k]);
    let above = |plus: Node, minus: Node, offset: f64| {
        Watch::comparator(Signal {
            terms: vec![(plus, 1.0), (minus, -1.0)],
            offset,
        })
    };
    let reset = chip.reset_threshold.si.get();
    let watches = vec![
        above(pin(Ne555::TRIG), lower, 0.0),
        above(pin(Ne555::THRES), pin(Ne555::CONT), 0.0),
        above(pin(Ne555::RESET), pin(Ne555::GND), reset),
    ];
    debug_assert_eq!((TRIG, THRES, RESET), (0, 1, 2));
    watches
}

/// The ladder through the internal lower node, the supply resistance, DISCH to GND, and the
/// output stage to the chip's own VCC or GND. The output drops are sources, so they scale.
pub(crate) fn stamp(chip: &Ne555, state: State, stamp: &mut Stamp<'_>) {
    let pin = |k: usize| chip.pins[k];
    let (vcc, gnd, out) = (pin(Ne555::VCC), pin(Ne555::GND), pin(Ne555::OUT));
    let lower = stamp.internal(0);
    let ladder = 1.0 / chip.ladder_resistance.si.get();
    stamp.conductance(vcc, pin(Ne555::CONT), ladder);
    stamp.conductance(pin(Ne555::CONT), lower, ladder);
    stamp.conductance(lower, gnd, ladder);
    stamp.conductance(vcc, gnd, 1.0 / chip.supply_resistance.si.get());
    let discharge = match state.latch {
        Level::High => &chip.discharge.off_resistance,
        Level::Low => &chip.discharge.on_resistance,
    };
    stamp.conductance(pin(Ne555::DISCH), gnd, 1.0 / discharge.si.get());
    let scale = stamp.conditions.source_scale;
    let output = &chip.output;
    match state.latch {
        Level::High => {
            let g = 1.0 / output.high_resistance.si.get();
            stamp.conductance(out, vcc, g);
            stamp.current(out, vcc, output.high_drop.si.get() * g * scale);
        }
        Level::Low => {
            let g = 1.0 / output.low_resistance.si.get();
            stamp.conductance(out, gnd, g);
            stamp.current(gnd, out, output.low_drop.si.get() * g * scale);
        }
    }
}

/// SLFS022K Table 6-1. RESET overrides TRIG, which overrides THRES. Level-sensitive.
pub(crate) fn react(state: State, levels: &[bool]) -> State {
    let latch = if !levels[RESET] {
        Level::Low
    } else if !levels[TRIG] {
        Level::High
    } else if levels[THRES] {
        Level::Low
    } else {
        state.latch
    };
    State { latch }
}

/// OUT shows the level the chip drives. An input shows the level its comparator reads.
pub(crate) fn levels(state: State, watched: &[bool]) -> [Level; 4] {
    [
        state.latch,
        Level::from_bool(watched[RESET]),
        Level::from_bool(watched[THRES]),
        Level::from_bool(watched[TRIG]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOW: State = State { latch: Level::Low };
    const HIGH: State = State { latch: Level::High };

    /// `[trig, thres, reset]` in watch order.
    fn levels(trig: bool, thres: bool, reset: bool) -> [bool; 3] {
        let mut levels = [false; 3];
        levels[TRIG] = trig;
        levels[THRES] = thres;
        levels[RESET] = reset;
        levels
    }

    #[test]
    fn reset_low_resets_whatever_trig_and_thres_read() {
        for state in [LOW, HIGH] {
            for trig in [false, true] {
                for thres in [false, true] {
                    assert_eq!(react(state, &levels(trig, thres, false)), LOW);
                }
            }
        }
    }

    #[test]
    fn trig_below_its_threshold_sets_whatever_thres_reads() {
        for state in [LOW, HIGH] {
            for thres in [false, true] {
                assert_eq!(react(state, &levels(false, thres, true)), HIGH);
            }
        }
    }

    #[test]
    fn thres_above_its_threshold_resets_once_trig_is_above_its_own() {
        for state in [LOW, HIGH] {
            assert_eq!(react(state, &levels(true, true, true)), LOW);
        }
    }

    #[test]
    fn both_inside_their_thresholds_keep_the_latch() {
        for state in [LOW, HIGH] {
            assert_eq!(react(state, &levels(true, false, true)), state);
        }
    }

    #[test]
    fn signals_are_in_name_order_and_inputs_show_their_comparator() {
        assert!(SIGNALS.is_sorted());
        assert_eq!(
            super::levels(HIGH, &levels(true, false, true)),
            [Level::High, Level::High, Level::Low, Level::High]
        );
    }
}
