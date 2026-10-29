//! The ESP32-S3 as one Xtensa LX7 core on its internal SRAM and RTC memory, with GPIO, IO_MUX
//! and the GPIO matrix, the interrupt matrix, SYSTIMER, the watchdogs, the USB Serial/JTAG
//! console, SPI2, and for a chip booted from flash the mask ROM, the flash behind SPI1 and the
//! MMU, and the registers the boot path reads.
//!
//! The chip starts in one of two ways. [`Esp32s3::new`] starts it as a bare-metal ELF leaves it:
//! the segments in SRAM, the PC at the entry, the core's reset state otherwise, no ROM and no
//! flash, and the CPU on the 40 MHz crystal. [`Esp32s3::boot`] starts it as the board's
//! bootloader leaves an ESP-IDF app at its entry, from the captured state in `boot`. Every
//! peripheral counts the 480 MHz [`Tick`]s of `clock`, and the CPU's cycles map to ticks through
//! the clock `SYSTEM_SYSCLK_CONF_REG` and `SYSTEM_CPU_PER_CONF_REG` pick.
//!
//! The 512 KB SRAM shows through two windows: from `0x4037_0000` on the instruction bus, and
//! from `0x3fc8_8000` on the data bus, 32 KB into it. The core fetches through the first, the
//! ROM, the instruction cache window and RTC fast memory. It loads from every memory, and stores
//! to SRAM, RTC memory and the peripherals. Through the instruction bus only an aligned word goes: a
//! byte or halfword raises `LoadStoreError`, and an unaligned access `LoadStoreAlignment`, as
//! the board does. A load or store anywhere else outside the peripherals raises
//! `LoadStoreError`. A fetch from anywhere else ends the run.
//!
//! A peripheral register the model does not keep reads 0 and drops writes. [`Esp32s3::unmodeled`]
//! lists each one the firmware touched, with the first access's cycle and PC. That access stops
//! the core once, so the timeline can read the PC, and then runs again. A byte or halfword
//! access to a kept register reads or merges into its word.

mod analog;
mod blocks;
mod boot;
mod calibration;
mod clock;
mod efuse;
mod extmem;
mod flash;
mod gates;
mod gdma;
mod gpio;
mod ledc;
mod matrix;
mod rom;
mod sha;
mod soc;
mod spi1;
mod spi2;
mod timers;
mod usb;

use std::sync::Arc;

use sim_xtensa::{Core, Stop, Unimplemented};

use clock::{CpuClock, Source, XTAL};
use flash::Flash;
use gpio::PADS;
use soc::{Ending, Soc};
use timers::Watchdogs;

pub use boot::Segment;
pub use clock::{TICK_HZ, Tick};
pub use gpio::PadDrive;
pub use spi1::FlashRefusal;
pub use spi2::{Pads, Phase, Setting, Transfer};
pub use timers::Watchdog;

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

/// Why a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The next instruction needs behavior the core does not model.
    Unsupported(Unimplemented),
    /// The core fetched outside the instruction window, at this PC.
    UnmappedFetch(u32),
    /// The core reached a jump to itself at this PC that nothing can take it out of.
    Halted(u32),
    /// A watchdog ran out its first stage.
    Watchdog(Watchdog),
    /// The store at this PC started an SPI2 transfer with a setting the model does not run.
    Setting(Setting, u32),
    /// The store at this PC started an eFuse program command, which would burn eFuses.
    EfuseProgram(u32),
    /// The store at this PC sent the flash a command the model refuses.
    FlashCommand(FlashRefusal, u32),
    /// The store at this PC picked a CPU clock the model does not run: the RC fast clock, or a
    /// reserved setting.
    Clock(u32),
    /// The store at this PC reset the chip for this cause.
    FaultRestart(ResetCause, u32),
    /// The store at this PC set something the model does not run.
    Refused(Refused, u32),
    /// The load at `pc` read `address` in a cache window, where the MMU maps no flash.
    UnmappedLoad { address: u32, pc: u32 },
}

/// Why firmware reset the chip, when the reset ends the run: the cause ESP-IDF records in
/// `RTC_CNTL_STORE6_REG` before the reset, or none the model can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetCause {
    Panic,
    InterruptWatchdog,
    TaskWatchdog,
    Watchdog,
    Brownout,
    Unreadable,
}

/// A write that sets something the model does not run, which ends the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// An interrupt enable of this peripheral, whose interrupt the model never raises.
    Interrupt(&'static str),
    /// A setting of this peripheral, by name, that the model does not run.
    Register(&'static str, &'static str),
    /// `RTC_CNTL_SLEEP_EN`.
    Sleep,
}

/// Where a run stopped before its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    /// At `at` a pad's drive changed, an SPI2 transfer ended, or both. [`Esp32s3::drive`] gives
    /// the new drives, and [`Esp32s3::take_transfers`] the transfer.
    Change { at: Tick, image: bool, record: bool },
    /// The run ended at this tick. The chip runs no further.
    Fault(Tick, Fault),
}

impl Halt {
    pub fn at(self) -> Tick {
        match self {
            Halt::Change { at, .. } | Halt::Fault(at, _) => at,
        }
    }
}

/// The first access to a peripheral register the model does not keep, or the first write that
/// changes a register whose effect it does not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub at: Tick,
    pub pc: u32,
    pub write: bool,
    /// The model keeps the register's word and does not run what the write does.
    pub kept: bool,
}

impl Touch {
    /// The register as its block and offset, such as `TIMG0+0x24`.
    pub fn register(address: u32) -> String {
        blocks::name(address)
    }
}

/// What a chip booted from flash starts from: the ROM ELF, the whole flash, and the app image
/// the bootloader would pick there, as its entry and segments.
pub struct Boot<'a> {
    pub rom: &'a [u8],
    pub flash: Arc<[u8]>,
    pub entry: u32,
    pub segments: &'a [Segment],
}

/// One ESP32-S3 running its firmware, on the timeline a run sees.
///
/// [`Esp32s3::run_until`] runs to a cycle and reports each image change, each SPI2 transfer's
/// end and the fault that ends the run, in cycle order, at the same cycles however a run splits
/// its limits.
#[derive(Clone, Debug)]
pub struct Esp32s3 {
    core: Core<sim_xtensa::Esp32s3>,
    soc: Soc,
    /// The drive of each netted pad, as the last reported image left it.
    shown: [PadDrive; PADS],
    /// The tick of an image change not reported yet.
    changed: Option<Tick>,
    /// How many of SPI2's transfers have had their end reported and wait to be taken.
    reported: usize,
    /// The fault that ends the run, not reported yet.
    fault: Option<(Tick, Fault)>,
    ended: bool,
    /// The tick and PC of the store that reset the chip, which holds it in reset from then on.
    restarted: Option<(Tick, u32)>,
}

