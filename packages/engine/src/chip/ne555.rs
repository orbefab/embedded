//! The NE555 as its ladder, three comparators and TI SLFS022K Table 6-1. There is no
//! propagation delay and no input current.

use crate::diagnostic::Diagnostic;
use crate::netlist::NetId;
use crate::netlist::Value;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireValue;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_value;
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use serde::Deserialize;

use crate::logic::Level;

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

/// An NE555 timer, behavioural. Pins in package order, TI SLFS022K's pin functions.
///
/// Three equal resistors run VCC to CONT to a lower node to GND. THRES compares with CONT,
/// TRIG with the lower node, and RESET with GND plus `reset_threshold`. The latch follows
/// SLFS022K Table 6-1: RESET low resets it, then TRIG low sets it, then THRES high resets it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ne555 {
    pub pins: [NetId; 8],
    /// Ohm. Each of the three ladder resistors.
    pub ladder_resistance: Value,
    /// Volt, greater than 0. RESET more than this above GND lets the latch run.
    pub reset_threshold: Value,
    pub discharge: Discharge,
    /// Ohm. VCC to GND, the supply current the ladder does not draw.
    pub supply_resistance: Value,
    pub output: OutputStage,
}

impl Ne555 {
    pub const PINS: [&'static str; 8] = [
        "gnd", "trig", "out", "reset", "cont", "thres", "disch", "vcc",
    ];
    pub const GND: usize = 0;
    pub const TRIG: usize = 1;
    pub const OUT: usize = 2;
    pub const RESET: usize = 3;
    pub const CONT: usize = 4;
    pub const THRES: usize = 5;
    pub const DISCH: usize = 6;
    pub const VCC: usize = 7;
}

/// DISCH to GND: `on_resistance` while the latch is reset, `off_resistance` while it is set.
#[derive(Clone, Debug, PartialEq)]
pub struct Discharge {
    /// Ohm.
    pub on_resistance: Value,
    /// Ohm.
    pub off_resistance: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireNe555 {
    pub(crate) name: String,
    pins: PinMap,
    ladder_resistance: WireValue,
    reset_threshold: WireValue,
    discharge: WireDischarge,
    supply_resistance: WireValue,
    output: WireOutputStage,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireDischarge {
    on_resistance: WireValue,
    off_resistance: WireValue,
}

pub(crate) fn resolve_ne555(ctx: ParseCtx<'_>, wire: WireNe555) -> Option<Ne555> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    let pins = resolve_pins(name, Ne555::PINS, &wire.pins.0, nets, out);
    let ladder = ohm("ladder_resistance", wire.ladder_resistance, out);
    let reset = resolve_value(
        part_ref,
        "reset_threshold",
        "V",
        true,
        wire.reset_threshold,
        out,
    );
    let on = ohm("discharge.on_resistance", wire.discharge.on_resistance, out);
    let off = ohm(
        "discharge.off_resistance",
        wire.discharge.off_resistance,
        out,
    );
    let supply = ohm("supply_resistance", wire.supply_resistance, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    Some(Ne555 {
        pins: pins?,
        ladder_resistance: ladder?,
        reset_threshold: reset?,
        discharge: Discharge {
            on_resistance: on?,
            off_resistance: off?,
        },
        supply_resistance: supply?,
        output: output?,
    })
}
