//! The `esp32s3` kind: its supply pins and the GPIOs it has nets on, its firmware as a blob
//! reference to an ELF executable or as blobs placed on its flash with the mask ROM's blob, and
//! the values of its pin electrics.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::atmega328p::{WireThreshold, resolve_threshold};
use super::{PinMap, WireOutputStage, WireValue, resolve_output_stage, resolve_value};
use crate::blob::{Blob, Blobs, Role, Sha256};
use crate::diagnostic::{BlobRule, Diagnostic, FirmwareRule, ScheduleRule};
use crate::name::{Name, Ref};
use crate::netlist::elf::Elf;
use crate::netlist::flash::Flash;
use crate::netlist::{Esp32s3, Esp32s3Firmware, Event, NetId};
use crate::time::Time;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireEsp32s3 {
    pub(super) name: String,
    pins: PinMap,
    #[serde(default)]
    rom: Option<WireBlob>,
    firmware: WireFirmware,
    output: WireOutputStage,
    pull_up_resistance: WireValue,
    leakage_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default)]
    console: Vec<WireSend>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSend {
    t_ps: u64,
    bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum WireFirmware {
    Elf(WireBlob),
    Flash(Vec<WireRegion>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRegion {
    offset: u32,
    #[serde(deserialize_with = "sha256")]
    sha256: Sha256,
    bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBlob {
    #[serde(deserialize_with = "sha256")]
    sha256: Sha256,
    bytes: u64,
}

fn sha256<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Sha256, D::Error> {
    let text = String::deserialize(deserializer)?;
    Sha256::parse(&text)
        .ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not 64 lowercase hex digits")))
}

impl WireEsp32s3 {
    /// The SHA-256 of each blob it names: the ROM, then its firmware's.
    pub(super) fn blob_names(&self) -> Vec<Sha256> {
        let firmware = match &self.firmware {
            WireFirmware::Elf(elf) => vec![elf.sha256],
            WireFirmware::Flash(regions) => regions.iter().map(|region| region.sha256).collect(),
        };
        self.rom
            .iter()
            .map(|rom| rom.sha256)
            .chain(firmware)
            .collect()
    }
}

pub(super) fn resolve_esp32s3(
    name: &Name,
    part_ref: &Ref,
    wire: WireEsp32s3,
    nets: &BTreeMap<String, NetId>,
    blobs: &Blobs,
    out: &mut Vec<Diagnostic>,
) -> Option<Esp32s3> {
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    let pins = resolve_pins(name, &wire.pins.0, nets, out);
    let firmware = resolve_firmware(part_ref, wire.rom, wire.firmware, blobs, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let pull_up = ohm("pull_up_resistance", wire.pull_up_resistance, out);
    let leakage = ohm("leakage_resistance", wire.leakage_resistance, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let console = resolve_console(part_ref, wire.console, out);
    let (pins, names) = pins?;
    Some(Esp32s3 {
        pins,
        names,
        firmware: firmware?,
        output: output?,
        pull_up_resistance: pull_up?,
        leakage_resistance: leakage?,
        input_threshold: input_threshold?,
        console: console?,
    })
}

/// The console's sends, each after t = 0, after the previous one, and with a byte.
fn resolve_console(
    part: &Ref,
    wires: Vec<WireSend>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<Event<Vec<u8>>>> {
    let mut ok = true;
    let mut previous = None;
    let mut events = Vec::with_capacity(wires.len());
    for (index, wire) in wires.into_iter().enumerate() {
        let rule = if previous.is_some_and(|previous| wire.t_ps <= previous) {
            Some(ScheduleRule::NotIncreasing)
        } else if wire.t_ps == 0 {
            Some(ScheduleRule::AtZero)
        } else if wire.bytes.is_empty() {
            Some(ScheduleRule::SameState)
        } else {
            None
        };
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: "console",
                event: index,
                rule,
                end_ps: 0,
            });
        }
        previous = Some(wire.t_ps);
        events.push(Event {
            at: Time::from_ps(wire.t_ps),
            to: wire.bytes,
        });
    }
    ok.then_some(events)
}

/// The firmware `wire` names, with `rom` beside a flash and absent beside an ELF.
fn resolve_firmware(
    part: &Ref,
    rom: Option<WireBlob>,
    wire: WireFirmware,
    blobs: &Blobs,
    out: &mut Vec<Diagnostic>,
) -> Option<Esp32s3Firmware> {
    let fail = |rule, out: &mut Vec<Diagnostic>| {
        out.push(Diagnostic::NetlistFirmware {
            part: part.clone(),
            line: None,
            rule,
        });
        None
    };
    match (rom, wire) {
        (None, WireFirmware::Elf(elf)) => {
            let blob = find(part, Role::Elf, &elf, blobs, out)?;
            match Elf::parse(blob, &Esp32s3::RAM) {
                Ok(elf) => Some(Esp32s3Firmware::Elf(elf)),
                Err(rule) => fail(rule, out),
            }
        }
        (Some(rom), WireFirmware::Flash(wire_regions)) => {
            let (sha256, bytes) = Esp32s3::ROM;
            if rom.sha256.to_string() != sha256 || rom.bytes != bytes {
                return fail(FirmwareRule::Rom, out);
            }
            let rom = find(part, Role::Rom, &rom, blobs, out);
            let mut regions = Vec::new();
            for region in wire_regions {
                let wire = WireBlob {
                    sha256: region.sha256,
                    bytes: region.bytes,
                };
                regions.push(find(part, Role::Flash(region.offset), &wire, blobs, out));
            }
            let regions: Option<Vec<_>> = regions.into_iter().collect();
            let (rom, regions) = (rom?, regions?);
            match Flash::parse(regions) {
                Ok(flash) => Some(Esp32s3Firmware::Flash { rom, flash }),
                Err(rule) => fail(rule, out),
            }
        }
        _ => fail(FirmwareRule::Rom, out),
    }
}

/// `VCC` and `GND`, then each GPIO the part has a net on, in [`Esp32s3::PINS`] order.
fn resolve_pins(
    part: &Name,
    pins: &BTreeMap<String, String>,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<(Vec<NetId>, Vec<&'static str>)> {
    let (mut ids, mut names) = (Vec::new(), Vec::new());
    let mut resolved = true;
    for &pin in &Esp32s3::PINS {
        let Some(net) = pins.get(pin) else {
            continue;
        };
        match nets.get(net) {
            Some(&id) => {
                ids.push(id);
                names.push(pin);
            }
            None => {
                resolved = false;
                out.push(Diagnostic::NetlistUnknownNet {
                    pin: Ref::Pin(part.clone(), Name::new(pin).expect("kind pin name")),
                    net: net.clone(),
                });
            }
        }
    }
    let missing: Vec<&'static str> = Esp32s3::PINS[..2]
        .iter()
        .copied()
        .filter(|pin| !pins.contains_key(*pin))
        .collect();
    let mut unexpected = Vec::new();
    for (key, net) in pins {
        if Esp32s3::PINS.contains(&key.as_str()) {
            continue;
        }
        unexpected.push(key.clone());
        if let Ok(pin) = Name::new(key)
            && !nets.contains_key(net)
        {
            out.push(Diagnostic::NetlistUnknownNet {
                pin: Ref::Pin(part.clone(), pin),
                net: net.clone(),
            });
        }
    }
    if !missing.is_empty() || !unexpected.is_empty() {
        out.push(Diagnostic::NetlistPins {
            part: Ref::Part(part.clone()),
            missing,
            unexpected,
        });
        resolved = false;
    }
    resolved.then_some((ids, names))
}

/// The blob `wire` names, when `blobs` has it under that name, it hashes to that name, and its
/// length is the netlist's.
fn find(
    part: &Ref,
    role: Role,
    wire: &WireBlob,
    blobs: &Blobs,
    out: &mut Vec<Diagnostic>,
) -> Option<Blob> {
    let mut fail = |rule, found| {
        out.push(Diagnostic::NetlistBlob {
            part: part.clone(),
            role,
            sha256: wire.sha256,
            rule,
            bytes: wire.bytes,
            found,
            source: blobs.source(),
        });
        None
    };
    let Some(bytes) = blobs.get(&wire.sha256) else {
        return fail(BlobRule::Missing, None);
    };
    let found = Some(bytes.len() as u64);
    if Sha256::of(bytes) != wire.sha256 {
        return fail(BlobRule::Hash, found);
    }
    if found != Some(wire.bytes) {
        return fail(BlobRule::Length, found);
    }
    Some(Blob {
        role,
        sha256: wire.sha256,
        bytes: bytes.clone(),
    })
}