impl Esp32s3 {
    /// The chip with `segments`, each an address and its bytes, loaded into SRAM and the PC at
    /// `entry`. `netted` names the GPIOs with nets. Every segment lies inside one [`RAM`] window.
    pub fn new<'a>(
        netted: &[u8],
        entry: u32,
        segments: impl IntoIterator<Item = (u32, &'a [u8])>,
    ) -> Esp32s3 {
        let mut soc = Soc::new(CpuClock::new(Source::Xtal(1)), netted, Watchdogs::new(XTAL));
        soc.keep_crystal_clock();
        for (addr, data) in segments {
            assert!(
                Soc::sram_offset(addr, false).is_some(),
                "segment inside an SRAM window"
            );
            soc.poke(addr, data);
        }
        let mut core = Core::new();
        core.set_pc(entry);
        Esp32s3::with(core, soc)
    }

    /// The chip booted from `boot.flash`, as the board's bootloader leaves it at the app's
    /// entry: the ROM's data in SRAM, the captured state applied, then the app's segments
    /// loaded or mapped, and the PC at `boot.entry`.
    pub fn boot(netted: &[u8], boot: Boot) -> Esp32s3 {
        let image = rom::load(boot.rom, |addr, len| {
            Soc::sram_offset(addr, false).is_some()
                && Soc::sram_offset(addr + len as u32 - 1, false).is_some()
        })
        .expect("the pinned ROM ELF");
        let state: Vec<boot::Value> = boot::state().collect();
        let source = boot::clock_source(&state).expect("the captured clock is one the model runs");
        let mut soc = Soc::new(CpuClock::new(source), netted, Watchdogs::new(source.apb()));
        soc.attach(image.rom, Flash::new(boot.flash));
        for (addr, bytes) in &image.data {
            soc.poke(*addr, bytes);
        }
        let mut core = Core::new();
        boot::apply(&state, &mut core, &mut soc);
        for segment in boot.segments {
            let flash = soc.flash.as_mut().expect("attached");
            if segment.mapped() {
                flash.map(segment.addr, segment.offset, segment.len);
            } else {
                let at = segment.offset as usize;
                let bytes = flash.bytes()[at..at + segment.len as usize].to_vec();
                soc.poke(segment.addr, &bytes);
            }
        }
        core.set_pc(boot.entry);
        Esp32s3::with(core, soc)
    }

    fn with(core: Core<sim_xtensa::Esp32s3>, soc: Soc) -> Esp32s3 {
        Esp32s3 {
            core,
            shown: soc.image(),
            soc,
            changed: None,
            reported: 0,
            fault: None,
            ended: false,
            restarted: None,
        }
    }

    /// Runs through tick `limit`, and returns the first image change, transfer end or fault at
    /// or before it. `None` when the chip reached the limit, or ended before.
    pub fn run_until(&mut self, limit: Tick) -> Option<Halt> {
        loop {
            if let Some(halt) = self.next_halt() {
                if halt.at() > limit {
                    return None;
                }
                self.report(halt);
                return Some(halt);
            }
            if self.ended || self.fault.is_some() || self.restarted.is_some() || self.now() > limit
            {
                return None;
            }
            let watchdog = self
                .soc
                .watchdogs
                .deadline()
                .map(|(at, which)| (self.soc.clock.round_up(at), which));
            let until = [
                Some(Tick(limit.0 + 1)),
                watchdog.map(|(at, _)| at),
                self.soc.spi2.end(self.reported),
            ]
            .into_iter()
            .flatten()
            .min()
            .expect("the limit");
            let until = self.soc.clock.cycle_after(until);
            let stop = self.core.run_until(&mut self.soc, until);
            if let Some(change) = self.soc.changed.take() {
                debug_assert!(self.changed.is_none(), "a change stops the core");
                self.changed = Some(change);
            }
            self.soc.rearmed = false;
            self.soc.reclocked = false;
            self.soc.started = false;
            let rounded = self
                .soc
                .watchdogs
                .deadline()
                .map(|(at, which)| (self.soc.clock.round_up(at), which));
            if let Some((at, which)) = watchdog
                && self.now() >= at
                && rounded == watchdog
            {
                self.fault = Some((at, Fault::Watchdog(which)));
                continue;
            }
            let pc = self.core.pc();
            let now = self.now();
            match stop {
                Stop::Limit => {}
                Stop::Bus => {
                    if let Some(addr) = self.soc.first_touch.take() {
                        self.soc.unmodeled.get_mut(&addr).expect("touched").pc = pc;
                    } else if std::mem::take(&mut self.soc.change_stop) {
                        self.soc.pending.as_mut().expect("reported").2.pc = pc;
                    } else if let Some(ending) = self.soc.ending.take() {
                        let fault = match ending {
                            Ending::Setting(setting) => Fault::Setting(setting, pc),
                            Ending::EfuseProgram => Fault::EfuseProgram(pc),
                            Ending::FlashCommand(refusal) => Fault::FlashCommand(refusal, pc),
                            Ending::Restart(None) => {
                                self.hold_in_reset(now, pc);
                                continue;
                            }
                            Ending::Restart(Some(cause)) => Fault::FaultRestart(cause, pc),
                            Ending::Clock => Fault::Clock(pc),
                            Ending::Refused(refused) => Fault::Refused(refused, pc),
                            Ending::UnmappedLoad(address) => Fault::UnmappedLoad { address, pc },
                        };
                        self.fault = Some((now, fault));
                    } else if self.soc.unmapped_fetch {
                        self.fault = Some((now, Fault::UnmappedFetch(pc)));
                    }
                }
                Stop::Spin => {
                    let nmi = self.soc.routes_to_nmi();
                    if !nmi && self.soc.watchdogs.deadline().is_none() {
                        self.fault = Some((now, Fault::Halted(pc)));
                    }
                }
                Stop::Unimplemented(missing) => {
                    self.fault = Some((now, Fault::Unsupported(missing)));
                }
            }
        }
    }

    /// Holds the chip in reset from `now`, after the store at `pc` reset it from `esp_restart`.
    /// The boot ROM does not run, so the core runs no further, and each pad drives as at
    /// power-on.
    fn hold_in_reset(&mut self, now: Tick, pc: u32) {
        self.restarted = Some((now, pc));
        self.soc.reset_pads();
        if self.soc.image() != self.shown {
            self.changed = Some(now);
        }
    }

    /// The tick and PC of the store that reset the chip, if one did.
    pub fn restart(&self) -> Option<(Tick, u32)> {
        self.restarted
    }

    /// The tick of the core's next cycle.
    fn now(&self) -> Tick {
        self.soc.clock.tick(self.core.cycle())
    }

    /// The earliest halt the core has reached and not reported. An image change and a transfer
    /// end at one cycle are one halt, and either comes before a fault at that cycle.
    fn next_halt(&self) -> Option<Halt> {
        if self.ended {
            return None;
        }
        let record = self
            .soc
            .spi2
            .end(self.reported)
            .filter(|&end| end <= self.now());
        let change = [self.changed, record].into_iter().flatten().min();
        match (change, self.fault) {
            (Some(at), Some((fault_at, fault))) if fault_at < at => {
                Some(Halt::Fault(fault_at, fault))
            }
            (Some(at), _) => Some(Halt::Change {
                at,
                image: self.changed == Some(at),
                record: record == Some(at),
            }),
            (None, Some((at, fault))) => Some(Halt::Fault(at, fault)),
            (None, None) => None,
        }
    }

