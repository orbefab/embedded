//! I2C0 and I2C1 as bus masters: each port's command list, both FIFOs, the master interrupts, and
//! the SCL and SDA waveform at pin level, timed in cycles of its clock from its own registers.
//!
//! The clock is the 40 MHz crystal over `I2C_SCLK_DIV_NUM + 1`. `I2C_CONF_UPGATE` copies the
//! timing registers into the timing a transfer runs with. Five lengths count their register plus
//! one: SCL low, the SDA hold after SCL falls, the SDA sample after SCL reads high, and the start
//! and stop setups and holds. `I2C_SCL_WAIT_HIGH_PERIOD` and `I2C_SCL_HIGH_PERIOD` count as
//! stored, so a bit is `low + wait_high + high` cycles, as IDF's `i2c_ll_master_set_bus_timing`
//! says the hardware measures.
//!
//! `I2C_TRANS_START` starts the list one cycle after its store. No source gives that delay, so it
//! is inferred. A STOP in the first command, which the driver writes to start a later run after a
//! NACK, makes the stop from the levels the pins already read: both high, it waits the setup and
//! the hold and raises `I2C_TRANS_COMPLETE_INT_RAW` with no edge. A bit drives SDA `sda_hold` after
//! SCL falls and releases SCL at `low`. The master then reads SCL through the input matrix: a pad
//! with no net reads its own output at once, and a netted pad reads high at the first input that
//! brings it high, `h`, so a target that holds SCL stretches the bit. The sample comes at
//! `h + sda_sample`, and SCL falls at `max(release + wait_high, h) + high`. With `I2C_TIME_OUT_EN`
//! set, a wait for SCL high that reaches `2^I2C_TIME_OUT_VALUE` cycles of this clock sets
//! `I2C_TIME_OUT_INT_RAW`, releases SDA, clears `I2C_BUS_BUSY` and ends the transfer with no
//! `I2C_TRANS_COMPLETE_INT_RAW` (TRM v1.8, 27.4.8, 27.6 and Register 27.12, which states
//! `I2C_SCLK` cycles). With `I2C_TIME_OUT_EN` clear, the wait does not time out. A read command
//! sends its `ack_val` as the ACK of every byte, as IDF's `I2C_MASTER_NACK`, "nack for each byte
//! read", says.
//!
//! An event that moves a netted pad's drive waits in `moved` for the chip's `Moved` halt, and no
//! later event runs before it, so each one reads the inputs of every edge before it. A write's
//! ninth bit that reads no ACK with `ack_en` set raises `I2C_NACK_INT_RAW`, and the master makes
//! the stop and clears `I2C_BUS_BUSY` with no `I2C_TRANS_COMPLETE_INT_RAW`. A list's stop raises
//! `I2C_TRANS_COMPLETE_INT_RAW`. While a NACK's automatic stop is running, `CMD_Controller` reads
//! no further command registers (TRM v1.8, 27.4.9), so a command-register store updates the
//! register without disturbing the stop. While a NACK's automatic stop runs or a standalone `STOP`
//! command started right after a NACK runs, and `COMD0` holds `STOP`, a `CONF_UPGATE` with
//! unchanged `CTR` and timing registers is a no-op and a `TRANS_START` is ignored, as IDF v5.5.5's
//! `s_i2c_send_commands` writes `COMD0 = STOP` and two `i2c_hal_master_trans_start` calls on
//! `I2C_STATUS_ACK_ERROR`. Reading `I2C_DATA_REG` from an empty RX FIFO returns 0 and sets
//! `I2C_RXFIFO_UDF_INT_RAW` (TRM v1.8, 27.6 and Register 27.22).
//!
//! An END command, as the ESP32-S3 TRM v1.8 (27.4.9 and 27.5.4) describes it, marks its command
//! done, raises `I2C_END_DETECT_INT_RAW` and stops the list with SCL held low and the bus busy.
//! IDF's driver splits a write or read longer than the 32-byte FIFO this way, and refills the
//! FIFO and the commands between parts. The next `I2C_TRANS_START` resumes the list from
//! `I2C_COMD0` with any command, one cycle after its store as a first start does, and that
//! command clocks from that tick: SDA moves `sda_hold` later and SCL releases `low` later.
//! The TRM says only that END extends SCL low, so that resume timing is inferred.
//!
//! `I2C_RX_FULL_ACK_LEVEL` is the ACK the master sends when `I2C_RXFIFO_CNT` reaches its
//! threshold, the TRM's Register 27.11 says, and 27.4.3 that it replaces the ACK when RX RAM is
//! full. The model takes that as the ACK of the byte that fills the 32-byte FIFO, and refuses a
//! read whose command asks a different level there. IDF clears it, and its parts never end a
//! read with a NACK on a full FIFO.
//!
//! The registers come from IDF v5.5.5's `soc/esp32s3/register/soc/i2c_reg.h`, with the reset
//! values its field comments give. Only IDF's master setup runs, and every other setting the
//! model meets ends the run by name.

