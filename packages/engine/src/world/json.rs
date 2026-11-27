//! The netlist's `world` and a world-driven field, from their wire form.

use std::borrow::Cow;

use serde::Deserialize;
use serde::de::{self, Deserializer};

use super::{Curve, Driven, Param, Quantity, World};
use crate::diagnostic::{Diagnostic, ValueRule, WorldRule};
use crate::finite::Finite;
use crate::name::Ref;
use crate::netlist::json::waveform::{ObjectVisitor, WirePosition};
use crate::netlist::json::{FRACTION_UNIT, WireValue, resolve_schedule, resolve_value};
use crate::netlist::{Schedule, Value};

/// The top-level `world`: each quantity by name, a value object or `steps`. A repeated name is
/// `netlist.syntax`.
pub(crate) struct WireWorld(Vec<(String, WirePosition)>);

impl<'de> Deserialize<'de> for WireWorld {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let object = deserializer.deserialize_map(ObjectVisitor)?;
        object
            .into_iter()
            .map(|(name, value)| {
                WirePosition::deserialize(value)
                    .map(|wire| (name.clone(), wire))
                    .map_err(|err| de::Error::custom(format!("world.{name}: {err}")))
            })
            .collect::<Result<_, _>>()
            .map(WireWorld)
    }
}

/// A field that holds a value object, or `{"kind": "world", ...}`.
pub(crate) enum WireParam {
    Value(WireValue),
    World(Box<WireDriven>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireDriven {
    quantity: String,
    curve: WireCurve,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorldOnly {
    World(WireDriven),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireCurve {
    Power {
        x: WireValue,
        y: WireValue,
        exponent: WireValue,
        max: WireValue,
    },
    Beta {
        x: WireValue,
        y: WireValue,
        beta: WireValue,
    },
    SteinhartHart {
        y: WireValue,
        a: WireValue,
        b: WireValue,
        c: WireValue,
        d: WireValue,
    },
    Linear {
        x0: WireValue,
        y0: WireValue,
        x1: WireValue,
        y1: WireValue,
    },
    Step {
        x: WireValue,
        below: WireValue,
        above: WireValue,
    },
}

impl<'de> Deserialize<'de> for WireParam {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let object = deserializer.deserialize_map(ObjectVisitor)?;
        let shaped = object.contains_key("kind");
        let object = serde_json::Value::Object(object);
        if shaped {
            WorldOnly::deserialize(object)
                .map(|WorldOnly::World(driven)| WireParam::World(Box::new(driven)))
        } else {
            WireValue::deserialize(object).map(WireParam::Value)
        }
        .map_err(de::Error::custom)
    }
}

/// The declared quantities. An unknown name is `netlist.world`. A value outside its quantity's
/// range is `netlist.value` with rule `range`, and an event that breaks a schedule rule is
/// `netlist.schedule`, each on `#world.<quantity>`.
pub(crate) fn resolve_world(wire: Option<WireWorld>, out: &mut Vec<Diagnostic>) -> World {
    let mut world = World::default();
    for (name, wire) in wire.map(|WireWorld(list)| list).unwrap_or_default() {
        let Some(quantity) = Quantity::parse(&name) else {
            out.push(Diagnostic::NetlistWorld {
                part: None,
                field: Cow::Borrowed("world"),
                name,
                rule: WorldRule::Unknown,
            });
            continue;
        };
        let at = Ref::World(quantity);
        let schedule = match wire {
            WirePosition::Value(value) => {
                let (ok, initial) = level(&at, quantity, Cow::Borrowed("value"), value, out);
                ok.then_some(Schedule {
                    initial,
                    events: Vec::new(),
                })
            }
            WirePosition::Steps(steps) => resolve_schedule(
                &at,
                "events",
                "initial",
                steps.initial,
                steps.events,
                |field, wire, out| level(&at, quantity, field, wire, out),
                out,
            )
            .map(|(initial, events)| Schedule { initial, events }),
        };
        // A rejected field still declares its quantity, so a part that reads it reports only
        // its own rules. Parse fails on the field's diagnostic, so no run sees the default.
        let schedule = schedule.unwrap_or_else(|| Schedule {
            initial: Value {
                si: Finite::new(quantity.row().default).expect("a row's default is finite"),
            },
            events: Vec::new(),
        });
        world.declare(quantity, schedule);
    }
    world
}

fn level(
    at: &Ref,
    quantity: Quantity,
    field: Cow<'static, str>,
    wire: WireValue,
    out: &mut Vec<Diagnostic>,
) -> (bool, Value) {
    let si = wire.value;
    let resolved = resolve_value(at, field.clone(), quantity.unit(), false, wire, out);
    let in_range = quantity.holds(si.get());
    if !in_range {
        out.push(Diagnostic::NetlistValue {
            part: at.clone(),
            field,
            value: si.get(),
            rule: ValueRule::Range,
        });
    }
    let ok = resolved.is_some() && in_range;
    let value = resolved.unwrap_or(Value { si });
    (ok, value)
}

/// What a world-driven field must hold: its name, its unit, and the rule each of its values
/// keeps, such as a resistance's `broken_positive`.
pub(crate) struct FieldRule {
    pub(crate) field: &'static str,
    pub(crate) unit: &'static str,
    pub(crate) broken: fn(f64) -> Option<ValueRule>,
}

/// A field that is a value, or a world quantity on a curve. A value keeps the field's rule. A
/// curve reads a quantity the netlist declares, and keeps the field's rule at both ends of
/// that quantity's range, so every value a run can set keeps it.
pub(crate) fn resolve_param(
    part: &Ref,
    rule: FieldRule,
    world: &World,
    wire: WireParam,
    out: &mut Vec<Diagnostic>,
) -> Option<Param> {
    let FieldRule {
        field,
        unit,
        broken,
    } = rule;
    let driven = match wire {
        WireParam::Value(value) => {
            let number = value.value.get();
            let resolved = resolve_value(part, field, unit, false, value, out);
            if let Some(rule) = broken(number) {
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: Cow::Borrowed(field),
                    value: number,
                    rule,
                });
                return None;
            }
            return resolved.map(Param::Fixed);
        }
        WireParam::World(driven) => *driven,
    };
    let quantity_field = format!("{field}.quantity");
    let Some(quantity) = Quantity::parse(&driven.quantity) else {
        out.push(Diagnostic::NetlistWorld {
            part: Some(part.clone()),
            field: Cow::Owned(quantity_field),
            name: driven.quantity,
            rule: WorldRule::Unknown,
        });
        return None;
    };
    let curve = resolve_curve(part, field, unit, quantity, driven.curve, out);
    let Some(schedule) = world.schedule(quantity) else {
        out.push(Diagnostic::NetlistWorld {
            part: Some(part.clone()),
            field: Cow::Owned(quantity_field),
            name: driven.quantity,
            rule: WorldRule::Undeclared,
        });
        return None;
    };
    let curve = curve?;
    let row = quantity.row();
    let mut ok = true;
    for end in curve.check_points(row.min, row.max) {
        let value = curve.at(end);
        let rule = if value.is_finite() {
            broken(value)
        } else {
            Some(ValueRule::Finite)
        };
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistValue {
                part: part.clone(),
                field: Cow::Owned(format!(
                    "{field}.curve at {} {}",
                    crate::diagnostic::number_text(end),
                    row.unit
                )),
                value,
                rule,
            });
        }
    }
    ok.then(|| Param::World(Box::new(Driven::new(quantity, curve, schedule))))
}

