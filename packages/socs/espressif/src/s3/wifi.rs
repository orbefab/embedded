//! The Wi-Fi radio's registers, which a chip built with a radio keeps, and the MAC's transmit
//! path.
//!
//! IDF documents none of them. The model keeps the words a stock Arduino-ESP32 3.3.12 station
//! sketch touches through `esp_wifi_init`, `esp_wifi_start`, a scan and a join: the PHY's in
//! FE2, FE, the analog I2C master past the words [`Analog`](super::analog::Analog) keeps, BT, the
//! range below NRX that IDF names no block for, NRX and BB, and the MAC's in RADIO. Each reads the word last
//! written, 0 before the first write. Four reads also carry a done bit that the driver polls for,
//! in [`STATUS`] and [`HAL_INIT`]. Each of those values, and each 0 before a write, was chosen so
//! the driver proceeds, and is not yet read from hardware.
//!
//! The MAC sends one frame at a time from transmit queue `q`. The meanings below come from the
//! driver's code, the ROM's `mac_tx_set_plcp0`, `mac_tx_set_plcp1`, `hal_mac_txq_enable`,
//! `wDev_ProcessFiq` and `lmacProcessTxComplete`. None has been checked on a chip, so each is a
//! guess:
//! - A store that sets bit 31 of the queue's [`PLCP0`] word starts it. Bits 0 to 19 are the
//!   address of its first DMA descriptor, an `lldesc_t` in SRAM.
//! - Bits 0 to 11 of its [`PLCP1`] word are the PSDU's length with the FCS, and bits 12 to 16 its
//!   rate as a `wifi_phy_rate_t`.
//! - The descriptors' buffers hold the MPDU and then 4 bytes for the FCS, which the MAC fills. The
//!   MAC sends the MPDU as it is.
//! - The frame starts [`DIFS`] after the store, with no backoff and no carrier sense, on the
//!   channel the RF PLL was last started on ([`FREQ`]).
//! - When the frame is done, the MAC sets the queue's bit in [`TX_DONE`], its outcome in its
//!   [`TX_STATUS`] word and [`EVENT_TX_DONE`] in [`EVENTS`]. The interrupt, source
//!   [`SOURCE`], is high while an event bit is set in [`EVENTS`] and in [`EVENTS_ENABLE`].
//! - A group-addressed frame is done at its end, and an individually addressed one SIFS and an ACK
//!   after it, acknowledged or not.
//!
//! The radio hears nothing: no frame it receives reaches the driver, and no MAC timer counts. Any
//! other access in these blocks is refused or listed as on a chip with no radio.

use std::sync::Arc;

use orbe_model::air::{Ack, Mcs, Ofdm, Preamble, Rate};

use super::clock::{TICK_HZ, Tick};

