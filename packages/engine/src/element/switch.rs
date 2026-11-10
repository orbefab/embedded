use crate::netlist::{Contact, Switch};
use crate::time::Time;

use crate::mna::{Probe, Stamp};
use crate::newton::Side;

fn conductance(switch: &Switch, contact: Contact) -> f64 {
    1.0 / switch.resistance(contact).si.get()
}

/// A restart takes the contact after the edge at `at`. Every other solve takes the one before it.
pub(super) fn stamp(switch: &Switch, stamp: &mut Stamp<'_>) {
    let contact = stamp
        .conditions
        .side()
        .contact(&switch.schedule, stamp.conditions.at);
    stamp.conductance(switch.pins[0], switch.pins[1], conductance(switch, contact));
}

pub(super) fn current(switch: &Switch, probe: &Probe<'_>, at: Time, side: Side) -> f64 {
    let g = conductance(switch, side.contact(&switch.schedule, at));
    (probe.v(switch.pins[0]) - probe.v(switch.pins[1])) * g
}
