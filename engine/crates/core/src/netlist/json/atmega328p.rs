//! The `atmega328p` kind: its pins, its firmware as Intel HEX text, its clock and fuses, and
//! the values of its pin electrics.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{
    PinMap, WireInt, WireOutputStage, WireValue, resolve_fraction, resolve_int,
    resolve_output_stage, resolve_pins, resolve_value,
};
use crate::diagnostic::{Diagnostic, FirmwareRule, ValueRule};
use crate::name::{Name, Ref};
use crate::netlist::{Atmega328p, Firmware, Fuses, NetId, Threshold};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireAtmega328p {
    pub(super) name: String,
    pins: PinMap,
    firmware: String,
    clock_hz: WireInt<u32>,
    fuses: WireFuses,
    output: WireOutputStage,
    pull_up_resistance: WireValue,
    reset_pull_up_resistance: WireValue,
    leakage_resistance: WireValue,
    input_threshold: WireThreshold,
    reset_threshold: WireThreshold,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFuses {
    low: u8,
    high: u8,
    extended: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireThreshold {
    low: WireValue,
    high: WireValue,
}

pub(super) fn resolve_atmega328p(
    name: &Name,
    part_ref: &Ref,
    wire: WireAtmega328p,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Atmega328p> {
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    let pins = resolve_pins(name, Atmega328p::PINS, &wire.pins.0, nets, out)
        .and_then(|pins| one_ground(name, pins, nets, out));
    let firmware = match flash(&wire.firmware) {
        Ok(flash) => Some(Firmware { flash }),
        Err((line, rule)) => {
            out.push(Diagnostic::NetlistFirmware {
                part: part_ref.clone(),
                line,
                rule,
            });
            None
        }
    };
    let clock_hz = resolve_int(part_ref, "clock_hz", "Hz", wire.clock_hz, out);
    let fuses = resolve_fuses(part_ref, &wire.fuses, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let pull_up = ohm("pull_up_resistance", wire.pull_up_resistance, out);
    let reset_pull_up = ohm(
        "reset_pull_up_resistance",
        wire.reset_pull_up_resistance,
        out,
    );
    let leakage = ohm("leakage_resistance", wire.leakage_resistance, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let reset_threshold = resolve_threshold(
        part_ref,
        ["reset_threshold.low", "reset_threshold.high"],
        wire.reset_threshold,
        out,
    );
    Some(Atmega328p {
        pins: pins?,
        firmware: firmware?,
        clock_hz: clock_hz?,
        fuses: fuses?,
        output: output?,
        pull_up_resistance: pull_up?,
        reset_pull_up_resistance: reset_pull_up?,
        leakage_resistance: leakage?,
        input_threshold: input_threshold?,
        reset_threshold: reset_threshold?,
    })
}

/// Figure 1-1 names pins 8 and 22 `GND`, one function, so they name one net.
fn one_ground(
    part: &Name,
    pins: [NetId; 28],
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<[NetId; 28]> {
    let index = |pin| Atmega328p::PINS.iter().position(|&p| p == pin).unwrap();
    let (gnd_8, gnd_22) = (pins[index("GND_8")], pins[index("GND_22")]);
    if gnd_8 == gnd_22 {
        return Some(pins);
    }
    let name = |id| {
        let (net, _) = nets.iter().find(|&(_, &net)| net == id).unwrap();
        Name::new(net).expect("a resolved net name")
    };
    let pin = |pin| Ref::Pin(part.clone(), Name::new(pin).expect("kind pin name"));
    out.push(Diagnostic::NetlistSameNet {
        pin: pin("GND_22"),
        with: pin("GND_8"),
        found: name(gnd_22),
        expected: name(gnd_8),
    });
    None
}

/// Each byte the model would not run is `unsupported`: a `CKSEL` outside the crystal and
/// ceramic resonator rows of Table 9-1 or a programmed `CKOUT` (Table 28-9), a programmed
/// `DWEN` or `BOOTRST` (Table 28-8), or a reserved `BODLEVEL` (Table 29-12).
fn resolve_fuses(part: &Ref, wire: &WireFuses, out: &mut Vec<Diagnostic>) -> Option<Fuses> {
    let checks = [
        (
            "fuses.low",
            wire.low,
            matches!(wire.low & 0x0F, 0b0110..=0b1111) && wire.low & 0x40 != 0,
        ),
        ("fuses.high", wire.high, wire.high & 0x41 == 0x41),
        (
            "fuses.extended",
            wire.extended,
            matches!(wire.extended & 0x07, 0b100..=0b111),
        ),
    ];
    let mut ok = true;
    for (field, byte, supported) in checks {
        if !supported {
            ok = false;
            out.push(Diagnostic::NetlistValue {
                part: part.clone(),
                field,
                value: f64::from(byte),
                rule: ValueRule::Unsupported,
            });
        }
    }
    ok.then_some(Fuses {
        low: wire.low,
        high: wire.high,
        extended: wire.extended,
    })
}

fn resolve_threshold(
    part: &Ref,
    [low_field, high_field]: [&'static str; 2],
    wire: WireThreshold,
    out: &mut Vec<Diagnostic>,
) -> Option<Threshold> {
    let low = resolve_fraction(part, low_field, wire.low, out);
    let high = resolve_fraction(part, high_field, wire.high, out);
    let (low, high) = (low?, high?);
    if low.si.get() > high.si.get() {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: low_field,
            value: low.si.get(),
            rule: ValueRule::AboveHigh,
        });
        return None;
    }
    Some(Threshold { low, high })
}

/// Flash from Intel HEX text, one record a line. Records 00 to 05 are read: 02 and 04 move the
/// base address, and 01, 03 and 05 may name a start address, which a part at reset does not
/// use. A gap is erased flash, `0xFF`, and no byte is written twice. A fault names its 1-based
/// line.
fn flash(text: &str) -> Result<Vec<u8>, (Option<usize>, FirmwareRule)> {
    let mut flash = Vec::new();
    let mut placed = Vec::new();
    let mut base = 0;
    let mut ended = false;
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let fault = |rule| (Some(index + 1), rule);
        if ended {
            return Err(fault(FirmwareRule::AfterEnd));
        }
        let bytes = record(line).ok_or(fault(FirmwareRule::Record))?;
        if bytes.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte)) != 0 {
            return Err(fault(FirmwareRule::Checksum));
        }
        let address = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
        let data = &bytes[4..bytes.len() - 1];
        let upper = |shift: u32| match *data {
            [high, low] => Ok(usize::from(u16::from_be_bytes([high, low])) << shift),
            _ => Err(fault(FirmwareRule::Record)),
        };
        match bytes[3] {
            0x00 => {
                let start = base + address;
                let end = start + data.len();
                if end > Atmega328p::FLASH_BYTES {
                    return Err(fault(FirmwareRule::PastFlash));
                }
                if flash.len() < end {
                    flash.resize(end, 0xFF);
                    placed.resize(end, false);
                }
                if placed[start..end].contains(&true) {
                    return Err(fault(FirmwareRule::Overlap));
                }
                flash[start..end].copy_from_slice(data);
                placed[start..end].fill(true);
            }
            0x01 if data.is_empty() => ended = true,
            0x01 => return Err(fault(FirmwareRule::Record)),
            0x02 => base = upper(4)?,
            0x04 => base = upper(16)?,
            0x03 | 0x05 if data.len() == 4 => {}
            0x03 | 0x05 => return Err(fault(FirmwareRule::Record)),
            _ => return Err(fault(FirmwareRule::RecordType)),
        }
    }
    if !ended {
        return Err((None, FirmwareRule::NoEnd));
    }
    Ok(flash)
}

/// The record's bytes, its count through its checksum, when its length matches its count.
fn record(line: &str) -> Option<Vec<u8>> {
    let digits = line.strip_prefix(':')?.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Vec<u8> = digits
        .chunks(2)
        .map(|pair| {
            let nibble = |digit: u8| char::from(digit).to_digit(16);
            Some((nibble(pair[0])? << 4 | nibble(pair[1])?) as u8)
        })
        .collect::<Option<_>>()?;
    (bytes.len() >= 5 && bytes.len() == usize::from(bytes[0]) + 5).then_some(bytes)
}
