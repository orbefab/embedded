//! SPI1, the controller through which the firmware sends commands to the flash, and the flash
//! chip behind it.
//!
//! A write of `SPI_MEM_CMD_REG` runs its one command at once and clears its bits, and a write
//! that starts more than one is refused. A user command sends the command byte, the address when
//! `SPI_MEM_USR_ADDR` is set, and MOSI bytes from `SPI_MEM_W0_REG` (or `SPI_MEM_W8_REG` when
//! `SPI_MEM_USR_MOSI_HIGHPART` is set) when `SPI_MEM_USR_MOSI` is set, and leaves MISO bytes from
//! `SPI_MEM_W0_REG` (or `SPI_MEM_W8_REG` when `SPI_MEM_USR_MISO_HIGHPART` is set) on. A user
//! command while `SPI_MEM_MISC_REG` has `CS0_DIS` set or `CS1_DIS` clear is refused. That register
//! resets to `0x2`, CS0 on and CS1 off, and the fixed commands do not look at it. Dummy cycles
//! have no effect.
//!
//! The chip is the board's 16 MB Boya BY25Q128ES, JEDEC ID `68 40 18`, its quad enable bit set.
//! It reads, reports its ID and three status registers, and runs page program (`0x02`, `0x32`,
//! fixed `FLASH_PP`), 4 KB sector erase (`0x20`, fixed `FLASH_SE`), 32 KB block erase (`0x52`),
//! 64 KB block erase (`0xd8`, fixed `FLASH_BE`) and chip erase (`0x60`, `0xc7`, fixed `FLASH_CE`).
//! Program and erase require `WREN` (`WEL` set) first: without `WEL`, the command is ignored and
//! `WIP` stays clear; with `WEL`, the write or erase runs and `WIP` stays set for the BY25Q128ES
//! datasheet's typical time (0.7 ms page program, 70 ms sector erase, 0.25 s 32 KB block erase,
//! 0.4 s 64 KB block erase, 100 s chip erase), clearing both `WIP` and `WEL` when done. Page
//! program only clears bits (1 to 0) and wraps inside its 256-byte page; erase sets bytes to
//! `0xFF`. While `WIP` is set, only status reads (`0x05`, `0x35`, `0x15`, fixed `FLASH_RDSR`) are
//! allowed; every other flash command or cache access ends the run.
//!
//! A status write can lock the flash for good, so it is refused before any status bit changes,
//! with or without `WREN`. Any read, program or erase past the flash's end, any unsupported
//! command, mode or phase length, and SPI1 auto-wait-idle, suspend or interrupt enables end the
//! run.
//!
//! IDF v5.5.5, `components/soc/esp32s3/register/soc/spi_mem_reg.h`,
//! `components/hal/esp32s3/include/hal/spimem_flash_ll.h`,
//! `components/hal/spi_flash_hal_common.inc`, `components/hal/spi_flash_hal_iram.c`,
//! `components/spi_flash`, and the Boya BY25Q128ES datasheet.

use super::clock::Tick;
use super::flash::{self, Flash};

const CMD: u32 = 0x0;
const ADDR: u32 = 0x4;
const CTRL: u32 = 0x8;
const USER: u32 = 0x18;
const USER1: u32 = 0x1c;
const USER2: u32 = 0x20;
const MOSI_DLEN: u32 = 0x24;
const MISO_DLEN: u32 = 0x28;
const RD_STATUS: u32 = 0x2c;
const MISC: u32 = 0x34;
const W0: u32 = 0x58;
pub(crate) const WAITI_CTRL: u32 = 0x98;
pub(crate) const SUS_CMD: u32 = 0x9c;
pub(crate) const SUS_CTRL: u32 = 0xa0;
const DDR: u32 = 0xe0;
pub(crate) const INT_ENA: u32 = 0xf0;
const BUFFER_WORDS: usize = 16;
const SIZE: u32 = 0x400;

/// `SPI_MEM_WAITI_EN` (bit 0 of `SPI_MEM_FLASH_WAITI_CTRL_REG`).
pub(crate) const WAITI_EN: u32 = 1 << 0;
/// `SPI_MEM_FLASH_PES | SPI_MEM_FLASH_PER | SPI_MEM_PES_PER_EN` in `SPI_MEM_FLASH_SUS_CMD_REG`.
pub(crate) const SUS_CMD_UNRUN: u32 = (1 << 0) | (1 << 1) | (1 << 4);
/// `SPI_MEM_FLASH_PES_EN` (bit 0 of `SPI_MEM_FLASH_SUS_CTRL_REG`).
pub(crate) const FLASH_PES_EN: u32 = 1 << 0;

const FLASH_READ: u32 = 1 << 31;
const FLASH_WREN: u32 = 1 << 30;
const FLASH_WRDI: u32 = 1 << 29;
const FLASH_RDID: u32 = 1 << 28;
const FLASH_RDSR: u32 = 1 << 27;
const FLASH_WRSR: u32 = 1 << 26;
const FLASH_PP: u32 = 1 << 25;
const FLASH_SE: u32 = 1 << 24;
const FLASH_BE: u32 = 1 << 23;
const FLASH_CE: u32 = 1 << 22;
const FLASH_DP: u32 = 1 << 21;
const FLASH_RES: u32 = 1 << 20;
const FLASH_HPM: u32 = 1 << 19;
/// The status write `FLASH_WRSR` sends.
const WRSR: u8 = 0x01;
const USR: u32 = 1 << 18;
/// The `SPI_MEM_CMD_REG` bits that start a command, which clear when it is done. `SPI_MEM_USR`
/// and the fixed commands each start one. `SPI_MEM_FLASH_PE` marks a user command as a program
/// or an erase.
pub(crate) const STARTS: u32 = 0xffff_0000 | USR | 1 << 17;
const COMMANDS: u32 = 0xfffc_0000;

