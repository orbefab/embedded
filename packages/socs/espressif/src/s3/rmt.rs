//! RMT transmit, channel 0, from its own 48-word block of `RMTMEM`.
//!
//! The channel clock is APB divided by `SCLK_DIV_NUM + 1` and then by `DIV_CNT`, and `DIV_CNT`
//! of 0 counts as 256. A store that sets `TX_START` on an idle channel snapshots that divider,
//! `IDLE_OUT_LV`, `TX_LIM` and the netted pads carrying signal 81, and starts at that store.
//! Each entry reads its word when the entry starts, so a word stored after that read is not
//! the one sent. The pad drives the idle level for the whole transmission.
//!
//! `TX_THR` sets as raw each time `TX_LIM` more entries have been sent, and `TX_END` sets when
//! an entry's duration is 0. Source 40 is high while `INT_ST` of channel 0 has a bit set.
//! IDF v5.5.5, `soc/esp32s3/register/soc/rmt_reg.h`.

use super::clock::{TICK_HZ, Tick};

/// TX channel 0's output in the GPIO matrix.
pub(crate) const SIGNAL: u16 = 81;

const MEM: u32 = 0x800;
const MEM_WORDS: usize = 384;
const BLOCK_WORDS: usize = 48;
/// Entries with no zero duration. The transmission ends the run here.
const ENTRY_LIMIT: u32 = 65_536;

const CH0CONF0: u32 = 0x20;
const CH_CONF_LAST: u32 = 0x2c;
const INT_RAW: u32 = 0x70;
const INT_ST: u32 = 0x74;
const INT_ENA: u32 = 0x78;
const INT_CLR: u32 = 0x7c;
const CH0_TX_LIM: u32 = 0xa0;
const SYS_CONF: u32 = 0xc0;
const TX_SIM: u32 = 0xc4;
const REF_CNT_RST: u32 = 0xc8;

const TX_START: u32 = 1 << 0;
const TX_CONTI: u32 = 1 << 3;
const MEM_TX_WRAP: u32 = 1 << 4;
const IDLE_OUT_LV: u32 = 1 << 5;
const IDLE_OUT_EN: u32 = 1 << 6;
const TX_STOP: u32 = 1 << 7;
const CONF_WT: u32 = TX_START | 1 << 1 | 1 << 2 | 1 << 23 | 1 << 24 | 1 << 25;
const CONF_KEPT: u32 = 0x007f_ff78;
const DMA_ACCESS: u32 = 1 << 25;

const TX_END_BIT: u32 = 1 << 0;
const TX_THR_BIT: u32 = 1 << 8;
const CH0_INT: u32 = TX_END_BIT | 1 << 4 | TX_THR_BIT | 1 << 12;

const APB_FIFO_MASK: u32 = 1 << 0;
const MEM_FORCE_PD: u32 = 1 << 2;
const SCLK_ACTIVE: u32 = 1 << 26;
const TX_SIM_EN: u32 = 1 << 4;
const RX_EN: u32 = 1 << 0;

const CONF_RESET: u32 = 0x0071_0200;
const RX_CONF0_RESET: u32 = 0x317f_ff02;
const RX_CONF1_RESET: u32 = 0x0000_01e8;
const LIM_RESET: u32 = 0x0000_0080;
const SYS_RESET: u32 = 0x0500_0010;
const LIM_KEPT: u32 = 0x002f_ffff;
const SYS_KEPT: u32 = 0x87ff_ffff;

/// One finished transmission on a single pad: the idle level, then each half-entry's level and
/// its duration in picoseconds. A duration of 0 is never present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Train {
    pub start: Tick,
    pub end: Tick,
    pub idle: bool,
    pub pad: u8,
    pub pulses: Vec<(bool, u64)>,
}

/// The part of a transmission still running at a cut: the pulses whose entries had started,
/// on the pads that carried the channel when it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flight {
    pub start: Tick,
    pub idle: bool,
    pub pads: Vec<u8>,
    pub pulses: Vec<(bool, u64)>,
}

/// A pad the matrix routes to signal 81.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Route {
    pub pad: u8,
    pub netted: bool,
    pub inverted: bool,
}

#[derive(Clone, Debug)]
struct Live {
    start: Tick,
    period: u64,
    idle: bool,
    tx_lim: u32,
    pads: Vec<u8>,
    word: usize,
    since: u32,
    begun: u32,
    /// The last entry's second half was zero. `cursor` is its end, and no further word is read.
    closed: bool,
    cursor: Tick,
    pulses: Vec<(bool, u64)>,
    starts: Vec<Tick>,
}

