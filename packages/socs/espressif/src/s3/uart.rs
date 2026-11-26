//! UART0, UART1 and UART2.
//!
//! Each port shifts bytes out of its TX FIFO onto its TXD signal and samples its RXD signal
//! into its RX FIFO. The GPIO matrix routes both. So does the UART function of GPIO43, GPIO44,
//! GPIO17 and GPIO18 when `SIG_IN_SEL` leaves the input on IO_MUX. A bit lasts `clkdiv` source
//! clocks, plus one more on the
//! bits where a fractional accumulator that starts at 0 for that frame crosses 16. Stop field 2
//! holds the line for the exact average half bit, `(clkdiv + frag/16) / 2`, on that same
//! accumulator, not for half of one stepped bit. The source is the APB clock when `sclk_sel`
//! is 1 and the 40 MHz crystal when it is 3, divided by `sclk_div_num` + 1. A FIFO access or a
//! frame while `mem_clk_en` or `SYSTEM_UART_MEM_CLK_EN` is clear ends the run. The clock bit
//! itself is stored.
//!
//! With `tx_idle_num` 0 and the shifter idle, the start bit begins on the FIFO write's tick.
//! The next queued byte's start bit begins when the previous stop bit ends, and that edge stays
//! queued so the pad is driven at that tick. After the FIFO empties, the next write waits
//! `tx_idle_num` bit times of idle mark. The TRM states the
//! average bit length and not which bits are the long ones, so the accumulator is this model's
//! rule. It also omits the APB-to-baud synchronizer, so a start bit is not delayed past the
//! write. Turning the baud clock off drops a frame that has not finished, and that byte is
//! not logged. A peripheral reset returns the port to power-on, apart from the FIFO pointers.
//! Neither it nor a core reset moves them, as a board capture on 2026-11-18 showed for UART0's
//! TX pointers (RX takes the same resets and is treated the same): only
//! `txfifo_rst` and `rxfifo_rst` zero them. So after a reset that followed traffic, a TX FIFO
//! write or an RX FIFO load before that FIFO's reset bit is refused as `fifo_pointers`. A
//! flash boot whose capture names no UART register programs UART0 from the captured
//! `UartDev.baut_rate` the way ROM `Uart_Init` did, and a later reset still returns the
//! silicon divider.

use std::collections::VecDeque;

use super::clock::{Tick, XTAL};
use super::gpio::Gpio;

/// `UART_FIFO_REG` through `UART_ID_REG`.
const LAST: u32 = 0x80;
const WORDS: usize = (LAST as usize / 4) + 1;

pub(crate) const UART0: u32 = 0x6000_0000;
pub(crate) const UART1: u32 = 0x6001_0000;
pub(crate) const UART2: u32 = 0x6002_e000;
pub(crate) const BASES: [u32; 3] = [UART0, UART1, UART2];
pub(crate) const NAMES: [&str; 3] = ["UART0", "UART1", "UART2"];
/// `ETS_UART0_INTR_SOURCE` and the two after it.
pub(crate) const SOURCES: [u32; 3] = [27, 28, 29];
/// `U0TXD_OUT_IDX`, `U1TXD_OUT_IDX`, `U2TXD_OUT_IDX`. The RXD inputs use the same numbers.
pub(crate) const SIGNAL: [u16; 3] = [12, 15, 18];
/// RX FIFO base address in `UART_MEM_RX_STATUS_REG`: UART0, UART1, UART2.
const RX_BASE: [u16; 3] = [0x200, 0x280, 0x300];

const FULL: u32 = 1 << 0;
const EMPTY: u32 = 1 << 1;
const PARITY_ERR: u32 = 1 << 2;
const FRM_ERR: u32 = 1 << 3;
const OVF: u32 = 1 << 4;
const BRK: u32 = 1 << 7;
const TOUT: u32 = 1 << 8;
const TX_DONE: u32 = 1 << 14;
/// Level bits. A clear leaves them set while the condition holds.
const LEVEL: u32 = FULL | EMPTY;
/// `INT_ENA` bits the model runs. Wakeup is kept and never raised: the IDF install mask sets
/// it, and light sleep already ends the run.
pub(crate) const ENA_ALLOWED: u32 =
    FULL | EMPTY | PARITY_ERR | FRM_ERR | OVF | BRK | TOUT | TX_DONE | 1 << 19;

const FIFO: u32 = 0x00;
const INT_RAW: u32 = 0x04;
const INT_ST: u32 = 0x08;
const INT_ENA: u32 = 0x0c;
const INT_CLR: u32 = 0x10;
const CLKDIV: u32 = 0x14;
const RX_FILT: u32 = 0x18;
pub(crate) const STATUS: u32 = 0x1c;
const CONF0: u32 = 0x20;
const CONF1: u32 = 0x24;
const FLOW: u32 = 0x34;
const IDLE: u32 = 0x48;
const RS485: u32 = 0x4c;
const MEM: u32 = 0x60;
const MEM_TX: u32 = 0x64;
const MEM_RX: u32 = 0x68;
const FSM: u32 = 0x6c;
const CLK: u32 = 0x78;
const ID: u32 = 0x80;

const CONF0_RESET: u32 = 0x1000_001c;
const CONF1_RESET: u32 = 96 | (96 << 10);
const IDLE_RESET: u32 = 256 | (256 << 10);
const MEM_RESET: u32 = (1 << 1) | (1 << 4) | (10 << 17);
const CLK_RESET: u32 = (1 << 12) | (3 << 20) | (1 << 22) | (1 << 24) | (1 << 25);
const ID_RESET: u32 = 1280 | (1 << 30);
/// `UART_SWFC_CONF0_REG`: `xoff_threshold` 224, `xoff_char` 19.
const SWFC0_RESET: u32 = 224 | (19 << 10);
/// `UART_SWFC_CONF1_REG`: `xon_threshold` 0, `xon_char` 17.
const SWFC1_RESET: u32 = 17 << 10;
/// `UART_AT_CMD_PRECNT_REG` and `UART_AT_CMD_POSTCNT_REG`.
const AT_IDLE_RESET: u32 = 2305;
/// `UART_AT_CMD_GAPTOUT_REG`.
const AT_GAP_RESET: u32 = 11;
/// `UART_AT_CMD_CHAR_REG`: char 43, `char_num` 3.
const AT_CHAR_RESET: u32 = 43 | (3 << 8);

/// A finished byte for `serial.csv`: its port, the byte, the start bit and the end of the stop bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sent {
    pub port: u8,
    pub byte: u8,
    pub start: Tick,
    pub end: Tick,
}

/// A TXD change still to drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edge {
    pub at: Tick,
    pub port: u8,
    pub level: bool,
}

#[derive(Clone, Debug)]
struct Tx {
    byte: u8,
    start: Tick,
    end: Tick,
    /// Wire levels that differ from the level already driven, in time order.
    edges: VecDeque<(Tick, bool)>,
}

#[derive(Clone, Debug)]
struct Rx {
    start: Tick,
    end: Tick,
    /// The byte assembled so far, low bit first.
    byte: u8,
    /// Samples not yet taken, and the bit index of the first. 0 is the start bit.
    samples: VecDeque<Tick>,
    next_bit: u8,
    data_bits: u8,
    /// The parity sample disagreed with the data bits.
    parity_bad: bool,
    /// Every sample so far was low.
    low: bool,
}

