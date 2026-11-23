//! The SSD1306 128 by 64 OLED controller as an I2C target. The host's I2C engine runs `SCL` and
//! `SDA` and hands this device each transfer's bytes. It reads `RES#` and drives nothing.
//!
//! Each write transfer starts with a control byte (datasheet 8.1.5.1). `D/C#`, bit 6, says
//! whether the bytes after it are commands or display data. With `Co`, bit 7, clear, every
//! later byte of the transfer is of that one kind. With `Co` set, one byte of that kind follows
//! and then another control byte. A command is a byte and its parameter bytes, and the decoder
//! keeps a command that is short of its parameters across a STOP, so a parameter may come in a
//! transfer of its own, as Adafruit SSD1306 sends some. Display data goes into the GDDRAM at the
//! address pointer, which moves as the memory addressing mode says (datasheet 10.1.3).
//!
//! The GDDRAM is held by segment: byte `page * 128 + seg` holds rows `8 * page` to
//! `8 * page + 7` of segment `seg`, bit 0 the top one. Segment remap, `A1h`, maps column address
//! `c` to segment `127 - c` as each byte is written (datasheet 10.1.8). The rows the glass shows
//! come from the display start line, the display offset, the multiplex ratio, the COM scan
//! direction and the COM pins configuration as it draws them, in `glass.rs`.

use orbe_model::{
    Ack, Cx, Device, Dir, Drawn, Drives, Error, I2cTarget, Levels, Panel, Params, Settings, Sha256,
    Shows, State, Stop, Time, View,
};

/// The panel's pixels across and down.
pub const WIDTH: u16 = 128;
pub const HEIGHT: u16 = 64;
const COLUMNS: usize = WIDTH as usize;
const PAGES: usize = HEIGHT as usize / 8;
/// The GDDRAM, one bit a pixel.
pub const FRAME_BYTES: usize = COLUMNS * PAGES;

/// Watch order: SCL, SDA, RES#.
pub const WATCH_SCL: usize = 0;
pub const WATCH_SDA: usize = 1;
pub const WATCH_RES: usize = 2;

/// It drives no output of its own. The I2C engine drives `SDA`.
pub const OUTPUTS: usize = 0;

/// A command byte the model does not know, a parameter outside the range the datasheet allows,
/// or a control byte whose low six bits are not zero.
const COMMAND: &str = "command";
/// A scroll setup or activation, `26h`, `27h`, `29h`, `2Ah`, `A3h` or `2Fh`. The model keeps
/// the picture still.
const SCROLL: &str = "scroll";
/// A read transfer. The model answers `0xFF`, the released line.
const READ: &str = "read";

/// The settings its panel carries, by name, in [`Registers::settings`] order.
pub const SETTINGS: [&str; 10] = [
    "contrast",
    "entire_on",
    "addressing",
    "start_line",
    "offset",
    "multiplex",
    "segment_remap",
    "com_scan",
    "com_pins",
    "charge_pump",
];

/// The memory addressing modes of `20h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addressing {
    Horizontal = 0,
    Vertical = 1,
    Page = 2,
}

/// Every register a reset sets (datasheet Table 9-1 and section 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registers {
    pub display_on: bool,
    pub inverted: bool,
    /// `A5h`: every pixel lit whatever the GDDRAM holds.
    pub entire_on: bool,
    pub contrast: u8,
    pub addressing: Addressing,
    /// `21h`, the column window of horizontal and vertical addressing.
    pub columns: (u8, u8),
    /// `22h`, the page window of horizontal and vertical addressing.
    pub pages: (u8, u8),
    /// The column page addressing wraps back to, which `00h` to `1Fh` set.
    pub page_column: u8,
    /// The address pointer: the column and the page the next data byte goes to.
    pub column: u8,
    pub page: u8,
    pub start_line: u8,
    pub offset: u8,
    /// The multiplex ratio less one, 15 to 63.
    pub multiplex: u8,
    /// `A1h`: column address `c` writes segment `127 - c`.
    pub segment_remap: bool,
    /// `C8h`: the COM scan runs from `COM[N-1]` to `COM0`.
    pub com_scan: bool,
    /// The `DAh` byte.
    pub com_pins: u8,
    /// The `8Dh` byte. Bit 2 enables the charge pump.
    pub charge_pump: u8,
    /// Bit 0, 1 or 2 set once `D5h`, `D9h` or `DBh` took a write. The model stores them and they
    /// change nothing it draws.
    pub stored: u8,
}

impl Registers {
    pub const RESET: Registers = Registers {
        display_on: false,
        inverted: false,
        entire_on: false,
        contrast: 0x7f,
        addressing: Addressing::Page,
        columns: (0, 127),
        pages: (0, 7),
        page_column: 0,
        column: 0,
        page: 0,
        start_line: 0,
        offset: 0,
        multiplex: 63,
        segment_remap: false,
        com_scan: false,
        com_pins: 0x12,
        charge_pump: 0x10,
        stored: 0,
    };

