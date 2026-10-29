//! What the core sees: its memories and the peripherals, on the chip's tick timeline.
//!
//! Every chip has SRAM, both RTC memories and the peripherals. A chip booted from flash also has
//! the mask ROM and the flash through the MMU. Its registers that the captured state
//! names and no model owns are kept as plain words.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use sim_xtensa::{Bus, Cause, Config, Cycle, Exit, PagedRam, Width};

use super::analog::Analog;
use super::calibration::{self, Calibration};
use super::clock::{CpuClock, Source, Tick};
use super::efuse::Efuse;
use super::extmem;
use super::flash::{self, Flash};
use super::gpio::{self, Gpio, PADS, PadDrive};
use super::matrix::{self, Matrix};
use super::rom;
use super::spi1::{self, FlashRefusal, Spi1};
use super::spi2::{Setting, Spi2};
use super::timers::{Register, Systimer, Watchdogs};
use super::usb::UsbSerialJtag;
use super::{RAM, Refused, ResetCause, Touch};

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
pub(crate) const USB_SERIAL_JTAG: u32 = 0x6003_8000;
/// GDMA, which the model does not run.
const GDMA: u32 = 0x6003_f000;
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
    analog: Analog,
    efuse: Efuse,
    spi1: Spi1,
    matrix: Matrix,
    /// Each timer group's RTC clock calibration.
    calibration: [Calibration; 2],
    /// The GPIOs with nets, whose drives the image holds.
    netted: Vec<u8>,
    pub(crate) unmodeled: BTreeMap<u32, Touch>,
    /// A store changed a netted pad's drive at this tick, so the next fetch stops the core.
    pub(crate) changed: Option<Tick>,
    /// A store changed the CPU clock, so the next fetch stops the core and the run maps its
    /// limits to cycles again.
    pub(crate) reclocked: bool,
    /// A store moved a watchdog's deadline, so the next fetch stops the core and the run reads
    /// the new one.
    pub(crate) rearmed: bool,
    /// The first access to an unmodeled register stopped the core, so the timeline can take the
    /// access's PC.
    pub(crate) first_touch: Option<u32>,
    /// The core fetched outside its memories.
    pub(crate) unmapped_fetch: bool,
    /// A store started an SPI2 transfer, so the next fetch stops the core and the run reads its
    /// end.
    pub(crate) started: bool,
    /// A store ended the run, and stopped the core.
    pub(crate) ending: Option<Ending>,
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

/// `RTC_CNTL_OPTIONS0_REG`, and its bits that reset the core and the whole chip. The bit that
/// resets the second core resets nothing here, as this chip runs one.
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
    /// The write is listed as an unmodeled register when it changes the word, and then kept.
    Report,
}

