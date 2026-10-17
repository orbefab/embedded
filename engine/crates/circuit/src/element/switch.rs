use sim_core::netlist::Switch;
use sim_core::time::Time;

use crate::mna::{Probe, Stamp};

/// Siemens on the interval that ends at `at`.
fn conductance(switch: &Switch, at: Time) -> f64 {
    1.0 / switch.resistance(switch.schedule.before(at)).si.get()
}

pub(super) fn stamp(switch: &Switch, stamp: &mut Stamp<'_>) {
    let g = conductance(switch, stamp.conditions.at);
    stamp.conductance(switch.pins[0], switch.pins[1], g);
}

pub(super) fn current(switch: &Switch, probe: &Probe<'_>, at: Time) -> f64 {
    (probe.v(switch.pins[0]) - probe.v(switch.pins[1])) * conductance(switch, at)
}