/// The words kept, sorted.
const WORDS: [u32; 269] = [
    // FE2
    0x6000_50f0,
    // FE
    0x6000_6000,
    0x6000_6014,
    0x6000_6018,
    0x6000_601c,
    0x6000_6020,
    0x6000_6024,
    0x6000_6028,
    0x6000_602c,
    0x6000_6030,
    0x6000_6040,
    0x6000_6044,
    0x6000_6048,
    0x6000_6050,
    0x6000_6078,
    0x6000_607c,
    0x6000_608c,
    0x6000_6090,
    0x6000_6094,
    0x6000_60b0,
    0x6000_60b4,
    0x6000_60b8,
    0x6000_60bc,
    0x6000_60c8,
    0x6000_60cc,
    0x6000_60e0,
    0x6000_60e4,
    0x6000_60e8,
    0x6000_60ec,
    0x6000_60f0,
    0x6000_60f4,
    0x6000_60f8,
    0x6000_60fc,
    0x6000_6100,
    0x6000_6104,
    0x6000_610c,
    0x6000_6110,
    0x6000_6118,
    0x6000_611c,
    0x6000_6140,
    0x6000_6144,
    0x6000_6148,
    0x6000_614c,
    0x6000_6150,
    0x6000_6154,
    0x6000_615c,
    0x6000_6160,
    0x6000_6164,
    0x6000_6174,
    0x6000_6180,
    0x6000_6184,
    0x6000_6188,
    0x6000_618c,
    0x6000_6190,
    0x6000_6194,
    0x6000_6198,
    0x6000_619c,
    0x6000_61a0,
    0x6000_61a4,
    0x6000_61a8,
    0x6000_61ac,
    0x6000_61b0,
    0x6000_61b4,
    0x6000_61b8,
    0x6000_61bc,
    0x6000_61c0,
    0x6000_61e4,
    // I2C_MST, past the words `analog` keeps
    0x6000_e04c,
    0x6000_e050,
    0x6000_e054,
    0x6000_e058,
    0x6000_e05c,
    0x6000_e060,
    0x6000_e080,
    0x6000_e084,
    0x6000_e088,
    0x6000_e08c,
    0x6000_e090,
    0x6000_e094,
    0x6000_e098,
    0x6000_e09c,
    0x6000_e0c0,
    0x6000_e0c4,
    0x6000_e0c8,
    0x6000_e0cc,
    0x6000_e0d0,
    0x6000_e0d4,
    0x6000_e0d8,
    0x6000_e0dc,
    0x6000_e0e0,
    0x6000_e0e4,
    0x6000_e0e8,
    0x6000_e100,
    0x6000_e104,
    0x6000_e118,
    0x6000_e11c,
    0x6000_e120,
    0x6000_e128,
    0x6000_e12c,
    0x6000_e130,
    0x6000_e148,
    0x6000_e150,
    0x6000_e164,
    0x6000_e168,
    0x6000_e170,
    // BT
    0x6001_104c,
    0x6001_1848,
    // the range below NRX that IDF names no block for
    0x6001_c010,
    0x6001_c014,
    0x6001_c018,
    0x6001_c01c,
    0x6001_c02c,
    0x6001_c030,
    0x6001_c034,
    0x6001_c044,
    0x6001_c05c,
    0x6001_c064,
    0x6001_c068,
    0x6001_c074,
    0x6001_c080,
    0x6001_c08c,
    0x6001_c094,
    0x6001_c0a0,
    0x6001_c0a4,
    0x6001_c0cc,
    0x6001_c0d0,
    0x6001_c104,
    0x6001_c108,
    0x6001_c114,
    0x6001_c11c,
    0x6001_c120,
    0x6001_c124,
    0x6001_c130,
    0x6001_c134,
    0x6001_c13c,
    0x6001_c1b0,
    0x6001_c400,
    0x6001_c804,
    0x6001_c85c,
    0x6001_c860,
    0x6001_c87c,
    // NRX
    0x6001_cc0c,
    0x6001_cc48,
    0x6001_ccb8,
    0x6001_ccd4,
    0x6001_cd04,
    0x6001_cd08,
    0x6001_cd0c,
    // BB
    0x6001_d000,
    0x6001_d014,
    0x6001_d018,
    0x6001_d03c,
    0x6001_d040,
    0x6001_d044,
    0x6001_d050,
    0x6001_d054,
    0x6001_d06c,
    // the MAC, in RADIO
    0x6003_3000,
    0x6003_3004,
    0x6003_3020,
    0x6003_3024,
    0x6003_302c,
    0x6003_3034,
    0x6003_3040,
    0x6003_3044,
    0x6003_3048,
    0x6003_304c,
    0x6003_3060,
    0x6003_3064,
    0x6003_3068,
    0x6003_306c,
    0x6003_3074,
    0x6003_3080,
    0x6003_3084,
    0x6003_3088,
    0x6003_309c,
    0x6003_30d8,
    0x6003_30dc,
    0x6003_30e0,
    0x6003_30e4,
    0x6003_3100,
    0x6003_3104,
    0x6003_3108,
    0x6003_310c,
    0x6003_3114,
    0x6003_3118,
    0x6003_311c,
    0x6003_3120,
    0x6003_3124,
    0x6003_3128,
    0x6003_312c,
    0x6003_3130,
    0x6003_3134,
    0x6003_313c,
    0x6003_3140,
    0x6003_3144,
    0x6003_3148,
    0x6003_314c,
    0x6003_3150,
    0x6003_3158,
    0x6003_315c,
    0x6003_3160,
    0x6003_3164,
    0x6003_3168,
    0x6003_316c,
    0x6003_32a8,
    0x6003_32ac,
    0x6003_32b8,
    0x6003_3400,
    0x6003_3404,
    0x6003_3408,
    0x6003_340c,
    0x6003_3410,
    0x6003_3414,
    0x6003_3418,
    0x6003_3800,
    0x6003_3804,
    0x6003_3808,
    0x6003_380c,
    0x6003_3810,
    0x6003_3814,
    0x6003_3840,
    0x6003_3c10,
    0x6003_3c14,
    0x6003_3c18,
    0x6003_3c34,
    0x6003_3c3c,
    0x6003_3c40,
    0x6003_3c54,
    0x6003_3c5c,
    0x6003_3c60,
    0x6003_3c64,
    0x6003_3c6c,
    0x6003_3c74,
    0x6003_3c78,
    0x6003_3c88,
    0x6003_3c94,
    0x6003_3ca0,
    0x6003_3cac,
    0x6003_3cb0,
    0x6003_3d04,
    0x6003_3d08,
    0x6003_3d14,
    0x6003_4100,
    0x6003_414c,
    0x6003_4198,
    0x6003_41e4,
    0x6003_4230,
    0x6003_427c,
    0x6003_42c8,
    0x6003_42f8,
    0x6003_42fc,
    0x6003_4310,
    0x6003_4314,
    0x6003_4318,
    0x6003_4320,
    0x6003_5000,
    0x6003_5024,
    0x6003_5028,
    0x6003_502c,
    0x6003_5058,
    0x6003_5084,
    0x6003_5094,
    0x6003_509c,
    0x6003_50a8,
    0x6003_50b8,
    0x6003_5118,
    0x6003_511c,
    0x6003_5128,
];