use super::clock::{Tick, XTAL};
use super::gpio::Gpio;

/// Base addresses of I2C0 and I2C1 (`DR_REG_I2C_EXT_BASE`, `DR_REG_I2C1_EXT_BASE`).
pub(crate) const I2C0: u32 = 0x6001_3000;
pub(crate) const I2C1: u32 = 0x6002_7000;
pub(crate) const BASES: [u32; 2] = [I2C0, I2C1];
pub(crate) const NAMES: [&str; 2] = ["I2C0", "I2C1"];
/// `(SCL, SDA)` GPIO matrix signals for I2C0 and I2C1 (`I2CEXTn_SCL_*_IDX`, `I2CEXTn_SDA_*_IDX`),
/// from `soc/esp32s3/include/soc/gpio_sig_map.h`.
pub(crate) const SIGNALS: [(u16, u16); 2] = [(89, 90), (91, 92)];

/// Port index for the peripheral block base `base`, or `None` when `base` is not an I2C port.
pub(crate) fn index(base: u32) -> Option<usize> {
    BASES.iter().position(|&b| b == base)
}

const SCL_LOW_PERIOD: u32 = 0x00;
const CTR: u32 = 0x04;
const SR: u32 = 0x08;
const TO: u32 = 0x0c;
const FIFO_CONF: u32 = 0x18;
pub(crate) const DATA: u32 = 0x1c;
const INT_RAW: u32 = 0x20;
const INT_CLR: u32 = 0x24;
const INT_ENA: u32 = 0x28;
const INT_STATUS: u32 = 0x2c;
const SDA_HOLD: u32 = 0x30;
const SDA_SAMPLE: u32 = 0x34;
const SCL_HIGH_PERIOD: u32 = 0x38;
const SCL_START_HOLD: u32 = 0x40;
const SCL_RSTART_SETUP: u32 = 0x44;
const SCL_STOP_HOLD: u32 = 0x48;
const SCL_STOP_SETUP: u32 = 0x4c;
const FILTER_CFG: u32 = 0x50;
const CLK_CONF: u32 = 0x54;
const COMD0: u32 = 0x58;
const COMMANDS: usize = 8;
const SCL_SP_CONF: u32 = 0x80;
const SCL_STRETCH_CONF: u32 = 0x84;
const WORDS: usize = (SCL_STRETCH_CONF / 4 + 1) as usize;

/// The registers a transfer's timing comes from.
const TIMING: [u32; 10] = [
    SCL_LOW_PERIOD,
    TO,
    SDA_HOLD,
    SDA_SAMPLE,
    SCL_HIGH_PERIOD,
    SCL_START_HOLD,
    SCL_RSTART_SETUP,
    SCL_STOP_HOLD,
    SCL_STOP_SETUP,
    CLK_CONF,
];

/// `I2C_CTR_REG`'s fields.
const SDA_FORCE_OUT: u32 = 1 << 0;
const SCL_FORCE_OUT: u32 = 1 << 1;
const SAMPLE_SCL_LEVEL: u32 = 1 << 2;
const MS_MODE: u32 = 1 << 4;
const TRANS_START: u32 = 1 << 5;
const TX_LSB_FIRST: u32 = 1 << 6;
const RX_LSB_FIRST: u32 = 1 << 7;
const ARBITRATION_EN: u32 = 1 << 9;
const FSM_RST: u32 = 1 << 10;
const CONF_UPGATE: u32 = 1 << 11;
const CTR_STROBES: u32 = TRANS_START | FSM_RST | CONF_UPGATE;
const RX_FULL_ACK_LEVEL: u32 = 1 << 3;
/// `SLV_TX_AUTO_START_EN`, `ADDR_10BIT_RW_CHECK_EN` and `ADDR_BROADCASTING_EN`, kept and not run:
/// they set a slave, and the model runs none. `RX_FULL_ACK_LEVEL` runs as the module header says,
/// and `CLK_EN` gates only the register interface's APB clock, TRM Register 27.11, so it changes
/// nothing on the bus. A write that changes either of those two is quiet.
pub(crate) const CTR_UNRUN: u32 = 1 << 12 | 1 << 13 | 1 << 14;

/// `I2C_SR_REG`'s `I2C_BUS_BUSY`, and its `I2C_STRETCH_CAUSE` at its reset value.
const BUS_BUSY: u32 = 1 << 4;
const STRETCH_CAUSE: u32 = 3 << 14;

const TIME_OUT_VALUE: u32 = 0x1f;
const TIME_OUT_EN: u32 = 1 << 5;

const NONFIFO_EN: u32 = 1 << 10;
/// The FIFO thresholds, `FIFO_ADDR_CFG_EN` and `FIFO_PRT_EN`, kept and not run.
pub(crate) const FIFO_CONF_UNRUN: u32 = 0x3ff | 1 << 11 | 1 << 14;
const RX_FIFO_RST: u32 = 1 << 12;
const TX_FIFO_RST: u32 = 1 << 13;

