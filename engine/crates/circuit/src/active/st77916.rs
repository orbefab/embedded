//! The `st77916` panel controller behind the seam, an active chip with no clock of its own. It
//! takes `Spi` records on its CS and SCLK nets and `Duty` records on BLK, reads RST through a
//! watch, and holds TE low. It never stops and never changes its image.
//!
//! A command is one 32-bit word, `opcode << 24 | command << 8`, in the phases before the data.
//! `RAMWR` and `RAMWRC` take opcode 0x32 and four data lanes, and every other command takes
//! opcode 0x02 and one. The opcode is checked on every command, and the lanes only when a
//! transaction carries bytes. A transaction that ends with CS held leaves its command open, and
//! the next data-only transaction continues it. The command closes when CS is released, when a new
//! command arrives, or when RST goes low. A register command runs when it closes. A pixel write
//! puts each pixel in the frame memory as it arrives and adds a rectangle row when it closes.

mod glass;

use std::collections::BTreeMap;
use std::sync::Arc;

use sim_core::diagnostic::ProtocolRule;
use sim_core::display::{Brightness, Flags, Panel, RectangleHash};
use sim_core::link::{Cs, Duty, Lanes, LinkRow, RecordKind, Spi, SpiPhase};
use sim_core::netlist::{NetId, PartId, St77916, Value};
use sim_core::time::Time;

use super::{Chip, Drive, Output, SpiTarget, Stage, Takes};
use crate::mna::Node;
use crate::watch::{Signal, Watch};

pub use glass::glass;

const VCC: usize = 0;
const GND: usize = 1;
const CS: usize = 2;
const SCLK: usize = 3;
const D0: usize = 4;
const RST: usize = 8;
const TE: usize = 9;
const BLK: usize = 10;

/// The panel's pixels across and down.
pub const WIDTH: u16 = 360;
pub const HEIGHT: u16 = 360;
const ROW_BYTES: usize = WIDTH as usize * 2;
/// Rows of the frame memory in one band. A commit copies only the bands a write touched.
const BAND_ROWS: usize = 8;
const BAND_BYTES: usize = BAND_ROWS * ROW_BYTES;
const BANDS: usize = (HEIGHT as usize).div_ceil(BAND_ROWS);

/// TE, held low.
const IMAGE: [Drive; 1] = [Drive::Low];

/// The pin pairs the chip joins at DC, for `net.floating`: TE to `VCC` and to `GND`.
pub(crate) const DC_PATHS: [(usize, usize); 2] = [(TE, VCC), (TE, GND)];

const OPCODE_REGISTER: u32 = 0x02;
const OPCODE_PIXELS: u32 = 0x32;

/// MIPI DCS commands the model runs.
const SWRESET: u8 = 0x01;
const SLPIN: u8 = 0x10;
const SLPOUT: u8 = 0x11;
const INVOFF: u8 = 0x20;
const INVON: u8 = 0x21;
const DISPOFF: u8 = 0x28;
const DISPON: u8 = 0x29;
const CASET: u8 = 0x2a;
const RASET: u8 = 0x2b;
const RAMWR: u8 = 0x2c;
const MADCTL: u8 = 0x36;
const COLMOD: u8 = 0x3a;
const RAMWRC: u8 = 0x3c;

/// MADCTL's row order, column order and row and column exchange bits.
const MY: u8 = 0x80;
const MX: u8 = 0x40;
const MV: u8 = 0x20;

/// COLMOD after a reset: 18 bits a pixel.
const COLMOD_RESET: u8 = 0x66;

const RULES: [ProtocolRule; 3] = [
    ProtocolRule::Command,
    ProtocolRule::SpiContinuation,
    ProtocolRule::PixelFormat,
];

pub(crate) fn chip(part: PartId, panel: &St77916) -> Chip {
    let pins = panel.pins;
    let stage = |drop: &Value, resistance: &Value| Stage {
        drop: drop.si.get(),
        resistance: resistance.si.get(),
    };
    let output = Output {
        pin: pins[TE],
        vcc: pins[VCC],
        gnd: pins[GND],
        high: stage(&panel.output.high_drop, &panel.output.high_resistance),
        low: stage(&panel.output.low_drop, &panel.output.low_resistance),
        pull_up: None,
        leakage: None,
    };
    // V(RST) - f * V(VCC) - (1 - f) * V(GND): RST against a fraction of the chip's supply.
    let against = |fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[RST]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let threshold = &panel.input_threshold;
    Chip {
        part: Some(part),
        outputs: vec![output],
        watches: vec![Watch {
            rise: against(threshold.high.si.get()),
            fall: against(threshold.low.si.get()),
        }],
        signals: vec![(St77916::PINS[RST], 0)],
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        takes: Takes {
            spi: Some(SpiTarget {
                cs: pins[CS],
                sclk: pins[SCLK],
            }),
            duty: Some(pins[BLK]),
            pulses: None,
        },
        core: super::Core::St77916(Box::new(Core::new(
            [D0, D0 + 1, D0 + 2, D0 + 3].map(|k| pins[k]),
        ))),
    }
}