/// The effect of a write of the register at `addr`, when it has one the model does not run.
fn effect(addr: u32) -> Option<Effect> {
    let (base, offset) = (addr & !(BLOCK - 1), addr & (BLOCK - 1));
    let interrupt = |peripheral| Ending::Refused(Refused::Interrupt(peripheral));
    let pads = Ending::FlashCommand(FlashRefusal::Pads);
    Some(match (base, offset) {
        (RTC_CNTL, 0x18) => Effect::Refuse(1 << 31, Ending::Refused(Refused::Sleep)),
        (RTC_CNTL, 0x74) => Effect::Report,
        (SPI2, 0x34) => Effect::Refuse(u32::MAX, interrupt("SPI2")),
        (TIMG0, 0x70) => Effect::Refuse(0b11, interrupt("TIMG0")),
        (TIMG1, 0x70) => Effect::Refuse(0b11, interrupt("TIMG1")),
        (USB_SERIAL_JTAG, 0x10) => Effect::Refuse(u32::MAX, interrupt("USB_SERIAL_JTAG")),
        (GDMA, _) if offset < 5 * 0xc0 && matches!(offset % 0xc0, 0x10 | 0x70) => {
            Effect::Refuse(u32::MAX, interrupt("GDMA"))
        }
        (SPI0, 0x0) => Effect::Refuse(spi1::STARTS, Ending::FlashCommand(FlashRefusal::Spi0)),
        (SPI0, 0x18..=0x20) => Effect::Report,
        (SYSTEM, 0x18..=0x24) => Effect::Report,
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
            analog: Analog::default(),
            efuse: Efuse::default(),
            spi1: Spi1::default(),
            matrix: Matrix::default(),
            calibration: Default::default(),
            netted: netted.to_vec(),
            unmodeled: BTreeMap::new(),
            changed: None,
            rearmed: false,
            reclocked: false,
            first_touch: None,
            unmapped_fetch: false,
            started: false,
            ending: None,
        };
        for (signal, level) in soc.spi2.idle() {
            soc.gpio.set_signal(signal, level);
        }
        soc
    }

    /// Adds the ROM and the flash, as a chip booted from flash has them.
    pub(crate) fn attach(&mut self, rom: Arc<[u8]>, flash: Flash) {
        self.rom = Some(rom);
        self.flash = Some(flash);
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
            TIMG0 | TIMG1 if (0x48..=0x64).contains(&offset) => {
                let group = usize::from(base == TIMG1);
                self.watchdogs
                    .restore(now, Register::Timg(group, offset), value);
            }
            RTC_CNTL if (0x98..=0xb8).contains(&offset) => match offset {
                0xb4 | 0xb8 => {
                    self.watchdogs.write_rtc(now, offset, value);
                }
                _ => self.watchdogs.restore(now, Register::Rtc(offset), value),
            },
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
            USB_SERIAL_JTAG => self.usb.read(offset),
            SPI2 => self.spi2.read(now, offset),
            ANALOG => self.analog.read(offset),
            EFUSE => self.efuse.read(offset),
            SPI1 => self.spi1.read(offset),
            SYSTEM if matrix::FROM_CPU.contains(&offset) => Some(self.matrix.read_from_cpu(offset)),
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
            RTC_CNTL => self.watchdogs.write_rtc(now, offset, value),
            TIMG0 | TIMG1 if calibration::OFFSETS.contains(&offset) => {
                self.calibration[usize::from(base == TIMG1)].write(offset, value)
            }
            TIMG0 => self.watchdogs.write_timg(now, 0, offset, value),
            TIMG1 => self.watchdogs.write_timg(now, 1, offset, value),
            SYSTIMER => self.systimer.write(now, offset, value),
            USB_SERIAL_JTAG => self.usb.write(now, offset, value),
            ANALOG => self.analog.write(offset, value),
            SYSTEM if matrix::FROM_CPU.contains(&offset) => {
                self.matrix.write_from_cpu(offset, value);
                true
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
                match self.spi2.write(now, cpu, offset, value, &self.gpio) {
                    Ok(started) => self.started |= started,
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
        self.matrix.sources(self.systimer.lines())
    }

    /// Runs the CPU from the clock the SYSTEM registers now pick, from core cycle `now` on.
    fn switch_clock(&mut self, now: Cycle) {
        let register = |offset| self.plain.get(&(SYSTEM + offset)).copied().unwrap_or(0);
        match Source::of(register(SYSCLK_CONF), register(CPU_PER_CONF)) {
            None => self.ending = Some(Ending::Clock),
            Some(source) if source != self.clock.source => {
                let tick = self.clock.tick(now);
                self.clock.switch(now, source);
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
            pc: 0,
            write,
            kept,
        };
        self.unmodeled.insert(addr, touch);
        self.first_touch = Some(addr);
        Err(Exit::Stop)
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

    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        let now = self.clock.tick(now);
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
        match self.read_kept(now, word) {
            Some(value) => Ok((value >> shift) & mask),
            None => self.touch(now, word, false, false).map(|()| 0),
        }
    }

    fn store(&mut self, cycle: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        let now = self.clock.tick(cycle);
        if let Some(offset) = Self::data_offset(addr, width)? {
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
        let (word, shift, mask) = word_of(addr, width);
        if word == RTC_CNTL + OPTIONS0 && ((value & mask) << shift) & RESETS != 0 {
            let store6 = self.read_kept(now, RTC_CNTL + STORE6);
            self.ending = Some(Ending::Restart(reset_cause(store6)));
            return Err(Exit::Stop);
        }
        let kept = self.read_kept(now, word);
        let value = (kept.unwrap_or(0) & !(mask << shift)) | ((value & mask) << shift);
        match effect(word) {
            Some(Effect::Refuse(bits, ending)) if value & bits != 0 => {
                self.ending = Some(ending);
                return Err(Exit::Stop);
            }
            Some(Effect::Pin(bits, held, ending)) if value & bits != held => {
                self.ending = Some(ending);
                return Err(Exit::Stop);
            }
            Some(Effect::Report) if kept != Some(value) => self.touch(now, word, true, true)?,
            _ => {}
        }
        if kept.is_none() {
            return self.touch(now, word, true, false);
        }
        if !matches!(word & !(BLOCK - 1), GPIO | IO_MUX | SPI2) {
            let deadline = self.watchdogs.deadline();
            self.write_kept(now, word, value);
            if word == SYSTEM + SYSCLK_CONF || word == SYSTEM + CPU_PER_CONF {
                self.switch_clock(cycle);
            }
            if self.ending.is_some() {
                return Err(Exit::Stop);
            }
            self.rearmed |= self.watchdogs.deadline() != deadline;
            return Ok(());
        }
        let before = self.image();
        self.write_kept(now, word, value);
        if self.ending.is_some() {
            return Err(Exit::Stop);
        }
        if self.image() != before {
            self.changed = Some(now);
        }
        Ok(())
    }

    fn interrupts(&mut self, now: Cycle) -> u32 {
        let sources = self.sources(self.clock.tick(now));
        self.matrix.lines(sources)
    }

    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        let next = self.systimer.next_event(self.clock.tick(now))?;
        Some(self.clock.cycle_after(next))
    }
}
