use sim_core::netlist::{Contact, Switch};
use sim_core::time::Time;

use crate::mna::{Probe, Stamp};
use crate::newton::History;

fn conductance(switch: &Switch, contact: Contact) -> f64 {
    1.0 / switch.resistance(contact).si.get()
}

/// A restart takes the contact after the edge at `at`. Every other solve takes the one before it.
pub(super) fn stamp(switch: &Switch, stamp: &mut Stamp<'_>) {
    let at = stamp.conditions.at;
    let contact = match stamp.conditions.history {
        History::Restart { .. } => switch.schedule.after(at),
        History::Start | History::Step(_) => switch.schedule.before(at),
    };
    stamp.conductance(switch.pins[0], switch.pins[1], conductance(switch, contact));
}

/// On the interval that ends at `at`.
pub(super) fn current(switch: &Switch, probe: &Probe<'_>, at: Time) -> f64 {
    let g = conductance(switch, switch.schedule.before(at));
    (probe.v(switch.pins[0]) - probe.v(switch.pins[1])) * g
}