/// Bits a read sets on top of the word kept, each one a poll the driver waits on.
const STATUS: [(u32, u32); 3] = [
    // `rom_pkdet_vol_start` waits for bits 24 to 26 of I2C_MST+0x50 to read 7.
    (0x6000_e050, 7 << 24),
    // `txdc_cal_v70` waits for bit 24 of I2C_MST+0x4c.
    (0x6000_e04c, 1 << 24),
    // `ram_iq_est_enable` waits for bit 16 of FE+0x174.
    (0x6000_6174, 1 << 16),
];

/// The MAC word `hal_init` sets bit 1 of, [`HAL_INIT_START`], and then waits on until bit 0,
/// [`HAL_INIT_DONE`], reads 1. Bit 0 reads 1 from the write that sets bit 1.
const HAL_INIT: u32 = 0x6003_3d14;
const HAL_INIT_START: u32 = 1 << 1;
const HAL_INIT_DONE: u32 = 1 << 0;

/// `ETS_WIFI_MAC_INTR_SOURCE`. IDF v5.5.5, `components/soc/esp32s3/include/soc/interrupts.h`.
pub(crate) const SOURCE: u32 = 0;

/// The transmit queues whose bits `lmacProcessTxComplete` reads from [`TX_DONE`].
const QUEUES: u32 = 4;
/// Queue 0's PLCP0 word, `0xc0067a1 << 3` in `hal_mac_txq_enable`. Queue `q`'s is `8 q` below.
const PLCP0: u32 = 0x6003_3d08;
/// Bit 31 of a PLCP0 word, which starts the queue. `hal_mac_txq_enable` sets it with bit 30,
/// which `hal_mac_is_txq_valid` reads. Which of the two starts the queue is a guess.
const PLCP0_START: u32 = 1 << 31;
/// The DMA descriptor's address in a PLCP0 word, as `mac_tx_set_plcp0` writes it. The high bits
/// are SRAM's on the data bus, a guess.
const PLCP0_DESCRIPTOR: u32 = 0xf_ffff;
const SRAM_DATA: u32 = 0x3fc0_0000;
/// Queue 0's PLCP1 word, the base of `mac_tx_set_plcp1`. Queue `q`'s words sit [`STRIDE`] `q`
/// below queue 0's, as the ROM's `mul -76` puts them.
const PLCP1: u32 = 0x6003_42f8;
const STRIDE: u32 = 0x4c;
/// The PSDU's length and rate in a PLCP1 word, a guess from `mac_tx_set_plcp1`.
const PLCP1_LENGTH: u32 = 0xfff;
const PLCP1_RATE_SHIFT: u32 = 12;
const PLCP1_RATE: u32 = 0x1f;
/// Queue 0's status word, which `hal_mac_get_txq_pmd` reads. Bits 12 to 15 are the outcome that
/// `lmacProcessTxComplete` dispatches on: 0 runs `lmacProcessTxSuccess` and 5
/// `lmacProcessAckTimeout`. Every other bit reads 0, a guess.
const TX_STATUS: u32 = 0x6003_4320;
const TX_SUCCESS: u32 = 0;
const TX_ACK_TIMEOUT: u32 = 5 << 12;
/// The MAC's event bits, which `hal_mac_interrupt_get_event` reads and `wDev_ProcessFiq`
/// dispatches on, the bits a store to [`EVENTS_CLEAR`] clears, and the word whose bits let an
/// event raise the interrupt. `hal_enable_mac` writes the last, which carries
/// [`EVENT_TX_DONE`]. That it masks the interrupt is a guess.
const EVENTS: u32 = 0x6003_3c3c;
const EVENTS_CLEAR: u32 = 0x6003_3c40;
const EVENTS_ENABLE: u32 = 0x6003_3c34;
/// The event `wDev_ProcessFiq` answers with `lmacPostTxComplete`.
const EVENT_TX_DONE: u32 = 1 << 7;
/// One bit per queue that finished a frame, as `hal_mac_get_txq_state(2)` reads it, and the word
/// `hal_mac_clr_txq_state(2)` ORs a queue's bit into to clear it. That word reads 0, a guess.
const TX_DONE: u32 = 0x6003_3cb0;
const TX_DONE_CLEAR: u32 = 0x6003_3cac;

/// The RF PLL's word in the analog I2C master's block, as `set_chan_freq_sw_start` writes it:
/// bits 0 to 7 the frequency above 2400 MHz in half megahertz, and bit 8 the pulse that starts
/// the PLL on it. The function also writes the frequency in whole megahertz to bits 20 to 27 of
/// I2C_MST+0x150. The field's unit is a guess that the two agree on.
const FREQ: u32 = 0x6000_e0c4;
const FREQ_HALF_MHZ: u32 = 0xff;
const FREQ_START: u32 = 1 << 8;

