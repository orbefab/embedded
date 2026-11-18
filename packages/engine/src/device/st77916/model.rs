//! The ST77916 360 by 360 QSPI panel controller as a device. The host's SPI engine gives it each
//! transaction on its bus, and its PWM pin each duty on `BLK`. It reads `RST` and holds `TE` low.
//!
//! A command is one 32-bit word, `opcode << 24 | command << 8`, the first 32 bits CS carries,
//! from a transaction's phases and then its bytes. `RAMWR` and `RAMWRC` take opcode 0x32 and
//! four data lanes, and every other command takes opcode 0x02 and one. The opcode is checked on
//! every command, and the lanes only on bytes past the word. A transaction that ends with CS held
//! leaves its command open, and the next transaction's bytes continue it. The command closes when
//! CS is released or when RST goes low. A register command runs when it closes. A pixel write
//! puts each pixel in the frame memory as it arrives and reports what it drew when it closes.

use orbe_model::{
    Cs, Cx, Device, Drawn, Drive, Drives, Duty, DutyTarget, Error, Lanes, Levels, Panel, Params,
    Sha256, Shows, Spi, SpiTarget, State, Stop, Time, View,
};

/// The panel's pixels across and down.
pub const WIDTH: u16 = 360;
pub const HEIGHT: u16 = 360;
const ROW_BYTES: usize = WIDTH as usize * 2;
/// Rows of the frame memory in one page. A copy copies only the pages a write touched.
const BAND_ROWS: usize = 8;
const BAND_BYTES: usize = BAND_ROWS * ROW_BYTES;
const BANDS: usize = (HEIGHT as usize).div_ceil(BAND_ROWS);

/// It drives `TE`, and reads `RST`.
pub(crate) const OUTPUTS: usize = 1;
pub(crate) const WATCH_RST: usize = 0;

const OPCODE_REGISTER: u32 = 0x02;
const OPCODE_PIXELS: u32 = 0x32;

/// MIPI DCS commands the model runs.
const SWRESET: u8 = 0x01;
const SLPIN: u8 = 0x10;
const SLPOUT: u8 = 0x11;
const INVOFF: u8 = 0x20;
const INVON: u8 = 0x21;
const DISPOFF: u8 = 0x28;
pub(crate) const DISPON: u8 = 0x29;
pub(crate) const CASET: u8 = 0x2a;
pub(crate) const RASET: u8 = 0x2b;
pub(crate) const RAMWR: u8 = 0x2c;
pub(crate) const MADCTL: u8 = 0x36;
pub(crate) const COLMOD: u8 = 0x3a;
pub(crate) const RAMWRC: u8 = 0x3c;

/// MADCTL's row order, column order and row and column exchange bits.
pub(crate) const MY: u8 = 0x80;
pub(crate) const MX: u8 = 0x40;
pub(crate) const MV: u8 = 0x20;

/// COLMOD after a reset: 18 bits a pixel.
const COLMOD_RESET: u8 = 0x66;

/// A command that is not one the model runs: a word cut short, phases past it, the wrong opcode
/// or lanes, too few parameter bytes, or a window past the panel.
const COMMAND: &str = "command";
/// Pixels while the pixel format is not 16 bits.
const PIXEL_FORMAT: &str = "pixel_format";

/// The frame memory, `HEIGHT` rows of `WIDTH` pixels of two bytes, one page per 8 rows.
/// The bytes are on the heap, so moving a controller copies a pointer. A checkpoint copies
/// the pages whose generations differ.
#[derive(Clone, Debug)]
struct Frame {
    bytes: Box<[u8]>,
    generations: [u64; BANDS],
    newest: u64,
}

impl Frame {
    fn new() -> Frame {
        Frame {
            bytes: vec![0u8; BANDS * BAND_BYTES].into_boxed_slice(),
            generations: [0; BANDS],
            newest: 0,
        }
    }

    fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn page_mut(&mut self, page: usize) -> &mut [u8] {
        self.newest += 1;
        self.generations[page] = self.newest;
        let start = page * BAND_BYTES;
        &mut self.bytes[start..start + BAND_BYTES]
    }

