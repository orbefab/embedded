//! The classic board as the probe kit sees it after `esptool load-ram`: 128 KB of instruction RAM
//! that takes only word accesses, the internal data RAM, peripheral stores that land nowhere, and
//! stand-ins for the ROM functions the kit calls.

use std::ops::Range;

use orbe_xtensa::{Bus, Cause, Core, Cycle, Esp32, Exit, PagedRam, Sr, Stop, Width};

use super::elf::Elf;
use super::{Pieces, Straight, run_in_pieces};

const ROM: Range<u32> = 0x4000_0000..0x4006_0000;
const IRAM: Range<u32> = 0x4008_0000..0x400a_0000;
const PERIPHERALS: Range<u32> = 0x3ff0_0000..0x3ff8_0000;
const DRAM: Range<u32> = 0x3ffa_e000..0x4000_0000;

/// The address the stand-in `ets_printf` calls to hand its text to the test.
const PRINT_HOOK: u32 = 0x4005_fff0;

/// Stand-ins for the ROM functions on the path `ets_printf("%s\n", text)` takes in the rev0 ROM,
/// at their ROM addresses, with GNU `as` encodings. On that path each matches its ROM function's
/// call depth, frame size and highest window quarter, so the window overflows and underflows the
/// path raises match the board's. The ROM calls `ets_write_char` once per character, and the
/// stand-in calls it once, which spills nothing more at the same depth. The ROM's other
/// conversions, which reach higher quarters, are not modelled.
pub const ROM_STUB: [(u32, &str, &[&str]); 5] = [
    (
        0x4000_7c58,
        "364100e509001df0",
        &["entry\ta1, 32", "call8\t0x40007cf8", "retw.n"],
    ),
    (
        0x4000_7cf8,
        "3641006550011df0",
        &["entry\ta1, 32", "call8\t0x40009200", "retw.n"],
    ),
    (
        0x4000_7d28,
        "3641008d091df0",
        &["entry\ta1, 32", "mov.n\ta8, a9", "retw.n"],
    ),
    (
        0x4000_7d54,
        "362101ad02bd03652958a5efff1df0",
        &[
            "entry\ta1, 144",
            "mov.n\ta10, a2",
            "mov.n\ta11, a3",
            "call8\t0x4005fff0",
            "call8\t0x40007c58",
            "retw.n",
        ],
    ),
    (
        0x4000_9200,
        "364100bd0b1df0",
        &["entry\ta1, 32", "mov.n\ta11, a11", "retw.n"],
    ),
];

/// `SAR` as the rev0 ROM leaves it when `load-ram` jumps to the image. The first `add` rows of
/// the classic `alu` fixture print it, before any probe op writes `SAR`.
const ROM_SAR: u32 = 0x1a;
/// The ROM leaves `WindowBase` nonzero, so the image's first op, `movi a1, _stack_top`, writes
/// a register of the ROM's window, and the image runs on whatever the ROM left in physical `a1`.
/// Which `WindowBase` the ROM leaves is not measured, and the image never reads the register the
/// `movi` writes.
const ROM_WINDOWBASE: u32 = 1;
/// Inferred from the stack pointers the classic `window` fixture prints. It equals the ROM's
/// `UartDev` address, so it is probably a pointer the download receiver left there, not a stack
/// pointer.
const ROM_A1: u32 = 0x3ffe_019c;
/// The word below `ROM_A1` that the overflow handler reads as the stack pointer of the ROM frame
/// above it, when it spills the image's first frame. The board's value is not measured. This one
/// puts that frame's `a4` to `a7` just above the stack, where nothing reads them.
const ROM_FRAME_LINK: u32 = ROM_A1 + 32;

pub struct Board {
    iram: PagedRam,
    dram: PagedRam,
    rom: PagedRam,
    /// The image functions the test answers in place of running them.
    served: Vec<u32>,
    pub uart: String,
}

impl Board {
    pub fn load(elf: &Elf, served: &[&str]) -> Board {
        let mut board = Board {
            iram: PagedRam::new(IRAM.start, (IRAM.end - IRAM.start) as usize),
            dram: PagedRam::new(DRAM.start, (DRAM.end - DRAM.start) as usize),
            rom: PagedRam::new(ROM.start, (ROM.end - ROM.start) as usize),
            served: served.iter().map(|name| elf.symbol(name)).collect(),
            uart: String::new(),
        };
        for segment in &elf.segments {
            let ram = if board.iram.contains(segment.addr) {
                &mut board.iram
            } else {
                &mut board.dram
            };
            ram.write_bytes(segment.addr, &segment.data);
        }
        for (addr, hex, _) in ROM_STUB {
            board.rom.write_bytes(addr, &bytes_of(hex));
        }
        board.dram.write(ROM_A1 - 12, Width::Word, ROM_FRAME_LINK);
        board
    }

    /// The board with a code version that moves at each store to instruction RAM. Its lines
    /// never move, so it is still.
    pub fn kept(self) -> Straight<Board> {
        Straight::kept(self, IRAM, true)
    }

