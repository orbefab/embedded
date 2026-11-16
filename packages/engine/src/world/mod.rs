//! The room a circuit sits in: a closed table of physical quantities, the schedule a netlist
//! gives each one it declares, and the parameters that read them. `docs/engine/netlist.md`
//! says what each means in a netlist.

mod curve;
pub(crate) mod json;

use orbe_model::Time;
use serde::{Deserialize, Serialize};

use crate::finite::Finite;
use crate::netlist::{Event, Schedule, Value};
use crate::newton::Side;

pub use curve::{Curve, Driven, Param};

/// A world quantity. The order is the table's, and every list of quantities follows it. Its
/// serde form is its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    Temperature,
    Humidity,
    Light,
}

/// One quantity's row of the table: its name in a netlist and a ref, the unit its values are
/// in, the range a value must sit in, both ends included, and the value a scene holds when it
/// sets none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

const ROWS: [Row; 3] = [
    Row {
        name: "temperature",
        unit: "degC",
        min: -40.0,
        max: 125.0,
        default: 25.0,
    },
    Row {
        name: "humidity",
        unit: "%RH",
        min: 0.0,
        max: 100.0,
        default: 50.0,
    },
    Row {
        name: "light",
        unit: "lx",
        min: 0.0,
        max: 100_000.0,
        default: 300.0,
    },
];

impl Quantity {
    /// Every quantity, in table order.
    pub const ALL: [Quantity; 3] = [Quantity::Temperature, Quantity::Humidity, Quantity::Light];

    pub fn row(self) -> &'static Row {
        &ROWS[self as usize]
    }

    pub fn name(self) -> &'static str {
        self.row().name
    }

    pub fn unit(self) -> &'static str {
        self.row().unit
    }

    /// The quantity named `name` in the table.
    pub fn parse(name: &str) -> Option<Quantity> {
        Quantity::ALL.into_iter().find(|q| q.name() == name)
    }

    /// Whether `value` is inside the quantity's range.
    pub fn holds(self, value: f64) -> bool {
        let row = self.row();
        (row.min..=row.max).contains(&value)
    }
}

/// The schedule of each quantity a netlist declares. A quantity it does not declare has none,
/// and no part reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    declared: [Option<Schedule<Value>>; 3],
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
    fn every_row_holds_its_default_and_names_no_part_quantity() {
        for quantity in Quantity::ALL {
            let row = quantity.row();
            assert!(row.min < row.max, "{}", row.name);
            assert!(quantity.holds(row.default), "{}", row.name);
            assert_eq!(Quantity::parse(row.name), Some(quantity));
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
