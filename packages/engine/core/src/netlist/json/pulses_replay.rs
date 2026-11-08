//! The `pulses_replay` kind: its pins, its idle level, its output stage and its trains.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;

use super::waveform::serial_schedule;
use super::{PinMap, WireOutputStage, resolve_output_stage, resolve_pins};
use crate::diagnostic::{Diagnostic, ScheduleRule, ValueRule};
use crate::name::{Name, Ref};
use crate::netlist::{NetId, PulseTrain, PulsesReplay};
use crate::time::Time;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WirePulsesReplay {
    pub(super) name: String,
    pins: PinMap,
    idle: bool,
    output: WireOutputStage,
    events: Vec<WireEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    t_ps: u64,
    pulses: Vec<WirePulse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePulse {
    level: bool,
    ps: u64,
}

pub(super) fn resolve_pulses_replay(
    name: &Name,
    part_ref: &Ref,
    wire: WirePulsesReplay,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<PulsesReplay> {
    let pins = resolve_pins(name, PulsesReplay::PINS, &wire.pins.0, nets, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let events = resolve_events(part_ref, wire.events, out);
    Some(PulsesReplay {
        pins: pins?,
        idle: wire.idle,
        output: output?,
        events: events?,
    })
}

fn resolve_events(
    part: &Ref,
    wire: Vec<WireEvent>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<PulseTrain>> {
    let mut ok = true;
    let mut events = Vec::with_capacity(wire.len());
    let mut previous_start = None;
    let mut previous_end = None;
    for (index, event) in wire.into_iter().enumerate() {
        let at = event.t_ps;
        let mut pulses = Vec::with_capacity(event.pulses.len());
        for (pulse, wire) in event.pulses.into_iter().enumerate() {
            if wire.ps == 0 {
                ok = false;
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: format!("events[{index}].pulses[{pulse}].ps").into(),
                    value: 0.0,
                    rule: ValueRule::Positive,
                });
                continue;
            }
            pulses.push((wire.level, wire.ps));
        }
        let end = end_of(at, &pulses);
        let rule = serial_schedule(at, pulses.is_empty(), previous_start, previous_end, end);
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: "events",
                event: index,
                rule,
                end_ps: if rule == ScheduleRule::Overlap {
                    previous_end.unwrap_or(0)
                } else {
                    0
                },
            });
            if rule == ScheduleRule::Overflow {
                break;
            }
        }
        if let Some(end) = end
            && !pulses.is_empty()
        {
            events.push(PulseTrain {
                at: Time::from_ps(at),
                end: Time::from_ps(end),
                pulses: Arc::from(pulses),
            });
        }
        previous_start = Some(at);
        previous_end = end;
    }
    ok.then_some(events)
}

fn end_of(at: u64, pulses: &[(bool, u64)]) -> Option<u64> {
    let mut t = at;
    for &(_, ps) in pulses {
        t = t.checked_add(ps)?;
    }
    Some(t)
}