fn resolve_curve(
    part: &Ref,
    field: &'static str,
    unit: &'static str,
    quantity: Quantity,
    wire: WireCurve,
    out: &mut Vec<Diagnostic>,
) -> Option<Curve> {
    let at = |name: &str| Cow::Owned(format!("{field}.curve.{name}"));
    let mut value = |name: &str, expected: &'static str, wire: WireValue| {
        resolve_value(part, at(name), expected, false, wire, out)
    };
    match wire {
        WireCurve::Power {
            x,
            y,
            exponent,
            max,
        } => {
            let x_number = x.value.get();
            let x = value("x", quantity.unit(), x);
            let y = value("y", unit, y);
            let exponent = value("exponent", FRACTION_UNIT, exponent);
            let max = value("max", unit, max);
            if quantity.row().min < 0.0 {
                out.push(Diagnostic::NetlistWorld {
                    part: Some(part.clone()),
                    field: at("kind"),
                    name: quantity.name().to_string(),
                    rule: WorldRule::NonNegative,
                });
                return None;
            }
            if x_number <= 0.0 {
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: at("x"),
                    value: x_number,
                    rule: ValueRule::Positive,
                });
                return None;
            }
            Some(Curve::Power {
                x: x?,
                y: y?,
                exponent: exponent?,
                max: max?,
            })
        }
        WireCurve::Beta { x, y, beta } => {
            let x_number = x.value.get();
            let x = value("x", quantity.unit(), x);
            let y = value("y", unit, y);
            let beta = value("beta", "K", beta);
            if quantity != Quantity::Temperature {
                out.push(Diagnostic::NetlistWorld {
                    part: Some(part.clone()),
                    field: at("kind"),
                    name: quantity.name().to_string(),
                    rule: WorldRule::Temperature,
                });
                return None;
            }
            if !quantity.holds(x_number) {
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: at("x"),
                    value: x_number,
                    rule: ValueRule::Range,
                });
                return None;
            }
            Some(Curve::Beta {
                x: x?,
                y: y?,
                beta: beta?,
            })
        }
        WireCurve::SteinhartHart { y, a, b, c, d } => {
            let y = value("y", unit, y);
            let a = value("a", FRACTION_UNIT, a);
            let b = value("b", "K", b);
            let c = value("c", "K^2", c);
            let d = value("d", "K^3", d);
            if quantity != Quantity::Temperature {
                out.push(Diagnostic::NetlistWorld {
                    part: Some(part.clone()),
                    field: at("kind"),
                    name: quantity.name().to_string(),
                    rule: WorldRule::Temperature,
                });
                return None;
            }
            Some(Curve::SteinhartHart {
                y: y?,
                a: a?,
                b: b?,
                c: c?,
                d: d?,
            })
        }
        WireCurve::Linear { x0, y0, x1, y1 } => {
            let same = x0.value == x1.value;
            let x1_number = x1.value.get();
            let x0 = value("x0", quantity.unit(), x0);
            let y0 = value("y0", unit, y0);
            let x1 = value("x1", quantity.unit(), x1);
            let y1 = value("y1", unit, y1);
            if same {
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: at("x1"),
                    value: x1_number,
                    rule: ValueRule::Distinct,
                });
                return None;
            }
            Some(Curve::Linear {
                x0: x0?,
                y0: y0?,
                x1: x1?,
                y1: y1?,
            })
        }
        WireCurve::Step { x, below, above } => {
            let x_number = x.value.get();
            let x = value("x", quantity.unit(), x);
            let below = value("below", unit, below);
            let above = value("above", unit, above);
            if !quantity.holds(x_number) {
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: at("x"),
                    value: x_number,
                    rule: ValueRule::Range,
                });
                return None;
            }
            Some(Curve::Step {
                x: x?,
                below: below?,
                above: above?,
            })
        }
    }
}
