use orbe_engine_core::netlist::VoltageSource;

use crate::mna::{Probe, Stamp};

pub(super) fn stamp(source: &VoltageSource, stamp: &mut Stamp<'_>) {
    let volts = stamp.conditions.level(&source.voltage);
    stamp.voltage_source(source.pins[0], source.pins[1], 0, volts);
}

pub(super) fn current(probe: &Probe<'_>) -> f64 {
    probe.branch(0)
}
