use crate::netlist::CurrentSource;
use crate::time::Time;

use crate::mna::Stamp;
use crate::newton::Side;

/// The current leaves at `p`, so it flows through the part from `n` to `p`. `Stamp::current`
/// does not scale, so the stamp applies `source_scale` itself.
pub(super) fn stamp(source: &CurrentSource, stamp: &mut Stamp<'_>) {
    let amps = stamp.conditions.level(&source.current) * stamp.conditions.source_scale;
    stamp.current(source.pins[1], source.pins[0], amps);
}

/// Into pin `p`.
pub(super) fn current(source: &CurrentSource, at: Time, side: Side) -> f64 {
    -side.level(&source.current, at)
}
