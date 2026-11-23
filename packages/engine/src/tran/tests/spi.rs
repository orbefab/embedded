//! SPI read-back. On the ATmega328P, `spi_read.S` sends 0x9F and three more bytes to a target on
//! its SPI pins, and puts the three bytes it read on PORTD in turn. On the ESP32-S3, a program
//! sends 0x9F as SPI2's command phase, reads three bytes into `SPI_W0`, and raises GPIO21 when
//! they are the ones the target answered.

use orbe_model::{Cx, Device, Drives, Error, Params, Spi, SpiPhase, SpiTarget, State, Stop, Time};

use std::sync::Arc;

use super::active::{US, at, net, resistor, run_active, volts};
use super::avr::{FUSES, MHZ_16, atmega, board};
use crate::active::native::Native;
use crate::active::spi::{self, Bus};
use crate::active::spi_pins::{Lines, Pins};
use crate::active::{self, Core, Output, SpiTarget as Target, Stage, Takes};
use crate::blob::Blobs;
use crate::diagnostic::Severity;
use crate::mna::Node;
use crate::netlist::Netlist;
use crate::watch::{Signal, Watch};

const SPI_READ: &str = include_str!("../../../tests/firmware/spi_read.hex");

/// What the target answers to 0x9F, as a flash chip answers its JEDEC ID command.
const ID: [u8; 3] = [0xEF, 0x40, 0x18];

#[derive(Clone, Copy, Debug)]
struct JedecId;

impl State for JedecId {}

impl Device for JedecId {
    fn new(_params: &Params<'_>) -> Result<JedecId, Error> {
        Ok(JedecId)
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }
}

impl SpiTarget for JedecId {
    fn transaction(&mut self, _cx: &mut Cx, _at: Time, _spi: &Spi) {}

    /// The command is the first phase, or the first data byte when there is none.
    fn answer(
        &mut self,
        _cx: &mut Cx,
        _at: Time,
        phases: &[SpiPhase],
        data: &[u8],
        index: usize,
    ) -> Option<u8> {
        let (command, after) = match phases {
            [command, ..] => (command.value, index),
            [] => (u32::from(*data.first()?), index.checked_sub(1)?),
        };
        if command == 0x9F {
            ID.get(after).copied()
        } else {
            None
        }
    }
}

/// The target on PB2 as CS, PB5 as SCLK and PB3 as MOSI, each read at 2.5 V, driving PB4, MISO,
/// through 25 ohm.
fn target(netlist: &Netlist) -> active::Chip {
    let watch = |name| {
        Watch::comparator(Signal {
            terms: vec![(Node::from(net(netlist, name)), 1.0)],
            offset: 2.5,
        })
    };
    let stage = Stage {
        drop: 0.0,
        resistance: 25.0,
    };
    active::Chip {
        part: None,
        outputs: vec![Output {
            pin: net(netlist, "pb4"),
            vcc: net(netlist, "vcc"),
            gnd: net(netlist, "gnd"),
            high: stage,
            low: stage,
            pull_up: None,
            pull_down: None,
            leakage: None,
        }],
        watches: vec![watch("pb2"), watch("pb5"), watch("pb3")],
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        takes: Takes::default(),
        core: Core::Device(Box::new(Native::new(
            JedecId,
            Pins::new(Lines {
                cs: 0,
                sclk: 1,
                mosi: 2,
            }),
            0,
        ))),
    }
}

/// Each byte PORTD held, in order, once per change.
fn portd(trace: &crate::trace::Trace) -> Vec<u8> {
    let pins: Vec<Vec<(u64, f64)>> = (0..8).map(|k| volts(trace, &format!("pd{k}"))).collect();
    let mut bytes: Vec<u8> = Vec::new();
    for row in 0..pins[0].len() {
        let byte = pins
            .iter()
            .enumerate()
            .fold(0, |byte, (k, pin)| byte | u8::from(pin[row].1 > 2.5) << k);
        if bytes.last() != Some(&byte) {
            bytes.push(byte);
        }
    }
    bytes
}

#[test]
fn the_firmware_reads_back_the_bytes_the_target_answers() {
    let netlist = board(
        60 * US,
        &[],
        &[
            resistor("rcs", "vcc", "pb2", 10e3),
            resistor("rmiso", "vcc", "pb4", 10e3),
        ],
        atmega("vcc", SPI_READ, MHZ_16, FUSES),
    );
    let target = target(&netlist);
    let (trace, diagnostics) = run_active(&netlist, vec![target]);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity() != Severity::Error),
        "{diagnostics:?}"
    );
    // PORTD reads 0 until the firmware drives it, then each byte the target answered.
    assert_eq!(portd(&trace), [0x00, 0xEF, 0x40, 0x18]);
}

