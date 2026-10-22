use serde_json::Value;

use super::{Model, Netlist};
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

    /// This document with every switch event of `netlist` past the document's own appended
    /// to that part's `events`, `stop_ps` set to `stop`, and `grid_ps` cut to `stop`. `netlist`
    /// parsed from this document and has only had events appended, and `stop` is from
    /// `max_step_ps` to `stop_ps`. Keys come out sorted and numbers reprinted.
    pub fn export(&self, netlist: &Netlist, stop: Time) -> Vec<u8> {
        let mut json = self.0.clone();
        let transient = &mut json["analysis"]["transient"];
        transient["stop_ps"] = stop.as_ps().into();
        let grid = transient["grid_ps"].as_u64().expect("a parsed grid");
        transient["grid_ps"] = grid.min(stop.as_ps()).into();
        let parts = json["parts"].as_array_mut().expect("parsed parts");
        for part in parts.iter_mut().filter(|part| part["kind"] == "switch") {
            let name = part["name"].as_str().and_then(|name| Name::new(name).ok());
            let id = name.and_then(|name| netlist.part_id(&name));
            let Some(Model::Switch(switch)) = id.map(|id| &netlist.part(id).model) else {
                panic!("{part} is not a switch of the netlist");
            };
            let events = part["events"].as_array_mut().expect("parsed events");
            let appended = &switch.events.events()[events.len()..];
            events.extend(
                appended
                    .iter()
                    .map(|edge| serde_json::to_value(edge).expect("an edge serializes")),
            );
        }
        serde_json::to_vec_pretty(&json).expect("a value serializes")
    }
}
