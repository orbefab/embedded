//! Built-in devices. A device on the `orbe-model` contract is a folder: `model.rs` is the device,
//! written against `orbe-model` and `std` alone, and `mod.rs` holds its netlist struct, its JSON
//! wire type and its [`Row`], which `Model::device` maps its parts to.

pub(crate) mod cst816;
pub(crate) mod dht22;
pub(crate) mod pulses_replay;
pub(crate) mod spi_replay;
pub mod ssd1306;
pub mod st77916;
pub(crate) mod ws2812;

pub use cst816::Cst816;
pub use dht22::Dht22;
pub use pulses_replay::{PulseTrain, PulsesReplay};
pub use spi_replay::{SpiEvent, SpiReplay};
pub use ssd1306::Ssd1306;
pub use st77916::St77916;
pub use ws2812::Ws2812;

use orbe_model::{Device, Event as ModelEvent, Fields, Quantity, Time, Value};

use crate::diagnostic::{Diagnostic, ScheduleRule, ValueRule};
use crate::name::Ref;
use crate::netlist::{Event, MIN_GAP_PS};

/// The world quantities a part of the device kind `kind` reads, by its netlist name. A scene
/// declares each one for every such part. None for a kind that is no device, or reads none.
pub fn reads(kind: &str) -> &'static [Quantity] {
    match kind {
        "dht22" => dht22::model::Dht22::WORLD,
        _ => &[],
    }
}

/// One built-in device kind on the contract: what the engine reads of it for every part.
#[derive(Debug, PartialEq)]
pub(crate) struct Row {
    /// The events a part of this kind takes, from its netlist and from a session.
    pub(crate) events: &'static [EventSpec],
}

impl Row {
    /// A row whose events each have their own name and netlist field, and whose fields each
    /// have their own name other than `t_ps`. A row that breaks this does not compile.
    pub(crate) const fn new(events: &'static [EventSpec]) -> Row {
        let mut k = 0;
        while k < events.len() {
            let event = &events[k];
            let mut j = 0;
            while j < k {
                assert!(
                    !same(events[j].name, event.name) && !same(events[j].field, event.field),
                    "each event has its own name and field"
                );
                j += 1;
            }
            let mut f = 0;
            while f < event.fields.len() {
                let name = event.fields[f].0;
                assert!(!same(name, "t_ps"), "t_ps is the event's time");
                let mut g = 0;
                while g < f {
                    assert!(
                        !same(event.fields[g].0, name),
                        "each field has its own name"
                    );
                    g += 1;
                }
                f += 1;
            }
            k += 1;
        }
        Row { events }
    }
}

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut k = 0;
    while k < a.len() {
        if a[k] != b[k] {
            return false;
        }
        k += 1;
    }
    true
}

/// An event a device declares: the name it sees, the netlist field that schedules it, and its
/// fields in the order it reads them.
#[derive(Debug, PartialEq)]
pub(crate) struct EventSpec {
    pub(crate) name: &'static str,
    pub(crate) field: &'static str,
    pub(crate) fields: &'static [(&'static str, FieldSpec)],
}

/// What one field of an event holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldSpec {
    /// An integer from 0 to `max`. One past it breaks rule `span`.
    Int {
        max: i64,
    },
    Bool,
}

/// Why a setting's fields are not an event's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mismatch {
    /// A field is missing, unknown, repeated, or of the wrong kind.
    Shape,
    /// An integer is outside its span.
    Span,
}

impl EventSpec {
    /// `fields`, each a name and a value, as this event, with its fields in declaration order.
    pub(crate) fn declared(
        &'static self,
        fields: &[(String, Value)],
    ) -> Result<Declared, Mismatch> {
        if fields.len() != self.fields.len() {
            return Err(Mismatch::Shape);
        }
        let mut values = Vec::with_capacity(self.fields.len());
        for &(name, spec) in self.fields {
            let mut given = fields.iter().filter(|(field, _)| field == name);
            let (Some((_, value)), None) = (given.next(), given.next()) else {
                return Err(Mismatch::Shape);
            };
            match (spec, *value) {
                (FieldSpec::Int { max }, Value::Int(int)) if !(0..=max).contains(&int) => {
                    return Err(Mismatch::Span);
                }
                (FieldSpec::Int { .. }, Value::Int(_)) | (FieldSpec::Bool, Value::Bool(_)) => {}
                _ => return Err(Mismatch::Shape),
            }
            values.push((name, *value));
        }
        Ok(Declared {
            spec: self,
            fields: values,
        })
    }
}

/// One event a device declares, with a value for each of its fields in declaration order.
#[derive(Clone, Debug, PartialEq)]
pub struct Declared {
    spec: &'static EventSpec,
    fields: Vec<(&'static str, Value)>,
}

impl Declared {
    pub fn name(&self) -> &'static str {
        self.spec.name
    }

    pub fn fields(&self) -> Fields<'_> {
        Fields::new(&self.fields)
    }

    pub(crate) fn event(&self) -> ModelEvent<'_> {
        ModelEvent::Declared {
            name: self.spec.name,
            fields: self.fields(),
        }
    }
}