    fn report(&mut self, halt: Halt) {
        match halt {
            Halt::Change { image, record, .. } => {
                if image {
                    self.shown = self.soc.image();
                    self.changed = None;
                }
                if record {
                    self.reported += 1;
                }
            }
            Halt::Fault(..) => {
                self.fault = None;
                self.ended = true;
            }
        }
    }

    /// The SPI2 transfers whose ends were reported, in the order they started.
    pub fn take_transfers(&mut self) -> impl Iterator<Item = Transfer> + '_ {
        let n = std::mem::take(&mut self.reported);
        self.soc.spi2.take(n)
    }

    /// What the pad of `gpio`, a netted one, drives in the last reported image.
    pub fn drive(&self, gpio: u8) -> PadDrive {
        self.shown[usize::from(gpio)]
    }

    /// The level the pad of `gpio`, a netted one, reads from now on.
    pub fn set_pad(&mut self, gpio: u8, level: bool) {
        self.soc.gpio.set_pad(gpio, level);
    }

    /// The next console byte whose packet went at or before `through`, with that tick.
    pub fn take_sent(&mut self, through: Tick) -> Option<(Tick, u8)> {
        self.soc.usb.take_sent(through)
    }

    /// Queues `bytes` for the console to receive from `at`, after every byte queued before.
    pub fn receive(&mut self, at: Tick, bytes: &[u8]) {
        self.soc.usb.receive(at, bytes);
    }

    /// Each unmodeled register the firmware touched, by address, then each write that changed
    /// a register the model keeps without running its effect, in the order written.
    pub fn unmodeled(&self) -> impl Iterator<Item = (u32, Touch)> + '_ {
        self.soc
            .unmodeled
            .iter()
            .chain(self.soc.changes.iter().map(|(addr, touch)| (addr, touch)))
            .map(|(&addr, &touch)| (addr, touch))
    }

    /// The tick of the core's next cycle.
    pub fn tick(&self) -> Tick {
        self.now()
    }
}

#[cfg(test)]
mod tests {
    use sim_xtensa::{Bus, Cause, Cycle, Exit, Sr, Width};

    use super::soc::{
        EFUSE, Ending, GDMA, GPIO, IO_MUX, RTC_CNTL, SPI2, SYSTEM, TIMG0, TIMG1, USB_SERIAL_JTAG,
    };
    use super::*;

