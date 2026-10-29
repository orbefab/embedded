//! What the cores see: their memories and the peripherals, on the chip's tick timeline.
//!
//! Every chip has SRAM, both RTC memories and the peripherals. A chip booted from flash also has
//! the mask ROM and the flash through the MMU. Its registers that the captured state
//! names and no model owns are kept as plain words.
//!
//! Each core reaches the SoC through a [`CoreBus`], which knows the core and the step it is in.
//! Every peripheral access turns its cycle into a tick through [`Soc::stamp`], so a peripheral
//! never sees time go back when one core's access comes a cycle after its step's start.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use sim_xtensa::{Bus, Cause, Config, CoreId, Cycle, Exit, PagedRam, Width};

use super::analog::Analog;
use super::app::{self, AppControl, Hold, Steer};
use super::calibration::{self, Calibration};
use super::clock::{CpuClock, Source, Tick};
use super::efuse::Efuse;
use super::extmem;
use super::flash::{self, Flash};
use super::gates::{self, Block, Gates};
use super::gdma::{self, Gdma};
use super::gpio::{self, Gpio, PADS, PadDrive};
use super::ledc::{self, Ledc};
use super::matrix::{self, Matrix};
use super::rom;
use super::sha::{self, Sha};
use super::spi1::{self, FlashRefusal, Spi1};
use super::spi2::{self, Setting, Spi2};
use super::timers::{Register, Systimer, Watchdogs};
use super::usb::UsbSerialJtag;
use super::{Duty, Pc, RAM, Refused, ResetCause, Touch};

pub(crate) const SRAM: usize = 512 * 1024;
/// RTC fast memory, on both buses at one address.
pub(crate) const RTC_FAST: Range<u32> = 0x600f_e000..0x6010_0000;
/// RTC slow memory, on the data bus.
pub(crate) const RTC_SLOW: Range<u32> = 0x5000_0000..0x5000_2000;

const PERIPHERALS: Range<u32> = 0x6000_0000..0x600d_1000;
pub(crate) const GPIO: u32 = 0x6000_4000;
pub(crate) const RTC_CNTL: u32 = 0x6000_8000;
pub(crate) const IO_MUX: u32 = 0x6000_9000;
/// SPI0, the controller the cache reads the flash through. Its registers are plain words.
const SPI0: u32 = 0x6000_3000;
const SPI1: u32 = 0x6000_2000;
pub(crate) const EFUSE: u32 = 0x6000_7000;
const ANALOG: u32 = 0x6000_e000;
pub(crate) const TIMG0: u32 = 0x6001_f000;
pub(crate) const TIMG1: u32 = 0x6002_0000;
pub(crate) const SYSTIMER: u32 = 0x6002_3000;
pub(crate) const SPI2: u32 = 0x6002_4000;
const LEDC: u32 = 0x6001_9000;
pub(crate) const USB_SERIAL_JTAG: u32 = 0x6003_8000;
const SHA: u32 = 0x6003_b000;
pub(crate) const GDMA: u32 = 0x6003_f000;
pub(crate) const SYSTEM: u32 = 0x600c_0000;
/// `SYSTEM_CPU_PER_CONF_REG` and `SYSTEM_SYSCLK_CONF_REG`, which pick the CPU clock.
const CPU_PER_CONF: u32 = 0x10;
const SYSCLK_CONF: u32 = 0x60;
/// The permission controls. Its registers start at 0 and keep what is written, and no access
/// is checked against them.
const SENSITIVE: u32 = 0x600c_1000;
const INTERRUPT_CORE0: u32 = 0x600c_2000;
pub(crate) const EXTMEM: u32 = 0x600c_4000;
const MMU_TABLE: Range<u32> = flash::MMU_TABLE..flash::MMU_TABLE + 4 * flash::ENTRIES as u32;
const BLOCK: u32 = 0x1000;

#[derive(Clone, Debug)]
pub(crate) struct Soc {
    pub(crate) clock: CpuClock,
    sram: PagedRam,
    rtc_fast: PagedRam,
    rtc_slow: PagedRam,
    /// The mask ROM, on a chip booted from flash.
    rom: Option<Arc<[u8]>>,
    pub(crate) flash: Option<Flash>,
    /// Registers kept as plain words, by address.
    plain: BTreeMap<u32, u32>,
    pub(crate) gpio: Gpio,
    pub(crate) systimer: Systimer,
    pub(crate) watchdogs: Watchdogs,
    pub(crate) usb: UsbSerialJtag,
    pub(crate) spi2: Spi2,
    gdma: Gdma,
    sha: Sha,
    pub(crate) ledc: Ledc,
    /// The LEDC updates that took effect and that the chip has not taken.
    pub(crate) effects: Vec<ledc::Effect>,
    /// The `Duty` records those updates and the routes to netted pads made, in time order.
    pub(crate) duties: Vec<Duty>,
    gates: Gates,
    analog: Analog,
    efuse: Efuse,
    spi1: Spi1,
    matrix: Matrix,
    /// Each timer group's RTC clock calibration.
    calibration: [Calibration; 2],
    /// The GPIOs with nets, whose drives the image holds.
    netted: Vec<u8>,
    pub(crate) unmodeled: BTreeMap<u32, Touch>,
    /// Each write that changed a register the model keeps without running its effect, with
    /// that register, in the order the writes landed.
    pub(crate) changes: Vec<(u32, Touch)>,
    /// The register, word and touch of the change the last stop reported, which is listed when
    /// the core runs the store again and it lands.
    pub(crate) pending: Option<(u32, u32, Touch)>,
    /// The SRAM offsets of the descriptors and buffers the last SPI2 transfer read through GDMA,
    /// which a store may not reach while it runs.
    in_flight: Vec<Range<u32>>,
    /// A store changed a netted pad's drive at this tick, so the next fetch stops the core.
    pub(crate) changed: Option<Tick>,
    /// A store changed the CPU clock, so the next fetch stops the core and the run maps its
    /// limits to cycles again.
    pub(crate) reclocked: bool,
    /// A store moved a watchdog's deadline or the tick of the next LEDC update, or sent a `Duty`
    /// record for a pad it routed, so the next fetch stops the core and the run reads the
    /// change.
    pub(crate) rearmed: bool,
    /// The first access to an unmodeled register stopped the core, so the timeline can take the
    /// access's PC.
    pub(crate) first_touch: Option<u32>,
    /// A change of a kept register stopped the core, so the timeline can take its PC.
    pub(crate) change_stop: bool,
    /// The core fetched outside its memories.
    pub(crate) unmapped_fetch: bool,
    /// A store started an SPI2 transfer, so the next fetch stops the core and the run reads its
    /// end.
    pub(crate) started: bool,
    /// A store ended the run, and stopped the core.
    pub(crate) ending: Option<Ending>,
    app: AppControl,
    /// Taken by the scheduler after every slice. A step keeps the strongest of its steers.
    pub(crate) steer: Option<Steer>,
    /// The latest tick a peripheral saw. A later access is never earlier.
    latest: Tick,
}