/// The interrupts IDF's master driver enables, `I2C_LL_MASTER_EVENT_INTR`: `END_DETECT`,
/// `ARBITRATION_LOST`, `TRANS_COMPLETE`, `TIME_OUT` and `NACK`.
const MASTER_EVENTS: u32 = END_DETECT | 1 << 5 | TRANS_COMPLETE | TIME_OUT | NACK;
const END_DETECT: u32 = 1 << 3;
const TRANS_COMPLETE: u32 = 1 << 7;
const TIME_OUT: u32 = 1 << 8;
const NACK: u32 = 1 << 10;
const RXFIFO_UDF: u32 = 1 << 12;

const SCL_FILTER_EN: u32 = 1 << 8;
const SDA_FILTER_EN: u32 = 1 << 9;

const SCLK_DIV_NUM: u32 = 0xff;
const SCLK_DIV_AB: u32 = 0xfff << 8;
const SCLK_SEL: u32 = 1 << 20;
const SCLK_ACTIVE: u32 = 1 << 21;

const SCL_RST_SLV_EN: u32 = 1 << 0;
const PD_EN: u32 = 1 << 6 | 1 << 7;

const COMMAND: u32 = 0x3fff;
const COMMAND_DONE: u32 = 1 << 31;

/// The bytes each FIFO holds, `SOC_I2C_FIFO_LEN`.
const FIFO: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct I2c {
    /// This port's GPIO matrix signals `(SCL, SDA)`.
    scl: u16,
    sda: u16,
    /// The words as stored, the timing registers among them.
    regs: [u32; WORDS],
    /// The timing the bus runs with, as `CONF_UPGATE` last took it.
    timing: Timing,
    tx: Fifo,
    rx: Fifo,
    int_raw: u32,
    /// The level this master puts on its SDA output. `true` releases the line.
    sda_out: bool,
    run: Option<Run>,
    /// An END ran, so SCL stays low and the bus busy until a `TRANS_START` resumes the list.
    held: bool,
    /// From the start this master made to the stop it made.
    busy: bool,
    /// The last finished transfer ended in a NACK, so a standalone `STOP` started next is IDF's
    /// NACK recovery stop.
    after_nack: bool,
    /// The drive move that waits for its halt: its tick, signal and level.
    moved: Option<(Tick, u16, bool)>,
    /// The setting a run met that the model does not run, and its tick.
    refused: Option<(Tick, &'static str)>,
    /// The level SCL and SDA last read, and the tick that level began. Both idle
    /// released, so a high line is not an edge at the first sample.
    seen: [(bool, Tick); 2],
}

/// The lengths a transfer runs with, in cycles of the I2C clock, each the length the hardware
/// counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    /// Ticks per cycle of the I2C clock.
    cycle: u64,
    low: u64,
    wait_high: u64,
    high: u64,
    sda_hold: u64,
    sda_sample: u64,
    start_hold: u64,
    rstart_setup: u64,
    stop_setup: u64,
    stop_hold: u64,
    /// `2^TIME_OUT_VALUE`, or `None` with `TIME_OUT_EN` clear.
    timeout: Option<u64>,
}

impl Timing {
    fn from_regs(regs: &[u32; WORDS]) -> Timing {
        let reg = |offset: u32| u64::from(regs[(offset / 4) as usize]);
        let field = |offset| reg(offset) & 0x1ff;
        let to = reg(TO) as u32;
        Timing {
            cycle: XTAL * ((reg(CLK_CONF) & u64::from(SCLK_DIV_NUM)) + 1),
            low: field(SCL_LOW_PERIOD) + 1,
            wait_high: (reg(SCL_HIGH_PERIOD) >> 9) & 0x7f,
            high: field(SCL_HIGH_PERIOD),
            sda_hold: field(SDA_HOLD) + 1,
            sda_sample: field(SDA_SAMPLE) + 1,
            start_hold: field(SCL_START_HOLD) + 1,
            rstart_setup: field(SCL_RSTART_SETUP) + 1,
            stop_setup: field(SCL_STOP_SETUP) + 1,
            stop_hold: field(SCL_STOP_HOLD) + 1,
            timeout: (to & TIME_OUT_EN != 0).then(|| 1 << (to & TIME_OUT_VALUE)),
        }
    }

    /// The tick `cycles` cycles after `at`.
    fn after(&self, at: Tick, cycles: u64) -> Tick {
        Tick(at.0.saturating_add(cycles.saturating_mul(self.cycle)))
    }
}

/// A FIFO of 32 bytes held inline, so a copy of the chip stays a copy of its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fifo {
    bytes: [u8; FIFO],
    head: u8,
    len: u8,
}

impl Fifo {
    const EMPTY: Fifo = Fifo {
        bytes: [0; FIFO],
        head: 0,
        len: 0,
    };

    fn push(&mut self, byte: u8) -> bool {
        if usize::from(self.len) == FIFO {
            return false;
        }
        self.bytes[(usize::from(self.head) + usize::from(self.len)) % FIFO] = byte;
        self.len += 1;
        true
    }