    fn in_stub(addr: u32) -> bool {
        ROM_STUB
            .iter()
            .any(|(at, hex, _)| (at & !3..at + hex.len() as u32 / 2).contains(&addr))
    }

    /// Answers the call the core stopped at: the print hook, whose text goes out with the ROM's
    /// `\r\n` for `\n`, or a served function, which returns 0.
    fn serve(&mut self, core: &mut Core<Esp32>) {
        let window = (core.special(Sr::Ps) >> 16) & 3;
        let args = window * 4 + 2;
        match core.pc() {
            PRINT_HOOK => {
                let format = self.c_string(core.a(args));
                assert_eq!(format, "%s\n", "the ets_printf stub takes only \"%s\\n\"");
                let text = self.c_string(core.a(args + 1));
                self.uart.push_str(&text);
                self.uart.push_str("\r\n");
            }
            pc if self.served.contains(&pc) => core.set_a(args, 0),
            pc => panic!("the image calls {pc:#010x}, which the test board does not serve"),
        }
        let link = core.a(window * 4);
        core.set_pc((core.pc() & 0xc000_0000) | (link & 0x3fff_ffff));
    }

    fn c_string(&self, addr: u32) -> String {
        let mut bytes = Vec::new();
        for at in addr..addr + 256 {
            let byte = self.dram.read(at, Width::Byte) as u8;
            if byte == 0 {
                return String::from_utf8(bytes).expect("printed text is UTF-8");
            }
            bytes.push(byte);
        }
        panic!("string at {addr:#010x} ends within 256 bytes");
    }
}

pub fn bytes_of(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

impl Bus for Board {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        if self.served.contains(&addr) {
            Err(Exit::Stop)
        } else if self.iram.contains(addr) {
            Ok(self.iram.read(addr, Width::Word))
        } else if Board::in_stub(addr) {
            Ok(self.rom.read(addr, Width::Word))
        } else if ROM.contains(&addr) {
            Err(Exit::Stop)
        } else {
            Err(Exit::Fault(Cause::InstructionFetchError))
        }
    }

    fn load(&mut self, _now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        if self.iram.contains(addr) && width == Width::Word {
            Ok(self.iram.read(addr, width))
        } else if self.dram.contains(addr) {
            Ok(self.dram.read(addr, width))
        } else {
            Err(Exit::Fault(Cause::LoadStoreError))
        }
    }

    fn store(&mut self, _now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        if self.iram.contains(addr) && width == Width::Word {
            self.iram.write(addr, width, value);
        } else if self.dram.contains(addr) {
            self.dram.write(addr, width, value);
        } else if !PERIPHERALS.contains(&addr) {
            return Err(Exit::Fault(Cause::LoadStoreError));
        }
        Ok(())
    }

    fn interrupts(&mut self, _now: Cycle) -> u32 {
        0
    }

    fn next_event(&mut self, _now: Cycle) -> Option<Cycle> {
        None
    }
}

pub struct Run {
    pub uart: String,
    pub stop: Stop,
    pub core: Core<Esp32>,
    /// The bus's trace of every load and store.
    pub trace: u64,
    /// The full steps the core made.
    pub steps: u64,
}

/// Runs a probe image from its entry on `Core<Esp32>` until it prints `END <probe>`, stops for
/// another reason, or reaches `limit`. `served` names the image functions the test answers with
/// 0, and `pieces` cuts each run at random limits. The board keeps a code version, so the core
/// runs straight runs.
pub fn run_probe(
    elf: &Elf,
    probe: &str,
    limit: Cycle,
    served: &[&str],
    pieces: Option<Pieces>,
) -> Run {
    run_probe_on(Board::load(elf, served).kept(), elf, probe, limit, pieces)
}

/// [`run_probe`] on `board`, as the image loaded it.
pub fn run_probe_on(
    mut board: Straight<Board>,
    elf: &Elf,
    probe: &str,
    limit: Cycle,
    mut pieces: Option<Pieces>,
) -> Run {
    let mut core = Core::<Esp32>::new();
    core.set_pc(elf.entry);
    core.set_special(Sr::Sar, ROM_SAR);
    core.set_a(1, ROM_A1);
    core.set_special(Sr::WindowBase, ROM_WINDOWBASE);
    let end = format!("END {probe}\r\n");
    let done = |board: Straight<Board>, stop, core| Run {
        uart: board.bus.uart,
        stop,
        core,
        trace: board.trace,
        steps: board.steps.get(),
    };
    loop {
        let stop = run_in_pieces(&mut core, &mut board, limit, &mut pieces);
        if stop != Stop::Bus {
            return done(board, stop, core);
        }
        board.bus.serve(&mut core);
        if board.bus.uart.ends_with(&end) {
            let after = Cycle(core.cycle().0 + 1000);
            let stop = run_in_pieces(&mut core, &mut board, after, &mut pieces);
            return done(board, stop, core);
        }
    }
}
