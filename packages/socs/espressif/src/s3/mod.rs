//! The ESP32-S3 as one Xtensa LX7 core on its internal SRAM and RTC memory, with GPIO, IO_MUX
//! and the GPIO matrix, the interrupt matrix, SYSTIMER, the watchdogs, the USB Serial/JTAG
//! console, UART0, UART1, UART2, SPI2, RMT, and for a chip booted from flash the mask ROM, the
//! flash behind SPI1 and the MMU, and the registers the boot path reads.
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
mod app;
mod blocks;
mod boot;
mod bridge;
mod calibration;
mod clock;
mod cores;
mod efuse;
mod extmem;
mod flash;
mod gates;
mod gdma;
mod gpio;
mod i2c;
mod ledc;
mod matrix;
mod rmt;
mod rom;
mod sha;
mod soc;
mod spi1;
mod spi2;
mod timers;
mod uart;
mod usb;

use std::sync::Arc;

use orbe_xtensa::{Core, Stop, Unimplemented};

use clock::{CpuClock, Source, XTAL};
use cores::Cores;
use flash::Flash;
use gpio::PADS;
use soc::{Ending, Soc};
use timers::{Action, Watchdogs};

pub use boot::Segment;
pub use bridge::{Bridge, PINS};
pub use clock::{TICK_HZ, Tick};
pub use gpio::PadDrive;
pub use orbe_xtensa::CoreId;
pub use rmt::{Flight, Train};
pub use spi1::FlashRefusal;
pub use spi2::{Pads, Phase, Setting, Transfer};
pub use timers::{Reset, Watchdog};

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

/// A PC and the core whose PC it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pc {
    pub core: CoreId,
    pub pc: u32,
}

/// Why a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The next instruction of this core needs behavior the core does not model.
    Unsupported(CoreId, Unimplemented),
    /// A core fetched outside the instruction window, at this PC.
    UnmappedFetch(Pc),
    /// A core reached a jump to itself at this PC that nothing can take it out of.
    Halted(Pc),
    /// A watchdog's stage ran out and reset the CPU, the system or the RTC domain.
    Watchdog {
        watchdog: Watchdog,
        stage: u8,
        reset: Reset,
    },
    /// The store at this PC started an SPI2 transfer with a setting the model does not run.
    Setting(Setting, Pc),
    /// The store at this PC started an eFuse program command, which would burn eFuses.
    EfuseProgram(Pc),
    /// The store at this PC sent the flash a command the model refuses.
    FlashCommand(FlashRefusal, Pc),
    /// The store at this PC picked a CPU clock the model does not run: the RC fast clock, or a
    /// reserved setting.
    Clock(Pc),
    /// The store at this PC reset the chip for this cause.
    FaultRestart(ResetCause, Pc),
    /// The store at this PC set something the model does not run.
    Refused(Refused, Pc),
    /// The load at `pc` read `address` in a cache window, where the MMU maps no flash.
    UnmappedLoad { address: u32, pc: Pc },
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
    /// At `at` a pad's drive changed, a record ended, or both: an SPI2 transfer, an LEDC
    /// update that took effect on a pad with a net, or an RMT transmission. [`Esp32s3::drive`]
    /// gives the new drives, and [`Esp32s3::take_transfers`], [`Esp32s3::take_duties`] and
    /// [`Esp32s3::take_trains`] the records.
    Change { at: Tick, image: bool, record: bool },
    /// The LEDC updates that took effect at `at` moved a pad's drive, after the `Change` at
    /// `at` that holds their records. [`Esp32s3::drive`] gives the new drives.
    Moved(Tick),
    /// The run ended at this tick. The chip runs no further.
    Fault(Tick, Fault),
}

impl Halt {
    pub fn at(self) -> Tick {
        match self {
            Halt::Change { at, .. } | Halt::Moved(at) | Halt::Fault(at, _) => at,
        }
    }
}

/// An LEDC update that took effect at `at`, or a route that put a netted pad on a channel at
/// `at`: `pad` sends `duty` of `2^resolution` steps at `hz`. Every netted pad that carries the
/// channel gets its own record, and an inverted pad's duty is the steps the channel does not
/// send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duty {
    pub at: Tick,
    pub pad: u8,
    pub duty: u32,
    pub resolution: u8,
    pub hz: u32,
}

/// The first access to a peripheral register the model does not keep, or the first write that
/// changes a register whose effect it does not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub at: Tick,
    pub pc: Pc,
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
    cores: Cores,
    soc: Soc,
    /// The drive of each netted pad, as the last reported image left it.
    shown: [PadDrive; PADS],
    /// The tick of an image change not reported yet.
    changed: Option<Tick>,
    /// How many of SPI2's transfers have had their end reported and wait to be taken.
    reported: usize,
    /// The `Duty` records, from the first not taken.
    duties: Vec<Duty>,
    /// How many of `duties` have been reported and wait to be taken.
    reported_duties: usize,
    /// The RMT transmissions that ended, one per netted pad, from the first not taken.
    trains: Vec<Train>,
    /// How many of `trains` have been reported and wait to be taken.
    reported_trains: usize,
    /// The LEDC signal changes that move a pad's drive and are not reported yet: each one's tick,
    /// signal and level.
    moves: Vec<(Tick, u16, bool)>,
    /// The fault that ends the run, not reported yet.
    fault: Option<(Tick, Fault)>,
    ended: bool,
    /// The tick and PC of the store that reset the chip, which holds it in reset from then on.
    restarted: Option<(Tick, Pc)>,
    /// The PC after the store that started the last I2C transfer, which a setting the transfer
    /// meets later names.
    i2c_pc: Pc,
    /// The PC after the store that started the last RMT transmission, which a transmission
    /// that never ends names.
    rmt_pc: Pc,
    /// The PC after the last UART store, which a frame that meets a setting names.
    uart_pc: Pc,
}

impl Esp32s3 {
    /// The chip with `segments`, each an address and its bytes, loaded into SRAM and the PC at
    /// `entry`. `netted` names the GPIOs with nets. Every segment lies inside one [`RAM`] window.
    /// With `rom`, the ROM ELF, the chip has the mask ROM and the ROM's data in SRAM under the
    /// segments, so the APP CPU can run the ROM's reset path.
    pub fn new<'a>(
        netted: &[u8],
        entry: u32,
        segments: impl IntoIterator<Item = (u32, &'a [u8])>,
        rom: Option<&[u8]>,
    ) -> Esp32s3 {
        let mut soc = Soc::new(CpuClock::new(Source::Xtal(1)), netted, Watchdogs::new(XTAL));
        soc.keep_crystal_clock();
        if let Some(rom) = rom {
            load_rom(&mut soc, rom);
        }
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
        let state: Vec<boot::Value> = boot::state().collect();
        let source = boot::clock_source(&state).expect("the captured clock is one the model runs");
        let mut soc = Soc::new(CpuClock::new(source), netted, Watchdogs::new(source.apb()));
        load_rom(&mut soc, boot.rom);
        soc.flash = Some(Flash::new(boot.flash));
        let mut core = Core::new();
        boot::apply(&state, &mut core, &mut soc);
        boot::program_rom_uart(&state, &mut soc);
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

    /// The part names no pull-down resistor. A store that sets `FUN_WPD` on a pad with a net
    /// ends the run and writes nothing. A pad with no net keeps the bit.
    pub fn omit_pull_down(&mut self) {
        self.soc.gpio.omit_pull_down();
    }

    fn with(core: Core<orbe_xtensa::Esp32s3>, soc: Soc) -> Esp32s3 {
        Esp32s3 {
            cores: Cores::new(core),
            shown: soc.image(),
            soc,
            changed: None,
            reported: 0,
            duties: Vec::new(),
            reported_duties: 0,
            trains: Vec::new(),
            reported_trains: 0,
            moves: Vec::new(),
            fault: None,
            ended: false,
            restarted: None,
            i2c_pc: Pc {
                core: CoreId::Pro,
                pc: 0,
            },
            rmt_pc: Pc {
                core: CoreId::Pro,
                pc: 0,
            },
            uart_pc: Pc {
                core: CoreId::Pro,
                pc: 0,
            },
        }
    }

    /// Runs through tick `limit`, and returns the first image change, transfer end or fault at
    /// or before it. `None` when the chip reached the limit, or ended before.
    pub fn run_until(&mut self, limit: Tick) -> Option<Halt> {
        loop {
            self.settle_ledc();
            self.settle_i2c(limit);
            self.settle_rmt();
            self.settle_uart();
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
            let watchdog = self.soc.watchdogs.deadline();
            let due = watchdog.map(|expiry| self.soc.clock.round_up(expiry.at));
            let ledc = self.soc.ledc.next(self.soc.clock.source.apb());
            let i2c = self.soc.i2c.next_event();
            let rmt = self.soc.rmt.upcoming().map(|at| Tick(at.0 + 1));
            let uart = self
                .soc
                .uart
                .next_event(self.now())
                .map(|at| Tick(at.0 + 1));
            let train = self
                .trains
                .get(self.reported_trains)
                .map(|train| Tick(train.end.0 + 1));
            let until = [
                Some(Tick(limit.0 + 1)),
                due,
                self.soc.spi2.end(self.reported),
                ledc.map(|at| Tick(at.0 + 1)),
                i2c.map(|at| Tick(at.0 + 1)),
                rmt,
                uart,
                train,
            ]
            .into_iter()
            .flatten()
            .min()
            .expect("the limit");
            let until = self.soc.clock.cycle_after(until);
            let (id, stop) = self.cores.run_until(&mut self.soc, until);
            if let Some(change) = self.soc.changed.take() {
                debug_assert!(self.changed.is_none(), "a change stops the core");
                self.changed = Some(change);
            }
            self.soc.rearmed = false;
            self.soc.reclocked = false;
            self.soc.started = false;
            if let Some(core) = self.soc.i2c_started.take() {
                self.i2c_pc = Pc {
                    core,
                    pc: self.cores.core(core).pc(),
                };
            }
            if let Some(core) = self.soc.rmt_started.take() {
                self.rmt_pc = Pc {
                    core,
                    pc: self.cores.core(core).pc(),
                };
            }
            if let Some(core) = self.soc.uart_started.take() {
                self.uart_pc = Pc {
                    core,
                    pc: self.cores.core(core).pc(),
                };
            }
            if let (Some(expiry), Some(due)) = (watchdog, due)
                && self.now() >= due
                && self.soc.watchdogs.deadline() == watchdog
            {
                if let Action::Reset(reset) = expiry.action {
                    let fault = Fault::Watchdog {
                        watchdog: expiry.watchdog,
                        stage: expiry.stage,
                        reset,
                    };
                    self.fault = Some((due, fault));
                    continue;
                }
                self.soc.watchdogs.run_out(expiry);
            }
            let core = self.cores.core(id);
            let pc = Pc {
                core: id,
                pc: core.pc(),
            };
            let now = self.soc.clock.tick(core.cycle());
            match stop {
                Stop::Limit | Stop::Yield => {}
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
                    let nmi = self.soc.routes_to_nmi(id);
                    if !nmi && self.soc.watchdogs.deadline().is_none() {
                        self.fault = Some((now, Fault::Halted(pc)));
                    }
                }
                Stop::Unimplemented(missing) => {
                    self.fault = Some((now, Fault::Unsupported(id, missing)));
                }
                Stop::Watchpoint { .. } => {
                    let refused = Refused::Register("CPU", "watchpoint");
                    self.fault = Some((now, Fault::Refused(refused, pc)));
                }
            }
        }
    }