    fn copy_from(&mut self, source: &Frame) -> usize {
        let mut copied = 0;
        for page in 0..BANDS {
            if self.generations[page] != source.generations[page] {
                let start = page * BAND_BYTES;
                let end = start + BAND_BYTES;
                self.bytes[start..end].copy_from_slice(&source.bytes[start..end]);
                self.generations[page] = source.generations[page];
                copied += 1;
            }
        }
        self.newest = source.newest;
        copied
    }
}

/// The controller and its frame memory. A copy copies the controller whole and the frame memory
/// by the page, so a commit costs the pages a write touched.
#[derive(Clone, Debug)]
pub struct St77916 {
    controller: Controller,
    frame: Frame,
}

/// Everything but the frame memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Controller {
    /// RST reads low.
    reset: bool,
    registers: Registers,
    open: Open,
    backlight: Option<Duty>,
}

impl State for St77916 {
    fn copy_from(&mut self, source: &St77916) {
        let St77916 { controller, frame } = self;
        *controller = source.controller;
        frame.copy_from(&source.frame);
    }
}

/// What CS has carried since it fell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    /// The first `bits` bits of the command word, fewer than 32, as `word`.
    Word { bits: u32, word: u32 },
    /// A register command and its first parameter bytes. It runs when it closes. No command it
    /// runs reads past the fourth byte.
    Register {
        command: u8,
        params: [u8; 4],
        /// How many of `params` it has.
        taken: u8,
    },
    /// A pixel write, in the window it began with.
    Pixels(Pixels),
    /// A command the model drops, with its data.
    Dropped,
}

/// CS released: no bit of the next command word yet.
const IDLE: Open = Open::Word { bits: 0, word: 0 };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pixels {
    window: Window,
    bytes: u64,
    hash: Sha256,
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

/// The register state a reset sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Bit `c` is set when register command `c`, one the model does not run, took a write.
    stored: [u64; 4],
}

impl Registers {
    const RESET: Registers = Registers {
        sleep: true,
        display_on: false,
        inverted: false,
        madctl: 0,
        colmod: COLMOD_RESET,
        window: FULL,
        cursor: (0, 0),
        stored: [0; 4],
    };
}

impl Device for St77916 {
    const SHOWS: Shows = Shows::Screen {
        kind: "st77916",
        width: WIDTH,
        height: HEIGHT,
        format: "rgb565",
    };

