use serde::Deserialize;

use orbe_model::Time;

use crate::diagnostic::{Diagnostic, ValueRule};
use crate::element::{Observed, OwnedCapacitor, OwnedInductor, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::{Quantity, Ref};
use crate::netlist::NetId;
use crate::netlist::Schedule;
use crate::netlist::Source;
use crate::netlist::Value;
use crate::netlist::broken_fraction;
use crate::netlist::broken_positive;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::waveform::{WirePosition, WireSteps};
use crate::netlist::json::{FRACTION_UNIT, PinMap, WireValue, resolve_pins, resolve_value};
use crate::netlist::{Conduction, Kind, LevelField};
use crate::newton::Side;

/// `a` to `wiper` is `position * resistance`. `wiper` to `b` is the rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Potentiometer {
    /// Nets of pins `a`, `wiper`, `b`.
    pub pins: [NetId; 3],
    /// Ohm, greater than 0.
    pub resistance: Value,
    /// Fraction of `resistance` from `a` to the wiper, in `(0, 1)`.
    pub position: Schedule<Value>,
}

impl Potentiometer {
    pub const PINS: [&'static str; 3] = ["a", "wiper", "b"];
}

/// A potentiometer position that breaks a rule, in the order parse reports them.
/// The fraction is outside `(0, 1)`. Otherwise each leg is checked: `position * resistance`,
/// then `(1 - position) * resistance`. A leg that is not positive is [`ValueRule::Positive`].
/// A leg whose reciprocal is not finite is [`ValueRule::FiniteReciprocal`].
pub struct PositionRule {
    kind: PositionKind,
}

enum PositionKind {
    Fraction,
    Legs([(f64, Option<ValueRule>); 2]),
}

impl PositionRule {
    /// The first broken rule. `None` when the position holds.
    pub fn first(self) -> Option<ValueRule> {
        match self.kind {
            PositionKind::Fraction => Some(ValueRule::Fraction),
            PositionKind::Legs(legs) => legs.into_iter().find_map(|(_, rule)| rule),
        }
    }

    /// Both legs when the fraction holds. Each entry is the leg's ohms and the rule it breaks.
    /// `None` when the fraction breaks, because the legs are not checked then.
    pub fn legs(self) -> Option<[(f64, Option<ValueRule>); 2]> {
        match self.kind {
            PositionKind::Fraction => None,
            PositionKind::Legs(legs) => Some(legs),
        }
    }
}

pub fn position_rule(position: f64, resistance: f64) -> PositionRule {
    if broken_fraction(position).is_some() {
        return PositionRule {
            kind: PositionKind::Fraction,
        };
    }
    let legs = [position * resistance, (1.0 - position) * resistance]
        .map(|ohms| (ohms, broken_positive(ohms)));
    PositionRule {
        kind: PositionKind::Legs(legs),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WirePotentiometer {
    pub(crate) name: String,
    pins: PinMap,
    resistance: WireValue,
    position: WirePosition,
}

pub(crate) fn resolve_potentiometer(
    ctx: ParseCtx<'_>,
    wire: WirePotentiometer,
) -> Option<Potentiometer> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let WirePotentiometer {
        pins,
        resistance,
        position,
        ..
    } = wire;
    let pins = resolve_pins(name, Potentiometer::PINS, &pins.0, nets, out);
    let resistance = resolve_value(part_ref, "resistance", "ohm", true, resistance, out);
    let position = resolve_position(part_ref, resistance.as_ref(), position, out);
    match (pins, resistance, position) {
        (Some(pins), Some(resistance), Some(position)) => Some(Potentiometer {
            pins,
            resistance,
            position,
        }),
        _ => None,
    }
}

fn resolve_position(
    part: &Ref,
    resistance: Option<&Value>,
    position: WirePosition,
    out: &mut Vec<Diagnostic>,
) -> Option<Schedule<Value>> {
    match position {
        WirePosition::Value(value) => {
            let (ok, position) = level_with_legs(part, "position", value, resistance, out);
            ok.then_some(Schedule {
                initial: position,
                events: Vec::new(),
            })
        }
        WirePosition::Steps(steps) => resolve_steps(part, resistance, steps, out),
    }
}

fn resolve_steps(
    part: &Ref,
    resistance: Option<&Value>,
    steps: WireSteps,
    out: &mut Vec<Diagnostic>,
) -> Option<Schedule<Value>> {
    crate::netlist::json::resolve_schedule(
        part,
        "position.events",
        "position.initial",
        steps.initial,
        steps.events,
        |field, wire, out| level_with_legs(part, field, wire, resistance, out),
        out,
    )
    .map(|(initial, events)| Schedule { initial, events })
}

fn level_with_legs(
    part: &Ref,
    field: impl Into<std::borrow::Cow<'static, str>>,
    wire: WireValue,
    resistance: Option<&Value>,
    out: &mut Vec<Diagnostic>,
) -> (bool, Value) {
    let si = wire.value;
    let number = si.get();
    let field = field.into();
    let resolved = resolve_value(part, field.clone(), FRACTION_UNIT, false, wire, out);
    let mut ok = resolved.is_some();
    let value = resolved.unwrap_or(Value {
        si,
        source: Source::User,
    });
    if let Some(resistance) = resistance {
        let Some(legs) = position_rule(number, resistance.si.get()).legs() else {
            refuse_fraction(part, field, number, out);
            return (false, value);
        };
        if !ok {
            return (false, value);
        }
        for (ohms, rule) in legs {
            let Some(rule) = rule else {
                continue;
            };
            ok = false;
            out.push(Diagnostic::NetlistValue {
                part: part.clone(),
                field: "resistance".into(),
                value: ohms,
                rule,
            });
        }
    } else if broken_fraction(number).is_some() {
        refuse_fraction(part, field, number, out);
        return (false, value);
    }
    (ok, value)
}

fn refuse_fraction(
    part: &Ref,
    field: std::borrow::Cow<'static, str>,
    number: f64,
    out: &mut Vec<Diagnostic>,
) {
    out.push(Diagnostic::NetlistValue {
        part: part.clone(),
        field,
        value: number,
        rule: ValueRule::Fraction,
    });
}

impl Kind for Potentiometer {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns::default()
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let ohms = self.resistance.si.get();
        let position = stamp
            .conditions
            .side()
            .read(&self.position, stamp.conditions.at)
            .si
            .get();
        let [a, wiper, b] = self.pins;
        stamp.conductance(a, wiper, 1.0 / (position * ohms));
        stamp.conductance(wiper, b, 1.0 / ((1.0 - position) * ohms));
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1), (1, 2)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        &[]
    }
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, _probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        None
    }
    fn conduction(&self) -> Conduction {
        Conduction::Limits
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.position.next_change_after(at)
    }
    fn level(&self) -> Option<LevelField<'_>> {
        Some(LevelField {
            schedule: &self.position,
            field: "position",
            unit: FRACTION_UNIT,
        })
    }
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        Some(&mut self.position)
    }
}
