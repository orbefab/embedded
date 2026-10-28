//! The ESP32-S3 as one Xtensa LX7 core on its internal SRAM, with GPIO, IO_MUX and the GPIO
//! matrix, SYSTIMER, the watchdogs and the USB Serial/JTAG console.
//!
//! The chip starts as a bare-metal ELF leaves it: the segments in SRAM, the PC at the entry, and
//! the core's reset state otherwise, with no boot ROM, no flash and no cache. The core runs on the
//! 40 MHz crystal, so `CCOUNT`, SYSTIMER and the watchdogs share one clock.
//!
//! The 512 KB SRAM shows through two windows: from `0x4037_0000` on the instruction bus, and
//! from `0x3fc8_8000` on the data bus, 32 KB into it. The core fetches only through the first,
//! and loads and stores through either. Through the instruction bus only an aligned word goes:
//! a byte or halfword raises `LoadStoreError`, and an unaligned access `LoadStoreAlignment`, as
//! the board does. A load or store anywhere else outside the peripherals raises
//! `LoadStoreError`. A fetch from outside the instruction window ends the run.
//!
//! A peripheral register the model does not keep reads 0 and drops writes. [`Esp32s3::unmodeled`]
//! lists each one the firmware touched, with the first access's cycle and PC. That access stops
//! the core once, so the timeline can read the PC, and then runs again. A byte or halfword
//! access to a kept register reads or merges into its word.

mod blocks;
mod gpio;
mod timers;
mod usb;

use std::collections::BTreeMap;

use sim_xtensa::{Bus, Cause, Config, Core, Cycle, Exit, PagedRam, Stop, Unimplemented, Width};

use gpio::{Gpio, PADS};
use timers::{Systimer, Watchdogs};
use usb::UsbSerialJtag;

pub use gpio::PadDrive;
pub use timers::Watchdog;

/// The crystal and CPU clock.
pub const HZ: u64 = 40_000_000;

/// A window onto SRAM: its bus addresses, and where in SRAM it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub start: u32,
    pub end: u32,
    pub offset: u32,
    /// Whether the core fetches through it.
    pub code: bool,
}

/// The instruction-bus window, then the data-bus window.
pub const RAM: [Window; 2] = [
    Window {
        start: 0x4037_0000,
        end: 0x403e_0000,
        offset: 0,
        code: true,
    },
    Window {
        start: 0x3fc8_8000,
        end: 0x3fd0_0000,
        offset: 0x8000,
        code: false,
    },
];

const SRAM: usize = 512 * 1024;

const PERIPHERALS: std::ops::Range<u32> = 0x6000_0000..0x600d_1000;
const GPIO: u32 = 0x6000_4000;
const RTC_CNTL: u32 = 0x6000_8000;
const IO_MUX: u32 = 0x6000_9000;
const TIMG0: u32 = 0x6001_f000;
const TIMG1: u32 = 0x6002_0000;
const SYSTIMER: u32 = 0x6002_3000;
const USB_SERIAL_JTAG: u32 = 0x6003_8000;
const BLOCK: u32 = 0x1000;

/// Why a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The next instruction needs behavior the core does not model.
    Unsupported(Unimplemented),
    /// The core fetched outside the instruction window, at this PC.
    UnmappedFetch(u32),
    /// A watchdog ran out its first stage.
    Watchdog(Watchdog),
}

/// Where a run stopped before its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    /// A pad's drive changed at this cycle. [`Esp32s3::drive`] gives the new drives.
    Image(Cycle),
    /// The run ended at this cycle. The chip runs no further.
    Fault(Cycle, Fault),
}

impl Halt {
    pub fn at(self) -> Cycle {
        match self {
            Halt::Image(at) | Halt::Fault(at, _) => at,
        }
    }
}

/// The first access to a peripheral register the model does not keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub at: Cycle,
    pub pc: u32,
    pub write: bool,
}

impl Touch {
    /// The register as its block and offset, such as `TIMG0+0x24`.
    pub fn register(address: u32) -> String {
        blocks::name(address)
    }
}