    fn settings(&self) -> Settings {
        Settings::new(
            &SETTINGS,
            [
                self.contrast,
                u8::from(self.entire_on),
                self.addressing as u8,
                self.start_line,
                self.offset,
                self.multiplex,
                u8::from(self.segment_remap),
                u8::from(self.com_scan),
                self.com_pins,
                self.charge_pump,
            ]
            .map(u16::from),
        )
    }
}

/// What the next byte of a write transfer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    /// No write transfer is open, or it is a read.
    Closed,
    Control,
    /// Every byte to the STOP, commands or data.
    Stream {
        data: bool,
    },
    /// One byte, then a control byte.
    One {
        data: bool,
    },
}

/// A command short of its parameter bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    command: u8,
    params: [u8; 6],
    taken: u8,
}

/// An open run of display data: the window it began in, its bytes and their hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    window: (u16, u16, u16, u16),
    bytes: u64,
    hash: Sha256,
}

/// The controller and its 1024-byte GDDRAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ssd1306 {
    ram: [u8; FRAME_BYTES],
    registers: Registers,
    /// RES# reads low.
    reset: bool,
    next: Next,
    pending: Option<Pending>,
    run: Option<Run>,
}

impl State for Ssd1306 {}

impl Device for Ssd1306 {
    const SHOWS: Shows = Shows::Screen {
        kind: "ssd1306",
        width: WIDTH,
        height: HEIGHT,
        format: "mono-pages",
    };

    fn new(_params: &Params<'_>) -> Result<Ssd1306, Error> {
        Ok(Ssd1306 {
            ram: [0; FRAME_BYTES],
            registers: Registers::RESET,
            reset: false,
            next: Next::Closed,
            pending: None,
            run: None,
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }

    /// `RES#` going low resets every register and drops a command short of its parameters and
    /// an open run of data, with no row. The GDDRAM keeps what it holds (datasheet 8.9).
    fn input(&mut self, _cx: &mut Cx, _at: Time, levels: Levels) {
        let low = !levels.high(WATCH_RES);
        if low && !self.reset {
            self.registers = Registers::RESET;
            self.next = Next::Closed;
            self.pending = None;
            self.run = None;
        }
        self.reset = low;
    }

    fn view(&self) -> View<'_> {
        let registers = &self.registers;
        View::Screen {
            frame: &self.ram,
            panel: Panel {
                on: registers.display_on,
                inverted: registers.inverted,
                reset: self.reset,
                asleep: !registers.display_on,
                settings: registers.settings(),
                registers: registers.stored.count_ones() as u16,
                backlight: None,
            },
        }
    }
}

impl I2cTarget for Ssd1306 {
    fn start(&mut self, cx: &mut Cx, at: Time, dir: Dir) {
        self.close(cx);
        self.next = match dir {
            Dir::Write => Next::Control,
            Dir::Read => {
                cx.broke(READ, at);
                Next::Closed
            }
        };
    }

    fn write(&mut self, cx: &mut Cx, at: Time, byte: u8) -> Ack {
        match self.next {
            Next::Closed => {}
            Next::Control => {
                if byte & 0x3f != 0 {
                    cx.broke(COMMAND, at);
                }
                let data = byte & 0x40 != 0;
                self.next = if byte & 0x80 != 0 {
                    Next::One { data }
                } else {
                    Next::Stream { data }
                };
            }
            Next::Stream { data } => self.take(cx, at, data, byte),
            Next::One { data } => {
                self.take(cx, at, data, byte);
                self.next = Next::Control;
            }
        }
        Ack::Yes
    }

    fn read(&mut self, _cx: &mut Cx, _at: Time) -> u8 {
        0xff
    }

    fn stop(&mut self, cx: &mut Cx, _at: Time) {
        self.close(cx);
        self.next = Next::Closed;
    }
}

impl Ssd1306 {
    #[cfg(test)]
    pub(crate) fn registers(&self) -> &Registers {
        &self.registers
    }

    fn take(&mut self, cx: &mut Cx, at: Time, data: bool, byte: u8) {
        if data {
            self.put(byte);
        } else {
            self.close(cx);
            self.command(cx, at, byte);
        }
    }

    /// Closes an open run of data with its row.
    fn close(&mut self, cx: &mut Cx) {
        let Some(run) = self.run.take() else {
            return;
        };
        let (x0, y0, x1, y1) = run.window;
        let mut hash = [0; 8];
        hash.copy_from_slice(&run.hash.finish()[..8]);
        cx.drew(Drawn {
            x0,
            y0,
            x1,
            y1,
            bytes: run.bytes,
            hash,
        });
    }

    /// The window a run of data that starts now writes: columns and rows, both ends included.
    fn window(&self) -> (u16, u16, u16, u16) {
        let r = &self.registers;
        let rows = |first: u8, last: u8| (u16::from(first) * 8, u16::from(last) * 8 + 7);
        match r.addressing {
            Addressing::Page => {
                let (y0, y1) = rows(r.page, r.page);
                (u16::from(r.column), y0, WIDTH - 1, y1)
            }
            Addressing::Horizontal | Addressing::Vertical => {
                let (y0, y1) = rows(r.pages.0, r.pages.1);
                (u16::from(r.columns.0), y0, u16::from(r.columns.1), y1)
            }
        }
    }