/// When the frame starts after the store that starts its queue: DIFS, aSIFSTime (10 us) and two
/// of the long aSlotTime (20 us) of the DSSS and ERP PHYs (IEEE 802.11-2020, 10.3.2.3, 15.4.4
/// and 18.5.4). The MAC draws no backoff and senses no carrier, a guess.
const DIFS: Tick = Tick(50 * TICK_HZ / 1_000_000);

/// The radio's words, in [`WORDS`] order, and the MAC's transmit path.
#[derive(Clone, Debug)]
pub(crate) struct Radio {
    words: [u32; WORDS.len()],
    /// The channel the RF PLL was last started on, `None` before then or off a channel's centre.
    channel: Option<u8>,
    /// The frame on the air, until the run reports its end.
    airing: Option<Frame>,
    /// The frame whose end the run reported, until it is taken.
    aired: Option<Frame>,
    /// The frame that is not done yet.
    pending: Option<Pending>,
    /// The bits of [`EVENTS`].
    events: u32,
    /// The bits of [`TX_DONE`].
    done: u32,
    /// Each queue's [`TX_STATUS`] word.
    status: [u32; QUEUES as usize],
}

/// A frame the MAC sent, from its first preamble symbol to the end of its airtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) start: Tick,
    pub(crate) end: Tick,
    /// 1 to 14.
    pub(crate) channel: u8,
    pub(crate) rate: Rate,
    /// Without the FCS.
    pub(crate) mpdu: Arc<[u8]>,
}

/// A frame that is not done yet.
#[derive(Clone, Copy, Debug)]
struct Pending {
    queue: u32,
    /// When the MAC is done with it.
    at: Tick,
    /// Whether it waits for an ACK.
    waits: bool,
    acked: bool,
}

/// What a store to the radio did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    /// The radio does not keep the word.
    Unkept,
    Kept,
    /// It kept the word and started a frame, whose end the run reads.
    Sent,
    /// It kept the word, which asks for a setting the model does not run, by name.
    Refused(&'static str),
}

impl Radio {
    pub(crate) fn new() -> Radio {
        Radio {
            words: [0; WORDS.len()],
            channel: None,
            airing: None,
            aired: None,
            pending: None,
            events: 0,
            done: 0,
            status: [0; QUEUES as usize],
        }
    }

    fn slot(addr: u32) -> Option<usize> {
        WORDS.binary_search(&addr).ok()
    }

    fn word(&self, addr: u32) -> u32 {
        Self::slot(addr).map_or(0, |slot| self.words[slot])
    }

    /// The transmit queue whose word at queue 0's `base` lies at `addr`, `stride` apart.
    fn queue(addr: u32, base: u32, stride: u32) -> Option<u32> {
        let below = base.checked_sub(addr)?;
        (below % stride == 0 && below / stride < QUEUES).then_some(below / stride)
    }

    /// The word at `addr`, a multiple of 4, at `now`, when the radio keeps it.
    pub(crate) fn read(&mut self, now: Tick, addr: u32) -> Option<u32> {
        let word = self.words[Self::slot(addr)?];
        self.settle(now);
        if let Some(queue) = Self::queue(addr, TX_STATUS, STRIDE) {
            return Some(self.status[queue as usize]);
        }
        let status = STATUS
            .iter()
            .find(|&&(at, _)| at == addr)
            .map_or(0, |&(_, bits)| bits);
        let done = if addr == HAL_INIT && word & HAL_INIT_START != 0 {
            HAL_INIT_DONE
        } else {
            0
        };
        Some(match addr {
            EVENTS => self.events,
            TX_DONE => self.done,
            TX_DONE_CLEAR => 0,
            _ => word | status | done,
        })
    }

