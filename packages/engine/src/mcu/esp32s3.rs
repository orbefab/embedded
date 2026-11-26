//! The `esp32s3` part: its netlist struct and how parse reads it, its firmware and blobs, and
//! the outputs and watches its wired GPIOs give the circuit around `orbe_espressif::s3::Bridge`.

use serde::Deserialize;

use orbe_model::Time;

use crate::active::Chip;
use crate::blob::{Blob, Blobs, Role, Sha256};
use crate::diagnostic::{BlobRule, Diagnostic, FirmwareRule, ScheduleRule};
use crate::mna::Node;
use crate::name::Ref;
use crate::netlist::Event;
use crate::netlist::NetId;
use crate::netlist::PartId;
use crate::netlist::Threshold;
use crate::netlist::Value;
use crate::netlist::elf::{self, Elf};
use crate::netlist::json::ParseCtx;
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_limit, resolve_some_pins, resolve_value};
use crate::output_stage::{OutputStage, WireOutputStage, resolve_output_stage};
use crate::ratings::{Ceiling, PinRatings, PinWindow};
use crate::watch::{Signal, Watch};
use orbe_espressif::s3::{self, Bridge};

/// The pin pairs the chip joins at DC, for `net.floating`, by pin index: each GPIO to `VCC`
/// and to `GND` through its leakage. A part's GPIOs with nets take the first `2 * n`.
pub(crate) const DC_PATHS: [(usize, usize); 80] = {
    let mut paths = [(0, 0); 80];
    let mut k = 0;
    while k < 40 {
        paths[2 * k] = (2 + k, 0);
        paths[2 * k + 1] = (2 + k, 1);
        k += 1;
    }
    paths
};
const _: () = assert!(Esp32s3::PINS.len() == 2 + DC_PATHS.len() / 2);

pub(crate) fn dc_paths(mcu: &Esp32s3) -> &'static [(usize, usize)] {
    &DC_PATHS[..2 * (mcu.pins.len() - 2)]
}

/// Margin inside the strict rising supply comparator so an exact 3.0 V supply reads high.
const SUPPLY_BOUND_INSIDE_V: f64 = 1e-6;

/// USB Serial/JTAG internal PHY Full-Speed D+ pull-up on GPIO20 at `PULLUP_VALUE = 0` (reset):
/// typical 2.4 kohm per ESP-IDF v5.5.5 `soc/esp32s3/register/soc/usb_wrap_struct.h` and
/// `hal/esp32s3/include/hal/usb_serial_jtag_ll.h` (the internal FSLS PHY shared by USB
/// Serial/JTAG and USB OTG).
const USB_DP_PULL_UP_OHM: f64 = 2_400.0;