    /// Lets every LEDC update take effect that is due at a tick whose instructions have all run,
    /// before any fault. An update that moves a pad's drive waits for its `Moved` halt to change
    /// the signal, and the others change it at once.
    fn settle_ledc(&mut self) {
        let Some(through) = self.now().0.checked_sub(1) else {
            return;
        };
        self.soc.settle_ledc(Tick(through));
        let fault = self.fault.map(|(at, _)| at);
        let before_fault = |at: Tick| fault.is_none_or(|fault| at < fault);
        let duties = self
            .soc
            .duties
            .drain(..)
            .filter(|duty| before_fault(duty.at));
        self.duties.extend(duties);
        for effect in std::mem::take(&mut self.soc.effects) {
            if !before_fault(effect.at) {
                continue;
            }
            let signal = ledc::SIGNAL + u16::from(effect.channel);
            let level = effect.duty > 0;
            if self.soc.gpio.moves(signal, level) {
                self.moves.push((effect.at, signal, level));
            } else {
                self.soc.gpio.set_signal(signal, level);
            }
        }
    }

    /// Runs every I2C event due through `limit` at a tick whose instructions have all run, and
    /// before any halt the chip has not reported, so an event never runs before the inputs of an
    /// earlier landing. A setting a transfer meets ends the run, naming the PC after the store
    /// that started it.
    fn settle_i2c(&mut self, limit: Tick) {
        if self.soc.i2c.idle() {
            return;
        }
        let Some(through) = self.now().0.checked_sub(1) else {
            return;
        };
        let halt = self.next_halt().map(|halt| match halt {
            Halt::Fault(at, _) => at.0.saturating_sub(1),
            halt => halt.at().0,
        });
        let through = [Some(through), Some(limit.0), halt]
            .into_iter()
            .flatten()
            .min();
        self.soc.settle_i2c(Tick(through.expect("a bound")));
        if let Some((at, setting)) = self.soc.i2c.take_refused()
            && self.fault.is_none_or(|(fault, _)| at < fault)
        {
            let refused = Refused::Register("I2C0", setting);
            self.fault = Some((at, Fault::Refused(refused, self.i2c_pc)));
        }
    }

    /// Lets every RMT entry take effect that is due at a tick whose instructions have all run,
    /// and takes the transmissions that ended. A transmission that passes 65,536 entries with
    /// no end word ends the run, naming the PC after the store that started it.
    fn settle_rmt(&mut self) {
        if self.soc.rmt.quiet() {
            return;
        }
        let Some(through) = self.now().0.checked_sub(1) else {
            return;
        };
        self.soc.rmt.settle(Tick(through));
        if let Some((at, setting)) = self.soc.rmt.take_refused()
            && self.fault.is_none_or(|(fault, _)| at < fault)
        {
            let refused = Refused::Register("RMT", setting);
            self.fault = Some((at, Fault::Refused(refused, self.rmt_pc)));
        }
        let fault = self.fault.map(|(at, _)| at);
        let trains = self
            .soc
            .rmt
            .drain()
            .into_iter()
            .filter(|train| fault.is_none_or(|fault| train.end < fault));
        self.trains.extend(trains);
    }

    /// Drives each UART TXD edge due at a tick whose instructions have all run. An edge that
    /// moves a netted pad waits for its `Moved` halt, and the others change the pad at once.
    /// A frame that ends here queues the next byte's start bit, and that edge is driven before
    /// the cores run on. A frame that meets a divider or a clock the model does not shift ends
    /// the run, naming the PC after the last UART store.
    fn settle_uart(&mut self) {
        let Some(through) = self.now().0.checked_sub(1) else {
            return;
        };
        let mut bound = Tick(through);
        let apb = self.soc.clock.source.apb();
        loop {
            let mut moved = false;
            while let Some(edge) = self.soc.uart.next_edge(bound) {
                let signal = uart::SIGNAL[usize::from(edge.port)];
                let moves = self.soc.gpio.moves(signal, edge.level);
                self.soc.uart.take_edge(usize::from(edge.port));
                if moves {
                    self.moves.push((edge.at, signal, edge.level));
                    self.soc
                        .uart
                        .after_edge(usize::from(edge.port), edge.at, &self.soc.gpio, apb);
                    bound = edge.at;
                    moved = true;
                    break;
                }
                self.soc.gpio.set_signal(signal, edge.level);
                self.soc
                    .uart
                    .after_edge(usize::from(edge.port), edge.at, &self.soc.gpio, apb);
            }
            self.soc.uart.settle(bound, &self.soc.gpio, apb);
            if moved || self.soc.uart.next_edge(bound).is_none() {
                break;
            }
        }
        if let Some((at, name, setting)) = self.soc.uart.take_refused()
            && self.fault.is_none_or(|(fault, _)| at < fault)
        {
            let refused = Refused::Register(name, setting);
            self.fault = Some((at, Fault::Refused(refused, self.uart_pc)));
        }
    }

    /// Holds the chip in reset from `now`, after the store at `pc` reset it from `esp_restart`.
    /// The boot ROM does not run, so the core runs no further, and each pad drives as at
    /// power-on.
    fn hold_in_reset(&mut self, now: Tick, pc: Pc) {
        self.restarted = Some((now, pc));
        self.soc.reset_pads();
        if self.soc.image() != self.shown {
            self.changed = Some(now);
        }
    }

    /// The tick and PC of the store that reset the chip, if one did.
    pub fn restart(&self) -> Option<(Tick, Pc)> {
        self.restarted
    }

    /// The tick through which both cores have run everything.
    fn now(&self) -> Tick {
        self.soc.clock.tick(self.cores.reached())
    }