/// What the core sees: SRAM and the peripherals.
#[derive(Clone, Debug)]
struct Soc {
    sram: PagedRam,
    gpio: Gpio,
    systimer: Systimer,
    watchdogs: Watchdogs,
    usb: UsbSerialJtag,
    /// The GPIOs with nets, whose drives the image holds.
    netted: Vec<u8>,
    unmodeled: BTreeMap<u32, Touch>,
    /// A store changed a netted pad's drive at this cycle, so the next fetch stops the core.
    changed: Option<Cycle>,
    /// A store moved a watchdog's deadline, so the next fetch stops the core and the run reads
    /// the new one.
    rearmed: bool,
    /// The first access to an unmodeled register stopped the core, so the timeline can take the
    /// access's PC.
    first_touch: Option<u32>,
    /// The core fetched outside the instruction window.
    unmapped_fetch: bool,
}

impl Soc {
    fn sram_offset(addr: u32, fetch: bool) -> Option<u32> {
        RAM.iter()
            .filter(|w| w.code || !fetch)
            .find(|w| (w.start..w.end).contains(&addr))
            .map(|w| addr - w.start + w.offset)
    }

    /// The SRAM offset a `width` load or store at `addr` reaches, if any. The core checks the
    /// instruction bus first, and the bus agrees.
    fn data_offset(addr: u32, width: Width) -> Result<Option<u32>, Exit> {
        if sim_xtensa::Esp32s3::instruction_bus(addr) {
            if !addr.is_multiple_of(width.bytes()) {
                return Err(Exit::Fault(Cause::LoadStoreAlignment));
            }
            if width != Width::Word {
                return Err(Exit::Fault(Cause::LoadStoreError));
            }
        }
        Ok(Self::sram_offset(addr, false))
    }

    /// The kept register's word at `addr`, a multiple of 4 in the peripherals.
    fn read_kept(&self, addr: u32) -> Option<u32> {
        let (base, offset) = (addr & !(BLOCK - 1), addr & (BLOCK - 1));
        match base {
            GPIO => self.gpio.read(offset),
            IO_MUX => self.gpio.read_io_mux(offset),
            RTC_CNTL => self.watchdogs.read_rtc(offset),
            TIMG0 => self.watchdogs.read_timg(0, offset),
            TIMG1 => self.watchdogs.read_timg(1, offset),
            SYSTIMER => self.systimer.read(offset),
            USB_SERIAL_JTAG => self.usb.read(offset),
            _ => None,
        }
    }

    fn write_kept(&mut self, now: Cycle, addr: u32, value: u32) -> bool {
        let (base, offset) = (addr & !(BLOCK - 1), addr & (BLOCK - 1));
        match base {
            GPIO => self.gpio.write(offset, value),
            IO_MUX => self.gpio.write_io_mux(offset, value),
            RTC_CNTL => self.watchdogs.write_rtc(now, offset, value),
            TIMG0 => self.watchdogs.write_timg(now, 0, offset, value),
            TIMG1 => self.watchdogs.write_timg(now, 1, offset, value),
            SYSTIMER => self.systimer.write(now, offset, value),
            USB_SERIAL_JTAG => self.usb.write(now, offset, value),
            _ => false,
        }
    }

    /// Lets an access to an unmodeled register through, or stops the core at the first one.
    fn touch(&mut self, now: Cycle, addr: u32, write: bool) -> Result<(), Exit> {
        if self.unmodeled.contains_key(&addr) {
            return Ok(());
        }
        let touch = Touch {
            at: now,
            pc: 0,
            write,
        };
        self.unmodeled.insert(addr, touch);
        self.first_touch = Some(addr);
        Err(Exit::Stop)
    }

    /// The drive of each netted pad, `Off` for the others.
    fn image(&self) -> [PadDrive; PADS] {
        let mut image = [PadDrive::Off; PADS];
        for &gpio in &self.netted {
            image[usize::from(gpio)] = self.gpio.drive(usize::from(gpio));
        }
        image
    }
}

/// Splits `addr` into its word and the bit shift of a `width` access in it.
fn word_of(addr: u32, width: Width) -> (u32, u32, u32) {
    let shift = (addr & 3) * 8;
    let mask = match width {
        Width::Byte => 0xff,
        Width::Half => 0xffff,
        Width::Word => u32::MAX,
    };
    (addr & !3, shift, mask)
}

