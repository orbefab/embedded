//! The `spi_replay` kind: its pins, its clock, its output stage and its transactions.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::waveform::serial_schedule;
use super::{PinMap, WireInt, WireOutputStage, resolve_int, resolve_output_stage, resolve_pins};
use crate::diagnostic::{Diagnostic, ScheduleRule};
use crate::link::{Cs, Lanes, SpiPhase};
use crate::name::{Name, Ref};
use crate::netlist::waveform::boundary;
use crate::netlist::{NetId, SpiEvent, SpiReplay};
use crate::time::Time;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireSpiReplay {
    pub(super) name: String,
    pins: PinMap,
    clock_hz: WireInt<u32>,
    output: WireOutputStage,
    events: Vec<WireSpiEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpiEvent {
    t_ps: u64,
    phases: Vec<WirePhase>,
    lanes: WireLanes,
    /// Each byte as a JSON integer from 0 to 255.
    bytes: Vec<u8>,
    cs: WireCs,
}

/// A phase whose `bits` is from 1 to 32, whose `value` fits in them, and whose `lanes` is 1, 2
/// or 4. Anything else is `netlist.syntax`.
#[derive(Deserialize)]
#[serde(try_from = "RawPhase")]
struct WirePhase(SpiPhase);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPhase {
    bits: u8,
    lanes: WireLanes,
    value: u32,
}

impl TryFrom<RawPhase> for WirePhase {
    type Error = &'static str;

    fn try_from(raw: RawPhase) -> Result<WirePhase, &'static str> {
        if !(1..=32).contains(&raw.bits) {
            return Err("a phase has 1 to 32 bits");
        }
        if raw.bits < 32 && raw.value >> raw.bits != 0 {
            return Err("a phase's value fits in its bits");
        }
        Ok(WirePhase(SpiPhase {
            bits: raw.bits,
            lanes: raw.lanes.0,
            value: raw.value,
        }))
    }
}

#[derive(Deserialize)]
#[serde(try_from = "u8")]
struct WireLanes(Lanes);

impl TryFrom<u8> for WireLanes {
    type Error = &'static str;

    fn try_from(count: u8) -> Result<WireLanes, &'static str> {
        match count {
            1 => Ok(WireLanes(Lanes::One)),
            2 => Ok(WireLanes(Lanes::Two)),
            4 => Ok(WireLanes(Lanes::Four)),
            _ => Err("lanes is 1, 2 or 4"),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireCs {
    Held,
    Released,
}

pub(super) fn resolve_spi_replay(
    name: &Name,
    part_ref: &Ref,
    wire: WireSpiReplay,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<SpiReplay> {
    let pins = resolve_pins(name, SpiReplay::PINS, &wire.pins.0, nets, out);
    let clock_hz = resolve_int(part_ref, "clock_hz", "Hz", wire.clock_hz, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let events = clock_hz
        .as_ref()
        .and_then(|clock| resolve_events(part_ref, clock.value, wire.events, out));
    Some(SpiReplay {
        pins: pins?,
        clock_hz: clock_hz?,
        output: output?,
        events: events?,
    })
}

/// The serial source's schedule rules, with an event that has no phase and no byte as
/// `same_state`, because it sends nothing.
fn resolve_events(
    part: &Ref,
    clock_hz: u64,
    wire: Vec<WireSpiEvent>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<SpiEvent>> {
    let mut ok = true;
    let mut events = Vec::with_capacity(wire.len());
    let mut previous_start = None;
    let mut previous_end = None;
    for (index, event) in wire.into_iter().enumerate() {
        let at = event.t_ps;
        let phases: Vec<SpiPhase> = event.phases.into_iter().map(|phase| phase.0).collect();
        let lanes = event.lanes.0;
        let end = boundary(at, clock_hz, cycles(&phases, lanes, event.bytes.len()));
        let empty = phases.is_empty() && event.bytes.is_empty();
        let rule = serial_schedule(at, empty, previous_start, previous_end, end);
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
        if let Some(end) = end {
            events.push(SpiEvent {
                at: Time::from_ps(at),
                end: Time::from_ps(end),
                phases,
                lanes,
                data: event.bytes.into(),
                cs: match event.cs {
                    WireCs::Held => Cs::Held,
                    WireCs::Released => Cs::Released,
                },
            });
        }
        previous_start = Some(at);
        previous_end = end;
    }
    ok.then_some(events)
}

/// SCLK cycles: each phase's bits over its lanes, then the data's bits over `lanes`, each
/// rounded up. Saturates, so a count past `u64` is an end that does not fit.
fn cycles(phases: &[SpiPhase], lanes: Lanes, bytes: usize) -> u64 {
    let over = |bits: u64, lanes: Lanes| bits.div_ceil(u64::from(lanes.count()));
    let data = u64::try_from(bytes).map_or(u64::MAX, |bytes| bytes.saturating_mul(8));
    phases
        .iter()
        .map(|phase| over(u64::from(phase.bits), phase.lanes))
        .fold(over(data, lanes), u64::saturating_add)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(bits: u8, lanes: Lanes) -> SpiPhase {
        SpiPhase {
            bits,
            lanes,
            value: 0,
        }
    }

    #[test]
    fn cycles_round_each_phase_and_the_data_up() {
        let command = [phase(8, Lanes::One), phase(24, Lanes::One)];
        assert_eq!(cycles(&command, Lanes::Four, 3), 32 + 6);
        assert_eq!(cycles(&[phase(5, Lanes::Two)], Lanes::Four, 1), 3 + 2);
        assert_eq!(cycles(&[], Lanes::One, 0), 0);
    }
}
