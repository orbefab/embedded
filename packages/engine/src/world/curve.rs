use orbe_model::Time;

use super::Quantity;
use crate::finite::Finite;
use crate::netlist::{Schedule, Value};
use crate::newton::Side;

/// Kelvin at 0 degC.
const ZERO_C: f64 = 273.15;

/// A part's field: one value, or a world quantity read through a curve.
#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    Fixed(Value),
    World(Box<Driven>),
}

impl Param {
    /// The field's SI value on `side` of `at`.
    #[inline]
    pub(crate) fn read(&self, side: Side, at: Time) -> f64 {
        match self {
            Param::Fixed(value) => value.si.get(),
            Param::World(driven) => side.read(&driven.values, at).get(),
        }
    }

    /// The fixed value. `None` for a field the world drives.
    pub fn fixed(&self) -> Option<&Value> {
        match self {
            Param::Fixed(value) => Some(value),
            Param::World(_) => None,
        }
    }

    pub(crate) fn driven_mut(&mut self) -> Option<&mut Driven> {
        match self {
            Param::Fixed(_) => None,
            Param::World(driven) => Some(driven),
        }
    }
}

/// A field that follows `quantity` through `curve`.
#[derive(Clone, Debug, PartialEq)]
pub struct Driven {
    pub quantity: Quantity,
    pub curve: Curve,
    /// `curve` on the world's schedule of `quantity`, rebuilt whenever that schedule grows.
    pub(crate) values: Schedule<Finite>,
}

impl Driven {
    pub(crate) fn new(quantity: Quantity, curve: Curve, schedule: &Schedule<Value>) -> Driven {
        Driven {
            quantity,
            values: super::evaluate(&curve, schedule),
            curve,
        }
    }

    pub(crate) fn follow(&mut self, schedule: &Schedule<Value>) {
        self.values = super::evaluate(&self.curve, schedule);
    }

    /// The field's value on each interval of the world's schedule, `initial` first.
    pub fn values(&self) -> &Schedule<Finite> {
        &self.values
    }
}

/// How a field follows a world value `w`. Each curve is monotonic in `w`, so over a range its
/// values lie between its values at the two ends. `x` is in the quantity's unit, and `y` and
/// `max` in the field's.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// `y * (w / x)^exponent`, at most `max`. A light-dependent resistor, whose resistance is
    /// `max` in the dark.
    Power {
        x: Value,
        y: Value,
        exponent: Value,
        max: Value,
    },
    /// `y * exp(beta * (1 / T - 1 / T0))`, with `T` and `T0` the kelvin of `w` and `x` in degC.
    /// An NTC thermistor's B equation. `beta` is in kelvin.
    Beta { x: Value, y: Value, beta: Value },
    /// The line through `(x0, y0)` and `(x1, y1)`. A sensor with a linear output, such as an
    /// LM35's 10 mV per degC.
    Linear {
        x0: Value,
        y0: Value,
        x1: Value,
        y1: Value,
    },
}

impl Curve {
    /// The field's value at world value `w`.
    pub fn at(&self, w: f64) -> f64 {
        match self {
            Curve::Power {
                x,
                y,
                exponent,
                max,
            } => {
                let y = y.si.get() * libm::pow(w / x.si.get(), exponent.si.get());
                y.min(max.si.get())
            }
            Curve::Beta { x, y, beta } => {
                let inverse = 1.0 / (w + ZERO_C) - 1.0 / (x.si.get() + ZERO_C);
                y.si.get() * libm::exp(beta.si.get() * inverse)
            }
            Curve::Linear { x0, y0, x1, y1 } => {
                let (x0, y0) = (x0.si.get(), y0.si.get());
                let slope = (y1.si.get() - y0) / (x1.si.get() - x0);
                y0 + slope * (w - x0)
            }
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Curve::Power { .. } => "power",
            Curve::Beta { .. } => "beta",
            Curve::Linear { .. } => "linear",
        }
    }
}