/// The step a core is in, as its bus sees it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Step {
    pub(crate) core: CoreId,
    /// The start of the step, which the poll that opens every step sets.
    pub(crate) start: Cycle,
    /// Whether the APP CPU has started. A clock switch then lands at `start`, or at the cycle
    /// the latest tick lies on when that is later.
    pub(crate) paired: bool,
    /// Whether the other core is halted, so a store outside SRAM and RTC memory wakes it.
    pub(crate) other_sleeps: bool,
}

/// What one core sees of the SoC.
pub(crate) struct CoreBus<'a> {
    pub(crate) soc: &'a mut Soc,
    pub(crate) step: Step,
}

/// Why a store ended the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ending {
    /// It started an SPI2 transfer with a setting the model does not run.
    Setting(Setting),
    /// It started an eFuse program command.
    EfuseProgram,
    /// It sent the flash a command the model refuses.
    FlashCommand(FlashRefusal),
    /// It reset the core or the whole chip, from `esp_restart` when the cause is `None`.
    Restart(Option<ResetCause>),
    /// It set something the model does not run.
    Refused(Refused),
    /// It loaded from this address in a cache window, where the MMU maps no flash.
    UnmappedLoad(u32),
    /// It picked a CPU clock the model does not run, the RC fast clock or a reserved setting.
    Clock,
}

/// `RTC_CNTL_OPTIONS0_REG`, and its bits that reset the PRO CPU and the whole chip.
const OPTIONS0: u32 = 0x0;
const RESETS: u32 = 1 << 31 | 1 << 5;
/// `RTC_CNTL_STORE6_REG`, where ESP-IDF records why it resets the chip. IDF v5.5.5,
/// `components/esp_system/port/soc/esp32s3/reset_reason.c`.
const STORE6: u32 = 0xc8;

/// Why the chip resets, from `store6`, the word in `RTC_CNTL_STORE6_REG`. IDF records a hint
/// there as `hint | hint << 16 | 1 << 31` before a panic or watchdog reset, and `esp_restart`
/// leaves it 0, which is `None`. Any other word, or none, is a cause the model cannot read.
fn reset_cause(store6: Option<u32>) -> Option<ResetCause> {
    let word = match store6 {
        Some(0) => return None,
        Some(word) => word,
        None => return Some(ResetCause::Unreadable),
    };
    let hint = word & 0x7fff;
    if word & 1 << 31 == 0 || (word >> 16) & 0x7fff != hint {
        return Some(ResetCause::Unreadable);
    }
    Some(match hint {
        3 => return None,
        4 => ResetCause::Panic,
        5 => ResetCause::InterruptWatchdog,
        6 => ResetCause::TaskWatchdog,
        7 => ResetCause::Watchdog,
        9 => ResetCause::Brownout,
        _ => ResetCause::Unreadable,
    })
}

/// What a write does that the model does not run.
#[derive(Clone, Copy, Debug)]
enum Effect {
    /// The write ends the run when it leaves any of these bits set.
    Refuse(u32, Ending),
    /// The write ends the run unless it leaves these bits at this value.
    Pin(u32, u32, Ending),
    /// Each write that changes the word is listed as an unmodeled register, and then kept.
    Report,
    /// A write to SYSTEM's gates is listed when it changes a bit the model keeps without
    /// running, and then kept.
    Gates,
}

/// The effect of a write of the register at `addr`, when it has one the model does not run.
fn effect(addr: u32) -> Option<Effect> {
    let (base, offset) = (addr & !(BLOCK - 1), addr & (BLOCK - 1));
    let interrupt = |peripheral| Ending::Refused(Refused::Interrupt(peripheral));
    let dma = |setting| Ending::Refused(Refused::Register("GDMA", setting));
    let pads = Ending::FlashCommand(FlashRefusal::Pads);
    Some(match (base, offset) {
        (RTC_CNTL, 0x18) => Effect::Refuse(1 << 31, Ending::Refused(Refused::Sleep)),
        (RTC_CNTL, 0x74) => Effect::Report,
        (SPI2, 0x34) => Effect::Refuse(!spi2::TRANS_DONE, interrupt("SPI2")),
        (TIMG0, 0x70) => Effect::Refuse(0b11, interrupt("TIMG0")),
        (TIMG1, 0x70) => Effect::Refuse(0b11, interrupt("TIMG1")),
        (USB_SERIAL_JTAG, 0x10) => Effect::Refuse(u32::MAX, interrupt("USB_SERIAL_JTAG")),
        (SHA, sha::INT_ENA) => Effect::Refuse(1, interrupt("SHA")),
        (LEDC, ledc::INT_ENA) => Effect::Refuse(u32::MAX, interrupt("LEDC")),
        (GDMA, _) if offset < 5 * 0xc0 => match offset % 0xc0 {
            0x10 | 0x70 => Effect::Refuse(u32::MAX, interrupt("GDMA")),
            0x00 => Effect::Refuse(gdma::MEM_TRANS_EN, dma("memory_copy")),
            0x60 => Effect::Refuse(gdma::AUTO_WRBACK, dma("write_back")),
            0x20 => Effect::Refuse(gdma::INLINK_RESTART, dma("restart")),
            0x80 => Effect::Refuse(gdma::OUTLINK_RESTART, dma("restart")),
            _ => return None,
        },
        (SPI0, 0x0) => Effect::Refuse(spi1::STARTS, Ending::FlashCommand(FlashRefusal::Spi0)),
        (SPI0, 0x18..=0x20) => Effect::Report,
        (SYSTEM, _) if gates::OFFSETS.contains(&offset) => Effect::Gates,
        // The flash's pads, GPIO26 to GPIO32, keep IO_MUX function 0, SPI0's and SPI1's, their
        // GPIO output enables clear, and their matrix outputs at the reset value.
        (IO_MUX, 0x6c..=0x84) => Effect::Pin(gpio::MCU_SEL, 0, pads),
        (GPIO, 0x20 | 0x24) => Effect::Refuse(0xfc00_0000, pads),
        (GPIO, 0x2c | 0x30) => Effect::Refuse(1, pads),
        (GPIO, 0x5bc..=0x5d4) => Effect::Pin(0xfff, 0x100, pads),
        _ => return None,
    })
}