#[derive(Clone, Debug)]
struct Port {
    words: [u32; WORDS],
    tx_fifo: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    tx: Option<Tx>,
    rx: Option<Rx>,
    /// The wire level TXD drives now, before the pad's own invert.
    wire: bool,
    /// The last logical level sampled on RXD. Mark is true.
    rx_line: bool,
    /// Latched event bits. The level bits are computed.
    latched: u32,
    ena: u32,
    /// The first tick a new frame may start after the FIFO went empty.
    idle_until: Tick,
    /// When `rxfifo_tout` sets, if a byte still sits and no frame has started.
    tout_at: Option<Tick>,
    tx_w: u16,
    tx_r: u16,
    rx_w: u16,
    rx_r: u16,
    /// `UART_MEM_RX_STATUS_REG`'s base for this port.
    rx_base: u16,
    /// A byte entered the TX FIFO since power-on or the last `txfifo_rst`.
    tx_used: bool,
    /// A byte entered the RX FIFO since power-on or the last `rxfifo_rst`.
    rx_used: bool,
    /// A peripheral or core reset came after TX traffic, and no `txfifo_rst` since. The reset
    /// kept the pointers and dropped the count, so where the next write lands and what the
    /// shifter reads are not known.
    tx_stale: bool,
    /// The same for the RX FIFO, cleared by `rxfifo_rst`.
    rx_stale: bool,
    /// A frame that would run on a clock or divider the model does not shift.
    refused: Option<(&'static str, Tick)>,
    sent: VecDeque<Sent>,
    /// The module clock is on and no reset bit holds the port. While this is clear a FIFO
    /// write completes and the byte does not enter the shifter.
    runs: bool,
    /// `SYSTEM_UART_MEM_CLK_EN`. `mem_clk_en` in CONF0 is the port's own RAM clock.
    mem_clk: bool,
}

impl Port {
    fn new(index: usize) -> Port {
        let mut words = [0; WORDS];
        words[slot(CLKDIV)] = 694;
        words[slot(RX_FILT)] = 8;
        words[slot(CONF0)] = CONF0_RESET;
        words[slot(CONF1)] = CONF1_RESET;
        words[slot(0x28)] = 4095;
        words[slot(0x2c)] = 4095;
        words[slot(0x38)] = 240;
        words[slot(0x3c)] = SWFC0_RESET;
        words[slot(0x40)] = SWFC1_RESET;
        words[slot(0x44)] = 10;
        words[slot(IDLE)] = IDLE_RESET;
        words[slot(0x50)] = AT_IDLE_RESET;
        words[slot(0x54)] = AT_IDLE_RESET;
        words[slot(0x58)] = AT_GAP_RESET;
        words[slot(0x5c)] = AT_CHAR_RESET;
        words[slot(MEM)] = MEM_RESET;
        words[slot(0x70)] = 4095;
        words[slot(0x74)] = 4095;
        words[slot(CLK)] = CLK_RESET;
        words[slot(0x7c)] = 33_587_824;
        words[slot(ID)] = ID_RESET;
        Port {
            words,
            tx_fifo: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            tx: None,
            rx: None,
            wire: true,
            rx_line: true,
            latched: 0,
            ena: 0,
            idle_until: Tick(0),
            tout_at: None,
            tx_w: 0,
            tx_r: 0,
            rx_w: RX_BASE[index],
            rx_r: RX_BASE[index],
            rx_base: RX_BASE[index],
            tx_used: false,
            rx_used: false,
            tx_stale: false,
            rx_stale: false,
            refused: None,
            sent: VecDeque::new(),
            runs: true,
            mem_clk: true,
        }
    }

    fn read(&self, offset: u32) -> Option<u32> {
        if offset > LAST || !offset.is_multiple_of(4) {
            return None;
        }
        Some(match offset {
            FIFO => self.rx_fifo.front().copied().unwrap_or(0).into(),
            INT_RAW => self.raw(),
            INT_ST => self.raw() & self.ena,
            INT_ENA => self.ena,
            INT_CLR => 0,
            STATUS => self.status(),
            MEM_TX => u32::from(self.tx_w) | u32::from(self.tx_r) << 11,
            MEM_RX => u32::from(self.rx_r) | u32::from(self.rx_w) << 11,
            FSM => u32::from(self.rx.is_some()) | u32::from(self.tx.is_some()) << 4,
            ID => self.words[slot(ID)] & !(1 << 31),
            _ => self.words[slot(offset)],
        })
    }

    fn write(&mut self, now: Tick, offset: u32, value: u32, apb: u64) -> Result<(), &'static str> {
        if offset > LAST || !offset.is_multiple_of(4) {
            return Ok(());
        }
        self.settle(now, apb);
        if let Some(reason) = self.refusal(offset, value) {
            return Err(reason);
        }
        match offset {
            FIFO => self.push_tx(now, value as u8, apb)?,
            INT_RAW | INT_CLR => self.latched &= !value,
            INT_ENA => self.ena = value & ENA_ALLOWED,
            CONF0 => self.write_conf0(value),
            CLK => self.write_clk(value),
            ID => self.words[slot(ID)] = value & !(1 << 31),
            STATUS | INT_ST | MEM_TX | MEM_RX | FSM | 0x28 | 0x2c | 0x30 | 0x70 | 0x74 => {}
            _ => self.words[slot(offset)] = value,
        }
        Ok(())
    }

