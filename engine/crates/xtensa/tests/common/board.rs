//! The classic board as the probe kit sees it after `esptool load-ram`: 128 KB of instruction RAM
//! that takes only word accesses, 32 KB of data RAM, peripheral stores that land nowhere, and the
//! two ROM entries the kit calls, served in Rust.

use std::ops::Range;

use sim_xtensa::{Bus, Cause, Core, Cycle, Esp32, Exit, PagedRam, Sr, Stop, Width};

use super::elf::Elf;

/// `ets_printf` and `ets_install_uart_printf` in the rev0 ROM, as the kit's `ram.ld` links them.
pub const ETS_PRINTF: u32 = 0x4000_7d54;
pub const ETS_INSTALL_UART_PRINTF: u32 = 0x4000_7d28;

const ROM: Range<u32> = 0x4000_0000..0x4006_0000;
const PERIPHERALS: Range<u32> = 0x3ff0_0000..0x3ff8_0000;

/// `SAR` as the rev0 ROM leaves it when `load-ram` jumps to the image. The first `add` rows of
/// the classic `alu` fixture print it, before any probe op writes `SAR`.
const ROM_SAR: u32 = 0x1a;

pub struct Board {
    iram: PagedRam,
    dram: PagedRam,
    pub uart: String,
}

impl Board {
    pub fn load(elf: &Elf) -> Board {
        let mut board = Board {
            iram: PagedRam::new(0x4008_0000, 0x2_0000),
            dram: PagedRam::new(0x3ffe_8000, 0x8000),
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
        board
    }

    /// Serves the ROM call the core stopped at. The ROM's UART output turns `\n` into `\r\n`.
    fn serve_rom(&mut self, core: &mut Core<Esp32>) {
        let window = (core.special(Sr::Ps) >> 16) & 3;
        let args = window * 4 + 2;
        let ret = match core.pc() {
            ETS_INSTALL_UART_PRINTF => 0,
            ETS_PRINTF => {
                let format = self.c_string(core.a(args));
                assert_eq!(format, "%s\n", "the ets_printf stub takes only \"%s\\n\"");
                let text = self.c_string(core.a(args + 1));
                self.uart.push_str(&text);
                self.uart.push_str("\r\n");
                text.len() as u32 + 2
            }
            pc => panic!("the image calls ROM at {pc:#010x}, which the test board does not serve"),
        };
        let link = core.a(window * 4);
        core.set_a(args, ret);
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

impl Bus for Board {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        if self.iram.contains(addr) {
            Ok(self.iram.read(addr, Width::Word))
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
}

pub struct Run {
    pub uart: String,
    pub stop: Stop,
    pub core: Core<Esp32>,
}

/// Runs a probe image from its entry on `Core<Esp32>` until it prints `END <probe>`, stops for
/// another reason, or reaches `limit`.
pub fn run_probe(elf: &Elf, probe: &str, limit: Cycle) -> Run {
    let mut board = Board::load(elf);
    let mut core = Core::<Esp32>::new();
    core.set_pc(elf.entry);
    core.set_special(Sr::Sar, ROM_SAR);
    let end = format!("END {probe}\r\n");
    loop {
        let stop = core.run_until(&mut board, limit);
        if stop != Stop::Bus {
            return Run {
                uart: board.uart,
                stop,
                core,
            };
        }
        board.serve_rom(&mut core);
        if board.uart.ends_with(&end) {
            let after = Cycle(core.cycle().0 + 1000);
            let stop = core.run_until(&mut board, after);
            return Run {
                uart: board.uart,
                stop,
                core,
            };
        }
    }
}