impl Bus for Soc {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        if self.changed.is_some() || self.rearmed {
            return Err(Exit::Stop);
        }
        match Self::sram_offset(addr, true) {
            Some(offset) => Ok(self.sram.read(offset, Width::Word)),
            None => {
                self.unmapped_fetch = true;
                Err(Exit::Stop)
            }
        }
    }

    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        if let Some(offset) = Self::data_offset(addr, width)? {
            return Ok(self.sram.read(offset, width));
        }
        if !PERIPHERALS.contains(&addr) {
            return Err(Exit::Fault(Cause::LoadStoreError));
        }
        let (word, shift, mask) = word_of(addr, width);
        match self.read_kept(word) {
            Some(value) => Ok((value >> shift) & mask),
            None => self.touch(now, word, false).map(|()| 0),
        }
    }

    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        if let Some(offset) = Self::data_offset(addr, width)? {
            self.sram.write(offset, width, value);
            return Ok(());
        }
        if !PERIPHERALS.contains(&addr) {
            return Err(Exit::Fault(Cause::LoadStoreError));
        }
        let (word, shift, mask) = word_of(addr, width);
        let Some(old) = self.read_kept(word) else {
            return self.touch(now, word, true);
        };
        let value = (old & !(mask << shift)) | ((value & mask) << shift);
        if !matches!(word & !(BLOCK - 1), GPIO | IO_MUX) {
            let deadline = self.watchdogs.deadline();
            self.write_kept(now, word, value);
            self.rearmed |= self.watchdogs.deadline() != deadline;
            return Ok(());
        }
        let before = self.image();
        self.write_kept(now, word, value);
        if self.image() != before {
            self.changed = Some(now);
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

/// One ESP32-S3 running its firmware, on the timeline a run sees.
///
/// [`Esp32s3::run_until`] runs to a cycle and reports each image change and the fault that ends
/// the run, in cycle order, at the same cycles however a run splits its limits.
#[derive(Clone, Debug)]
pub struct Esp32s3 {
    core: Core<sim_xtensa::Esp32s3>,
    soc: Soc,
    /// The drive of each netted pad, as the last reported image left it.
    shown: [PadDrive; PADS],
    /// A halt past the last limit, reported first next run.
    pending: Option<Halt>,
    ended: bool,
}

impl Esp32s3 {
    /// The chip with `segments`, each an address and its bytes, loaded into SRAM and the PC at
    /// `entry`. `netted` names the GPIOs with nets. Every segment lies inside one [`RAM`] window.
    pub fn new<'a>(
        netted: &[u8],
        entry: u32,
        segments: impl IntoIterator<Item = (u32, &'a [u8])>,
    ) -> Esp32s3 {
        let mut sram = PagedRam::new(0, SRAM);
        for (addr, data) in segments {
            let offset = Soc::sram_offset(addr, false).expect("segment inside an SRAM window");
            sram.write_bytes(offset, data);
        }
        let mut core = Core::new();
        core.set_pc(entry);
        let soc = Soc {
            sram,
            gpio: Gpio::new(netted),
            systimer: Systimer::new(),
            watchdogs: Watchdogs::new(),
            usb: UsbSerialJtag::default(),
            netted: netted.to_vec(),
            unmodeled: BTreeMap::new(),
            changed: None,
            rearmed: false,
            first_touch: None,
            unmapped_fetch: false,
        };
        Esp32s3 {
            core,
            shown: soc.image(),
            soc,
            pending: None,
            ended: false,
        }
    }

    /// Runs through cycle `limit`, and returns the first image change or fault at or before it.
    /// `None` when the chip reached the limit, or ended before.
    pub fn run_until(&mut self, limit: Cycle) -> Option<Halt> {
        loop {
            if let Some(halt) = self.pending {
                if halt.at() > limit {
                    return None;
                }
                self.pending = None;
                match halt {
                    Halt::Image(_) => self.shown = self.soc.image(),
                    Halt::Fault(..) => self.ended = true,
                }
                return Some(halt);
            }
            if self.ended {
                return None;
            }
            let watchdog = self.soc.watchdogs.deadline();
            let end = Cycle(limit.0 + 1);
            let until = watchdog.map_or(end, |(at, _)| at.min(end));
            let stop = self.core.run_until(&mut self.soc, until);
            let changed = self.soc.changed.take();
            self.soc.rearmed = false;
            if let Some((at, which)) = watchdog
                && self.core.cycle() >= at
                && self.soc.watchdogs.deadline() == watchdog
            {
                self.pending = Some(match changed {
                    Some(change) if change <= at => Halt::Image(change),
                    _ => Halt::Fault(at, Fault::Watchdog(which)),
                });
                continue;
            }
            if let Some(change) = changed {
                self.pending = Some(Halt::Image(change));
                continue;
            }
            match stop {
                Stop::Limit if self.core.cycle() >= end => return None,
                Stop::Limit => {}
                Stop::Bus => {
                    if let Some(addr) = self.soc.first_touch.take() {
                        let pc = self.core.pc();
                        self.soc.unmodeled.get_mut(&addr).expect("touched").pc = pc;
                    } else if self.soc.unmapped_fetch {
                        let pc = self.core.pc();
                        self.pending =
                            Some(Halt::Fault(self.core.cycle(), Fault::UnmappedFetch(pc)));
                    }
                }
                Stop::Unimplemented(missing) => {
                    self.pending =
                        Some(Halt::Fault(self.core.cycle(), Fault::Unsupported(missing)));
                }
            }
        }
    }

    /// What the pad of `gpio`, a netted one, drives in the last reported image.
    pub fn drive(&self, gpio: u8) -> PadDrive {
        self.shown[usize::from(gpio)]
    }

    /// The level the pad of `gpio`, a netted one, reads from now on.
    pub fn set_pad(&mut self, gpio: u8, level: bool) {
        self.soc.gpio.set_pad(gpio, level);
    }

    /// The next console byte whose packet went at or before `through`, with that cycle.
    pub fn take_sent(&mut self, through: Cycle) -> Option<(Cycle, u8)> {
        self.soc.usb.take_sent(through)
    }

    /// Each unmodeled register the firmware touched, by address.
    pub fn unmodeled(&self) -> impl Iterator<Item = (u32, Touch)> + '_ {
        self.soc
            .unmodeled
            .iter()
            .map(|(&addr, &touch)| (addr, touch))
    }

    pub fn cycle(&self) -> Cycle {
        self.core.cycle()
    }
}