#[derive(Clone, Debug)]
struct Done {
    start: Tick,
    end: Tick,
    idle: bool,
    pads: Vec<u8>,
    pulses: Vec<(bool, u64)>,
}

#[derive(Clone, Debug)]
pub(crate) struct Rmt {
    /// Kept words, indexed by `offset / 4` for `0x00..=0xcc`.
    words: [u32; 0x34],
    mem: [u32; MEM_WORDS],
    live: Option<Live>,
    done: Vec<Done>,
    /// Raw bits that rise at this tick. The channel is still sending until then.
    pending: Option<(Tick, u32)>,
    /// Set when a transmission passes [`ENTRY_LIMIT`] with no end word.
    refused: Option<(Tick, &'static str)>,
}

impl Rmt {
    pub(crate) fn new() -> Rmt {
        let mut rmt = Rmt {
            words: [0; 0x34],
            mem: [0; MEM_WORDS],
            live: None,
            done: Vec::new(),
            pending: None,
            refused: None,
        };
        for offset in (0..0xd0).step_by(4) {
            if let Some(word) = reset_word(offset) {
                rmt.words[(offset / 4) as usize] = word;
            }
        }
        rmt
    }

    /// Whether an access at `offset` in the block is the APB FIFO.
    pub(crate) fn fifo(offset: u32) -> bool {
        offset <= 0x1c
    }

    /// A channel 0 `TX_START` is the store that reads which pads carry signal 81.
    pub(crate) fn wants_routes(offset: u32, value: u32) -> bool {
        offset == CH0CONF0 && value & TX_START != 0
    }

    pub(crate) fn read(&mut self, now: Tick, offset: u32) -> Option<u32> {
        self.settle(now);
        self.word_at(offset)
    }