pub(crate) fn chip(part: PartId, mcu: &Esp32s3) -> Chip {
    let (vcc, gnd) = (mcu.pins[0], mcu.pins[1]);
    let gpios: Vec<u8> = mcu.gpios().collect();
    let pins = &mcu.pins[2..];
    let pads = super::Pads {
        vcc,
        gnd,
        output: &mcu.output,
        leakage: mcu.leakage_resistance.si.get(),
        pads: pins
            .iter()
            .zip(&gpios)
            .map(|(&net, &gpio)| super::Pad {
                net,
                pull_up: mcu.pull_up_resistance.si.get(),
                strong_pull_up: (gpio == 20).then_some(USB_DP_PULL_UP_OHM),
                pull_down: mcu
                    .pull_down_resistance
                    .as_ref()
                    .map(|value| value.si.get()),
                threshold: &mcu.input_threshold,
            })
            .collect(),
    };
    let (outputs, mut watches) = pads.build();
    // Supply watch with hysteresis:
    // - `rise` turns high once V(VCC) - V(GND) enters the recommended operating range
    //   (3.0 V min, ESP32-S3 Series Datasheet v2.2 Section 5.2, Table 5-2), offset 1 uV inside
    //   the strict comparator so an exact 3.0 V supply reads high.
    // - `fall` turns low at or below the brown-out threshold (2.44 V, ESP-IDF v5.5.5
    //   `CONFIG_ESP_BROWNOUT_DET_LVL_SEL_7`; ESP32-S3 Series Datasheet v2.2 Section 2.5.3).
    let supply = |offset| Signal {
        terms: vec![(Node::from(vcc), 1.0), (Node::from(gnd), -1.0)],
        offset,
    };
    watches.push(Watch {
        rise: supply(s3::OPERATING_SUPPLY_MIN_V - SUPPLY_BOUND_INSIDE_V),
        fall: supply(s3::MIN_SUPPLY_V),
    });
    // V(pin) - V(GND) of each ADC pad, in the order the bridge reads them.
    let samples = pins
        .iter()
        .zip(&gpios)
        .filter(|&(_, &gpio)| s3::adc_pad(gpio))
        .map(|(&pin, _)| Signal {
            terms: vec![(Node::from(pin), 1.0), (Node::from(gnd), -1.0)],
            offset: 0.0,
        })
        .collect();
    let mut signals: Vec<(&'static str, usize)> = mcu.names[2..]
        .iter()
        .enumerate()
        .map(|(n, &name)| (name, n))
        .collect();
    signals.sort_unstable();

    let mut chip = match &mcu.firmware {
        Esp32s3Firmware::Elf(elf) => {
            let segments = elf
                .segments()
                .iter()
                .map(|segment| (segment.addr, elf.data(segment)));
            s3::Esp32s3::new(&gpios, elf.entry(), segments, None)
        }
        Esp32s3Firmware::Flash { rom, app, .. } => {
            let boot = s3::Boot {
                rom: &rom.bytes,
                flash: app.flash.clone(),
                entry: app.entry,
                segments: &app.segments,
            };
            s3::Esp32s3::boot(&gpios, boot)
        }
    };
    // A part that names no resistor has nothing to stamp, so a netted pad's pull-down is refused
    // at the store instead of being dropped here.
    if mcu.pull_down_resistance.is_none() {
        chip.omit_pull_down();
    }
    chip.use_efuse(mcu.efuse);
    let console = mcu.console.iter().map(|send| (send.at, send.to.as_slice()));
    let bridge = Bridge::new(chip, gpios, console);
    let output_pins = mcu.names[2..].to_vec();
    super::chip(
        part,
        outputs,
        output_pins,
        watches,
        signals,
        samples,
        bridge,
    )
}

/// The part's voltage ratings on `VCC` and each wired GPIO, `None` when it has no `ratings`
/// object. Every window is over `GND`: `VCC` takes `-rail_margin` to `supply_voltage`, and each
/// wired GPIO takes `-rail_margin` to `VCC + rail_margin`.
pub(crate) fn pin_ratings(mcu: &Esp32s3) -> Option<PinRatings> {
    let ratings = mcu.ratings.as_ref()?;
    let si = |value: &Option<Value>| value.as_ref().map(|value| value.si.get());
    let margin = si(&ratings.rail_margin);
    let supply = si(&ratings.supply_voltage);
    let mut windows = Vec::new();
    if margin.is_some() || supply.is_some() {
        windows.push(PinWindow {
            pin: Esp32s3::PINS[0],
            net: mcu.pins[0],
            below: margin,
            above: supply.map(Ceiling::Fixed),
        });
    }
    if let Some(margin) = margin {
        for (&pin, &net) in mcu.names[2..].iter().zip(&mcu.pins[2..]) {
            windows.push(PinWindow {
                pin,
                net,
                below: Some(margin),
                above: Some(Ceiling::OverSupply(margin)),
            });
        }
    }
    Some(PinRatings {
        outputs: Vec::new(),
        pin_current: None,
        supply_current: None,
        vcc: (Esp32s3::PINS[0], mcu.pins[0]),
        gnd: (Esp32s3::PINS[1], mcu.pins[1]),
        windows,
    })
}

/// An ESP32-S3 running `firmware`: a bare-metal program whose core starts at its entry at
/// power-on, with every segment loaded, or the app on its flash, from where the bootloader
/// leaves it.
///
/// `VCC` and `GND` need nets, and every GPIO is optional. A GPIO with no net is neither stamped
/// nor watched. One with a net drives through `output` from `VCC` and into `GND`, pulls up
/// through `pull_up_resistance`, pulls down through `pull_down_resistance` when the part has
/// one, pulls both ways, or is off, as the firmware sets it. A part that names no pull-down
/// refuses a store that sets one on a pad with a net, and writes nothing. It leaks to `VCC`
/// and to `GND` through `leakage_resistance`, and reads its level through `input_threshold`.
/// Its ADCs read GPIO1 to GPIO20 against `GND`, through the calibration of the eFuse set `efuse`.
/// `console` is
/// what the host sends its console: each event's bytes, in order, after t = 0 and after the
/// previous event. Only parse builds one.
#[derive(Clone, Debug, PartialEq)]
pub struct Esp32s3 {
    /// `VCC`, `GND`, then each GPIO with a net, in `PINS` order.
    pub pins: Vec<NetId>,
    /// The name of each of `pins`.
    pub names: Vec<&'static str>,
    pub firmware: Esp32s3Firmware,
    pub output: OutputStage,
    /// Ohm. Every GPIO's pull-up.
    pub pull_up_resistance: Value,
    /// Ohm. Every GPIO's pull-down. `None` when the part names none: a store that sets `FUN_WPD`
    /// on a pad with a net ends the run, and a pad with no net keeps the bit.
    pub pull_down_resistance: Option<Value>,
    /// Ohm. Every GPIO's to `VCC` and again to `GND`, in every drive state.
    pub leakage_resistance: Value,
    pub input_threshold: Threshold,
    /// The chip's eFuses, by the name of a set the chip crate holds.
    pub efuse: s3::EfuseSet,
    pub console: Vec<Event<Vec<u8>>>,
    /// The pin voltage ratings the run checks. `None` when the part has no `ratings` object,
    /// which leaves the chip unrated.
    pub ratings: Option<Esp32s3Ratings>,
}

/// An ESP32-S3's voltage ratings. Each is `None` when the part leaves it unrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Esp32s3Ratings {
    /// Volt. How far `VCC` and each wired GPIO may sit below `GND`, and each wired GPIO above
    /// `VCC`.
    pub rail_margin: Option<Value>,
    /// Volt. The most `VCC` may sit above `GND`.
    pub supply_voltage: Option<Value>,
}

