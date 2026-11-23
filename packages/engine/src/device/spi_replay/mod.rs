//! The `spi_replay` part. [`model`] is the device, which holds the pins at their idle levels.
//! This file is its netlist struct, its JSON wire type, its row in the device table, and its
//! events as the records the host's replay engine sends.

use std::sync::Arc;

use serde::Deserialize;

use orbe_model::{Cs, Device, Fields, Lanes, Miso, Spi, SpiPhase, Time};

use crate::active::native::Native;
use crate::active::replay::Replay;
use crate::active::{Chip, Core, Output, Stage, Takes};
use crate::diagnostic::{Diagnostic, ScheduleRule};
use crate::link::{Record, RecordKind, SpiNets};
use crate::name::Ref;
use crate::netlist::IntValue;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Value;
use crate::netlist::json::ParseCtx;
use crate::netlist::json::waveform::serial_schedule;
use crate::netlist::json::{PinMap, WireInt, resolve_int, resolve_pins};
use crate::netlist::waveform::boundary;
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};

const VCC: usize = 0;
const GND: usize = 1;
const CS: usize = 2;
const SCLK: usize = 3;
const D0: usize = 4;

pub(crate) mod model;

/// The outputs, by package index, in image order: `CS`, `SCLK`, then `D0` to `D3`.
const OUTPUTS: [usize; model::OUTPUTS] = [CS, SCLK, D0, D0 + 1, D0 + 2, D0 + 3];

/// No events. A replay's transactions are its netlist's.
pub(crate) const ROW: crate::device::Row = crate::device::Row::new(&[]);

/// The pin pairs the chip joins at DC, for `net.floating`: each output to `VCC` and to `GND`,
/// one of which its drive always stamps.
pub(crate) const DC_PATHS: [(usize, usize); 12] = {
    let mut paths = [(0, 0); 12];
    let mut k = 0;
    while k < OUTPUTS.len() {
        paths[2 * k] = (OUTPUTS[k], VCC);
        paths[2 * k + 1] = (OUTPUTS[k], GND);
        k += 1;
    }
    paths
};

pub(crate) fn chip(part: PartId, replay: &SpiReplay) -> Chip {
    let pins = replay.pins;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let high = stage(&replay.output.high_drop, &replay.output.high_resistance);
    let low = stage(&replay.output.low_drop, &replay.output.low_resistance);
    let outputs = OUTPUTS
        .iter()
        .map(|&k| Output {
            pin: pins[k],
            vcc: pins[VCC],
            gnd: pins[GND],
            high,
            low,
            pull_up: None,
            pull_down: None,
            leakage: None,
        })
        .collect();
    let nets = SpiNets {
        cs: Some(pins[CS]),
        sclk: Some(pins[SCLK]),
        data: [D0, D0 + 1, D0 + 2, D0 + 3].map(|k| Some(pins[k])),
    };
    let clock_hz = u32::try_from(replay.clock_hz.value).expect("parse keeps the clock in u32");
    let records: Arc<[Record]> = (0u64..)
        .zip(&replay.events)
        .map(|(seq, event)| Record {
            seq,
            start: event.at,
            end: event.end,
            pin: SpiReplay::PINS[CS],
            kind: RecordKind::Spi {
                spi: Spi {
                    phases: event.phases.clone(),
                    lanes: event.lanes,
                    data: event.data.clone(),
                    clock_hz,
                    cs: event.cs,
                    miso: Miso::None,
                },
                nets,
            },
        })
        .collect();
    Chip {
        part: Some(part),
        outputs,
        watches: Vec::new(),
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: true,
        supply: None,
        draw: None,
        takes: Takes::default(),
        core: Core::Device(Box::new(Native::new(
            model::SpiReplay::new(&Fields::NONE).expect("a spi_replay takes no params"),
            Replay::new(records),
            model::OUTPUTS,
        ))),
    }
}

/// A replay of SPI transactions, an active chip with no clock of its own. It drives `CS` high
/// and `SCLK` and `D0` to `D3` low, through `output` from `VCC` and into `GND`, for the whole
/// run, and sends each of `events` as a `Spi` record at the event's end. Only parse builds one,
/// so the events start in order and none starts before the one before it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct SpiReplay {
    pub pins: [NetId; 8],
    /// Hertz, greater than 0, at most `u32::MAX`. Every event's SCLK.
    pub clock_hz: IntValue,
    pub output: OutputStage,
    pub events: Vec<SpiEvent>,
}

impl SpiReplay {
    pub const PINS: [&'static str; 8] = ["VCC", "GND", "CS", "SCLK", "D0", "D1", "D2", "D3"];
}

/// One transaction of a `spi_replay`: its phases, then `data` on `lanes`, from `at` to `end`.
/// `end` is `at` plus the transaction's SCLK cycles at the part's clock, rounded down to a
/// picosecond, and is after `at`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpiEvent {
    pub at: Time,
    pub end: Time,
    pub phases: Vec<SpiPhase>,
    pub lanes: Lanes,
    pub data: Arc<[u8]>,
    pub cs: Cs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSpiReplay {
    pub(crate) name: String,
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

pub(crate) fn resolve_spi_replay(ctx: ParseCtx<'_>, wire: WireSpiReplay) -> Option<SpiReplay> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
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