const IRAM: u32 = 0x4037_0000;
const GPIO: u32 = 0x6000_4000;
const IO_MUX: u32 = 0x6000_9000;
const SPI2: u32 = 0x6002_4000;

/// Xtensa code after literal words, each op as `Program` in `orbe-espressif`'s tests encodes it.
struct Program {
    literals: Vec<u32>,
    code: Vec<u8>,
}

impl Program {
    fn pc(&self) -> u32 {
        IRAM + 4 * self.literals.len() as u32 + self.code.len() as u32
    }

    fn l32r(&mut self, t: u8, literal: usize) -> &mut Program {
        let target = IRAM + 4 * literal as u32;
        let imm = (target.wrapping_sub((self.pc() + 3) & !3) as i32 >> 2) as u16;
        self.code.extend([t << 4 | 1, imm as u8, (imm >> 8) as u8]);
        self
    }

    fn s32i(&mut self, t: u8, s: u8, offset: u16) -> &mut Program {
        self.code
            .extend([t << 4 | 2, 6 << 4 | s, (offset / 4) as u8]);
        self
    }

    fn l32i(&mut self, t: u8, s: u8, offset: u16) -> &mut Program {
        self.code
            .extend([t << 4 | 2, 2 << 4 | s, (offset / 4) as u8]);
        self
    }

    fn movi(&mut self, t: u8, value: u8) -> &mut Program {
        self.code.extend([t << 4 | 2, 0xa0, value]);
        self
    }

    /// `and ar, as, at`.
    fn and(&mut self, r: u8, s: u8, t: u8) -> &mut Program {
        self.code.extend([t << 4, r << 4 | s, 0x10]);
        self
    }

    fn bnez(&mut self, s: u8, to: u32) -> &mut Program {
        let imm = to.wrapping_sub(self.pc() + 4);
        self.code
            .extend([0x56, s | (imm as u8 & 0xf) << 4, (imm >> 4) as u8]);
        self
    }

    /// `bne as, at` over the next `skip` bytes.
    fn bne_over(&mut self, s: u8, t: u8, skip: u8) -> &mut Program {
        self.code.extend([t << 4 | 7, 9 << 4 | s, skip - 1]);
        self
    }

    /// An executable that loads the literals and the code at `IRAM`, and starts at the code.
    fn elf(&self) -> Vec<u8> {
        let mut image: Vec<u8> = self.literals.iter().flat_map(|w| w.to_le_bytes()).collect();
        let entry = IRAM + image.len() as u32;
        image.extend(&self.code);
        crate::netlist::elf::tests::build(
            entry,
            &[(IRAM, &image, image.len() as u32)],
            &[("main", entry)],
        )
    }
}

/// Routes CS to GPIO10, MOSI to GPIO11 and SCLK to GPIO12, reads MISO on GPIO13, and runs one
/// SPI2 transfer: an 8-bit command 0x9F, then a 3-byte read. Raises GPIO21 when `SPI_W0` then
/// holds `ID`.
fn jedec_id_program() -> Program {
    let mut program = Program {
        literals: vec![
            IO_MUX,
            1 << 12,
            1 << 12 | 1 << 9,
            GPIO + 0x500,
            GPIO,
            1 << 7 | 13,
            SPI2,
            1 << 31 | 1 << 28,
            7 << 28 | 0x9F,
            8 * 3 - 1,
            0x3,
            1 << 23,
            1 << 24,
            u32::from_le_bytes([ID[0], ID[1], ID[2], 0]),
            1 << 21,
        ],
        code: Vec::new(),
    };
    // Each pad on the matrix, GPIO13's input on.
    program
        .l32r(2, 0)
        .l32r(3, 1)
        .s32i(3, 2, 0x04 + 4 * 10)
        .s32i(3, 2, 0x04 + 4 * 11)
        .s32i(3, 2, 0x04 + 4 * 12)
        .l32r(3, 2)
        .s32i(3, 2, 0x04 + 4 * 13);
    // FSPICS0, FSPID and FSPICLK out, and FSPIQ in from GPIO13.
    program
        .l32r(2, 3)
        .movi(3, 110)
        .s32i(3, 2, 0x54 + 4 * 10)
        .movi(3, 103)
        .s32i(3, 2, 0x54 + 4 * 11)
        .movi(3, 101)
        .s32i(3, 2, 0x54 + 4 * 12)
        .l32r(2, 4)
        .l32r(3, 5)
        .s32i(3, 2, 0x154 + 4 * 102);
    // `SPI_USER`, `SPI_USER2`, `SPI_MS_DLEN` and `SPI_CLK_GATE`, then `SPI_UPDATE` and `SPI_USR`.
    program
        .l32r(2, 6)
        .l32r(3, 7)
        .s32i(3, 2, 0x10)
        .l32r(3, 8)
        .s32i(3, 2, 0x18)
        .l32r(3, 9)
        .s32i(3, 2, 0x1c)
        .l32r(3, 10)
        .s32i(3, 2, 0xe8)
        .l32r(3, 11)
        .s32i(3, 2, 0x00)
        .l32r(4, 12)
        .s32i(4, 2, 0x00);
    let poll = program.pc();
    program.l32i(3, 2, 0x00).and(5, 3, 4).bnez(5, poll);
    // `SPI_W0` against `ID`, then GPIO21 out and high.
    program
        .l32i(3, 2, 0x98)
        .l32r(6, 13)
        .bne_over(3, 6, 12)
        .l32r(2, 4)
        .l32r(3, 14)
        .s32i(3, 2, 0x24)
        .s32i(3, 2, 0x08);
    program.code.extend([0x06, 0xff, 0xff]);
    program
}

