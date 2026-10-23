//! One USART in asynchronous mode.
//!
//! Figure 20-2 and §20.3.1: one down-counter at `fosc`, reloaded with `UBRRn`
//! when it reaches zero and when `UBRRnL` is written. Each zero is one baud
//! tick, so ticks are `UBRR + 1` clocks apart. The counter runs from reset,
//! whether or not `TXEN` or `RXEN` is set. `txclk` is every 16th baud tick,
//! or every 8th when `U2X` is set (Table 20-1). That divider counts baud
//! ticks from reset and is not cleared by an `UBRRnL` write. Its power-on
//! phase is not in the manual. The model starts the count at 0.
//!
//! An idle shifter loads on the next `txclk` at or after the buffer commit
//! and then sends one bit per `txclk` (§20.6.1). The receiver's start bit is
//! the first baud tick that sees `RXD` low. Majority samples 8/9/10 (4/5/6
//! when `U2X` is set) are counted in baud ticks from that sample (§20.8.1,
//! Figure 20-5). A mid-frame `UBRRnL` write moves the next baud tick and
//! leaves the divider count alone. Whatever that does to the bit on the wire
//! is Figure 20-2, not a separate §20.11.5 stretch.
//!
//! Nothing is polled while the line is idle. The next `txclk` or sample tick
//! is computed from the reload cycle. `UMSEL` = 01 (synchronous USART) and
//! the reserved `10` are logged and do not shift frames. `UMSEL` = 11 is
//! Master SPI mode (§21): `XCK` is SCK, the baud generator sets the rate
//! (Table 21-1, `fosc / (2·(UBRR+1))`), and the host queue on this USART is
//! the `MISO` byte instead of an async waveform. Reserved `UCSZ` encodings
//! are treated as 8-bit. Reserved parity `UPM = 01` is no parity.

use std::collections::VecDeque;

use crate::Level;
use crate::Port;
use crate::chip::UsartCfg;
use crate::mux::{DriveSource, PinMux};
use crate::periph::{Ctx, Gate};
use crate::regs::{UBRRH, UBRRL, UCSRA, UCSRB, UCSRC, UDR};

/// `UCSRnC` after reset.
const UCSRC_RESET: u8 = 0x06;

const RXC: u8 = 1 << 7;
const TXC_BIT: u8 = 1 << 6;
const UDRE_BIT: u8 = 1 << 5;
const FE: u8 = 1 << 4;
const DOR: u8 = 1 << 3;
const UPE: u8 = 1 << 2;
const U2X: u8 = 1 << 1;
const MPCM: u8 = 1 << 0;

const RXCIE: u8 = 1 << 7;
const TXCIE: u8 = 1 << 6;
const UDRIE: u8 = 1 << 5;
const RXEN: u8 = 1 << 4;
const TXEN: u8 = 1 << 3;
const UCSZ2: u8 = 1 << 2;
const RXB8: u8 = 1 << 1;
const TXB8: u8 = 1 << 0;

const UMSEL: u8 = 0xC0;
const USBS: u8 = 1 << 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParityMode {
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone, Copy)]
struct FrameBits {
    data: u16,
    n_data: u8,
    parity_high: Option<bool>,
    n_stop: u8,
}

impl FrameBits {
    fn new(data: u16, n_data: u8, parity: ParityMode, n_stop: u8) -> Self {
        let n_data = n_data.clamp(5, 9);
        let n_stop = if n_stop >= 2 { 2 } else { 1 };
        let mask = (1u16 << n_data) - 1;
        let data = data & mask;
        let parity_high = match parity {
            ParityMode::None => None,
            ParityMode::Even => Some(parity_even(data, n_data)),
            ParityMode::Odd => Some(!parity_even(data, n_data)),
        };
        Self {
            data,
            n_data,
            parity_high,
            n_stop,
        }
    }

    fn n_bits(self) -> u8 {
        1 + self.n_data + u8::from(self.parity_high.is_some()) + self.n_stop
    }

    /// `true` is mark. Bit 0 is the start bit.
    fn high_at(self, bit: u8) -> bool {
        if bit == 0 {
            return false;
        }
        if bit <= self.n_data {
            return (self.data >> (bit - 1)) & 1 == 1;
        }
        if self.parity_high.is_some() && bit == self.n_data + 1 {
            return self.parity_high.unwrap_or(false);
        }
        true
    }
}

fn parity_even(data: u16, n: u8) -> bool {
    let mut parity = false;
    for bit in 0..n {
        parity ^= (data >> bit) & 1 == 1;
    }
    parity
}

/// Transmit shifter. `bit` is `None` until the start bit is launched on `txclk`.
#[derive(Debug, Clone, Copy)]
struct TxLive {
    bits: FrameBits,
    bit: Option<u8>,
    /// Cycle the current bit began. Meaningful once `bit` is `Some`.
    started: u64,
    /// Oscillator cycle the start bit began. Meaningful once `bit` is `Some`. A gated clock
    /// does not move it, because it is a time that has passed, not a deadline.
    start: u64,
}

impl TxLive {
    fn waiting(bits: FrameBits) -> Self {
        Self {
            bits,
            bit: None,
            started: 0,
            start: 0,
        }
    }
}

/// A byte the shifter finished, in the order the shifter finished them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Shifted {
    byte: u8,
    /// CPU cycle its last bit ended, which `Cpu::take` returns.
    cycle: u64,
    /// Oscillator cycle its start bit began. `None` for a Master SPI mode byte, which has none.
    start: Option<u64>,
    /// Oscillator cycle its last bit ended.
    end: u64,
}

/// An asynchronous frame a USART sent, on the oscillator's cycle count from power-on.
///
/// `start` is the oscillator cycle its start bit drove `TXD` low, and `end` the one its last
/// stop bit ended. The core stamps both as they happen, so a later `CLKPR` write, a sleep, or a
/// reset does not move them. A host on a fixed oscillator of `f` hertz puts the start at
/// `ceil(start * 1e12 / f)` ps, the time a host stops for a `TXD` drive change on that cycle. The
/// net falls at that time only when that drive is the pin's net level change in that picosecond
/// and nothing else loads the pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    /// The USART's index on the chip, 0 for `USART0`.
    pub usart: u8,
    pub byte: u8,
    pub start: u64,
    pub end: u64,
}

