//! SPI2 and SPI3, the general-purpose SPI hosts IDF calls GPSPI2 and GPSPI3, as masters: the
//! firmware writes up to 64 bytes into `SPI_W0` to `SPI_W15`, or points a GDMA channel at them
//! with `SPI_DMA_TX_ENA` set, and sets `SPI_USR`, and the host sends a command phase, an address
//! phase and the data on one, two or four lanes. With `SPI_USR_MISO` it reads up to 64 bytes on
//! MISO into `SPI_W0` on: in a read phase after the data, or with `SPI_DOUTDIN` one byte beside
//! each byte it sends, on one lane. The read is `SPI_MS_DATA_BITLEN` long, as the data is. The
//! target's bytes come at the transfer's end, before the firmware can read `SPI_USR` as 0.
//!
//! Each transfer is one [`Transfer`], with the phases and the data as the registers set them when
//! `SPI_USR` was written, and the pads the GPIO matrix routed its signals to then. It lasts its
//! clocks at the rate `SPI_CLOCK` and the `SPI_CLK_GATE` source give. Until it ends `SPI_USR`
//! reads 1, and at its end `SPI_TRANS_DONE_INT_RAW` sets. Its interrupt line is that bit and
//! `SPI_TRANS_DONE_INT_ENA`. A write of `SPI_USR` while one runs starts nothing.
//! `SPI_CS_KEEP_ACTIVE` leaves CS asserted after it, which the transfer records.
//!
//! The pads carry the signals' idle levels for as long as they are routed, never the bits: CS
//! deasserted, SCLK at `SPI_CK_IDLE_EDGE`, and each data line at its `SPI_*_POL` bit. A setting
//! the model does not run, such as a dummy phase, or a register written since the last
//! `SPI_UPDATE`, refuses the transfer with a [`Setting`]. The registers come from IDF's
//! `soc/esp32s3/register/soc/spi_reg.h`.

use std::collections::VecDeque;

use orbe_model::Miso;

use super::clock::{TICK_HZ, Tick, XTAL};
use super::gpio::Gpio;

const CMD: u32 = 0x00;
const ADDR: u32 = 0x04;
const CTRL: u32 = 0x08;
const CLOCK: u32 = 0x0c;
const USER: u32 = 0x10;
const USER1: u32 = 0x14;
const USER2: u32 = 0x18;
const MS_DLEN: u32 = 0x1c;
const MISC: u32 = 0x20;
const DOUT_MODE: u32 = 0x2c;
const DMA_CONF: u32 = 0x30;
const DMA_INT_ENA: u32 = 0x34;
const DMA_INT_CLR: u32 = 0x38;
const DMA_INT_RAW: u32 = 0x3c;
const DMA_INT_ST: u32 = 0x40;
const DMA_INT_SET: u32 = 0x44;
const W0: u32 = 0x98;
const W15: u32 = 0xd4;
const SLAVE: u32 = 0xe0;
const CLK_GATE: u32 = 0xe8;
const DATE: u32 = 0xf0;

const USR: u32 = 1 << 24;
const UPDATE: u32 = 1 << 23;

const WR_BIT_ORDER: u32 = 0x3 << 25;
const RD_BIT_ORDER: u32 = 0x3 << 23;
const WP_POL: u32 = 1 << 21;
const HOLD_POL: u32 = 1 << 20;
const D_POL: u32 = 1 << 19;
const Q_POL: u32 = 1 << 18;
const FREAD_OCT: u32 = 1 << 16;
const FREAD_QUAD: u32 = 1 << 15;
const FREAD_DUAL: u32 = 1 << 14;
const FCMD_OCT: u32 = 1 << 10;
const FCMD_QUAD: u32 = 1 << 9;
const FCMD_DUAL: u32 = 1 << 8;
const FADDR_OCT: u32 = 1 << 7;
const FADDR_QUAD: u32 = 1 << 6;
const FADDR_DUAL: u32 = 1 << 5;

const CLK_EQU_SYSCLK: u32 = 1 << 31;

const USR_COMMAND: u32 = 1 << 31;
const USR_ADDR: u32 = 1 << 30;
const USR_DUMMY: u32 = 1 << 29;
const USR_MISO: u32 = 1 << 28;
const USR_MOSI: u32 = 1 << 27;
const USR_MOSI_HIGHPART: u32 = 1 << 25;
const USR_MISO_HIGHPART: u32 = 1 << 24;
const SIO: u32 = 1 << 17;
const FWRITE_OCT: u32 = 1 << 14;
const FWRITE_QUAD: u32 = 1 << 13;
const FWRITE_DUAL: u32 = 1 << 12;
const CK_OUT_EDGE: u32 = 1 << 9;
const CS_SETUP: u32 = 1 << 7;
const CS_HOLD: u32 = 1 << 6;
const OPI_MODE: u32 = 1 << 4;
const QPI_MODE: u32 = 1 << 3;
const DOUTDIN: u32 = 1 << 0;

const CS_KEEP_ACTIVE: u32 = 1 << 30;
const CK_IDLE_EDGE: u32 = 1 << 29;
const DTR: u32 = 0xf << 16;
const MASTER_CS0_POL: u32 = 1 << 7;
const CS0_DIS: u32 = 1 << 0;

const DMA_TX_ENA: u32 = 1 << 28;
const DMA_RX_ENA: u32 = 1 << 27;
/// `SPI_DMA_AFIFO_RST`, `SPI_BUF_AFIFO_RST` and `SPI_RX_AFIFO_RST`, which read 0.
const AFIFO_RST: u32 = 0x7 << 29;
/// `SPI_DMA_INFIFO_FULL` and `SPI_DMA_OUTFIFO_EMPTY`, which read 1.
const FIFO_STATUS: u32 = 0x3;

const USR_CONF: u32 = 1 << 28;
const SLAVE_MODE: u32 = 1 << 26;
const MST_CLK_SEL: u32 = 1 << 2;
const MST_CLK_ACTIVE: u32 = 1 << 1;
const CLK_EN: u32 = 1 << 0;

/// `SPI_TRANS_DONE_INT_*`, the one interrupt each host raises here.
pub(crate) const TRANS_DONE: u32 = 1 << 12;