    fn refusal(&self, offset: u32, value: u32) -> Option<&'static str> {
        let shifting = self.tx.is_some() || self.rx.is_some();
        let format = |mask: u32, old: u32| shifting && value & mask != old & mask;
        match offset {
            FIFO if self.words[slot(FLOW)] & (1 << 3) != 0 => Some("sw_flow"),
            CLKDIV if format(0xfff | 0xf << 20, self.words[slot(CLKDIV)]) => {
                Some("baud_while_sending")
            }
            CONF0 if value & 1 << 8 != 0 => Some("break"),
            // Bits 9–13 and 16 are IrDA. Bit 14 is loopback, which the shifter runs.
            CONF0 if value & (0x1f << 9 | 1 << 16) != 0 => Some("irda"),
            CONF0 if value & 1 << 6 != 0 => Some("sw_rts"),
            CONF0 if value & 1 << 7 != 0 => Some("sw_dtr"),
            CONF0 if value & 1 << 15 != 0 => Some("hw_flow"),
            CONF0 if value & 1 << 27 != 0 => Some("autobaud"),
            CONF0 if (value >> 4) & 3 == 0 => Some("stop_bits"),
            CONF0
                if (self.tx.is_some() || self.rx.is_some()) && value & (1 << 17 | 1 << 18) != 0 =>
            {
                Some("reset_while_sending")
            }
            CONF0 if format(0x3f | 1 << 14 | 1 << 19 | 1 << 22, self.words[slot(CONF0)]) => {
                Some("baud_while_sending")
            }
            CONF1 if value & 1 << 22 != 0 => Some("hw_flow"),
            CONF1
                if format(((1 << 10) - 1) << 10 | 1 << 23, self.words[slot(CONF1)]) && shifting =>
            {
                Some("baud_while_sending")
            }
            RX_FILT if value & 1 << 8 != 0 => Some("glitch"),
            // `uart_ll_force_xoff` sets `SW_FLOW_CON_EN | FORCE_XOFF` while preparing for light
            // sleep; letting that bit pair through lets light sleep reach `RTC_CNTL_SLEEP_EN`.
            FLOW if value & 0x3f != 0 && value & 0x3f != (1 | 1 << 3) => Some("sw_flow"),
            IDLE if format(0x3ff << 10, self.words[slot(IDLE)]) && shifting => {
                Some("baud_while_sending")
            }
            RS485 if value & 1 != 0 => Some("rs485"),
            MEM if (value >> 1) & 7 != 1 || (value >> 4) & 7 != 1 => Some("fifo_size"),
            MEM if value & 1 << 27 != 0 => Some("power_down"),
            // A clear `high_speed` syncs the divider and the frame format only at
            // `UART_REG_UPDATE`. The model does not run that barrier.
            ID if value & 1 << 30 == 0 => Some("high_speed"),
            CLK if value & (0x3f | 0x3f << 6) != 0 => Some("sclk_frac"),
            CLK if {
                let sel = (value >> 20) & 3;
                sel != 1 && sel != 3
            } =>
            {
                Some("clock")
            }
            CLK if shifting && value & (1 << 23 | 1 << 26 | 1 << 27) != 0 => {
                Some("reset_while_sending")
            }
            CLK if format(0xff << 12 | 3 << 20, self.words[slot(CLK)]) => {
                Some("baud_while_sending")
            }
            _ => None,
        }
    }

    fn write_conf0(&mut self, value: u32) {
        if value & 1 << 17 != 0 {
            self.rx_fifo.clear();
            self.rx = None;
            self.tout_at = None;
            self.rx_w = self.rx_base;
            self.rx_r = self.rx_base;
            self.rx_used = false;
            self.rx_stale = false;
        }
        if value & 1 << 18 != 0 {
            self.tx_fifo.clear();
            self.tx_w = 0;
            self.tx_r = 0;
            self.tx_used = false;
            self.tx_stale = false;
            if self.tx.is_none() {
                self.wire = true ^ self.txd_inv();
            }
        }
        self.words[slot(CONF0)] = value;
    }

    fn write_clk(&mut self, value: u32) {
        // A baud clock that drops mid-frame stops the shifter. The unfinished byte is not a sent byte.
        if value & (1 << 22 | 1 << 24) != (1 << 22 | 1 << 24) {
            self.tx = None;
            self.wire = true ^ self.txd_inv();
        }
        if value & (1 << 22 | 1 << 25) != (1 << 22 | 1 << 25) {
            self.rx = None;
            self.tout_at = None;
        }
        // A core reset empties the FIFO and keeps its pointers, as a board capture on
        // 2026-11-18 showed for UART0's TX side. Only `txfifo_rst` and `rxfifo_rst` zero them.
        if value & (1 << 23 | 1 << 26) != 0 {
            self.tx_fifo.clear();
            self.tx = None;
            self.wire = true ^ self.txd_inv();
            self.tx_stale |= self.tx_used;
        }
        if value & (1 << 23 | 1 << 27) != 0 {
            self.rx_fifo.clear();
            self.rx = None;
            self.tout_at = None;
            self.rx_stale |= self.rx_used;
        }
        self.words[slot(CLK)] = value;
    }

    fn push_tx(&mut self, now: Tick, byte: u8, apb: u64) -> Result<(), &'static str> {
        // The module clock is off, or a reset holds the port. The store completes and nothing shifts.
        if !self.runs {
            return Ok(());
        }
        if self.words[slot(CONF0)] & 1 << 18 != 0
            || self.words[slot(CLK)] & (1 << 23 | 1 << 26) != 0
        {
            return Ok(());
        }
        if !self.ram_clock() {
            return Err("mem_clock");
        }
        if self.tx_stale {
            return Err("fifo_pointers");
        }
        if !self.tx_clock() {
            return Err("clock_off");
        }
        if self.clkdiv() == 0 {
            return Err("divider");
        }
        let depth = self.tx_depth();
        if depth == 0 || self.tx_fifo.len() >= depth {
            return Ok(());
        }
        self.tx_fifo.push_back(byte);
        self.tx_used = true;
        self.tx_w = self.bump(self.tx_w, depth, 0);
        if self.tx.is_none() {
            // The write drives the pad from `wire` after this returns.
            self.start_tx(now, apb, true);
        }
        Ok(())
    }

    /// Starts the next queued byte at `now`. `apply` is set only for a FIFO write: a start bit
    /// that begins on `now` changes `wire` and leaves the queue, and the write drives the pad.
    /// A byte that starts as the previous frame ends, or when the idle gap ends, leaves that
    /// edge queued so the pad is driven at the edge's own tick.
    fn start_tx(&mut self, now: Tick, apb: u64, apply: bool) {
        if self.tx.is_some() || now < self.idle_until {
            return;
        }
        let Some(byte) = self.tx_fifo.front().copied() else {
            return;
        };
        if !self.ram_clock() {
            self.refused = Some(("mem_clock", now));
            return;
        }
        self.tx_fifo.pop_front().expect("the byte");
        let depth = self.tx_depth();
        self.tx_r = self.bump(self.tx_r, depth, 0);
        let Some(period) = self.period(apb) else {
            self.tx_fifo.push_front(byte);
            self.refused = Some(("clock_off", now));
            return;
        };
        let mut tx = self.schedule(now, byte, period);
        if apply && tx.edges.front().is_some_and(|&(at, _)| at == now) {
            let (_, level) = tx.edges.pop_front().expect("the edge");
            self.wire = level;
        }
        self.tx = Some(tx);
    }

    fn schedule(&self, at: Tick, byte: u8, period: u64) -> Tx {
        let mut acc = 0u32;
        let mut t = at.0;
        let mut wire = self.wire;
        let mut edges = VecDeque::new();
        let data_bits = self.data_bits();
        let mut bits = Vec::new();
        bits.push(false);
        for i in 0..data_bits {
            bits.push(byte >> i & 1 == 1);
        }
        if self.parity_en() {
            let mask = (1u32 << u32::from(data_bits)) - 1;
            let ones = (u32::from(byte) & mask).count_ones() % 2 == 1;
            bits.push(if self.odd() { !ones } else { ones });
        }
        bits.push(true);
        for logical in bits {
            let next = logical ^ self.txd_inv();
            if next != wire {
                edges.push_back((Tick(t), next));
                wire = next;
            }
            t += bit_len(self.clkdiv(), self.frag(), &mut acc, period);
        }
        match self.stop() {
            3 => t += bit_len(self.clkdiv(), self.frag(), &mut acc, period),
            2 => t += half_bit(self.clkdiv(), self.frag(), &mut acc, period),
            _ => {}
        }
        Tx {
            byte,
            start: at,
            end: Tick(t),
            edges,
        }
    }

    /// The earliest TXD change at or before `now` that is not driven yet.
    fn next_edge(&self, now: Tick) -> Option<Edge> {
        let tx = self.tx.as_ref()?;
        let &(at, level) = tx.edges.front()?;
        (at <= now).then_some(Edge { at, port: 0, level })
    }

    fn take_edge(&mut self) -> Option<Edge> {
        let tx = self.tx.as_mut()?;
        let (at, level) = tx.edges.pop_front()?;
        self.wire = level;
        Some(Edge { at, port: 0, level })
    }

    /// Advances frames, samples and the timeout through `now`. Pin edges are applied by the
    /// caller first, which is what [`Self::note`] saw.
    /// A port with no frame in flight and nothing queued, as most are at most polls, tests that
    /// here and calls nothing.
    #[inline]
    fn settle(&mut self, now: Tick, apb: u64) {
        if self.tx.is_some() {
            self.finish_tx(now, apb);
        }
        if self.rx.is_some() {
            self.sample(now, apb);
        }
        self.fire_tout(now);
        if self.tx.is_none() && now >= self.idle_until && !self.tx_fifo.is_empty() {
            self.start_tx(now, apb, false);
            self.finish_tx(now, apb);
        }
    }

    fn finish_tx(&mut self, now: Tick, apb: u64) {
        loop {
            let Some(tx) = self.tx.as_ref() else {
                return;
            };
            if tx.edges.front().is_some_and(|&(at, _)| at <= now) {
                return;
            }
            if tx.end > now {
                return;
            }
            let tx = self.tx.take().expect("the frame");
            self.sent.push_back(Sent {
                port: 0,
                byte: tx.byte,
                start: tx.start,
                end: tx.end,
            });
            self.wire = true ^ self.txd_inv();
            if self.tx_fifo.is_empty() {
                self.idle_until = self.idle_end(tx.end, apb);
                self.latched |= TX_DONE;
                return;
            }
            self.idle_until = tx.end;
            self.start_tx(tx.end, apb, false);
        }
    }

    fn idle_end(&self, from: Tick, apb: u64) -> Tick {
        let Some(period) = self.period(apb) else {
            return from;
        };
        let mut acc = 0;
        let mut t = from.0;
        for _ in 0..self.tx_idle() {
            t += bit_len(self.clkdiv(), self.frag(), &mut acc, period);
        }
        Tick(t)
    }

    fn note(&mut self, at: Tick, level: Option<bool>, apb: u64) {
        let logical = self.sensed(level);
        if self.rx_line && !logical && self.rx.is_none() && self.rx_runs() {
            self.start_rx(at, apb);
        }
        self.rx_line = logical;
    }

    fn sensed(&self, level: Option<bool>) -> bool {
        let wire = if self.loopback() {
            self.wire
        } else {
            level.unwrap_or(true)
        };
        wire ^ self.rxd_inv()
    }

    fn start_rx(&mut self, at: Tick, apb: u64) {
        if !self.runs || self.words[slot(CONF0)] & 1 << 17 != 0 {
            return;
        }
        if !self.ram_clock() {
            self.refused = Some(("mem_clock", at));
            return;
        }
        if self.clkdiv() == 0 {
            self.refused = Some(("divider", at));
            return;
        }
        let Some(period) = self.period(apb) else {
            self.refused = Some(("clock_off", at));
            return;
        };
        self.tout_at = None;
        let data_bits = self.data_bits();
        let mut acc = 0u32;
        let mut t = at.0;
        let mut samples = VecDeque::new();
        let n = 1 + data_bits + u8::from(self.parity_en()) + 1;
        for _ in 0..n {
            let dur = bit_len(self.clkdiv(), self.frag(), &mut acc, period);
            samples.push_back(Tick(t + dur / 2));
            t += dur;
        }
        match self.stop() {
            3 => t += bit_len(self.clkdiv(), self.frag(), &mut acc, period),
            2 => t += half_bit(self.clkdiv(), self.frag(), &mut acc, period),
            _ => {}
        }
        self.rx = Some(Rx {
            start: at,
            end: Tick(t),
            byte: 0,
            samples,
            next_bit: 0,
            data_bits,
            parity_bad: false,
            low: true,
        });
    }

    fn sample(&mut self, now: Tick, apb: u64) {
        loop {
            let Some(rx) = self.rx.as_ref() else {
                return;
            };
            let Some(&at) = rx.samples.front() else {
                return;
            };
            if at > now {
                return;
            }
            let logical = self.rx_line;
            let parity_on = self.parity_en();
            let odd = self.odd();
            let mask_errors = self.words[slot(CONF0)] & 1 << 26 != 0;
            let rx = self.rx.as_mut().expect("the frame");
            rx.samples.pop_front();
            let bit = rx.next_bit;
            rx.next_bit += 1;
            rx.low &= !logical;
            let data_bits = rx.data_bits;
            let start = rx.start;
            let end = rx.end;
            if bit == 0 {
                if logical {
                    self.rx = None;
                }
                continue;
            }
            if bit <= data_bits {
                if logical {
                    rx.byte |= 1u8 << (bit - 1);
                }
                continue;
            }
            let parity_bit = parity_on && bit == data_bits + 1;
            if parity_bit {
                let ones = rx.byte.count_ones() % 2 == 1;
                let expect = if odd { !ones } else { ones };
                if logical != expect {
                    rx.parity_bad = true;
                }
                let bad = rx.parity_bad;
                if bad {
                    self.latched |= PARITY_ERR;
                }
                continue;
            }
            let bad_parity = rx.parity_bad;
            let stayed_low = rx.low;
            let byte = rx.byte;
            let frame = !logical;
            if frame {
                self.latched |= FRM_ERR;
                if stayed_low {
                    self.latched |= BRK;
                }
            }
            let keep = !mask_errors || (!frame && !bad_parity);
            self.rx = None;
            self.sent.push_back(Sent {
                port: 0,
                byte,
                start,
                end,
            });
            if keep {
                self.push_rx(byte);
            }
            self.arm_tout(end, apb);
        }
    }

    fn push_rx(&mut self, byte: u8) {
        let depth = self.rx_depth();
        if depth == 0 || self.rx_fifo.len() >= depth {
            if self.words[slot(CONF1)] & 1 << 20 == 0 {
                self.latched |= OVF;
            }
            return;
        }
        self.rx_fifo.push_back(byte);
        self.rx_used = true;
        self.rx_w = self.bump(self.rx_w, depth, self.rx_base);
    }

    fn arm_tout(&mut self, from: Tick, apb: u64) {
        if !self.rx_tout_en() || self.rx_fifo.is_empty() {
            self.tout_at = None;
            return;
        }
        let Some(period) = self.period(apb) else {
            return;
        };
        let mut acc = 0;
        let mut t = from.0;
        for _ in 0..self.rx_tout() {
            t += bit_len(self.clkdiv(), self.frag(), &mut acc, period);
        }
        self.tout_at = Some(Tick(t));
    }

    fn fire_tout(&mut self, now: Tick) {
        if self.rx.is_some() {
            return;
        }
        if self.tout_at.is_some_and(|at| at <= now) && !self.rx_fifo.is_empty() {
            self.latched |= TOUT;
            self.tout_at = None;
        }
    }

    fn raw(&self) -> u32 {
        let mut raw = self.latched & !LEVEL;
        if self.rx_fifo.len() > self.rx_full() {
            raw |= FULL;
        }
        if self.tx_fifo.len() < self.tx_empty() {
            raw |= EMPTY;
        }
        raw
    }

    fn status(&self) -> u32 {
        let rx_cnt = self.rx_fifo.len() as u32;
        let tx_cnt = self.tx_fifo.len() as u32;
        rx_cnt
            | 1 << 14
            | u32::from(self.rx_line) << 15
            | tx_cnt << 16
            | 1 << 29
            | 1 << 30
            | u32::from(self.wire) << 31
    }

    fn next_event(&self, now: Tick) -> Option<Tick> {
        let edge = self
            .tx
            .as_ref()
            .and_then(|tx| tx.edges.front().map(|&(at, _)| at));
        let end = self.tx.as_ref().map(|tx| tx.end);
        let sample = self.rx.as_ref().and_then(|rx| rx.samples.front().copied());
        let idle = (self.tx.is_none() && !self.tx_fifo.is_empty() && self.idle_until > now)
            .then_some(self.idle_until);
        [edge, end, sample, self.tout_at, idle]
            .into_iter()
            .flatten()
            .filter(|&at| at > now)
            .min()
    }

    fn period(&self, apb: u64) -> Option<u64> {
        if !self.sclk_en() {
            return None;
        }
        let src = match self.sclk_sel() {
            1 => apb,
            3 => XTAL,
            _ => return None,
        };
        Some(src * (u64::from(self.sclk_div()) + 1))
    }

    fn tx_clock(&self) -> bool {
        let clk = self.words[slot(CLK)];
        clk & (1 << 22) != 0 && clk & (1 << 24) != 0
    }

    fn rx_runs(&self) -> bool {
        let clk = self.words[slot(CLK)];
        self.sclk_en() && clk & (1 << 25) != 0 && self.words[slot(CONF0)] & 1 << 17 == 0
    }

    fn sclk_en(&self) -> bool {
        self.words[slot(CLK)] & 1 << 22 != 0
    }

    fn sclk_sel(&self) -> u32 {
        (self.words[slot(CLK)] >> 20) & 3
    }

    fn sclk_div(&self) -> u32 {
        (self.words[slot(CLK)] >> 12) & 0xff
    }

    fn clkdiv(&self) -> u32 {
        self.words[slot(CLKDIV)] & 0xfff
    }

    fn frag(&self) -> u32 {
        (self.words[slot(CLKDIV)] >> 20) & 0xf
    }

    fn data_bits(&self) -> u8 {
        (((self.words[slot(CONF0)] >> 2) & 3) + 5) as u8
    }

    fn stop(&self) -> u32 {
        (self.words[slot(CONF0)] >> 4) & 3
    }

    fn parity_en(&self) -> bool {
        self.words[slot(CONF0)] & 1 << 1 != 0
    }

    fn odd(&self) -> bool {
        self.words[slot(CONF0)] & 1 != 0
    }

    fn txd_inv(&self) -> bool {
        self.words[slot(CONF0)] & 1 << 22 != 0
    }

    fn rxd_inv(&self) -> bool {
        self.words[slot(CONF0)] & 1 << 19 != 0
    }

    fn loopback(&self) -> bool {
        self.words[slot(CONF0)] & 1 << 14 != 0
    }

    /// Both RAM clocks. CONF0 `mem_clk_en` and `SYSTEM_UART_MEM_CLK_EN` reset set, and IDF
    /// leaves them set. A FIFO access or a new frame while either is clear is refused.
    fn ram_clock(&self) -> bool {
        self.mem_clk && self.words[slot(CONF0)] & (1 << 28) != 0
    }

    fn tx_depth(&self) -> usize {
        (((self.words[slot(MEM)] >> 4) & 7) as usize) * 128
    }

    fn rx_depth(&self) -> usize {
        (((self.words[slot(MEM)] >> 1) & 7) as usize) * 128
    }

    fn rx_full(&self) -> usize {
        (self.words[slot(CONF1)] & 0x3ff) as usize
    }

    fn tx_empty(&self) -> usize {
        ((self.words[slot(CONF1)] >> 10) & 0x3ff) as usize
    }

    fn tx_idle(&self) -> u32 {
        (self.words[slot(IDLE)] >> 10) & 0x3ff
    }

    fn rx_tout_en(&self) -> bool {
        self.words[slot(CONF1)] & 1 << 23 != 0
    }

    fn rx_tout(&self) -> u32 {
        (self.words[slot(MEM)] >> 17) & 0x3ff
    }

    fn bump(&self, addr: u16, depth: usize, base: u16) -> u16 {
        if depth == 0 {
            return addr;
        }
        let off = addr.wrapping_sub(base) as usize;
        base + ((off + 1) % depth) as u16
    }
}