/// MSPIM shifter. The half-period is snapshotted when the bit boundary arrives.
#[derive(Debug, Clone, Copy)]
struct MspimSh {
    byte: u8,
    rx: u8,
    bit: u8,
    /// 0 waits for the bit boundary. 1 is the leading edge. 2 is the trailing edge.
    phase: u8,
    half: u64,
    cpha: bool,
    cpol: bool,
    lsb: bool,
}
#[derive(Debug, Clone, Copy)]
struct Live {
    bits: FrameBits,
    /// Bit currently on the wire.
    bit: u8,
    /// Cycle this bit ends.
    ends: u64,
    bit_cycles: u64,
}

impl Live {
    fn start(bits: FrameBits, at: u64, bit_cycles: u64) -> Self {
        Self {
            bits,
            bit: 0,
            ends: at.saturating_add(bit_cycles),
            bit_cycles,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RxByte {
    data: u8,
    ninth: bool,
    fe: bool,
    dor: bool,
    upe: bool,
}

#[derive(Debug, Clone, Copy)]
struct Vote {
    /// Baud-tick index of sample 1, the first tick that saw the line low.
    origin_index: u64,
    /// 16, or 8 when `U2X` was set at sample 1.
    n: u8,
    votes: [u8; 3],
    bit: u8,
    got: u8,
    ones: u8,
    /// Global baud-tick index of the next majority sample.
    next_index: u64,
    data: u16,
    n_data: u8,
    parity: ParityMode,
    parity_high: bool,
    saw_parity: bool,
}

impl Vote {
    /// Sample `n` (1-based) of bit `b`, counted from sample 1 (§20.8.1).
    fn sample_index(self, bit: u8, sample: u8) -> u64 {
        self.origin_index + u64::from(bit) * u64::from(self.n) + u64::from(sample.saturating_sub(1))
    }
}

#[derive(Debug, Clone, Copy)]
enum Rx {
    Hunt,
    /// A falling edge is waiting for the next baud tick (§20.8.1, sample 1).
    Confirm,
    Vote(Vote),
}

#[derive(Debug, Clone, Copy)]
struct HostOver {
    bit_cycles: u64,
    n_data: u8,
    parity: ParityMode,
    n_stop: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct Usart {
    tx_port: Port,
    tx_bit: u8,
    tx_pin: u8,
    rx_port: Port,
    rx_bit: u8,
    rx_pin: u8,
    xck_pin: u8,
    /// `UCSRnC` address, so a synchronous-mode write can be logged.
    ucsrc_addr: u16,
    rx_vec: u8,
    udre_vec: u8,
    tx_vec: u8,
    /// `U2X` and `MPCM` only. Status flags are computed.
    ctrl: u8,
    ucsrb: u8,
    ucsrc: u8,
    /// Twelve-bit baud divider. `UBRRnH` bits 7:4 are not stored.
    ubrr: u16,
    /// Cycle the down-counter was loaded. Reset loads it at cycle 0.
    brg_at: u64,
    /// Clocks per baud tick in the count that is running now.
    brg_period: u64,
    /// Baud ticks at or before `brg_at`. The tick on `brg_at` is included
    /// only when `brg_tick` is set.
    brg_ticks: u64,
    /// `brg_at` itself was a baud tick (the counter reached zero, then reloaded).
    brg_tick: bool,
    udre: bool,
    txc: bool,
    tx_on_pin: bool,
    rx_forced: bool,
    tx: Option<TxLive>,
    /// The one-byte transmit buffer. `UDRE` is clear while this is `Some`.
    tx_buf: Option<FrameBits>,
    pending_tx: Option<FrameBits>,
    tx_out: Vec<Shifted>,
    rx_buf: VecDeque<RxByte>,
    /// Completed frame that could not enter the two-deep buffer.
    rx_held: Option<RxByte>,
    /// The next stored frame lost one or more characters ahead of it.
    drop_dor: bool,
    last_rx: u8,
    rx: Rx,
    host_q: Vec<u8>,
    host: Option<Live>,
    host_over: Option<HostOver>,
    next_tx: u64,
    next_rx: u64,
    next_host: u64,
    /// Heap so the idle USART stays inside the `Device` size bound. `next_tx`
    /// is the MSPIM edge while this is `Some`; UART transmit is idle then.
    mspim_sh: Option<Box<MspimSh>>,
    mspim_buf: Option<u8>,
    mspim_miso: u8,
    xck_on: bool,
    defer_xck: bool,
    defer_tx_pin: bool,
    defer_rx_pin: bool,
    /// `UBRRnL` reloads the down-counter at the instruction boundary.
    defer_reload: bool,
    /// `U2X` changes the `txclk` divisor without reloading the counter.
    defer_divisor: bool,
    defer_flush_rx: bool,
    /// `clkI/O` and `PRUSART0`. Either one freezes the baud generator (§10.9).
    clock_on: bool,
    prr_off: bool,
    frozen_at: u64,
}

fn pop_front<T>(queue: &mut Vec<T>) -> Option<T> {
    if queue.is_empty() {
        None
    } else {
        Some(queue.remove(0))
    }
}

impl Usart {
    pub(crate) fn new(cfg: &UsartCfg) -> Self {
        Self {
            tx_port: cfg.tx.port,
            tx_bit: cfg.tx.bit,
            tx_pin: cfg.tx.port.pin_id(cfg.tx.bit) as u8,
            rx_port: cfg.rx.port,
            rx_bit: cfg.rx.bit,
            rx_pin: cfg.rx.port.pin_id(cfg.rx.bit) as u8,
            xck_pin: cfg.xck.port.pin_id(cfg.xck.bit) as u8,
            ucsrc_addr: cfg
                .regs
                .iter()
                .find(|reg| reg.id == UCSRC)
                .map(|reg| reg.addr)
                .unwrap_or(0),
            rx_vec: cfg.rx_vec,
            udre_vec: cfg.udre_vec,
            tx_vec: cfg.tx_vec,
            ctrl: 0,
            ucsrb: 0,
            ucsrc: UCSRC_RESET,
            ubrr: 0,
            brg_at: 0,
            brg_period: 1,
            brg_ticks: 0,
            brg_tick: false,
            udre: true,
            txc: false,
            tx_on_pin: false,
            rx_forced: false,
            tx: None,
            tx_buf: None,
            pending_tx: None,
            tx_out: Vec::new(),
            rx_buf: VecDeque::new(),
            rx_held: None,
            drop_dor: false,
            last_rx: 0,
            rx: Rx::Hunt,
            host_q: Vec::new(),
            host: None,
            host_over: None,
            next_tx: u64::MAX,
            next_rx: u64::MAX,
            next_host: u64::MAX,
            mspim_sh: None,
            mspim_buf: None,
            mspim_miso: 0xFF,
            xck_on: false,
            defer_xck: false,
            defer_tx_pin: false,
            defer_rx_pin: false,
            defer_reload: false,
            defer_divisor: false,
            defer_flush_rx: false,
            clock_on: true,
            prr_off: false,
            frozen_at: 0,
        }
    }

    fn running(&self) -> bool {
        self.clock_on && !self.prr_off
    }

    /// §11.1. The baud generator and `txclk` divider start at the release
    /// cycle, with the same phase a power-on at cycle 0 has. The time-out
    /// itself does not advance them.
    pub(crate) fn release_epoch(&mut self, now: u64) {
        self.brg_at = now;
        self.brg_ticks = 0;
        self.brg_tick = false;
        self.brg_period = self.reg_period();
        self.next_tx = u64::MAX;
        self.next_rx = u64::MAX;
        self.next_host = u64::MAX;
        self.mspim_sh = None;
        self.mspim_buf = None;
    }

    /// §10.9 and Table 10-1. A stopped USART keeps its shifter and ignores I/O.
    pub(crate) fn set_clock_gate(&mut self, io: Option<bool>, prr: Option<bool>, cycle: u64) {
        let delta = Gate {
            clock_on: &mut self.clock_on,
            prr_off: &mut self.prr_off,
            frozen_at: &mut self.frozen_at,
        }
        .set(io, prr, cycle);
        if let Some(delta) = delta {
            self.shift_clock(delta);
        }
    }

    fn shift_clock(&mut self, delta: u64) {
        self.brg_at = self.brg_at.wrapping_add(delta);
        self.next_tx = super::shift_deadline(self.next_tx, delta);
        self.next_rx = super::shift_deadline(self.next_rx, delta);
        self.next_host = super::shift_deadline(self.next_host, delta);
        if let Some(tx) = &mut self.tx {
            tx.started = tx.started.wrapping_add(delta);
        }
        if let Some(host) = &mut self.host {
            host.ends = host.ends.wrapping_add(delta);
        }
    }

    pub(crate) fn set_host_timing(
        &mut self,
        bit_cycles: u64,
        data_bits: u8,
        parity: u8,
        stop_bits: u8,
    ) {
        self.host_over = Some(HostOver {
            bit_cycles: bit_cycles.max(1),
            n_data: data_bits.clamp(5, 9),
            parity: parity_from_code(parity),
            n_stop: if stop_bits >= 2 { 2 } else { 1 },
        });
    }

    /// Host bytes become a waveform on `RXD` whether or not `RXEN` is set.
    /// A receiver that is off misses them. Frames in one call are back to back.
    pub(crate) fn host_supply(
        &mut self,
        bytes: &[u8],
        at: u64,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
    ) {
        self.host_q.extend(bytes);
        if self.mspim() || self.host.is_some() {
            return;
        }
        self.kick_host(at, mux, ctx);
    }

    pub(crate) fn take_at(&mut self) -> Option<(u8, u64)> {
        pop_front(&mut self.tx_out).map(|out| (out.byte, out.cycle))
    }

    /// The next asynchronous frame whose last bit ended at or before oscillator cycle `through`,
    /// labelled with `usart`, this USART's index on the chip. A Master SPI mode byte in that
    /// prefix is dropped on the way and does not end the scan.
    pub(crate) fn take_sent(&mut self, usart: u8, through: u64) -> Option<Sent> {
        while self.tx_out.first().is_some_and(|out| out.end <= through) {
            let out = self.tx_out.remove(0);
            if let Some(start) = out.start {
                return Some(Sent {
                    usart,
                    byte: out.byte,
                    start,
                    end: out.end,
                });
            }
        }
        None
    }

    /// The bytes the shifter finished. A reset keeps them for [`Self::restore_sent`], and the
    /// frame still in the shifter or the buffer is not among them.
    pub(crate) fn take_all_sent(&mut self) -> Vec<Shifted> {
        std::mem::take(&mut self.tx_out)
    }

    pub(crate) fn restore_sent(&mut self, sent: Vec<Shifted>) {
        self.tx_out = sent;
    }

    /// `(byte, recompute interrupts)`. Reading `UDRn` pops the receive buffer.
    pub(crate) fn read(&mut self, reg: u8) -> (u8, bool) {
        // §10.9. A frozen read does not pop `UDR`.
        if !self.running() {
            return (self.peek(reg), false);
        }
        if reg == UDR {
            (self.read_udr(), true)
        } else {
            (self.peek(reg), false)
        }
    }

    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        _mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        _cycle: u64,
    ) -> crate::periph::Touch {
        if !self.running() {
            return crate::periph::Touch::default();
        }
        let mut touch = crate::periph::Touch {
            irq: true,
            ..crate::periph::Touch::default()
        };
        match reg {
            UCSRA => {
                if value & TXC_BIT != 0 {
                    self.txc = false;
                }
                let u2x = self.ctrl & U2X;
                self.ctrl = value & (U2X | MPCM);
                if self.ctrl & U2X != u2x {
                    self.defer_divisor = true;
                    touch.stamp = true;
                }
            }
            UCSRB => {
                let was_rx = self.ucsrb & RXEN != 0;
                self.ucsrb = value & !RXB8;
                if was_rx && self.ucsrb & RXEN == 0 {
                    self.defer_flush_rx = true;
                }
                self.defer_tx_pin = true;
                self.defer_rx_pin = true;
                touch.stamp = true;
            }
            UCSRC => {
                let umsel = value & UMSEL;
                if umsel != 0 && umsel != UMSEL {
                    ctx.log(self.ucsrc_addr, value);
                }
                let was_async = self.async_mode();
                let was_mspim = self.mspim();
                self.ucsrc = value;
                if was_async && !self.async_mode() {
                    self.abort_async();
                    self.defer_tx_pin = true;
                    self.defer_rx_pin = true;
                    touch.stamp = true;
                }
                if was_mspim && !self.mspim() {
                    self.defer_xck = true;
                    touch.stamp = true;
                }
                if self.mspim() {
                    self.defer_tx_pin = true;
                    self.defer_rx_pin = true;
                    self.defer_xck = true;
                    touch.stamp = true;
                }
            }
            UBRRL => {
                self.ubrr = (self.ubrr & 0x0F00) | u16::from(value);
                self.defer_reload = true;
                touch.stamp = true;
            }
            UBRRH => {
                self.ubrr = (self.ubrr & 0x00FF) | (u16::from(value & 0x0F) << 8);
            }
            UDR => {
                let mspim = self.mspim();
                if self.ucsrb & TXEN == 0 || !self.udre || !(self.async_mode() || mspim) {
                    touch.irq = false;
                    return touch;
                }
                self.udre = false;
                self.pending_tx = Some(self.latch_tx(value));
                touch.stamp = true;
            }
            _ => touch.irq = false,
        }
        touch
    }

    /// `UDRE`, `TXC` and `RXC` change at a frame event. Not `UDR`, whose read pops the receive FIFO.
    pub(crate) fn steady(&self, reg: u8) -> bool {
        matches!(reg, UCSRA | UCSRB | UCSRC | UBRRL | UBRRH)
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            UCSRA => self.read_ucsra(),
            UCSRB => {
                let mut value = self.ucsrb & !RXB8;
                if self.head().is_some_and(|byte| byte.ninth) {
                    value |= RXB8;
                }
                value
            }
            UCSRC => self.ucsrc,
            UBRRL => self.ubrr as u8,
            UBRRH => (self.ubrr >> 8) as u8,
            UDR => self.last_rx,
            _ => 0,
        }
    }

    /// Plants the register. Does not start a frame, move a pin, or log.
    ///
    /// `UBRRnL` still reloads the down-counter, which is what a store does
    /// (Figure 20-2). The divider count is left as it is.
    pub(crate) fn poke(&mut self, reg: u8, value: u8, cycle: u64) {
        if !self.running() {
            return;
        }
        match reg {
            UCSRA => self.ctrl = value & (U2X | MPCM),
            UCSRB => self.ucsrb = value & !RXB8,
            UCSRC => self.ucsrc = value,
            UBRRL => {
                self.ubrr = (self.ubrr & 0x0F00) | u16::from(value);
                self.reload(cycle);
            }
            UBRRH => self.ubrr = (self.ubrr & 0x00FF) | (u16::from(value & 0x0F) << 8),
            _ => {}
        }
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        let mut bits = 0u64;
        if self.rxc() && self.ucsrb & RXCIE != 0 {
            bits |= 1u64 << self.rx_vec;
        }
        if self.udre && self.ucsrb & UDRIE != 0 {
            bits |= 1u64 << self.udre_vec;
        }
        if self.txc && self.ucsrb & TXCIE != 0 {
            bits |= 1u64 << self.tx_vec;
        }
        bits
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        if vector == self.tx_vec {
            self.txc = false;
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        if !self.running() {
            return u64::MAX;
        }
        self.next_tx.min(self.next_rx).min(self.next_host)
    }

    pub(crate) fn fire(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        self.sync_period(at);
        if self.next_host == at {
            self.fire_host(at, mux, ctx);
        }
        if self.next_tx == at {
            if self.mspim_sh.is_some() {
                self.fire_mspim(at, mux, ctx);
            } else {
                self.fire_tx(at, mux, ctx);
            }
        }
        if self.next_rx == at {
            self.fire_rx(at, mux);
        }
    }

    pub(crate) fn on_pad(&mut self, pin: u8, level: Level, at: u64) -> crate::periph::Touch {
        if pin != self.rx_pin || level != Level::Low {
            return crate::periph::Touch::default();
        }
        if self.ucsrb & RXEN == 0 || !self.async_mode() || !matches!(self.rx, Rx::Hunt) {
            return crate::periph::Touch::default();
        }
        self.sync_period(at);
        let (tick, _) = self.first_tick_at_or_after(at);
        if tick == at {
            self.begin_from_sample1(at);
        } else {
            self.rx = Rx::Confirm;
            self.next_rx = tick;
        }
        crate::periph::Touch {
            schedule: true,
            ..crate::periph::Touch::default()
        }
    }

    /// Instruction boundary. Deferred pin changes, the prescaler reload, and a
    /// buffered transmit byte land on this cycle.
    pub(crate) fn stamp(&mut self, cycle: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if self.defer_flush_rx {
            self.flush_rx();
            self.defer_flush_rx = false;
        }
        if self.defer_reload {
            // Count ticks with the period that was actually running. The new
            // UBRR is already in the register, but it was not in the counter.
            self.reload(cycle);
            self.defer_reload = false;
            self.resync(cycle, mux, ctx);
        } else {
            self.sync_period(cycle);
        }
        if self.defer_divisor {
            self.defer_divisor = false;
            self.resync(cycle, mux, ctx);
        }
        if let Some(bits) = self.pending_tx.take() {
            self.commit_tx(bits, cycle, mux, ctx);
        }
        if self.mspim_sh.is_some() && !self.mspim() {
            self.abort_mspim(cycle, mux, ctx);
        }
        if self.defer_xck {
            self.sync_xck(cycle, mux, ctx);
            self.defer_xck = false;
        }
        if self.defer_tx_pin {
            self.sync_tx_pin(cycle, mux, ctx);
            self.defer_tx_pin = false;
        }
        if self.defer_rx_pin {
            self.sync_rx_pin(cycle, mux, ctx);
            self.defer_rx_pin = false;
        }
    }

    fn commit_tx(&mut self, bits: FrameBits, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if self.mspim() {
            if self.ucsrb & TXEN == 0 {
                self.udre = self.mspim_buf.is_none() && self.mspim_sh.is_none();
                return;
            }
            self.commit_mspim(bits.data as u8, at, mux, ctx);
            return;
        }
        if !self.async_mode() || self.ucsrb & TXEN == 0 {
            self.udre = self.tx_buf.is_none();
            return;
        }
        if !self.tx_on_pin {
            self.drive_tx(true, at, mux, ctx);
            self.tx_on_pin = true;
        }
        if self.tx.is_none() {
            // The idle shifter takes the byte now, so the next instruction
            // sees UDRE set (§20.6.3). The start bit waits for txclk (§20.6.1).
            self.udre = true;
            self.tx = Some(TxLive::waiting(bits));
            let tick = self.next_txclk(at);
            if tick == at {
                self.begin_bit(at, mux, ctx);
            } else {
                self.next_tx = tick;
            }
        } else {
            self.tx_buf = Some(bits);
            self.udre = false;
        }
    }

    fn fire_tx(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some(live) = self.tx else {
            self.next_tx = u64::MAX;
            return;
        };
        if live.bit.is_none() {
            self.begin_bit(at, mux, ctx);
        } else {
            self.end_bit(at, mux, ctx);
        }
    }

    /// Put the start bit on the wire. `at` is a `txclk`.
    fn begin_bit(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if let Some(live) = self.tx.as_mut() {
            live.bit = Some(0);
            live.started = at;
            live.start = ctx.oscillator_cycles(at);
        }
        self.drive_tx(false, at, mux, ctx);
        self.next_tx = self.next_txclk_after(at);
    }

    /// `at` is the `txclk` that ends the bit now on the wire.
    fn end_bit(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some(live) = self.tx else {
            self.next_tx = u64::MAX;
            return;
        };
        let bit = live.bit.unwrap_or(0);
        let n_bits = live.bits.n_bits();
        let data = live.bits.data as u8;
        let high_next = live.bits.high_at(bit.saturating_add(1));
        if bit + 1 >= n_bits {
            self.tx = None;
            self.tx_out.push(Shifted {
                byte: data,
                cycle: at,
                start: Some(live.start),
                end: ctx.oscillator_cycles(at),
            });
            if let Some(next) = self.tx_buf.take() {
                self.udre = true;
                self.tx = Some(TxLive::waiting(next));
                self.begin_bit(at, mux, ctx);
            } else {
                self.udre = true;
                self.txc = true;
                self.next_tx = u64::MAX;
                if self.ucsrb & TXEN == 0 || !self.async_mode() {
                    self.release_tx(at, mux, ctx);
                    self.tx_on_pin = false;
                }
            }
            return;
        }
        let next_bit = bit + 1;
        if let Some(live) = self.tx.as_mut() {
            live.bit = Some(next_bit);
            live.started = at;
        }
        self.drive_tx(high_next, at, mux, ctx);
        self.next_tx = self.next_txclk_after(at);
    }

    fn fire_host(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some(mut live) = self.host else {
            self.next_host = u64::MAX;
            return;
        };
        if live.bit + 1 >= live.bits.n_bits() {
            self.host = None;
            self.next_host = u64::MAX;
            self.kick_host(at, mux, ctx);
            return;
        }
        live.bit += 1;
        live.ends = at.saturating_add(live.bit_cycles);
        let high = live.bits.high_at(live.bit);
        self.next_host = live.ends;
        self.host = Some(live);
        self.drive_rx(if high { Level::High } else { Level::Low }, at, mux, ctx);
    }

    fn kick_host(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some(byte) = pop_front(&mut self.host_q) else {
            self.next_host = u64::MAX;
            return;
        };
        let (clocks, n_data, parity, n_stop) = self.host_format();
        let bits = FrameBits::new(u16::from(byte), n_data, parity, n_stop);
        self.drive_rx(Level::Low, at, mux, ctx);
        let live = Live::start(bits, at, clocks);
        self.next_host = live.ends;
        self.host = Some(live);
    }

    fn fire_rx(&mut self, at: u64, mux: &PinMux) {
        match self.rx {
            Rx::Hunt => self.next_rx = u64::MAX,
            Rx::Confirm => {
                if mux.pad_high(self.rx_port, self.rx_bit) {
                    self.rx = Rx::Hunt;
                    self.next_rx = u64::MAX;
                } else {
                    self.begin_from_sample1(at);
                }
            }
            Rx::Vote(_) => self.sample_vote(at, mux),
        }
    }

    fn sample_vote(&mut self, _at: u64, mux: &PinMux) {
        let Rx::Vote(vote) = self.rx else {
            self.next_rx = u64::MAX;
            return;
        };
        let mut vote = vote;
        if mux.pad_high(self.rx_port, self.rx_bit) {
            vote.ones = vote.ones.saturating_add(1);
        }
        vote.got = vote.got.saturating_add(1);
        if vote.got < 3 {
            self.arm_vote(vote);
            return;
        }
        let high = vote.ones >= 2;
        vote.got = 0;
        vote.ones = 0;
        if vote.bit == 0 {
            if high {
                self.rx = Rx::Hunt;
                self.next_rx = u64::MAX;
            } else {
                vote.bit = 1;
                self.arm_vote(vote);
            }
            return;
        }
        let data_limit = vote.n_data;
        if vote.bit <= data_limit {
            if high {
                vote.data |= 1u16 << (vote.bit - 1);
            }
            vote.bit += 1;
            self.arm_vote(vote);
            return;
        }
        if vote.parity != ParityMode::None && !vote.saw_parity {
            vote.parity_high = high;
            vote.saw_parity = true;
            vote.bit += 1;
            self.arm_vote(vote);
            return;
        }
        self.store_rx(vote, !high);
        self.rx = Rx::Hunt;
        self.next_rx = u64::MAX;
    }

    fn arm_vote(&mut self, mut vote: Vote) {
        vote.next_index = vote.sample_index(vote.bit, vote.votes[usize::from(vote.got)]);
        self.next_rx = self.cycle_of_index(vote.next_index);
        self.rx = Rx::Vote(vote);
    }

    /// `cycle` is sample 1: the baud tick that saw the start edge's low.
    fn begin_from_sample1(&mut self, cycle: u64) {
        if self.rx_held.is_some() {
            self.rx_held = None;
            self.drop_dor = true;
        }
        let n = self.samples_per_bit();
        let votes = if n == 8 { [4, 5, 6] } else { [8, 9, 10] };
        let (n_data, parity, _) = self.format_now();
        let origin_index = self.tick_index(cycle);
        let vote = Vote {
            origin_index,
            n,
            votes,
            bit: 0,
            got: 0,
            ones: 0,
            next_index: origin_index,
            data: 0,
            n_data,
            parity,
            parity_high: false,
            saw_parity: false,
        };
        self.arm_vote(vote);
    }

    fn store_rx(&mut self, vote: Vote, fe: bool) {
        let n = vote.n_data;
        let data = vote.data & ((1u16 << n) - 1);
        let ninth = n == 9 && data & 0x100 != 0;
        let address = if n == 9 { ninth } else { !fe };
        if self.ctrl & MPCM != 0 && !address {
            return;
        }
        let upe = match vote.parity {
            ParityMode::None => false,
            ParityMode::Even => vote.parity_high != parity_even(data, n),
            ParityMode::Odd => vote.parity_high != !parity_even(data, n),
        };
        let slot = RxByte {
            data: data as u8,
            ninth,
            fe,
            dor: self.drop_dor,
            upe,
        };
        self.drop_dor = false;
        self.push_slot(slot);
    }

    fn push_slot(&mut self, slot: RxByte) {
        if self.rx_buf.len() < 2 {
            self.rx_buf.push_back(slot);
        } else if self.rx_held.is_none() {
            self.rx_held = Some(slot);
        } else {
            self.rx_held = Some(RxByte { dor: true, ..slot });
        }
    }

    fn read_udr(&mut self) -> u8 {
        let Some(slot) = self.rx_buf.pop_front() else {
            return self.last_rx;
        };
        self.last_rx = slot.data;
        if self.rx_buf.len() < 2
            && let Some(held) = self.rx_held.take()
        {
            self.rx_buf.push_back(held);
        }
        slot.data
    }

    fn flush_rx(&mut self) {
        self.rx_buf.clear();
        self.rx_held = None;
        self.drop_dor = false;
        self.rx = Rx::Hunt;
        self.next_rx = u64::MAX;
    }

    fn abort_async(&mut self) {
        self.tx = None;
        self.tx_buf = None;
        self.pending_tx = None;
        self.next_tx = u64::MAX;
        self.host = None;
        self.next_host = u64::MAX;
        self.udre = true;
        self.flush_rx();
    }

    /// Move a waiting or in-progress frame onto the generator's current grid.
    fn resync(&mut self, now: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        self.resync_tx(now, mux, ctx);
        self.resync_rx(now, mux);
    }

    fn resync_tx(&mut self, now: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some(live) = self.tx else {
            self.next_tx = u64::MAX;
            return;
        };
        let waiting = live.bit.is_none();
        let started = live.started;
        if waiting {
            let tick = self.next_txclk(now);
            if tick == now {
                self.begin_bit(now, mux, ctx);
            } else {
                self.next_tx = tick;
            }
            return;
        }
        if started == now {
            self.next_tx = self.next_txclk_after(now);
            return;
        }
        let tick = self.next_txclk(now);
        if tick == now {
            self.end_bit(now, mux, ctx);
        } else {
            self.next_tx = tick;
        }
    }

    fn resync_rx(&mut self, now: u64, mux: &PinMux) {
        match self.rx {
            Rx::Hunt => self.next_rx = u64::MAX,
            Rx::Confirm => {
                let (tick, _) = self.first_tick_at_or_after(now);
                if tick > now {
                    self.next_rx = tick;
                } else if mux.pad_high(self.rx_port, self.rx_bit) {
                    self.rx = Rx::Hunt;
                    self.next_rx = u64::MAX;
                } else {
                    self.begin_from_sample1(now);
                }
            }
            Rx::Vote(vote) => {
                let cycle = self.cycle_of_index(vote.next_index);
                if cycle > now {
                    self.next_rx = cycle;
                } else {
                    self.sample_vote(now, mux);
                }
            }
        }
    }

    fn sync_tx_pin(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let busy = self.tx.is_some() || self.tx_buf.is_some();
        let want = (self.async_mode() || self.mspim()) && (self.ucsrb & TXEN != 0 || busy);
        if want {
            if !self.tx_on_pin {
                self.drive_tx(true, at, mux, ctx);
                self.tx_on_pin = true;
            }
        } else if self.tx_on_pin {
            self.release_tx(at, mux, ctx);
            self.tx_on_pin = false;
        }
    }

    fn sync_rx_pin(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let force = (self.async_mode() || self.mspim()) && self.ucsrb & RXEN != 0;
        if force == self.rx_forced {
            return;
        }
        mux.set_force_input(self.rx_port, self.rx_bit, force, ctx, Some(at), at);
        self.rx_forced = force;
    }

    fn drive_tx(&self, high: bool, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let level = if high { Level::High } else { Level::Low };
        mux.set_override(
            DriveSource::UsartTx { pin: self.tx_pin },
            self.tx_port,
            self.tx_bit,
            Some(level),
            ctx,
            Some(at),
            at,
        );
    }

    fn release_tx(&self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        mux.set_override(
            DriveSource::UsartTx { pin: self.tx_pin },
            self.tx_port,
            self.tx_bit,
            None,
            ctx,
            Some(at),
            at,
        );
    }

    fn drive_rx(&self, level: Level, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        mux.drive(u32::from(self.rx_pin), level, ctx, at);
    }

    fn read_ucsra(&self) -> u8 {
        let mut value = self.ctrl & (U2X | MPCM);
        if self.udre {
            value |= UDRE_BIT;
        }
        if self.txc {
            value |= TXC_BIT;
        }
        if let Some(head) = self.head() {
            value |= RXC;
            if !self.mspim() && head.fe {
                value |= FE;
            }
            if !self.mspim() && head.dor {
                value |= DOR;
            }
            if !self.mspim() && head.upe {
                value |= UPE;
            }
        }
        value
    }

    fn head(&self) -> Option<RxByte> {
        self.rx_buf.front().copied()
    }

    fn rxc(&self) -> bool {
        !self.rx_buf.is_empty()
    }

    fn async_mode(&self) -> bool {
        self.ucsrc & UMSEL == 0
    }

    fn mspim(&self) -> bool {
        self.ucsrc & UMSEL == UMSEL
    }

    /// Next even baud tick at or after `at`. MSPIM bit boundaries use divisor 2
    /// on the same phase-0 count as `txclk` (§21.3, Table 21-1).
    fn next_even_tick(&self, at: u64) -> u64 {
        let (cycle, index) = self.first_tick_at_or_after(at);
        if index % 2 == 0 {
            cycle
        } else {
            self.cycle_of_index(index + 1)
        }
    }

    fn commit_mspim(&mut self, byte: u8, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        if self.mspim_sh.is_some() {
            self.mspim_buf = Some(byte);
            self.udre = false;
            return;
        }
        self.udre = true;
        self.arm_mspim(byte, at, mux, ctx);
    }

    fn arm_mspim(&mut self, byte: u8, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        self.mspim_sh = Some(Box::new(MspimSh {
            byte,
            rx: 0,
            bit: 0,
            phase: 0,
            half: 1,
            cpha: self.ucsrc & 0x02 != 0,
            cpol: self.ucsrc & 0x01 != 0,
            lsb: self.ucsrc & 0x04 != 0,
        }));
        self.mspim_miso = pop_front(&mut self.host_q).unwrap_or(0xFF);
        let boundary = self.next_even_tick(at);
        if boundary == at {
            self.begin_mspim(at, mux, ctx);
        } else {
            self.next_tx = boundary;
        }
    }

    fn begin_mspim(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let half = self.reg_period().max(1);
        let (cpha, cpol) = {
            let Some(sh) = self.mspim_sh.as_mut() else {
                self.next_tx = u64::MAX;
                return;
            };
            sh.half = half;
            sh.phase = 1;
            (sh.cpha, sh.cpol)
        };
        if !cpha {
            self.present_mspim(0, at, mux, ctx);
        }
        self.drive_xck(cpol, at, mux, ctx);
        self.next_tx = at + half;
    }

    fn fire_mspim(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some(phase) = self.mspim_sh.as_ref().map(|sh| sh.phase) else {
            self.next_tx = u64::MAX;
            return;
        };
        if phase == 0 {
            self.begin_mspim(at, mux, ctx);
            return;
        }
        let (half, cpha, cpol, lsb, bit) = {
            let sh = self.mspim_sh.as_ref().unwrap();
            (sh.half.max(1), sh.cpha, sh.cpol, sh.lsb, sh.bit)
        };
        if phase == 1 {
            if cpha {
                self.present_mspim(bit, at, mux, ctx);
            } else if self.sample_miso(mux) {
                self.deposit_mspim(bit, lsb);
            }
            self.drive_xck(!cpol, at, mux, ctx);
            if let Some(sh) = self.mspim_sh.as_mut() {
                sh.phase = 2;
            }
            self.next_tx = at + half;
            return;
        }
        if cpha && self.sample_miso(mux) {
            self.deposit_mspim(bit, lsb);
        }
        self.drive_xck(cpol, at, mux, ctx);
        let done = {
            let Some(sh) = self.mspim_sh.as_mut() else {
                self.next_tx = u64::MAX;
                return;
            };
            sh.bit = sh.bit.saturating_add(1);
            sh.bit == 8
        };
        if done {
            self.finish_mspim(at, mux, ctx);
            return;
        }
        if !cpha {
            let bit = self.mspim_sh.as_ref().map(|sh| sh.bit).unwrap_or(0);
            self.present_mspim(bit, at, mux, ctx);
        }
        if let Some(sh) = self.mspim_sh.as_mut() {
            sh.phase = 1;
        }
        self.next_tx = at + half;
    }

    fn finish_mspim(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let Some((byte, rx)) = self.mspim_sh.as_ref().map(|sh| (sh.byte, sh.rx)) else {
            self.next_tx = u64::MAX;
            return;
        };
        self.tx_out.push(Shifted {
            byte,
            cycle: at,
            start: None,
            end: ctx.oscillator_cycles(at),
        });
        self.push_slot(RxByte {
            data: rx,
            ninth: false,
            fe: false,
            dor: false,
            upe: false,
        });
        if let Some(next) = self.mspim_buf.take() {
            self.udre = true;
            if let Some(sh) = self.mspim_sh.as_mut() {
                sh.byte = next;
                sh.rx = 0;
                sh.bit = 0;
                sh.phase = 1;
            }
            self.mspim_miso = pop_front(&mut self.host_q).unwrap_or(0xFF);
            let (cpha, half) = self
                .mspim_sh
                .as_ref()
                .map(|sh| (sh.cpha, sh.half.max(1)))
                .unwrap_or((false, 1));
            if !cpha {
                self.present_mspim(0, at, mux, ctx);
            }
            self.next_tx = at + half;
            return;
        }
        self.mspim_sh = None;
        self.udre = true;
        self.txc = true;
        self.next_tx = u64::MAX;
    }

    fn present_mspim(&mut self, index: u8, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let (byte, lsb, miso) = self
            .mspim_sh
            .as_ref()
            .map(|sh| (sh.byte, sh.lsb, self.mspim_miso))
            .unwrap_or((0, false, 0xFF));
        let shift = if lsb { index } else { 7 - index };
        self.drive_tx((byte >> shift) & 1 == 1, at, mux, ctx);
        self.tx_on_pin = true;
        let level = if (miso >> shift) & 1 == 1 {
            Level::High
        } else {
            Level::Low
        };
        self.drive_rx(level, at, mux, ctx);
    }

    fn deposit_mspim(&mut self, index: u8, lsb: bool) {
        let Some(sh) = self.mspim_sh.as_mut() else {
            return;
        };
        let shift = if lsb { index } else { 7 - index };
        sh.rx |= 1 << shift;
    }

    fn sample_miso(&self, mux: &PinMux) -> bool {
        mux.pad_high(self.rx_port, self.rx_bit)
    }

    fn abort_mspim(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        self.mspim_sh = None;
        self.mspim_buf = None;
        self.next_tx = u64::MAX;
        self.udre = true;
        if self.xck_on {
            self.release_xck(at, mux, ctx);
            self.xck_on = false;
        }
    }

    fn sync_xck(&mut self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let want = self.mspim() && self.ucsrb & TXEN != 0;
        if want && self.mspim_sh.is_none() {
            self.drive_xck(self.ucsrc & 0x01 != 0, at, mux, ctx);
        } else if !want && self.xck_on {
            self.release_xck(at, mux, ctx);
            self.xck_on = false;
        }
    }

    fn xck_pad(&self) -> (Port, u8) {
        Port::from_pin(u32::from(self.xck_pin)).unwrap_or((Port::D, 4))
    }

    fn drive_xck(&mut self, high: bool, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let level = if high { Level::High } else { Level::Low };
        let (port, bit) = self.xck_pad();
        mux.set_override(
            DriveSource::UsartXck { pin: self.xck_pin },
            port,
            bit,
            Some(level),
            ctx,
            Some(at),
            at,
        );
        self.xck_on = true;
    }

    fn release_xck(&self, at: u64, mux: &mut PinMux, ctx: &mut Ctx<'_>) {
        let (port, bit) = self.xck_pad();
        mux.set_override(
            DriveSource::UsartXck { pin: self.xck_pin },
            port,
            bit,
            None,
            ctx,
            Some(at),
            at,
        );
    }

    fn reg_period(&self) -> u64 {
        u64::from(self.ubrr & 0x0FFF) + 1
    }

    fn divisor(&self) -> u64 {
        if self.ctrl & U2X != 0 { 8 } else { 16 }
    }

    /// Collapse a `UBRRnH`-only change once the running count has hit zero.
    /// The query helpers below also handle that one-tick transition, so this
    /// is only called with the cycle time has actually reached.
    fn sync_period(&mut self, now: u64) {
        let next = self.reg_period();
        if self.brg_period == next || now < self.brg_at.saturating_add(self.brg_period) {
            return;
        }
        let first = self.brg_at + self.brg_period;
        let steps = (now - first) / next;
        let last = first + steps * next;
        self.brg_ticks = self.brg_ticks + 1 + steps;
        self.brg_at = last;
        self.brg_tick = true;
        self.brg_period = next;
    }

    /// `UBRRnL` loads the down-counter at `now`. A zero on this same cycle
    /// still counts as a baud tick. The divider count is not cleared, and
    /// ticks already produced keep the period that was loaded then.
    fn reload(&mut self, now: u64) {
        let through = self.ticks_loaded(now);
        let hit = self.tick_loaded(now);
        self.brg_ticks = through;
        self.brg_tick = hit;
        self.brg_at = now;
        self.brg_period = self.reg_period();
    }

    /// Tick test against the period that is loaded, ignoring a `UBRRn` store
    /// that has not reached a zero or an `UBRRnL` write yet.
    fn tick_loaded(&self, cycle: u64) -> bool {
        if cycle < self.brg_at {
            return false;
        }
        if cycle == self.brg_at {
            return self.brg_tick;
        }
        (cycle - self.brg_at).is_multiple_of(self.brg_period)
    }

    fn ticks_loaded(&self, cycle: u64) -> u64 {
        if cycle <= self.brg_at {
            return self.brg_ticks;
        }
        self.brg_ticks + (cycle - self.brg_at) / self.brg_period
    }

    fn tick_index(&self, cycle: u64) -> u64 {
        if cycle <= self.brg_at {
            return self.brg_ticks;
        }
        let next = self.reg_period();
        if self.brg_period == next {
            return self.brg_ticks + (cycle - self.brg_at) / self.brg_period;
        }
        let first = self.brg_at + self.brg_period;
        if cycle <= first {
            return self.brg_ticks + 1;
        }
        self.brg_ticks + 1 + (cycle - first) / next
    }

    /// `(cycle, 1-based baud-tick index)` of the first tick at or after `at`.
    fn first_tick_at_or_after(&self, at: u64) -> (u64, u64) {
        if self.brg_tick && at <= self.brg_at {
            return (self.brg_at, self.brg_ticks);
        }
        if at <= self.brg_at {
            return self.tick_after_load(1);
        }
        let next = self.reg_period();
        if self.brg_period == next {
            let i = (at - self.brg_at).div_ceil(self.brg_period);
            return self.tick_after_load(i.max(1));
        }
        let first_cycle = self.brg_at + self.brg_period;
        let first_index = self.brg_ticks + 1;
        if at <= first_cycle {
            return (first_cycle, first_index);
        }
        let steps = (at - first_cycle).div_ceil(next);
        (first_cycle + steps * next, first_index + steps)
    }

    fn tick_after_load(&self, i: u64) -> (u64, u64) {
        let next = self.reg_period();
        if self.brg_period == next || i == 1 {
            return (self.brg_at + i * self.brg_period, self.brg_ticks + i);
        }
        let first = self.brg_at + self.brg_period;
        (first + (i - 1) * next, self.brg_ticks + i)
    }

    fn cycle_of_index(&self, index: u64) -> u64 {
        if index <= self.brg_ticks {
            return self.brg_at;
        }
        self.tick_after_load(index - self.brg_ticks).0
    }

    /// Next `txclk` at or after `at`. Divider phase 0: tick `k` (from 1) is
    /// a `txclk` when `k` is a multiple of 16, or of 8 when `U2X` is set.
    fn next_txclk(&self, at: u64) -> u64 {
        let div = self.divisor();
        let (cycle, index) = self.first_tick_at_or_after(at);
        let rem = index % div;
        if rem == 0 {
            return cycle;
        }
        self.cycle_of_index(index + (div - rem))
    }

    fn next_txclk_after(&self, at: u64) -> u64 {
        self.next_txclk(at.saturating_add(1))
    }

    fn bit_clocks(&self) -> u64 {
        let mul = if self.ctrl & U2X != 0 { 8 } else { 16 };
        mul * self.sample_period()
    }

    fn sample_period(&self) -> u64 {
        u64::from(self.ubrr & 0x0FFF) + 1
    }

    fn samples_per_bit(&self) -> u8 {
        if self.ctrl & U2X != 0 { 8 } else { 16 }
    }

    fn format_now(&self) -> (u8, ParityMode, u8) {
        (self.data_bits(), self.parity_mode(), self.stop_count())
    }

    fn data_bits(&self) -> u8 {
        let sz = u8::from(self.ucsrb & UCSZ2 != 0) << 2 | ((self.ucsrc >> 1) & 0x03);
        match sz {
            0 => 5,
            1 => 6,
            2 => 7,
            3 => 8,
            7 => 9,
            _ => 8,
        }
    }

    fn parity_mode(&self) -> ParityMode {
        match (self.ucsrc >> 4) & 0x03 {
            0b10 => ParityMode::Even,
            0b11 => ParityMode::Odd,
            _ => ParityMode::None,
        }
    }

    fn stop_count(&self) -> u8 {
        if self.ucsrc & USBS != 0 { 2 } else { 1 }
    }

    fn latch_tx(&self, value: u8) -> FrameBits {
        if self.mspim() {
            return FrameBits::new(u16::from(value), 8, ParityMode::None, 1);
        }
        let n = self.data_bits();
        let mut data = u16::from(value);
        if n == 9 && self.ucsrb & TXB8 != 0 {
            data |= 0x100;
        }
        FrameBits::new(data, n, self.parity_mode(), self.stop_count())
    }

    fn host_format(&self) -> (u64, u8, ParityMode, u8) {
        if let Some(over) = self.host_over {
            (over.bit_cycles, over.n_data, over.parity, over.n_stop)
        } else {
            let (n_data, parity, n_stop) = self.format_now();
            (self.bit_clocks(), n_data, parity, n_stop)
        }
    }
}

fn parity_from_code(code: u8) -> ParityMode {
    match code {
        1 => ParityMode::Even,
        2 => ParityMode::Odd,
        _ => ParityMode::None,
    }
}