    fn first(&self) -> Option<u8> {
        (self.len > 0).then(|| self.bytes[usize::from(self.head)])
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.bytes[usize::from(self.head)];
        self.head = ((usize::from(self.head) + 1) % FIFO) as u8;
        self.len -= 1;
        Some(byte)
    }
}

/// One `I2C_COMDn` word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Rstart,
    Write { n: u8, check: bool, expect: bool },
    Read { n: u8, nack: bool },
    Stop,
    End,
}

impl Op {
    /// The command in `word`, or the setting that refuses it.
    fn of(word: u32) -> Result<Op, &'static str> {
        let n = (word & 0xff) as u8;
        let op = match (word >> 11) & 0x7 {
            6 => Op::Rstart,
            1 => Op::Write {
                n,
                check: word & 1 << 8 != 0,
                expect: word & 1 << 9 != 0,
            },
            3 => Op::Read {
                n,
                nack: word & 1 << 10 != 0,
            },
            2 => Op::Stop,
            4 => Op::End,
            _ => return Err("opcode"),
        };
        match op {
            Op::Write { n: 0, .. } | Op::Read { n: 0, .. } => Err("opcode"),
            op => Ok(op),
        }
    }
}

/// A transfer, from `TRANS_START` to the end of its last command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    cmd: usize,
    op: Op,
    phase: Phase,
    /// What the clock being made carries.
    slot: Slot,
    /// The tick of the next thing the run does on its own, `None` while it waits for SCL to read
    /// high.
    next: Option<Tick>,
    /// When a wait for SCL high gives up.
    deadline: Option<Tick>,
    /// The tick this master last released SCL, and the tick SCL first read high after it.
    release: Tick,
    h: Tick,
    /// The bit of the byte, 8 for its ACK, the byte, and the bytes left in the command.
    bit: u8,
    byte: u8,
    left: u8,
    /// A write read no ACK, so the run makes its stop and ends with no `TRANS_COMPLETE`.
    nack: bool,
    /// A standalone `STOP` started right after a transfer ended in a NACK.
    nack_stop: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// SDA falls, then SCL falls `start_hold` later.
    Start(Edge),
    /// SCL low. SDA moves at `sda_hold` and SCL releases at `low`.
    Low { sda_set: bool },
    /// A `TRANS_START` after an END: the first command begins at this event.
    Resume,
    /// SCL released, until an input reads it high. Its only event is the deadline.
    Rising,
    /// SCL high. The sample comes at `sda_sample` and SCL falls after it.
    High { sampled: bool },
    /// SDA releases `stop_setup` after SCL reads high, and the stop ends `stop_hold` later.
    Stop { released: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Sda,
    Scl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Bit,
    Ack,
    Restart,
    Stop,
}

/// The word at `offset` of `regs`.
fn word(regs: &[u32; WORDS], offset: u32) -> u32 {
    regs[(offset / 4) as usize]
}

/// What the register at `offset` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Kept,
    /// It reads what the run holds now.
    Status,
    /// It reads 0.
    Strobe,
}

fn kind(offset: u32) -> Option<Kind> {
    Some(match offset {
        SR | DATA | INT_RAW | INT_STATUS => Kind::Status,
        INT_CLR => Kind::Strobe,
        SCL_LOW_PERIOD | CTR | TO | 0x10 | FIFO_CONF | INT_ENA => Kind::Kept,
        SDA_HOLD..=SCL_HIGH_PERIOD | SCL_START_HOLD..=SCL_STRETCH_CONF => Kind::Kept,
        _ => return None,
    })
}

impl I2c {
    pub(crate) fn new(port: usize) -> I2c {
        let (scl, sda) = SIGNALS[port];
        let mut regs = [0; WORDS];
        for (offset, value) in [
            (
                CTR,
                SDA_FORCE_OUT | SCL_FORCE_OUT | RX_FULL_ACK_LEVEL | ARBITRATION_EN,
            ),
            (TO, 0x10),
            (FIFO_CONF, 0x408b),
            (SCL_START_HOLD, 8),
            (SCL_RSTART_SETUP, 8),
            (SCL_STOP_HOLD, 8),
            (SCL_STOP_SETUP, 8),
            (FILTER_CFG, SCL_FILTER_EN | SDA_FILTER_EN),
            (CLK_CONF, SCLK_ACTIVE),
            (0x78, 0x10),
            (0x7c, 0x10),
        ] {
            regs[(offset / 4) as usize] = value;
        }
        I2c {
            scl,
            sda,
            timing: Timing::from_regs(&regs),
            regs,
            tx: Fifo::EMPTY,
            rx: Fifo::EMPTY,
            int_raw: 0,
            sda_out: true,
            run: None,
            held: false,
            busy: false,
            after_nack: false,
            moved: None,
            refused: None,
            seen: [(true, Tick(0)); 2],
        }
    }

    fn reg(&self, offset: u32) -> u32 {
        word(&self.regs, offset)
    }