/// Base addresses of SPI2 and SPI3 (`DR_REG_SPI2_BASE`, `DR_REG_SPI3_BASE`).
pub(crate) const SPI2: u32 = 0x6002_4000;
pub(crate) const SPI3: u32 = 0x6002_5000;
pub(crate) const BASES: [u32; 2] = [SPI2, SPI3];
pub(crate) const NAMES: [&str; 2] = ["SPI2", "SPI3"];
/// `GDMA_PERI_OUT_SEL` values for SPI2 and SPI3 (`SOC_GDMA_TRIG_PERIPH_SPI2`, `..._SPI3`).
pub(crate) const DMA_PERIPHERALS: [u32; 2] = [super::gdma::SPI2, super::gdma::SPI3];

/// Port index for the peripheral block base `base`, or `None` when `base` is not an SPI host.
pub(crate) fn index(base: u32) -> Option<usize> {
    BASES.iter().position(|&b| b == base)
}

/// The GPIO matrix signals of one SPI host: CS0, CLK, and data lines D0 to D3 (MOSI, MISO, WP,
/// HD), from IDF's `soc/esp32s3/include/soc/gpio_sig_map.h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Signals {
    cs0: u16,
    clk: u16,
    lanes: [u16; 4],
}

const SIGNALS: [Signals; 2] = [
    // SPI2 (FSPI): FSPICS0, FSPICLK, FSPID, FSPIQ, FSPIWP, FSPIHD.
    Signals {
        cs0: 110,
        clk: 101,
        lanes: [103, 102, 105, 104],
    },
    // SPI3 (HSPI): SPI3_CS0, SPI3_CLK, SPI3_D, SPI3_Q, SPI3_WP, SPI3_HD.
    Signals {
        cs0: 71,
        clk: 66,
        lanes: [68, 67, 70, 69],
    },
];

/// `PLL_F80M`, the clock `SPI_MST_CLK_SEL` 1 picks. 0 picks the crystal.
const PLL_F80M: u64 = 80_000_000;
const XTAL_HZ: u64 = TICK_HZ / XTAL;

const BUFFER: usize = 64;
/// The bits `SPI_MS_DATA_BITLEN` can count, 2^18.
const DMA_BITS: u32 = 1 << 18;

/// The registers `SPI_UPDATE` carries into SPI2's own clock domain, `SPI_ADDR` to
/// `SPI_DOUT_MODE`, by offset over 4. They are the ones IDF's `spi_ll.h` has a segment's CONF
/// phase reload.
const SYNCED: std::ops::RangeInclusive<usize> = (ADDR / 4) as usize..=(DOUT_MODE / 4) as usize;

/// A setting of SPI2 that the model does not run. A transfer started with one ends the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// `SPI_SLAVE_MODE`.
    Slave,
    /// `SPI_DMA_RX_ENA`, or `SPI_USR_CONF`, which starts a segmented transfer the DMA feeds.
    Dma,
    /// `SPI_DMA_TX_ENA` with no GDMA OUT channel started on SPI2, or more than one.
    DmaLink,
    /// A GDMA descriptor or its buffer outside the data-bus SRAM, one the owner check refuses,
    /// or a list that goes round a loop with no `suc_eof`.
    DmaDescriptor,
    /// A GDMA descriptor list that holds more or fewer bytes than the data phase sends.
    DmaLength,
    /// A nonzero `SPI_WR_BIT_ORDER`, or `SPI_RD_BIT_ORDER` on a read.
    LsbFirst,
    /// `SPI_QPI_MODE`.
    Qpi,
    /// `SPI_OPI_MODE` or an octal phase.
    Octal,
    /// One of `SPI_*_DTR_EN`.
    Dtr,
    /// `SPI_USR_DUMMY`.
    Dummy,
    /// `SPI_USR_MISO` with `SPI_FREAD_DUAL` or `SPI_FREAD_QUAD`, a read on more than one lane.
    Read,
    /// `SPI_USR_MOSI_HIGHPART` or `SPI_USR_MISO_HIGHPART`.
    HighPart,
    /// `SPI_SIO`, MOSI and MISO on one line.
    ThreeLine,
    /// `SPI_DOUTDIN` with the data on more than one lane.
    FullDuplex,
    /// `SPI_CK_OUT_EDGE`, which samples on the falling edge of SCLK in modes 1 and 2.
    ClockMode,
    /// `SPI_CS0_DIS`.
    CsDisabled,
    /// A data length that is not whole bytes, or more than the 64-byte buffer without DMA or
    /// with a read.
    DataLength,
    /// `SPI_CLK_EN` or `SPI_MST_CLK_ACTIVE` clear, so SPI2 has no clock.
    ClockOff,
    /// A register of [`SYNCED`] that differs from its value at the last `SPI_UPDATE`.
    Unsynced,
}

impl Setting {
    pub fn name(self) -> &'static str {
        match self {
            Setting::Slave => "slave",
            Setting::Dma => "dma",
            Setting::DmaLink => "dma_link",
            Setting::DmaDescriptor => "dma_descriptor",
            Setting::DmaLength => "dma_length",
            Setting::LsbFirst => "lsb_first",
            Setting::Qpi => "qpi",
            Setting::Octal => "octal",
            Setting::Dtr => "dtr",
            Setting::Dummy => "dummy",
            Setting::Read => "read",
            Setting::HighPart => "high_part",
            Setting::ThreeLine => "three_line",
            Setting::FullDuplex => "full_duplex",
            Setting::ClockMode => "clock_mode",
            Setting::CsDisabled => "cs_disabled",
            Setting::DataLength => "data_length",
            Setting::ClockOff => "clock_off",
            Setting::Unsynced => "unsynced",
        }
    }
}

/// One phase before the data: `bits` bits of `value`, most significant first, on `lanes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phase {
    pub bits: u8,
    pub lanes: u8,
    pub value: u32,
}

/// The pads the GPIO matrix routed SPI2's signals to when a transfer started. `None` for a
/// signal no pad carried.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pads {
    pub cs: Option<u8>,
    pub sclk: Option<u8>,
    /// D0 to D3. On a read with no pad driving `FSPIQ`, D1 is the pad its input reads.
    pub data: [Option<u8>; 4],
}