    /// Keeps `value` at `addr`, a multiple of 4, at `now`, and runs what it starts. `byte` reads
    /// SRAM on the data bus, `None` outside it.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        addr: u32,
        value: u32,
        byte: impl Fn(u32) -> Option<u8>,
    ) -> Store {
        let Some(slot) = Self::slot(addr) else {
            return Store::Unkept;
        };
        self.settle(now);
        let kept = std::mem::replace(&mut self.words[slot], value);
        let starts = |bit: u32| value & bit != 0 && kept & bit == 0;
        match addr {
            EVENTS_CLEAR => self.events &= !value,
            TX_DONE_CLEAR => self.done &= !value,
            FREQ if starts(FREQ_START) => self.channel = channel(value & FREQ_HALF_MHZ),
            _ => {}
        }
        match Self::queue(addr, PLCP0, 8) {
            Some(queue) if starts(PLCP0_START) => match self.send(now, queue, value, byte) {
                Ok(()) => Store::Sent,
                Err(setting) => Store::Refused(setting),
            },
            _ => Store::Kept,
        }
    }

    /// Puts queue `queue`'s frame on the air, from the store at `now` of `plcp0`.
    fn send(
        &mut self,
        now: Tick,
        queue: u32,
        plcp0: u32,
        byte: impl Fn(u32) -> Option<u8>,
    ) -> Result<(), &'static str> {
        if self.airing.is_some() || self.pending.is_some() {
            return Err("tx_busy");
        }
        let channel = self.channel.ok_or("tx_channel")?;
        let plcp1 = self.word(PLCP1 - STRIDE * queue);
        let rate = rate((plcp1 >> PLCP1_RATE_SHIFT) & PLCP1_RATE).ok_or("tx_rate")?;
        let length = (plcp1 & PLCP1_LENGTH) as usize;
        let psdu = gather(SRAM_DATA | plcp0 & PLCP0_DESCRIPTOR, byte).ok_or("tx_descriptor")?;
        // An MPDU holds at least Frame Control, Duration and Address 1 (9.3.1).
        if psdu.len() != length || length < FCS + 10 {
            return Err("tx_length");
        }
        let mpdu: Arc<[u8]> = Arc::from(&psdu[..length - FCS]);
        // Address 1's Individual/Group bit (9.2.4.3.1). A group-addressed frame is not
        // acknowledged (10.3.2.11).
        let waits = mpdu[4] & 1 == 0;
        let start = Tick(now.0 + DIFS.0);
        let end = after(start, rate.airtime(length));
        let done = if waits {
            after(end, rate.acked_after())
        } else {
            end
        };
        self.airing = Some(Frame {
            start,
            end,
            channel,
            rate,
            mpdu,
        });
        self.pending = Some(Pending {
            queue,
            at: done,
            waits,
            acked: false,
        });
        Ok(())
    }

    /// The end of the frame on the air, which the run has not reported.
    pub(crate) fn end(&self) -> Option<Tick> {
        self.airing.as_ref().map(|frame| frame.end)
    }

    /// The run reports the end of the frame on the air, at `at`.
    pub(crate) fn report(&mut self, at: Tick) {
        if self.end() == Some(at) {
            self.aired = self.airing.take();
        }
    }

    /// The frame whose end the run reported, once.
    pub(crate) fn take_aired(&mut self) -> Option<Frame> {
        self.aired.take()
    }

    /// Whether a station acknowledged the frame that just ended.
    pub(crate) fn acked(&mut self, ack: Ack) {
        if let Some(pending) = &mut self.pending {
            pending.acked = ack == Ack::Acked;
        }
    }

    /// The channel the radio is tuned to.
    pub(crate) fn tuned(&self) -> Option<u8> {
        self.channel
    }

    /// Finishes the frame that is done by `now`.
    fn settle(&mut self, now: Tick) {
        let Some(pending) = self.pending.filter(|pending| pending.at <= now) else {
            return;
        };
        self.pending = None;
        self.status[pending.queue as usize] = if pending.waits && !pending.acked {
            TX_ACK_TIMEOUT
        } else {
            TX_SUCCESS
        };
        self.done |= 1 << pending.queue;
        self.events |= EVENT_TX_DONE;
    }

    /// The MAC interrupt's level at `now`.
    pub(crate) fn line(&mut self, now: Tick) -> bool {
        self.settle(now);
        self.events & self.word(EVENTS_ENABLE) != 0
    }

    /// The first tick after `now` at which the MAC interrupt can rise.
    pub(crate) fn next_event(&mut self, now: Tick) -> Option<Tick> {
        self.settle(now);
        self.pending.map(|pending| pending.at)
    }
}

/// The FCS after the MPDU.
const FCS: usize = 4;
/// The most descriptors a frame's chain may have before the model calls it a loop.
const DESCRIPTORS: usize = 64;

/// The bytes of the chain of `lldesc_t` from `first`: `size` in bits 0 to 11 of the first word,
/// `length` in bits 12 to 23 and `eof` in bit 30, then the buffer's address and the next
/// descriptor's. IDF v5.5.5, `components/esp_rom/include/esp_rom_lldesc.h`. `None` when a word or
/// a byte lies outside SRAM, or the chain does not end.
fn gather(first: u32, byte: impl Fn(u32) -> Option<u8>) -> Option<Vec<u8>> {
    let word = |at: u32| -> Option<u32> {
        let bytes = [byte(at)?, byte(at + 1)?, byte(at + 2)?, byte(at + 3)?];
        Some(u32::from_le_bytes(bytes))
    };
    let mut bytes = Vec::new();
    let mut at = first;
    for _ in 0..DESCRIPTORS {
        let (head, buffer, next) = (word(at)?, word(at + 4)?, word(at + 8)?);
        let length = (head >> 12) & 0xfff;
        for k in 0..length {
            bytes.push(byte(buffer + k)?);
        }
        if head & 1 << 30 != 0 {
            return Some(bytes);
        }
        at = next;
    }
    None
}

