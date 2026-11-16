use serde_json::Value;

use orbe_model::{Time, Value as FieldValue};

use super::{Event, Model, Netlist, SerialEvent, Value as Level};
use crate::blob::Blobs;
use crate::device::{Declared, Events};
use crate::diagnostic::Diagnostic;
use crate::name::Name;
use crate::passive::Switch;

/// The JSON a netlist parsed from, kept so an export is the caller's document and not one
/// written from a `Netlist`.
#[derive(Clone, Debug)]
pub struct Document(Value);

impl Document {
    /// `json` and the netlist it parses to.
    pub fn parse(json: &[u8]) -> Result<(Document, Netlist), Vec<Diagnostic>> {
        Document::parse_with(json, &Blobs::new())
    }

    /// [`Document::parse`], finding each blob the netlist names in `blobs`.
    pub fn parse_with(json: &[u8], blobs: &Blobs) -> Result<(Document, Netlist), Vec<Diagnostic>> {
        let netlist = super::parse_with(json, blobs)?;
        let document = serde_json::from_slice(json).expect("a document that parses is JSON");
        Ok((Document(document), netlist))
    }

    /// This document with every event of `netlist` past the document's own written back,
    /// `stop_ps` set to `stop`, and `grid_ps` cut to `stop`. A switch event is appended to
    /// `events`. A plain `voltage`, `current` or `position` becomes `steps`, with that value
    /// object as `initial`. A `steps` field gains the new levels. A `serial` field gains the new
    /// frames on `events`. A world quantity's changes join its field as a level's do. A part
    /// or a quantity with nothing appended is left as it was. `netlist` parsed from
    /// this document and has only had events appended,
    /// and `stop` is from `max_step_ps` to `stop_ps`. Keys come out sorted and numbers reprinted.
    pub fn export(&self, netlist: &Netlist, stop: Time) -> Vec<u8> {
        let mut json = self.0.clone();
        let transient = &mut json["analysis"]["transient"];
        transient["stop_ps"] = stop.as_ps().into();
        let grid = transient["grid_ps"].as_u64().expect("a parsed grid");
        transient["grid_ps"] = grid.min(stop.as_ps()).into();
        for (quantity, schedule) in netlist.world().declared() {
            let field = &mut json["world"][quantity.name()];
            append_levels(field, schedule.events(), quantity.unit());
        }
        let parts = json["parts"].as_array_mut().expect("parsed parts");
        for part in parts.iter_mut() {
            let name = part["name"].as_str().and_then(|name| Name::new(name).ok());
            let Some(id) = name.and_then(|name| netlist.part_id(&name)) else {
                panic!("{part} is not a part of the netlist");
            };
            match &netlist.part(id).model {
                Model::Switch(switch) => append_edges(part, switch),
                model => {
                    if let Some(level) = model.level() {
                        append_levels(&mut part[level.field], level.schedule.events(), level.unit);
                    }
                    if let Some(serial) = model.serial() {
                        append_serial(&mut part[serial.field], serial.waveform.events());
                    }
                    if let Some((_, events)) = model.device() {
                        append_events(part, events);
                    }
                }
            }
        }
        serde_json::to_vec_pretty(&json).expect("a value serializes")
    }
}

fn append_edges(part: &mut Value, switch: &Switch) {
    let events = part["events"].as_array_mut().expect("parsed events");
    let appended = &switch.events.events()[events.len()..];
    events.extend(
        appended
            .iter()
            .map(|edge| serde_json::to_value(edge).expect("an edge serializes")),
    );
}

fn append_levels(field: &mut Value, events: &[Event<Level>], unit: &str) {
    let kind = field.get("kind").and_then(|kind| kind.as_str());
    let own = match kind {
        None => 0,
        Some("steps") => field["events"].as_array().expect("parsed events").len(),
        Some(kind) => panic!("a level field is plain or steps, not {kind}"),
    };
    let appended = &events[own..];
    if appended.is_empty() {
        return;
    }
    let extra = appended.iter().map(|event| level_event(event, unit));
    if kind.is_none() {
        let initial = field.clone();
        *field = serde_json::json!({
            "kind": "steps",
            "initial": initial,
            "events": extra.collect::<Vec<_>>(),
        });
    } else {
        field["events"]
            .as_array_mut()
            .expect("parsed events")
            .extend(extra);
    }
}

fn append_serial(field: &mut Value, events: &[SerialEvent]) {
    let kind = field.get("kind").and_then(|kind| kind.as_str());
    if kind != Some("serial") {
        panic!("a serial field is serial, not {}", kind.unwrap_or("plain"));
    }
    let own = field["events"].as_array().expect("parsed events").len();
    let appended = &events[own..];
    if appended.is_empty() {
        return;
    }
    field["events"]
        .as_array_mut()
        .expect("parsed events")
        .extend(appended.iter().map(serial_event));
}

/// Each device event past the ones the document lists on its event's field, which it adds
/// when absent.
fn append_events(part: &mut Value, events: &Events) {
    let mut by_field: Vec<(&str, Vec<&Event<Declared>>)> = Vec::new();
    for event in events.all() {
        let field = event.to.field();
        match by_field.iter_mut().find(|(name, _)| *name == field) {
            Some((_, list)) => list.push(event),
            None => by_field.push((field, vec![event])),
        }
    }
    for (field, events) in by_field {
        let own = part
            .get(field)
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let appended = &events[own..];
        if appended.is_empty() {
            continue;
        }
        if part.get(field).is_none() {
            part[field] = serde_json::json!([]);
        }
        part[field]
            .as_array_mut()
            .expect("parsed events")
            .extend(appended.iter().map(|event| device_event(event)));
    }
}

fn device_event(event: &Event<Declared>) -> Value {
    let mut json = serde_json::Map::new();
    json.insert("t_ps".into(), event.at.as_ps().into());
    for &(name, value) in event.to.pairs() {
        json.insert(
            name.into(),
            match value {
                FieldValue::Int(int) => int.into(),
                FieldValue::Bool(on) => on.into(),
            },
        );
    }
    Value::Object(json)
}

fn serial_event(event: &SerialEvent) -> Value {
    serde_json::json!({
        "t_ps": event.at().as_ps(),
        "baud": event.baud().get(),
        "frame": event.frame().as_str(),
        "bytes": event.bytes(),
    })
}

fn level_event(event: &Event<Level>, unit: &str) -> Value {
    serde_json::json!({
        "t_ps": event.at.as_ps(),
        "level": {
            "value": event.to.si,
            "unit": unit,
            "source": "user",
        },
    })
}