/// One SPI2 or SPI3 transfer as its registers set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// The host that sent it, `"SPI2"` or `"SPI3"`.
    pub peripheral: &'static str,
    /// The tick `SPI_USR` was written.
    pub start: Tick,
    /// The tick its last clock ends, rounded up to a CPU cycle, from which `SPI_USR` reads 0.
    pub end: Tick,
    /// The command phase, then the address phase, each when enabled.
    pub phases: Vec<Phase>,
    /// The data's lanes.
    pub lanes: u8,
    pub data: Vec<u8>,
    /// What it reads on MISO into `SPI_W0` on.
    pub miso: Miso,
    pub clock_hz: u32,
    /// Whether CS stays asserted after it.
    pub held: bool,
    pub pads: Pads,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Spi {
    /// 0 for SPI2, 1 for SPI3.
    port: usize,
    /// The registers from `SPI_CMD` to `SPI_DATE`, by offset over 4.
    regs: [u32; 61],
    /// The registers of [`SYNCED`] as the last `SPI_UPDATE` left them.
    synced: [u32; 11],
    /// `SPI_TRANS_DONE_INT_RAW` comes due at this tick, the last transfer's end. An access at
    /// or after it sets the bit.
    done_at: Option<Tick>,
    /// The transfers started and not yet taken, in the order they started.
    transfers: VecDeque<Transfer>,
}

impl Spi {
    pub(crate) fn new(port: usize) -> Spi {
        let mut regs = [0; 61];
        for (offset, value) in [
            (CTRL, WP_POL | HOLD_POL | D_POL | Q_POL),
            (CLOCK, 0x8000_3043),
            (USER, USR_COMMAND | CS_SETUP | CS_HOLD),
            (USER1, 0xb841_0007),
            (USER2, 0x7800_0000),
            (MISC, 0x3e),
            (SLAVE, 0x0280_0000),
            (DATE, 0x0210_1190),
        ] {
            regs[(offset / 4) as usize] = value;
        }
        let mut synced = [0; 11];
        synced.copy_from_slice(&regs[SYNCED]);
        Spi {
            port,
            regs,
            synced,
            done_at: None,
            transfers: VecDeque::new(),
        }
    }

    fn kept(offset: u32) -> bool {
        matches!(offset, CMD..=DMA_INT_SET | W0..=W15 | SLAVE..=CLK_GATE | DATE)
    }

    fn reg(&self, offset: u32) -> u32 {
        self.regs[(offset / 4) as usize]
    }

    /// Sets `SPI_TRANS_DONE_INT_RAW` once the last transfer has ended by `now`.
    fn settle(&mut self, now: Tick) {
        if self.done_at.is_some_and(|at| at <= now) {
            self.regs[(DMA_INT_RAW / 4) as usize] |= TRANS_DONE;
            self.done_at = None;
        }
    }

    /// Puts the registers back to power-on and drops a pending transfer-done bit. The transfers
    /// sent stay to be taken.
    pub(crate) fn reset(&mut self) {
        let transfers = std::mem::take(&mut self.transfers);
        *self = Spi {
            transfers,
            ..Spi::new(self.port)
        };
    }

    /// Whether a transfer runs at `now`.
    pub(crate) fn busy(&self, now: Tick) -> bool {
        self.transfers.back().is_some_and(|last| now < last.end)
    }

    /// The register at `offset` at `now`, or `None` for one the model does not keep.
    pub(crate) fn read(&mut self, now: Tick, offset: u32) -> Option<u32> {
        if !Self::kept(offset) {
            return None;
        }
        self.settle(now);
        let value = self.reg(offset);
        Some(match offset {
            CMD => (value & !(USR | UPDATE)) | if self.busy(now) { USR } else { 0 },
            DMA_CONF => (value & !AFIFO_RST) | FIFO_STATUS,
            DMA_INT_CLR | DMA_INT_SET => 0,
            DMA_INT_ST => self.reg(DMA_INT_RAW) & self.reg(DMA_INT_ENA),
            _ => value,
        })
    }