/// A 3.3 V ESP32-S3 running `elf`, with CS, MOSI, SCLK and MISO on GPIO10 to GPIO13 and `ok` on
/// GPIO21.
fn esp32s3(elf: Vec<u8>) -> Netlist {
    let mut blobs = Blobs::new();
    let bytes = elf.len();
    let sha256 = blobs.add(Arc::from(elf));
    let ohm = |value: f64| format!(r#"{{ "value": {value:?}, "unit": "ohm" }}"#);
    let json = format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {US}, "grid_ps": {US} }} }},
          "ground": "gnd",
          "nets": ["gnd", "vdd", "cs", "mosi", "sclk", "miso", "ok"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vdd", "n": "gnd" }},
              "voltage": {{ "value": 3.3, "unit": "V" }} }},
            {{ "name": "u1", "kind": "esp32s3",
              "pins": {{ "VCC": "vdd", "GND": "gnd", "GPIO10": "cs", "GPIO11": "mosi",
                        "GPIO12": "sclk", "GPIO13": "miso", "GPIO21": "ok" }},
              "firmware": {{ "elf": {{ "sha256": "{sha256}", "bytes": {bytes} }} }},
              "output": {{ "high_drop": {{ "value": 0.0, "unit": "V" }}, "high_resistance": {r},
                          "low_drop": {{ "value": 0.0, "unit": "V" }}, "low_resistance": {r} }},
              "pull_up_resistance": {pull},
              "leakage_resistance": {leak},
              "input_threshold": {{ "low": {{ "value": 0.25, "unit": "1" }},
                                   "high": {{ "value": 0.75, "unit": "1" }} }} }}
          ]
        }}"#,
        stop = 20 * US,
        r = ohm(17.0),
        pull = ohm(45e3),
        leak = ohm(66e6),
    );
    crate::netlist::parse_with(json.as_bytes(), &blobs).unwrap()
}

/// The target on the S3's SPI2 records, with D0 on MOSI and D1 on MISO.
fn record_target(netlist: &Netlist) -> active::Chip {
    let gnd = net(netlist, "gnd");
    let bus = Bus {
        data: [net(netlist, "mosi"), net(netlist, "miso"), gnd, gnd],
        reset: None,
    };
    active::Chip {
        part: None,
        outputs: Vec::new(),
        watches: Vec::new(),
        signals: Vec::new(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        supply: None,
        takes: Takes {
            spi: Some(Target {
                cs: net(netlist, "cs"),
                sclk: net(netlist, "sclk"),
            }),
            ..Takes::default()
        },
        core: Core::Device(Box::new(Native::new(JedecId, spi::Engine::new(bus), 0))),
    }
}

#[test]
fn the_s3_reads_the_bytes_the_target_answers_into_spi_w0() {
    let netlist = esp32s3(jedec_id_program().elf());
    let target = record_target(&netlist);
    let (trace, diagnostics) = run_active(&netlist, vec![target]);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity() != Severity::Error),
        "{diagnostics:?}"
    );
    let ok = volts(&trace, "ok");
    assert!(at(&ok, 20 * US) > 3.0, "GPIO21 is high: {:?}", ok.last());
}

/// With no target on its bus the read finds MISO undriven, `SPI_W0` is not `ID`, and GPIO21
/// stays an input, at half the supply through its leakage.
#[test]
fn the_s3_reads_no_id_with_no_target() {
    let netlist = esp32s3(jedec_id_program().elf());
    let (trace, _) = run_active(&netlist, Vec::new());
    let ok = volts(&trace, "ok");
    assert!(
        at(&ok, 20 * US) < 2.0,
        "GPIO21 is not high: {:?}",
        ok.last()
    );
}