const USR_COMMAND: u32 = 1 << 31;
const USR_ADDR: u32 = 1 << 30;
const USR_MISO: u32 = 1 << 28;
const USR_MOSI: u32 = 1 << 27;
const MOSI_HIGHPART: u32 = 1 << 25;
const MISO_HIGHPART: u32 = 1 << 24;
const FWRITE_QIO: u32 = 1 << 15;
const FWRITE_DIO: u32 = 1 << 14;
const FWRITE_QUAD: u32 = 1 << 13;
const FWRITE_DUAL: u32 = 1 << 12;
const FWRITE_UNSUPPORTED: u32 = FWRITE_QIO | FWRITE_DIO | FWRITE_DUAL;
/// Octal phase bits in `SPI_MEM_CTRL_REG`: `FCMD_OCT`, `FADDR_OCT`, `FDIN_OCT`, `FDOUT_OCT`.
const CTRL_OCT: u32 = (1 << 9) | (1 << 6) | (1 << 5) | (1 << 4);
/// `SPI_MEM_FMEM_DDR_EN` (bit 0 of `SPI_MEM_DDR_REG`).
const FMEM_DDR_EN: u32 = 1 << 0;
/// `SPI_MEM_MISC_REG` bit 0. Set disables the flash's chip select.
const CS0_DIS: u32 = 1;
/// `SPI_MEM_MISC_REG` bit 1. Clear selects the second chip, which this flash is not.
const CS1_DIS: u32 = 1 << 1;

const JEDEC_ID: [u8; 3] = [0x68, 0x40, 0x18];
/// What `RDID` reads with no flash on the bus, whose MISO line idles high.
const NO_FLASH_ID: [u8; 3] = [0xff; 3];
/// Status Register 1 bit 0: Write In Progress (BUSY).
const WIP: u8 = 1 << 0;
/// Status Register 1 bit 1: Write Enable Latch.
const WEL: u8 = 1 << 1;
/// Status Register 2 bit 1: Quad Enable.
const QE: u8 = 1 << 1;

/// Ticks per microsecond on the chip's 480 MHz tick timeline.
const TICKS_PER_US: u64 = 480;
/// BY25Q128ES datasheet typical program and erase times, in 480 MHz ticks:
/// tPP = 0.7 ms, tSE = 70 ms, tBE1 (32 KB) = 0.25 s, tBE2 (64 KB) = 0.4 s, tCE = 100 s.
pub(crate) const T_PP: u64 = 700 * TICKS_PER_US;
pub(crate) const T_SE: u64 = 70_000 * TICKS_PER_US;
pub(crate) const T_BE_32K: u64 = 250_000 * TICKS_PER_US;
pub(crate) const T_BE_64K: u64 = 400_000 * TICKS_PER_US;
pub(crate) const T_CE: u64 = 100_000_000 * TICKS_PER_US;

/// Result of one SPI1 register write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// Whether the offset belonged to SPI1.
    pub(crate) handled: bool,
    /// Whether the write modified flash contents, so the cores' decode tables must invalidate.
    pub(crate) mutated: bool,
}

/// Why the chip refused a flash command, which ends the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashRefusal {
    /// A command the model does not know.
    Command(u8),
    /// A status write: `SPI_MEM_FLASH_WRSR`, or a user `0x01`, `0x31` or `0x11`.
    StatusWrite(u8),
    /// A user command with no command phase, or a write that sets only `SPI_MEM_FLASH_PE`.
    NoCommandPhase,
    /// One write of `SPI_MEM_CMD_REG` that starts more than one command.
    StartBits,
    /// A read, program or erase whose bytes reach past the flash's end. A chip with no flash
    /// refuses every read, program or erase.
    PastEnd(u8),
    /// A command whose address phase has a bit length the flash or SPI1 does not take.
    AddressBits(u8),
    /// A write of SPI0's `SPI_MEM_CMD_REG` that starts a command.
    Spi0,
    /// A write that would take a flash pad, GPIO26 to GPIO32, from SPI0 and SPI1: another
    /// IO_MUX function, its output enable, or a matrix output route.
    Pads,
    /// A user command while `CS0_DIS` is set or `CS1_DIS` is clear.
    Cs(u8),
    /// A command or cache access while a program or erase is in progress (`WIP` set).
    Busy(Option<u8>),
    /// A program or erase with a data or address phase length SPI1 or the flash does not take.
    Length(u8),
    /// A command started in an octal, DDR or unsupported write width mode.
    Mode(u8),
}

#[derive(Clone, Debug)]
pub(crate) struct Spi1 {
    /// The controller's registers, by word.
    regs: Vec<u32>,
    /// The chip's three status registers (`status[0]` holds `WEL`; `WIP` is live from `busy_until`).
    status: [u8; 3],
    /// Tick at which the current program or erase finishes (`WIP` and `WEL` clear). `None` when idle.
    busy_until: Option<Tick>,
}

impl Default for Spi1 {
    fn default() -> Spi1 {
        let mut regs = vec![0; (SIZE / 4) as usize];
        regs[(MISC / 4) as usize] = CS1_DIS;
        Spi1 {
            regs,
            status: [0, QE, 0],
            busy_until: None,
        }
    }
}