    /// The earliest halt the core has reached and not reported. An image change and the records
    /// that end at one cycle are one halt, then the drive moves of that cycle's LEDC updates,
    /// and either comes before a fault at that cycle.
    fn next_halt(&self) -> Option<Halt> {
        if self.ended {
            return None;
        }
        let transfer = self
            .soc
            .spi2
            .end(self.reported)
            .filter(|&end| end <= self.now());
        let duty = self.duties.get(self.reported_duties).map(|duty| duty.at);
        let train = self
            .trains
            .get(self.reported_trains)
            .map(|train| train.end)
            .filter(|&end| end <= self.now());
        let record = [transfer, duty, train].into_iter().flatten().min();
        let change = [self.changed, record].into_iter().flatten().min();
        let change = change.map(|at| {
            let halt = Halt::Change {
                at,
                image: self.changed == Some(at),
                record: record == Some(at),
            };
            (at, 0, halt)
        });
        let moved = [
            self.moves.first().map(|&(at, ..)| at),
            self.soc.i2c.moved().map(|(at, ..)| at),
        ]
        .into_iter()
        .flatten()
        .min()
        .map(|at| (at, 1, Halt::Moved(at)));
        let fault = self
            .fault
            .map(|(at, fault)| (at, 2, Halt::Fault(at, fault)));
        [change, moved, fault]
            .into_iter()
            .flatten()
            .min_by_key(|&(at, rank, _)| (at, rank))
            .map(|(.., halt)| halt)
    }