    /// Stores `value` at `offset`. `Err` refuses before any write. `Ok(true)` started a
    /// transmission.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
        apb: u64,
        routes: &[Route],
    ) -> Result<bool, &'static str> {
        self.settle(now);
        if let Some(index) = mem_index(offset) {
            self.mem[index] = value;
            return Ok(false);
        }
        if reset_word(offset).is_none() {
            return Ok(false);
        }
        if let Some(reason) = self.refusal(offset, value, routes) {
            return Err(reason);
        }
        let started = self.store(now, offset, value, apb, routes);
        Ok(started)
    }

    /// Source 40's level at `now`: high while channel 0's `INT_ST` has a bit set.
    pub(crate) fn line(&mut self, now: Tick) -> bool {
        if !self.busy() {
            return self.line_high();
        }
        self.settle(now);
        self.line_high()
    }

    /// The level signal 81 drives: `IDLE_OUT_LV` while `IDLE_OUT_EN` is set, and low otherwise.
    pub(crate) fn level(&self) -> bool {
        let conf = self.words[(CH0CONF0 / 4) as usize];
        conf & IDLE_OUT_EN != 0 && conf & IDLE_OUT_LV != 0
    }

    /// Whether channel 0 is still sending at `now`. A transmission whose end is `now` is idle.
    pub(crate) fn sending(&mut self, now: Tick) -> bool {
        self.settle(now);
        self.live.is_some() || self.pending.is_some()
    }

    /// The tick the interrupt line next rises, if a threshold or the end will raise it.
    pub(crate) fn next_event(&self) -> Option<Tick> {
        if self.line_high() {
            return None;
        }
        let pending = self.pending_rise();
        let walked = self.live.as_ref().and_then(|live| self.walk(live, true));
        pending.into_iter().chain(walked).min()
    }

    /// The next tick the run must stop for: a line rise, the end, or the entry limit.
    pub(crate) fn upcoming(&self) -> Option<Tick> {
        let pending = self.pending.map(|(at, _)| at);
        let walked = self.live.as_ref().and_then(|live| self.walk(live, false));
        pending.into_iter().chain(walked).min()
    }

    pub(crate) fn settle(&mut self, now: Tick) {
        if !self.busy() {
            return;
        }
        self.apply(now);
        let Some(mut live) = self.live.take() else {
            return;
        };
        if live.closed {
            if live.cursor <= now {
                let end = live.cursor;
                self.done.push(done_from(live, end));
            } else {
                self.live = Some(live);
            }
            return;
        }
        while live.cursor <= now {
            if live.begun == ENTRY_LIMIT {
                self.refused = Some((live.cursor, "no_end"));
                return;
            }
            let word = self.mem[live.word];
            let (d0, level0, d1, level1) = halves(word);
            if d0 == 0 {
                self.raw_set(TX_END_BIT);
                let end = live.cursor;
                if !live.pulses.is_empty() {
                    self.done.push(done_from(live, end));
                }
                return;
            }
            push_pulse(&mut live, level0, d0);
            let end0 = add(live.cursor, d0, live.period);
            if d1 == 0 {
                self.count_entry(&mut live, end0, now);
                self.rise(now, end0, TX_END_BIT);
                if end0 <= now {
                    self.done.push(done_from(live, end0));
                    return;
                }
                live.cursor = end0;
                live.closed = true;
                self.live = Some(live);
                return;
            }
            let period = live.period;
            push_pulse_at(&mut live, level1, d1, end0, period);
            let end1 = add(end0, d1, period);
            live.word = (live.word + 1) % BLOCK_WORDS;
            live.cursor = end1;
            self.count_entry(&mut live, end1, now);
        }
        self.live = Some(live);
    }

    /// The refusal a transmission hit, once.
    pub(crate) fn take_refused(&mut self) -> Option<(Tick, &'static str)> {
        self.refused.take()
    }

    /// Finished transmissions, in the order they started. One entry per pad.
    pub(crate) fn drain(&mut self) -> Vec<Train> {
        let done = std::mem::take(&mut self.done);
        done.into_iter().flat_map(expand).collect()
    }

    /// A transmission that started by `at` and has not ended by then. The caller has already
    /// settled through `at`.
    pub(crate) fn covers(&self, at: Tick) -> bool {
        let open = |start: Tick, end: Tick| start <= at && end > at;
        self.live
            .as_ref()
            .is_some_and(|live| open(live.start, live.cursor))
            || self.done.iter().any(|done| open(done.start, done.end))
    }

    /// Pulses that had started by `at` of the transmission still running then.
    pub(crate) fn flight(&self, at: Tick) -> Option<Flight> {
        let live = self.live.as_ref()?;
        if live.start > at {
            return None;
        }
        let pulses = live
            .pulses
            .iter()
            .zip(&live.starts)
            .take_while(|(_, start)| **start <= at)
            .map(|(pulse, _)| *pulse)
            .collect();
        Some(Flight {
            start: live.start,
            idle: live.idle,
            pads: live.pads.clone(),
            pulses,
        })
    }

    fn refusal(&self, offset: u32, value: u32, routes: &[Route]) -> Option<&'static str> {
        if offset == CH0CONF0 && self.busy() && value & TX_STOP != 0 {
            return Some("stop_while_sending");
        }
        if offset == CH0CONF0 && self.busy() && value & TX_START != 0 {
            return Some("start_while_sending");
        }
        if (CH0CONF0..=CH_CONF_LAST).contains(&offset)
            && offset != CH0CONF0
            && value & (TX_START | IDLE_OUT_EN) != 0
        {
            return Some("channel");
        }
        if offset == CH0CONF0 && value & TX_START != 0 {
            return self.start_refusal(value, routes);
        }
        if (CH0CONF0..=CH_CONF_LAST).contains(&offset) && value & DMA_ACCESS != 0 {
            return Some("dma");
        }
        if offset == TX_SIM && value & TX_SIM_EN != 0 {
            return Some("sync");
        }
        if is_rx_conf1(offset) && value & RX_EN != 0 {
            return Some("rx");
        }
        None
    }

    fn start_refusal(&self, value: u32, routes: &[Route]) -> Option<&'static str> {
        let sys = self.words[(SYS_CONF / 4) as usize];
        if sys & APB_FIFO_MASK == 0 {
            return Some("fifo");
        }
        if sys & MEM_FORCE_PD != 0 {
            return Some("power_down");
        }
        if (sys >> 24) & 0x3 != 1 {
            return Some("clock_source");
        }
        if sys & SCLK_ACTIVE == 0 {
            return Some("sclk_active");
        }
        if (sys >> 12) & 0x3f != 0 {
            return Some("fractional_divider");
        }
        if (value >> 16) & 0xf != 1 {
            return Some("mem_size");
        }
        if value & (1 << 21) != 0 {
            return Some("carrier");
        }
        if value & TX_CONTI != 0 {
            return Some("loop");
        }
        if value & MEM_TX_WRAP == 0 {
            return Some("no_wrap");
        }
        if value & IDLE_OUT_EN == 0 {
            return Some("idle_out");
        }
        if routes.iter().any(|route| route.inverted) {
            return Some("inverted");
        }
        None
    }

    /// Applies a store that has already passed [`Self::refusal`].
    fn store(&mut self, now: Tick, offset: u32, value: u32, apb: u64, routes: &[Route]) -> bool {
        match offset {
            INT_ST => {}
            INT_CLR => self.words[(INT_RAW / 4) as usize] &= !value,
            INT_RAW => self.words[(INT_RAW / 4) as usize] &= !value,
            REF_CNT_RST => {}
            CH0CONF0 => {
                let start = value & TX_START != 0 && self.live.is_none();
                self.words[(offset / 4) as usize] = value & CONF_KEPT;
                if start {
                    self.begin(now, value, apb, routes);
                    return true;
                }
            }
            offset if (CH0CONF0..=CH_CONF_LAST).contains(&offset) => {
                self.words[(offset / 4) as usize] = value & CONF_KEPT;
            }
            CH0_TX_LIM..=0xac => {
                self.words[(offset / 4) as usize] = value & LIM_KEPT;
            }
            SYS_CONF => self.words[(offset / 4) as usize] = value & SYS_KEPT,
            TX_SIM => self.words[(offset / 4) as usize] = value & 0x1f & !TX_SIM_EN,
            _ => self.words[(offset / 4) as usize] = value,
        }
        false
    }

    fn begin(&mut self, now: Tick, conf: u32, apb: u64, routes: &[Route]) {
        let sys = self.words[(SYS_CONF / 4) as usize];
        let div = (conf >> 8) & 0xff;
        let div = if div == 0 { 256 } else { u64::from(div) };
        let integral = u64::from((sys >> 4) & 0xff) + 1;
        let lim = self.words[(CH0_TX_LIM / 4) as usize] & 0x1ff;
        self.live = Some(Live {
            start: now,
            period: integral * div * apb,
            idle: conf & IDLE_OUT_LV != 0,
            tx_lim: lim,
            pads: routes
                .iter()
                .filter(|route| route.netted)
                .map(|route| route.pad)
                .collect(),
            word: 0,
            since: 0,
            begun: 0,
            closed: false,
            cursor: now,
            pulses: Vec::new(),
            starts: Vec::new(),
        });
        self.settle(now);
    }

    fn count_entry(&mut self, live: &mut Live, at: Tick, now: Tick) {
        live.begun += 1;
        live.since += 1;
        if live.since == live.tx_lim {
            self.rise(now, at, TX_THR_BIT);
            live.since = 0;
        }
    }

    fn busy(&self) -> bool {
        self.live.is_some() || self.pending.is_some()
    }

    /// No channel is sending, and neither a finished train nor a refusal is waiting.
    /// `settle` then changes nothing, and `drain` and `take_refused` return nothing.
    pub(crate) fn quiet(&self) -> bool {
        !self.busy() && self.done.is_empty() && self.refused.is_none()
    }

    /// Sets `bit` at `at`. A bit whose tick is still ahead waits, so a poll before it sees the
    /// line low.
    fn rise(&mut self, now: Tick, at: Tick, bit: u32) {
        if at <= now {
            self.raw_set(bit);
            return;
        }
        self.pending = Some(match self.pending {
            Some((tick, bits)) => {
                debug_assert_eq!(tick, at, "one transmission has one future edge");
                (tick, bits | bit)
            }
            None => (at, bit),
        });
    }

    fn apply(&mut self, now: Tick) {
        if let Some((at, bits)) = self.pending
            && at <= now
        {
            self.raw_set(bits);
            self.pending = None;
        }
    }

    /// The pending edge, when its enable would raise a low line.
    fn pending_rise(&self) -> Option<Tick> {
        let (at, bits) = self.pending?;
        let ena = self.words[(INT_ENA / 4) as usize];
        let raw = self.words[(INT_RAW / 4) as usize];
        ((ena & !raw & bits & CH0_INT) != 0).then_some(at)
    }

    fn raw_set(&mut self, bit: u32) {
        self.words[(INT_RAW / 4) as usize] |= bit;
    }

    fn word_at(&self, offset: u32) -> Option<u32> {
        if let Some(index) = mem_index(offset) {
            return Some(self.mem[index]);
        }
        if offset == INT_ST {
            return Some(self.words[(INT_RAW / 4) as usize] & self.words[(INT_ENA / 4) as usize]);
        }
        if offset == INT_CLR || offset == REF_CNT_RST {
            return Some(0);
        }
        if offset == INT_RAW {
            return Some(self.words[(INT_RAW / 4) as usize]);
        }
        reset_word(offset).map(|_| {
            let word = self.words[(offset / 4) as usize];
            if (CH0CONF0..=CH_CONF_LAST).contains(&offset) {
                word & !CONF_WT
            } else {
                word
            }
        })
    }

    fn line_high(&self) -> bool {
        self.words[(INT_RAW / 4) as usize] & self.words[(INT_ENA / 4) as usize] & CH0_INT != 0
    }

    /// The earliest tick that matters. With `rise`, only a tick that raises a low line.
    fn walk(&self, live: &Live, rise: bool) -> Option<Tick> {
        if live.closed {
            return (!rise).then_some(live.cursor);
        }
        let mut word = live.word;
        let mut since = live.since;
        let mut begun = live.begun;
        let mut cursor = live.cursor;
        let ena = self.words[(INT_ENA / 4) as usize];
        let mut raw = self.words[(INT_RAW / 4) as usize];
        let rises = |raw: u32, bit: u32| ena & bit != 0 && raw & bit == 0;
        loop {
            if begun == ENTRY_LIMIT {
                return (!rise).then_some(cursor);
            }
            let (d0, _, d1, _) = halves(self.mem[word]);
            if d0 == 0 {
                return (rises(raw, TX_END_BIT) || !rise).then_some(cursor);
            }
            let end0 = add(cursor, d0, live.period);
            if d1 == 0 {
                let at_thr = since + 1 == live.tx_lim;
                if !rise || (at_thr && rises(raw, TX_THR_BIT)) || rises(raw, TX_END_BIT) {
                    return Some(end0);
                }
                return None;
            }
            let end1 = add(end0, d1, live.period);
            let at_thr = since + 1 == live.tx_lim;
            if at_thr && (!rise || rises(raw, TX_THR_BIT)) {
                return Some(end1);
            }
            if at_thr {
                raw |= TX_THR_BIT;
                since = 0;
            } else {
                since += 1;
            }
            begun += 1;
            word = (word + 1) % BLOCK_WORDS;
            cursor = end1;
        }
    }
}