impl Spi1 {
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        self.regs.get((offset / 4) as usize).copied()
    }

    /// Cheap check for whether a program or erase might still be in progress.
    #[inline]
    pub(crate) fn has_busy(&self) -> bool {
        self.busy_until.is_some()
    }

    /// Whether the flash chip's `WIP` bit is set at `now`.
    #[inline]
    pub(crate) fn is_busy(&self, now: Tick) -> bool {
        self.busy_until.is_some_and(|until| now < until)
    }

    fn settle(&mut self, now: Tick) {
        if let Some(until) = self.busy_until
            && now >= until
        {
            self.busy_until = None;
            self.status[0] &= !WEL;
        }
    }

    fn sr1(&self, now: Tick) -> u8 {
        if self.is_busy(now) {
            self.status[0] | WIP
        } else {
            self.status[0]
        }
    }

    fn reg(&self, offset: u32) -> u32 {
        self.regs[(offset / 4) as usize]
    }

    fn check_oct_ddr(&self, command: u8) -> Result<(), FlashRefusal> {
        if self.reg(CTRL) & CTRL_OCT != 0 || self.reg(DDR) & FMEM_DDR_EN != 0 {
            return Err(FlashRefusal::Mode(command));
        }
        Ok(())
    }

    fn buffer_bytes(&self) -> [u8; BUFFER_WORDS * 4] {
        let mut out = [0u8; BUFFER_WORDS * 4];
        for (k, &word) in self.regs[(W0 / 4) as usize..][..BUFFER_WORDS]
            .iter()
            .enumerate()
        {
            out[4 * k..4 * k + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// Writes a register, and runs the command a write of `SPI_MEM_CMD_REG` starts.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
        flash: Option<&mut Flash>,
    ) -> Result<Outcome, FlashRefusal> {
        let Some(word) = self.regs.get_mut((offset / 4) as usize) else {
            return Ok(Outcome {
                handled: false,
                mutated: false,
            });
        };
        *word = value;
        let mut mutated = false;
        if offset == CMD && value & STARTS != 0 {
            self.regs[0] = value & !STARTS;
            self.settle(now);
            mutated = self.run(now, value, flash)?;
        }
        Ok(Outcome {
            handled: true,
            mutated,
        })
    }

    fn run(
        &mut self,
        now: Tick,
        cmd: u32,
        flash: Option<&mut Flash>,
    ) -> Result<bool, FlashRefusal> {
        match cmd & COMMANDS {
            0 => Err(FlashRefusal::NoCommandPhase),
            USR => self.user(now, flash),
            FLASH_RDSR => {
                self.regs[(RD_STATUS / 4) as usize] = u32::from(self.sr1(now));
                Ok(false)
            }
            FLASH_WRSR => Err(FlashRefusal::StatusWrite(WRSR)),
            FLASH_WREN => {
                if self.is_busy(now) {
                    return Err(FlashRefusal::Busy(Some(0x06)));
                }
                self.status[0] |= WEL;
                Ok(false)
            }
            FLASH_WRDI => {
                if self.is_busy(now) {
                    return Err(FlashRefusal::Busy(Some(0x04)));
                }
                self.status[0] &= !WEL;
                Ok(false)
            }
            FLASH_RDID => {
                if self.is_busy(now) {
                    return Err(FlashRefusal::Busy(Some(0x9f)));
                }
                let [a, b, c] = jedec_id(flash.as_deref());
                self.regs[(W0 / 4) as usize] = u32::from_le_bytes([a, b, c, 0]);
                Ok(false)
            }
            FLASH_PP => {
                if self.is_busy(now) {
                    return Err(FlashRefusal::Busy(Some(0x02)));
                }
                self.check_oct_ddr(0x02)?;
                let addr_reg = self.reg(ADDR);
                let addr = addr_reg & 0x00ff_ffff;
                let len = (addr_reg >> 24) as usize;
                if len == 0 || len > BUFFER_WORDS * 4 {
                    return Err(FlashRefusal::Length(0x02));
                }
                let buffer = self.buffer_bytes();
                self.program(now, 0x02, addr, &buffer[..len], flash)
            }
            FLASH_SE => {
                if self.is_busy(now) {
                    return Err(FlashRefusal::Busy(Some(0x20)));
                }
                self.check_oct_ddr(0x20)?;
                let addr = self.reg(ADDR) & 0x00ff_ffff;
                self.erase(now, 0x20, addr, flash::SECTOR, T_SE, flash)
            }
            FLASH_BE => {
                if self.is_busy(now) {
                    return Err(FlashRefusal::Busy(Some(0xd8)));
                }
                self.check_oct_ddr(0xd8)?;
                let addr = self.reg(ADDR) & 0x00ff_ffff;
                self.erase(now, 0xd8, addr, flash::BLOCK_64K, T_BE_64K, flash)
            }
            FLASH_CE => {
                if self.is_busy(now) {
                    return Err(FlashRefusal::Busy(Some(0xc7)));
                }
                self.check_oct_ddr(0xc7)?;
                self.erase_chip(now, 0xc7, flash)
            }
            FLASH_READ => Err(FlashRefusal::Command(0x03)),
            FLASH_DP => Err(FlashRefusal::Command(0xb9)),
            FLASH_RES => Err(FlashRefusal::Command(0xab)),
            FLASH_HPM => Err(FlashRefusal::Command(0xa3)),
            _ => Err(FlashRefusal::StartBits),
        }
    }

    /// Runs the user command the registers describe.
    fn user(&mut self, now: Tick, flash: Option<&mut Flash>) -> Result<bool, FlashRefusal> {
        let user = self.reg(USER);
        if user & USR_COMMAND == 0 {
            return Err(FlashRefusal::NoCommandPhase);
        }
        let command = self.reg(USER2) as u8;
        let misc = self.reg(MISC);
        if misc & CS0_DIS != 0 || misc & CS1_DIS == 0 {
            return Err(FlashRefusal::Cs(command));
        }
        self.check_oct_ddr(command)?;
        let addr = if user & USR_ADDR != 0 {
            let bits = (self.reg(USER1) >> 26) + 1;
            if bits > 32 {
                return Err(FlashRefusal::AddressBits(command));
            }
            Some((bits, self.reg(ADDR) & (u32::MAX >> (32 - bits))))
        } else {
            None
        };
        let mut buffer = self.buffer_bytes();
        let mosi = if user & USR_MOSI != 0 {
            let bitlen = (self.reg(MOSI_DLEN) & 0x3ff) as usize + 1;
            if !bitlen.is_multiple_of(8) {
                return Err(FlashRefusal::Length(command));
            }
            let len = bitlen / 8;
            let start = if user & MOSI_HIGHPART != 0 { 32 } else { 0 };
            if start + len > buffer.len() {
                return Err(FlashRefusal::Length(command));
            }
            &buffer[start..start + len]
        } else {
            &[]
        };
        let miso_len = if user & USR_MISO != 0 {
            ((self.reg(MISO_DLEN) & 0x3ff) as usize + 1).div_ceil(8)
        } else {
            0
        };
        let (miso, mutated) = self.transact(now, command, user, addr, mosi, miso_len, flash)?;
        if !miso.is_empty() {
            let start = if user & MISO_HIGHPART != 0 { 32 } else { 0 };
            let end = (start + miso.len()).min(buffer.len());
            buffer[start..end].copy_from_slice(&miso[..end - start]);
            for (k, word) in buffer.chunks(4).enumerate() {
                self.regs[(W0 / 4) as usize + k] =
                    u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            }
        }
        Ok(mutated)
    }

    /// Runs `command` in user mode, returning its MISO bytes (`miso_len` of them) and whether
    /// flash contents changed.
    #[allow(clippy::too_many_arguments)]
    fn transact(
        &mut self,
        now: Tick,
        command: u8,
        user: u32,
        addr: Option<(u32, u32)>,
        mosi: &[u8],
        miso_len: usize,
        flash: Option<&mut Flash>,
    ) -> Result<(Vec<u8>, bool), FlashRefusal> {
        let repeat = |byte: u8| vec![byte; miso_len];
        // Status register writes are always refused before any state change.
        if matches!(command, 0x01 | 0x31 | 0x11) {
            return Err(FlashRefusal::StatusWrite(command));
        }
        // Status register reads are allowed while WIP is set.
        match command {
            0x05 => return Ok((repeat(self.sr1(now)), false)),
            0x35 => return Ok((repeat(self.status[1]), false)),
            0x15 => return Ok((repeat(self.status[2]), false)),
            _ => {}
        }
        if self.is_busy(now) {
            return Err(FlashRefusal::Busy(Some(command)));
        }
        match command {
            0x9f => Ok((
                jedec_id(flash.as_deref())
                    .iter()
                    .copied()
                    .chain(std::iter::repeat(0))
                    .take(miso_len)
                    .collect(),
                false,
            )),
            0x06 => {
                self.status[0] |= WEL;
                Ok((Vec::new(), false))
            }
            0x04 => {
                self.status[0] &= !WEL;
                Ok((Vec::new(), false))
            }
            0x03 | 0x0b | 0x3b | 0x6b | 0xbb | 0xeb => {
                let start = addr.map_or(0, |(_, a)| a) as usize;
                let Some(bytes) = flash.and_then(|f| f.read(start, miso_len)) else {
                    return Err(FlashRefusal::PastEnd(command));
                };
                Ok((bytes, false))
            }
            0x02 | 0x32 => {
                if user & FWRITE_UNSUPPORTED != 0 || (command == 0x02 && user & FWRITE_QUAD != 0) {
                    return Err(FlashRefusal::Mode(command));
                }
                if miso_len != 0 || mosi.is_empty() {
                    return Err(FlashRefusal::Length(command));
                }
                let Some((24, address)) = addr else {
                    return Err(FlashRefusal::AddressBits(command));
                };
                let mutated = self.program(now, command, address, mosi, flash)?;
                Ok((Vec::new(), mutated))
            }
            0x20 | 0x52 | 0xd8 => {
                if user & (FWRITE_UNSUPPORTED | FWRITE_QUAD) != 0 {
                    return Err(FlashRefusal::Mode(command));
                }
                if miso_len != 0 {
                    return Err(FlashRefusal::Length(command));
                }
                // Standard 24-bit address with no MOSI, or ESP32-S3 IDF's PMS workaround
                // (`spi_flash_hal_common.inc`), which sends the top 16 bits in ADDR and the low
                // 8 bits as 1 byte of MOSI.
                let address = match (addr, mosi) {
                    (Some((24, address)), []) => address,
                    (Some((16, high)), [low]) => (high << 8) | u32::from(*low),
                    (Some((bits, _)), _) if bits != 24 && bits != 16 => {
                        return Err(FlashRefusal::AddressBits(command));
                    }
                    (None, _) => return Err(FlashRefusal::AddressBits(command)),
                    _ => return Err(FlashRefusal::Length(command)),
                };
                let (size, duration) = match command {
                    0x20 => (flash::SECTOR, T_SE),
                    0x52 => (flash::BLOCK_32K, T_BE_32K),
                    0xd8 => (flash::BLOCK_64K, T_BE_64K),
                    _ => unreachable!(),
                };
                let mutated = self.erase(now, command, address, size, duration, flash)?;
                Ok((Vec::new(), mutated))
            }
            0x60 | 0xc7 => {
                if addr.is_some() || !mosi.is_empty() || miso_len != 0 {
                    return Err(FlashRefusal::Length(command));
                }
                let mutated = self.erase_chip(now, command, flash)?;
                Ok((Vec::new(), mutated))
            }
            0x99 => {
                self.status[0] &= !WEL;
                Ok((repeat(0), false))
            }
            0xab | 0xb9 | 0x66 | 0x7a | 0x75 | 0xff => Ok((repeat(0), false)),
            other => Err(FlashRefusal::Command(other)),
        }
    }

    fn program(
        &mut self,
        now: Tick,
        command: u8,
        addr: u32,
        data: &[u8],
        flash: Option<&mut Flash>,
    ) -> Result<bool, FlashRefusal> {
        let Some(flash) = flash else {
            return Err(FlashRefusal::PastEnd(command));
        };
        if !flash.can_program_page(addr as usize, data.len()) {
            return Err(FlashRefusal::PastEnd(command));
        }
        if self.status[0] & WEL == 0 {
            return Ok(false);
        }
        let mutated = flash
            .program_page(addr as usize, data)
            .ok_or(FlashRefusal::PastEnd(command))?;
        self.busy_until = Some(Tick(now.0.saturating_add(T_PP)));
        Ok(mutated)
    }

    fn erase(
        &mut self,
        now: Tick,
        command: u8,
        addr: u32,
        size: usize,
        duration: u64,
        flash: Option<&mut Flash>,
    ) -> Result<bool, FlashRefusal> {
        let Some(flash) = flash else {
            return Err(FlashRefusal::PastEnd(command));
        };
        if !flash.can_erase_range(addr as usize, size) {
            return Err(FlashRefusal::PastEnd(command));
        }
        if self.status[0] & WEL == 0 {
            return Ok(false);
        }
        let mutated = flash
            .erase_range(addr as usize, size)
            .ok_or(FlashRefusal::PastEnd(command))?;
        self.busy_until = Some(Tick(now.0.saturating_add(duration)));
        Ok(mutated)
    }

    fn erase_chip(
        &mut self,
        now: Tick,
        command: u8,
        flash: Option<&mut Flash>,
    ) -> Result<bool, FlashRefusal> {
        let Some(flash) = flash else {
            return Err(FlashRefusal::PastEnd(command));
        };
        if flash.is_empty() {
            return Err(FlashRefusal::PastEnd(command));
        }
        if self.status[0] & WEL == 0 {
            return Ok(false);
        }
        let mutated = flash.erase_chip().ok_or(FlashRefusal::PastEnd(command))?;
        self.busy_until = Some(Tick(now.0.saturating_add(T_CE)));
        Ok(mutated)
    }
}

fn jedec_id(flash: Option<&Flash>) -> [u8; 3] {
    if flash.is_none_or(Flash::is_empty) {
        NO_FLASH_ID
    } else {
        JEDEC_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spi1() -> Spi1 {
        let mut spi1 = Spi1::default();
        spi1.regs[(USER1 / 4) as usize] = 23 << 26;
        spi1
    }

    fn flash(bytes: Vec<u8>) -> Flash {
        Flash::new(bytes.into())
    }

    /// Sends `command` with optional 24-bit `addr` and MISO read of `len` bytes at `now`.
    fn send_at(
        spi1: &mut Spi1,
        now: Tick,
        mut flash: Option<&mut Flash>,
        command: u8,
        addr: Option<u32>,
        len: u32,
    ) -> Result<Vec<u8>, FlashRefusal> {
        let mut user = USR_COMMAND;
        if let Some(addr) = addr {
            user |= USR_ADDR;
            spi1.write(now, ADDR, addr, flash.as_deref_mut()).unwrap();
        }
        if len > 0 {
            user |= USR_MISO;
            spi1.write(now, MISO_DLEN, 8 * len - 1, flash.as_deref_mut())
                .unwrap();
        }
        spi1.write(now, USER, user, flash.as_deref_mut()).unwrap();
        spi1.write(
            now,
            USER2,
            7 << 28 | u32::from(command),
            flash.as_deref_mut(),
        )
        .unwrap();
        let sent = spi1.write(now, CMD, USR, flash);
        assert_eq!(spi1.read(CMD), Some(0), "the command is done at once");
        sent.map(|_| {
            let buf = spi1.buffer_bytes();
            buf[..len as usize].to_vec()
        })
    }

    fn send(
        spi1: &mut Spi1,
        flash: Option<&mut Flash>,
        command: u8,
        addr: Option<u32>,
        len: u32,
    ) -> Result<Vec<u8>, FlashRefusal> {
        send_at(spi1, Tick(0), flash, command, addr, len)
    }

    fn user(spi1: &mut Spi1, command: u8, addr: Option<u32>, len: u32) -> Vec<u8> {
        let mut f = flash((0..=255).collect());
        send(spi1, Some(&mut f), command, addr, len).unwrap()
    }

    fn program_user(
        spi1: &mut Spi1,
        now: Tick,
        f: &mut Flash,
        command: u8,
        addr: u32,
        data: &[u8],
    ) -> Result<Outcome, FlashRefusal> {
        let mut words = [0u32; BUFFER_WORDS];
        for (k, chunk) in data.chunks(4).enumerate() {
            let mut b = [0u8; 4];
            b[..chunk.len()].copy_from_slice(chunk);
            words[k] = u32::from_le_bytes(b);
        }
        for (k, &w) in words.iter().enumerate() {
            spi1.write(now, W0 + 4 * k as u32, w, Some(f)).unwrap();
        }
        spi1.write(now, ADDR, addr, Some(f)).unwrap();
        spi1.write(now, USER1, 23 << 26, Some(f)).unwrap();
        spi1.write(
            now,
            MOSI_DLEN,
            (8 * data.len() as u32).saturating_sub(1),
            Some(f),
        )
        .unwrap();
        let mut u = USR_COMMAND | USR_ADDR | USR_MOSI;
        if command == 0x32 {
            u |= FWRITE_QUAD;
        }
        spi1.write(now, USER, u, Some(f)).unwrap();
        spi1.write(now, USER2, 7 << 28 | u32::from(command), Some(f))
            .unwrap();
        spi1.write(now, CMD, USR | 1 << 17, Some(f))
    }

    #[test]
    fn the_chip_reports_its_id_status_and_reads_from_an_address() {
        let mut spi1 = spi1();
        assert_eq!(user(&mut spi1, 0x9f, None, 3), [0x68, 0x40, 0x18]);
        assert_eq!(user(&mut spi1, 0x35, None, 1), [QE]);
        assert_eq!(
            user(&mut spi1, 0x03, Some(0x10), 4),
            [0x10, 0x11, 0x12, 0x13]
        );
        assert_eq!(
            user(&mut spi1, 0x03, Some(0xfc), 4),
            [0xfc, 0xfd, 0xfe, 0xff],
            "the last bytes"
        );
    }

    #[test]
    fn a_chip_with_no_flash_reads_its_id_as_ff() {
        let mut spi1 = spi1();
        assert_eq!(send(&mut spi1, None, 0x9f, None, 3), Ok(vec![0xff; 3]));
        spi1.write(Tick(0), CMD, FLASH_RDID, None).unwrap();
        assert_eq!(spi1.read(W0), Some(0x00ff_ffff));
    }

    #[test]
    fn a_status_write_is_refused_before_any_status_bit_changes() {
        let mut f = flash(vec![0xff; 16]);
        for enable in [false, true] {
            for command in [0x01, 0x31, 0x11] {
                let mut spi1 = spi1();
                if enable {
                    send(&mut spi1, Some(&mut f), 0x06, None, 0).unwrap();
                }
                spi1.write(Tick(0), W0, 0, Some(&mut f)).unwrap();
                spi1.write(Tick(0), MOSI_DLEN, 15, Some(&mut f)).unwrap();
                spi1.write(Tick(0), USER, USR_COMMAND | USR_MOSI, Some(&mut f))
                    .unwrap();
                spi1.write(Tick(0), USER2, 7 << 28 | u32::from(command), Some(&mut f))
                    .unwrap();
                assert_eq!(
                    spi1.write(Tick(0), CMD, USR, Some(&mut f)),
                    Err(FlashRefusal::StatusWrite(command)),
                    "{command:#04x}, WREN {enable}"
                );
                let wel = if enable { WEL } else { 0 };
                assert_eq!(spi1.status, [wel, QE, 0], "{command:#04x}, WREN {enable}");
            }
            let mut spi1 = spi1();
            if enable {
                send(&mut spi1, Some(&mut f), 0x06, None, 0).unwrap();
            }
            assert_eq!(
                spi1.write(Tick(0), CMD, FLASH_WRSR, Some(&mut f)),
                Err(FlashRefusal::StatusWrite(0x01)),
                "the fixed WRSR, WREN {enable}"
            );
            assert_eq!(spi1.status[1], QE);
        }
    }

    #[test]
    fn program_needs_wren_clears_bits_wraps_page_and_holds_wip_for_tpp() {
        let mut f = flash(vec![0xff; flash::SECTOR]);
        let mut spi1 = spi1();

        // Without WREN, page program is ignored and WIP stays clear.
        let out = program_user(
            &mut spi1,
            Tick(10),
            &mut f,
            0x02,
            0x00fc,
            &[0xaa, 0x55, 0xf0, 0x0f, 0x12],
        )
        .unwrap();
        assert!(!out.mutated);
        assert_eq!(
            send_at(&mut spi1, Tick(10), Some(&mut f), 0x05, None, 1),
            Ok(vec![0])
        );
        assert_eq!(f.read(0x00fc, 4), Some(vec![0xff; 4]));

        // With WREN, page program writes, wraps inside the 256-byte page, and sets WIP for T_PP.
        spi1.write(Tick(100), CMD, FLASH_WREN, Some(&mut f))
            .unwrap();
        assert_eq!(
            send_at(&mut spi1, Tick(100), Some(&mut f), 0x05, None, 1),
            Ok(vec![WEL])
        );
        let out = program_user(
            &mut spi1,
            Tick(100),
            &mut f,
            0x02,
            0x00fc,
            &[0xaa, 0x55, 0xf0, 0x0f, 0x12],
        )
        .unwrap();
        assert!(out.mutated);
        // WIP stays set until 100 + T_PP.
        assert_eq!(
            send_at(&mut spi1, Tick(100 + T_PP - 1), Some(&mut f), 0x05, None, 1),
            Ok(vec![WIP | WEL])
        );
        // Fixed RDSR also sees WIP.
        spi1.write(Tick(100 + T_PP - 1), CMD, FLASH_RDSR, Some(&mut f))
            .unwrap();
        assert_eq!(spi1.read(RD_STATUS), Some(u32::from(WIP | WEL)));
        // Any non-status command while WIP=1 is refused as Busy.
        assert_eq!(
            send_at(
                &mut spi1,
                Tick(100 + T_PP - 1),
                Some(&mut f),
                0x03,
                Some(0),
                4
            ),
            Err(FlashRefusal::Busy(Some(0x03)))
        );
        assert_eq!(
            spi1.write(Tick(100 + T_PP - 1), CMD, FLASH_WREN, Some(&mut f)),
            Err(FlashRefusal::Busy(Some(0x06)))
        );

        // At 100 + T_PP, both WIP and WEL are clear.
        assert_eq!(
            send_at(&mut spi1, Tick(100 + T_PP), Some(&mut f), 0x05, None, 1),
            Ok(vec![0])
        );
        assert_eq!(
            send_at(
                &mut spi1,
                Tick(100 + T_PP),
                Some(&mut f),
                0x03,
                Some(0x00fc),
                4
            ),
            Ok(vec![0xaa, 0x55, 0xf0, 0x0f])
        );
        assert_eq!(
            send_at(
                &mut spi1,
                Tick(100 + T_PP),
                Some(&mut f),
                0x03,
                Some(0x0000),
                2
            ),
            Ok(vec![0x12, 0xff]),
            "wrapped byte at page start"
        );
        assert_eq!(
            send_at(
                &mut spi1,
                Tick(100 + T_PP),
                Some(&mut f),
                0x03,
                Some(0x0100),
                1
            ),
            Ok(vec![0xff]),
            "next page untouched"
        );

        // Fixed FLASH_PP also works (and only clears bits 1 -> 0).
        let t2 = Tick(100 + T_PP + 10);
        spi1.write(t2, CMD, FLASH_WREN, Some(&mut f)).unwrap();
        spi1.write(t2, W0, u32::from_le_bytes([0xff, 0x0f, 0, 0]), Some(&mut f))
            .unwrap();
        spi1.write(t2, ADDR, (2 << 24) | 0x00fc, Some(&mut f))
            .unwrap();
        assert!(spi1.write(t2, CMD, FLASH_PP, Some(&mut f)).unwrap().mutated);
        let t3 = Tick(t2.0 + T_PP);
        assert_eq!(
            send_at(&mut spi1, t3, Some(&mut f), 0x03, Some(0x00fc), 2),
            Ok(vec![0xaa, 0x05])
        );
    }

    #[test]
    fn sector_block_and_chip_erase_work_with_sheet_times_and_pms_split_address() {
        let mut f = flash(vec![0x5a; flash::BLOCK_64K * 2]);
        let mut spi1 = spi1();

        // Sector erase via ESP32-S3 PMS split address (16-bit ADDR + 1-byte MOSI).
        let t = Tick(0);
        spi1.write(t, CMD, FLASH_WREN, Some(&mut f)).unwrap();
        spi1.write(t, USER1, 15 << 26, Some(&mut f)).unwrap();
        spi1.write(t, ADDR, 0x0010, Some(&mut f)).unwrap();
        spi1.write(t, MOSI_DLEN, 7, Some(&mut f)).unwrap();
        spi1.write(t, W0, 0x34, Some(&mut f)).unwrap();
        spi1.write(t, USER, USR_COMMAND | USR_ADDR | USR_MOSI, Some(&mut f))
            .unwrap();
        spi1.write(t, USER2, 7 << 28 | 0x20, Some(&mut f)).unwrap();
        assert!(
            spi1.write(t, CMD, USR | 1 << 17, Some(&mut f))
                .unwrap()
                .mutated
        );
        assert!(spi1.is_busy(Tick(T_SE - 1)));
        assert!(!spi1.is_busy(Tick(T_SE)));
        assert_eq!(f.read(0x1000, 4), Some(vec![0xff; 4]));
        assert_eq!(f.read(0x0ffc, 4), Some(vec![0x5a; 4]));
        assert_eq!(f.read(0x2000, 4), Some(vec![0x5a; 4]));

        // 32 KB block erase (0x52) via 24-bit user command.
        let t = Tick(T_SE);
        spi1.write(t, CMD, FLASH_WREN, Some(&mut f)).unwrap();
        spi1.write(t, USER1, 23 << 26, Some(&mut f)).unwrap();
        spi1.write(t, ADDR, 0x0000_0100, Some(&mut f)).unwrap();
        spi1.write(t, USER, USR_COMMAND | USR_ADDR, Some(&mut f))
            .unwrap();
        spi1.write(t, USER2, 7 << 28 | 0x52, Some(&mut f)).unwrap();
        assert!(spi1.write(t, CMD, USR, Some(&mut f)).unwrap().mutated);
        assert!(spi1.is_busy(Tick(t.0 + T_BE_32K - 1)));
        assert!(!spi1.is_busy(Tick(t.0 + T_BE_32K)));
        assert_eq!(f.read(0x0000, 1), Some(vec![0xff]));
        assert_eq!(f.read(0x7fff, 1), Some(vec![0xff]));
        assert_eq!(f.read(0x8000, 1), Some(vec![0x5a]));

        // 64 KB block erase via fixed FLASH_BE.
        let t = Tick(t.0 + T_BE_32K);
        spi1.write(t, CMD, FLASH_WREN, Some(&mut f)).unwrap();
        spi1.write(t, ADDR, 0x0000_9000, Some(&mut f)).unwrap();
        assert!(spi1.write(t, CMD, FLASH_BE, Some(&mut f)).unwrap().mutated);
        assert!(spi1.is_busy(Tick(t.0 + T_BE_64K - 1)));
        assert!(!spi1.is_busy(Tick(t.0 + T_BE_64K)));
        assert_eq!(f.read(0x8000, 1), Some(vec![0xff]));
        assert_eq!(f.read(flash::BLOCK_64K, 1), Some(vec![0x5a]));

        // Chip erase via fixed FLASH_CE.
        let t = Tick(t.0 + T_BE_64K);
        spi1.write(t, CMD, FLASH_WREN, Some(&mut f)).unwrap();
        assert!(spi1.write(t, CMD, FLASH_CE, Some(&mut f)).unwrap().mutated);
        assert!(spi1.is_busy(Tick(t.0 + T_CE - 1)));
        assert!(!spi1.is_busy(Tick(t.0 + T_CE)));
        assert_eq!(f.read(flash::BLOCK_64K, 1), Some(vec![0xff]));
    }

    #[test]
    fn unsupported_fixed_commands_lengths_and_modes_fail_loudly() {
        let mut f = flash(vec![0xff; flash::SECTOR]);
        assert_eq!(
            spi1().write(Tick(0), CMD, FLASH_DP, Some(&mut f)),
            Err(FlashRefusal::Command(0xb9)),
            "the fixed deep power-down"
        );
        // FLASH_PP with length 0 or > 64 fails loudly.
        let mut s = spi1();
        s.write(Tick(0), ADDR, 0, Some(&mut f)).unwrap();
        assert_eq!(
            s.write(Tick(0), CMD, FLASH_PP, Some(&mut f)),
            Err(FlashRefusal::Length(0x02))
        );
        s.write(Tick(0), ADDR, 65 << 24, Some(&mut f)).unwrap();
        assert_eq!(
            s.write(Tick(0), CMD, FLASH_PP, Some(&mut f)),
            Err(FlashRefusal::Length(0x02))
        );
        // Quad write mode on 0x02 fails loudly; 0x32 succeeds.
        let mut s = spi1();
        s.write(Tick(0), CMD, FLASH_WREN, Some(&mut f)).unwrap();
        s.write(Tick(0), ADDR, 0, Some(&mut f)).unwrap();
        s.write(Tick(0), MOSI_DLEN, 7, Some(&mut f)).unwrap();
        s.write(
            Tick(0),
            USER,
            USR_COMMAND | USR_ADDR | USR_MOSI | FWRITE_QUAD,
            Some(&mut f),
        )
        .unwrap();
        s.write(Tick(0), USER2, 7 << 28 | 0x02, Some(&mut f))
            .unwrap();
        assert_eq!(
            s.write(Tick(0), CMD, USR, Some(&mut f)),
            Err(FlashRefusal::Mode(0x02))
        );
        assert!(
            program_user(&mut s, Tick(0), &mut f, 0x32, 0, &[0x12])
                .unwrap()
                .mutated
        );
    }

    #[test]
    fn a_write_that_starts_two_commands_runs_neither() {
        let mut f = flash(vec![0xff; flash::SECTOR]);
        let mut spi1 = spi1();
        assert_eq!(
            spi1.write(Tick(0), CMD, FLASH_WREN | FLASH_SE, Some(&mut f)),
            Err(FlashRefusal::StartBits),
            "WREN and a sector erase"
        );
        assert_eq!(spi1.status[0], 0, "no WREN");
        spi1.write(Tick(0), RD_STATUS, 0xa5, Some(&mut f)).unwrap();
        assert_eq!(
            spi1.write(Tick(0), CMD, FLASH_RDSR | FLASH_PP, Some(&mut f)),
            Err(FlashRefusal::StartBits),
            "RDSR and a page program"
        );
        assert_eq!(spi1.read(RD_STATUS), Some(0xa5), "no RDSR");
        assert_eq!(
            spi1.write(Tick(0), CMD, USR | FLASH_RDID, Some(&mut f)),
            Err(FlashRefusal::StartBits),
            "a user command and RDID"
        );
        assert_eq!(spi1.read(CMD), Some(0), "the start bits clear");
    }

    #[test]
    fn a_read_or_write_past_the_flash_or_on_no_flash_is_refused() {
        let mut f = flash((0..=255).collect());
        let mut spi1 = spi1();
        assert_eq!(
            send(&mut spi1, Some(&mut f), 0x03, Some(0xfe), 4),
            Err(FlashRefusal::PastEnd(0x03))
        );
        assert_eq!(
            send(&mut spi1, None, 0x03, Some(0), 1),
            Err(FlashRefusal::PastEnd(0x03)),
            "no flash"
        );
        assert_eq!(
            send(&mut spi1, Some(&mut f), 0xeb, Some(0xff_ffff), 4),
            Err(FlashRefusal::PastEnd(0xeb))
        );
        assert_eq!(
            spi1.write(Tick(0), CMD, FLASH_SE, None),
            Err(FlashRefusal::PastEnd(0x20)),
            "erase on no flash"
        );
    }

    #[test]
    fn an_address_longer_than_32_bits_is_refused() {
        let mut f = flash((0..=255).collect());
        let mut spi1 = spi1();
        spi1.regs[(USER1 / 4) as usize] = 32 << 26;
        assert_eq!(
            send(&mut spi1, Some(&mut f), 0x03, Some(0x10), 4),
            Err(FlashRefusal::AddressBits(0x03)),
            "33 bits"
        );
        spi1.regs[(USER1 / 4) as usize] = 31 << 26;
        assert_eq!(
            send(&mut spi1, Some(&mut f), 0x03, Some(0x10), 1),
            Ok(vec![0x10]),
            "32 bits"
        );
    }

    #[test]
    fn a_user_command_with_the_wrong_chip_select_is_refused() {
        let mut f = flash((0..=255).collect());
        let mut reset = spi1();
        assert_eq!(reset.read(MISC), Some(CS1_DIS), "CS0 on, CS1 off");
        assert_eq!(
            send(&mut reset, Some(&mut f), 0x03, Some(0x10), 1),
            Ok(vec![0x10]),
            "the reset select"
        );

        let mut cs0 = spi1();
        cs0.write(Tick(0), MISC, CS0_DIS | CS1_DIS, Some(&mut f))
            .unwrap();
        assert_eq!(
            send(&mut cs0, Some(&mut f), 0x03, Some(0x10), 1),
            Err(FlashRefusal::Cs(0x03)),
            "CS0 disabled"
        );
        assert_eq!(cs0.read(W0), Some(0), "the read does not run");

        let mut cs1 = spi1();
        cs1.write(Tick(0), MISC, 0, Some(&mut f)).unwrap();
        assert_eq!(
            send(&mut cs1, Some(&mut f), 0x9f, None, 3),
            Err(FlashRefusal::Cs(0x9f)),
            "CS1 enabled"
        );

        let mut bare = spi1();
        bare.write(Tick(0), MISC, CS0_DIS, Some(&mut f)).unwrap();
        bare.write(Tick(0), USER, 0, Some(&mut f)).unwrap();
        bare.write(Tick(0), USER2, 7 << 28 | 0x03, Some(&mut f))
            .unwrap();
        assert_eq!(
            bare.write(Tick(0), CMD, USR, Some(&mut f)),
            Err(FlashRefusal::NoCommandPhase),
            "no command phase is refused first"
        );

        let mut fixed = spi1();
        fixed.write(Tick(0), MISC, 0, Some(&mut f)).unwrap();
        fixed.write(Tick(0), CMD, FLASH_RDSR, Some(&mut f)).unwrap();
        assert_eq!(
            fixed.read(RD_STATUS),
            Some(0),
            "a fixed command ignores the select"
        );
    }
}