/// No events, for a device kind whose row is empty.
pub(crate) static NO_EVENTS: Events = Events::EMPTY;

/// A device part's events in time order. Only parse and `Netlist::append` build one, so each is
/// after t = 0 and at least [`MIN_GAP_PS`] after the one before.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Events(Vec<Event<Declared>>);

impl Events {
    /// A part that takes no events.
    pub(crate) const EMPTY: Events = Events(Vec::new());

    pub fn all(&self) -> &[Event<Declared>] {
        &self.0
    }

    /// The event at `at`.
    pub(crate) fn at(&self, at: Time) -> Option<&Declared> {
        let k = self.0.partition_point(|event| event.at < at);
        self.0
            .get(k)
            .filter(|event| event.at == at)
            .map(|event| &event.to)
    }

    /// The next event strictly after `at`.
    pub(crate) fn next_after(&self, at: Time) -> Option<Time> {
        let k = self.0.partition_point(|event| event.at <= at);
        self.0.get(k).map(|event| event.at)
    }

    /// The first time at or after `from` that one more event passes the schedule rules. With no
    /// event that is 1 ps. After one it is that event plus [`MIN_GAP_PS`].
    pub(crate) fn earliest(&self, from: Time) -> Option<Time> {
        let first = match self.0.last() {
            None => 1,
            Some(last) => last.at.as_ps().checked_add(MIN_GAP_PS)?,
        };
        Some(Time::from_ps(from.as_ps().max(first)))
    }

    pub(crate) fn push(&mut self, at: Time, event: Declared) {
        debug_assert_eq!(self.earliest(at), Some(at));
        self.0.push(Event { at, to: event });
    }
}

/// The events of `spec` a netlist schedules on `part`, each its time in picoseconds and its
/// fields. Each must be after t = 0 and at least [`MIN_GAP_PS`] after the one before, and each
/// integer field inside its span. Every break is a diagnostic, and then there are no events.
pub(crate) fn resolve_events(
    part: &Ref,
    spec: &'static EventSpec,
    wires: Vec<(u64, Vec<(String, Value)>)>,
    out: &mut Vec<Diagnostic>,
) -> Option<Events> {
    let mut ok = true;
    let mut previous: Option<u64> = None;
    let mut earliest = Some(0u64);
    let mut events = Events::default();
    for (index, (at, fields)) in wires.into_iter().enumerate() {
        let rule = if previous.is_some_and(|previous| at <= previous) {
            Some(ScheduleRule::NotIncreasing)
        } else if at == 0 {
            Some(ScheduleRule::AtZero)
        } else if earliest.is_none_or(|earliest| at < earliest) {
            Some(ScheduleRule::TooClose)
        } else {
            None
        };
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: spec.field,
                event: index,
                rule,
                end_ps: 0,
            });
        }
        for (name, value) in &fields {
            let Some((_, FieldSpec::Int { max })) = spec.fields.iter().find(|(f, _)| f == name)
            else {
                continue;
            };
            if let Value::Int(int) = *value
                && !(0..=*max).contains(&int)
            {
                ok = false;
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: format!("{}[{index}].{name}", spec.field).into(),
                    value: int as f64,
                    rule: ValueRule::Span,
                });
            }
        }
        previous = Some(at);
        earliest = at.checked_add(MIN_GAP_PS);
        if ok {
            let event = spec
                .declared(&fields)
                .expect("a wire type names each field of its event");
            events.0.push(Event {
                at: Time::from_ps(at),
                to: event,
            });
        }
    }
    ok.then_some(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINT: EventSpec = EventSpec {
        name: "point",
        field: "points",
        fields: &[("x", FieldSpec::Int { max: 9 }), ("on", FieldSpec::Bool)],
    };

    fn fields(x: i64, on: bool) -> Vec<(String, Value)> {
        vec![
            ("on".to_string(), Value::Bool(on)),
            ("x".to_string(), Value::Int(x)),
        ]
    }

    #[test]
    fn a_setting_is_an_event_only_with_each_field_once_in_its_span() {
        let event = POINT.declared(&fields(9, true)).unwrap();
        assert_eq!(event.fields().int("x"), Some(9));
        assert_eq!(event.fields().bool("on"), Some(true));
        assert_eq!(POINT.declared(&fields(10, true)), Err(Mismatch::Span));
        assert_eq!(POINT.declared(&fields(-1, true)), Err(Mismatch::Span));
        let mut extra = fields(1, true);
        extra.push(("y".to_string(), Value::Int(1)));
        assert_eq!(POINT.declared(&extra), Err(Mismatch::Shape));
        let twice = vec![
            ("x".to_string(), Value::Int(1)),
            ("x".to_string(), Value::Int(2)),
        ];
        assert_eq!(POINT.declared(&twice), Err(Mismatch::Shape));
        let swapped = vec![
            ("x".to_string(), Value::Bool(true)),
            ("on".to_string(), Value::Int(1)),
        ];
        assert_eq!(POINT.declared(&swapped), Err(Mismatch::Shape));
    }
}
