use serde_json::Value;

use super::{Chip, Event, Model, Netlist, SerialEvent, Switch, Touch, Value as Level};
use crate::diagnostic::Diagnostic;
use crate::name::Name;
use crate::time::Time;

/// The JSON a netlist parsed from, kept so an export is the caller's document and not one
/// written from a `Netlist`.
#[derive(Clone, Debug)]
pub struct Document(Value);

impl Document {
    /// `json` and the netlist it parses to.
    pub fn parse(json: &[u8]) -> Result<(Document, Netlist), Vec<Diagnostic>> {
        let netlist = super::parse(json)?;
        let document = serde_json::from_slice(json).expect("a document that parses is JSON");
        Ok((Document(document), netlist))
    }

    /// This document with every event of `netlist` past the document's own written back,
    /// `stop_ps` set to `stop`, and `grid_ps` cut to `stop`. A switch event is appended to
    /// `events`. A plain `voltage`, `current` or `position` becomes `steps`, with that value
    /// object as `initial`. A `steps` field gains the new levels. A `serial` field gains the new
    /// frames on `events`. A part with nothing appended is left as it was. `netlist` parsed from
    /// this document and has only had events appended,
    /// and `stop` is from `max_step_ps` to `stop_ps`. Keys come out sorted and numbers reprinted.
    pub fn export(&self, netlist: &Netlist, stop: Time) -> Vec<u8> {
        let mut json = self.0.clone();
        let transient = &mut json["analysis"]["transient"];
        transient["stop_ps"] = stop.as_ps().into();
        let grid = transient["grid_ps"].as_u64().expect("a parsed grid");
        transient["grid_ps"] = grid.min(stop.as_ps()).into();
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
                    if let Model::Chip(Chip::Cst816(dev)) = model {
                        append_touches(part, &dev.touches);
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

fn append_touches(part: &mut Value, events: &[Event<Touch>]) {
    let own = part
        .get("touches")
        .and_then(|touches| touches.as_array())
        .map(|touches| touches.len())
        .unwrap_or(0);
    let appended = &events[own..];
    if appended.is_empty() {
        return;
    }
    if part.get("touches").is_none() {
        part["touches"] = serde_json::json!([]);
    }
    part["touches"]
        .as_array_mut()
        .expect("parsed touches")
        .extend(appended.iter().map(touch_event));
}

fn touch_event(event: &Event<Touch>) -> Value {
    serde_json::json!({
        "t_ps": event.at.as_ps(),
        "x": event.to.x,
        "y": event.to.y,
        "down": event.to.down,
    })
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
