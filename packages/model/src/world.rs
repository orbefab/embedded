/// A world quantity: a property of the room a circuit sits in, which a scene declares and a
/// device reads. The order is the table's, and every list of quantities follows it. The table
/// is closed: a new quantity is a new row here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Quantity {
    Temperature,
    Humidity,
    Light,
    Pressure,
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

const ROWS: [Row; 4] = [
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
    Row {
        name: "pressure",
        unit: "Pa",
        min: 30_000.0,
        max: 110_000.0,
        default: 101_325.0,
    },
];

impl Quantity {
    /// Every quantity, in table order.
    pub const ALL: [Quantity; 4] = [
        Quantity::Temperature,
        Quantity::Humidity,
        Quantity::Light,
        Quantity::Pressure,
    ];

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

/// The value of each quantity a device reads at one call's time, in the quantity's unit. The
/// host fills one for every call on a device that declares a quantity in
/// [`Device::WORLD`](crate::Device::WORLD).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Readings([Option<f64>; Quantity::ALL.len()]);

impl Readings {
    /// No quantity read.
    pub const NONE: Readings = Readings([None; Quantity::ALL.len()]);

    /// These readings, and `quantity` at `value`.
    pub const fn with(mut self, quantity: Quantity, value: f64) -> Readings {
        self.0[quantity as usize] = Some(value);
        self
    }

    pub const fn get(&self, quantity: Quantity) -> Option<f64> {
        self.0[quantity as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_holds_its_default_and_parses_by_name() {
        for quantity in Quantity::ALL {
            let row = quantity.row();
            assert!(row.min < row.max, "{}", row.name);
            assert!(quantity.holds(row.default), "{}", row.name);
            assert_eq!(Quantity::parse(row.name), Some(quantity));
        }
    }

    #[test]
    fn a_reading_reads_back_only_for_its_quantity() {
        let readings = Readings::NONE.with(Quantity::Humidity, 41.5);
        assert_eq!(readings.get(Quantity::Humidity), Some(41.5));
        assert_eq!(readings.get(Quantity::Temperature), None);
    }
}