fn reset_word(offset: u32) -> Option<u32> {
    Some(match offset {
        0x20 | 0x24 | 0x28 | 0x2c => CONF_RESET,
        0x30 | 0x38 | 0x40 | 0x48 => RX_CONF0_RESET,
        0x34 | 0x3c | 0x44 | 0x4c => RX_CONF1_RESET,
        INT_RAW | INT_ST | INT_ENA | INT_CLR => 0,
        0x90 | 0x94 | 0x98 | 0x9c => 0,
        0xa0 | 0xa4 | 0xa8 | 0xac => LIM_RESET,
        0xb0 | 0xb4 | 0xb8 | 0xbc => LIM_RESET,
        SYS_CONF => SYS_RESET,
        TX_SIM | REF_CNT_RST => 0,
        _ => return None,
    })
}

fn is_rx_conf1(offset: u32) -> bool {
    matches!(offset, 0x34 | 0x3c | 0x44 | 0x4c)
}

fn mem_index(offset: u32) -> Option<usize> {
    let index = offset.checked_sub(MEM)? / 4;
    (offset & 3 == 0 && (index as usize) < MEM_WORDS).then_some(index as usize)
}

fn halves(word: u32) -> (u32, bool, u32, bool) {
    (
        word & 0x7fff,
        word & 1 << 15 != 0,
        (word >> 16) & 0x7fff,
        word & 1 << 31 != 0,
    )
}

