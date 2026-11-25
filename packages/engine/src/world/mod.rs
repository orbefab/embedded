//! The room a circuit sits in: a closed table of physical quantities, the schedule a netlist
//! gives each one it declares, and the parameters that read them. `docs/engine/netlist.md`
//! says what each means in a netlist.

mod curve;
pub(crate) mod json;

use orbe_model::{Readings, Time};

use crate::finite::Finite;
use crate::netlist::{Event, Schedule, Value};
use crate::newton::Side;

pub use curve::{Curve, Driven, Param};
pub use orbe_model::{Quantity, Row};

/// The schedule of each quantity a netlist declares. A quantity it does not declare has none,
/// and no part reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    declared: [Option<Schedule<Value>>; Quantity::ALL.len()],
}

impl World {
    /// Each declared quantity with its schedule, in table order.
    pub fn declared(&self) -> impl Iterator<Item = (Quantity, &Schedule<Value>)> {
        Quantity::ALL
            .into_iter()
            .zip(&self.declared)
            .filter_map(|(q, schedule)| Some((q, schedule.as_ref()?)))
    }

    pub fn schedule(&self, quantity: Quantity) -> Option<&Schedule<Value>> {
        self.declared[quantity as usize].as_ref()
    }

    /// The value of a declared quantity on the interval that ends at `at`: after every change
    /// strictly before `at`. A device and a row at `at` read this side.
    pub fn before(&self, quantity: Quantity, at: Time) -> Option<f64> {
        self.read(quantity, Side::Before, at)
    }

    pub(crate) fn read(&self, quantity: Quantity, side: Side, at: Time) -> Option<f64> {
        let schedule = self.schedule(quantity)?;
        Some(side.read(schedule, at).si.get())
    }

    /// The next change of `quantity` strictly after `at`. `None` when it is not declared.
    pub(crate) fn next_change_after(&self, quantity: Quantity, at: Time) -> Option<Time> {
        self.schedule(quantity)?.next_change_after(at)
    }

    /// What a device that reads `reads` sees at `at`: each one [`before`](Self::before) `at`.
    /// Parse checks that the netlist declares every quantity a device part reads.
    pub(crate) fn readings(&self, reads: &[Quantity], at: Time) -> Readings {
        reads.iter().fold(Readings::NONE, |readings, &quantity| {
            let value = self
                .before(quantity, at)
                .expect("parse checks a device's quantities are declared");
            readings.with(quantity, value)
        })
    }

    /// Whether any of `reads` changes at `at`.
    pub(crate) fn changes_at(&self, reads: &[Quantity], at: Time) -> bool {
        let Some(just_before) = at.as_ps().checked_sub(1) else {
            return false;
        };
        reads.iter().any(|&quantity| {
            self.next_change_after(quantity, Time::from_ps(just_before)) == Some(at)
        })
    }

    pub(crate) fn declare(&mut self, quantity: Quantity, schedule: Schedule<Value>) {
        self.declared[quantity as usize] = Some(schedule);
    }

    /// Appends a change of a declared quantity at `at`, a time `Schedule::earliest` returned
    /// for it, to a value in its range that differs from the one after its last change.
    pub(crate) fn append(&mut self, quantity: Quantity, at: Time, level: Finite) {
        let schedule = self.declared[quantity as usize]
            .as_mut()
            .expect("an appended quantity is declared");
        debug_assert!(quantity.holds(level.get()));
        schedule.append_level(at, level);
    }

    /// Drops events of a declared quantity at or after `from`, and returns how many.
    pub(crate) fn drop_from(&mut self, quantity: Quantity, from: Time) -> usize {
        self.declared[quantity as usize]
            .as_mut()
            .expect("a dropped quantity is declared")
            .drop_from(from)
    }

    pub fn is_empty(&self) -> bool {
        self.declared.iter().all(Option::is_none)
    }
}

/// `curve` on `schedule`: its value on each interval of the schedule.
pub(crate) fn evaluate(curve: &Curve, schedule: &Schedule<Value>) -> Schedule<Finite> {
    let at = |value: &Value| {
        Finite::new(curve.at(value.si.get()))
            .expect("parse checks a curve is finite over its range")
    };
    Schedule {
        initial: at(&schedule.initial),
        events: schedule
            .events
            .iter()
            .map(|event| Event {
                at: event.at,
                to: at(&event.to),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::Quantity;
    use crate::name::Ref;

    #[test]
    fn every_quantity_names_no_part_quantity() {
        for quantity in Quantity::ALL {
            let row = quantity.row();
            let world: Ref = format!("#world.{}", row.name).parse().unwrap();
            assert_eq!(world, Ref::World(quantity));
            assert!(
                format!("#r1.{}", row.name).parse::<Ref>().is_err(),
                "`{}` is also a part quantity",
                row.name
            );
        }
    }
}