    fn new(_params: &Params<'_>) -> Result<St77916, Error> {
        Ok(St77916 {
            controller: Controller {
                reset: false,
                registers: Registers::RESET,
                open: IDLE,
                backlight: None,
            },
            frame: Frame::new(),
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::new([Drive::Low])
    }

    /// RST's level from `at` on. Going low resets the registers and drops an open command with
    /// no row. The frame memory keeps its pixels.
    fn input(&mut self, _cx: &mut Cx, _at: Time, levels: Levels) {
        let low = !levels.high(WATCH_RST);
        let controller = &mut self.controller;
        if low && !controller.reset {
            controller.registers = Registers::RESET;
            controller.open = IDLE;
        }
        controller.reset = low;
    }

    fn view(&self) -> View<'_> {
        let Controller {
            reset,
            registers,
            backlight,
            ..
        } = self.controller;
        let stored: u32 = registers.stored.iter().map(|word| word.count_ones()).sum();
        View::Screen {
            frame: self.frame.bytes(),
            panel: Panel {
                on: registers.display_on,
                inverted: registers.inverted,
                reset,
                asleep: registers.sleep,
                madctl: registers.madctl,
                colmod: registers.colmod,
                registers: stored as u16,
                backlight,
            },
        }
    }
}

#[cfg(test)]
impl St77916 {
    /// How many pages of frame memory a copy of `source` over this would copy.
    pub(crate) fn pages_to_copy(&self, source: &St77916) -> usize {
        let mut frame = self.frame.clone();
        frame.copy_from(&source.frame)
    }
}

impl DutyTarget for St77916 {
    fn duty(&mut self, _cx: &mut Cx, _at: Time, duty: Duty) {
        self.controller.backlight = Some(duty);
    }
}

impl SpiTarget for St77916 {
    fn transaction(&mut self, cx: &mut Cx, at: Time, spi: &Spi) {
        let mut data = &spi.data[..];
        let Some(opened) = self.take_word(cx, at, spi, &mut data) else {
            if spi.cs == Cs::Released {
                self.close(cx, at);
            }
            return;
        };
        if !data.is_empty()
            && let Some(expect) = open_lanes(&self.controller.open)
            && spi.lanes != expect
        {
            cx.broke(COMMAND, at);
            if opened {
                self.controller.open = Open::Dropped;
            }
            data = &[];
        }
        self.feed(data);
        if spi.cs == Cs::Released {
            self.close(cx, at);
        }
    }
}

impl St77916 {
    /// Takes the bits of the command word from the transaction's phases and then from the front
    /// of `data`, and begins the command once all 32 have come. Whether this transaction began
    /// it, or `None` when its phases run past the word or a bit of the word comes on more than
    /// one lane, which drops the transaction.
    fn take_word(&mut self, cx: &mut Cx, at: Time, spi: &Spi, data: &mut &[u8]) -> Option<bool> {
        let Open::Word { mut bits, mut word } = self.controller.open else {
            if spi.phases.is_empty() {
                return Some(false);
            }
            cx.broke(COMMAND, at);
            return None;
        };
        for phase in &spi.phases {
            bits += u32::from(phase.bits);
            if bits > 32 || phase.lanes != Lanes::One {
                cx.broke(COMMAND, at);
                self.controller.open = Open::Dropped;
                return None;
            }
            word = word.unbounded_shl(u32::from(phase.bits)) | phase.value;
        }
        if bits < 32 && !data.is_empty() && spi.lanes != Lanes::One {
            cx.broke(COMMAND, at);
            self.controller.open = Open::Dropped;
            return None;
        }
        while bits < 32
            && let Some((&byte, rest)) = data.split_first()
        {
            (bits, word, *data) = (bits + 8, word << 8 | u32::from(byte), rest);
        }
        if bits < 32 {
            self.controller.open = Open::Word { bits, word };
            return Some(false);
        }
        let (opcode, command) = (word >> 24, (word >> 8) as u8);
        let lanes = if matches!(command, RAMWR | RAMWRC) {
            (OPCODE_PIXELS, Lanes::Four)
        } else {
            (OPCODE_REGISTER, Lanes::One)
        };
        self.controller.open = if opcode != lanes.0 || (!data.is_empty() && spi.lanes != lanes.1) {
            cx.broke(COMMAND, at);
            Open::Dropped
        } else {
            self.begin(cx, at, command)
        };
        Some(true)
    }

    fn begin(&mut self, cx: &mut Cx, at: Time, command: u8) -> Open {
        let registers = &mut self.controller.registers;
        if matches!(command, RAMWR | RAMWRC) && registers.colmod & 0x07 != 0x05 {
            cx.broke(PIXEL_FORMAT, at);
            return Open::Dropped;
        }
        match command {
            RAMWR | RAMWRC => {
                if command == RAMWR {
                    registers.cursor = (registers.window.x0, registers.window.y0);
                }
                Open::Pixels(Pixels {
                    window: registers.window,
                    bytes: 0,
                    hash: Sha256::new(),
                    half: None,
                })
            }
            _ => Open::Register {
                command,
                params: [0; 4],
                taken: 0,
            },
        }
    }