    fn report(&mut self, halt: Halt) {
        match halt {
            Halt::Change { at, image, record } => {
                if image {
                    self.shown = self.soc.image();
                    self.changed = None;
                }
                if record {
                    if self.soc.spi2.end(self.reported) == Some(at) {
                        self.reported += 1;
                    }
                    let unreported = &self.duties[self.reported_duties..];
                    self.reported_duties += unreported.iter().take_while(|d| d.at == at).count();
                    let unreported = &self.trains[self.reported_trains..];
                    self.reported_trains += unreported.iter().take_while(|t| t.end == at).count();
                }
            }
            Halt::Moved(at) => {
                while let Some(&(_, signal, level)) = self.moves.first().filter(|m| m.0 == at) {
                    self.soc.gpio.set_signal(signal, level);
                    self.moves.remove(0);
                }
                if let Some((signal, level)) = self.soc.i2c.land(at) {
                    self.soc.gpio.set_signal(signal, level);
                }
                self.shown = self.soc.image();
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

    /// The `Duty` records whose ends were reported, in time order.
    pub fn take_duties(&mut self) -> impl Iterator<Item = Duty> + '_ {
        let n = std::mem::take(&mut self.reported_duties);
        self.duties.drain(..n)
    }

    /// The RMT transmissions whose ends were reported, in the order they ended, one per pad.
    pub fn take_trains(&mut self) -> impl Iterator<Item = Train> + '_ {
        let n = std::mem::take(&mut self.reported_trains);
        self.trains.drain(..n)
    }

    /// A transmission that started by `at` and ends after it. The chip has already run past `at`.
    pub fn transmission_open(&self, at: Tick) -> bool {
        self.soc.rmt.covers(at)
            || self
                .trains
                .iter()
                .any(|train| train.start <= at && train.end > at)
    }

    /// The part of channel 0's transmission that had started by `at`. The chip is cloned and run
    /// to `at`, which meets no halt at or before `at`.
    pub fn in_flight(&self, at: Tick) -> Option<Flight> {
        let mut copy = self.clone();
        assert!(
            copy.run_until(at).is_none(),
            "a cut meets no halt at or before {at:?}"
        );
        copy.soc.rmt.settle(at);
        copy.soc.rmt.flight(at)
    }

    /// The word at `addr`, as the core would load it at the cycle it has reached.
    #[cfg(test)]
    pub(crate) fn load_word(&mut self, addr: u32) -> Result<u32, orbe_xtensa::Exit> {
        let cycle = self.cores.reached();
        self.soc.load_word(cycle, addr)
    }

    /// What the pad of `gpio`, a netted one, drives in the last reported image.
    pub fn drive(&self, gpio: u8) -> PadDrive {
        self.shown[usize::from(gpio)]
    }

    /// The levels of netted pads from tick `at` on, each a GPIO and its level. The chip has run
    /// through `at`. A core in `WAITI` polls on the first cycle after `at`, so a pad edge reaches
    /// it at the same cycle however the run splits.
    pub fn input(&mut self, at: Tick, pads: impl IntoIterator<Item = (u8, bool)>) {
        // Each return from a run unparks both cores. A change that keeps one parked must rouse
        // it here.
        debug_assert!(!self.cores.parked(), "an input at {at:?} met a parked core");
        for (gpio, level) in pads {
            self.soc.gpio.set_pad(gpio, level);
        }
        self.soc.i2c.input(at, &self.soc.gpio);
        let apb = self.soc.clock.source.apb();
        self.soc.uart.note_pins(at, &self.soc.gpio, apb);
    }

    /// The next console byte whose packet went at or before `through`, with that tick.
    pub fn take_sent(&mut self, through: Tick) -> Option<(Tick, u8)> {
        self.soc.usb.take_sent(through)
    }

    /// The next UART byte, sent or received, whose stop bit ended at or before `through`.
    pub(crate) fn take_uart(&mut self, through: Tick) -> Option<uart::Sent> {
        self.soc.uart.take_sent(through)
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

    /// The tick through which both cores have run everything.
    pub fn tick(&self) -> Tick {
        self.now()
    }
}

/// Adds the mask ROM from the ROM ELF `rom`, and puts its data in SRAM.
fn load_rom(soc: &mut Soc, rom: &[u8]) {
    let image = rom::load(rom, |addr, len| {
        Soc::sram_offset(addr, false).is_some()
            && Soc::sram_offset(addr + len as u32 - 1, false).is_some()
    })
    .expect("the pinned ROM ELF");
    soc.attach_rom(image.rom);
    for (addr, bytes) in &image.data {
        soc.poke(*addr, bytes);
    }
}

#[cfg(test)]
mod tests {
    use orbe_xtensa::{Bus, Cause, Cycle, Exit, Sr, Width};

    use super::soc::{
        EFUSE, Ending, GDMA, GPIO, IO_MUX, RTC_CNTL, SPI2, SYSTEM, SYSTIMER, TIMG0, TIMG1,
        USB_SERIAL_JTAG,
    };
    use super::*;

    /// The tick of crystal cycle `n`, a CPU cycle at power-on.
    fn c(n: u64) -> Tick {
        Tick(n * XTAL)
    }

    /// `pc` on the PRO CPU.
    fn pro(pc: u32) -> Pc {
        Pc {
            core: CoreId::Pro,
            pc,
        }
    }

    const BASE: u32 = 0x4037_0000;
    const GPIO21: u32 = 1 << 21;

    /// A program: literal words from `base`, then the code after them.
    struct Program {
        base: u32,
        literals: Vec<u32>,
        code: Vec<u8>,
    }

    impl Program {
        fn new(literals: &[u32]) -> Program {
            Program::at(BASE, literals)
        }

        fn at(base: u32, literals: &[u32]) -> Program {
            Program {
                base,
                literals: literals.to_vec(),
                code: Vec::new(),
            }
        }

        fn entry(&self) -> u32 {
            self.base + 4 * self.literals.len() as u32
        }

        fn pc(&self) -> u32 {
            self.entry() + self.code.len() as u32
        }

        /// The literals and the code, from `base`.
        fn image(&self) -> Vec<u8> {
            let mut image: Vec<u8> = self.literals.iter().flat_map(|w| w.to_le_bytes()).collect();
            image.extend(&self.code);
            image
        }

        fn l32r(&mut self, t: u8, literal: usize) -> &mut Program {
            let target = self.base + 4 * literal as u32;
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

        fn wsr(&mut self, t: u8, sr: u8) -> &mut Program {
            self.raw(&[t << 4, sr, 0x13])
        }

        fn chip(&self) -> Esp32s3 {
            self.chip_on(&[21])
        }

        fn chip_on(&self, netted: &[u8]) -> Esp32s3 {
            Esp32s3::new(
                netted,
                self.entry(),
                [(BASE, self.image().as_slice())],
                None,
            )
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
    const PS: u8 = 230;
    const INTENABLE: u8 = 228;
    const WAITI: [u8; 3] = [0x00, 0x70, 0x00];
    /// Timer group 0's watchdog in flash boot mode, as at power-on, at its stage 0.
    const TIMG0_FLASHBOOT: Fault = Fault::Watchdog {
        watchdog: Watchdog::Timg0,
        stage: 0,
        reset: Reset::System,
    };

    /// Enables GPIO21, raises it, reads `GPIO_PCPU_NMI_INT`, sends `hi` and spins.
    fn blink() -> Program {
        let mut program = Program::new(&[GPIO, GPIO21, USB_SERIAL_JTAG]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .l32r(5, 2)
            .rri8(6, 3, 2, 0x24)
            .rri8(6, 3, 2, 0x08)
            .rri8(2, 4, 2, 0x60)
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

    /// A transmission already listed still waits until both cores have reached its end.
    /// Reporting it earlier lets a halt from the core still behind land before it.
    #[test]
    fn a_train_the_cores_have_not_reached_is_not_a_halt_yet() {
        let mut program = Program::new(&[]);
        program.raw(&LOOP);
        let mut chip = program.chip();
        let end = c(10);
        chip.trains.push(Train {
            start: c(1),
            end,
            idle: false,
            pad: 21,
            pulses: vec![(true, 25_000)],
        });
        let halt = chip.run_until(c(40)).expect("the train's end");
        assert!(
            matches!(halt, Halt::Change { record: true, .. }) && halt.at() == end,
            "{halt:?}"
        );
        assert!(
            chip.tick() >= end,
            "the record was reported at {:?}, before the cores reached {end:?}",
            chip.tick()
        );
    }

    mod dual;
    mod i2c;
    mod pads;
    mod rmt;

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

    /// Routes GPIO21 to UART0, sends 0x55 at 1 Mbaud from the APB clock, and checks the edges
    /// and the logged byte against a run whose limits fall inside the byte.
    fn uart_byte() -> Program {
        let mut program = Program::new(&[
            IO_MUX + 0x58,
            1 << 12,
            GPIO + 0x5a8,
            12,
            0x6000_0078,
            (1 << 20) | (1 << 22) | (1 << 24) | (1 << 25),
            0x6000_0014,
            80,
            0x6000_0048,
            256,
            0x6000_0000,
            0x55,
        ]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0)
            .l32r(2, 2)
            .l32r(3, 3)
            .rri8(6, 3, 2, 0)
            .l32r(2, 4)
            .l32r(3, 5)
            .rri8(6, 3, 2, 0)
            .l32r(2, 6)
            .l32r(3, 7)
            .rri8(6, 3, 2, 0)
            .l32r(2, 8)
            .l32r(3, 9)
            .rri8(6, 3, 2, 0)
            .l32r(2, 10)
            .l32r(3, 11)
            .rri8(6, 3, 2, 0)
            .raw(&LOOP);
        program
    }

    #[test]
    fn a_uart_byte_lands_on_the_same_edges_however_the_run_splits() {
        let bytes = |chip: &mut Esp32s3| {
            std::iter::from_fn(|| chip.take_uart(Tick(u64::MAX)))
                .map(|sent| (sent.port, sent.byte, sent.start, sent.end))
                .collect::<Vec<_>>()
        };
        let mut whole = uart_byte().chip();
        let seen = halts(&mut whole, 4000, 4000);
        let sent = bytes(&mut whole);
        assert_eq!(sent.len(), 1, "{seen:?}");
        assert_eq!(sent[0].0, 0);
        assert_eq!(sent[0].1, 0x55);
        assert!(seen.len() > 2, "{seen:?}");
        for step in [1, 3, 7, 40] {
            let mut split = uart_byte().chip();
            assert_eq!(halts(&mut split, 4000, step), seen, "limits {step} apart");
            assert_eq!(bytes(&mut split), sent, "limits {step} apart");
            assert_eq!(split.tick(), whole.tick(), "limits {step} apart");
        }
    }

    /// Two 0x00 bytes written back to back with `tx_idle_num` 0. GPIO21 must fall and rise
    /// for both frames: the second start bit is queued when the first stop bit ends.
    fn uart_queued() -> Program {
        let mut program = Program::new(&[
            IO_MUX + 0x58,
            1 << 12,
            GPIO + 0x5a8,
            12,
            0x6000_0078,
            (1 << 20) | (1 << 22) | (1 << 24) | (1 << 25),
            0x6000_0014,
            80,
            0x6000_0048,
            256,
            0x6000_0000,
            0x00,
        ]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0)
            .l32r(2, 2)
            .l32r(3, 3)
            .rri8(6, 3, 2, 0)
            .l32r(2, 4)
            .l32r(3, 5)
            .rri8(6, 3, 2, 0)
            .l32r(2, 6)
            .l32r(3, 7)
            .rri8(6, 3, 2, 0)
            .l32r(2, 8)
            .l32r(3, 9)
            .rri8(6, 3, 2, 0)
            .l32r(2, 10)
            .l32r(3, 11)
            .rri8(6, 3, 2, 0)
            .rri8(6, 3, 2, 0)
            .raw(&LOOP);
        program
    }

    #[test]
    fn a_queued_byte_drives_its_start_bit_however_the_run_splits() {
        let bytes = |chip: &mut Esp32s3| {
            std::iter::from_fn(|| chip.take_uart(Tick(u64::MAX)))
                .map(|sent| (sent.port, sent.byte, sent.start, sent.end))
                .collect::<Vec<_>>()
        };
        let mut whole = uart_queued().chip();
        let seen = halts(&mut whole, 4000, 4000);
        let drives: Vec<PadDrive> = seen.iter().map(|&(_, drive)| drive).collect();
        let sent = bytes(&mut whole);
        // The route idles the pad high. Each 0x00 frame then falls for its start bit and
        // rises for its stop bit, including the byte that was queued behind the first.
        assert_eq!(
            drives,
            [
                PadDrive::High,
                PadDrive::Low,
                PadDrive::High,
                PadDrive::Low,
                PadDrive::High
            ],
            "{seen:?}"
        );
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert!(sent.iter().all(|byte| byte.0 == 0 && byte.1 == 0x00));
        assert_eq!(
            sent[1].2, sent[0].3,
            "the next start bit begins as the stop bit ends"
        );
        for step in [1, 3, 7, 40] {
            let mut split = uart_queued().chip();
            assert_eq!(halts(&mut split, 4000, step), seen, "limits {step} apart");
            assert_eq!(bytes(&mut split), sent, "limits {step} apart");
            assert_eq!(split.tick(), whole.tick(), "limits {step} apart");
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
        assert_eq!(whole.restart(), Some((c(6), pro(at))));
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
        assert_eq!(
            chip.restart(),
            Some((c(6), pro(at))),
            "a hint of ESP_RST_SW"
        );
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
                Some(Halt::Fault(c(6), Fault::FaultRestart(cause, pro(at)))),
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
                Halt::Fault(c(hold.into()), TIMG0_FLASHBOOT)
            } else {
                Halt::Fault(c(5), Fault::EfuseProgram(pro(store)))
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
                [(Halt::Fault(c(3), TIMG0_FLASHBOOT), PadDrive::Off)],
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
            Some(Halt::Fault(c(2), Fault::EfuseProgram(pro(store))))
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
                1 << 0,
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
            (0x6000_000c, 1 << 5, Refused::Interrupt("UART0")),
        ];
        for (address, value, why) in refused {
            let program = store_after_two_loads(address, value, 0);
            let mut chip = program.chip();
            chip.soc
                .restore(SYSTEM + 0x18, 1 << 6 | 1 << 11 | 1 << 2 | 1 << 5);
            chip.soc.restore(SYSTEM + 0x1c, 1 << 6 | 1 << 2 | 1 << 9);
            chip.soc.restore(SYSTEM + 0x24, 0);
            assert_eq!(
                chip.run_until(c(1000)),
                Some(Halt::Fault(
                    c(2),
                    Fault::Refused(why, pro(program.entry() + 6))
                )),
                "{address:#x}"
            );
        }
        let mut chip = store_after_two_loads(TIMG0 + 0x70, 1 << 2, 0).chip();
        assert_eq!(chip.run_until(c(1000)), None, "the watchdog's interrupt");
    }

    /// `JTAG_IN_FLUSH` and SOF are not among the USB Serial/JTAG enables the model runs.
    #[test]
    fn enabling_sof_or_jtag_in_flush_ends_the_run() {
        for bit in [1 << 0, 1 << 1] {
            let program = store_after_two_loads(USB_SERIAL_JTAG + 0x10, bit, 0);
            assert_eq!(
                program.chip().run_until(c(1000)),
                Some(Halt::Fault(
                    c(2),
                    Fault::Refused(
                        Refused::Interrupt("USB_SERIAL_JTAG"),
                        pro(program.entry() + 6)
                    )
                )),
                "{bit:#x}"
            );
        }
    }

    /// `SERIAL_OUT_RECV_PKT`, `SERIAL_IN_EMPTY` and `USB_BUS_RESET` may be enabled together.
    #[test]
    fn serial_out_recv_pkt_serial_in_empty_and_bus_reset_may_be_enabled() {
        let value = (1 << 2) | (1 << 3) | (1 << 9);
        let mut chip = store_after_two_loads(USB_SERIAL_JTAG + 0x10, value, 0).chip();
        assert_eq!(chip.run_until(c(1000)), None);
    }

    #[test]
    fn a_store_to_usb_wrap_ends_the_run() {
        for address in [0x6003_9000, 0x6003_9004] {
            let program = store_after_two_loads(address, 1, 0);
            assert_eq!(
                program.chip().run_until(c(1000)),
                Some(Halt::Fault(
                    c(2),
                    Fault::Refused(
                        Refused::Register("USB_WRAP", "otg"),
                        pro(program.entry() + 6)
                    )
                )),
                "{address:#x}"
            );
        }
    }

    #[test]
    fn a_load_of_usb_wrap_is_unmodeled() {
        let mut program = Program::new(&[0x6003_9000]);
        program.l32r(2, 0).rri8(2, 3, 2, 0).raw(&LOOP);
        let mut chip = program.chip();
        assert_eq!(chip.run_until(c(1000)), None);
        let touched: Vec<(u32, Touch)> = chip.unmodeled().collect();
        assert_eq!(touched.len(), 1);
        assert_eq!(touched[0].0, 0x6003_9000);
        assert!(!touched[0].1.write);
    }

    /// Each write that changes a register the model keeps without running its effect is
    /// listed, with the PC of its own store, and the word keeps it. A write that leaves the word
    /// as it was is not listed. On SYSTEM's gates only a change of a bit the model keeps without
    /// running counts, and on `GPIO_PINn` only a change of a synchronizer or `CONFIG` bit.
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
            .chain([(RTC_CNTL + 0x74, 0x40), (GPIO + 0x74 + 4 * 21, 1 << 3)])
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
            let listed: Vec<(u32, u32)> = chip
                .unmodeled()
                .map(|(at, touch)| (at, touch.pc.pc))
                .collect();
            assert_eq!(
                listed,
                [(address, first), (address, second)],
                "{address:#x}"
            );
            assert!(chip.unmodeled().all(|(_, touch)| touch.write && touch.kept));
            assert_eq!(chip.soc.bus().load(Cycle(0), address, Width::Word), Ok(0));
        }
        let mut chip = store_after_two_loads(SYSTEM + 0x18, 1 << 6 | 1 << 2, 0).chip();
        chip.soc.restore(SYSTEM + 0x18, 0);
        assert_eq!(chip.run_until(c(1000)), None);
        assert_eq!(chip.unmodeled().count(), 0, "SPI2 runs its clock bit");
        let mut chip = store_after_two_loads(GPIO + 0x74, 1 << 2 | 2 << 7 | 1 << 13, 0).chip();
        assert_eq!(chip.run_until(c(1000)), None);
        assert_eq!(chip.unmodeled().count(), 0, "GPIO_PIN0 runs the rest");
    }

    /// A jump to itself with every interrupt masked ends the run as halted once no watchdog
    /// runs and no source reaches the NMI line, as IDF's print-and-halt panic ends. While a
    /// watchdog runs, or a source is routed to the NMI line, the core spins on.
    #[test]
    fn a_jump_to_itself_that_nothing_can_leave_halts_the_run() {
        let mut program = Program::new(&[]);
        program.raw(&LOOP);
        let quiet = |chip: &mut Esp32s3| {
            chip.soc.watchdogs.write_timg(Tick(0), 0, 0x48, 0).unwrap();
            chip.soc.watchdogs.write_rtc(Tick(0), 0x98, 0).unwrap();
        };
        let mut chip = program.chip();
        quiet(&mut chip);
        assert_eq!(
            chip.run_until(c(1000)),
            Some(Halt::Fault(c(0), Fault::Halted(pro(program.entry()))))
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
        assert_eq!(
            chip.soc.bus().load(Cycle(0), user1, Width::Word),
            Ok(0xb841_0007)
        );

        let mut program = Program::new(&[SYSTEM + 0x20, 1 << 6, user1]);
        program.l32r(2, 0).l32r(3, 1).rri8(6, 3, 2, 0).l32r(4, 2);
        let load = program.pc();
        program.rri8(2, 5, 4, 0).raw(&LOOP);
        let held = Refused::Register("SPI2", "held_in_reset");
        assert_eq!(
            program.chip().run_until(c(1000)),
            Some(Halt::Fault(c(4), Fault::Refused(held, pro(load))))
        );

        let program = store_after_two_loads(0x6001_9000, 0, 0);
        let off = Refused::Register("LEDC", "clock_off");
        assert_eq!(
            program.chip().run_until(c(1000)),
            Some(Halt::Fault(
                c(2),
                Fault::Refused(off, pro(program.entry() + 6))
            ))
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
                        Fault::FlashCommand(FlashRefusal::Pads, pro(program.entry() + 6))
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
                Fault::FlashCommand(FlashRefusal::Spi0, pro(program.entry() + 6))
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
                    pc: pro(program.entry() + 3)
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
                assert_eq!(chip.cores.core(CoreId::Pro).a(3), 0xa5a5_a5a5);
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
        assert_eq!(Touch::register(address), "GPIO+0x60");
        assert!(!touch.write);
        assert_eq!(touch.pc, pro(program.entry() + 5 * 3));
        assert!(touch.at < first);
    }

    #[test]
    fn a_watchdog_left_running_ends_the_run_at_its_first_stage() {
        let mut program = Program::new(&[]);
        program.raw(&WAITI).raw(&LOOP);
        let mut chip = program.chip();
        let expiry = Watchdogs::new(XTAL)
            .deadline()
            .expect("a watchdog runs at power-on");
        assert_eq!(chip.run_until(Tick(expiry.at.0 - 1)), None);
        assert_eq!(
            chip.run_until(Tick(u64::MAX - 1)),
            Some(Halt::Fault(expiry.at, TIMG0_FLASHBOOT))
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

        let mut probe = store_after_two_loads(0x6001_7000, 0, 0).chip();
        assert_eq!(probe.run_until(c(1000)), None);
        let (_, touch) = probe.unmodeled().next().expect("the store touched PCNT");
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
                    (Halt::Fault(at, TIMG0_FLASHBOOT), PadDrive::Low),
                ],
                "limits {step} apart"
            );
        }

        let mut chip = program(1).chip();
        let seen = halts(&mut chip, 1000, 1000);
        assert!(
            matches!(
                seen[..],
                [(Halt::Fault(at, TIMG0_FLASHBOOT), PadDrive::Off)] if at < store
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
            Fault::Setting(Setting::Dummy, pro(program.entry() + 14 * 3))
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
                Fault::Refused(sending, pro(program.entry() + 17 * 3))
            ))
        );
        assert_eq!(
            chip.soc.bus().load(Cycle(0), SYSTEM + 0x20, Width::Word),
            Ok(0)
        );
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
                Fault::Refused(off, pro(program.entry() + 17 * 3))
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
            assert_eq!(
                soc.bus().store(Cycle(cycle), address, Width::Word, value),
                Ok(())
            );
        }
        chip
    }

    /// `dma_chip` with its transfer started at cycle 10, which runs for 64 crystal cycles.
    fn sending(buffer: u32) -> Esp32s3 {
        let mut chip = dma_chip(buffer);
        assert_eq!(
            chip.soc.bus().store(Cycle(10), SPI2, Width::Word, 1 << 24),
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
                chip.soc.bus().store(Cycle(11), address, width, 0),
                Err(Exit::Stop),
                "{address:#x}"
            );
            assert_eq!(chip.soc.ending, refused, "{address:#x}");
            assert_eq!(
                chip.soc.bus().load(Cycle(12), BUFFER + 4, Width::Word),
                Ok(0x0807_0605)
            );
        }
        let mut chip = sending(BUFFER);
        for address in [BUFFER + 8, BUFFER - 4, DESCRIPTOR + 12] {
            assert_eq!(
                chip.soc.bus().store(Cycle(11), address, Width::Word, 0),
                Ok(())
            );
        }
        assert_eq!(
            chip.soc.bus().store(Cycle(74), BUFFER, Width::Word, 0),
            Ok(())
        );
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
                chip.soc.bus().store(Cycle(11), address, width, 0xffff),
                Ok(()),
                "{address:#x}"
            );
            assert_eq!(chip.soc.ending, None, "{address:#x}");
        }
        for address in [BUFFER + 1, BUFFER + 8] {
            let mut chip = sending(BUFFER + 1);
            assert_eq!(
                chip.soc.bus().store(Cycle(11), address, Width::Byte, 0xff),
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
            .bus()
            .load(Cycle(11), SYSTEM + 0x24, Width::Word)
            .unwrap();
        assert_eq!(
            chip.soc.bus().store(
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
            chip.soc.bus().load(Cycle(12), SYSTEM + 0x24, Width::Word),
            Ok(before)
        );
        let mut chip = sending(BUFFER);
        assert_eq!(
            chip.soc.bus().store(
                Cycle(74),
                SYSTEM + 0x24,
                Width::Word,
                before | RESTART_RST_EN1
            ),
            Ok(())
        );
        assert_eq!(chip.soc.ending, None);
        assert_eq!(
            chip.soc.bus().load(Cycle(75), SYSTEM + 0x24, Width::Word),
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
                chip.soc.bus().store(Cycle(11), address, Width::Word, value),
                Err(Exit::Stop),
                "{setting}"
            );
            let refused = Refused::Register(block, setting);
            assert_eq!(chip.soc.ending, Some(Ending::Refused(refused)));
            let mut chip = sending(BUFFER);
            assert_eq!(
                chip.soc.bus().store(Cycle(74), address, Width::Word, value),
                Ok(()),
                "{setting} after the transfer"
            );
        }
        let mut chip = dma_chip(BUFFER);
        assert_eq!(
            chip.soc
                .bus()
                .store(Cycle(10), SYSTEM + 0x1c, Width::Word, 0x600),
            Ok(())
        );
        assert_eq!(
            chip.soc.bus().store(Cycle(11), SPI2, Width::Word, 1 << 24),
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
                chip.soc.bus().store(Cycle(10), address, Width::Word, value),
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
        assert_eq!(
            soc.bus().store(Cycle(1), rtc, Width::Word, 1),
            Err(Exit::Stop)
        );
        assert_eq!(
            soc.bus().store(Cycle(2), spi0, Width::Word, 1),
            Err(Exit::Stop)
        );
        assert_eq!(soc.bus().store(Cycle(2), spi0, Width::Word, 1), Ok(()));
        assert_eq!(
            soc.bus().store(Cycle(3), rtc, Width::Word, 1),
            Err(Exit::Stop)
        );
        assert_eq!(soc.bus().store(Cycle(3), rtc, Width::Word, 1), Ok(()));
        assert_eq!(
            soc.bus().store(Cycle(4), spi0, Width::Word, 2),
            Err(Exit::Stop)
        );
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
            chip.soc.bus().next_event(Cycle(18)),
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
        let mut chip = Esp32s3::new(&[], 0x4000_0400, [], None);
        assert_eq!(
            chip.run_until(c(10)),
            Some(Halt::Fault(c(0), Fault::UnmappedFetch(pro(0x4000_0400))))
        );
        let data = RAM[1].start;
        let mut chip = Esp32s3::new(&[], data, [(data, [0u8; 4].as_slice())], None);
        assert_eq!(
            chip.run_until(c(10)),
            Some(Halt::Fault(c(0), Fault::UnmappedFetch(pro(data)))),
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
            assert_eq!(
                chip.cores.core(CoreId::Pro).special(Sr::ExcCause),
                3,
                "{op}"
            );
            assert_eq!(
                chip.cores.core(CoreId::Pro).special(Sr::ExcVaddr),
                at,
                "{op}"
            );
            assert_eq!(
                chip.soc.bus().load(Cycle(0), at - 1, Width::Word),
                Ok(0),
                "{op}"
            );
        }

        let mut soc = Esp32s3::new(&[], BASE, [], None).soc;
        let iram = RAM[0].start + 0x8040;
        for width in [Width::Byte, Width::Half] {
            assert_eq!(
                soc.bus().load(Cycle(0), iram, width),
                Err(Exit::Fault(Cause::LoadStoreError))
            );
            assert_eq!(
                soc.bus().store(Cycle(0), iram, width, 1),
                Err(Exit::Fault(Cause::LoadStoreError))
            );
        }
        for width in [Width::Half, Width::Word] {
            assert_eq!(
                soc.bus().load(Cycle(0), iram + 1, width),
                Err(Exit::Fault(Cause::LoadStoreAlignment))
            );
            assert_eq!(
                soc.bus().store(Cycle(0), iram + 1, width, 1),
                Err(Exit::Fault(Cause::LoadStoreAlignment))
            );
        }
        assert_eq!(
            soc.bus().load(Cycle(0), RAM[1].start + 0x40, Width::Byte),
            Ok(0),
            "the data bus takes a byte"
        );
    }

    #[test]
    fn both_windows_reach_the_same_sram() {
        let mut soc = Esp32s3::new(&[], BASE, [], None).soc;
        let through_data = RAM[1].start + 0x40;
        let through_code = RAM[0].start + 0x8040;
        soc.bus()
            .store(Cycle(0), through_data, Width::Word, 0x1234_5678)
            .unwrap();
        assert_eq!(
            soc.bus().load(Cycle(0), through_code, Width::Word),
            Ok(0x1234_5678)
        );
        assert_eq!(
            soc.bus().load(Cycle(0), 0x5100_0000, Width::Word),
            Err(Exit::Fault(Cause::LoadStoreError))
        );
    }

    /// Each store of `pairs` in turn, the value to the address, store `k` at cycle `3k + 2`,
    /// then a jump to itself.
    fn stores(pairs: &[(u32, u32)]) -> Program {
        let literals: Vec<u32> = pairs.iter().flat_map(|&(a, v)| [a, v]).collect();
        let mut program = Program::new(&literals);
        for k in 0..pairs.len() {
            program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
        }
        program.raw(&LOOP);
        program
    }

    /// The PC of store `k` of [`stores`].
    fn store_pc(program: &Program, k: u32) -> Pc {
        pro(program.entry() + 9 * k + 6)
    }

    const LEDC: u32 = 0x6001_9000;

    /// Routes LEDC channel 0 to GPIO21, which drives it from store 1, at cycle 5, enables the
    /// pad, turns LEDC's clock on, picks the clock `clock` and starts timer 0 at 1 bit and divider
    /// 1, a period of two clock cycles. The timer counts from store 5, at cycle 17.
    fn ledc_on(clock: u32) -> Vec<(u32, u32)> {
        vec![
            (IO_MUX + 4 + 4 * 21, 1 << 12 | 3 << 10 | 1 << 9),
            (GPIO + 0x554 + 4 * 21, u32::from(ledc::SIGNAL)),
            (GPIO + 0x24, GPIO21),
            (SYSTEM + 0x18, 0xf9c1_e06f | 1 << 11),
            (LEDC + 0xd0, clock),
            (LEDC + 0xa0, 1 | 256 << 4 | 1 << 25),
        ]
    }

    /// The three stores that set channel 0 to `duty` of its steps on timer 0, as IDF's
    /// `ledc_set_duty` and `ledc_update_duty` do.
    fn duty(duty: u32) -> [(u32, u32); 3] {
        [
            (LEDC + 0x08, duty << 4),
            (LEDC + 0x0c, 1 << 31 | 1 << 30 | 1 << 20 | 1 << 10),
            (LEDC, 1 << 2 | 1 << 4),
        ]
    }

    fn record_at(at: Tick) -> Halt {
        Halt::Change {
            at,
            image: false,
            record: true,
        }
    }

    /// The updates at cycles 26 and 35 take effect at the timer's next overflows, 24 ticks
    /// apart from cycle 17: cycles 27 and 37. Each sends its record, then the drive it moves.
    #[test]
    fn an_led_duty_update_sends_its_record_then_moves_the_pads_drive() {
        let program = stores(&[ledc_on(3), duty(1).to_vec(), duty(0).to_vec()].concat());
        for step in [1, 1000] {
            let mut chip = program.chip();
            assert_eq!(
                halts(&mut chip, 1000, step),
                [
                    (image_at(c(5)), PadDrive::Low),
                    (record_at(c(27)), PadDrive::Low),
                    (Halt::Moved(c(27)), PadDrive::High),
                    (record_at(c(37)), PadDrive::High),
                    (Halt::Moved(c(37)), PadDrive::Low),
                ],
                "limits {step} apart"
            );
            let duties: Vec<Duty> = chip.take_duties().collect();
            let sent = |at, duty| Duty {
                at,
                pad: 21,
                duty,
                resolution: 1,
                hz: 20_000_000,
            };
            assert_eq!(duties, [sent(c(27), 1), sent(c(37), 0)]);
        }
    }

    #[test]
    fn a_duty_change_that_keeps_the_drive_sends_only_its_record() {
        let program = stores(&[ledc_on(3), duty(1).to_vec(), duty(2).to_vec()].concat());
        let mut chip = program.chip();
        assert_eq!(
            halts(&mut chip, 1000, 1000),
            [
                (image_at(c(5)), PadDrive::Low),
                (record_at(c(27)), PadDrive::Low),
                (Halt::Moved(c(27)), PadDrive::High),
                (record_at(c(37)), PadDrive::High),
            ]
        );
    }

    /// Routes LEDC channel `channel` to GPIO22 and enables the pad, in three stores.
    fn pad22(channel: u16) -> [(u32, u32); 3] {
        [
            (IO_MUX + 4 + 4 * 22, 1 << 12 | 3 << 10 | 1 << 9),
            (GPIO + 0x554 + 4 * 22, u32::from(ledc::SIGNAL + channel)),
            (GPIO + 0x24, GPIO21 | 1 << 22),
        ]
    }

    fn pads_and_duties(chip: &mut Esp32s3) -> Vec<(Tick, u8, u32)> {
        halts(chip, 1000, 1000);
        chip.take_duties().map(|d| (d.at, d.pad, d.duty)).collect()
    }

    /// Each netted pad that carries the channel gets its own record, and a pad with no net or on
    /// another channel gets none.
    #[test]
    fn every_netted_pad_on_a_channel_gets_its_duty() {
        let other = stores(&[ledc_on(3), pad22(1).to_vec(), duty(1).to_vec()].concat());
        let pads = pads_and_duties(&mut other.chip_on(&[21, 22]));
        assert_eq!(pads, [(c(37), 21, 1)]);
        let program = stores(&[ledc_on(3), pad22(0).to_vec(), duty(1).to_vec()].concat());
        let both = pads_and_duties(&mut program.chip_on(&[21, 22]));
        assert_eq!(both, [(c(37), 21, 1), (c(37), 22, 1)]);
        assert_eq!(
            pads_and_duties(&mut program.chip_on(&[22])),
            [(c(37), 22, 1)]
        );
        assert_eq!(pads_and_duties(&mut program.chip_on(&[])), []);
    }

    /// A pad routed to the channel after its update took effect gets the channel's duty at the
    /// store that routes it, cycle 32, since LEDC enables its own output. A route away from the
    /// channel sends nothing.
    #[test]
    fn a_pad_routed_to_a_running_channel_gets_its_duty_at_the_route() {
        let off = [(GPIO + 0x554 + 4 * 22, 0x100)];
        let pairs = [
            ledc_on(3),
            duty(1).to_vec(),
            pad22(0).to_vec(),
            off.to_vec(),
        ];
        let program = stores(&pairs.concat());
        let mut chip = program.chip_on(&[21, 22]);
        assert_eq!(pads_and_duties(&mut chip), [(c(27), 21, 1), (c(32), 22, 1)]);
    }

    /// GPIO22 drives its own `GPIO_OUT` low from cycle 23, and channel 0 sends 0 from cycle 33.
    /// The store at cycle 35 that routes the pad to the channel keeps its drive, and its record
    /// still stops the core at that store, before the two SRAM stores after it.
    #[test]
    fn a_route_that_keeps_the_drive_still_halts_at_its_store() {
        let pairs = [
            ledc_on(3),
            vec![
                (IO_MUX + 4 + 4 * 22, 1 << 12 | 3 << 10 | 1 << 9),
                (GPIO + 0x24, GPIO21 | 1 << 22),
            ],
            duty(0).to_vec(),
            vec![(GPIO + 0x554 + 4 * 22, u32::from(ledc::SIGNAL))],
            vec![(RAM[1].start + 0x100, 0); 2],
        ];
        let program = stores(&pairs.concat());
        let mut chip = program.chip_on(&[21, 22]);
        let mut at_route = None;
        while let Some(halt) = chip.run_until(c(1000)) {
            if halt == record_at(c(35)) {
                at_route = Some(chip.now());
            }
        }
        assert_eq!(at_route, Some(c(36)));
    }

    /// A timer of three cycles a period, from cycle 17, overflows at every store. The update sent
    /// at cycle 35 takes effect at cycle 38, before the store there that routes GPIO22 away, so
    /// both pads get it.
    #[test]
    fn an_update_due_at_a_route_store_goes_on_the_pads_from_before_it() {
        let mut on = ledc_on(3);
        on[5].1 = 1 | 384 << 4 | 1 << 25;
        let pairs = [
            on,
            pad22(0).to_vec(),
            duty(1).to_vec(),
            vec![(GPIO + 0x554 + 4 * 22, 0x100)],
        ];
        let program = stores(&pairs.concat());
        let pads = pads_and_duties(&mut program.chip_on(&[21, 22]));
        assert_eq!(pads, [(c(38), 21, 1), (c(38), 22, 1)]);
    }

    /// `OUT_INV_SEL` on the pad: at full duty the pad drives low, and the record sends 0 of the
    /// 2 steps. Clearing it at cycle 29 drives the pad high and sends the duty, 2, at that store.
    #[test]
    fn an_inverted_pad_sends_the_duty_it_drives() {
        let mut on = ledc_on(3);
        on[1].1 |= 1 << 9;
        let pairs = [
            on,
            duty(2).to_vec(),
            vec![(GPIO + 0x554 + 4 * 21, u32::from(ledc::SIGNAL))],
        ];
        let program = stores(&pairs.concat());
        let mut chip = program.chip();
        assert_eq!(
            halts(&mut chip, 1000, 1000),
            [
                (image_at(c(5)), PadDrive::High),
                (record_at(c(27)), PadDrive::High),
                (Halt::Moved(c(27)), PadDrive::Low),
                (
                    Halt::Change {
                        at: c(29),
                        image: true,
                        record: true,
                    },
                    PadDrive::High
                ),
            ]
        );
        let duties: Vec<u32> = chip.take_duties().map(|d| d.duty).collect();
        assert_eq!(duties, [0, 2]);
    }

    /// At a partial duty an inverted pad drives low, and the record sends the steps the channel
    /// does not: 3 of the 4 steps of a 2-bit timer at duty 1, from cycle 29.
    #[test]
    fn an_inverted_pad_at_a_partial_duty_drives_low() {
        let mut on = ledc_on(3);
        on[1].1 |= 1 << 9;
        on[5].1 = 2 | 256 << 4 | 1 << 25;
        let program = stores(&[on, duty(1).to_vec()].concat());
        let mut chip = program.chip();
        assert_eq!(
            halts(&mut chip, 1000, 1000),
            [
                (image_at(c(5)), PadDrive::High),
                (record_at(c(29)), PadDrive::High),
                (Halt::Moved(c(29)), PadDrive::Low),
            ]
        );
        let duties: Vec<u32> = chip.take_duties().map(|d| d.duty).collect();
        assert_eq!(duties, [3]);
    }

    /// LEDC's reset bit, set at cycle 32 once GPIO21 is routed away from channel 0 at cycle 29,
    /// puts the signal back at its power-on level, low. Routing the pad back to the channel at
    /// cycle 35 then keeps it low and sends no record, since the channel has no duty.
    #[test]
    fn an_led_reset_with_no_pad_on_a_channel_drives_its_signals_low() {
        let route = |signal| [(GPIO + 0x554 + 4 * 21, signal)];
        let pairs = [
            ledc_on(3),
            duty(1).to_vec(),
            route(0x100).to_vec(),
            vec![(SYSTEM + 0x20, 1 << 11)],
            route(u32::from(ledc::SIGNAL)).to_vec(),
        ];
        let program = stores(&pairs.concat());
        let mut chip = program.chip();
        assert_eq!(
            halts(&mut chip, 1000, 1000),
            [
                (image_at(c(5)), PadDrive::Low),
                (record_at(c(27)), PadDrive::Low),
                (Halt::Moved(c(27)), PadDrive::High),
                (image_at(c(29)), PadDrive::Low),
            ]
        );
        assert_eq!(chip.take_duties().count(), 1);
    }

    /// LEDC's reset bit ends the run at its store while a netted pad carries a channel, while an
    /// update waits for its overflow, or when an update takes effect at that store, on a pad or
    /// on none.
    #[test]
    fn an_led_reset_while_a_channel_runs_ends_the_run() {
        let reset = vec![(SYSTEM + 0x20, 1 << 11)];
        let why = Refused::Register("LEDC", "reset_while_running");
        let refused = |program: &Program, netted: &[u8], k, records| {
            let mut chip = program.chip_on(netted);
            let last = halts(&mut chip, 1000, 1000).pop().map(|(halt, _)| halt);
            let fault = Fault::Refused(why, store_pc(program, k));
            assert_eq!(last, Some(Halt::Fault(c(u64::from(3 * k + 2)), fault)));
            assert_eq!(chip.take_duties().count(), records);
        };
        let on_pad = stores(&[ledc_on(3), duty(1).to_vec(), reset.clone()].concat());
        refused(&on_pad, &[21], 9, 1);
        let mut slow = ledc_on(3);
        slow[5].1 = 1 | 2048 << 4 | 1 << 25;
        let waiting = stores(&[slow, duty(1).to_vec(), reset.clone()].concat());
        refused(&waiting, &[], 9, 0);
        let mut on = ledc_on(3);
        on[5].1 = 1 | 384 << 4 | 1 << 25;
        let pairs = [on, pad22(0).to_vec(), duty(1).to_vec(), reset];
        let due = stores(&pairs.concat());
        refused(&due, &[21, 22], 12, 0);
        refused(&due, &[], 12, 0);
    }

    /// IDF's cold-boot order: start-up sets LEDC's reset bit, `ledc_set_pin` routes the netted
    /// GPIO21 to channel 0, and `ledc_timer_config` turns LEDC's clock on before it clears the
    /// reset. Neither store sets the bit, so the run goes on and channel 0 sends its duty.
    #[test]
    fn a_reset_held_from_start_up_lets_the_clock_store_through() {
        let on = ledc_on(3);
        let pairs = [
            vec![(SYSTEM + 0x20, 1 << 11)],
            on[..4].to_vec(),
            vec![(SYSTEM + 0x20, 0)],
            on[4..].to_vec(),
            duty(1).to_vec(),
        ];
        let program = stores(&pairs.concat());
        let mut chip = program.chip();
        assert_eq!(
            halts(&mut chip, 1000, 1000),
            [
                (image_at(c(8)), PadDrive::Low),
                (record_at(c(33)), PadDrive::Low),
                (Halt::Moved(c(33)), PadDrive::High),
            ]
        );
        assert_eq!(chip.take_duties().count(), 1);
    }

    /// Timer group 0's watchdog, armed at cycle 29, resets the system at cycle 49, inside a jump
    /// of the program's closing loop, so the core runs past it. The update sent at cycle 38 takes
    /// effect at the overflow at cycle 49 of a timer of 16 cycles a period, and the reset comes
    /// first.
    #[test]
    fn an_update_at_the_watchdogs_reset_is_not_sent() {
        let mut on = ledc_on(3);
        on[5].1 = 1 | 2048 << 4 | 1 << 25;
        let watchdog = [
            (TIMG0 + 0x48, 0),
            (TIMG0 + 0x50, 20),
            (TIMG0 + 0x54, 0),
            (TIMG0 + 0x48, EN | 3 << 29),
        ];
        let program = stores(&[on, watchdog.to_vec(), duty(1).to_vec()].concat());
        let mut chip = program.chip();
        let reset = Fault::Watchdog {
            watchdog: Watchdog::Timg0,
            stage: 0,
            reset: Reset::System,
        };
        let seen: Vec<Halt> = halts(&mut chip, 1000, 1000)
            .into_iter()
            .map(|(h, _)| h)
            .collect();
        assert_eq!(seen, [image_at(c(5)), Halt::Fault(c(49), reset)]);
    }

    /// LEDC's clock gate off, or an APB change under a timer that counts the APB clock, ends
    /// the run at its store. A crystal-clocked timer keeps counting across the APB change.
    #[test]
    fn a_clock_change_under_a_counting_led_timer_ends_the_run() {
        let last = |program: &Program| {
            halts(&mut program.chip(), 1000, 1000)
                .pop()
                .map(|(halt, _)| halt)
        };
        let off = stores(&[ledc_on(3), vec![(SYSTEM + 0x18, 0xf9c1_e06f)]].concat());
        let why = Refused::Register("LEDC", "clock_off_while_running");
        assert_eq!(
            last(&off),
            Some(Halt::Fault(c(20), Fault::Refused(why, store_pc(&off, 6))))
        );
        let pll = [(SYSTEM + 0x10, 0x6), (SYSTEM + 0x60, 1 << 10)];
        let apb = stores(&[ledc_on(1), pll.to_vec()].concat());
        let why = Refused::Register("LEDC", "clock_change_while_running");
        assert_eq!(
            last(&apb),
            Some(Halt::Fault(c(23), Fault::Refused(why, store_pc(&apb, 7))))
        );
        let xtal = stores(&[ledc_on(3), pll.to_vec()].concat());
        assert_eq!(
            last(&xtal),
            Some(image_at(c(5))),
            "the crystal does not change"
        );
    }

    const EN: u32 = 1 << 31;

    /// Timer group 0's watchdog, stopped and then started at cycle 11: stage 0 interrupts at
    /// cycle 21, and its source reads in the interrupt matrix's status from then while `INT_ENA`
    /// holds the bit. Stage 1 resets the system 5 cycles later.
    #[test]
    fn a_watchdog_stage_that_interrupts_raises_its_source_and_the_next_stage_resets() {
        let program = stores(&[
            (TIMG0 + 0x48, 0),
            (TIMG0 + 0x50, 10),
            (TIMG0 + 0x54, 5),
            (TIMG0 + 0x48, EN | 1 << 29 | 3 << 27),
        ]);
        let status = |chip: &mut Esp32s3| {
            let word = chip.soc.bus().load(Cycle(0), 0x600c_2190, Width::Word);
            word.map(|word| word >> (52 - 32) & 1)
        };
        let mut chip = program.chip();
        chip.soc.restore(TIMG0 + 0x70, 1 << 2);
        assert_eq!(chip.run_until(c(15)), None);
        assert_eq!(status(&mut chip), Ok(0));
        assert_eq!(chip.run_until(c(25)), None);
        assert_eq!(status(&mut chip), Ok(1));
        let reset = Fault::Watchdog {
            watchdog: Watchdog::Timg0,
            stage: 1,
            reset: Reset::System,
        };
        assert_eq!(chip.run_until(c(1000)), Some(Halt::Fault(c(26), reset)));

        let mut quiet = program.chip();
        assert_eq!(quiet.run_until(c(25)), None);
        assert_eq!(status(&mut quiet), Ok(0), "INT_ENA clear");
    }

    #[test]
    fn a_watchdog_stage_the_model_does_not_run_ends_the_run_at_its_store() {
        let refused = [
            (
                TIMG0 + 0x48,
                EN | 2 << 29,
                Refused::Register("TIMG0", "watchdog_cpu_reset"),
            ),
            (
                TIMG1 + 0x48,
                EN | 2 << 29,
                Refused::Register("TIMG1", "watchdog_cpu_reset"),
            ),
            (
                RTC_CNTL + 0x98,
                EN | 1 << 28,
                Refused::Register("RTC_CNTL", "watchdog_interrupt"),
            ),
            (
                RTC_CNTL + 0x98,
                EN | 6 << 28,
                Refused::Register("RTC_CNTL", "watchdog_stage"),
            ),
            (
                RTC_CNTL + 0xb4,
                300 << 18,
                Refused::Register("RTC_CNTL", "super_watchdog"),
            ),
        ];
        for (address, value, why) in refused {
            let program = store_after_two_loads(address, value, 0);
            assert_eq!(
                program.chip().run_until(c(1000)),
                Some(Halt::Fault(
                    c(2),
                    Fault::Refused(why, pro(program.entry() + 6))
                )),
                "{address:#x}"
            );
        }
    }

    /// Enables GPIO21 at cycle 2, lets level-1 interrupt 1 in, stops timer group 0's watchdog
    /// and holds its stage 0 to 0, starts it with `config` at cycle 12, and sets GPIO21 at
    /// cycle 13, at the PC it returns. Timer group 0's watchdog source reaches interrupt 1.
    fn armed(config: u32) -> (Esp32s3, u32) {
        let mut program = Program::new(&[GPIO, GPIO21, TIMG0, config]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, 0x24)
            .movi(7, 2)
            .wsr(7, INTENABLE)
            .movi(7, 0)
            .wsr(7, PS)
            .l32r(4, 2)
            .movi(5, 0)
            .rri8(6, 5, 4, 0x48)
            .rri8(6, 5, 4, 0x50)
            .l32r(5, 3)
            .rri8(6, 5, 4, 0x48);
        let set = program.pc();
        program.rri8(6, 3, 2, 0x08).raw(&LOOP);
        let mut chip = program.chip();
        chip.soc.restore(TIMG0 + 0x70, 1 << 2);
        chip.soc.restore(0x600c_2000 + 4 * 52, 1);
        (chip, set)
    }

    /// A watchdog start whose stage 0 runs out on the next cycle comes before the next
    /// instruction, however the run splits. A stage that resets ends the run before that
    /// instruction sets GPIO21. A stage that interrupts is taken at it, so `EPC1` holds its PC
    /// and GPIO21 stays low. The kernel vector is outside any memory, so the run ends there.
    #[test]
    fn a_watchdog_stage_due_just_after_its_start_comes_before_the_next_instruction() {
        for step in [1, 1000] {
            let (mut chip, _) = armed(EN | 3 << 29);
            assert_eq!(
                halts(&mut chip, 1000, step),
                [
                    (image_at(c(2)), PadDrive::Low),
                    (Halt::Fault(c(13), TIMG0_FLASHBOOT), PadDrive::Low),
                ],
                "limits {step} apart"
            );

            let (mut chip, set) = armed(EN | 1 << 29 | 3 << 27);
            let seen = halts(&mut chip, 1000, step);
            assert!(
                matches!(
                    seen[..],
                    [
                        (Halt::Change { .. }, PadDrive::Low),
                        (Halt::Fault(_, Fault::UnmappedFetch(pc)), PadDrive::Low),
                    ] if pc == pro(0x4000_0300)
                ),
                "limits {step} apart: {seen:?}"
            );
            assert_eq!(
                chip.cores.core(CoreId::Pro).special(Sr::Epc1),
                set,
                "limits {step} apart"
            );
        }
    }
}