#[cfg(test)]
mod tests {
    use sim_xtensa::Sr;

    use super::*;

    const BASE: u32 = 0x4037_0000;
    const GPIO21: u32 = 1 << 21;

    /// A program: literal words from `BASE`, then the code after them.
    struct Program {
        literals: Vec<u32>,
        code: Vec<u8>,
    }

    impl Program {
        fn new(literals: &[u32]) -> Program {
            Program {
                literals: literals.to_vec(),
                code: Vec::new(),
            }
        }

        fn entry(&self) -> u32 {
            BASE + 4 * self.literals.len() as u32
        }

        fn pc(&self) -> u32 {
            self.entry() + self.code.len() as u32
        }

        fn l32r(&mut self, t: u8, literal: usize) -> &mut Program {
            let target = BASE + 4 * literal as u32;
            let imm = (target.wrapping_sub((self.pc() + 3) & !3) as i32 >> 2) as u16;
            self.code.extend([t << 4 | 1, imm as u8, (imm >> 8) as u8]);
            self
        }

        /// `s32i` with `r` 6, `l32i` with `r` 2.
        fn rri8(&mut self, r: u8, t: u8, s: u8, offset: u8) -> &mut Program {
            self.code.extend([t << 4 | 2, r << 4 | s, offset / 4]);
            self
        }

        fn movi(&mut self, t: u8, value: u8) -> &mut Program {
            self.code.extend([t << 4 | 2, 0xa0, value]);
            self
        }

        fn raw(&mut self, bytes: &[u8]) -> &mut Program {
            self.code.extend(bytes);
            self
        }

        fn chip(&self) -> Esp32s3 {
            let mut image: Vec<u8> = self.literals.iter().flat_map(|w| w.to_le_bytes()).collect();
            image.extend(&self.code);
            Esp32s3::new(&[21], self.entry(), [(BASE, image.as_slice())])
        }
    }

    const LOOP: [u8; 3] = [0x06, 0xff, 0xff];
    const WAITI: [u8; 3] = [0x00, 0x70, 0x00];

