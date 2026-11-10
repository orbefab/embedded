use crate::netlist::Resistor;

use crate::mna::{Probe, Stamp};

pub(super) fn stamp(resistor: &Resistor, stamp: &mut Stamp<'_>) {
    let g = 1.0 / resistor.resistance.si.get();
    stamp.conductance(resistor.pins[0], resistor.pins[1], g);
}

pub(super) fn current(resistor: &Resistor, probe: &Probe<'_>) -> f64 {
    (probe.v(resistor.pins[0]) - probe.v(resistor.pins[1])) / resistor.resistance.si.get()
}