    fn feed(&mut self, data: &[u8]) {
        let Controller {
            registers, open, ..
        } = &mut self.controller;
        match open {
            Open::Word { .. } | Open::Dropped => {}
            Open::Register { params, taken, .. } => {
                for &byte in data.iter().take(params.len() - usize::from(*taken)) {
                    params[usize::from(*taken)] = byte;
                    *taken += 1;
                }
            }
            Open::Pixels(pixels) => {
                pixels.bytes += data.len() as u64;
                pixels.hash.update(data);
                for &byte in data {
                    let Some(first) = pixels.half.take() else {
                        pixels.half = Some(byte);
                        continue;
                    };
                    let (x, y) = place(registers.madctl, registers.cursor);
                    let (x, y) = (usize::from(x), usize::from(y));
                    let band = self.frame.page_mut(y / BAND_ROWS);
                    let at = (y % BAND_ROWS) * ROW_BYTES + x * 2;
                    band[at..at + 2].copy_from_slice(&[first, byte]);
                    registers.cursor = advance(pixels.window, registers.cursor);
                }
            }
        }
    }

    fn close(&mut self, cx: &mut Cx, at: Time) {
        match std::mem::replace(&mut self.controller.open, IDLE) {
            Open::Word { bits: 0, .. } | Open::Dropped => {}
            Open::Word { .. } => cx.broke(COMMAND, at),
            Open::Pixels(pixels) => {
                if pixels.bytes > 0 {
                    let Window { x0, y0, x1, y1 } = pixels.window;
                    let digest = pixels.hash.finish();
                    let mut hash = [0; 8];
                    hash.copy_from_slice(&digest[..8]);
                    cx.drew(Drawn {
                        x0,
                        y0,
                        x1,
                        y1,
                        bytes: pixels.bytes,
                        hash,
                    });
                }
            }
            Open::Register {
                command,
                params,
                taken,
            } => self.run(cx, at, command, &params[..usize::from(taken)]),
        }
    }

    fn run(&mut self, cx: &mut Cx, at: Time, command: u8, params: &[u8]) {
        let registers = &mut self.controller.registers;
        match command {
            SWRESET => *registers = Registers::RESET,
            SLPIN | SLPOUT => registers.sleep = command == SLPIN,
            INVOFF | INVON => registers.inverted = command == INVON,
            DISPOFF | DISPON => registers.display_on = command == DISPON,
            CASET | RASET => {
                let Some(&[s0, s1, e0, e1]) = params.first_chunk::<4>() else {
                    return cx.broke(COMMAND, at);
                };
                let (start, end) = (u16::from_be_bytes([s0, s1]), u16::from_be_bytes([e0, e1]));
                let limit = if command == CASET { WIDTH } else { HEIGHT };
                if start > end || end >= limit {
                    return cx.broke(COMMAND, at);
                }
                let window = &mut registers.window;
                if command == CASET {
                    (window.x0, window.x1) = (start, end);
                } else {
                    (window.y0, window.y1) = (start, end);
                }
            }
            MADCTL | COLMOD => {
                let Some(&value) = params.first() else {
                    return cx.broke(COMMAND, at);
                };
                if command == MADCTL {
                    registers.madctl = value;
                } else {
                    registers.colmod = value;
                }
            }
            _ => registers.stored[usize::from(command / 64)] |= 1 << (command % 64),
        }
    }
}

fn open_lanes(open: &Open) -> Option<Lanes> {
    match open {
        Open::Register { .. } => Some(Lanes::One),
        Open::Pixels(_) => Some(Lanes::Four),
        Open::Dropped | Open::Word { .. } => None,
    }
}

/// The frame memory column and row of a pixel at `(column, row)`. MADCTL's MX mirrors the
/// column, MY mirrors the row, and then MV exchanges them.
pub(crate) fn place(madctl: u8, (column, row): (u16, u16)) -> (u16, u16) {
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
mod size_probe {
    use super::*;

    #[test]
    fn the_frame_bytes_are_not_inside_the_value() {
        assert!(
            std::mem::size_of::<St77916>() < 4 * 1024,
            "st77916 {}",
            std::mem::size_of::<St77916>()
        );
        assert!(
            std::mem::size_of::<Frame>() < 4 * 1024,
            "frame {}",
            std::mem::size_of::<Frame>()
        );
    }
}
