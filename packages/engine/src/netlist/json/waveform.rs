//! A source's `voltage` or `current`: a value object, or an object with `kind`.
//! A value object is a `steps` waveform with no events.

use crate::netlist::{MIN_GAP_PS, Source, Value, Waveform};

use std::borrow::Cow;
use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

use super::{WireValue, resolve_value};
use crate::diagnostic::{Diagnostic, ScheduleRule, WaveformRule};
use crate::name::Ref;
use crate::netlist::waveform::{Baud, Frame, SerialEvent, serial_end};

use crate::time::Time;

pub(crate) enum WireWaveform {
    Dc(WireValue),
    Shape(WireShape),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WireShape {
    Step {
        initial: WireValue,
        #[serde(rename = "final")]
        final_level: WireValue,
        at_ps: u64,
    },
    Pulse {
        initial: WireValue,
        pulsed: WireValue,
        delay_ps: u64,
        width_ps: u64,
        period_ps: u64,
    },
    Steps(WireSteps),
    Serial(WireSerial),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSerial {
    mark: WireValue,
    space: WireValue,
    events: Vec<WireSerialEvent>,
}

/// One `serial` event. `bytes` holds each byte as a JSON integer from 0 to 255.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSerialEvent {
    t_ps: u64,
    #[serde(deserialize_with = "baud_at_least_one")]
    baud: u64,
    frame: WireFrame,
    bytes: Vec<u8>,
}

#[derive(Deserialize)]
enum WireFrame {
    #[serde(rename = "8N1")]
    EightN1,
}

fn baud_at_least_one<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let baud = u64::deserialize(deserializer)?;
    if baud == 0 {
        return Err(de::Error::invalid_value(
            de::Unexpected::Unsigned(0),
            &"an integer from 1 to 500000000",
        ));
    }
    Ok(baud)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSteps {
    pub(crate) initial: WireValue,
    pub(crate) events: Vec<WireLevel>,
}

/// One `steps` event. `level` is a value object. `t_ps` is a JSON integer.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireLevel {
    pub(crate) t_ps: u64,
    pub(crate) level: WireValue,
}

/// A potentiometer `position`: a value object, or `steps` and nothing else.
/// `step` and `pulse` stay unknown variants, so they are `netlist.syntax`.
pub(crate) enum WirePosition {
    Value(WireValue),
    Steps(WireSteps),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StepsOnly {
    Steps(WireSteps),
}

impl<'de> Deserialize<'de> for WirePosition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let object = deserializer.deserialize_map(ObjectVisitor)?;
        let shaped = object.contains_key("kind");
        let object = serde_json::Value::Object(object);
        if shaped {
            StepsOnly::deserialize(object).map(|StepsOnly::Steps(steps)| WirePosition::Steps(steps))
        } else {
            WireValue::deserialize(object).map(WirePosition::Value)
        }
        .map_err(de::Error::custom)
    }
}

/// The object is read whole and split on `kind`, so either spelling keeps serde's own
/// field errors. An untagged enum would replace them with one that names neither.
impl<'de> Deserialize<'de> for WireWaveform {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let object = deserializer.deserialize_map(ObjectVisitor)?;
        let shaped = object.contains_key("kind");
        let object = serde_json::Value::Object(object);
        if shaped {
            WireShape::deserialize(object).map(WireWaveform::Shape)
        } else {
            WireValue::deserialize(object).map(WireWaveform::Dc)
        }
        .map_err(de::Error::custom)
    }
}

/// A JSON object that rejects a repeated key, as a derived struct does. A map keeps the last.
struct ObjectVisitor;

impl<'de> Visitor<'de> for ObjectVisitor {
    type Value = serde_json::Map<String, serde_json::Value>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a value object or a waveform object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut object = serde_json::Map::new();
        while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate field `{key}`")));
            }
            object.insert(key, value);
        }
        Ok(object)
    }
}

/// The field names of one source key, for diagnostics.
pub(crate) struct Fields {
    key: &'static str,
    unit: &'static str,
    initial: &'static str,
    final_level: &'static str,
    pulsed: &'static str,
    at: &'static str,
    delay: &'static str,
    width: &'static str,
    period: &'static str,
    events: &'static str,
    mark: &'static str,
    space: &'static str,
}

pub(crate) const VOLT: &str = "V";
pub(crate) const AMPERE: &str = "A";

pub(crate) const VOLTAGE: Fields = Fields {
    key: "voltage",
    unit: VOLT,
    initial: "voltage.initial",
    final_level: "voltage.final",
    pulsed: "voltage.pulsed",
    at: "voltage.at_ps",
    delay: "voltage.delay_ps",
    width: "voltage.width_ps",
    period: "voltage.period_ps",
    events: "voltage.events",
    mark: "voltage.mark",
    space: "voltage.space",
};

pub(crate) const CURRENT: Fields = Fields {
    key: "current",
    unit: AMPERE,
    initial: "current.initial",
    final_level: "current.final",
    pulsed: "current.pulsed",
    at: "current.at_ps",
    delay: "current.delay_ps",
    width: "current.width_ps",
    period: "current.period_ps",
    events: "current.events",
    mark: "current.mark",
    space: "current.space",
};

