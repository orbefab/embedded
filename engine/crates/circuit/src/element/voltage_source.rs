use sim_core::netlist::VoltageSource;

use crate::mna::{Probe, Stamp};

pub(super) fn stamp(source: &VoltageSource, stamp: &mut Stamp<'_>) {
    stamp.voltage_source(source.pins[0], source.pins[1], 0, source.voltage.si.get());
}

pub(super) fn current(probe: &Probe<'_>) -> f64 {
    probe.branch(0)
}