fn slot(offset: u32) -> usize {
    (offset / 4) as usize
}

fn bit_len(clkdiv: u32, frag: u32, acc: &mut u32, period: u64) -> u64 {
    let mut clocks = u64::from(clkdiv);
    *acc += frag;
    if *acc >= 16 {
        clocks += 1;
        *acc -= 16;
    }
    clocks * period
}

/// Half a bit for stop field 2. The length is the exact average `(clkdiv + frag/16) / 2`
/// source clocks, in the same sixteenths [`bit_len`] counts, not half of one stepped bit.
/// The accumulator advances by half of `frag`.
fn half_bit(clkdiv: u32, frag: u32, acc: &mut u32, period: u64) -> u64 {
    *acc += frag / 2;
    if *acc >= 16 {
        *acc -= 16;
    }
    (u64::from(clkdiv) * 16 + u64::from(frag)) * period / 32
}

#[derive(Clone, Debug)]
pub(crate) struct Uarts {
    ports: [Port; 3],
}

impl Uarts {
    pub(crate) fn new() -> Uarts {
        Uarts {
            ports: [Port::new(0), Port::new(1), Port::new(2)],
        }
    }

    pub(crate) fn index(addr: u32) -> Option<usize> {
        let base = addr & !0xfff;
        BASES.iter().position(|&known| known == base)
    }

