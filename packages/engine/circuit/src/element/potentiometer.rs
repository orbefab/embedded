use orbe_engine_core::netlist::Potentiometer;

use crate::mna::Stamp;

pub(super) fn stamp(pot: &Potentiometer, stamp: &mut Stamp<'_>) {
    let ohms = pot.resistance.si.get();
    let position = stamp
        .conditions
        .side()
        .read(&pot.position, stamp.conditions.at)
        .si
        .get();
    let [a, wiper, b] = pot.pins;
    stamp.conductance(a, wiper, 1.0 / (position * ohms));
    stamp.conductance(wiper, b, 1.0 / ((1.0 - position) * ohms));
}