/// The frame memory, `HEIGHT` rows of `WIDTH` pixels of two bytes, in bands. A copy shares
/// every band, and a write copies only the band it lands in.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    bands: Vec<Arc<[u8; BAND_BYTES]>>,
}

impl Frame {
    /// All zero bytes. Every band starts as one shared band.
    fn new() -> Frame {
        let zero = Arc::new([0; BAND_BYTES]);
        Frame {
            bands: (0..BANDS).map(|_| Arc::clone(&zero)).collect(),
        }
    }

    fn put(&mut self, x: u16, y: u16, pixel: [u8; 2]) {
        let (x, y) = (usize::from(x), usize::from(y));
        let band = Arc::make_mut(&mut self.bands[y / BAND_ROWS]);
        let at = (y % BAND_ROWS) * ROW_BYTES + x * 2;
        band[at..at + 2].copy_from_slice(&pixel);
    }

    /// Becomes `source`, sharing its bands. Returns how many bands it replaced: those not
    /// already shared.
    pub(crate) fn copy_from(&mut self, source: &Frame) -> usize {
        let mut replaced = 0;
        for (band, from) in self.bands.iter_mut().zip(&source.bands) {
            if !Arc::ptr_eq(band, from) {
                band.clone_from(from);
                replaced += 1;
            }
        }
        replaced
    }

    /// The whole frame memory, row after row.
    pub(crate) fn image(&self) -> Vec<u8> {
        let mut image: Vec<u8> = self
            .bands
            .iter()
            .flat_map(|band| band.iter())
            .copied()
            .collect();
        image.truncate(usize::from(HEIGHT) * ROW_BYTES);
        image
    }
}

/// What a transaction that ended with CS held left open.
#[derive(Clone, Debug, PartialEq)]
enum Open {
    Nothing,
    /// A register command and the bytes it has taken. It runs when it closes.
    Register {
        command: u8,
        bytes: Vec<u8>,
    },
    /// A pixel write, in the window it began with.
    Pixels(Pixels),
    /// A pixel write the model drops, with its data.
    Dropped,
}

#[derive(Clone, Debug, PartialEq)]
struct Pixels {
    window: Window,
    bytes: u64,
    hash: RectangleHash,
    /// The first byte of a pixel whose second byte has not come.
    half: Option<u8>,
}

/// Columns `x0` to `x1` and rows `y0` to `y1`, both ends included, before MADCTL maps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
}

const FULL: Window = Window {
    x0: 0,
    y0: 0,
    x1: WIDTH - 1,
    y1: HEIGHT - 1,
};

/// A closed pixel write, before the seam names its part.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Closed {
    pub(crate) at: Time,
    pub(crate) x0: u16,
    pub(crate) y0: u16,
    pub(crate) x1: u16,
    pub(crate) y1: u16,
    pub(crate) bytes: u64,
    pub(crate) hash: [u8; 8],
}

/// The register state a reset sets.
#[derive(Clone, Debug, PartialEq)]
struct Registers {
    sleep: bool,
    display_on: bool,
    inverted: bool,
    madctl: u8,
    colmod: u8,
    window: Window,
    /// The next pixel's column and row, which RAMWR sets to the window's corner and RAMWRC
    /// continues from.
    cursor: (u16, u16),
    /// The last bytes of each register command the model stores and does not run.
    last_unrun: Arc<BTreeMap<u8, Arc<[u8]>>>,
}

