//! A source's `voltage` or `current`: today's value object for DC, or an object with
//! `kind` for a timed shape.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

use super::{WireValue, resolve_value};
use crate::diagnostic::{Diagnostic, WaveformRule};
use crate::name::Ref;
use crate::netlist::{Schedule, Waveform};
use crate::time::Time;

pub(super) enum WireWaveform {
    Dc(WireValue),
    Shape(WireShape),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireShape {
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
pub(super) struct Fields {
    key: &'static str,
    unit: &'static str,
    initial: &'static str,
    final_level: &'static str,
    pulsed: &'static str,
    at: &'static str,
    delay: &'static str,
    width: &'static str,
    period: &'static str,
}

pub(super) const VOLTAGE: Fields = Fields {
    key: "voltage",
    unit: "V",
    initial: "voltage.initial",
    final_level: "voltage.final",
    pulsed: "voltage.pulsed",
    at: "voltage.at_ps",
    delay: "voltage.delay_ps",
    width: "voltage.width_ps",
    period: "voltage.period_ps",
};

pub(super) const CURRENT: Fields = Fields {
    key: "current",
    unit: "A",
    initial: "current.initial",
    final_level: "current.final",
    pulsed: "current.pulsed",
    at: "current.at_ps",
    delay: "current.delay_ps",
    width: "current.width_ps",
    period: "current.period_ps",
};

pub(super) fn resolve_waveform(
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
        WireShape::Pulse {
            initial,
            pulsed,
            delay_ps,
            width_ps,
            period_ps,
        } => {
            let gap = Schedule::MIN_GAP_PS;
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
                field,
                value,
                rule,
            }),
    );
    waveform.filter(|_| ok)
}