fn add(at: Tick, rmt_ticks: u32, period: u64) -> Tick {
    Tick(at.0 + u64::from(rmt_ticks) * period)
}

fn picoseconds(rmt_ticks: u32, period: u64) -> u64 {
    let soc = u128::from(rmt_ticks) * u128::from(period);
    u64::try_from(soc * 1_000_000_000_000 / u128::from(TICK_HZ)).expect("a pulse duration fits")
}

fn push_pulse(live: &mut Live, level: bool, rmt_ticks: u32) {
    push_pulse_at(live, level, rmt_ticks, live.cursor, live.period);
}

fn push_pulse_at(live: &mut Live, level: bool, rmt_ticks: u32, at: Tick, period: u64) {
    if rmt_ticks == 0 {
        return;
    }
    live.pulses.push((level, picoseconds(rmt_ticks, period)));
    live.starts.push(at);
}

fn done_from(live: Live, end: Tick) -> Done {
    Done {
        start: live.start,
        end,
        idle: live.idle,
        pads: live.pads,
        pulses: live.pulses,
    }
}

fn expand(done: Done) -> Vec<Train> {
    done.pads
        .into_iter()
        .map(|pad| Train {
            start: done.start,
            end: done.end,
            idle: done.idle,
            pad,
            pulses: done.pulses.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const APB: u64 = 6;

    fn pad() -> Route {
        Route {
            pad: 0,
            netted: true,
            inverted: false,
        }
    }

    fn entry(d0: u32, level0: bool, d1: u32, level1: bool) -> u32 {
        d0 | u32::from(level0) << 15 | d1 << 16 | u32::from(level1) << 31
    }

    fn sys(div_num: u32) -> u32 {
        1 | 1 << 24 | SCLK_ACTIVE | div_num << 4
    }

    fn conf(div: u32, idle: bool) -> u32 {
        1 | 1 << 4 | 1 << 6 | div << 8 | 1 << 16 | u32::from(idle) << 5
    }

    fn ps(ticks: u32, period: u64) -> u64 {
        u64::try_from(u128::from(ticks) * u128::from(period) * 1_000_000_000_000 / 480_000_000)
            .unwrap()
    }

    fn put(rmt: &mut Rmt, offset: u32, value: u32) {
        assert_eq!(rmt.write(Tick(0), offset, value, APB, &[pad()]), Ok(false));
    }

    fn arm(div: u32, div_num: u32, lim: u32, idle: bool, words: &[u32]) -> Rmt {
        let mut rmt = Rmt::new();
        put(&mut rmt, SYS_CONF, sys(div_num));
        put(&mut rmt, CH0_TX_LIM, lim);
        put(&mut rmt, INT_ENA, TX_END_BIT | TX_THR_BIT);
        for (index, word) in words.iter().enumerate() {
            put(&mut rmt, MEM + 4 * index as u32, *word);
        }
        assert_eq!(
            rmt.write(Tick(0), CH0CONF0, conf(div, idle), APB, &[pad()]),
            Ok(true)
        );
        rmt
    }

    fn one(level0: bool, high: u32, level1: bool, low: u32, period: u64) -> [(bool, u64); 2] {
        [(level0, ps(high, period)), (level1, ps(low, period))]
    }

    #[test]
    fn ten_entries_at_div_cnt_8_are_100_ns_pairs_then_the_end() {
        let symbol = entry(8, true, 4, false);
        let mut words = vec![symbol; 10];
        words.push(0);
        let mut rmt = arm(8, 0, 128, false, &words);
        let period = 48;
        let end = Tick(10 * (8 + 4) * period);
        let mut expect = Vec::new();
        for _ in 0..10 {
            expect.extend(one(true, 8, false, 4, period));
        }
        assert_eq!(rmt.read(Tick(end.0 - 1), INT_RAW).unwrap() & TX_END_BIT, 0);
        assert_eq!(rmt.read(end, INT_RAW).unwrap() & TX_END_BIT, TX_END_BIT);
        assert_eq!(rmt.read(end, INT_RAW).unwrap() & TX_THR_BIT, 0);
        let train = rmt.drain().pop().expect("a train");
        assert_eq!(train.pulses, expect);
        assert_eq!(train.end, end);
        assert!(!train.idle);
        assert_eq!(train.pad, 0);
    }

    #[test]
    fn a_zero_second_half_ends_after_the_first_and_still_counts() {
        let mut rmt = arm(1, 0, 1, false, &[entry(8, true, 0, false)]);
        let end = Tick(8 * 6);
        assert_eq!(
            rmt.read(Tick(end.0 - 1), INT_RAW).unwrap() & (TX_END_BIT | TX_THR_BIT),
            0
        );
        assert_eq!(
            rmt.read(end, INT_RAW).unwrap() & (TX_END_BIT | TX_THR_BIT),
            TX_END_BIT | TX_THR_BIT
        );
        let train = rmt.drain().pop().expect("the first half");
        assert_eq!(train.pulses, vec![(true, ps(8, 6))]);
        assert_eq!(train.end, end);
    }

    #[test]
    fn a_zero_first_half_ends_at_the_entry_start_with_no_train() {
        let mut rmt = arm(1, 0, 128, false, &[0]);
        assert!(rmt.drain().is_empty());
        assert_eq!(rmt.read(Tick(0), INT_RAW).unwrap() & TX_END_BIT, TX_END_BIT);
        assert!(!rmt.sending(Tick(0)));
    }

    #[test]
    fn sclk_div_num_1_doubles_every_duration_and_div_cnt_0_is_256() {
        let symbol = entry(2, true, 2, false);
        let once = finished(2, 0, false, &[symbol, 0]);
        let twice = finished(2, 1, false, &[symbol, 0]);
        let wide = finished(0, 0, true, &[entry(1, false, 1, true), 0]);
        assert_eq!(once.pulses, vec![(true, ps(2, 12)), (false, ps(2, 12))]);
        assert_eq!(twice.pulses, vec![(true, ps(2, 24)), (false, ps(2, 24))]);
        assert_eq!(
            wide.pulses,
            vec![(false, ps(1, 256 * 6)), (true, ps(1, 256 * 6))]
        );
        assert!(wide.idle);
    }

    fn finished(div: u32, div_num: u32, idle: bool, words: &[u32]) -> Train {
        let mut rmt = arm(div, div_num, 128, idle, words);
        let _ = rmt.read(Tick(10_000_000), INT_RAW);
        rmt.drain().pop().expect("a train")
    }

    #[test]
    fn the_line_follows_channel_0_status_and_a_clear_drops_it() {
        let mut rmt = arm(1, 0, 128, false, &[entry(2, true, 2, false), 0]);
        let end = Tick(4 * 6);
        assert!(!rmt.line(Tick(end.0 - 1)));
        assert!(rmt.line(end));
        assert_eq!(rmt.read(end, INT_ST).unwrap(), TX_END_BIT);
        put(&mut rmt, INT_CLR, TX_END_BIT);
        assert!(!rmt.line(end));
        assert_eq!(rmt.read(end, INT_ST).unwrap(), 0);
        assert_eq!(rmt.read(end, INT_RAW).unwrap() & TX_END_BIT, 0);
    }

    #[test]
    fn the_block_wraps_to_word_0_and_a_late_store_of_the_reread_is_what_is_sent() {
        let symbol = entry(1, true, 1, false);
        let mut words = vec![symbol; 48];
        words[0] = entry(1, false, 1, true);
        let mut rmt = arm(1, 0, 128, false, &words);
        let period = 6;
        let reread = Tick(48 * 2 * period);
        assert_eq!(rmt.write(reread, MEM + 4, 0, APB, &[pad()]), Ok(false));
        let _ = rmt.read(Tick(49 * 2 * period), INT_RAW);
        let train = rmt.drain().pop().expect("wrapped");
        let first = one(false, 1, true, 1, period);
        let pair = one(true, 1, false, 1, period);
        assert_eq!(&train.pulses[..2], &first);
        assert_eq!(train.pulses.len(), 49 * 2);
        assert_eq!(&train.pulses[48 * 2..], &first);
        assert_eq!(&train.pulses[2..4], &pair);
        assert_eq!(train.end, Tick(49 * 2 * period));
    }

    #[test]
    fn a_word_stored_after_its_read_is_not_the_one_sent() {
        let pattern = entry(4, true, 4, false);
        let poison = entry(9, true, 9, false);
        let mut words = vec![pattern; 26];
        words[24] = poison;
        words[25] = poison;
        let mut rmt = arm(1, 0, 24, false, &words);
        let period = 6;
        let entry_len = 8 * period;
        let threshold = Tick(24 * entry_len);
        assert_eq!(
            rmt.read(Tick(threshold.0 - 1), INT_RAW).unwrap() & TX_THR_BIT,
            0
        );
        let fresh = entry(3, false, 3, true);
        assert_eq!(
            rmt.write(threshold, MEM + 24 * 4, fresh, APB, &[pad()]),
            Ok(false)
        );
        assert_eq!(
            rmt.write(threshold, MEM + 25 * 4, fresh, APB, &[pad()]),
            Ok(false)
        );
        assert_eq!(
            rmt.write(threshold, MEM + 26 * 4, 0, APB, &[pad()]),
            Ok(false)
        );
        let _ = rmt.read(Tick(27 * entry_len), INT_RAW);
        let train = rmt.drain().pop().expect("refilled");
        let sent = &train.pulses[24 * 2..];
        assert_eq!(&sent[..2], &one(true, 9, false, 9, period));
        assert_eq!(&sent[2..], &one(false, 3, true, 3, period));
    }

    #[test]
    fn the_twelfth_threshold_and_the_end_are_the_same_tick() {
        let symbol = entry(1, true, 1, false);
        let words = vec![symbol; 48];
        let mut rmt = arm(1, 0, 24, false, &words);
        let twelfth = Tick(288 * 12);
        let eleventh = Tick(264 * 12);
        let tenth = Tick(240 * 12);
        let _ = rmt.read(tenth, INT_RAW);
        assert_eq!(
            rmt.write(tenth, INT_CLR, TX_THR_BIT, APB, &[pad()]),
            Ok(false)
        );
        assert_eq!(
            rmt.read(Tick(eleventh.0 - 1), INT_RAW).unwrap() & TX_THR_BIT,
            0
        );
        assert_eq!(
            rmt.read(eleventh, INT_RAW).unwrap() & TX_THR_BIT,
            TX_THR_BIT
        );
        assert_eq!(rmt.next_event(), None, "the line is already high");
        put(&mut rmt, INT_CLR, TX_THR_BIT);
        put(&mut rmt, INT_ENA, TX_END_BIT);
        assert_eq!(rmt.next_event(), None, "no end word yet");
        assert_eq!(rmt.write(eleventh, MEM, 0, APB, &[pad()]), Ok(false));
        assert_eq!(rmt.next_event(), Some(twelfth));
        assert_eq!(
            rmt.read(Tick(twelfth.0 - 1), INT_RAW).unwrap() & TX_END_BIT,
            0
        );
        let raw = rmt.read(twelfth, INT_RAW).unwrap();
        assert_eq!(raw & (TX_THR_BIT | TX_END_BIT), TX_THR_BIT | TX_END_BIT);
        assert_eq!(rmt.read(twelfth, INT_ST).unwrap() & TX_THR_BIT, 0);
        assert_eq!(rmt.read(twelfth, INT_ST).unwrap() & TX_END_BIT, TX_END_BIT);
    }

    #[test]
    fn tx_stop_reads_back_clear_and_that_word_does_not_stop_a_later_send() {
        let mut idle = Rmt::new();
        put(&mut idle, SYS_CONF, sys(0));
        assert_eq!(
            idle.write(Tick(0), CH0CONF0, TX_STOP, APB, &[pad()]),
            Ok(false)
        );
        let read = idle.read(Tick(0), CH0CONF0).unwrap();
        assert_eq!(read & TX_STOP, 0);
        let mut sending = arm(1, 0, 128, false, &[entry(8, true, 8, false), 0]);
        assert_eq!(
            sending.write(Tick(0), CH0CONF0, read, APB, &[pad()]),
            Ok(false)
        );
        assert!(sending.sending(Tick(0)));
    }

    #[test]
    fn a_start_keeps_the_refused_conf_word() {
        let mut rmt = Rmt::new();
        let err = rmt
            .write(Tick(0), CH0CONF0, conf(2, false), APB, &[pad()])
            .unwrap_err();
        assert_eq!(err, "fifo");
        assert_eq!(rmt.read(Tick(0), CH0CONF0), Some(CONF_RESET));
    }

    #[test]
    fn each_refused_setting_names_itself_and_writes_nothing() {
        let reasons = [
            (0x24, TX_START, "channel"),
            (0x24, IDLE_OUT_EN, "channel"),
            (0x2c, DMA_ACCESS, "dma"),
            (0x2c, DMA_ACCESS | TX_START, "channel"),
            (TX_SIM, TX_SIM_EN, "sync"),
            (0x34, RX_EN, "rx"),
        ];
        for (offset, value, reason) in reasons {
            let mut rmt = Rmt::new();
            assert_eq!(
                rmt.write(Tick(0), offset, value, APB, &[pad()]),
                Err(reason),
                "offset {offset:#x}"
            );
        }
        let mut ready = Rmt::new();
        put(&mut ready, SYS_CONF, sys(0));
        let start = [
            (sys(0) | MEM_FORCE_PD, conf(2, false), "power_down"),
            (1, conf(2, false), "clock_source"),
            (1 | 1 << 24, conf(2, false), "sclk_active"),
            (sys(0) | 1 << 12, conf(2, false), "fractional_divider"),
            (
                sys(0),
                (conf(2, false) & !(0xf << 16)) | (2 << 16),
                "mem_size",
            ),
            (
                sys(0),
                CONF_RESET | TX_START | MEM_TX_WRAP | IDLE_OUT_EN,
                "carrier",
            ),
            (sys(0), conf(2, false) | TX_CONTI, "loop"),
            (sys(0), conf(2, false) & !MEM_TX_WRAP, "no_wrap"),
            (sys(0), conf(2, false) & !IDLE_OUT_EN, "idle_out"),
        ];
        for (sys_word, conf_word, reason) in start {
            let mut rmt = Rmt::new();
            put(&mut rmt, SYS_CONF, sys_word);
            assert_eq!(
                rmt.write(Tick(0), CH0CONF0, conf_word, APB, &[pad()]),
                Err(reason)
            );
            assert_eq!(rmt.read(Tick(0), CH0CONF0), Some(CONF_RESET));
        }
        let mut inverted = Rmt::new();
        put(&mut inverted, SYS_CONF, sys(0));
        let flipped = Route {
            inverted: true,
            ..pad()
        };
        assert_eq!(
            inverted.write(Tick(0), CH0CONF0, conf(2, false), APB, &[flipped]),
            Err("inverted")
        );
        let mut sending = arm(1, 0, 128, false, &[entry(8, true, 8, false), 0]);
        assert_eq!(
            sending.write(Tick(0), CH0CONF0, TX_STOP | 1 << 21, APB, &[pad()]),
            Err("stop_while_sending")
        );
        assert_eq!(
            sending.write(Tick(0), CH0CONF0, TX_START, APB, &[pad()]),
            Err("start_while_sending")
        );
        assert_eq!(
            ready.write(Tick(0), CH0CONF0, conf(2, false), APB, &[pad()]),
            Ok(true)
        );
    }
}
