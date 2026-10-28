//! The `esp32s3` kind: its supply pins and the GPIOs it has nets on, its firmware as a blob
//! reference to an ELF executable, and the values of its pin electrics.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::atmega328p::{WireThreshold, resolve_threshold};
use super::{PinMap, WireOutputStage, WireValue, resolve_output_stage, resolve_value};
use crate::blob::{Blob, Blobs, Role, Sha256};
use crate::diagnostic::{BlobRule, Diagnostic};
use crate::name::{Name, Ref};
use crate::netlist::elf::Elf;
use crate::netlist::{Esp32s3, NetId};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireEsp32s3 {
    pub(super) name: String,
    pins: PinMap,
    firmware: WireFirmware,
    output: WireOutputStage,
    pull_up_resistance: WireValue,
    leakage_resistance: WireValue,
    input_threshold: WireThreshold,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum WireFirmware {
    Elf(WireBlob),
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
    /// The SHA-256 its firmware names.
    pub(super) fn firmware(&self) -> Sha256 {
        let WireFirmware::Elf(elf) = &self.firmware;
        elf.sha256
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
    let WireFirmware::Elf(elf) = wire.firmware;
    let firmware = find(part_ref, Role::Elf, &elf, blobs, out).and_then(|blob| {
        Elf::parse(blob, &Esp32s3::RAM)
            .map_err(|rule| {
                out.push(Diagnostic::NetlistFirmware {
                    part: part_ref.clone(),
                    line: None,
                    rule,
                })
            })
            .ok()
    });
    let output = resolve_output_stage(part_ref, wire.output, out);
    let pull_up = ohm("pull_up_resistance", wire.pull_up_resistance, out);
    let leakage = ohm("leakage_resistance", wire.leakage_resistance, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let (pins, names) = pins?;
    Some(Esp32s3 {
        pins,
        names,
        firmware: firmware?,
        output: output?,
        pull_up_resistance: pull_up?,
        leakage_resistance: leakage?,
        input_threshold: input_threshold?,
    })
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