    /// Enables GPIO21, raises it, reads `GPIO_PIN0`, sends `hi` and spins.
    fn blink() -> Program {
        let mut program = Program::new(&[GPIO, GPIO21, USB_SERIAL_JTAG]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .l32r(5, 2)
            .rri8(6, 3, 2, 0x24)
            .rri8(6, 3, 2, 0x08)
            .rri8(2, 4, 2, 0x74)
            .movi(6, b'h')
            .rri8(6, 6, 5, 0x00)
            .movi(6, b'i')
            .rri8(6, 6, 5, 0x00)
            .movi(6, 1)
            .rri8(6, 6, 5, 0x04)
            .raw(&LOOP);
        program
    }

    /// Each halt and the drive of GPIO21 after it, through `through`, taking limits from
    /// `step`.
    fn halts(chip: &mut Esp32s3, through: u64, step: u64) -> Vec<(Halt, PadDrive)> {
        let mut seen = Vec::new();
        let mut limit = 0;
        while limit < through {
            limit = (limit + step).min(through);
            while let Some(halt) = chip.run_until(Cycle(limit)) {
                assert!(halt.at() <= Cycle(limit), "{halt:?} past {limit}");
                seen.push((halt, chip.drive(21)));
            }
        }
        seen
    }

    #[test]
    fn a_drive_change_halts_at_its_store_however_the_run_splits() {
        let mut whole = blink().chip();
        assert_eq!(whole.drive(21), PadDrive::Off);
        let seen = halts(&mut whole, 1000, 1000);
        let drives: Vec<PadDrive> = seen.iter().map(|&(_, drive)| drive).collect();
        assert_eq!(drives, [PadDrive::Low, PadDrive::High]);
        assert!(seen.iter().all(|(halt, _)| matches!(halt, Halt::Image(_))));
        for step in [1, 2, 3, 7] {
            let mut split = blink().chip();
            assert_eq!(halts(&mut split, 1000, step), seen, "limits {step} apart");
            assert_eq!(split.cycle(), whole.cycle());
        }
    }

    #[test]
    fn the_console_sends_at_wr_done_and_an_unmodeled_read_is_listed_with_its_pc() {
        let program = blink();
        let mut chip = program.chip();
        halts(&mut chip, 1000, 1000);
        let (first, h) = chip.take_sent(Cycle(1000)).expect("h");
        assert_eq!(h, b'h');
        assert_eq!(chip.take_sent(Cycle(1000)), Some((first, b'i')));
        assert_eq!(chip.take_sent(Cycle(u64::MAX)), None);
        let touched: Vec<(u32, Touch)> = chip.unmodeled().collect();
        assert_eq!(touched.len(), 1);
        let (address, touch) = touched[0];
        assert_eq!(Touch::register(address), "GPIO+0x74");
        assert!(!touch.write);
        assert_eq!(touch.pc, program.entry() + 5 * 3);
        assert!(touch.at < first);
    }

    #[test]
    fn a_watchdog_left_running_ends_the_run_at_its_first_stage() {
        let mut program = Program::new(&[]);
        program.raw(&WAITI).raw(&LOOP);
        let mut chip = program.chip();
        let (at, which) = Watchdogs::new()
            .deadline()
            .expect("a watchdog runs at power-on");
        assert_eq!(chip.run_until(Cycle(at.0 - 1)), None);
        assert_eq!(
            chip.run_until(Cycle(u64::MAX - 1)),
            Some(Halt::Fault(at, Fault::Watchdog(which)))
        );
        assert_eq!(chip.run_until(Cycle(u64::MAX - 1)), None, "ended");
    }

    /// `l32r`, `l32r`, then `s32i` of the second literal to the first plus `offset`.
    fn store_after_two_loads(address: u32, value: u32, offset: u8) -> Program {
        let mut program = Program::new(&[address, value]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, offset)
            .raw(&LOOP);
        program
    }

    #[test]
    fn a_drive_change_lands_on_the_cycle_the_store_reaches_the_bus() {
        let mut chip = store_after_two_loads(GPIO, GPIO21, 0x24).chip();
        let image = chip.run_until(Cycle(1000));
        assert_eq!(chip.drive(21), PadDrive::Low);

        let mut probe = store_after_two_loads(0x6000_0000, 0, 0).chip();
        assert_eq!(probe.run_until(Cycle(1000)), None);
        let (_, touch) = probe.unmodeled().next().expect("the store touched UART0");
        assert_eq!(image, Some(Halt::Image(touch.at)));
    }

    #[test]
    fn a_drive_change_on_the_watchdogs_last_cycle_comes_before_its_fault() {
        // Holds timer group 0's first stage to `hold` cycles, then raises GPIO21's enable.
        let program = |hold: u32| {
            let mut program = Program::new(&[TIMG0, hold, GPIO, GPIO21]);
            program
                .l32r(2, 0)
                .l32r(3, 1)
                .rri8(6, 3, 2, 0x50)
                .l32r(4, 2)
                .l32r(5, 3)
                .rri8(6, 5, 4, 0x24)
                .raw(&LOOP);
            program
        };
        let Some(Halt::Image(store)) = program(1000).chip().run_until(Cycle(1000)) else {
            panic!("the store changes GPIO21");
        };
        let at = Cycle(store.0 + 1);
        for step in [1, 1000] {
            let mut chip = program(at.0 as u32).chip();
            assert_eq!(
                halts(&mut chip, 1000, step),
                [
                    (Halt::Image(store), PadDrive::Low),
                    (
                        Halt::Fault(at, Fault::Watchdog(Watchdog::Timg0)),
                        PadDrive::Low
                    ),
                ],
                "limits {step} apart"
            );
        }

        let mut chip = program(1).chip();
        let seen = halts(&mut chip, 1000, 1000);
        assert!(
            matches!(
                seen[..],
                [(Halt::Fault(at, Fault::Watchdog(Watchdog::Timg0)), PadDrive::Off)] if at < store
            ),
            "a hold below the count ends the run before the drive changes: {seen:?}"
        );
    }

    #[test]
    fn a_fetch_outside_the_instruction_window_ends_the_run() {
        let mut chip = Esp32s3::new(&[], 0x4000_0400, []);
        assert_eq!(
            chip.run_until(Cycle(10)),
            Some(Halt::Fault(Cycle(0), Fault::UnmappedFetch(0x4000_0400)))
        );
        let data = RAM[1].start;
        let mut chip = Esp32s3::new(&[], data, [(data, [0u8; 4].as_slice())]);
        assert_eq!(
            chip.run_until(Cycle(10)),
            Some(Halt::Fault(Cycle(0), Fault::UnmappedFetch(data))),
            "the data bus does not fetch"
        );
    }

    /// `iram l8 +=01` and `iram s8 +=01` of the board's `s3-unaligned.txt`: a byte through the
    /// instruction bus raises cause 3 with the address in `EXCVADDR`, and the store writes
    /// nothing.
    #[test]
    fn a_byte_through_the_instruction_bus_takes_cause_3() {
        let at = 0x4037_8dc1;
        for (r, op) in [(0, "l8ui"), (4, "s8i")] {
            let mut program = Program::new(&[at, 0xa5]);
            program.l32r(2, 0).l32r(3, 1).rri8(r, 3, 2, 0);
            let mut chip = program.chip();
            chip.run_until(Cycle(100));
            assert_eq!(chip.core.special(Sr::ExcCause), 3, "{op}");
            assert_eq!(chip.core.special(Sr::ExcVaddr), at, "{op}");
            assert_eq!(chip.soc.load(Cycle(0), at - 1, Width::Word), Ok(0), "{op}");
        }

        let mut soc = Esp32s3::new(&[], BASE, []).soc;
        let iram = RAM[0].start + 0x8040;
        for width in [Width::Byte, Width::Half] {
            assert_eq!(
                soc.load(Cycle(0), iram, width),
                Err(Exit::Fault(Cause::LoadStoreError))
            );
            assert_eq!(
                soc.store(Cycle(0), iram, width, 1),
                Err(Exit::Fault(Cause::LoadStoreError))
            );
        }
        for width in [Width::Half, Width::Word] {
            assert_eq!(
                soc.load(Cycle(0), iram + 1, width),
                Err(Exit::Fault(Cause::LoadStoreAlignment))
            );
            assert_eq!(
                soc.store(Cycle(0), iram + 1, width, 1),
                Err(Exit::Fault(Cause::LoadStoreAlignment))
            );
        }
        assert_eq!(
            soc.load(Cycle(0), RAM[1].start + 0x40, Width::Byte),
            Ok(0),
            "the data bus takes a byte"
        );
    }

    #[test]
    fn both_windows_reach_the_same_sram() {
        let mut soc = Esp32s3::new(&[], BASE, []).soc;
        let through_data = RAM[1].start + 0x40;
        let through_code = RAM[0].start + 0x8040;
        soc.store(Cycle(0), through_data, Width::Word, 0x1234_5678)
            .unwrap();
        assert_eq!(
            soc.load(Cycle(0), through_code, Width::Word),
            Ok(0x1234_5678)
        );
        assert_eq!(
            soc.load(Cycle(0), 0x5000_0000, Width::Word),
            Err(Exit::Fault(Cause::LoadStoreError))
        );
    }
}