/// What an `esp32s3` runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Esp32s3Firmware {
    /// A bare-metal program in SRAM.
    Elf(elf::Elf),
    /// The app on the flash, booted with the mask ROM, and the flash's regions in netlist order.
    Flash {
        rom: Blob,
        regions: Vec<Blob>,
        app: s3::App,
    },
}

impl Esp32s3Firmware {
    /// The blobs it names: the ELF, or the ROM and then each flash region.
    pub fn blobs(&self) -> Vec<&Blob> {
        match self {
            Esp32s3Firmware::Elf(elf) => vec![elf.blob()],
            Esp32s3Firmware::Flash { rom, regions, .. } => {
                std::iter::once(rom).chain(regions).collect()
            }
        }
    }

    /// The symbol a diagnostic names `pc` by. A flash's app carries none.
    pub fn symbolize(&self, pc: u32) -> Option<String> {
        match self {
            Esp32s3Firmware::Elf(elf) => elf.symbolize(pc),
            Esp32s3Firmware::Flash { .. } => None,
        }
    }
}

impl Esp32s3 {
    /// The mask ROM the model runs, revision 0, as its SHA-256 and length.
    pub const ROM: (&'static str, u64) = (
        "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
        949_552,
    );

    /// The supply pins, then the GPIOs a board can wire, as the chip names them.
    pub const PINS: [&'static str; 42] = s3::PINS;
    /// How many of [`PINS`](Self::PINS) a part must wire: the supply pins, `VCC` and `GND`.
    pub(crate) const REQUIRED: usize = 2;

    /// The GPIO number of each of `pins` past `VCC` and `GND`.
    pub fn gpios(&self) -> impl Iterator<Item = u8> + '_ {
        self.names[2..].iter().map(|name| {
            name.strip_prefix("GPIO")
                .and_then(|number| number.parse().ok())
                .expect("a GPIO pin name")
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireEsp32s3 {
    pub(crate) name: String,
    pins: PinMap,
    #[serde(default)]
    rom: Option<WireBlob>,
    firmware: WireFirmware,
    output: WireOutputStage,
    pull_up_resistance: WireValue,
    #[serde(default)]
    pull_down_resistance: Option<WireValue>,
    leakage_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default = "default_efuse", deserialize_with = "efuse")]
    efuse: s3::EfuseSet,
    #[serde(default)]
    console: Vec<WireSend>,
    /// Optional, so a netlist written before the chip had ratings keeps its meaning.
    #[serde(default)]
    ratings: Option<WireRatings>,
}

/// Every key is required, and `null` leaves that rating out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    rail_margin: Option<WireValue>,
    #[serde(deserialize_with = "Option::deserialize")]
    supply_voltage: Option<WireValue>,
}

fn default_efuse() -> s3::EfuseSet {
    s3::EfuseSet::default()
}

/// The eFuse set a name picks. A section that does not parse is the same `netlist.syntax`
/// failure as an unknown name, with the parse error as the message, and it does not panic.
fn efuse<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<s3::EfuseSet, D::Error> {
    let name = String::deserialize(deserializer)?;
    match s3::EfuseSet::named(&name) {
        Ok(Some(set)) => Ok(set),
        Ok(None) => {
            let names: Vec<&str> = s3::EfuseSet::names().collect();
            Err(serde::de::Error::custom(format!(
                "`{name}` is not an eFuse set; the sets are {}",
                names.join(", ")
            )))
        }
        Err(error) => Err(serde::de::Error::custom(error.to_string())),
    }
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
    pub(crate) fn blob_names(&self) -> Vec<Sha256> {
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

pub(crate) fn resolve_esp32s3(ctx: ParseCtx<'_>, wire: WireEsp32s3) -> Option<Esp32s3> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        blobs,
        out,
        ..
    } = ctx;
    let ohm = |field, value, out: &mut Vec<Diagnostic>| {
        resolve_value(part_ref, field, "ohm", true, value, out)
    };
    // `VCC` and `GND`, then each GPIO the part has a net on.
    let pins = resolve_some_pins(
        name,
        &Esp32s3::PINS,
        Esp32s3::REQUIRED,
        &wire.pins.0,
        nets,
        out,
    );
    let firmware = resolve_firmware(part_ref, wire.rom, wire.firmware, blobs, out);
    let output = resolve_output_stage(part_ref, wire.output, out);
    let pull_up = ohm("pull_up_resistance", wire.pull_up_resistance, out);
    let pull_down = match wire.pull_down_resistance {
        None => Some(None),
        Some(value) => ohm("pull_down_resistance", value, out).map(Some),
    };
    let leakage = ohm("leakage_resistance", wire.leakage_resistance, out);
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let console = resolve_console(part_ref, wire.console, out);
    let ratings = match wire.ratings {
        None => Some(None),
        Some(wire) => resolve_ratings(part_ref, wire, out).map(Some),
    };
    let (pins, names) = pins?;
    Some(Esp32s3 {
        pins,
        names,
        firmware: firmware?,
        output: output?,
        pull_up_resistance: pull_up?,
        pull_down_resistance: pull_down?,
        leakage_resistance: leakage?,
        input_threshold: input_threshold?,
        efuse: wire.efuse,
        console: console?,
        ratings: ratings?,
    })
}

fn resolve_ratings(
    part: &Ref,
    wire: WireRatings,
    out: &mut Vec<Diagnostic>,
) -> Option<Esp32s3Ratings> {
    let rail_margin = resolve_limit(part, "ratings.rail_margin", "V", wire.rail_margin, out);
    let supply_voltage = resolve_limit(
        part,
        "ratings.supply_voltage",
        "V",
        wire.supply_voltage,
        out,
    );
    Some(Esp32s3Ratings {
        rail_margin: rail_margin?,
        supply_voltage: supply_voltage?,
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
            let windows = s3::RAM.map(|window| elf::Window {
                start: window.start,
                end: window.end,
                offset: window.offset,
                code: window.code,
            });
            match Elf::parse(blob, &windows) {
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
            let placed: Vec<(u32, &[u8])> = regions
                .iter()
                .map(|region| {
                    let offset = region.role.offset().expect("a flash region");
                    (offset, &region.bytes[..])
                })
                .collect();
            let rule = match s3::App::parse(&placed) {
                Ok(app) => return Some(Esp32s3Firmware::Flash { rom, regions, app }),
                Err(s3::ImageRule::Region) => FirmwareRule::Region,
                Err(s3::ImageRule::Image) => FirmwareRule::Image,
                Err(s3::ImageRule::Unmapped) => FirmwareRule::Unmapped,
            };
            fail(rule, out)
        }
        _ => fail(FirmwareRule::Rom, out),
    }
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