    /// Writes the register at `offset` at `now`. A write of `SPI_USR` starts a transfer, with
    /// its pads as `gpio` routes them and, with `SPI_DMA_TX_ENA`, the bytes `dma` gives for
    /// its data length, unless one runs. `Ok(true)` when it started one, and the setting that
    /// refused it otherwise, which leaves every register as it was. A write of `SPI_UPDATE`
    /// syncs the registers after any start in the same write.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        cpu: u64,
        offset: u32,
        value: u32,
        gpio: &Gpio,
        dma: impl FnOnce(usize) -> Result<Vec<u8>, Setting>,
    ) -> Result<bool, Setting> {
        debug_assert!(Self::kept(offset), "{offset:#x} is kept");
        self.settle(now);
        let start = offset == CMD && value & USR != 0 && !self.busy(now);
        if start {
            let transfer = self.transfer(now, cpu, gpio, dma)?;
            self.done_at = Some(transfer.end);
            self.transfers.push_back(transfer);
        }
        let reg = &mut self.regs[(offset / 4) as usize];
        match offset {
            CMD => *reg = value & !(USR | UPDATE),
            DMA_INT_CLR => self.regs[(DMA_INT_RAW / 4) as usize] &= !value,
            DMA_INT_RAW => *reg &= !value,
            DMA_INT_SET => self.regs[(DMA_INT_RAW / 4) as usize] |= value,
            DMA_INT_ST => {}
            DMA_CONF => *reg = value & !(AFIFO_RST | FIFO_STATUS),
            _ => *reg = value,
        }
        if offset == CMD && value & UPDATE != 0 {
            self.synced.copy_from_slice(&self.regs[SYNCED]);
        }
        Ok(start)
    }

    fn refused(&self) -> Option<Setting> {
        let (ctrl, user, misc) = (self.reg(CTRL), self.reg(USER), self.reg(MISC));
        let data_bits = (self.reg(MS_DLEN) & 0x3ffff) + 1;
        let clocked = CLK_EN | MST_CLK_ACTIVE;
        let reads = user & USR_MISO != 0;
        [
            (self.reg(SLAVE) & SLAVE_MODE != 0, Setting::Slave),
            (
                self.reg(DMA_CONF) & DMA_RX_ENA != 0 || self.reg(SLAVE) & USR_CONF != 0,
                Setting::Dma,
            ),
            (
                ctrl & WR_BIT_ORDER != 0 || reads && ctrl & RD_BIT_ORDER != 0,
                Setting::LsbFirst,
            ),
            (user & QPI_MODE != 0, Setting::Qpi),
            (
                user & (OPI_MODE | FWRITE_OCT) != 0
                    || ctrl & (FCMD_OCT | FADDR_OCT) != 0
                    || reads && ctrl & FREAD_OCT != 0,
                Setting::Octal,
            ),
            (misc & DTR != 0, Setting::Dtr),
            (user & USR_DUMMY != 0, Setting::Dummy),
            (
                reads && ctrl & (FREAD_DUAL | FREAD_QUAD) != 0,
                Setting::Read,
            ),
            (
                user & (USR_MOSI_HIGHPART | USR_MISO_HIGHPART) != 0,
                Setting::HighPart,
            ),
            (user & SIO != 0, Setting::ThreeLine),
            (
                user & DOUTDIN != 0 && user & (FWRITE_DUAL | FWRITE_QUAD) != 0,
                Setting::FullDuplex,
            ),
            (user & CK_OUT_EDGE != 0, Setting::ClockMode),
            (misc & CS0_DIS != 0, Setting::CsDisabled),
            (
                user & USR_MOSI != 0
                    && (!data_bits.is_multiple_of(8) || data_bits > self.data_limit())
                    || reads && (!data_bits.is_multiple_of(8) || data_bits > 8 * BUFFER as u32),
                Setting::DataLength,
            ),
            (self.reg(CLK_GATE) & clocked != clocked, Setting::ClockOff),
            (self.regs[SYNCED] != self.synced, Setting::Unsynced),
        ]
        .into_iter()
        .find_map(|(refused, setting)| refused.then_some(setting))
    }

    /// The most data bits one transfer sends: the buffer's, or with DMA what
    /// `SPI_MS_DATA_BITLEN` counts.
    fn data_limit(&self) -> u32 {
        if self.reg(DMA_CONF) & DMA_TX_ENA != 0 {
            DMA_BITS
        } else {
            8 * BUFFER as u32
        }
    }

    fn transfer(
        &self,
        now: Tick,
        cpu: u64,
        gpio: &Gpio,
        dma: impl FnOnce(usize) -> Result<Vec<u8>, Setting>,
    ) -> Result<Transfer, Setting> {
        if let Some(setting) = self.refused() {
            return Err(setting);
        }
        let (ctrl, user, user1, user2) = (
            self.reg(CTRL),
            self.reg(USER),
            self.reg(USER1),
            self.reg(USER2),
        );
        let lanes = |quad: bool, dual: bool| match (quad, dual) {
            (true, _) => 4,
            (false, true) => 2,
            (false, false) => 1,
        };
        let mut phases = Vec::new();
        if user & USR_COMMAND != 0 {
            let bits = (user2 >> 28) + 1;
            // The command goes out from bit 7 to bit 0 of its field, then from bit 15 to 8.
            let field = user2 & 0xffff;
            let sent = (field & 0xff) << 8 | field >> 8;
            phases.push(Phase {
                bits: bits as u8,
                lanes: lanes(ctrl & FCMD_QUAD != 0, ctrl & FCMD_DUAL != 0),
                value: sent >> (16 - bits),
            });
        }
        if user & USR_ADDR != 0 {
            let bits = (user1 >> 27) + 1;
            phases.push(Phase {
                bits: bits as u8,
                lanes: lanes(ctrl & FADDR_QUAD != 0, ctrl & FADDR_DUAL != 0),
                value: self.reg(ADDR).checked_shr(32 - bits).unwrap_or(0),
            });
        }
        let data_lanes = lanes(user & FWRITE_QUAD != 0, user & FWRITE_DUAL != 0);
        let bytes = ((self.reg(MS_DLEN) & 0x3ffff) + 1) as usize / 8;
        let data: Vec<u8> = if user & USR_MOSI == 0 {
            Vec::new()
        } else if self.reg(DMA_CONF) & DMA_TX_ENA != 0 {
            dma(bytes)?
        } else {
            self.regs[(W0 / 4) as usize..=(W15 / 4) as usize]
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .take(bytes)
                .collect()
        };
        let miso = match (
            user & USR_MISO != 0,
            user & DOUTDIN != 0 && !data.is_empty(),
        ) {
            (false, _) => Miso::None,
            (true, true) => Miso::Duplex,
            (true, false) => Miso::Read(bytes as u32),
        };
        let read = match miso {
            Miso::Read(n) => 8 * u64::from(n),
            Miso::None | Miso::Duplex => 0,
        };
        let setup = if user & CS_SETUP != 0 {
            u64::from((user1 >> 17) & 0x1f) + 1
        } else {
            0
        };
        let hold = if user & CS_HOLD != 0 {
            u64::from((user1 >> 22) & 0x1f)
        } else {
            0
        };
        let clocks = setup
            + phases
                .iter()
                .map(|phase| u64::from(phase.bits).div_ceil(u64::from(phase.lanes)))
                .sum::<u64>()
            + (8 * data.len() as u64).div_ceil(u64::from(data_lanes))
            + read
            + hold;
        let clock = self.reg(CLOCK);
        let divider = if clock & CLK_EQU_SYSCLK != 0 {
            1
        } else {
            (u64::from((clock >> 18) & 0xf) + 1) * (u64::from((clock >> 12) & 0x3f) + 1)
        };
        let source = if self.reg(CLK_GATE) & MST_CLK_SEL != 0 {
            PLL_F80M
        } else {
            XTAL_HZ
        };
        let ticks = clocks * divider * (TICK_HZ / source);
        let signals = SIGNALS[self.port];
        let q = signals.lanes[1];
        Ok(Transfer {
            peripheral: NAMES[self.port],
            start: now,
            end: Tick(now.0 + ticks.div_ceil(cpu) * cpu),
            phases,
            lanes: data_lanes,
            data,
            miso,
            clock_hz: u32::try_from(source / divider).expect("an SPI clock below 4 GHz"),
            held: self.reg(MISC) & CS_KEEP_ACTIVE != 0,
            pads: Pads {
                cs: gpio.routed(signals.cs0),
                sclk: gpio.routed(signals.clk),
                data: signals.lanes.map(|signal| match gpio.routed(signal) {
                    None if signal == q && miso != Miso::None => gpio.reads(q),
                    routed => routed,
                }),
            },
        })
    }

    /// Each output signal and the level its pads drive while it is routed: CS deasserted, SCLK
    /// idle, and each data line at its polarity bit.
    pub(crate) fn idle(&self) -> [(u16, bool); 6] {
        let (ctrl, misc) = (self.reg(CTRL), self.reg(MISC));
        let signals = SIGNALS[self.port];
        [
            (signals.cs0, misc & MASTER_CS0_POL == 0),
            (signals.clk, misc & CK_IDLE_EDGE != 0),
            (signals.lanes[0], ctrl & D_POL != 0),
            (signals.lanes[1], ctrl & Q_POL != 0),
            (signals.lanes[2], ctrl & WP_POL != 0),
            (signals.lanes[3], ctrl & HOLD_POL != 0),
        ]
    }

    /// Whether the interrupt line is high at `now`.
    pub(crate) fn line(&mut self, now: Tick) -> bool {
        self.settle(now);
        self.reg(DMA_INT_RAW) & self.reg(DMA_INT_ENA) != 0
    }

    /// The tick the interrupt line rises with no access, when a running transfer's end will
    /// raise it.
    pub(crate) fn next_event(&self) -> Option<Tick> {
        self.done_at
            .filter(|_| self.reg(DMA_INT_ENA) & TRANS_DONE != 0)
    }

    /// The end of the `n`th transfer not yet taken, counting from 0.
    pub(crate) fn end(&self, n: usize) -> Option<Tick> {
        self.transfers.get(n).map(|transfer| transfer.end)
    }

    /// Whether the `n`th transfer not yet taken reads bytes on MISO.
    pub(crate) fn reads(&self, n: usize) -> bool {
        self.transfers
            .get(n)
            .is_some_and(|transfer| transfer.miso != Miso::None)
    }

    /// Puts the bytes a transfer read into `SPI_W0` on, the first in the low byte of `SPI_W0`.
    pub(crate) fn answered(&mut self, miso: &[u8]) {
        debug_assert!(miso.len() <= BUFFER, "a read fits the buffer");
        let words = &mut self.regs[(W0 / 4) as usize..=(W15 / 4) as usize];
        for (k, &byte) in miso.iter().enumerate() {
            let shift = 8 * (k % 4);
            words[k / 4] = words[k / 4] & !(0xff << shift) | u32::from(byte) << shift;
        }
    }

    /// Takes the first `n` transfers.
    pub(crate) fn take(&mut self, n: usize) -> impl Iterator<Item = Transfer> + '_ {
        self.transfers.drain(..n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tick of crystal cycle `n`.
    fn c(n: u64) -> Tick {
        Tick(n * XTAL)
    }

    const CLOCKED: u32 = CLK_EN | MST_CLK_ACTIVE;

    fn no_dma(_: usize) -> Result<Vec<u8>, Setting> {
        panic!("a transfer from the buffer reads no DMA")
    }

    fn spi() -> (Spi, Gpio) {
        spi_port(0)
    }

    fn spi_port(port: usize) -> (Spi, Gpio) {
        let (mut spi, gpio) = (Spi::new(port), Gpio::new(&[]));
        set(&mut spi, &gpio, &[(CLK_GATE, CLOCKED)]);
        (spi, gpio)
    }

    /// Flips a bit that no earlier refusal reads. `MS_DLEN` moves by 8 so a one-byte
    /// transfer stays a whole number of bytes.
    fn differ(offset: u32, value: u32) -> u32 {
        let bit = match offset {
            CTRL => 1 << 23,
            USER => CS_SETUP,
            MS_DLEN => 8,
            MISC => CK_IDLE_EDGE,
            _ => 1,
        };
        value ^ bit
    }

    /// Makes `writes`, then syncs them with `SPI_UPDATE`.
    fn set(spi: &mut Spi, gpio: &Gpio, writes: &[(u32, u32)]) {
        for &(offset, value) in writes.iter().chain(&[(CMD, UPDATE)]) {
            assert_eq!(
                spi.write(c(0), XTAL, offset, value, gpio, no_dma),
                Ok(false)
            );
        }
    }

    /// A command of 8 bits, a 24-bit address and `data` on `lanes`, with CS kept, at 10 MHz from
    /// the crystal.
    fn command_and_data(spi: &mut Spi, gpio: &Gpio, data: &[u8], quad: bool) {
        let mut words = [0u32; 16];
        for (k, &byte) in data.iter().enumerate() {
            words[k / 4] |= u32::from(byte) << (8 * (k % 4));
        }
        let mut writes = vec![
            (CLOCK, 3 << 12 | 1 << 6 | 3),
            (
                USER,
                USR_COMMAND | USR_ADDR | USR_MOSI | if quad { FWRITE_QUAD } else { 0 },
            ),
            (USER1, 23 << 27),
            (USER2, 7 << 28 | 0x32),
            (ADDR, 0x2c << 16),
            (MS_DLEN, 8 * data.len() as u32 - 1),
            (MISC, CS_KEEP_ACTIVE),
        ];
        writes.extend((0..16).map(|k| (W0 + 4 * k, words[k as usize])));
        set(spi, gpio, &writes);
    }

    #[test]
    fn a_transfer_sends_its_phases_and_buffer_and_lasts_its_clocks() {
        let (mut spi, gpio) = spi();
        command_and_data(&mut spi, &gpio, &[0xab, 0xcd, 0xef], true);
        assert_eq!(spi.write(c(100), XTAL, CMD, USR, &gpio, no_dma), Ok(true));
        let transfer = spi.transfers.back().unwrap().clone();
        assert_eq!(
            transfer.phases,
            [
                Phase {
                    bits: 8,
                    lanes: 1,
                    value: 0x32
                },
                Phase {
                    bits: 24,
                    lanes: 1,
                    value: 0x2c00
                },
            ]
        );
        assert_eq!(
            (transfer.lanes, &transfer.data[..]),
            (4, &[0xab, 0xcd, 0xef][..])
        );
        assert!(transfer.held);
        // 8 + 24 clocks for the phases and 24 bits over 4 lanes, 38 clocks of 4 cycles.
        assert_eq!(transfer.clock_hz, 10_000_000);
        assert_eq!((transfer.start, transfer.end), (c(100), c(100 + 38 * 4)));
    }

    /// A command and a 3-byte read: the read adds its 24 clocks, and the bytes the target
    /// answered land in `SPI_W0` from its low byte. MISO is read on the pad `FSPIQ`'s input reads.
    #[test]
    fn a_read_phase_lasts_its_clocks_and_its_bytes_land_in_w0() {
        for (port, name) in [(0, "SPI2"), (1, "SPI3")] {
            let (mut spi, _) = spi_port(port);
            let mut gpio = Gpio::new(&[13]);
            let q = SIGNALS[port].lanes[1];
            gpio.write(0x154 + 4 * u32::from(q), 1 << 7 | 13).unwrap();
            set(
                &mut spi,
                &gpio,
                &[
                    (CLOCK, 3 << 12 | 1 << 6 | 3),
                    (USER, USR_COMMAND | USR_MISO),
                    (USER2, 7 << 28 | 0x9f),
                    (MS_DLEN, 8 * 3 - 1),
                    (W0, 0xaaaa_aaaa),
                ],
            );
            assert_eq!(spi.write(c(100), XTAL, CMD, USR, &gpio, no_dma), Ok(true));
            let transfer = spi.transfers.back().unwrap().clone();
            assert_eq!(transfer.peripheral, name);
            assert_eq!(transfer.miso, Miso::Read(3));
            assert!(transfer.data.is_empty());
            assert_eq!(transfer.pads.data[1], Some(13));
            // 8 clocks for the command and 24 for the read, of 4 cycles each.
            assert_eq!(transfer.end, c(100 + 32 * 4));
            spi.answered(&[0xef, 0x40, 0x18]);
            assert_eq!(spi.read(transfer.end, W0), Some(0xaa18_40ef));
        }
    }

    /// With `SPI_DOUTDIN` the read is one byte beside each byte it sends, and adds no clocks.
    #[test]
    fn a_full_duplex_transfer_reads_beside_its_data() {
        let (mut spi, gpio) = spi();
        command_and_data(&mut spi, &gpio, &[1, 2], false);
        set(
            &mut spi,
            &gpio,
            &[(USER, USR_COMMAND | USR_ADDR | USR_MOSI | USR_MISO | DOUTDIN)],
        );
        assert_eq!(spi.write(c(100), XTAL, CMD, USR, &gpio, no_dma), Ok(true));
        let transfer = spi.transfers.back().unwrap().clone();
        assert_eq!(transfer.miso, Miso::Duplex);
        assert_eq!(transfer.end, c(100 + (8 + 24 + 16) * 4));
    }

    #[test]
    fn usr_reads_1_and_done_reads_0_until_the_end() {
        let (mut spi, gpio) = spi();
        command_and_data(&mut spi, &gpio, &[1], false);
        spi.write(c(10), XTAL, CMD, USR, &gpio, no_dma).unwrap();
        let end = spi.end(0).unwrap();
        let done = |spi: &mut Spi, at| {
            (
                spi.read(Tick(at), CMD).unwrap() & USR != 0,
                spi.read(Tick(at), DMA_INT_RAW).unwrap() & TRANS_DONE != 0,
            )
        };
        assert_eq!(done(&mut spi, end.0 - XTAL), (true, false));
        assert_eq!(
            spi.write(Tick(end.0 - XTAL), XTAL, CMD, USR, &gpio, no_dma),
            Ok(false),
            "a start while one runs starts nothing"
        );
        assert_eq!(done(&mut spi, end.0), (false, true));
        spi.write(end, XTAL, DMA_INT_CLR, TRANS_DONE, &gpio, no_dma)
            .unwrap();
        assert_eq!(done(&mut spi, end.0), (false, false));
        assert_eq!(spi.take(1).count(), 1);
        assert_eq!(spi.end(0), None);
    }

    #[test]
    fn the_command_goes_out_low_byte_first_and_the_address_from_its_top_bit() {
        let (mut spi, gpio) = spi();
        set(
            &mut spi,
            &gpio,
            &[
                (USER, USR_COMMAND | USR_ADDR),
                (USER2, 11 << 28 | 0x0234),
                (USER1, 7 << 27),
                (ADDR, 0xa500_0000),
            ],
        );
        spi.write(c(0), XTAL, CMD, USR, &gpio, no_dma).unwrap();
        let transfer = spi.transfers.back().unwrap();
        // 12 bits of 0x34 then 0x02: 0011_0100 0000.
        assert_eq!(transfer.phases[0].value, 0x340);
        assert_eq!(transfer.phases[1].value, 0xa5);
        assert!(transfer.data.is_empty());
        assert!(!transfer.held);
    }

    #[test]
    fn each_setting_the_model_does_not_run_refuses_the_transfer_and_writes_nothing() {
        let user = USR_COMMAND | USR_ADDR | USR_MOSI;
        let ctrl = WP_POL | HOLD_POL | D_POL | Q_POL;
        let slave = 0x0280_0000;
        let rows = [
            (vec![(SLAVE, slave | SLAVE_MODE)], Setting::Slave),
            (vec![(DMA_CONF, DMA_RX_ENA)], Setting::Dma),
            (vec![(SLAVE, slave | USR_CONF)], Setting::Dma),
            (vec![(CTRL, ctrl | 1 << 25)], Setting::LsbFirst),
            (vec![(CTRL, ctrl | 2 << 25)], Setting::LsbFirst),
            (
                vec![(USER, user | USR_MISO), (CTRL, ctrl | 1 << 23)],
                Setting::LsbFirst,
            ),
            (vec![(USER, user | QPI_MODE)], Setting::Qpi),
            (vec![(USER, user | OPI_MODE)], Setting::Octal),
            (vec![(USER, user | FWRITE_OCT)], Setting::Octal),
            (vec![(CTRL, ctrl | FCMD_OCT)], Setting::Octal),
            (vec![(CTRL, ctrl | FADDR_OCT)], Setting::Octal),
            (
                vec![(USER, user | USR_MISO), (CTRL, ctrl | FREAD_OCT)],
                Setting::Octal,
            ),
            (vec![(MISC, CS_KEEP_ACTIVE | 1 << 16)], Setting::Dtr),
            (vec![(MISC, CS_KEEP_ACTIVE | 1 << 17)], Setting::Dtr),
            (vec![(MISC, CS_KEEP_ACTIVE | 1 << 18)], Setting::Dtr),
            (vec![(MISC, CS_KEEP_ACTIVE | 1 << 19)], Setting::Dtr),
            (vec![(USER, user | USR_DUMMY)], Setting::Dummy),
            (
                vec![(USER, user | USR_MISO), (CTRL, ctrl | FREAD_QUAD)],
                Setting::Read,
            ),
            (
                vec![(USER, user | USR_MISO), (CTRL, ctrl | FREAD_DUAL)],
                Setting::Read,
            ),
            (vec![(USER, user | USR_MOSI_HIGHPART)], Setting::HighPart),
            (vec![(USER, user | USR_MISO_HIGHPART)], Setting::HighPart),
            (vec![(USER, user | SIO)], Setting::ThreeLine),
            (
                vec![(USER, user | DOUTDIN | FWRITE_QUAD)],
                Setting::FullDuplex,
            ),
            (vec![(USER, user | CK_OUT_EDGE)], Setting::ClockMode),
            (vec![(MISC, CS_KEEP_ACTIVE | CS0_DIS)], Setting::CsDisabled),
            (vec![(MS_DLEN, 8 * 65 - 1)], Setting::DataLength),
            (vec![(MS_DLEN, 11)], Setting::DataLength),
            (
                vec![(USER, USR_COMMAND | USR_MISO), (MS_DLEN, 11)],
                Setting::DataLength,
            ),
            (vec![(CLK_GATE, 0)], Setting::ClockOff),
            (vec![(CLK_GATE, CLK_EN)], Setting::ClockOff),
            (
                vec![(CLK_GATE, MST_CLK_ACTIVE | MST_CLK_SEL)],
                Setting::ClockOff,
            ),
        ];
        for (writes, setting) in rows {
            let (mut spi, gpio) = spi();
            command_and_data(&mut spi, &gpio, &[1], false);
            set(&mut spi, &gpio, &writes);
            let before = spi.clone();
            assert_eq!(
                spi.write(c(0), XTAL, CMD, USR, &gpio, no_dma),
                Err(setting),
                "{writes:x?}"
            );
            assert_eq!(spi, before, "{} writes nothing", setting.name());
        }

        let (mut spi, gpio) = spi();
        command_and_data(&mut spi, &gpio, &[1], false);
        spi.write(c(0), XTAL, ADDR, 0, &gpio, no_dma).unwrap();
        assert_eq!(
            spi.write(c(0), XTAL, CMD, USR | UPDATE, &gpio, no_dma),
            Err(Setting::Unsynced),
            "a start in the update's own store uses the registers the last update synced"
        );
        spi.write(c(0), XTAL, CMD, UPDATE, &gpio, no_dma).unwrap();
        spi.write(c(0), XTAL, W0, 0xff, &gpio, no_dma).unwrap();
        assert_eq!(
            spi.write(c(0), XTAL, CMD, USR, &gpio, no_dma),
            Ok(true),
            "the buffer needs no update"
        );
    }

    #[test]
    fn each_synced_register_refuses_a_start_until_it_is_updated() {
        for offset in (ADDR..=DOUT_MODE).step_by(4) {
            let (mut spi, gpio) = spi();
            command_and_data(&mut spi, &gpio, &[1], false);
            let value = spi.read(c(0), offset).unwrap();
            spi.write(c(0), XTAL, offset, differ(offset, value), &gpio, no_dma)
                .unwrap();
            let before = spi.clone();
            assert_eq!(
                spi.write(c(0), XTAL, CMD, USR, &gpio, no_dma),
                Err(Setting::Unsynced),
                "offset {offset:#x} with no SPI_UPDATE"
            );
            assert_eq!(spi, before, "offset {offset:#x} writes nothing");
        }
    }

    #[test]
    fn the_settings_beside_the_refused_ones_still_start() {
        let (mut spi, gpio) = spi();
        command_and_data(&mut spi, &gpio, &[1], false);
        set(
            &mut spi,
            &gpio,
            &[
                (
                    CTRL,
                    WP_POL | HOLD_POL | D_POL | Q_POL | 3 << 23 | 0x7 << 14,
                ),
                (MISC, CS_KEEP_ACTIVE | CK_IDLE_EDGE | 0x3e),
                (DMA_INT_ENA, 0x1f_ffff),
                (CLK_GATE, CLOCKED | MST_CLK_SEL),
            ],
        );
        spi.write(c(0), XTAL, CMD, 0x3ffff, &gpio, no_dma).unwrap();
        assert_eq!(spi.write(c(0), XTAL, CMD, USR, &gpio, no_dma), Ok(true));
    }

    /// IDF's `esp_lcd` sends each command word as a 32-bit data phase from DMA, and its pixels
    /// in DMA transfers of up to 32,768 bytes on four lanes.
    #[test]
    fn a_dma_transfer_sends_the_bytes_the_channel_gives_for_its_data_length() {
        let (mut spi, gpio) = spi();
        let pixels = 32_768;
        set(
            &mut spi,
            &gpio,
            &[
                (CLOCK, 1 << 12 | 1),
                (CLK_GATE, CLOCKED | MST_CLK_SEL),
                (USER, USR_MOSI | FWRITE_QUAD),
                (MS_DLEN, 8 * pixels - 1),
                (DMA_CONF, DMA_TX_ENA),
            ],
        );
        let mut asked = None;
        let dma = |bytes: usize| {
            asked = Some(bytes);
            Ok((0..bytes).map(|k| k as u8).collect())
        };
        assert_eq!(spi.write(c(0), XTAL, CMD, USR, &gpio, dma), Ok(true));
        assert_eq!(asked, Some(pixels as usize));
        let transfer = spi.transfers.back().unwrap();
        assert_eq!(transfer.data.len(), pixels as usize);
        assert_eq!(transfer.data[..3], [0, 1, 2]);
        let clocks_on_four_lanes = 8 * u64::from(pixels) / 4;
        assert_eq!(spi.end(0), Some(c(clocks_on_four_lanes)));

        set(&mut spi, &gpio, &[(MS_DLEN, 8 * pixels - 2)]);
        let at = c(clocks_on_four_lanes);
        assert_eq!(
            spi.write(at, XTAL, CMD, USR, &gpio, no_dma),
            Err(Setting::DataLength)
        );
        set(&mut spi, &gpio, &[(DMA_CONF, 0), (MS_DLEN, 8 * 65 - 1)]);
        assert_eq!(
            spi.write(at, XTAL, CMD, USR, &gpio, no_dma),
            Err(Setting::DataLength),
            "65 bytes need DMA"
        );
    }

    #[test]
    fn a_dma_refusal_refuses_the_transfer_and_writes_nothing() {
        for setting in [Setting::DmaLink, Setting::DmaDescriptor, Setting::DmaLength] {
            let (mut spi, gpio) = spi();
            command_and_data(&mut spi, &gpio, &[1, 2, 3, 4], false);
            set(&mut spi, &gpio, &[(DMA_CONF, DMA_TX_ENA)]);
            let before = spi.clone();
            assert_eq!(
                spi.write(c(0), XTAL, CMD, USR, &gpio, |_| Err(setting)),
                Err(setting)
            );
            assert_eq!(spi, before, "{} writes nothing", setting.name());
        }
    }

    #[test]
    fn the_interrupt_line_rises_at_the_end_of_a_transfer_while_trans_done_is_enabled() {
        let (mut spi, gpio) = spi();
        command_and_data(&mut spi, &gpio, &[0x5a], false);
        set(&mut spi, &gpio, &[(DMA_INT_ENA, TRANS_DONE)]);
        spi.write(c(10), XTAL, CMD, USR, &gpio, no_dma).unwrap();
        let end = spi.end(0).unwrap();
        assert_eq!(spi.next_event(), Some(end));
        assert!(!spi.line(Tick(end.0 - 1)));
        assert!(spi.line(end));
        assert_eq!(spi.next_event(), None, "it rose");
        let write = |spi: &mut Spi, offset, value| {
            spi.write(end, XTAL, offset, value, &gpio, no_dma).unwrap();
        };
        write(&mut spi, DMA_INT_CLR, TRANS_DONE);
        assert!(!spi.line(end));
        write(&mut spi, DMA_INT_SET, TRANS_DONE);
        assert!(
            spi.line(end),
            "SPI_TRANS_DONE_INT_SET raises it, as spi_hal_init does"
        );
        write(&mut spi, DMA_INT_CLR, TRANS_DONE);
        write(&mut spi, DMA_INT_ENA, 0);
        write(&mut spi, CMD, USR);
        let second = spi.end(1).unwrap();
        assert_eq!(spi.next_event(), None, "not enabled");
        assert!(!spi.line(second));
        assert_eq!(spi.read(second, DMA_INT_RAW), Some(TRANS_DONE));
    }

    /// `spi_hal_init` sets `SPI_TRANS_DONE_INT_ENA`. A polling master then pulses
    /// `SPI_UPDATE`, sets `SPI_USR`, and waits on `SPI_TRANS_DONE_INT_RAW`.
    #[test]
    fn an_idf_polling_master_completes_its_transfer_and_sends_its_record() {
        let (mut spi, gpio) = spi();
        command_and_data(&mut spi, &gpio, &[0x5a], false);
        set(&mut spi, &gpio, &[(DMA_INT_ENA, TRANS_DONE)]);
        assert_eq!(spi.write(c(10), XTAL, CMD, USR, &gpio, no_dma), Ok(true));
        let end = spi.end(0).unwrap();
        assert_eq!(
            spi.read(Tick(end.0 - XTAL), DMA_INT_RAW).unwrap() & TRANS_DONE,
            0
        );
        assert_eq!(spi.read(end, DMA_INT_RAW).unwrap() & TRANS_DONE, TRANS_DONE);
        let sent: Vec<_> = spi.take(1).collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].data, [0x5a]);
    }

    /// Setup is `SPI_CS_SETUP_TIME` + 1 clocks, as IDF's `spi_ll_master_set_cs_setup` writes
    /// the field as one less than the clocks, and hold is `SPI_CS_HOLD_TIME` clocks, as
    /// `spi_ll_master_set_cs_hold` writes it. A clock is the source over
    /// (`SPI_CLKDIV_PRE` + 1)(`SPI_CLKCNT_N` + 1).
    #[test]
    fn a_transfer_lasts_its_cs_setup_its_phases_and_its_cs_hold_at_the_divided_clock() {
        let (mut spi, gpio) = spi();
        let (setup_time, hold_time, pre, n) = (2u64, 3u64, 2u64, 1u64);
        set(
            &mut spi,
            &gpio,
            &[
                (USER, USR_COMMAND | USR_MOSI | CS_SETUP | CS_HOLD),
                (USER2, 7 << 28),
                (USER1, (setup_time as u32) << 17 | (hold_time as u32) << 22),
                (MS_DLEN, 15),
                (CLOCK, (pre as u32) << 18 | (n as u32) << 12 | 1),
            ],
        );
        assert_eq!(spi.write(c(50), XTAL, CMD, USR, &gpio, no_dma), Ok(true));
        let transfer = spi.transfers.back().unwrap();
        let clocks = (setup_time + 1) + 8 + 16 + hold_time;
        let divider = (pre + 1) * (n + 1);
        assert_eq!(u64::from(transfer.clock_hz), XTAL_HZ / divider);
        assert_eq!(
            (transfer.start, transfer.end),
            (c(50), c(50 + clocks * divider))
        );
    }

    #[test]
    fn the_pll_source_halves_a_transfer() {
        let (mut spi, gpio) = spi();
        set(
            &mut spi,
            &gpio,
            &[(USER, USR_COMMAND), (USER2, 7 << 28), (CLOCK, 1 << 12 | 1)],
        );
        spi.write(c(0), XTAL, CMD, USR, &gpio, no_dma).unwrap();
        // 8 clocks at 20 MHz, plus setup and hold off: 16 cycles.
        assert_eq!(spi.end(0), Some(c(16)));
        set(&mut spi, &gpio, &[(CLK_GATE, CLOCKED | MST_CLK_SEL)]);
        spi.write(c(16), XTAL, CMD, USR, &gpio, no_dma).unwrap();
        assert_eq!(spi.end(1), Some(c(24)));
        assert_eq!(spi.transfers[1].clock_hz, 40_000_000);
    }

    #[test]
    fn the_idle_levels_follow_the_polarity_and_idle_edge_bits() {
        for (port, expected) in [
            (0, [110, 101, 103, 102, 105, 104]),
            (1, [71, 66, 68, 67, 70, 69]),
        ] {
            let (mut spi, gpio) = spi_port(port);
            assert_eq!(
                spi.idle(),
                [
                    (expected[0], true),
                    (expected[1], false),
                    (expected[2], true),
                    (expected[3], true),
                    (expected[4], true),
                    (expected[5], true),
                ]
            );
            set(
                &mut spi,
                &gpio,
                &[(MISC, CK_IDLE_EDGE | MASTER_CS0_POL), (CTRL, Q_POL)],
            );
            assert_eq!(
                spi.idle().map(|(_, level)| level),
                [false, true, false, true, false, false]
            );
        }
    }
}