/// The channel whose centre lies `half_mhz` half megahertz above 2400 MHz: 2407 + 5 n MHz for
/// channels 1 to 13 and 2484 MHz for 14 (IEEE 802.11-2020, 15.4.4.3).
fn channel(half_mhz: u32) -> Option<u8> {
    if !half_mhz.is_multiple_of(2) {
        return None;
    }
    match half_mhz / 2 {
        84 => Some(14),
        mhz if mhz >= 12 && (mhz - 7) % 5 == 0 && (mhz - 7) / 5 <= 13 => {
            Some(((mhz - 7) / 5) as u8)
        }
        _ => None,
    }
}

/// The rate of `wifi_phy_rate_t` `code`, IDF v5.5.5,
/// `components/esp_wifi/include/esp_wifi_types_generic.h`. `None` for the reserved 4 and for a
/// short guard interval, whose airtime the model does not run.
fn rate(code: u32) -> Option<Rate> {
    Some(match code {
        0x00 => Rate::Dsss1,
        0x01 => Rate::Dsss2(Preamble::Long),
        0x02 => Rate::Cck5_5(Preamble::Long),
        0x03 => Rate::Cck11(Preamble::Long),
        0x05 => Rate::Dsss2(Preamble::Short),
        0x06 => Rate::Cck5_5(Preamble::Short),
        0x07 => Rate::Cck11(Preamble::Short),
        0x08 => Rate::Ofdm(Ofdm::M48),
        0x09 => Rate::Ofdm(Ofdm::M24),
        0x0a => Rate::Ofdm(Ofdm::M12),
        0x0b => Rate::Ofdm(Ofdm::M6),
        0x0c => Rate::Ofdm(Ofdm::M54),
        0x0d => Rate::Ofdm(Ofdm::M36),
        0x0e => Rate::Ofdm(Ofdm::M18),
        0x0f => Rate::Ofdm(Ofdm::M9),
        0x10..=0x17 => Rate::Ht(Mcs::new((code - 0x10) as u8)?),
        _ => return None,
    })
}