    /// The tick of crystal cycle `n`, a CPU cycle at power-on.
    fn c(n: u64) -> Tick {
        Tick(n * XTAL)
    }

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
            self.chip_on(&[21])
        }

        fn chip_on(&self, netted: &[u8]) -> Esp32s3 {
            let mut image: Vec<u8> = self.literals.iter().flat_map(|w| w.to_le_bytes()).collect();
            image.extend(&self.code);
            Esp32s3::new(netted, self.entry(), [(BASE, image.as_slice())])
        }
    }

    fn image_at(at: Tick) -> Halt {
        Halt::Change {
            at,
            image: true,
            record: false,
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
            while let Some(halt) = chip.run_until(c(limit)) {
                assert!(halt.at() <= c(limit), "{halt:?} past {limit}");
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
        assert!(seen.iter().all(|(halt, _)| matches!(
            halt,
            Halt::Change {
                image: true,
                record: false,
                ..
            }
        )));
        for step in [1, 2, 3, 7] {
            let mut split = blink().chip();
            assert_eq!(halts(&mut split, 1000, step), seen, "limits {step} apart");
            assert_eq!(split.tick(), whole.tick());
        }
    }

    /// `RTC_CNTL_STORE6_REG`, where IDF records why it resets.
    const STORE6: u32 = RTC_CNTL + 0xc8;

    /// IDF's record of reset reason `hint` in `RTC_CNTL_STORE6_REG`.
    fn hint(hint: u32) -> u32 {
        hint | hint << 16 | 1 << 31
    }

    /// Raises GPIO21, then sets `RTC_CNTL_SW_SYS_RST`, which is the store at the PC it returns,
    /// on cycle 6: two loads, two stores, two loads.
    fn reset() -> (Program, u32) {
        let mut program = Program::new(&[GPIO, GPIO21, RTC_CNTL, 1 << 31]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0x24)
            .rri8(6, 3, 2, 0x08)
            .l32r(4, 2)
            .l32r(5, 3);
        let at = program.pc();
        program.rri8(6, 5, 4, 0x00).raw(&LOOP);
        (program, at)
    }

    /// A reset from `esp_restart`, which leaves `RTC_CNTL_STORE6_REG` 0, holds the chip from its
    /// store on: GPIO21 drops its drive at that cycle as at power-on, and the core runs no
    /// further, however the run splits its limits.
    #[test]
    fn a_reset_from_esp_restart_releases_the_pads_and_holds_the_core() {
        let (program, at) = reset();
        let chip = || {
            let mut chip = program.chip();
            chip.soc.restore(STORE6, 0);
            chip
        };
        let mut whole = chip();
        let seen = halts(&mut whole, 1000, 1000);
        let drives: Vec<PadDrive> = seen.iter().map(|&(_, drive)| drive).collect();
        assert_eq!(drives, [PadDrive::Low, PadDrive::High, PadDrive::Off]);
        assert_eq!(seen[2].0, image_at(c(6)));
        assert_eq!(whole.restart(), Some((c(6), at)));
        assert_eq!(whole.run_until(c(100_000)), None);
        assert_eq!(whole.tick(), c(6), "the core stays at its reset");
        for step in [1, 2, 3, 7] {
            let mut split = chip();
            assert_eq!(halts(&mut split, 1000, step), seen, "limits {step} apart");
            assert_eq!(split.restart(), whole.restart(), "limits {step} apart");
            assert_eq!(split.tick(), c(6), "limits {step} apart");
        }
        let mut chip = chip();
        chip.soc.restore(STORE6, hint(3));
        halts(&mut chip, 1000, 1000);
        assert_eq!(chip.restart(), Some((c(6), at)), "a hint of ESP_RST_SW");
    }

    /// A reset after IDF recorded a panic or a watchdog, or with no reason the model can read,
    /// ends the run at its store.
    #[test]
    fn a_reset_after_a_panic_or_a_watchdog_or_with_no_readable_reason_ends_the_run() {
        let (program, at) = reset();
        let cases = [
            (Some(hint(4)), ResetCause::Panic),
            (Some(hint(5)), ResetCause::InterruptWatchdog),
            (Some(hint(6)), ResetCause::TaskWatchdog),
            (Some(hint(7)), ResetCause::Watchdog),
            (Some(hint(9)), ResetCause::Brownout),
            (Some(hint(8)), ResetCause::Unreadable),
            (Some(1 << 31 | 5 << 16 | 4), ResetCause::Unreadable),
            (Some(0x4037_8000), ResetCause::Unreadable),
            (None, ResetCause::Unreadable),
        ];
        for (store6, cause) in cases {
            let mut chip = program.chip();
            if let Some(word) = store6 {
                chip.soc.restore(STORE6, word);
            }
            let seen = halts(&mut chip, 1000, 1000);
            assert_eq!(
                seen.last().map(|&(halt, _)| halt),
                Some(Halt::Fault(c(6), Fault::FaultRestart(cause, at))),
                "{store6:x?}"
            );
            assert_eq!(chip.restart(), None, "{store6:x?}");
        }
    }

    /// The core runs no cycle at or after a watchdog's deadline, so a store that ends the run
    /// before it names the store, and one due at the deadline never runs: timer group 0's hold
    /// is written to `hold` cycles, then an eFuse program command is due at cycle 5.
    #[test]
    fn a_store_that_ends_the_run_before_the_watchdog_runs_out_names_the_store() {
        for hold in 3..=7 {
            let mut program = Program::new(&[TIMG0, hold, EFUSE + 0x1d4, 2]);
            program
                .l32r(2, 0)
                .l32r(3, 1)
                .rri8(6, 3, 2, 0x50)
                .l32r(4, 2)
                .l32r(5, 3);
            let store = program.pc();
            program.rri8(6, 5, 4, 0x00).raw(&LOOP);
            let want = if u64::from(hold) <= 5 {
                Halt::Fault(c(hold.into()), Fault::Watchdog(Watchdog::Timg0))
            } else {
                Halt::Fault(c(5), Fault::EfuseProgram(store))
            };
            for step in [1, 1000] {
                let mut chip = program.chip();
                let seen = halts(&mut chip, 1000, step);
                assert_eq!(
                    seen.iter().map(|&(halt, _)| halt).collect::<Vec<_>>(),
                    [want],
                    "hold {hold}, limits {step} apart"
                );
            }
        }
    }

    /// The core stops at the watchdog's deadline, so a store after it never reaches the bus.
    #[test]
    fn the_core_runs_no_store_after_the_watchdog_runs_out() {
        let mut program = Program::new(&[TIMG0, 3, 0x6000_0000]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0x50)
            .l32r(4, 2)
            .rri8(6, 3, 4, 0)
            .raw(&LOOP);
        for step in [1, 1000] {
            let mut chip = program.chip();
            assert_eq!(
                halts(&mut chip, 1000, step),
                [(
                    Halt::Fault(c(3), Fault::Watchdog(Watchdog::Timg0)),
                    PadDrive::Off
                )],
                "limits {step} apart"
            );
            assert_eq!(chip.unmodeled().count(), 0, "limits {step} apart");
        }
    }

    /// A byte store of `EFUSE_PGM_CMD` merges into `EFUSE_CMD_REG`, so it ends the run as a
    /// word store does.
    #[test]
    fn a_byte_store_of_the_program_command_ends_the_run() {
        let mut program = Program::new(&[EFUSE + 0x1d4]);
        program.l32r(2, 0).movi(3, 2);
        let store = program.pc();
        program.rri8(4, 3, 2, 0).raw(&LOOP);
        assert_eq!(
            program.chip().run_until(c(1000)),
            Some(Halt::Fault(c(2), Fault::EfuseProgram(store)))
        );
    }

    /// Each write that turns on an interrupt the model never raises, a GDMA mode it does not run,
    /// or sleep, ends the run at its store, and a write that leaves those bits clear does not.
    #[test]
    fn an_interrupt_the_model_never_raises_a_gdma_mode_or_sleep_ends_the_run() {
        let gdma = 0x6003_f000;
        let out = gdma + 4 * 0xc0 + 0x60;
        let refused = [
            (SPI2 + 0x34, 1 << 10, Refused::Interrupt("SPI2")),
            (TIMG0 + 0x70, 1, Refused::Interrupt("TIMG0")),
            (TIMG1 + 0x70, 2, Refused::Interrupt("TIMG1")),
            (
                USB_SERIAL_JTAG + 0x10,
                1 << 3,
                Refused::Interrupt("USB_SERIAL_JTAG"),
            ),
            (gdma + 0x10, 1, Refused::Interrupt("GDMA")),
            (out + 0x10, 1, Refused::Interrupt("GDMA")),
            (0x6003_b028, 1, Refused::Interrupt("SHA")),
            (0x6001_90c8, 1 << 3, Refused::Interrupt("LEDC")),
            (gdma, 1 << 4, Refused::Register("GDMA", "memory_copy")),
            (out, 1 << 2, Refused::Register("GDMA", "write_back")),
            (gdma + 0x20, 1 << 23, Refused::Register("GDMA", "restart")),
            (out + 0x20, 1 << 22, Refused::Register("GDMA", "restart")),
            (RTC_CNTL + 0x18, 1 << 31, Refused::Sleep),
        ];
        for (address, value, why) in refused {
            let program = store_after_two_loads(address, value, 0);
            let mut chip = program.chip();
            chip.soc.restore(SYSTEM + 0x18, 1 << 6 | 1 << 11);
            chip.soc.restore(SYSTEM + 0x1c, 1 << 6 | 1 << 2);
            chip.soc.restore(SYSTEM + 0x24, 0);
            assert_eq!(
                chip.run_until(c(1000)),
                Some(Halt::Fault(c(2), Fault::Refused(why, program.entry() + 6))),
                "{address:#x}"
            );
        }
        let mut chip = store_after_two_loads(TIMG0 + 0x70, 1 << 2, 0).chip();
        assert_eq!(chip.run_until(c(1000)), None, "the watchdog's interrupt");
    }

    /// Each write that changes a register the model keeps without running its effect is
    /// listed, with the PC of its own store, and the word keeps it. A write that leaves the word
    /// as it was is not listed. On SYSTEM's gates only a change of a bit the model keeps without
    /// running counts.
    #[test]
    fn each_change_to_a_register_whose_effect_does_not_run_is_listed_and_kept() {
        let system = [
            (0x18, 1 << 29),
            (0x1c, 1 << 10),
            (0x20, 1 << 13),
            (0x24, 1 << 10),
        ];
        let spi0 = (0x18..=0x20)
            .step_by(4)
            .map(|offset| (0x6000_3000 + offset, 0x40));
        let system = system.map(|(offset, bit)| (SYSTEM + offset, bit));
        for (address, value) in system
            .into_iter()
            .chain(spi0)
            .chain([(RTC_CNTL + 0x74, 0x40)])
        {
            let mut program = Program::new(&[address, value, 0]);
            program.l32r(2, 0).l32r(3, 1).l32r(4, 2);
            let first = program.pc();
            program.rri8(6, 3, 2, 0);
            let second = program.pc();
            program.rri8(6, 4, 2, 0).rri8(6, 4, 2, 0).raw(&LOOP);
            let mut chip = program.chip();
            chip.soc.restore(address, 0);
            assert_eq!(chip.run_until(c(1000)), None, "{address:#x}");
            let listed: Vec<(u32, u32)> =
                chip.unmodeled().map(|(at, touch)| (at, touch.pc)).collect();
            assert_eq!(
                listed,
                [(address, first), (address, second)],
                "{address:#x}"
            );
            assert!(chip.unmodeled().all(|(_, touch)| touch.write && touch.kept));
            assert_eq!(chip.soc.load(Cycle(0), address, Width::Word), Ok(0));
        }
        let mut chip = store_after_two_loads(SYSTEM + 0x18, 1 << 6 | 1 << 2, 0).chip();
        chip.soc.restore(SYSTEM + 0x18, 0);
        assert_eq!(chip.run_until(c(1000)), None);
        assert_eq!(chip.unmodeled().count(), 0, "SPI2 runs its clock bit");
    }

    /// A jump to itself with every interrupt masked ends the run as halted once no watchdog
    /// runs and no source reaches the NMI line, as IDF's print-and-halt panic ends. While a
    /// watchdog runs, or a source is routed to the NMI line, the core spins on.
    #[test]
    fn a_jump_to_itself_that_nothing_can_leave_halts_the_run() {
        let mut program = Program::new(&[]);
        program.raw(&LOOP);
        let quiet = |chip: &mut Esp32s3| {
            chip.soc.watchdogs.write_timg(Tick(0), 0, 0x48, 0);
            chip.soc.watchdogs.write_rtc(Tick(0), 0x98, 0);
        };
        let mut chip = program.chip();
        quiet(&mut chip);
        assert_eq!(
            chip.run_until(c(1000)),
            Some(Halt::Fault(c(0), Fault::Halted(program.entry())))
        );
        assert_eq!(program.chip().run_until(c(1000)), None, "a watchdog runs");
        let mut nmi = program.chip();
        quiet(&mut nmi);
        nmi.soc.restore(0x600c_2000, 14);
        assert_eq!(
            nmi.run_until(c(1000)),
            None,
            "a source reaches the NMI line"
        );
    }

    /// SPI2's reset bit puts its registers back to power-on, and an access while the bit holds
    /// it, or while a block's clock is off, ends the run at that access.
    #[test]
    fn a_peripheral_reset_clears_its_registers_and_a_held_or_unclocked_block_ends_the_run() {
        let user1 = SPI2 + 0x14;
        let mut program = Program::new(&[user1, 0x1234, SYSTEM + 0x20, 1 << 6, 0]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0)
            .l32r(4, 2)
            .l32r(5, 3)
            .l32r(7, 4)
            .rri8(6, 5, 4, 0)
            .rri8(6, 7, 4, 0)
            .raw(&LOOP);
        let mut chip = program.chip();
        assert_eq!(chip.run_until(c(1000)), None);
        assert_eq!(chip.soc.load(Cycle(0), user1, Width::Word), Ok(0xb841_0007));

        let mut program = Program::new(&[SYSTEM + 0x20, 1 << 6, user1]);
        program.l32r(2, 0).l32r(3, 1).rri8(6, 3, 2, 0).l32r(4, 2);
        let load = program.pc();
        program.rri8(2, 5, 4, 0).raw(&LOOP);
        let held = Refused::Register("SPI2", "held_in_reset");
        assert_eq!(
            program.chip().run_until(c(1000)),
            Some(Halt::Fault(c(4), Fault::Refused(held, load)))
        );

        let program = store_after_two_loads(0x6001_9000, 0, 0);
        let off = Refused::Register("LEDC", "clock_off");
        assert_eq!(
            program.chip().run_until(c(1000)),
            Some(Halt::Fault(c(2), Fault::Refused(off, program.entry() + 6)))
        );
    }

    /// A write that would take a flash pad, GPIO26 to GPIO32, from SPI0 and SPI1 ends the run:
    /// another IO_MUX function, its GPIO output enable, or a matrix output. A write that keeps
    /// function 0, as IDF's start-up writes those pads, and the same writes on the pads beside
    /// them, finish.
    #[test]
    fn a_write_that_takes_a_flash_pad_ends_the_run() {
        let takes = |pad: u32| {
            let (enable, bit) = if pad < 32 {
                (0x20, 1 << pad)
            } else {
                (0x2c, 1 << (pad - 32))
            };
            [
                (IO_MUX + 4 + 4 * pad, 1 << 12 | 0x800),
                (GPIO + enable, bit),
                (GPIO + enable + 4, bit),
                (GPIO + 0x554 + 4 * pad, 0x80),
            ]
        };
        for pad in 26..=32 {
            for (address, value) in takes(pad) {
                let program = store_after_two_loads(address, value, 0);
                assert_eq!(
                    program.chip().run_until(c(1000)),
                    Some(Halt::Fault(
                        c(2),
                        Fault::FlashCommand(FlashRefusal::Pads, program.entry() + 6)
                    )),
                    "GPIO{pad} {address:#x}"
                );
            }
            let mut chip = store_after_two_loads(IO_MUX + 4 + 4 * pad, 0x80a, 0).chip();
            assert_eq!(chip.run_until(c(1000)), None, "GPIO{pad} keeps function 0");
        }
        for pad in [21, 33] {
            for (address, value) in takes(pad) {
                let halt = store_after_two_loads(address, value, 0)
                    .chip()
                    .run_until(c(1000));
                assert!(
                    !matches!(halt, Some(Halt::Fault(..))),
                    "GPIO{pad} {address:#x}: {halt:?}"
                );
            }
        }
    }

    /// A command started on SPI0, the cache's controller, ends the run.
    #[test]
    fn a_command_on_spi0_ends_the_run() {
        let program = store_after_two_loads(0x6000_3000, 1 << 18, 0);
        assert_eq!(
            program.chip().run_until(c(1000)),
            Some(Halt::Fault(
                c(2),
                Fault::FlashCommand(FlashRefusal::Spi0, program.entry() + 6)
            ))
        );
    }

    /// A load from a cache window ends the run where the MMU maps no flash: on a chip with no
    /// flash, through an invalid entry, and through a PSRAM entry.
    #[test]
    fn a_load_the_mmu_maps_to_no_flash_ends_the_run() {
        let load = |address: u32| {
            let mut program = Program::new(&[address]);
            program.l32r(2, 0).rri8(2, 3, 2, 0).raw(&LOOP);
            program
        };
        let program = load(0x3c00_0010);
        assert_eq!(
            program.chip().run_until(c(1000)),
            Some(Halt::Fault(
                c(1),
                Fault::UnmappedLoad {
                    address: 0x3c00_0010,
                    pc: program.entry() + 3
                }
            )),
            "no flash"
        );
        for (entry, mapped) in [(0, true), (1 << 14, false), (1 << 15, false)] {
            let mut chip = program.chip();
            let mut flash = Flash::new(vec![0xa5; 0x1_0000].into());
            flash.write_mmu(0, entry);
            chip.soc.flash = Some(flash);
            let halt = chip.run_until(c(1000));
            if mapped {
                assert_eq!(halt, None, "entry {entry:#x}");
                assert_eq!(chip.core.a(3), 0xa5a5_a5a5);
            } else {
                assert!(
                    matches!(halt, Some(Halt::Fault(_, Fault::UnmappedLoad { .. }))),
                    "entry {entry:#x}: {halt:?}"
                );
            }
        }
    }

    #[test]
    fn the_console_sends_at_wr_done_and_an_unmodeled_read_is_listed_with_its_pc() {
        let program = blink();
        let mut chip = program.chip();
        halts(&mut chip, 1000, 1000);
        let (first, h) = chip.take_sent(c(1000)).expect("h");
        assert_eq!(h, b'h');
        assert_eq!(chip.take_sent(c(1000)), Some((first, b'i')));
        assert_eq!(chip.take_sent(Tick(u64::MAX)), None);
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
        let (at, which) = Watchdogs::new(XTAL)
            .deadline()
            .expect("a watchdog runs at power-on");
        assert_eq!(chip.run_until(Tick(at.0 - 1)), None);
        assert_eq!(
            chip.run_until(Tick(u64::MAX - 1)),
            Some(Halt::Fault(at, Fault::Watchdog(which)))
        );
        assert_eq!(chip.run_until(Tick(u64::MAX - 1)), None, "ended");
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
        let image = chip.run_until(c(1000));
        assert_eq!(chip.drive(21), PadDrive::Low);

        let mut probe = store_after_two_loads(0x6000_0000, 0, 0).chip();
        assert_eq!(probe.run_until(c(1000)), None);
        let (_, touch) = probe.unmodeled().next().expect("the store touched UART0");
        assert_eq!(image, Some(image_at(touch.at)));
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
        let Some(Halt::Change { at: store, .. }) = program(1000).chip().run_until(c(1000)) else {
            panic!("the store changes GPIO21");
        };
        let at = Tick(store.0 + XTAL);
        for step in [1, 1000] {
            let mut chip = program((at.0 / XTAL) as u32).chip();
            assert_eq!(
                halts(&mut chip, 1000, step),
                [
                    (image_at(store), PadDrive::Low),
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

    /// Routes GPIO12 to SPI2's CS, turns SPI2's clock on, then syncs and starts a transfer of
    /// an 8-bit command with `user`, and then stores `then`'s value to its address.
    fn transfer(user: u32, then: Option<(u32, u32)>) -> Program {
        let (address, value) = then.unwrap_or_default();
        let mut program = Program::new(&[
            IO_MUX,
            1 << 12,
            GPIO + 0x500,
            110,
            SPI2,
            user,
            1 << 24,
            0x3,
            1 << 23,
            address,
            value,
        ]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0x34)
            .l32r(2, 2)
            .l32r(3, 3)
            .rri8(6, 3, 2, 0x84)
            .l32r(2, 4)
            .l32r(3, 5)
            .rri8(6, 3, 2, 0x10)
            .l32r(3, 7)
            .rri8(6, 3, 2, 0xe8)
            .l32r(3, 8)
            .rri8(6, 3, 2, 0x00)
            .l32r(3, 6)
            .rri8(6, 3, 2, 0x00);
        if then.is_some() {
            program.l32r(2, 9).l32r(3, 10).rri8(6, 3, 2, 0x00);
        }
        program.raw(&LOOP);
        program
    }

    #[test]
    fn a_transfer_ends_in_a_record_halt_however_the_run_splits() {
        let program = transfer(1 << 31, None);
        let run = |step: u64| {
            let mut chip = program.chip_on(&[12]);
            let mut seen = Vec::new();
            let mut limit = 0;
            while limit < 1000 {
                limit = (limit + step).min(1000);
                while let Some(halt) = chip.run_until(c(limit)) {
                    seen.push((
                        halt,
                        chip.drive(12),
                        chip.take_transfers().collect::<Vec<_>>(),
                        chip.tick(),
                    ));
                }
            }
            seen
        };
        let seen = run(1000);
        let [
            (routed, drive, ref none, _),
            (
                Halt::Change {
                    at,
                    image: false,
                    record: true,
                },
                _,
                ref sent,
                core,
            ),
        ] = seen[..]
        else {
            panic!("a CS route, then a transfer: {seen:?}");
        };
        assert!(matches!(
            routed,
            Halt::Change {
                image: true,
                record: false,
                ..
            }
        ));
        assert_eq!(drive, PadDrive::High, "CS idles high once routed");
        assert!(none.is_empty());
        let [ref transfer] = sent[..] else {
            panic!("one transfer: {sent:?}");
        };
        assert_eq!(transfer.end, at);
        assert!(
            core.0 <= at.0 + 2 * XTAL,
            "the core stops in the `j` that reaches the end, not at the limit: {core:?}"
        );
        assert_eq!(
            transfer.end.0 - transfer.start.0,
            8 * XTAL,
            "8 clocks at the crystal"
        );
        assert_eq!(transfer.pads.cs, Some(12));
        assert_eq!(transfer.pads.sclk, None);
        assert_eq!(
            transfer.phases,
            [Phase {
                bits: 8,
                lanes: 1,
                value: 0
            }]
        );
        for step in [1, 2, 3, 7] {
            assert_eq!(run(step), seen, "limits {step} apart");
        }
    }

    #[test]
    fn a_transfer_with_a_setting_the_model_does_not_run_ends_the_run_at_its_store() {
        let program = transfer(1 << 31 | 1 << 29, None);
        let mut chip = program.chip_on(&[]);
        let Some(Halt::Fault(_, fault)) = chip.run_until(c(1000)) else {
            panic!("the store faults");
        };
        assert_eq!(
            fault,
            Fault::Setting(Setting::Dummy, program.entry() + 14 * 3)
        );
        assert_eq!(chip.run_until(c(2000)), None, "ended");
        assert_eq!(chip.take_transfers().count(), 0);
    }

    /// SPI2's reset bit while a transfer runs ends the run at its store, and leaves the gates as
    /// they were.
    #[test]
    fn a_reset_of_spi2_while_it_sends_ends_the_run() {
        let program = transfer(1 << 31, Some((SYSTEM + 0x20, 1 << 6)));
        let mut chip = program.chip_on(&[]);
        let sending = Refused::Register("SPI2", "reset_while_sending");
        assert_eq!(
            chip.run_until(c(1000)),
            Some(Halt::Fault(
                c(17),
                Fault::Refused(sending, program.entry() + 17 * 3)
            ))
        );
        assert_eq!(chip.soc.load(Cycle(0), SYSTEM + 0x20, Width::Word), Ok(0));
    }

    /// A turn of SPI2's clock off while it sends ends the run at its store. A GDMA reset bit
    /// that a transfer from the CPU buffer leaves set does not.
    #[test]
    fn spi2_clock_off_while_it_sends_ends_the_run() {
        let program = transfer(1 << 31, Some((SYSTEM + 0x18, 0xf9c1_e06f & !(1 << 6))));
        let off = Refused::Register("SPI2", "clock_off_while_sending");
        assert_eq!(
            program.chip_on(&[]).run_until(c(1000)),
            Some(Halt::Fault(
                c(17),
                Fault::Refused(off, program.entry() + 17 * 3)
            ))
        );
        let program = transfer(1 << 31, Some((SYSTEM + 0x24, 0x1fe)));
        let halt = program.chip_on(&[]).run_until(c(1000));
        assert!(
            matches!(halt, Some(Halt::Change { record: true, .. })),
            "{halt:?}"
        );
    }

    const DESCRIPTOR: u32 = 0x3fce_0000;
    const BUFFER: u32 = 0x3fca_0000;

    /// A chip with GDMA clocked and out of reset, its channel 0 started on SPI2 at one
    /// descriptor for the 8 bytes at `buffer`, and SPI2 set and synced to send 8 bytes from
    /// DMA, with no CS setup or hold, at the crystal.
    fn dma_chip(buffer: u32) -> Esp32s3 {
        let mut program = Program::new(&[]);
        program.raw(&LOOP);
        let mut chip = program.chip_on(&[]);
        let soc = &mut chip.soc;
        soc.poke(buffer, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let head: u32 = 1 << 31 | 1 << 30 | 8 << 12 | 8;
        let descriptor: Vec<u8> = [head, buffer, 0]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        soc.poke(DESCRIPTOR, &descriptor);
        for (cycle, address, value) in [
            (1, SYSTEM + 0x1c, 0x640),
            (2, SYSTEM + 0x24, 0x1be),
            (3, GDMA + 0xa8, 0),
            (4, GDMA + 0x80, DESCRIPTOR & 0xf_ffff | 1 << 21),
            (5, SPI2 + 0x10, 1 << 27),
            (6, SPI2 + 0x1c, 63),
            (7, SPI2 + 0x30, 1 << 28),
            (8, SPI2 + 0xe8, 0x3),
            (9, SPI2, 1 << 23),
        ] {
            assert_eq!(soc.store(Cycle(cycle), address, Width::Word, value), Ok(()));
        }
        chip
    }

    /// `dma_chip` with its transfer started at cycle 10, which runs for 64 crystal cycles.
    fn sending(buffer: u32) -> Esp32s3 {
        let mut chip = dma_chip(buffer);
        assert_eq!(
            chip.soc.store(Cycle(10), SPI2, Width::Word, 1 << 24),
            Ok(())
        );
        assert!(chip.soc.spi2.busy(c(73)) && !chip.soc.spi2.busy(c(74)));
        chip
    }

    /// While a transfer sends from GDMA, a store into a byte of its descriptors or buffers ends
    /// the run, before it lands. A store beside them, or after the transfer, lands.
    #[test]
    fn a_store_into_a_list_a_running_dma_transfer_sends_from_ends_the_run() {
        let refused = Some(Ending::Refused(Refused::Register("GDMA", "buffer_write")));
        for (address, width) in [
            (BUFFER + 4, Width::Word),
            (BUFFER + 7, Width::Byte),
            (BUFFER - 1, Width::Half),
            (DESCRIPTOR + 8, Width::Word),
        ] {
            let mut chip = sending(BUFFER);
            assert_eq!(
                chip.soc.store(Cycle(11), address, width, 0),
                Err(Exit::Stop),
                "{address:#x}"
            );
            assert_eq!(chip.soc.ending, refused, "{address:#x}");
            assert_eq!(
                chip.soc.load(Cycle(12), BUFFER + 4, Width::Word),
                Ok(0x0807_0605)
            );
        }
        let mut chip = sending(BUFFER);
        for address in [BUFFER + 8, BUFFER - 4, DESCRIPTOR + 12] {
            assert_eq!(chip.soc.store(Cycle(11), address, Width::Word, 0), Ok(()));
        }
        assert_eq!(chip.soc.store(Cycle(74), BUFFER, Width::Word, 0), Ok(()));
        assert_eq!(chip.soc.ending, None);
    }

    /// While a transfer sends from GDMA, a store into a byte that shares a word with its buffer
    /// lands, and a store into the buffer's own bytes ends the run.
    #[test]
    fn a_store_beside_a_buffer_in_a_word_they_share_lands() {
        for (address, width) in [
            (BUFFER, Width::Byte),
            (BUFFER + 9, Width::Byte),
            (BUFFER + 10, Width::Half),
        ] {
            let mut chip = sending(BUFFER + 1);
            assert_eq!(
                chip.soc.store(Cycle(11), address, width, 0xffff),
                Ok(()),
                "{address:#x}"
            );
            assert_eq!(chip.soc.ending, None, "{address:#x}");
        }
        for address in [BUFFER + 1, BUFFER + 8] {
            let mut chip = sending(BUFFER + 1);
            assert_eq!(
                chip.soc.store(Cycle(11), address, Width::Byte, 0xff),
                Err(Exit::Stop),
                "{address:#x}"
            );
            let refused = Refused::Register("GDMA", "buffer_write");
            assert_eq!(chip.soc.ending, Some(Ending::Refused(refused)));
        }
    }

    /// IDF's `esp_system_reset_modules_on_exit`, which its restart and panic paths run, sets
    /// these bits of `SYSTEM_PERIP_RST_EN1_REG`: `SYSTEM_DMA_RST` and the crypto and SDIO host
    /// resets. While a transfer sends from GDMA that store ends the run with GDMA's
    /// `reset_while_sending`, before it lands. Once the transfer has ended it lands.
    #[test]
    fn a_restart_while_a_transfer_sends_from_gdma_ends_the_run() {
        const RESTART_RST_EN1: u32 = 1 << 7 | 1 << 6 | 1 << 5 | 1 << 4 | 1 << 3 | 1 << 2 | 1 << 1;
        let mut chip = sending(BUFFER);
        let before = chip
            .soc
            .load(Cycle(11), SYSTEM + 0x24, Width::Word)
            .unwrap();
        assert_eq!(
            chip.soc.store(
                Cycle(11),
                SYSTEM + 0x24,
                Width::Word,
                before | RESTART_RST_EN1
            ),
            Err(Exit::Stop)
        );
        let refused = Refused::Register("GDMA", "reset_while_sending");
        assert_eq!(chip.soc.ending, Some(Ending::Refused(refused)));
        assert_eq!(
            chip.soc.load(Cycle(12), SYSTEM + 0x24, Width::Word),
            Ok(before)
        );
        let mut chip = sending(BUFFER);
        assert_eq!(
            chip.soc.store(
                Cycle(74),
                SYSTEM + 0x24,
                Width::Word,
                before | RESTART_RST_EN1
            ),
            Ok(())
        );
        assert_eq!(chip.soc.ending, None);
        assert_eq!(
            chip.soc.load(Cycle(75), SYSTEM + 0x24, Width::Word),
            Ok(before | RESTART_RST_EN1)
        );
    }

    /// While a transfer sends from GDMA, a GDMA reset or a turn of its clock off ends the run,
    /// as SPI2's own do. Once the transfer has ended, each lands. A transfer started while GDMA's
    /// clock is off has no channel to feed it.
    #[test]
    fn a_gate_change_that_stops_a_running_dma_transfer_ends_the_run() {
        for (address, value, block, setting) in [
            (SYSTEM + 0x24, 0x1fe, "GDMA", "reset_while_sending"),
            (SYSTEM + 0x1c, 0x600, "GDMA", "clock_off_while_sending"),
            (
                SYSTEM + 0x18,
                0xf9c1_e02f,
                "SPI2",
                "clock_off_while_sending",
            ),
            (SYSTEM + 0x20, 1 << 6, "SPI2", "reset_while_sending"),
        ] {
            let mut chip = sending(BUFFER);
            assert_eq!(
                chip.soc.store(Cycle(11), address, Width::Word, value),
                Err(Exit::Stop),
                "{setting}"
            );
            let refused = Refused::Register(block, setting);
            assert_eq!(chip.soc.ending, Some(Ending::Refused(refused)));
            let mut chip = sending(BUFFER);
            assert_eq!(
                chip.soc.store(Cycle(74), address, Width::Word, value),
                Ok(()),
                "{setting} after the transfer"
            );
        }
        let mut chip = dma_chip(BUFFER);
        assert_eq!(
            chip.soc.store(Cycle(10), SYSTEM + 0x1c, Width::Word, 0x600),
            Ok(())
        );
        assert_eq!(
            chip.soc.store(Cycle(11), SPI2, Width::Word, 1 << 24),
            Err(Exit::Stop)
        );
        assert_eq!(chip.soc.ending, Some(Ending::Setting(Setting::DmaLink)));
    }

    /// A store that starts a GDMA link the model does not feed ends the run before it lands.
    #[test]
    fn a_gdma_link_the_model_does_not_feed_ends_the_run() {
        for (address, value, setting) in [
            (
                GDMA + 0x20,
                DESCRIPTOR & 0xf_ffff | 1 << 22,
                "link_peripheral",
            ),
            (GDMA + 0xa8, 7, "peri_sel_while_started"),
        ] {
            let mut chip = dma_chip(BUFFER);
            assert_eq!(
                chip.soc.store(Cycle(10), address, Width::Word, value),
                Err(Exit::Stop)
            );
            let refused = Refused::Register("GDMA", setting);
            assert_eq!(chip.soc.ending, Some(Ending::Refused(refused)));
        }
    }

    /// A change is listed once, when its store lands: a change that another reported store
    /// cuts in on, as an interrupt can between the stop and the store run again, is listed
    /// after that one, and a change whose store never runs again is not listed.
    #[test]
    fn a_change_is_listed_once_when_its_store_lands() {
        let (rtc, spi0) = (RTC_CNTL + 0x74, 0x6000_3018);
        let mut chip = Program::new(&[]).chip();
        chip.soc.restore(rtc, 0);
        chip.soc.restore(spi0, 0);
        let soc = &mut chip.soc;
        assert_eq!(soc.store(Cycle(1), rtc, Width::Word, 1), Err(Exit::Stop));
        assert_eq!(soc.store(Cycle(2), spi0, Width::Word, 1), Err(Exit::Stop));
        assert_eq!(soc.store(Cycle(2), spi0, Width::Word, 1), Ok(()));
        assert_eq!(soc.store(Cycle(3), rtc, Width::Word, 1), Err(Exit::Stop));
        assert_eq!(soc.store(Cycle(3), rtc, Width::Word, 1), Ok(()));
        assert_eq!(soc.store(Cycle(4), spi0, Width::Word, 2), Err(Exit::Stop));
        let listed: Vec<(u32, Tick)> = soc.changes.iter().map(|&(a, t)| (a, t.at)).collect();
        assert_eq!(listed, [(spi0, c(2)), (rtc, c(3))]);
    }

    /// While `SPI_TRANS_DONE` is enabled, a running transfer's end is the core's next event, so
    /// its interrupt rises on that tick with no access.
    #[test]
    fn the_end_of_a_transfer_whose_interrupt_is_enabled_is_the_next_event() {
        let program = transfer(1 << 31, Some((SPI2 + 0x34, spi2::TRANS_DONE)));
        let mut chip = program.chip_on(&[]);
        assert_eq!(chip.run_until(c(18)), None);
        let end = chip.soc.spi2.end(0).expect("a transfer runs");
        assert!(c(18) < end);
        assert_eq!(
            chip.soc.next_event(Cycle(18)),
            Some(chip.soc.clock.cycle_after(end))
        );
    }

    /// A reset of SPI2 puts its pads back to their idle levels, so a store that resets it and
    /// moves a pad ends the step there.
    #[test]
    fn a_reset_of_spi2_that_moves_a_pad_ends_the_step_at_its_store() {
        let mut program = Program::new(&[
            IO_MUX,
            1 << 12,
            GPIO + 0x500,
            110,
            SPI2,
            1 << 7,
            SYSTEM + 0x20,
            1 << 6,
        ]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0x34)
            .l32r(2, 2)
            .l32r(3, 3)
            .rri8(6, 3, 2, 0x84)
            .l32r(2, 4)
            .l32r(3, 5)
            .rri8(6, 3, 2, 0x20)
            .l32r(2, 6)
            .l32r(3, 7)
            .rri8(6, 3, 2, 0)
            .raw(&LOOP);
        let mut chip = program.chip_on(&[12]);
        let mut drives = Vec::new();
        while let Some(halt) = chip.run_until(c(1000)) {
            assert!(matches!(halt, Halt::Change { image: true, .. }), "{halt:?}");
            drives.push((halt.at(), chip.drive(12)));
        }
        assert_eq!(
            drives,
            [
                (c(5), PadDrive::High),
                (c(8), PadDrive::Low),
                (c(11), PadDrive::High)
            ],
            "routed, MASTER_CS0_POL, then the reset"
        );
    }

    #[test]
    fn a_fetch_outside_the_instruction_window_ends_the_run() {
        let mut chip = Esp32s3::new(&[], 0x4000_0400, []);
        assert_eq!(
            chip.run_until(c(10)),
            Some(Halt::Fault(c(0), Fault::UnmappedFetch(0x4000_0400)))
        );
        let data = RAM[1].start;
        let mut chip = Esp32s3::new(&[], data, [(data, [0u8; 4].as_slice())]);
        assert_eq!(
            chip.run_until(c(10)),
            Some(Halt::Fault(c(0), Fault::UnmappedFetch(data))),
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
            chip.run_until(c(100));
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
            soc.load(Cycle(0), 0x5100_0000, Width::Word),
            Err(Exit::Fault(Cause::LoadStoreError))
        );
    }
}