impl Soc {
    pub(crate) fn new(clock: CpuClock, netted: &[u8], watchdogs: Watchdogs) -> Soc {
        let mut soc = Soc {
            clock,
            sram: PagedRam::new(0, SRAM),
            rtc_fast: PagedRam::new(RTC_FAST.start, (RTC_FAST.end - RTC_FAST.start) as usize),
            rtc_slow: PagedRam::new(RTC_SLOW.start, (RTC_SLOW.end - RTC_SLOW.start) as usize),
            rom: None,
            flash: None,
            plain: BTreeMap::new(),
            gpio: Gpio::new(netted),
            systimer: Systimer::new(),
            watchdogs,
            usb: UsbSerialJtag::default(),
            spi2: Spi2::new(),
            gdma: Gdma::new(),
            sha: Sha::new(),
            ledc: Ledc::new(),
            effects: Vec::new(),
            duties: Vec::new(),
            gates: Gates::new(),
            analog: Analog::default(),
            efuse: Efuse::default(),
            spi1: Spi1::default(),
            matrix: Matrix::default(),
            calibration: Default::default(),
            netted: netted.to_vec(),
            unmodeled: BTreeMap::new(),
            changes: Vec::new(),
            pending: None,
            in_flight: Vec::new(),
            changed: None,
            rearmed: false,
            reclocked: false,
            first_touch: None,
            change_stop: false,
            unmapped_fetch: false,
            started: false,
            ending: None,
            app: AppControl::default(),
            steer: None,
            latest: Tick(0),
        };
        soc.plain.insert(RTC_CNTL + OPTIONS0, 0);
        soc.plain.insert(RTC_CNTL + app::SW_CPU_STALL, 0);
        for (signal, level) in soc.spi2.idle() {
            soc.gpio.set_signal(signal, level);
        }
        soc.idle_ledc();
        soc
    }

    /// Puts each LEDC channel's signal at its power-on level, low.
    fn idle_ledc(&mut self) {
        for channel in 0..ledc::CHANNELS as u16 {
            self.gpio.set_signal(ledc::SIGNAL + channel, false);
        }
    }

    /// Lets every LEDC update due at or before `through` take effect, each with a `Duty` record
    /// on every netted pad that carries its channel.
    pub(crate) fn settle_ledc(&mut self, through: Tick) {
        self.ledc.settle(through, self.clock.source.apb());
        for effect in self.ledc.take_done() {
            for (pad, channel, inverted) in self.ledc_pads() {
                if channel == effect.channel {
                    self.duties.push(duty(effect, pad, inverted));
                }
            }
            self.effects.push(effect);
        }
    }

    /// Whether a netted pad carries an LEDC channel, or an update has yet to take effect or to
    /// drive its signal. A reset then would leave a pad's last `Duty` record stale, or let the
    /// update drive its signal after the reset.
    fn ledc_runs(&self) -> bool {
        self.ledc.pending() || !self.effects.is_empty() || !self.ledc_pads().is_empty()
    }

    /// Each netted pad that drives an LEDC channel's signal from the matrix, lowest first, with
    /// that channel and whether `OUT_INV_SEL` inverts it.
    fn ledc_pads(&self) -> Vec<(u8, u8, bool)> {
        let channels = ledc::SIGNAL..ledc::SIGNAL + ledc::CHANNELS as u16;
        (0..PADS as u8)
            .filter(|&pad| self.netted(pad))
            .filter_map(|pad| {
                let (signal, inverted) = self.gpio.carries(usize::from(pad))?;
                let channel = channels
                    .contains(&signal)
                    .then(|| (signal - ledc::SIGNAL) as u8)?;
                Some((pad, channel, inverted))
            })
            .collect()
    }

    /// Adds the ROM and the flash, as a chip booted from flash has them.
    pub(crate) fn attach_rom(&mut self, rom: Arc<[u8]>) {
        self.rom = Some(rom);
    }