/// The first tick at or after `ps` picoseconds past `from`.
fn after(from: Tick, ps: u64) -> Tick {
    let ticks = (u128::from(ps) * u128::from(TICK_HZ)).div_ceil(1_000_000_000_000);
    Tick(from.0 + ticks as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s3::blocks;
    use crate::s3::rng;

    #[test]
    fn the_words_are_sorted_and_lie_in_the_radio_blocks() {
        assert!(WORDS.windows(2).all(|pair| pair[0] < pair[1]));
        for addr in WORDS {
            assert_eq!(addr % 4, 0, "{addr:#x}");
            let name = blocks::name(addr);
            let block = name.split('+').next().unwrap();
            match block {
                "FE2" | "FE" | "BT" | "NRX" | "BB" | "RADIO" => {}
                // Past `I2C_MST_ANA_CONF0_REG` to `ANA_CONFIG2_REG`, which `Analog` keeps.
                "I2C_MST" => assert!(addr > 0x6000_e048, "{name}"),
                "RESERVED" => assert!((0x6001_c000..0x6001_cc00).contains(&addr), "{name}"),
                _ => panic!("{name}"),
            }
            assert_ne!(addr, rng::WDEV_RND_REG);
        }
        for (addr, _) in STATUS {
            assert!(Radio::slot(addr).is_some(), "{addr:#x}");
        }
        assert!(Radio::slot(HAL_INIT).is_some());
    }

    fn read(radio: &mut Radio, addr: u32) -> Option<u32> {
        radio.read(Tick(0), addr)
    }

    fn write(radio: &mut Radio, addr: u32, value: u32) -> bool {
        radio.write(Tick(0), addr, value, |_| None) != Store::Unkept
    }

    #[test]
    fn a_word_reads_what_was_written_and_0_before() {
        let mut radio = Radio::new();
        assert_eq!(read(&mut radio, 0x6003_3000), Some(0));
        assert!(write(&mut radio, 0x6003_3000, 0x1234_5678));
        assert_eq!(read(&mut radio, 0x6003_3000), Some(0x1234_5678));
        assert_eq!(read(&mut radio, 0x6000_6000), Some(0));
    }

    #[test]
    fn a_word_the_radio_does_not_keep_is_left_to_its_block() {
        let mut radio = Radio::new();
        for addr in [
            0x6003_3ffc,
            0x6000_6ffc,
            0x6000_e000,
            0x6000_e040,
            rng::WDEV_RND_REG,
        ] {
            assert_eq!(read(&mut radio, addr), None, "{addr:#x}");
            assert!(!write(&mut radio, addr, 1), "{addr:#x}");
        }
    }

    #[test]
    fn each_polled_word_reads_its_done_bits() {
        let mut radio = Radio::new();
        assert_eq!(read(&mut radio, 0x6000_e050), Some(0x0700_0000));
        assert!(write(&mut radio, 0x6000_e050, 0x2));
        assert_eq!(read(&mut radio, 0x6000_e050), Some(0x0700_0002));
        assert_eq!(read(&mut radio, 0x6000_e04c), Some(0x0100_0000));
        assert_eq!(read(&mut radio, 0x6000_6174), Some(0x0001_0000));
    }

    #[test]
    fn hal_init_reads_done_once_it_has_set_start() {
        let mut radio = Radio::new();
        assert_eq!(read(&mut radio, HAL_INIT), Some(0));
        assert!(write(&mut radio, HAL_INIT, HAL_INIT_START));
        assert_eq!(
            read(&mut radio, HAL_INIT),
            Some(HAL_INIT_START | HAL_INIT_DONE)
        );
        assert!(write(&mut radio, HAL_INIT, 0));
        assert_eq!(read(&mut radio, HAL_INIT), Some(0));
    }

    #[test]
    fn the_queue_words_are_kept() {
        for addr in [PLCP0, PLCP1, TX_STATUS, EVENTS, EVENTS_CLEAR, EVENTS_ENABLE] {
            assert!(Radio::slot(addr).is_some(), "{addr:#x}");
        }
        for addr in [TX_DONE, TX_DONE_CLEAR, FREQ] {
            assert!(Radio::slot(addr).is_some(), "{addr:#x}");
        }
    }

    #[test]
    fn a_pll_start_tunes_the_channel_at_its_centre() {
        assert_eq!(channel(2 * 12), Some(1));
        assert_eq!(channel(2 * 37), Some(6));
        assert_eq!(channel(2 * 72), Some(13));
        assert_eq!(channel(2 * 84), Some(14));
        // Between channels 8 and 9, and half a megahertz off channel 1.
        assert_eq!(channel(2 * 50), None);
        assert_eq!(channel(2 * 12 + 1), None);
        assert_eq!(channel(2 * 7), None);
        assert_eq!(channel(2 * 77), None);

        let mut radio = Radio::new();
        assert!(write(&mut radio, FREQ, 2 * 12));
        assert_eq!(radio.tuned(), None, "the PLL has not started");
        assert!(write(&mut radio, FREQ, FREQ_START | (2 * 12)));
        assert_eq!(radio.tuned(), Some(1));
        assert!(write(&mut radio, FREQ, 2 * 37));
        assert_eq!(radio.tuned(), Some(1));
        assert!(write(&mut radio, FREQ, FREQ_START | (2 * 37)));
        assert_eq!(radio.tuned(), Some(6));
    }

    /// The first probe request the stock station sends in its scan, on channel 1, as its
    /// descriptor and buffer lay in SRAM: the MPDU, then 4 bytes for the FCS.
    const PROBE: [u8; 49] = [
        0x40, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02, 0x00, 0x00, 0x00, 0x00,
        0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x08, 0x8b, 0x96,
        0x82, 0x84, 0x0c, 0x18, 0x30, 0x60, 0x32, 0x04, 0x6c, 0x12, 0x24, 0x48, 0x03, 0x01, 0x01,
        0x00, 0x00, 0x00, 0x00,
    ];
    const DESCRIPTOR: u32 = 0x3fcb_3a58;
    const BUFFER: u32 = 0x3fcb_3a88;

    /// SRAM holding one descriptor at [`DESCRIPTOR`], with `length` and EOF, for `frame` at
    /// [`BUFFER`].
    fn sram(frame: &[u8], length: u32) -> impl Fn(u32) -> Option<u8> + '_ {
        let head = 1 << 31 | 1 << 30 | length << 12 | 0x50;
        let words = [head, BUFFER, 0];
        move |addr| {
            if (DESCRIPTOR..DESCRIPTOR + 12).contains(&addr) {
                let k = addr - DESCRIPTOR;
                return Some(words[(k / 4) as usize].to_le_bytes()[(k % 4) as usize]);
            }
            let k = addr.checked_sub(BUFFER)? as usize;
            frame.get(k).copied().or(Some(0))
        }
    }

    /// A radio tuned to channel 1 with the MAC's TX event enabled, and `plcp1` in queue 0.
    fn ready(plcp1: u32) -> Radio {
        let mut radio = Radio::new();
        assert!(write(&mut radio, EVENTS_ENABLE, 0x19a8_79e0));
        assert!(write(&mut radio, FREQ, FREQ_START | (2 * 12)));
        assert!(write(&mut radio, PLCP1, plcp1));
        assert!(write(
            &mut radio,
            PLCP0,
            0x0060_0000 | DESCRIPTOR & PLCP0_DESCRIPTOR
        ));
        radio
    }

    /// Starts queue 0 at `now` as `hal_mac_txq_enable` does.
    fn start(radio: &mut Radio, now: Tick, frame: &[u8], length: u32) -> Store {
        let plcp0 = 0xc060_0000 | DESCRIPTOR & PLCP0_DESCRIPTOR;
        radio.write(now, PLCP0, plcp0, sram(frame, length))
    }

    #[test]
    fn a_broadcast_probe_goes_on_the_air_and_is_done_at_its_end() {
        let mut radio = ready(0x31);
        let now = Tick(1_000);
        assert_eq!(start(&mut radio, now, &PROBE, 49), Store::Sent);
        // DIFS, then 192 us of long preamble and header and 392 us for 49 octets at 1 Mb/s.
        let begin = Tick(now.0 + 50 * 480);
        let end = Tick(begin.0 + (192 + 392) * 480);
        assert_eq!(radio.end(), Some(end));
        assert_eq!(radio.next_event(now), Some(end));
        assert!(!radio.line(Tick(end.0 - 1)));
        assert_eq!(radio.read(Tick(end.0 - 1), EVENTS), Some(0));

        radio.report(end);
        assert_eq!(radio.end(), None);
        let frame = radio.take_aired().expect("the frame");
        assert_eq!(radio.take_aired(), None);
        assert_eq!((frame.start, frame.end, frame.channel), (begin, end, 1));
        assert_eq!(frame.rate, Rate::Dsss1);
        assert_eq!(*frame.mpdu, PROBE[..45]);

        radio.acked(Ack::None);
        assert!(radio.line(end));
        assert_eq!(radio.next_event(end), None);
        assert_eq!(radio.read(end, EVENTS), Some(EVENT_TX_DONE));
        assert_eq!(radio.read(end, TX_DONE), Some(1));
        assert_eq!(radio.read(end, TX_STATUS), Some(TX_SUCCESS));
        // `hal_mac_clr_txq_state(2, 1)` and `hal_mac_interrupt_clr_event`.
        assert_eq!(radio.write(end, TX_DONE_CLEAR, 1, |_| None), Store::Kept);
        assert_eq!(radio.read(end, TX_DONE), Some(0));
        assert_eq!(radio.read(end, TX_DONE_CLEAR), Some(0));
        assert!(radio.line(end));
        radio.write(end, EVENTS_CLEAR, EVENT_TX_DONE, |_| None);
        assert!(!radio.line(end));

        // The queue is free again, once `mac_tx_set_plcp0` has written its word afresh.
        assert!(write(&mut radio, PLCP0, DESCRIPTOR & PLCP0_DESCRIPTOR));
        assert_eq!(start(&mut radio, end, &PROBE, 49), Store::Sent);
    }

    #[test]
    fn a_frame_to_one_station_is_done_after_its_ack_and_says_whether_it_came() {
        let mut unicast = PROBE;
        unicast[4..10].copy_from_slice(&[0x02, 0x6f, 0x72, 0x62, 0x65, 0x01]);
        for (ack, status) in [(Ack::Acked, TX_SUCCESS), (Ack::None, TX_ACK_TIMEOUT)] {
            let mut radio = ready(0x31);
            assert_eq!(start(&mut radio, Tick(0), &unicast, 49), Store::Sent);
            let end = radio.end().unwrap();
            radio.report(end);
            radio.acked(ack);
            // SIFS and a 304 us ACK at 1 Mb/s.
            let done = Tick(end.0 + (10 + 304) * 480);
            assert_eq!(radio.next_event(end), Some(done));
            assert!(!radio.line(Tick(done.0 - 1)));
            assert!(radio.line(done));
            assert_eq!(radio.read(done, TX_STATUS), Some(status), "{ack:?}");
        }
    }

    #[test]
    fn the_rate_comes_from_plcp1() {
        let mut radio = ready(0x0b << PLCP1_RATE_SHIFT | 0x31);
        assert_eq!(start(&mut radio, Tick(0), &PROBE, 49), Store::Sent);
        let end = radio.end().unwrap();
        radio.report(end);
        assert_eq!(radio.take_aired().unwrap().rate, Rate::Ofdm(Ofdm::M6));
        assert_eq!(rate(0x17), Some(Rate::Ht(Mcs::new(7).unwrap())));
        assert_eq!(rate(0x05), Some(Rate::Dsss2(Preamble::Short)));
    }

    #[test]
    fn a_frame_the_model_cannot_send_is_refused_by_name() {
        let refused = |radio: &mut Radio, length: u32| start(radio, Tick(0), &PROBE, length);
        let mut untuned = Radio::new();
        assert!(write(&mut untuned, PLCP1, 0x31));
        assert_eq!(refused(&mut untuned, 49), Store::Refused("tx_channel"));
        // The reserved code 4, and MCS 0 with the short guard interval.
        for code in [0x04, 0x18] {
            let mut radio = ready(code << PLCP1_RATE_SHIFT | 0x31);
            assert_eq!(refused(&mut radio, 49), Store::Refused("tx_rate"));
        }
        let mut radio = ready(0x31);
        assert_eq!(refused(&mut radio, 48), Store::Refused("tx_length"));
        let mut radio = ready(0x31);
        let outside = radio.write(Tick(0), PLCP0, 0xc060_0000 | DESCRIPTOR & 0xfffff, |_| None);
        assert_eq!(outside, Store::Refused("tx_descriptor"));
        let mut radio = ready(0x31);
        assert_eq!(refused(&mut radio, 49), Store::Sent);
        assert!(write(&mut radio, PLCP0, 0));
        assert_eq!(refused(&mut radio, 49), Store::Refused("tx_busy"));
    }
}