impl Registers {
    fn reset() -> Registers {
        Registers {
            sleep: true,
            display_on: false,
            inverted: false,
            madctl: 0,
            colmod: COLMOD_RESET,
            window: FULL,
            cursor: (0, 0),
            last_unrun: Arc::default(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Core {
    /// The nets of D0 to D3. A transaction whose lanes sit elsewhere is dropped.
    lanes: [NetId; 4],
    /// RST reads low.
    reset: bool,
    registers: Registers,
    open: Open,
    brightness: Option<Brightness>,
    frame: Frame,
    /// Pixel writes it closed that the seam has not taken.
    closed: Vec<Closed>,
    /// The first time each of `RULES` broke.
    broke: [Option<Time>; 3],
    /// The first time it dropped a transaction whose lanes sat elsewhere, and its sender, an
    /// index in the run's senders.
    miswired: Option<(Time, u16)>,
}

impl Clone for Core {
    fn clone(&self) -> Core {
        Core {
            lanes: self.lanes,
            reset: self.reset,
            registers: self.registers.clone(),
            open: self.open.clone(),
            brightness: self.brightness,
            frame: self.frame.clone(),
            closed: self.closed.clone(),
            broke: self.broke,
            miswired: self.miswired,
        }
    }

    /// Reuses the band list and copies only the bands `source` does not share.
    fn clone_from(&mut self, source: &Core) {
        let Core {
            lanes,
            reset,
            registers,
            open,
            brightness,
            frame,
            closed,
            broke,
            miswired,
        } = self;
        *lanes = source.lanes;
        *reset = source.reset;
        registers.clone_from(&source.registers);
        open.clone_from(&source.open);
        *brightness = source.brightness;
        frame.copy_from(&source.frame);
        closed.clone_from(&source.closed);
        *broke = source.broke;
        *miswired = source.miswired;
    }
}

impl Core {
    fn new(lanes: [NetId; 4]) -> Core {
        Core {
            lanes,
            reset: false,
            registers: Registers::reset(),
            open: Open::Nothing,
            brightness: None,
            frame: Frame::new(),
            closed: Vec::new(),
            broke: [None; 3],
            miswired: None,
        }
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        &IMAGE
    }

    /// RST's level from `at` on. Going low resets the registers and drops an open command with
    /// no row. The frame memory keeps its pixels.
    pub(crate) fn input(&mut self, _at: Time, levels: &[bool]) {
        let low = !levels[0];
        if low && !self.reset {
            self.registers = Registers::reset();
            self.open = Open::Nothing;
        }
        self.reset = low;
    }

    /// Takes the records that end at `at`.
    pub(crate) fn take(&mut self, at: Time, records: &[LinkRow]) {
        for row in records {
            match &row.record.kind {
                RecordKind::Spi(spi) => self.transaction(at, row.sender, spi),
                RecordKind::Duty(duty) => self.duty(duty),
                RecordKind::Pulses(_) => unreachable!("a panel takes no pulses"),
            }
        }
    }

    pub(crate) fn drain_rows(&mut self, sent: impl FnMut(Closed)) {
        self.closed.drain(..).for_each(sent);
    }

    /// Each rule that broke, at its first time.
    pub(crate) fn broke_rules(&self) -> impl Iterator<Item = (ProtocolRule, Time)> {
        RULES
            .into_iter()
            .zip(self.broke)
            .filter_map(|(rule, at)| at.map(|at| (rule, at)))
    }

    /// The first miswired drop, with its sender's index.
    pub(crate) fn miswired(&self) -> Option<(Time, u16)> {
        self.miswired
    }

    pub(crate) fn panel(&self) -> Panel {
        let registers = &self.registers;
        Panel {
            kind: "st77916",
            width: WIDTH,
            height: HEIGHT,
            format: "rgb565",
            flags: Flags {
                display_on: registers.display_on,
                inverted: registers.inverted,
                reset: self.reset,
                sleep: registers.sleep,
            },
            madctl: registers.madctl,
            colmod: registers.colmod,
            registers: registers.last_unrun.len() as u64,
            brightness: self.brightness,
        }
    }

    pub(crate) fn frame(&self) -> &Frame {
        &self.frame
    }

    fn duty(&mut self, duty: &Duty) {
        let steps = 1u64
            .checked_shl(u32::from(duty.resolution))
            .filter(|_| duty.resolution < 32)
            .expect("a duty resolution is below 32");
        self.brightness = Some(Brightness {
            duty: duty.duty,
            steps,
            hz: duty.hz,
        });
    }

    fn transaction(&mut self, at: Time, sender: u16, spi: &Spi) {
        if self.reset {
            return;
        }
        if !self.wired(spi) {
            self.miswired.get_or_insert((at, sender));
            return;
        }
        if spi.phases.is_empty() {
            if matches!(self.open, Open::Nothing) {
                self.broke(at, ProtocolRule::SpiContinuation);
                return;
            }
            if !spi.data.is_empty()
                && let Some(expect) = open_lanes(&self.open)
                && spi.lanes != expect
            {
                self.broke(at, ProtocolRule::Command);
                if spi.cs == Cs::Released {
                    self.close(at);
                }
                return;
            }
        } else {
            self.close(at);
            let has_data = !spi.data.is_empty();
            self.open = match command(&spi.phases) {
                Some((opcode, command)) if accept(opcode, command, spi.lanes, has_data) => {
                    self.begin(at, command)
                }
                _ => {
                    self.broke(at, ProtocolRule::Command);
                    Open::Nothing
                }
            };
        }
        self.feed(&spi.data);
        if spi.cs == Cs::Released {
            self.close(at);
        }
    }

    /// Whether each lane the transaction uses sits on this chip's own data pin.
    fn wired(&self, spi: &Spi) -> bool {
        let used = spi
            .phases
            .iter()
            .map(|phase| phase.lanes.count())
            .chain([spi.lanes.count()])
            .max()
            .unwrap_or(1);
        (0..usize::from(used)).all(|k| spi.nets.data[k] == Some(self.lanes[k]))
    }

    fn begin(&mut self, at: Time, command: u8) -> Open {
        if matches!(command, RAMWR | RAMWRC) && self.registers.colmod & 0x07 != 0x05 {
            self.broke(at, ProtocolRule::PixelFormat);
            return Open::Dropped;
        }
        let registers = &mut self.registers;
        match command {
            RAMWR | RAMWRC => {
                if command == RAMWR {
                    registers.cursor = (registers.window.x0, registers.window.y0);
                }
                Open::Pixels(Pixels {
                    window: registers.window,
                    bytes: 0,
                    hash: RectangleHash::default(),
                    half: None,
                })
            }
            _ => Open::Register {
                command,
                bytes: Vec::new(),
            },
        }
    }

    fn feed(&mut self, data: &[u8]) {
        match &mut self.open {
            Open::Nothing | Open::Dropped => {}
            Open::Register { bytes, .. } => bytes.extend_from_slice(data),
            Open::Pixels(pixels) => {
                pixels.bytes += data.len() as u64;
                pixels.hash.update(data);
                let registers = &mut self.registers;
                for &byte in data {
                    let Some(first) = pixels.half.take() else {
                        pixels.half = Some(byte);
                        continue;
                    };
                    let (x, y) = place(registers.madctl, registers.cursor);
                    self.frame.put(x, y, [first, byte]);
                    registers.cursor = advance(pixels.window, registers.cursor);
                }
            }
        }
    }

    fn close(&mut self, at: Time) {
        match std::mem::replace(&mut self.open, Open::Nothing) {
            Open::Nothing | Open::Dropped => {}
            Open::Pixels(pixels) => {
                if pixels.bytes > 0 {
                    let Window { x0, y0, x1, y1 } = pixels.window;
                    self.closed.push(Closed {
                        at,
                        x0,
                        y0,
                        x1,
                        y1,
                        bytes: pixels.bytes,
                        hash: pixels.hash.finish(),
                    });
                }
            }
            Open::Register { command, bytes } => self.run(at, command, bytes),
        }
    }

    fn run(&mut self, at: Time, command: u8, bytes: Vec<u8>) {
        let registers = &mut self.registers;
        match command {
            SWRESET => *registers = Registers::reset(),
            SLPIN | SLPOUT => registers.sleep = command == SLPIN,
            INVOFF | INVON => registers.inverted = command == INVON,
            DISPOFF | DISPON => registers.display_on = command == DISPON,
            CASET | RASET => {
                let Some(&[s0, s1, e0, e1]) = bytes.first_chunk::<4>() else {
                    return self.broke(at, ProtocolRule::Command);
                };
                let (start, end) = (u16::from_be_bytes([s0, s1]), u16::from_be_bytes([e0, e1]));
                let limit = if command == CASET { WIDTH } else { HEIGHT };
                if start > end || end >= limit {
                    return self.broke(at, ProtocolRule::Command);
                }
                let window = &mut registers.window;
                if command == CASET {
                    (window.x0, window.x1) = (start, end);
                } else {
                    (window.y0, window.y1) = (start, end);
                }
            }
            MADCTL | COLMOD => {
                let Some(&value) = bytes.first() else {
                    return self.broke(at, ProtocolRule::Command);
                };
                if command == MADCTL {
                    registers.madctl = value;
                } else {
                    registers.colmod = value;
                }
            }
            _ => {
                Arc::make_mut(&mut registers.last_unrun).insert(command, bytes.into());
            }
        }
    }

    fn broke(&mut self, at: Time, rule: ProtocolRule) {
        let k = RULES
            .iter()
            .position(|&r| r == rule)
            .expect("every rule is listed");
        self.broke[k].get_or_insert(at);
    }
}

fn accept(opcode: u32, command: u8, lanes: Lanes, has_data: bool) -> bool {
    let (want_opcode, want_lanes) = if matches!(command, RAMWR | RAMWRC) {
        (OPCODE_PIXELS, Lanes::Four)
    } else {
        (OPCODE_REGISTER, Lanes::One)
    };
    opcode == want_opcode && (!has_data || lanes == want_lanes)
}

fn open_lanes(open: &Open) -> Option<Lanes> {
    match open {
        Open::Register { .. } => Some(Lanes::One),
        Open::Pixels(_) => Some(Lanes::Four),
        Open::Dropped | Open::Nothing => None,
    }
}

/// The opcode and the command of a 32-bit command word, `opcode << 24 | command << 8`, sent
/// as the phases before the data. `None` when the phases are not 32 bits.
fn command(phases: &[SpiPhase]) -> Option<(u32, u8)> {
    let mut bits = 0u32;
    let mut word = 0u64;
    for phase in phases {
        bits += u32::from(phase.bits);
        word = (word << phase.bits) | u64::from(phase.value);
    }
    let word = u32::try_from(word).ok().filter(|_| bits == 32)?;
    Some((word >> 24, (word >> 8) as u8))
}

/// The frame memory column and row of a pixel at `(column, row)`. MADCTL's MX mirrors the
/// column, MY mirrors the row, and then MV exchanges them.
fn place(madctl: u8, (column, row): (u16, u16)) -> (u16, u16) {
    let column = if madctl & MX != 0 {
        WIDTH - 1 - column
    } else {
        column
    };
    let row = if madctl & MY != 0 {
        HEIGHT - 1 - row
    } else {
        row
    };
    if madctl & MV != 0 {
        (row, column)
    } else {
        (column, row)
    }
}

/// The next pixel's column and row: along the row, then to the next row at the window's right
/// edge, and back to its top after its bottom row.
fn advance(window: Window, (column, row): (u16, u16)) -> (u16, u16) {
    if column < window.x1 {
        return (column + 1, row);
    }
    let row = if row < window.y1 { row + 1 } else { window.y0 };
    (window.x0, row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::link::{Lanes, Record, SpiNets};

    fn lanes() -> [NetId; 4] {
        [NetId::GROUND; 4]
    }

    fn spi(phases: &[(u8, u32)], data: &[u8], data_lanes: Lanes, cs: Cs) -> LinkRow {
        LinkRow {
            sender: 0,
            record: Record {
                seq: 0,
                start: Time::ZERO,
                end: Time::ZERO,
                pin: "CS",
                kind: RecordKind::Spi(Spi {
                    phases: phases
                        .iter()
                        .map(|&(bits, value)| SpiPhase {
                            bits,
                            lanes: Lanes::One,
                            value,
                        })
                        .collect(),
                    lanes: data_lanes,
                    data: data.into(),
                    clock_hz: 1,
                    cs,
                    nets: SpiNets {
                        cs: Some(NetId::GROUND),
                        sclk: Some(NetId::GROUND),
                        data: lanes().map(Some),
                    },
                }),
            },
        }
    }

    fn write(opcode: u32, command: u8, data: &[u8]) -> LinkRow {
        let lanes = if opcode == OPCODE_PIXELS {
            Lanes::Four
        } else {
            Lanes::One
        };
        spi(
            &[(8, opcode), (24, u32::from(command) << 8)],
            data,
            lanes,
            Cs::Released,
        )
    }

    fn pixel(image: &[u8], x: usize, y: usize) -> &[u8] {
        &image[y * ROW_BYTES + x * 2..][..2]
    }

    fn ready() -> Core {
        let mut core = Core::new(lanes());
        core.input(Time::ZERO, &[true]);
        core.take(Time::ZERO, &[write(0x02, COLMOD, &[0x55])]);
        core
    }

    fn bands_apart(a: &Frame, b: &Frame) -> usize {
        a.bands
            .iter()
            .zip(&b.bands)
            .filter(|(a, b)| !Arc::ptr_eq(a, b))
            .count()
    }

    /// A write over rows 0 to 20 lands in bands 0, 1 and 2. A copy over its twin replaces
    /// those three, after which every band is shared.
    #[test]
    fn a_clone_from_after_three_dirty_bands_copies_three_bands() {
        let mut core = ready();
        let committed = core.clone();
        assert_eq!(bands_apart(&core.frame, &committed.frame), 0);
        core.take(
            Time::from_ps(1),
            &[
                write(0x02, RASET, &[0, 0, 0, 20]),
                write(0x02, CASET, &[0, 0, 0, 0]),
                write(0x32, RAMWR, &[0xab; 42]),
            ],
        );
        assert_eq!(bands_apart(&core.frame, &committed.frame), 3);
        let mut copy = committed.clone();
        let before = copy.frame.bands.clone();
        assert_eq!(copy.frame.copy_from(&core.frame), 3);
        assert_eq!(bands_apart(&copy.frame, &core.frame), 0);
        let kept = before
            .iter()
            .zip(&copy.frame.bands)
            .filter(|(a, b)| Arc::ptr_eq(a, b))
            .count();
        assert_eq!(kept, BANDS - 3);
        let mut whole = committed.clone();
        whole.clone_from(&core);
        assert_eq!(whole, core);
    }

    #[test]
    fn a_held_write_continues_in_the_next_data_only_transaction() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[
                write(0x02, CASET, &[0, 1, 0, 2]),
                write(0x02, RASET, &[0, 3, 0, 3]),
                spi(&[(8, 0x32), (24, 0x2c00)], &[1, 2], Lanes::Four, Cs::Held),
            ],
        );
        assert!(core.closed.is_empty(), "a held write is open");
        core.take(
            Time::from_ps(2),
            &[spi(&[], &[3, 4, 5, 6], Lanes::Four, Cs::Released)],
        );
        let mut hash = RectangleHash::default();
        hash.update(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(
            core.closed,
            [Closed {
                at: Time::from_ps(2),
                x0: 1,
                y0: 3,
                x1: 2,
                y1: 3,
                bytes: 6,
                hash: hash.finish(),
            }]
        );
        let image = core.frame.image();
        let at = |x: usize, y: usize| &image[y * ROW_BYTES + x * 2..][..2];
        assert_eq!((at(1, 3), at(2, 3)), (&[5, 6][..], &[3, 4][..]));
    }

    #[test]
    fn data_with_no_open_command_breaks_spi_continuation_once() {
        let mut core = ready();
        core.take(
            Time::from_ps(5),
            &[spi(&[], &[1], Lanes::One, Cs::Released)],
        );
        core.take(
            Time::from_ps(9),
            &[spi(&[], &[1], Lanes::One, Cs::Released)],
        );
        assert_eq!(
            core.broke_rules().collect::<Vec<_>>(),
            [(ProtocolRule::SpiContinuation, Time::from_ps(5))]
        );
    }

    #[test]
    fn madctl_mirrors_then_exchanges() {
        assert_eq!(place(0, (3, 7)), (3, 7));
        assert_eq!(place(MX, (3, 7)), (356, 7));
        assert_eq!(place(MY, (3, 7)), (3, 352));
        assert_eq!(place(MV, (3, 7)), (7, 3));
        assert_eq!(place(MX | MV, (3, 7)), (7, 356));
    }

    #[test]
    fn madctl_with_mirror_and_swap_lands_the_pixel_after_the_exchange() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[
                write(0x02, CASET, &[0, 3, 0, 3]),
                write(0x02, RASET, &[0, 7, 0, 7]),
                write(0x02, MADCTL, &[MX | MV]),
                write(0x32, RAMWR, &[0xbe, 0xef]),
            ],
        );
        let image = core.frame.image();
        assert_eq!(pixel(&image, 7, 356), &[0xbe, 0xef]);
        assert_eq!(pixel(&image, 356, 7), &[0, 0]);
        assert_eq!(pixel(&image, 352, 3), &[0, 0]);
    }

    #[test]
    fn a_pixel_opcode_on_a_register_command_runs_nothing() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[spi(
                &[(8, 0x32), (24, u32::from(CASET) << 8)],
                &[0, 1, 0, 2],
                Lanes::Four,
                Cs::Released,
            )],
        );
        core.take(Time::from_ps(2), &[write(0x32, RAMWR, &[9, 8])]);
        let image = core.frame.image();
        assert_eq!(pixel(&image, 0, 0), &[9, 8]);
        assert_eq!(pixel(&image, 1, 0), &[0, 0]);
        assert_eq!(
            core.broke_rules().collect::<Vec<_>>(),
            [(ProtocolRule::Command, Time::from_ps(1))]
        );
    }