    /// The PRO CPU's bus for one access outside a run, as a test makes it.
    #[cfg(test)]
    pub(crate) fn bus(&mut self) -> CoreBus<'_> {
        CoreBus {
            soc: self,
            step: Step {
                core: CoreId::Pro,
                start: Cycle(0),
                paired: false,
                other_sleeps: false,
            },
        }
    }

    /// The tick of core cycle `cycle` for a peripheral, never before the latest one given.
    fn stamp(&mut self, cycle: Cycle) -> Tick {
        let tick = self.clock.tick(cycle).max(self.latest);
        self.latest = tick;
        tick
    }

    /// `RTC_CNTL_OPTIONS0_REG` and `RTC_CNTL_SW_CPU_STALL_REG`, which hold both cores' RTC
    /// stall fields.
    fn stall_words(&self) -> (u32, u32) {
        let word = |offset| self.plain.get(&(RTC_CNTL + offset)).copied().unwrap_or(0);
        (word(OPTIONS0), word(app::SW_CPU_STALL))
    }

    /// What holds the APP CPU now.
    fn hold(&self) -> Hold {
        let (options0, sw_cpu_stall) = self.stall_words();
        Hold::new(self.app.control, options0, sw_cpu_stall)
    }

    /// Whether the APP CPU may run.
    pub(crate) fn app_runs(&self) -> bool {
        self.hold().runs()
    }

    /// Keeps `steer` unless the step already has a stronger one.
    fn steer(&mut self, steer: Steer) {
        if self.steer.is_none_or(|kept| kept.rank() < steer.rank()) {
            self.steer = Some(steer);
        }
    }

    /// The byte at `addr` in the GDMA's SRAM window, with its SRAM offset added to `read`.
    fn dma_byte(sram: &PagedRam, read: &mut Vec<Range<u32>>, addr: u32) -> u8 {
        let offset = Self::sram_offset(addr, false).expect("in the DMA window");
        match read.last_mut() {
            Some(last) if last.end == offset => last.end += 1,
            _ => read.push(offset..offset + 1),
        }
        sram.read(offset, Width::Byte) as u8
    }

    /// The `bytes` bytes GDMA's channel started on `peripheral` sends, with the SRAM offsets of
    /// the bytes it read added to `read`. While GDMA's clock is off no channel feeds it.
    fn feed(
        gdma: &mut Gdma,
        sram: &PagedRam,
        gates: &Gates,
        peripheral: u32,
        bytes: usize,
        read: &mut Vec<Range<u32>>,
    ) -> Result<Vec<u8>, Setting> {
        if !gates.clocked(Block::Gdma) {
            return Err(Setting::DmaLink);
        }
        gdma.send(peripheral, bytes, |addr| Self::dma_byte(sram, read, addr))
    }

    /// Whether a `width` store at SRAM offset `offset` reaches a descriptor or a buffer that a
    /// running SPI2 transfer sends from.
    fn in_flight(&mut self, now: Tick, offset: u32, width: Width) -> bool {
        if !self.spi2.busy(now) {
            self.in_flight.clear();
            return false;
        }
        let end = offset + width.bytes();
        self.in_flight
            .iter()
            .any(|range| offset < range.end && range.start < end)
    }

    pub(crate) fn sram_offset(addr: u32, fetch: bool) -> Option<u32> {
        RAM.iter()
            .filter(|w| w.code || !fetch)
            .find(|w| (w.start..w.end).contains(&addr))
            .map(|w| addr - w.start + w.offset)
    }

    /// Writes `bytes` from `addr` into SRAM or RTC memory, as a loader does. Every byte lies in
    /// one of them.
    pub(crate) fn poke(&mut self, addr: u32, bytes: &[u8]) {
        if let Some(offset) = Self::sram_offset(addr, false) {
            self.sram.write_bytes(offset, bytes);
        } else if RTC_FAST.contains(&addr) {
            self.rtc_fast.write_bytes(addr, bytes);
        } else if RTC_SLOW.contains(&addr) {
            self.rtc_slow.write_bytes(addr, bytes);
        } else if MMU_TABLE.contains(&addr) {
            for (k, word) in bytes.chunks(4).enumerate() {
                let word = u32::from_le_bytes(word.try_into().expect("whole words"));
                let flash = self.flash.as_mut().expect("a chip with flash");
                flash.write_mmu(addr - MMU_TABLE.start + 4 * k as u32, word);
            }
        } else {
            panic!("{} bytes at {addr:#x} lie in no RAM", bytes.len());
        }
    }

    /// Keeps the clock registers of a chip no captured state gives, at the undivided crystal the
    /// clock starts on: `SYSTEM_SOC_CLK_SEL` and `SYSTEM_PRE_DIV_CNT` 0, and
    /// `SYSTEM_CPU_PER_CONF_REG` at its reset value: the 480 MHz PLL, `SYSTEM_CPUPERIOD_SEL` 0 and
    /// `SYSTEM_CPU_WAIT_MODE_FORCE_ON`.
    pub(crate) fn keep_crystal_clock(&mut self) {
        debug_assert_eq!(self.clock.source, Source::Xtal(1));
        self.plain.insert(SYSTEM + SYSCLK_CONF, 0);
        self.plain.insert(SYSTEM + CPU_PER_CONF, 0xc);
    }

    /// Keeps the register at `addr` with `value`, as the boot left it: in the model that owns
    /// it, or as a plain word.
    pub(crate) fn restore(&mut self, addr: u32, value: u32) {
        let (base, offset) = (addr & !(BLOCK - 1), addr & (BLOCK - 1));
        let now = Tick(0);
        match base {
            TIMG0 | TIMG1 if (0x48..=0x64).contains(&offset) || (0x70..=0x7c).contains(&offset) => {
                let group = usize::from(base == TIMG1);
                self.watchdogs
                    .restore(now, Register::Timg(group, offset), value);
            }
            RTC_CNTL if (0x98..=0xb8).contains(&offset) => {
                self.watchdogs.restore(now, Register::Rtc(offset), value);
            }
            SYSTIMER => self.systimer.restore(now, offset, value),
            TIMG0 | TIMG1 if calibration::OFFSETS.contains(&offset) => {
                self.calibration[usize::from(base == TIMG1)].write(offset, value);
            }
            SPI1 => {
                self.spi1
                    .write(offset, value, &[])
                    .expect("a register, not a command");
            }
            INTERRUPT_CORE0 if self.matrix.write(offset, value) => {}
            SYSTEM if matrix::FROM_CPU.contains(&offset) => {
                self.matrix.write_from_cpu(offset, value);
            }
            SYSTEM if offset == app::CONTROL_0 => self.app.control = value,
            SYSTEM if offset == app::CONTROL_1 => self.app.boot = value,
            SYSTEM if gates::OFFSETS.contains(&offset) => {
                self.gates.write(offset, value).for_each(drop);
            }
            _ => {
                self.plain.insert(addr, value);
            }
        }
    }

    /// The SRAM offset a `width` load or store at `addr` reaches, if any. The core checks the
    /// instruction bus first, and the bus agrees.
    fn data_offset(addr: u32, width: Width) -> Result<Option<u32>, Exit> {
        Self::instruction_bus(addr, width)?;
        Ok(Self::sram_offset(addr, false))
    }

    /// Through instruction RAM and the ROM only an aligned word goes.
    fn instruction_bus(addr: u32, width: Width) -> Result<(), Exit> {
        if sim_xtensa::Esp32s3::instruction_bus(addr) {
            if !addr.is_multiple_of(width.bytes()) {
                return Err(Exit::Fault(Cause::LoadStoreAlignment));
            }
            if width != Width::Word {
                return Err(Exit::Fault(Cause::LoadStoreError));
            }
        }
        Ok(())
    }

    /// The ROM's word at `addr`, a multiple of 4 in either ROM window.
    fn rom_word(&self, addr: u32) -> Option<u32> {
        let rom = self.rom.as_ref()?;
        let at = if rom::ROM.contains(&addr) {
            addr - rom::ROM.start
        } else if rom::DROM.contains(&addr) {
            addr - rom::DROM.start + rom::DROM_AT - rom::ROM.start
        } else {
            return None;
        } as usize;
        let b = &rom[at..at + 4];
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// The word at `addr`, a multiple of 4, in a memory that is not SRAM.
    fn memory_word(&self, addr: u32) -> Option<u32> {
        if RTC_FAST.contains(&addr) {
            return Some(self.rtc_fast.read(addr, Width::Word));
        }
        if RTC_SLOW.contains(&addr) {
            return Some(self.rtc_slow.read(addr, Width::Word));
        }
        if let Some(word) = self.rom_word(addr) {
            return Some(word);
        }
        if flash::IROM.contains(&addr) || flash::DROM.contains(&addr) {
            return self.flash.as_ref()?.word(addr);
        }
        None
    }

    /// The kept register's word at `addr`, a multiple of 4 in the peripherals, at `now`.
    fn read_kept(&mut self, now: Tick, addr: u32) -> Option<u32> {
        let (base, offset) = (addr & !(BLOCK - 1), addr & (BLOCK - 1));
        let value = match base {
            GPIO => self.gpio.read(offset),
            IO_MUX => self.gpio.read_io_mux(offset),
            RTC_CNTL => self.watchdogs.read_rtc(offset),
            TIMG0 | TIMG1 if calibration::OFFSETS.contains(&offset) => {
                self.calibration[usize::from(base == TIMG1)].read(offset)
            }
            TIMG0 => self.watchdogs.read_timg(0, offset),
            TIMG1 => self.watchdogs.read_timg(1, offset),
            SYSTIMER => self.systimer.read(now, offset),
            USB_SERIAL_JTAG => self.usb.read(now, offset),
            SPI2 => self.spi2.read(now, offset),
            GDMA => self.gdma.read(offset),
            SHA => self.sha.read(offset),
            LEDC => self.ledc.read(offset),
            SYSTEM if gates::OFFSETS.contains(&offset) => Some(self.gates.read(offset)),
            ANALOG => self.analog.read(offset),
            EFUSE => self.efuse.read(offset),
            SPI1 => self.spi1.read(offset),
            SYSTEM if matrix::FROM_CPU.contains(&offset) => Some(self.matrix.read_from_cpu(offset)),
            SYSTEM if offset == app::CONTROL_0 => Some(self.app.control),
            SYSTEM if offset == app::CONTROL_1 => Some(self.app.boot),
            INTERRUPT_CORE0 => {
                let sources = self.sources(now);
                self.matrix.read(offset, sources)
            }
            _ if MMU_TABLE.contains(&addr) => self.flash.as_ref()?.read_mmu(addr - MMU_TABLE.start),
            _ => None,
        };
        value
            .or_else(|| self.plain.get(&addr).copied())
            .or_else(|| (base == SENSITIVE).then_some(0))
    }

    fn write_kept(&mut self, now: Tick, addr: u32, value: u32) -> bool {
        let (base, offset) = (addr & !(BLOCK - 1), addr & (BLOCK - 1));
        let done = match base {
            GPIO => self.gpio.write(offset, value),
            IO_MUX => self.gpio.write_io_mux(offset, value),
            RTC_CNTL => self
                .watchdogs
                .write_rtc(now, offset, value)
                .unwrap_or_else(|setting| {
                    self.ending = Some(Ending::Refused(Refused::Register("RTC_CNTL", setting)));
                    true
                }),
            TIMG0 | TIMG1 if calibration::OFFSETS.contains(&offset) => {
                self.calibration[usize::from(base == TIMG1)].write(offset, value)
            }
            TIMG0 | TIMG1 => {
                let group = usize::from(base == TIMG1);
                self.watchdogs
                    .write_timg(now, group, offset, value)
                    .unwrap_or_else(|setting| {
                        let block = if group == 0 { "TIMG0" } else { "TIMG1" };
                        self.ending = Some(Ending::Refused(Refused::Register(block, setting)));
                        true
                    })
            }
            SYSTIMER => self.systimer.write(now, offset, value),
            USB_SERIAL_JTAG => self.usb.write(now, offset, value),
            ANALOG => self.analog.write(offset, value),
            GDMA => self.gdma.write(offset, value).unwrap_or_else(|setting| {
                self.ending = Some(Ending::Refused(Refused::Register("GDMA", setting)));
                true
            }),
            SHA => {
                let (gdma, sram, gates) = (&mut self.gdma, &self.sram, &self.gates);
                let mut read = Vec::new();
                let dma = |bytes| Self::feed(gdma, sram, gates, sha::PERIPHERAL, bytes, &mut read);
                self.sha
                    .write(offset, value, dma)
                    .unwrap_or_else(|setting| {
                        self.ending = Some(Ending::Refused(Refused::Register("SHA", setting)));
                        true
                    })
            }
            SYSTEM if matrix::FROM_CPU.contains(&offset) => {
                self.matrix.write_from_cpu(offset, value);
                true
            }
            SYSTEM if offset == app::CONTROL_0 => {
                self.app.control = value & 0x7;
                true
            }
            SYSTEM if offset == app::CONTROL_1 => {
                self.app.boot = value;
                true
            }
            SYSTEM if gates::OFFSETS.contains(&offset) => {
                let mut gates = self.gates.clone();
                let held: Vec<Block> = gates.write(offset, value).collect();
                let sending = self.spi2.busy(now);
                let dma = sending && !self.in_flight.is_empty();
                let refused = [
                    (
                        sending && held.contains(&Block::Spi2),
                        "SPI2",
                        "reset_while_sending",
                    ),
                    (
                        sending && !gates.clocked(Block::Spi2),
                        "SPI2",
                        "clock_off_while_sending",
                    ),
                    (
                        dma && held.contains(&Block::Gdma),
                        "GDMA",
                        "reset_while_sending",
                    ),
                    (
                        dma && !gates.clocked(Block::Gdma),
                        "GDMA",
                        "clock_off_while_sending",
                    ),
                    (
                        self.ledc.counts() && !gates.clocked(Block::Ledc),
                        "LEDC",
                        "clock_off_while_running",
                    ),
                    (
                        held.contains(&Block::Ledc)
                            && !self.gates.held(Block::Ledc)
                            && self.ledc_runs(),
                        "LEDC",
                        "reset_while_running",
                    ),
                ]
                .into_iter()
                .find(|&(refused, ..)| refused);
                if let Some((_, block, setting)) = refused {
                    self.ending = Some(Ending::Refused(Refused::Register(block, setting)));
                    return true;
                }
                self.gates = gates;
                for block in held {
                    self.reset(block);
                }
                true
            }
            LEDC => {
                let apb = self.clock.source.apb();
                self.ledc
                    .write(now, offset, value, apb)
                    .unwrap_or_else(|setting| {
                        self.ending = Some(Ending::Refused(Refused::Register("LEDC", setting)));
                        true
                    })
            }
            INTERRUPT_CORE0 => self.matrix.write(offset, value),
            EFUSE => self.efuse.write(offset, value).unwrap_or_else(|()| {
                self.ending = Some(Ending::EfuseProgram);
                true
            }),
            SPI1 => {
                let flash = self.flash.as_ref().map_or(&[][..], |flash| flash.bytes());
                self.spi1
                    .write(offset, value, flash)
                    .unwrap_or_else(|refusal| {
                        self.ending = Some(Ending::FlashCommand(refusal));
                        true
                    })
            }
            SPI2 => {
                let cpu = self.clock.source.cpu();
                let (gdma, sram, gates) = (&mut self.gdma, &self.sram, &self.gates);
                let mut read = Vec::new();
                let dma = |bytes| Self::feed(gdma, sram, gates, gdma::SPI2, bytes, &mut read);
                match self.spi2.write(now, cpu, offset, value, &self.gpio, dma) {
                    Ok(started) => {
                        self.started |= started;
                        if started {
                            self.in_flight = read;
                        }
                    }
                    Err(setting) => self.ending = Some(Ending::Setting(setting)),
                }
                for (signal, level) in self.spi2.idle() {
                    self.gpio.set_signal(signal, level);
                }
                true
            }
            _ if MMU_TABLE.contains(&addr) => self
                .flash
                .as_mut()
                .is_some_and(|flash| flash.write_mmu(addr - MMU_TABLE.start, value)),
            _ => false,
        };
        let value = if base == EXTMEM {
            extmem::written(offset, value)
        } else {
            value
        };
        if base == SENSITIVE {
            self.plain.insert(addr, value);
            return true;
        }
        done || self
            .plain
            .get_mut(&addr)
            .map(|word| *word = value)
            .is_some()
    }

    /// The interrupt sources' levels at `now`.
    fn sources(&mut self, now: Tick) -> u128 {
        self.systimer.settle(now);
        let spi2 = self.spi2.line(now);
        self.matrix
            .sources(self.systimer.lines(), spi2, self.watchdogs.lines())
    }

    /// Runs the CPU from the clock the SYSTEM registers now pick, from core cycle `at` on, which
    /// lies at tick `tick`.
    fn switch_clock(&mut self, at: Cycle, tick: Tick) {
        let register = |offset| self.plain.get(&(SYSTEM + offset)).copied().unwrap_or(0);
        match Source::of(register(SYSCLK_CONF), register(CPU_PER_CONF)) {
            None => self.ending = Some(Ending::Clock),
            Some(source) if source.apb() != self.clock.source.apb() && self.ledc.counts_apb() => {
                let refused = Refused::Register("LEDC", "clock_change_while_running");
                self.ending = Some(Ending::Refused(refused));
            }
            Some(source) if source != self.clock.source => {
                self.clock.switch(at, source);
                self.watchdogs.reclock(tick, source.apb());
                self.reclocked = true;
            }
            Some(_) => {}
        }
    }

    /// Lets an access to an unmodeled register through, or stops the core at the first one.
    /// `kept` marks a write the register keeps while its effect does not run.
    fn touch(&mut self, now: Tick, addr: u32, write: bool, kept: bool) -> Result<(), Exit> {
        if self.unmodeled.contains_key(&addr) {
            return Ok(());
        }
        let touch = Touch {
            at: now,
            pc: Pc {
                core: CoreId::Pro,
                pc: 0,
            },
            write,
            kept,
        };
        self.unmodeled.insert(addr, touch);
        self.first_touch = Some(addr);
        Err(Exit::Stop)
    }

    /// Whether the interrupt matrix routes a source to `core`'s NMI line.
    pub(crate) fn routes_to_nmi(&self, core: CoreId) -> bool {
        self.matrix.routes_to(core, matrix::NMI)
    }

    /// Stops the core at the write of `value` to the kept register at `addr`, whose effect the
    /// model does not run, so the timeline can take its PC. When the core runs the store again,
    /// the touch to list once it lands.
    fn report(&mut self, now: Tick, addr: u32, value: u32) -> Result<Option<Touch>, Exit> {
        if let Some((.., touch)) = self
            .pending
            .take_if(|&mut (at, word, _)| (at, word) == (addr, value))
        {
            return Ok(Some(touch));
        }
        let touch = Touch {
            at: now,
            pc: Pc {
                core: CoreId::Pro,
                pc: 0,
            },
            write: true,
            kept: true,
        };
        self.pending = Some((addr, value, touch));
        self.change_stop = true;
        Err(Exit::Stop)
    }

    /// Puts `block` back to its power-on state, as its reset bit does. SPI2's transfers already
    /// sent stay to be taken, and its pads go to its idle levels.
    fn reset(&mut self, block: Block) {
        match block {
            Block::Spi2 => {
                self.spi2.reset();
                for (signal, level) in self.spi2.idle() {
                    self.gpio.set_signal(signal, level);
                }
            }
            Block::Ledc => {
                self.ledc = Ledc::new();
                self.idle_ledc();
            }
            Block::Gdma => self.gdma = Gdma::new(),
            Block::Sha => self.sha = Sha::new(),
        }
    }

    /// The block whose registers hold `addr` and why an access to them ends the run: its
    /// clock is off, or a reset bit holds it.
    fn gated(&self, addr: u32) -> Option<Refused> {
        let block = match addr & !(BLOCK - 1) {
            SPI2 => Block::Spi2,
            LEDC => Block::Ledc,
            GDMA => Block::Gdma,
            SHA => Block::Sha,
            _ => return None,
        };
        let why = if !self.gates.clocked(block) {
            "clock_off"
        } else if self.gates.held(block) {
            "held_in_reset"
        } else {
            return None;
        };
        Some(Refused::Register(block.name(), why))
    }

    /// Whether pad `gpio` has a net.
    pub(crate) fn netted(&self, gpio: u8) -> bool {
        self.netted.contains(&gpio)
    }

    /// Puts GPIO and IO_MUX back to power-on, as a reset does.
    pub(crate) fn reset_pads(&mut self) {
        self.gpio = Gpio::new(&self.netted);
    }

    /// The drive of each netted pad, `Off` for the others.
    pub(crate) fn image(&self) -> [PadDrive; PADS] {
        let mut image = [PadDrive::Off; PADS];
        for &gpio in &self.netted {
            image[usize::from(gpio)] = self.gpio.drive(usize::from(gpio));
        }
        image
    }
}