    /// The register at `offset` at `now`, or `None` for one not kept. `I2C_DATA_REG` reads the
    /// RX FIFO's first byte, which [`I2c::take`] takes.
    pub(crate) fn read(&mut self, now: Tick, offset: u32, gpio: &mut Gpio) -> Option<u32> {
        let kind = kind(offset)?;
        self.settle(now, gpio);
        Some(match (kind, offset) {
            (Kind::Status, SR) => {
                let busy = if self.busy { BUS_BUSY } else { 0 };
                busy | STRETCH_CAUSE | u32::from(self.rx.len) << 8 | u32::from(self.tx.len) << 18
            }
            (Kind::Status, DATA) => self.rx.first().map_or(0, u32::from),
            (Kind::Status, INT_RAW) => self.int_raw,
            (Kind::Status, _) => self.int_raw & self.reg(INT_ENA),
            (Kind::Strobe, _) => 0,
            (Kind::Kept, CTR) => self.reg(CTR) & !CTR_STROBES,
            (Kind::Kept, _) => self.reg(offset),
        })
    }

    /// Takes the RX FIFO's first byte, as a load of `I2C_DATA_REG` does. Reading an empty RX FIFO
    /// sets `I2C_RXFIFO_UDF_INT_RAW` (TRM v1.8, 27.6 and Register 27.22) and leaves it empty.
    pub(crate) fn take(&mut self) {
        if self.rx.pop().is_none() {
            self.int_raw |= RXFIFO_UDF;
        }
    }

