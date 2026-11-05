//! The `st77916` kind: its pins, its `TE` output stage and its `RST` threshold.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::atmega328p::{WireThreshold, resolve_threshold};
use super::{PinMap, WireOutputStage, resolve_output_stage, resolve_pins};
use crate::diagnostic::Diagnostic;
use crate::name::{Name, Ref};
use crate::netlist::{NetId, St77916};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireSt77916 {
    pub(super) name: String,
    pins: PinMap,
    output: WireOutputStage,
    input_threshold: WireThreshold,
}

pub(super) fn resolve_st77916(
    name: &Name,
    part_ref: &Ref,
    wire: WireSt77916,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<St77916> {
    let pins = resolve_pins(name, St77916::PINS, &wire.pins.0, nets, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    Some(St77916 {
        pins: pins?,
        output: output?,
        input_threshold: input_threshold?,
    })
}