/// `effect`'s `Duty` record on `pad`, which sends every step the channel does not when
/// `inverted`, since the pad drives the inverted level.
fn duty(effect: ledc::Effect, pad: u8, inverted: bool) -> Duty {
    let steps = 1 << effect.resolution;
    Duty {
        at: effect.at,
        pad,
        duty: if inverted {
            steps - effect.duty
        } else {
            effect.duty
        },
        resolution: effect.resolution,
        hz: effect.hz,
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

impl Soc {
    fn fetch(&mut self, addr: u32) -> Result<u32, Exit> {
        if self.changed.is_some() || self.rearmed || self.reclocked || self.started {
            return Err(Exit::Stop);
        }
        if let Some(offset) = Self::sram_offset(addr, true) {
            return Ok(self.sram.read(offset, Width::Word));
        }
        let word = if flash::IROM.contains(&addr) || rom::ROM.contains(&addr) {
            self.memory_word(addr)
        } else if RTC_FAST.contains(&addr) {
            Some(self.rtc_fast.read(addr, Width::Word))
        } else {
            None
        };
        word.ok_or_else(|| {
            self.unmapped_fetch = true;
            Exit::Stop
        })
    }

    fn load(&mut self, cycle: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        if let Some(offset) = Self::data_offset(addr, width)? {
            return Ok(self.sram.read(offset, width));
        }
        let (word, shift, mask) = word_of(addr, width);
        if let Some(value) = self.memory_word(word) {
            return Ok((value >> shift) & mask);
        }
        if flash::IROM.contains(&addr) || flash::DROM.contains(&addr) {
            self.ending = Some(Ending::UnmappedLoad(addr));
            return Err(Exit::Stop);
        }
        if !PERIPHERALS.contains(&addr) {
            return Err(Exit::Fault(Cause::LoadStoreError));
        }
        if let Some(refused) = self.gated(word) {
            self.ending = Some(Ending::Refused(refused));
            return Err(Exit::Stop);
        }
        let now = self.stamp(cycle);
        let kept = self.read_kept(now, word);
        if word == USB_SERIAL_JTAG {
            self.usb.take_received(now);
        }
        match kept {
            Some(value) => Ok((value >> shift) & mask),
            None => self.touch(now, word, false, false).map(|()| 0),
        }
    }

    fn store(
        &mut self,
        step: &Step,
        cycle: Cycle,
        addr: u32,
        width: Width,
        value: u32,
    ) -> Result<(), Exit> {
        if let Some(offset) = Self::data_offset(addr, width)? {
            if !self.in_flight.is_empty() {
                let now = self.stamp(cycle);
                if self.in_flight(now, offset, width) {
                    let refused = Refused::Register("GDMA", "buffer_write");
                    self.ending = Some(Ending::Refused(refused));
                    return Err(Exit::Stop);
                }
            }
            self.sram.write(offset, width, value);
            return Ok(());
        }
        if RTC_FAST.contains(&addr) {
            self.rtc_fast.write(addr, width, value);
            return Ok(());
        }
        if RTC_SLOW.contains(&addr) {
            self.rtc_slow.write(addr, width, value);
            return Ok(());
        }
        if !PERIPHERALS.contains(&addr) {
            return Err(Exit::Fault(Cause::LoadStoreError));
        }
        self.store_register(step, cycle, addr, width, value)?;
        if step.other_sleeps {
            self.steer(Steer::Wake(step.start));
        }
        Ok(())
    }

    /// A store to the peripheral register at `addr`.
    fn store_register(
        &mut self,
        step: &Step,
        cycle: Cycle,
        addr: u32,
        width: Width,
        value: u32,
    ) -> Result<(), Exit> {
        let (word, shift, mask) = word_of(addr, width);
        if let Some(refused) = self.gated(word) {
            self.ending = Some(Ending::Refused(refused));
            return Err(Exit::Stop);
        }
        let clock = word == SYSTEM + SYSCLK_CONF || word == SYSTEM + CPU_PER_CONF;
        // The other core's refetched access on a tie can have stamped the cycle after `start`,
        // and a switch at `start` would hold the reported tick until the new clock caught up.
        let at = if clock && step.paired {
            step.start.max(self.clock.cycle_after(self.latest))
        } else {
            cycle
        };
        let now = self.stamp(at);
        if word == RTC_CNTL + OPTIONS0 && ((value & mask) << shift) & RESETS != 0 {
            let store6 = self.read_kept(now, RTC_CNTL + STORE6);
            self.ending = Some(Ending::Restart(reset_cause(store6)));
            return Err(Exit::Stop);
        }
        let kept = self.read_kept(now, word);
        let value = (kept.unwrap_or(0) & !(mask << shift)) | ((value & mask) << shift);
        let change = match effect(word) {
            Some(Effect::Refuse(bits, ending)) if value & bits != 0 => {
                self.ending = Some(ending);
                return Err(Exit::Stop);
            }
            Some(Effect::Pin(bits, held, ending)) if value & bits != held => {
                self.ending = Some(ending);
                return Err(Exit::Stop);
            }
            Some(Effect::Report) if kept != Some(value) => self.report(now, word, value)?,
            Some(Effect::Gates) if self.gates.reports(word - SYSTEM, value) => {
                self.report(now, word, value)?
            }
            _ => None,
        };
        if kept.is_none() {
            return self.touch(now, word, true, false);
        }
        let pads = matches!(word & !(BLOCK - 1), GPIO | IO_MUX | SPI2)
            || (word & !(BLOCK - 1) == SYSTEM && gates::OFFSETS.contains(&(word - SYSTEM)));
        if !pads {
            let deadline = self.watchdogs.deadline();
            let app = [
                SYSTEM + app::CONTROL_0,
                RTC_CNTL + OPTIONS0,
                RTC_CNTL + app::SW_CPU_STALL,
            ]
            .contains(&word)
            .then(|| self.hold());
            let reset = word == RTC_CNTL + OPTIONS0 && value & app::APPCPU_RST != 0;
            let kept = if reset {
                value & !app::APPCPU_RST
            } else {
                value
            };
            let due = self.ledc.next(self.clock.source.apb());
            self.write_kept(now, word, kept);
            if clock {
                self.switch_clock(at, now);
            }
            if let Some(before) = app {
                self.steer_app(step, before, reset);
            }
            if self.ending.is_some() {
                return Err(Exit::Stop);
            }
            self.changes.extend(change.map(|touch| (word, touch)));
            self.rearmed |= self.watchdogs.deadline() != deadline
                || self.ledc.next(self.clock.source.apb()) != due;
            return Ok(());
        }
        self.settle_ledc(now);
        let before = self.image();
        let carried = self.ledc_pads();
        self.write_kept(now, word, value);
        if self.ending.is_some() {
            return Err(Exit::Stop);
        }
        self.changes.extend(change.map(|touch| (word, touch)));
        if self.image() != before {
            self.changed = Some(now);
        }
        for (pad, channel, inverted) in self.ledc_pads() {
            if let Some(effect) = self.ledc.sent(usize::from(channel))
                && !carried.contains(&(pad, channel, inverted))
            {
                self.duties
                    .push(duty(ledc::Effect { at: now, ..effect }, pad, inverted));
                self.rearmed = true;
            }
        }
        Ok(())
    }

    /// Steers the run after a store took the APP CPU's hold from `before`, with `reset` a write
    /// of `RTC_CNTL_SW_APPCPU_RST`. A store that would let an APP CPU never reset run ends the
    /// run, as the model cannot know its state, and so does one that would stall the PRO CPU,
    /// which the model does not do.
    fn steer_app(&mut self, step: &Step, before: Hold, reset: bool) {
        let (options0, sw_cpu_stall) = self.stall_words();
        if app::pro_stalled(options0, sw_cpu_stall) {
            let refused = Refused::Register("RTC_CNTL", "pro_cpu_stall");
            self.ending = Some(Ending::Refused(refused));
            return;
        }
        let after = self.hold();
        let Some(steer) = Steer::of_store(before, after, reset, step.start) else {
            return;
        };
        if !step.paired && matches!(steer, Steer::Run { .. }) && after.runs() {
            let refused = Refused::Register("SYSTEM", "app_cpu_without_reset");
            self.ending = Some(Ending::Refused(refused));
            return;
        }
        self.steer(steer);
    }

    fn interrupts(&mut self, core: CoreId, cycle: Cycle) -> u32 {
        let now = self.stamp(cycle);
        let sources = self.sources(now);
        self.matrix.lines(core, sources)
    }

    fn next_event(&mut self, cycle: Cycle) -> Option<Cycle> {
        let now = self.stamp(cycle);
        let next = [self.systimer.next_event(now), self.spi2.next_event()]
            .into_iter()
            .flatten()
            .min()?;
        Some(self.clock.cycle_after(next))
    }
}

impl Bus for CoreBus<'_> {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        self.soc.fetch(addr)
    }

    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        self.soc.load(now, addr, width)
    }

    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        self.soc.store(&self.step, now, addr, width, value)
    }

    /// The poll that opens every step, so it marks the step's start.
    fn interrupts(&mut self, now: Cycle) -> u32 {
        self.step.start = now;
        self.soc.interrupts(self.step.core, now)
    }

    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        self.soc.next_event(now)
    }

    fn yields(&self) -> bool {
        self.soc.steer.is_some()
    }
}
