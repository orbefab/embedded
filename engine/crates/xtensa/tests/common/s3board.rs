//! The S3 board as the probe kit sees it after `load-ram`: instruction RAM, internal data RAM,
//! the mask ROM bytes the unaligned probe reads, and the USB serial console the kit writes.

use std::collections::VecDeque;
use std::ops::Range;

use sim_xtensa::{
    Bus, Cause, Core, Cycle, Esp32s3, Exit, PagedRam, Sr, Stop, Unimplemented, Width,
};

use super::elf::Elf;

const IRAM: Range<u32> = 0x4037_0000..0x403e_0000;
const DRAM: Range<u32> = 0x3fc8_8000..0x3fd0_0000;
const PERIPHERALS: Range<u32> = 0x6000_0000..0x6010_0000;

/// `USB_SERIAL_JTAG_EP1_REG` and `USB_SERIAL_JTAG_EP1_CONF_REG`.
const USB_EP1: u32 = 0x6003_8000;
const USB_EP1_CONF: u32 = 0x6003_8004;
const DATA_FREE: u32 = 1 << 1;
const DATA_AVAIL: u32 = 1 << 2;

/// Bytes the unaligned fixture reads at 0x40000400. An aligned word there is `00001406`.
const IROM_AT: u32 = 0x4000_0400;
const IROM_BYTES: &[u8] = &[0x06, 0x14, 0x00, 0x00, 0xab, 0xab, 0x00, 0x00];
/// Bytes the unaligned fixture reads at 0x3ff18c28, `0004080c10131619` and the bytes after them.
const DROM_AT: u32 = 0x3ff1_8c28;
const DROM_BYTES: &[u8] = &[
    0x00, 0x04, 0x08, 0x0c, 0x10, 0x13, 0x16, 0x19, 0x1c, 0x1f, 0x22, 0x24, 0x27, 0x29, 0x2c, 0x2e,
];

pub struct Board {
    iram: PagedRam,
    dram: PagedRam,
    served: Vec<u32>,
    input: VecDeque<u8>,
    pub uart: String,
}

impl Board {
    pub fn load(elf: &Elf, served: &[&str]) -> Board {
        let mut board = Board {
            iram: PagedRam::new(IRAM.start, (IRAM.end - IRAM.start) as usize),
            dram: PagedRam::new(DRAM.start, (DRAM.end - DRAM.start) as usize),
            served: served.iter().map(|name| elf.symbol(name)).collect(),
            input: VecDeque::from(*b"g"),
            uart: String::new(),
        };
        for segment in &elf.segments {
            let ram = if board.iram.contains(segment.addr) {
                &mut board.iram
            } else if board.dram.contains(segment.addr) {
                &mut board.dram
            } else {
                panic!("segment at {:#010x} is outside the S3 RAM", segment.addr);
            };
            ram.write_bytes(segment.addr, &segment.data);
        }
        board
    }

    /// The print hook the classic board uses does not exist here. A served function returns 0
    /// the way the classic `fpu` test answers the double-precision ops.
    fn serve(&mut self, core: &mut Core<Esp32s3>) {
        let window = (core.special(Sr::Ps) >> 16) & 3;
        let args = window * 4 + 2;
        if !self.served.contains(&core.pc()) {
            panic!(
                "the image calls {:#010x}, which the test board does not serve",
                core.pc()
            );
        }
        core.set_a(args, 0);
        let link = core.a(window * 4);
        core.set_pc((core.pc() & 0xc000_0000) | (link & 0x3fff_ffff));
    }

    fn rom(addr: u32, width: Width) -> Option<u32> {
        for (base, bytes) in [(IROM_AT, IROM_BYTES), (DROM_AT, DROM_BYTES)] {
            let Some(off) = addr.checked_sub(base) else {
                continue;
            };
            let end = off as usize + width.bytes() as usize;
            if end <= bytes.len() {
                let chunk = &bytes[off as usize..end];
                return Some(
                    chunk
                        .iter()
                        .rev()
                        .fold(0, |value, byte| (value << 8) | u32::from(*byte)),
                );
            }
        }
        None
    }

    fn read_mem(&self, addr: u32, width: Width) -> Option<u32> {
        if self.iram.contains(addr) {
            Some(self.iram.read(addr, width))
        } else if self.dram.contains(addr) {
            Some(self.dram.read(addr, width))
        } else {
            Self::rom(addr, width)
        }
    }

    fn write_mem(&mut self, addr: u32, width: Width, value: u32) -> bool {
        if self.iram.contains(addr) {
            self.iram.write(addr, width, value);
            true
        } else if self.dram.contains(addr) {
            self.dram.write(addr, width, value);
            true
        } else {
            false
        }
    }
}

impl Bus for Board {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        if self.served.contains(&addr) {
            Err(Exit::Stop)
        } else if self.iram.contains(addr) {
            Ok(self.iram.read(addr, Width::Word))
        } else {
            Err(Exit::Fault(Cause::InstructionFetchError))
        }
    }

    fn load(&mut self, _now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        if addr == USB_EP1 {
            return Ok(u32::from(self.input.pop_front().unwrap_or(0)));
        }
        if addr == USB_EP1_CONF {
            let mut value = DATA_FREE;
            if !self.input.is_empty() {
                value |= DATA_AVAIL;
            }
            return Ok(value);
        }
        if let Some(value) = self.read_mem(addr, width) {
            return Ok(value);
        }
        if PERIPHERALS.contains(&addr) {
            return Ok(0);
        }
        Err(Exit::Fault(Cause::LoadStoreError))
    }

    fn store(&mut self, _now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        if addr == USB_EP1 {
            self.uart.push(char::from(value as u8));
            return Ok(());
        }
        if self.write_mem(addr, width, value) || PERIPHERALS.contains(&addr) {
            return Ok(());
        }
        Err(Exit::Fault(Cause::LoadStoreError))
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
    pub core: Core<Esp32s3>,
}

/// Runs a probe image from its entry until it prints `END <probe>`, stops, or reaches `limit`.
pub fn run_probe(elf: &Elf, probe: &str, limit: Cycle, served: &[&str]) -> Run {
    run_answering(elf, probe, limit, served, |_, _| false)
}

/// [`run_probe`], where `answer` may carry out an op the core stops at as unimplemented, move the
/// PC past it and return true to run on.
pub fn run_answering(
    elf: &Elf,
    probe: &str,
    limit: Cycle,
    served: &[&str],
    mut answer: impl FnMut(&mut Core<Esp32s3>, Unimplemented) -> bool,
) -> Run {
    let mut board = Board::load(elf, served);
    let mut core = Core::<Esp32s3>::new();
    core.set_pc(elf.entry);
    let end = format!("END {probe}\n");
    loop {
        let to = Cycle((core.cycle().0 + 20_000).min(limit.0));
        let stop = core.run_until(&mut board, to);
        if stop == Stop::Bus {
            board.serve(&mut core);
            continue;
        }
        if let Stop::Unimplemented(missing) = stop
            && answer(&mut core, missing)
        {
            continue;
        }
        if board.uart.contains(&end) || stop != Stop::Limit || core.cycle() >= limit {
            return Run {
                uart: board.uart,
                stop,
                core,
            };
        }
    }
}