    pub(crate) fn read(
        &mut self,
        index: usize,
        now: Tick,
        offset: u32,
        gpio: &Gpio,
        apb: u64,
    ) -> Option<u32> {
        self.settle_port(index, now, gpio, apb);
        let port = &mut self.ports[index];
        // A FIFO load uses the RAM. The module clock being off still completes the load.
        if offset == FIFO && port.runs && !port.ram_clock() {
            port.refused = Some(("mem_clock", now));
        }
        // After a reset that followed RX traffic, the load's slot is not known.
        if offset == FIFO && port.runs && port.rx_stale {
            port.refused = Some(("fifo_pointers", now));
        }
        port.read(offset)
    }

    pub(crate) fn write(
        &mut self,
        index: usize,
        now: Tick,
        offset: u32,
        value: u32,
        gpio: &mut Gpio,
        apb: u64,
    ) -> Result<(), &'static str> {
        self.settle_port(index, now, gpio, apb);
        self.ports[index].write(now, offset, value, apb)?;
        self.drive(index, gpio);
        self.note(index, now, gpio, apb);
        Ok(())
    }

    pub(crate) fn pop(&mut self, index: usize) {
        let base = RX_BASE[index];
        let port = &mut self.ports[index];
        if port.rx_fifo.pop_front().is_some() {
            let depth = port.rx_depth();
            port.rx_r = port.bump(port.rx_r, depth, base);
            if port.rx_fifo.is_empty() {
                port.tout_at = None;
            }
        }
    }

    pub(crate) fn line(&self, index: usize) -> bool {
        self.ports[index].raw() & self.ports[index].ena & !(1 << 19) != 0
    }

    pub(crate) fn next_event(&self, now: Tick) -> Option<Tick> {
        self.ports
            .iter()
            .filter_map(|port| port.next_event(now))
            .min()
    }

    pub(crate) fn set_runs(&mut self, index: usize, runs: bool) {
        self.ports[index].runs = runs;
    }

    /// `SYSTEM_UART_MEM_CLK_EN`, one clock for every port's RAM.
    pub(crate) fn set_mem_clock(&mut self, on: bool) {
        for port in &mut self.ports {
            port.mem_clk = on;
        }
    }

    /// Drops a frame the peripheral clock will not finish. Bytes already shifted stay logged.
    pub(crate) fn halt(&mut self, index: usize) {
        let port = &mut self.ports[index];
        port.tx = None;
        port.rx = None;
        port.tout_at = None;
        port.wire = true ^ port.txd_inv();
    }

    /// A byte is in the shifter, or a received frame is still being sampled.
    pub(crate) fn shifting(&self, index: usize) -> bool {
        let port = &self.ports[index];
        port.tx.is_some() || port.rx.is_some()
    }

    pub(crate) fn uses_apb(&self, index: usize) -> bool {
        self.ports[index].sclk_sel() == 1
    }

    pub(crate) fn take_refused(&mut self) -> Option<(Tick, &'static str, &'static str)> {
        let index = (0..3).find(|&index| self.ports[index].refused.is_some())?;
        let (setting, at) = self.ports[index].refused.take()?;
        Some((at, NAMES[index], setting))
    }

    pub(crate) fn reset(&mut self, index: usize) {
        // Bytes that already left the shifter stay in the trace. The reset drops only what
        // had not finished. The FIFO pointers survive it, as a board capture on 2026-11-18
        // showed, so a reset after traffic leaves them where the model cannot source them.
        let old = std::mem::replace(&mut self.ports[index], Port::new(index));
        let port = &mut self.ports[index];
        port.sent = old.sent;
        port.mem_clk = old.mem_clk;
        port.tx_w = old.tx_w;
        port.tx_r = old.tx_r;
        port.rx_w = old.rx_w;
        port.rx_r = old.rx_r;
        port.tx_used = old.tx_used;
        port.rx_used = old.rx_used;
        port.tx_stale = old.tx_stale || old.tx_used;
        port.rx_stale = old.rx_stale || old.rx_used;
    }

    /// ROM `Uart_Init` programs a console port from `UartDev.baut_rate` on the 40 MHz
    /// crystal before the app runs: `(40_000_000 << 4) / baud`, `sclk_div_num` 0 and
    /// `sclk_sel` 3. A flash boot whose capture holds no UART register applies that to
    /// UART0. [`reset`](Self::reset) still returns the silicon divider.
    pub(crate) fn apply_rom_baud(&mut self, index: usize, baud: u32) {
        if baud == 0 {
            return;
        }
        let div = (40_000_000u64 << 4) / u64::from(baud);
        let integer = u32::try_from(div >> 4).unwrap_or(u32::MAX);
        if integer == 0 || integer > 0xfff {
            return;
        }
        let frag = (div as u32) & 0xf;
        let port = &mut self.ports[index];
        port.words[slot(CLKDIV)] = integer | frag << 20;
        port.words[slot(CLK)] = CLK_RESET & !(0xff << 12);
    }

    /// The earliest undriven TXD change at or before `now`.
    pub(crate) fn next_edge(&self, now: Tick) -> Option<Edge> {
        self.ports
            .iter()
            .enumerate()
            .filter_map(|(index, port)| {
                port.next_edge(now).map(|mut edge| {
                    edge.port = index as u8;
                    edge
                })
            })
            .min_by_key(|edge| (edge.at, edge.port))
    }

    pub(crate) fn take_edge(&mut self, port: usize) -> Option<Edge> {
        let edge = self.ports[port].take_edge()?;
        Some(Edge {
            port: port as u8,
            ..edge
        })
    }

    pub(crate) fn after_edge(&mut self, index: usize, at: Tick, gpio: &Gpio, apb: u64) {
        self.note(index, at, gpio, apb);
    }

    pub(crate) fn settle(&mut self, now: Tick, gpio: &Gpio, apb: u64) {
        for index in 0..3 {
            self.settle_port(index, now, gpio, apb);
        }
    }

    pub(crate) fn take_sent(&mut self, through: Tick) -> Option<Sent> {
        let (index, _) = self
            .ports
            .iter()
            .enumerate()
            .filter_map(|(index, port)| port.sent.front().map(|sent| (index, sent.end)))
            .filter(|&(_, end)| end <= through)
            .min_by_key(|&(index, end)| (end, index))?;
        let mut sent = self.ports[index].sent.pop_front().expect("the byte");
        sent.port = index as u8;
        Some(sent)
    }

    fn settle_port(&mut self, index: usize, now: Tick, gpio: &Gpio, apb: u64) {
        self.note(index, now, gpio, apb);
        self.ports[index].settle(now, apb);
    }

    pub(crate) fn note_pins(&mut self, at: Tick, gpio: &Gpio, apb: u64) {
        for index in 0..3 {
            self.note(index, at, gpio, apb);
        }
    }

    fn note(&mut self, index: usize, at: Tick, gpio: &Gpio, apb: u64) {
        let level = gpio.input(SIGNAL[index]);
        self.ports[index].note(at, level, apb);
    }

    fn drive(&self, index: usize, gpio: &mut Gpio) {
        gpio.set_signal(SIGNAL[index], self.ports[index].wire);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apb() -> u64 {
        6
    }

    /// 1 Mbaud from an 80 MHz APB: every bit is 80 APB clocks, 480 ticks.
    fn megabaud(port: &mut Port) {
        port.words[slot(CLK)] = (1 << 20) | (1 << 22) | (1 << 24) | (1 << 25);
        port.words[slot(CLKDIV)] = 80;
        port.words[slot(IDLE)] = 256;
        port.words[slot(CONF0)] = CONF0_RESET;
    }

    #[test]
    fn a_bit_with_a_fraction_grows_when_the_accumulator_crosses_16() {
        let mut acc = 0;
        let mut lengths = Vec::new();
        for _ in 0..16 {
            lengths.push(bit_len(694, 7, &mut acc, 6));
        }
        assert_eq!(acc, 0);
        assert_eq!(lengths.iter().filter(|&&n| n == 694 * 6).count(), 9);
        assert_eq!(lengths.iter().filter(|&&n| n == 695 * 6).count(), 7);
    }

    #[test]
    fn a_byte_starts_on_its_write_and_its_edges_are_the_bit_boundaries() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        port.write(Tick(1000), FIFO, 0x55, apb()).unwrap();
        assert!(!port.wire, "the start bit");
        assert_eq!(port.tx.as_ref().map(|tx| tx.start), Some(Tick(1000)));
        let tx = port.tx.as_ref().unwrap();
        // 0x55 is 01010101, LSB first: high, low, high, low, high, low, high, low, then stop high.
        // Start is already driven. The remaining edges are each bit that differs, 480 ticks apart.
        let edges: Vec<_> = tx.edges.iter().map(|&(at, level)| (at.0, level)).collect();
        assert_eq!(
            edges,
            vec![
                (1480, true),
                (1960, false),
                (2440, true),
                (2920, false),
                (3400, true),
                (3880, false),
                (4360, true),
                (4840, false),
                (5320, true),
            ]
        );
        assert_eq!(tx.end, Tick(1000 + 10 * 480));
    }

    #[test]
    fn loopback_returns_the_byte_and_times_out_after_the_stop_bit() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        port.write(Tick(0), CONF0, CONF0_RESET | 1 << 14, apb())
            .unwrap();
        port.words[slot(CONF1)] |= 1 << 23;
        port.words[slot(MEM)] = (port.words[slot(MEM)] & !(0x3ff << 17)) | (2 << 17);
        port.write(Tick(0), FIFO, 0x00, apb()).unwrap();
        port.note(Tick(0), None, apb());
        for _ in 0..64 {
            let Some(at) = port.next_event(Tick(0)) else {
                break;
            };
            while let Some(edge) = port.next_edge(at) {
                port.take_edge();
                port.note(edge.at, None, apb());
            }
            port.settle(at, apb());
            if port.raw() & TOUT != 0 {
                break;
            }
        }
        assert_eq!(port.rx_fifo.front().copied(), Some(0x00));
        assert!(port.raw() & TOUT != 0, "two bit times after the stop bit");
        let sent: Vec<_> = port.sent.iter().map(|sent| sent.byte).collect();
        assert_eq!(
            sent,
            vec![0x00, 0x00],
            "the received byte and the sent byte"
        );
        assert_eq!(
            port.sent.iter().map(|sent| sent.start).collect::<Vec<_>>(),
            vec![Tick(0), Tick(0)]
        );
    }

    #[test]
    fn even_parity_on_an_eight_bit_byte_comes_back() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        port.words[slot(CONF0)] |= 1 << 1 | 1 << 14;
        port.write(Tick(0), FIFO, 0x01, apb()).unwrap();
        port.note(Tick(0), None, apb());
        for _ in 0..64 {
            let Some(at) = port.next_event(Tick(0)) else {
                break;
            };
            while let Some(edge) = port.next_edge(at) {
                port.take_edge();
                port.note(edge.at, None, apb());
            }
            port.settle(at, apb());
            if port.rx.is_none() && !port.rx_fifo.is_empty() {
                break;
            }
        }
        assert_eq!(port.rx_fifo.front().copied(), Some(0x01));
        assert_eq!(port.raw() & PARITY_ERR, 0);
    }

    #[test]
    fn hardware_flow_and_a_baud_change_while_shifting_are_refused() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        port.write(Tick(0), FIFO, 0x11, apb()).unwrap();
        assert_eq!(
            port.write(Tick(10), CONF0, CONF0_RESET | 1 << 15, apb()),
            Err("hw_flow")
        );
        assert_eq!(
            port.write(Tick(10), CLKDIV, 40, apb()),
            Err("baud_while_sending")
        );
        assert_eq!(port.write(Tick(10), FLOW, 1, apb()), Err("sw_flow"));
        assert_eq!(
            port.write(Tick(10), FLOW, 1 | 1 << 1, apb()),
            Err("sw_flow")
        );
        assert_eq!(port.write(Tick(10), FLOW, 1 << 2, apb()), Err("sw_flow"));
        assert_eq!(port.write(Tick(10), FLOW, 1 << 3, apb()), Err("sw_flow"));
        assert_eq!(port.write(Tick(10), FLOW, 1 << 4, apb()), Err("sw_flow"));
        assert_eq!(port.write(Tick(10), FLOW, 1 << 5, apb()), Err("sw_flow"));
        assert_eq!(port.write(Tick(10), RS485, 1, apb()), Err("rs485"));
        let rtc = (CLK_RESET & !(3 << 20)) | (2 << 20);
        assert_eq!(port.write(Tick(10), CLK, rtc, apb()), Err("clock"));
    }

    /// `uart_ll_force_xoff` sets `SW_FLOW_CON_EN | FORCE_XOFF` while preparing for light sleep.
    /// That write completes so light sleep reaches `RTC_CNTL_SLEEP_EN`, and a TX FIFO write while
    /// `FORCE_XOFF` stays set is still refused as `sw_flow`.
    #[test]
    fn force_xoff_for_sleep_completes_and_refuses_a_later_fifo_write() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        assert_eq!(port.write(Tick(0), FLOW, 1 | 1 << 3, apb()), Ok(()));
        assert_eq!(port.read(FSM), Some(0));
        assert_eq!(
            port.write(Tick(10), FIFO, b'A'.into(), apb()),
            Err("sw_flow")
        );
        assert_eq!(port.write(Tick(20), FLOW, 0, apb()), Ok(()));
        assert_eq!(port.write(Tick(30), FIFO, b'A'.into(), apb()), Ok(()));
    }

    #[test]
    fn clearing_the_baud_clock_while_shifting_drops_the_unfinished_byte() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        port.write(Tick(0), FIFO, 0x11, apb()).unwrap();
        let off = port.words[slot(CLK)] & !(1 << 24);
        port.write(Tick(10), CLK, off, apb()).unwrap();
        assert!(port.tx.is_none());
        assert!(port.sent.is_empty());
        assert!(port.wire);
    }

    #[test]
    fn rom_init_programs_uart0_for_115200_on_the_crystal() {
        let mut uarts = Uarts::new();
        uarts.apply_rom_baud(0, 115200);
        let port = &uarts.ports[0];
        assert_eq!(port.clkdiv(), 347);
        assert_eq!(port.frag(), 3);
        assert_eq!(port.sclk_div(), 0);
        assert_eq!(port.sclk_sel(), 3);
        assert_eq!(uarts.ports[1].clkdiv(), 694);
        assert_eq!(uarts.ports[1].sclk_div(), 1);
        assert_eq!(uarts.ports[2].clkdiv(), 694);
        uarts.reset(0);
        assert_eq!(uarts.ports[0].clkdiv(), 694);
        assert_eq!(uarts.ports[0].sclk_div(), 1);
        uarts.apply_rom_baud(0, 0);
        assert_eq!(uarts.ports[0].clkdiv(), 694);
    }

    /// Port 0 at 1 Mbaud after one byte has shifted out, as an app's early log leaves it.
    fn after_traffic() -> Uarts {
        let mut uarts = Uarts::new();
        megabaud(&mut uarts.ports[0]);
        uarts.ports[0].write(Tick(0), FIFO, 0x55, apb()).unwrap();
        while uarts.ports[0].take_edge().is_some() {}
        uarts.ports[0].settle(Tick(10 * 480), apb());
        assert_eq!(uarts.ports[0].sent.len(), 1);
        uarts
    }

    fn pointers(port: &Port) -> (u32, u32) {
        let word = port.read(MEM_TX).unwrap();
        (word & 0x3ff, word >> 11 & 0x3ff)
    }

    #[test]
    fn a_peripheral_reset_after_tx_traffic_refuses_the_next_tx_write() {
        let mut uarts = after_traffic();
        uarts.reset(0);
        megabaud(&mut uarts.ports[0]);
        let port = &mut uarts.ports[0];
        assert_eq!(
            port.write(Tick(10_000), FIFO, 0x00, apb()),
            Err("fifo_pointers")
        );
        assert!(port.tx.is_none());
        assert!(port.tx_fifo.is_empty());
    }

    #[test]
    fn a_core_reset_after_tx_traffic_refuses_the_next_tx_write() {
        let mut uarts = after_traffic();
        let port = &mut uarts.ports[0];
        let clk = port.words[slot(CLK)];
        port.write(Tick(10_000), CLK, clk | 1 << 23, apb()).unwrap();
        port.write(Tick(10_001), CLK, clk, apb()).unwrap();
        assert_eq!(
            port.write(Tick(10_002), FIFO, 0x00, apb()),
            Err("fifo_pointers")
        );
    }

    #[test]
    fn a_fifo_reset_after_the_reset_lets_the_write_shift() {
        let mut uarts = after_traffic();
        uarts.reset(0);
        megabaud(&mut uarts.ports[0]);
        let port = &mut uarts.ports[0];
        let conf0 = port.words[slot(CONF0)];
        // uart_param_config's order: RXFIFO_RST, then TXFIFO_RST, each set and cleared.
        for bit in [17, 18] {
            port.write(Tick(10_000), CONF0, conf0 | 1 << bit, apb())
                .unwrap();
            port.write(Tick(10_000), CONF0, conf0, apb()).unwrap();
        }
        assert_eq!(pointers(port), (0, 0));
        port.write(Tick(10_001), FIFO, 0xa5, apb()).unwrap();
        assert_eq!(port.tx.as_ref().map(|tx| tx.byte), Some(0xa5));
        assert_eq!(pointers(port), (1, 1));
    }

    #[test]
    fn a_reset_with_no_traffic_since_power_on_leaves_the_write_alone() {
        let mut uarts = Uarts::new();
        uarts.reset(0);
        megabaud(&mut uarts.ports[0]);
        uarts.ports[0].write(Tick(0), FIFO, 0x55, apb()).unwrap();
        assert!(uarts.ports[0].tx.is_some());
    }

    #[test]
    fn fifo_pointers_survive_a_peripheral_and_a_core_reset() {
        let mut uarts = after_traffic();
        let port = &mut uarts.ports[0];
        // Two more bytes: one shifting and one queued when the reset comes.
        port.write(Tick(10_000), FIFO, 0x01, apb()).unwrap();
        port.write(Tick(10_001), FIFO, 0x02, apb()).unwrap();
        assert_eq!(pointers(port), (3, 2));
        uarts.reset(0);
        let port = &mut uarts.ports[0];
        assert_eq!(pointers(port), (3, 2));
        assert_eq!(port.status() >> 16 & 0x3ff, 0, "the count is dropped");
        let clk = port.words[slot(CLK)];
        port.write(Tick(20_000), CLK, clk | 1 << 23 | 1 << 26 | 1 << 27, apb())
            .unwrap();
        port.write(Tick(20_001), CLK, clk, apb()).unwrap();
        assert_eq!(pointers(port), (3, 2));
    }

    #[test]
    fn a_reset_after_rx_traffic_refuses_the_next_rx_load_until_rxfifo_rst() {
        let mut uarts = Uarts::new();
        megabaud(&mut uarts.ports[0]);
        uarts.ports[0].push_rx(0x41);
        let rx = uarts.ports[0].read(MEM_RX).unwrap();
        uarts.reset(0);
        assert_eq!(
            uarts.ports[0].read(MEM_RX),
            Some(rx),
            "the RX pointers survive"
        );
        let gpio = Gpio::new(&[]);
        assert_eq!(uarts.read(0, Tick(0), FIFO, &gpio, apb()), Some(0));
        assert_eq!(uarts.ports[0].refused, Some(("fifo_pointers", Tick(0))));
        uarts.ports[0].refused = None;
        let conf0 = uarts.ports[0].words[slot(CONF0)];
        uarts.ports[0]
            .write(Tick(1), CONF0, conf0 | 1 << 17, apb())
            .unwrap();
        uarts.ports[0].write(Tick(1), CONF0, conf0, apb()).unwrap();
        uarts.read(0, Tick(2), FIFO, &gpio, apb());
        assert_eq!(uarts.ports[0].refused, None);
    }

    #[test]
    fn a_fifo_write_while_the_module_is_gated_sends_nothing() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        port.runs = false;
        port.write(Tick(0), FIFO, 0x55, apb()).unwrap();
        assert!(port.tx.is_none());
        assert!(port.tx_fifo.is_empty());
        assert!(port.sent.is_empty());
    }

    #[test]
    fn a_fifo_size_other_than_one_block_is_refused() {
        let mut port = Port::new(0);
        assert_eq!(port.write(Tick(0), MEM, MEM_RESET, apb()), Ok(()));
        let rx = (MEM_RESET & !(7 << 1)) | (2 << 1);
        assert_eq!(port.write(Tick(0), MEM, rx, apb()), Err("fifo_size"));
        let none = MEM_RESET & !(7 << 1);
        assert_eq!(port.write(Tick(0), MEM, none, apb()), Err("fifo_size"));
        let tx = (MEM_RESET & !(7 << 4)) | (2 << 4);
        assert_eq!(port.write(Tick(0), MEM, tx, apb()), Err("fifo_size"));
        assert_eq!(port.read(MEM), Some(MEM_RESET));
    }

    #[test]
    fn clearing_high_speed_is_refused_and_reg_update_reads_zero() {
        let mut port = Port::new(0);
        assert_eq!(port.write(Tick(0), ID, ID_RESET, apb()), Ok(()));
        assert_eq!(port.read(ID), Some(ID_RESET));
        assert_eq!(
            port.write(Tick(0), ID, ID_RESET & !(1 << 30), apb()),
            Err("high_speed")
        );
        assert_eq!(port.read(ID), Some(ID_RESET));
        assert_eq!(port.write(Tick(0), ID, ID_RESET | 1 << 31, apb()), Ok(()));
        assert_eq!(port.read(ID), Some(ID_RESET));
    }

    #[test]
    fn software_rts_and_dtr_are_refused() {
        let mut port = Port::new(0);
        assert_eq!(
            port.write(Tick(0), CONF0, CONF0_RESET | 1 << 6, apb()),
            Err("sw_rts")
        );
        assert_eq!(
            port.write(Tick(0), CONF0, CONF0_RESET | 1 << 7, apb()),
            Err("sw_dtr")
        );
        assert_eq!(port.read(CONF0), Some(CONF0_RESET));
    }

    #[test]
    fn a_fifo_access_or_a_frame_with_a_ram_clock_off_is_refused() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        assert_eq!(
            port.write(Tick(0), CONF0, CONF0_RESET & !(1 << 28), apb()),
            Ok(())
        );
        assert_eq!(port.write(Tick(0), FIFO, 0x55, apb()), Err("mem_clock"));
        assert!(port.tx_fifo.is_empty());
        port.note(Tick(5), Some(false), apb());
        assert_eq!(port.refused, Some(("mem_clock", Tick(5))));

        let mut gated = Port::new(0);
        megabaud(&mut gated);
        gated.mem_clk = false;
        assert_eq!(gated.write(Tick(0), FIFO, 0x55, apb()), Err("mem_clock"));
        gated.refused = None;
        gated.note(Tick(5), Some(false), apb());
        assert_eq!(gated.refused, Some(("mem_clock", Tick(5))));

        let mut uarts = Uarts::new();
        let gpio = Gpio::new(&[]);
        assert!(uarts.read(0, Tick(0), FIFO, &gpio, apb()).is_some());
        assert!(uarts.take_refused().is_none());
        uarts.set_mem_clock(false);
        assert!(uarts.read(0, Tick(0), FIFO, &gpio, apb()).is_some());
        assert_eq!(
            uarts
                .take_refused()
                .map(|(_, name, setting)| (name, setting)),
            Some(("UART0", "mem_clock"))
        );
    }

    #[test]
    fn a_stop_bit_and_a_half_is_the_exact_average() {
        let mut port = Port::new(0);
        megabaud(&mut port);
        let conf0 = (CONF0_RESET & !(3 << 4)) | (2 << 4);
        port.write(Tick(0), CONF0, conf0, apb()).unwrap();
        port.write(Tick(0), CLKDIV, 80 | (8 << 20), apb()).unwrap();
        port.write(Tick(1000), FIFO, 0x00, apb()).unwrap();
        let mut acc = 0u32;
        let mut full = 0u64;
        for _ in 0..10 {
            full += bit_len(80, 8, &mut acc, 6);
        }
        assert_eq!(acc, 0, "ten bits of frag 8 land on a sixteenth boundary");
        let average = (80u64 * 16 + 8) * 6 / 32;
        let mut stepped_acc = 0u32;
        let stepped = bit_len(80, 8, &mut stepped_acc, 6) / 2;
        assert_eq!(average, 241);
        assert_ne!(average, stepped);
        assert_eq!(
            port.tx.as_ref().map(|tx| tx.end),
            Some(Tick(1000 + full + average))
        );

        let mut rx = Port::new(0);
        megabaud(&mut rx);
        rx.write(Tick(0), CONF0, conf0, apb()).unwrap();
        rx.write(Tick(0), CLKDIV, 80 | (8 << 20), apb()).unwrap();
        rx.note(Tick(1000), Some(false), apb());
        assert_eq!(
            rx.rx.as_ref().map(|frame| frame.end),
            Some(Tick(1000 + full + average))
        );
    }

    #[test]
    fn software_flow_and_at_command_registers_read_their_reset_values() {
        let port = Port::new(1);
        assert_eq!(port.read(0x3c), Some(224 | (19 << 10)));
        assert_eq!(port.read(0x40), Some(17 << 10));
        assert_eq!(port.read(0x50), Some(2305));
        assert_eq!(port.read(0x54), Some(2305));
        assert_eq!(port.read(0x58), Some(11));
        assert_eq!(port.read(0x5c), Some(43 | (3 << 8)));
    }

    #[test]
    fn a_nonzero_fraction_pins_each_edge_and_a_burst_passes_the_poll_bound() {
        // 1 Mbaud from an 80 MHz APB is 80 source clocks, 6 ticks each. Frag 7
        // lengthens four of every ten bits by one source clock, 6 ticks.
        let mut port = Port::new(0);
        megabaud(&mut port);
        port.write(Tick(0), CLKDIV, 80 | (7 << 20), apb()).unwrap();
        port.write(Tick(0), FIFO, 0x55, apb()).unwrap();
        let tx = port.tx.as_ref().unwrap();
        let edges: Vec<_> = tx.edges.iter().map(|&(at, level)| (at.0, level)).collect();
        assert_eq!(
            edges,
            vec![
                (480, true),
                (960, false),
                (1446, true),
                (1926, false),
                (2412, true),
                (2892, false),
                (3378, true),
                (3858, false),
                (4338, true),
            ]
        );
        assert_eq!(tx.end, Tick(4824));

        let mut burst = Port::new(0);
        megabaud(&mut burst);
        burst.write(Tick(0), CLKDIV, 80 | (7 << 20), apb()).unwrap();
        for at in 0..8 {
            burst.write(Tick(at), FIFO, 0x00, apb()).unwrap();
        }
        let mut starts = Vec::new();
        if let Some(tx) = &burst.tx {
            starts.push(tx.start.0);
        }
        for _ in 0..64 {
            let Some(at) = burst.next_event(Tick(0)) else {
                break;
            };
            while burst.next_edge(at).is_some() {
                burst.take_edge();
            }
            burst.settle(at, apb());
            if let Some(tx) = &burst.tx
                && starts.last().copied() != Some(tx.start.0)
            {
                starts.push(tx.start.0);
            }
            if starts.len() == 8 {
                break;
            }
        }
        // Eight 8N1 frames. Each is 4824 ticks with frag 7 and 4800 without.
        // Six frames of that extra are 144 ticks. One CPU cycle is 2 of these
        // ticks, so 144 ticks is 72 CPU cycles, past `EDGE_POLL_BOUND` of 64.
        assert_eq!(starts, (0..8).map(|n| n * 4824).collect::<Vec<_>>());
        let without = 6 * 4800;
        assert_eq!(starts[6] - without, 144);
        assert!(starts[6] - without > 64 * 2);
    }
}