    /// One byte of display data at the address pointer, which then moves on.
    fn put(&mut self, byte: u8) {
        if self.run.is_none() {
            self.run = Some(Run {
                window: self.window(),
                bytes: 0,
                hash: Sha256::new(),
            });
        }
        let run = self.run.as_mut().expect("a run is open");
        run.bytes += 1;
        run.hash.update(&[byte]);
        let r = &mut self.registers;
        let segment = if r.segment_remap {
            COLUMNS - 1 - usize::from(r.column)
        } else {
            usize::from(r.column)
        };
        self.ram[usize::from(r.page) * COLUMNS + segment] = byte;
        let last = (COLUMNS - 1) as u8;
        match r.addressing {
            Addressing::Page => {
                r.column = if r.column >= last {
                    r.page_column
                } else {
                    r.column + 1
                };
            }
            Addressing::Horizontal => {
                if r.column == r.columns.1 || r.column >= last {
                    r.column = r.columns.0;
                    r.page = if r.page == r.pages.1 || r.page >= 7 {
                        r.pages.0
                    } else {
                        r.page + 1
                    };
                } else {
                    r.column += 1;
                }
            }
            Addressing::Vertical => {
                if r.page == r.pages.1 || r.page >= 7 {
                    r.page = r.pages.0;
                    r.column = if r.column == r.columns.1 || r.column >= last {
                        r.columns.0
                    } else {
                        r.column + 1
                    };
                } else {
                    r.page += 1;
                }
            }
        }
    }

    /// One byte of the command stream: a command, or the next parameter of the one pending.
    fn command(&mut self, cx: &mut Cx, at: Time, byte: u8) {
        let pending = match self.pending {
            Some(mut pending) => {
                pending.params[usize::from(pending.taken)] = byte;
                pending.taken += 1;
                pending
            }
            None => Pending {
                command: byte,
                params: [0; 6],
                taken: 0,
            },
        };
        if pending.taken < params(pending.command) {
            self.pending = Some(pending);
            return;
        }
        self.pending = None;
        let taken = usize::from(pending.taken);
        self.run(cx, at, pending.command, &pending.params[..taken]);
    }

    /// Runs a whole command, its parameters in `p`.
    fn run(&mut self, cx: &mut Cx, at: Time, command: u8, p: &[u8]) {
        let r = &mut self.registers;
        let first = p.first().copied().unwrap_or(0);
        match command {
            0x00..=0x0f => {
                r.column = r.column & 0xf0 | command;
                r.page_column = r.column;
            }
            0x10..=0x17 => {
                r.column = (command & 0x07) << 4 | r.column & 0x0f;
                r.page_column = r.column;
            }
            0x20 => match first & 0x03 {
                0 => r.addressing = Addressing::Horizontal,
                1 => r.addressing = Addressing::Vertical,
                2 => r.addressing = Addressing::Page,
                _ => cx.broke(COMMAND, at),
            },
            0x21 => {
                r.columns = (first & 0x7f, p[1] & 0x7f);
                r.column = r.columns.0;
            }
            0x22 => {
                r.pages = (first & 0x07, p[1] & 0x07);
                r.page = r.pages.0;
            }
            0x2e => {}
            0x26 | 0x27 | 0x29 | 0x2a | 0x2f | 0xa3 => cx.broke(SCROLL, at),
            0x40..=0x7f => r.start_line = command & 0x3f,
            0x81 => r.contrast = first,
            0x8d => r.charge_pump = first,
            0xa0 | 0xa1 => r.segment_remap = command == 0xa1,
            0xa4 | 0xa5 => r.entire_on = command == 0xa5,
            0xa6 | 0xa7 => r.inverted = command == 0xa7,
            0xa8 => {
                if first & 0x3f < 15 {
                    cx.broke(COMMAND, at);
                } else {
                    r.multiplex = first & 0x3f;
                }
            }
            0xae | 0xaf => r.display_on = command == 0xaf,
            0xb0..=0xb7 => r.page = command & 0x07,
            0xc0 | 0xc8 => r.com_scan = command == 0xc8,
            0xd3 => r.offset = first & 0x3f,
            0xda => r.com_pins = first & 0x32,
            0xd5 => r.stored |= 1,
            0xd9 => r.stored |= 2,
            0xdb => r.stored |= 4,
            0xe3 => {}
            _ => cx.broke(COMMAND, at),
        }
    }
}

/// How many parameter bytes follow `command` (datasheet Table 9-1).
fn params(command: u8) -> u8 {
    match command {
        0x20 | 0x81 | 0x8d | 0xa8 | 0xd3 | 0xd5 | 0xd9 | 0xda | 0xdb => 1,
        0x21 | 0x22 | 0xa3 => 2,
        0x29 | 0x2a => 5,
        0x26 | 0x27 => 6,
        _ => 0,
    }
}