    #[test]
    fn a_register_opcode_on_a_pixel_command_runs_nothing() {
        for command in [RAMWR, RAMWRC] {
            let mut core = ready();
            core.take(
                Time::from_ps(4),
                &[spi(
                    &[(8, 0x02), (24, u32::from(command) << 8)],
                    &[9, 8],
                    Lanes::One,
                    Cs::Released,
                )],
            );
            assert!(core.closed.is_empty(), "command {command:#x}");
            assert_eq!(pixel(&core.frame.image(), 0, 0), &[0, 0]);
            assert_eq!(
                core.broke_rules().collect::<Vec<_>>(),
                [(ProtocolRule::Command, Time::from_ps(4))]
            );
        }
    }

    #[test]
    fn data_lanes_that_do_not_match_the_opcode_run_nothing() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[spi(
                &[(8, 0x02), (24, u32::from(COLMOD) << 8)],
                &[0x66],
                Lanes::Four,
                Cs::Released,
            )],
        );
        assert_eq!(core.panel().colmod, 0x55);
        let mut pixels = ready();
        pixels.take(
            Time::from_ps(3),
            &[spi(
                &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
                &[1, 2],
                Lanes::One,
                Cs::Released,
            )],
        );
        assert!(pixels.closed.is_empty());
        assert_eq!(pixel(&pixels.frame.image(), 0, 0), &[0, 0]);
        assert_eq!(
            pixels.broke_rules().collect::<Vec<_>>(),
            [(ProtocolRule::Command, Time::from_ps(3))]
        );
    }

    #[test]
    fn a_nonzero_middle_byte_of_the_command_word_is_ignored() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[
                spi(
                    &[(8, 0x02), (24, 0x00ab_2a00)],
                    &[0, 5, 0, 6],
                    Lanes::One,
                    Cs::Released,
                ),
                write(0x32, RAMWR, &[3, 4]),
            ],
        );
        assert_eq!(pixel(&core.frame.image(), 5, 0), &[3, 4]);
        assert_eq!(pixel(&core.frame.image(), 0, 0), &[0, 0]);
        assert_eq!(core.panel().registers, 0);
        assert_eq!(core.broke_rules().count(), 0);
    }

    #[test]
    fn a_write_wraps_to_the_top_left_of_a_three_row_window() {
        let mut core = ready();
        let mut data = Vec::new();
        for i in 0..7u8 {
            data.extend([i, 0x10 + i]);
        }
        core.take(
            Time::from_ps(1),
            &[
                write(0x02, CASET, &[0, 1, 0, 2]),
                write(0x02, RASET, &[0, 5, 0, 7]),
                write(0x32, RAMWR, &data),
            ],
        );
        let image = core.frame.image();
        assert_eq!(pixel(&image, 1, 5), &[6, 0x16]);
        assert_eq!(pixel(&image, 2, 5), &[1, 0x11]);
        assert_eq!(pixel(&image, 2, 7), &[5, 0x15]);
        assert_eq!(core.closed[0].bytes, 14);
        assert_eq!((core.closed[0].x0, core.closed[0].y0), (1, 5));
        assert_eq!((core.closed[0].x1, core.closed[0].y1), (2, 7));
    }

    #[test]
    fn ramwrc_continues_from_where_ramwr_stopped() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[
                write(0x02, CASET, &[0, 1, 0, 2]),
                write(0x02, RASET, &[0, 4, 0, 4]),
                write(0x32, RAMWR, &[1, 2]),
            ],
        );
        core.take(Time::from_ps(2), &[write(0x32, RAMWRC, &[3, 4])]);
        let image = core.frame.image();
        assert_eq!(pixel(&image, 1, 4), &[1, 2]);
        assert_eq!(pixel(&image, 2, 4), &[3, 4]);
        assert_eq!(core.closed.len(), 2);
        assert_eq!(core.closed[0].bytes, 2);
        assert_eq!(core.closed[1].bytes, 2);
        assert_eq!((core.closed[1].x0, core.closed[1].y0), (1, 4));
    }

    #[test]
    fn a_pixel_split_on_an_odd_byte_joins_across_two_records() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[spi(
                &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
                &[0x11, 0x22, 0x33],
                Lanes::Four,
                Cs::Held,
            )],
        );
        assert!(core.closed.is_empty());
        core.take(
            Time::from_ps(2),
            &[spi(&[], &[0x44, 0x55, 0x66], Lanes::Four, Cs::Released)],
        );
        let image = core.frame.image();
        assert_eq!(pixel(&image, 0, 0), &[0x11, 0x22]);
        assert_eq!(pixel(&image, 1, 0), &[0x33, 0x44]);
        assert_eq!(pixel(&image, 2, 0), &[0x55, 0x66]);
        assert_eq!(core.closed[0].bytes, 6);
    }

    #[test]
    fn stored_registers_keep_the_last_bytes_of_each_command() {
        let mut core = ready();
        let before = core.panel();
        for _ in 0..300 {
            core.take(Time::from_ps(1), &[write(0x02, 0xf0, &[1, 2, 3])]);
        }
        assert_eq!(core.panel().registers, 1);
        assert_eq!(core.registers.last_unrun[&0xf0].as_ref(), &[1, 2, 3]);
        assert_eq!(core.panel().madctl, before.madctl);
        assert_eq!(core.panel().colmod, before.colmod);
        assert_eq!(core.panel().flags, before.flags);
        let committed = core.clone();
        core.take(Time::from_ps(2), &[write(0x02, 0xf0, &[9])]);
        assert_eq!(core.registers.last_unrun[&0xf0].as_ref(), &[9]);
        assert_eq!(core.panel().registers, 1);
        assert_eq!(committed.registers.last_unrun[&0xf0].as_ref(), &[1, 2, 3]);
        assert_eq!(committed.panel().registers, 1);
        core.take(Time::from_ps(3), &[write(0x02, 0xf1, &[4, 5])]);
        assert_eq!(core.panel().registers, 2);
        assert_eq!(committed.panel().registers, 1);
        assert_eq!(core.registers.last_unrun.len(), 2);
    }

    #[test]
    fn a_ramwr_sent_alone_on_one_lane_draws_the_four_lane_data() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[spi(
                &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
                &[],
                Lanes::One,
                Cs::Held,
            )],
        );
        assert!(core.closed.is_empty(), "a command with no bytes stays open");
        assert_eq!(core.broke_rules().count(), 0);
        core.take(
            Time::from_ps(2),
            &[spi(&[], &[0x11, 0x22], Lanes::Four, Cs::Released)],
        );
        assert_eq!(pixel(&core.frame.image(), 0, 0), &[0x11, 0x22]);
        assert_eq!(core.closed.len(), 1);
        assert_eq!(core.closed[0].bytes, 2);
        assert_eq!(core.broke_rules().count(), 0);
    }

    #[test]
    fn a_dispon_with_no_bytes_labelled_four_lanes_runs() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[spi(
                &[(8, 0x02), (24, u32::from(DISPON) << 8)],
                &[],
                Lanes::Four,
                Cs::Released,
            )],
        );
        assert!(core.panel().flags.display_on);
        assert_eq!(core.broke_rules().count(), 0);
        let mut crossed = ready();
        crossed.take(
            Time::from_ps(2),
            &[spi(
                &[(8, 0x32), (24, u32::from(DISPON) << 8)],
                &[],
                Lanes::One,
                Cs::Released,
            )],
        );
        assert!(!crossed.panel().flags.display_on);
        assert_eq!(
            crossed.broke_rules().collect::<Vec<_>>(),
            [(ProtocolRule::Command, Time::from_ps(2))]
        );
    }

    #[test]
    fn a_continuation_on_the_wrong_lanes_drops_its_bytes() {
        let mut core = ready();
        core.take(
            Time::from_ps(1),
            &[spi(
                &[(8, 0x32), (24, u32::from(RAMWR) << 8)],
                &[0x11, 0x22],
                Lanes::Four,
                Cs::Held,
            )],
        );
        core.take(
            Time::from_ps(2),
            &[spi(&[], &[0x33, 0x44], Lanes::One, Cs::Held)],
        );
        core.take(
            Time::from_ps(3),
            &[spi(&[], &[0x55, 0x66], Lanes::Four, Cs::Released)],
        );
        let image = core.frame.image();
        assert_eq!(pixel(&image, 0, 0), &[0x11, 0x22]);
        assert_eq!(pixel(&image, 1, 0), &[0x55, 0x66]);
        assert_eq!(pixel(&image, 2, 0), &[0, 0]);
        assert_eq!(core.closed.len(), 1);
        assert_eq!(core.closed[0].bytes, 4);
        assert_eq!(
            core.broke_rules().collect::<Vec<_>>(),
            [(ProtocolRule::Command, Time::from_ps(2))]
        );
    }

    #[test]
    fn a_command_word_is_32_bits_of_phases() {
        assert_eq!(
            command(&[phase(8, 0x32), phase(24, 0x2c00)]),
            Some((0x32, 0x2c))
        );
        assert_eq!(command(&[phase(32, 0x0211_2200)]), Some((0x02, 0x22)));
        assert_eq!(command(&[phase(8, 0x02), phase(16, 0x2c00)]), None);
    }

    fn phase(bits: u8, value: u32) -> SpiPhase {
        SpiPhase {
            bits,
            lanes: Lanes::One,
            value,
        }
    }
}