    /// Writes the register at `offset` at `now`. `Ok(false)` for one not kept, which drops the
    /// write, and `Err` names the setting the model does not run, writing nothing.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
        gpio: &mut Gpio,
    ) -> Result<bool, &'static str> {
        let Some(kind) = kind(offset) else {
            return Ok(false);
        };
        self.settle(now, gpio);
        let running = self.run.is_some();
        let nack_stopping = self.run.is_some_and(|run| run.nack);
        let nack_recovery = self.run.is_some_and(|run| run.nack || run.nack_stop)
            && Op::of(self.reg(COMD0) & COMMAND) == Ok(Op::Stop);
        let busy_write = match offset {
            CTR if value & (TRANS_START | CONF_UPGATE) != 0 => {
                let same_ctr = value & !CTR_STROBES == self.reg(CTR) & !CTR_STROBES;
                let same_timing =
                    value & CONF_UPGATE == 0 || Timing::from_regs(&self.regs) == self.timing;
                !(nack_recovery && same_ctr && same_timing)
            }
            CTR => false,
            FIFO_CONF => value & (TX_FIFO_RST | RX_FIFO_RST) != 0,
            _ if Self::command(offset).is_some() => !nack_stopping,
            _ => TIMING.contains(&offset),
        };
        let refused = [
            (offset == CTR && value & FSM_RST != 0, "fsm_reset"),
            (running && busy_write, "busy_write"),
            (offset == FIFO_CONF && value & NONFIFO_EN != 0, "nonfifo"),
            (offset == CLK_CONF && value & SCLK_SEL != 0, "rc_fast_clock"),
            (
                offset == CLK_CONF && value & SCLK_DIV_AB != 0,
                "fractional_divider",
            ),
            (
                offset == SCL_SP_CONF && value & SCL_RST_SLV_EN != 0,
                "clear_bus",
            ),
            (offset == SCL_SP_CONF && value & PD_EN != 0, "power_down"),
            (offset == INT_ENA && value & !MASTER_EVENTS != 0, "int_ena"),
        ];
        if let Some(&(_, setting)) = refused.iter().find(|&&(refused, _)| refused) {
            return Err(setting);
        }
        match (kind, offset) {
            (Kind::Status, DATA) => {
                if !self.tx.push(value as u8) {
                    return Err("tx_overflow");
                }
            }
            (Kind::Status | Kind::Strobe, INT_RAW | INT_CLR) => self.int_raw &= !value,
            (Kind::Status | Kind::Strobe, _) => {}
            (Kind::Kept, _) => {
                let mut regs = self.regs;
                regs[(offset / 4) as usize] = value;
                let timing = if offset == CTR && value & CONF_UPGATE != 0 {
                    Timing::from_regs(&regs)
                } else {
                    self.timing
                };
                let start = !running && offset == CTR && value & TRANS_START != 0;
                let op = start
                    .then(|| Self::startable(&regs, timing, gpio, self.held, self.scl, self.sda))
                    .transpose()?;
                self.regs = regs;
                self.timing = timing;
                if offset == FIFO_CONF {
                    if value & TX_FIFO_RST != 0 {
                        self.tx = Fifo::EMPTY;
                    }
                    if value & RX_FIFO_RST != 0 {
                        self.rx = Fifo::EMPTY;
                    }
                }
                if let Some(op) = op {
                    self.start(now, op, gpio);
                }
            }
        }
        Ok(true)
    }

    /// The index of the `I2C_COMDn` register at `offset`.
    fn command(offset: u32) -> Option<usize> {
        let k = (offset.checked_sub(COMD0)? / 4) as usize;
        (offset.is_multiple_of(4) && k < COMMANDS).then_some(k)
    }

    /// The first command of a list that `regs` and `timing` would start, or the setting that
    /// refuses it. The bus must be IDF's open-drain master on two routed pads, and the timing
    /// must keep each SDA move inside its SCL level. A list on an idle bus starts with a
    /// repeated START or a STOP, and one `held` after an END with any command.
    fn startable(
        regs: &[u32; WORDS],
        timing: Timing,
        gpio: &Gpio,
        held: bool,
        scl: u16,
        sda: u16,
    ) -> Result<Op, &'static str> {
        let ctr = word(regs, CTR);
        let pads = [scl, sda].map(|signal| {
            let pad = gpio.routed(signal)?;
            let both = gpio.carries(usize::from(pad)) == Some((signal, false))
                && gpio.reads(signal) == Some(pad);
            both.then_some(pad)
        });
        let refused = [
            (ctr & MS_MODE == 0, "slave_mode"),
            (pads.contains(&None), "not_routed"),
            (
                ctr & (SDA_FORCE_OUT | SCL_FORCE_OUT) != SDA_FORCE_OUT | SCL_FORCE_OUT
                    || pads.iter().flatten().any(|&pad| !gpio.open_drain(pad)),
                "push_pull",
            ),
            (ctr & (TX_LSB_FIRST | RX_LSB_FIRST) != 0, "lsb_first"),
            (ctr & ARBITRATION_EN != 0, "arbitration"),
            (ctr & SAMPLE_SCL_LEVEL != 0, "sample_scl_level"),
            (word(regs, CLK_CONF) & SCLK_ACTIVE == 0, "clock_off"),
            (
                timing.sda_hold >= timing.low || timing.sda_sample >= timing.high,
                "timing",
            ),
        ];
        if let Some(&(_, setting)) = refused.iter().find(|&&(refused, _)| refused) {
            return Err(setting);
        }
        match Op::of(word(regs, COMD0))? {
            op @ (Op::Rstart | Op::Stop) => Ok(op),
            op if held => Ok(op),
            _ => Err("opcode"),
        }
    }

    /// Starts the command list at `now` with its first command `op`: a repeated START or a STOP
    /// on an idle bus, and any command on one an END holds.
    fn start(&mut self, now: Tick, op: Op, gpio: &Gpio) {
        let timing = self.timing;
        let after_nack = std::mem::take(&mut self.after_nack);
        self.observe(now, gpio);
        if self.held {
            self.held = false;
            self.run = Some(Run {
                cmd: 0,
                op,
                phase: Phase::Resume,
                slot: Slot::Bit,
                next: Some(timing.after(now, 1)),
                deadline: None,
                release: now,
                h: now,
                bit: 0,
                byte: 0,
                left: 0,
                nack: false,
                nack_stop: false,
            });
            return;
        }
        if op == Op::Stop {
            self.start_stop(now, gpio, after_nack);
            return;
        }
        self.run = Some(Run {
            cmd: 0,
            op,
            phase: Phase::Start(Edge::Sda),
            slot: Slot::Restart,
            next: Some(timing.after(now, 1)),
            deadline: None,
            release: now,
            h: now,
            bit: 0,
            byte: 0,
            left: 0,
            nack: false,
            nack_stop: false,
        });
    }

    /// A STOP that is the first command. Both lines already high, the bus is idle and the
    /// command only waits its setup and hold. SCL still low, it makes the stop from this tick.
    /// SDA still low while SCL is high, it releases SDA after the setup.
    fn start_stop(&mut self, now: Tick, gpio: &Gpio, nack_stop: bool) {
        let timing = self.timing;
        let scl_high = gpio.input(self.scl) == Some(true);
        let sda_high = gpio.input(self.sda) == Some(true);
        self.busy = true;
        let mut run = Run {
            cmd: 0,
            op: Op::Stop,
            phase: Phase::Stop { released: true },
            slot: Slot::Stop,
            next: None,
            deadline: None,
            release: now,
            h: now,
            bit: 0,
            byte: 0,
            left: 0,
            nack: false,
            nack_stop,
        };
        if scl_high && sda_high {
            let wait = timing.stop_setup + timing.stop_hold;
            run.next = Some(timing.after(now, wait));
        } else if scl_high {
            run.phase = Phase::Stop { released: false };
            run.next = Some(timing.after(now, timing.stop_setup));
        } else {
            self.clock(&mut run, now, Slot::Stop);
        }
        self.run = Some(run);
    }

    /// Whether a transfer runs.
    pub(crate) fn runs(&self) -> bool {
        self.run.is_some()
    }

    /// Whether nothing waits to settle: no transfer and no refusal. A drive move waits only
    /// inside a transfer, or after a step that a refusal ended.
    pub(crate) fn idle(&self) -> bool {
        self.run.is_none() && self.refused.is_none()
    }

    /// Whether the interrupt line is high.
    pub(crate) fn line(&self) -> bool {
        self.int_raw & self.reg(INT_ENA) != 0
    }

    /// The tick of the run's next event: its next step, or the deadline of its wait for SCL.
    pub(crate) fn next_event(&self) -> Option<Tick> {
        if self.refused.is_some() {
            return None;
        }
        let run = self.run.as_ref()?;
        run.next.or(run.deadline)
    }

    /// The drive move that waits for its halt.
    pub(crate) fn moved(&self) -> Option<(Tick, u16, bool)> {
        self.moved
    }

    /// Lets the move at `at` land, so the run goes on.
    pub(crate) fn land(&mut self, at: Tick) -> Option<(u16, bool)> {
        let (_, signal, level) = self.moved.take_if(|&mut (tick, ..)| tick == at)?;
        Some((signal, level))
    }

    /// Takes the setting a run met that the model does not run, with its tick.
    pub(crate) fn take_refused(&mut self) -> Option<(Tick, &'static str)> {
        self.refused.take().inspect(|_| self.run = None)
    }

    /// Runs every event due at or before `through`, until one waits for its drive to land.
    pub(crate) fn settle(&mut self, through: Tick, gpio: &mut Gpio) {
        while self.moved.is_none()
            && let Some(at) = self.next_event().filter(|&at| at <= through)
        {
            if let Err(setting) = self.step(at, gpio) {
                self.refused = Some((at, setting));
            }
        }
    }

    /// The levels of the netted pads from `at` on, as the chip took them. A run waiting for SCL
    /// to read high takes `at` as the start of the high side.
    pub(crate) fn input(&mut self, at: Tick, gpio: &Gpio) {
        if self.refused.is_some() {
            return;
        }
        self.observe(at, gpio);
        if let Some(mut run) = self.run
            && run.phase == Phase::Rising
            && gpio.input(self.scl) == Some(true)
        {
            self.high(&mut run, at);
            self.run = Some(run);
        }
    }

    /// Takes the levels SCL and SDA read at `at`. While this master holds the bus with the filter
    /// on, a level that held for fewer cycles than the filter's threshold is a pulse the hardware
    /// would drop, which the model refuses, since it does not run the filter.
    fn observe(&mut self, at: Tick, gpio: &Gpio) {
        let filter = self.reg(FILTER_CFG);
        for (k, signal) in [self.scl, self.sda].into_iter().enumerate() {
            let Some(level) = gpio.input(signal) else {
                continue;
            };
            let (was, since) = self.seen[k];
            if level == was {
                continue;
            }
            let threshold = u64::from((filter >> (4 * k)) & 0xf) * self.timing.cycle;
            let on = filter & [SCL_FILTER_EN, SDA_FILTER_EN][k] != 0;
            if self.busy && on && at.0.saturating_sub(since.0) < threshold {
                self.refused.get_or_insert((at, "glitch"));
            }
            self.seen[k] = (level, at);
        }
    }

    /// Puts `level` on `signal` at `at`. A move of a netted pad's drive waits for its halt, and
    /// any other takes effect at once.
    fn drive(&mut self, at: Tick, signal: u16, level: bool, gpio: &mut Gpio) {
        if signal == self.sda {
            self.sda_out = level;
        }
        if gpio.moves(signal, level) {
            self.moved = Some((at, signal, level));
        } else {
            gpio.set_signal(signal, level);
            self.observe(at, gpio);
        }
    }

    /// SCL read high at `h`: the high side of `run`'s clock starts.
    fn high(&self, run: &mut Run, h: Tick) {
        let timing = self.timing;
        run.h = h;
        run.deadline = None;
        let (phase, cycles) = match run.slot {
            Slot::Bit | Slot::Ack => (Phase::High { sampled: false }, timing.sda_sample),
            Slot::Restart => (Phase::Start(Edge::Sda), timing.rstart_setup),
            Slot::Stop => (Phase::Stop { released: false }, timing.stop_setup),
        };
        run.phase = phase;
        run.next = Some(timing.after(h, cycles));
    }

    /// Starts a clock of `run` carrying `slot`, after SCL fell at `fall`.
    fn clock(&self, run: &mut Run, fall: Tick, slot: Slot) {
        run.slot = slot;
        run.phase = Phase::Low { sda_set: false };
        run.next = Some(self.timing.after(fall, self.timing.sda_hold));
    }

    /// Marks `run`'s command done and starts the next one, after SCL fell at `fall`.
    fn next_command(&mut self, run: &mut Run, fall: Tick) -> Result<(), &'static str> {
        self.regs[(COMD0 / 4) as usize + run.cmd] |= COMMAND_DONE;
        run.cmd += 1;
        if run.cmd == COMMANDS {
            return Err("opcode");
        }
        self.begin(run, fall)
    }

    /// Starts `run`'s command `cmd` with SCL low since `fall`. An END marks itself done, raises
    /// `END_DETECT` and holds the bus, which ends the run.
    fn begin(&mut self, run: &mut Run, fall: Tick) -> Result<(), &'static str> {
        let k = (COMD0 / 4) as usize + run.cmd;
        run.op = Op::of(self.regs[k] & COMMAND)?;
        let slot = match run.op {
            Op::End => {
                self.regs[k] |= COMMAND_DONE;
                self.int_raw |= END_DETECT;
                self.held = true;
                return Ok(());
            }
            Op::Rstart => Slot::Restart,
            Op::Stop => Slot::Stop,
            Op::Write { n, .. } | Op::Read { n, .. } => {
                run.left = n;
                self.byte(run)?;
                Slot::Bit
            }
        };
        self.clock(run, fall, slot);
        Ok(())
    }

    /// Starts `run`'s next byte. A write takes it from the TX FIFO.
    fn byte(&mut self, run: &mut Run) -> Result<(), &'static str> {
        run.bit = 0;
        run.byte = 0;
        if let Op::Write { .. } = run.op {
            run.byte = self.tx.pop().ok_or("tx_underflow")?;
        }
        Ok(())
    }

    /// Runs the run's event due at `at`.
    fn step(&mut self, at: Tick, gpio: &mut Gpio) -> Result<(), &'static str> {
        let timing = self.timing;
        let mut run = self.run.take().expect("a run");
        match run.phase {
            Phase::Start(Edge::Sda) => {
                self.busy = true;
                self.drive(at, self.sda, false, gpio);
                run.phase = Phase::Start(Edge::Scl);
                run.next = Some(timing.after(at, timing.start_hold));
            }
            Phase::Start(Edge::Scl) => {
                self.drive(at, self.scl, false, gpio);
                self.next_command(&mut run, at)?;
            }
            Phase::Resume => self.begin(&mut run, at)?,
            Phase::Low { sda_set: false } => {
                let level = match (run.slot, run.op) {
                    (Slot::Bit, Op::Write { .. }) => run.byte >> (7 - run.bit) & 1 != 0,
                    (Slot::Ack, Op::Read { nack, .. }) => {
                        let full = self.reg(CTR) & RX_FULL_ACK_LEVEL != 0;
                        if usize::from(self.rx.len) == FIFO && full != nack {
                            return Err("rx_full_ack_level");
                        }
                        nack
                    }
                    (Slot::Stop, _) => false,
                    _ => true,
                };
                if level != self.sda_out {
                    self.drive(at, self.sda, level, gpio);
                }
                run.phase = Phase::Low { sda_set: true };
                run.next = Some(timing.after(at, timing.low - timing.sda_hold));
            }
            Phase::Low { sda_set: true } => {
                self.drive(at, self.scl, true, gpio);
                run.release = at;
                if gpio.input(self.scl) == Some(true) {
                    self.high(&mut run, at);
                } else {
                    run.phase = Phase::Rising;
                    run.next = None;
                    run.deadline = timing.timeout.map(|cycles| timing.after(at, cycles));
                }
            }
            Phase::Rising => {
                self.int_raw |= TIME_OUT;
                self.busy = false;
                self.held = false;
                self.after_nack = false;
                if !self.sda_out {
                    self.drive(at, self.sda, true, gpio);
                }
                return Ok(());
            }
            Phase::High { sampled: false } => {
                let sda = gpio.input(self.sda) == Some(true);
                match (run.slot, run.op) {
                    (Slot::Bit, Op::Read { .. }) => {
                        run.byte = run.byte << 1 | u8::from(sda);
                        if run.bit == 7 && !self.rx.push(run.byte) {
                            return Err("rx_overflow");
                        }
                    }
                    (Slot::Ack, Op::Write { check, expect, .. }) if check && sda != expect => {
                        run.nack = true;
                        self.int_raw |= NACK;
                    }
                    _ => {}
                }
                let waited = timing.after(run.release, timing.wait_high).max(run.h);
                run.phase = Phase::High { sampled: true };
                run.next = Some(timing.after(waited, timing.high));
            }
            Phase::High { sampled: true } => {
                self.drive(at, self.scl, false, gpio);
                match run.slot {
                    Slot::Bit => {
                        run.bit += 1;
                        let slot = if run.bit == 8 { Slot::Ack } else { Slot::Bit };
                        self.clock(&mut run, at, slot);
                    }
                    _ if run.nack => self.clock(&mut run, at, Slot::Stop),
                    _ => {
                        run.left -= 1;
                        if run.left == 0 {
                            self.next_command(&mut run, at)?;
                        } else {
                            self.byte(&mut run)?;
                            self.clock(&mut run, at, Slot::Bit);
                        }
                    }
                }
            }
            Phase::Stop { released: false } => {
                self.drive(at, self.sda, true, gpio);
                run.phase = Phase::Stop { released: true };
                run.next = Some(timing.after(at, timing.stop_hold));
            }
            Phase::Stop { released: true } => {
                self.busy = false;
                if run.nack {
                    self.after_nack = true;
                } else {
                    self.regs[(COMD0 / 4) as usize + run.cmd] |= COMMAND_DONE;
                    self.int_raw |= TRANS_COMPLETE;
                }
                return Ok(());
            }
        }
        if !self.held {
            self.run = Some(run);
        }
        Ok(())
    }
}