pub(crate) fn resolve_waveform(
    part: &Ref,
    fields: &Fields,
    wire: WireWaveform,
    out: &mut Vec<Diagnostic>,
) -> Option<Waveform> {
    let level = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part, field, fields.unit, false, value, out)
    };
    let shape = match wire {
        WireWaveform::Dc(value) => return level(fields.key, value, out).map(Waveform::dc),
        WireWaveform::Shape(shape) => shape,
    };
    let mut broken = Vec::new();
    let waveform = match shape {
        WireShape::Step {
            initial,
            final_level,
            at_ps,
        } => {
            if at_ps == 0 {
                broken.push((fields.at, at_ps, WaveformRule::AtZero));
            }
            let initial = level(fields.initial, initial, out);
            let final_level = level(fields.final_level, final_level, out);
            initial.zip(final_level).map(|(initial, final_level)| {
                Waveform::step(initial, final_level, Time::from_ps(at_ps))
            })
        }
        WireShape::Steps(steps) => resolve_steps(part, fields, steps, out),
        WireShape::Serial(serial) => return resolve_serial(part, fields, serial, out),
        WireShape::Pulse {
            initial,
            pulsed,
            delay_ps,
            width_ps,
            period_ps,
        } => {
            let gap = MIN_GAP_PS;
            if delay_ps == 0 {
                broken.push((fields.delay, delay_ps, WaveformRule::AtZero));
            }
            if width_ps < gap {
                broken.push((fields.width, width_ps, WaveformRule::TooClose));
            }
            if period_ps.checked_sub(width_ps).is_none_or(|low| low < gap) {
                broken.push((fields.period, period_ps, WaveformRule::TooClose));
            }
            if delay_ps.checked_add(width_ps).is_none() {
                broken.push((fields.width, width_ps, WaveformRule::Overflow));
            }
            let initial = level(fields.initial, initial, out);
            let pulsed = level(fields.pulsed, pulsed, out);
            initial.zip(pulsed).map(|(initial, pulsed)| {
                Waveform::pulse(
                    initial,
                    pulsed,
                    Time::from_ps(delay_ps),
                    width_ps,
                    period_ps,
                )
            })
        }
    };
    let ok = broken.is_empty();
    out.extend(
        broken
            .into_iter()
            .map(|(field, value, rule)| Diagnostic::NetlistWaveform {
                part: part.clone(),
                field: Cow::Borrowed(field),
                value,
                rule,
            }),
    );
    waveform.filter(|_| ok)
}

fn resolve_steps(
    part: &Ref,
    fields: &Fields,
    steps: WireSteps,
    out: &mut Vec<Diagnostic>,
) -> Option<Waveform> {
    let unit = fields.unit;
    super::resolve_schedule(
        part,
        fields.events,
        fields.initial,
        steps.initial,
        steps.events,
        |field, wire, out| taken_level(part, field, unit, wire, out),
        out,
    )
    .map(|(initial, events)| Waveform::steps(initial, events))
}

/// `resolve_value`, or the SI number alone when the unit is wrong, so a later
/// schedule check can still compare levels.
fn taken_level(
    part: &Ref,
    field: impl Into<Cow<'static, str>>,
    unit: &'static str,
    wire: WireValue,
    out: &mut Vec<Diagnostic>,
) -> (bool, Value) {
    let si = wire.value;
    match resolve_value(part, field, unit, false, wire, out) {
        Some(value) => (true, value),
        None => (
            false,
            Value {
                si,
                source: Source::User,
            },
        ),
    }
}

fn resolve_serial(
    part: &Ref,
    fields: &Fields,
    wire: WireSerial,
    out: &mut Vec<Diagnostic>,
) -> Option<Waveform> {
    let (mark_ok, mark) = taken_level(part, fields.mark, fields.unit, wire.mark, out);
    let (space_ok, space) = taken_level(part, fields.space, fields.unit, wire.space, out);
    let mut ok = mark_ok && space_ok;
    if mark.si == space.si {
        ok = false;
        out.push(Diagnostic::NetlistWaveform {
            part: part.clone(),
            field: Cow::Borrowed(fields.space),
            value: 0,
            rule: WaveformRule::Equal,
        });
    }
    let mut events = Vec::new();
    let mut previous_start = None;
    let mut previous_end = None;
    for (index, event) in wire.events.into_iter().enumerate() {
        let at = event.t_ps;
        let baud = event.baud;
        let WireFrame::EightN1 = event.frame;
        let span = serial_end(at, baud, event.bytes.len());
        let rule = serial_schedule(
            at,
            event.bytes.is_empty(),
            previous_start,
            previous_end,
            span,
        );
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: fields.events,
                event: index,
                rule,
                end_ps: if rule == ScheduleRule::Overlap {
                    previous_end.unwrap_or(0)
                } else {
                    0
                },
            });
        }
        if baud > u64::from(Baud::MAX) {
            ok = false;
            out.push(Diagnostic::NetlistWaveform {
                part: part.clone(),
                field: Cow::Owned(format!("{}.events[{index}].baud", fields.key)),
                value: baud,
                rule: WaveformRule::TooClose,
            });
        }
        if rule == Some(ScheduleRule::Overflow) {
            break;
        }
        if ok {
            let rate = u32::try_from(baud).expect("a baud within Baud::MAX fits in u32");
            events.push(SerialEvent::new(
                Time::from_ps(at),
                Baud::new(rate).expect("a baud within Baud::MAX is a baud"),
                Frame::EightN1,
                event.bytes,
            ));
        }
        previous_start = Some(at);
        previous_end = span;
    }
    ok.then(|| Waveform::serial(mark, space, events))
}

pub(crate) fn serial_schedule(
    at: u64,
    bytes_empty: bool,
    previous_start: Option<u64>,
    previous_end: Option<u64>,
    span: Option<u64>,
) -> Option<ScheduleRule> {
    if previous_start.is_some_and(|previous| at <= previous) {
        Some(ScheduleRule::NotIncreasing)
    } else if at == 0 {
        Some(ScheduleRule::AtZero)
    } else if bytes_empty {
        Some(ScheduleRule::SameState)
    } else if span.is_none() {
        Some(ScheduleRule::Overflow)
    } else if previous_end.is_some_and(|end| at < end) {
        Some(ScheduleRule::Overlap)
    } else {
        None
    }
}
