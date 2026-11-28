//! The Wi-Fi radio's registers, which a chip built with a radio keeps, and the MAC's transmit
//! and receive paths.
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
//! The MAC receives a frame that one of its two address filter slots takes into the next
//! descriptor of its receive list, as `rx_ctrl`, the MPDU and the FCS, raises [`EVENT_RX`] and
//! acknowledges a frame to the station, as [`Radio::hear`] says. The TSF counts microseconds from
//! the chip's start. The meanings come from the driver's `wDev_ProcessRxSucData`,
//! `wdevProcessRxSucDataAll`, `wDev_AppendRxBlocks`, `hal_mac_set_addr`, `hal_mac_set_bssid` and
//! `hal_get_tsf_time`, and from esp32-open-mac's work on the ESP32's MAC, and each is a guess.
//!
//! The join after the scan touches more, from `hal_crypto_*`, `hal_tsf`, `hal_coex`,
//! `ic_update_sta_tsf`, `pm_update_next_tbtt`, `pm_set_beacon_filter`, `mac_tx_set_txop_q`,
//! `hal_mac_fill_hwtxop`, the PHY's sleep and wake and the ROM's `coex_hw_timer_*`, and each
//! meaning is a guess: the MAC encrypts and decrypts CCMP-128 with the key table, as [`keys`]
//! says, a TSF load sets a timer and a TBTT latch sets a TBTT's start ([`TSF_CONTROL`]), bit 31
//! of [`TBTT_CONTROL`] runs or freezes TSF timer 0, the MAC's clock counts microseconds
//! ([`MAC_CLOCK`]), an armed TBTT raises the power interrupt, source [`PWR_SOURCE`]
//! ([`Radio::tbtt`]) and sets bit 1 of [`TSF_ACTIVE`], the beacon filter drops a Beacon whose TIM
//! does not name the station ([`BEACON_FILTER`]), hardware TXOP chains extra frames from
//! [`TXOP_BASE`] on SIFS spacing (`TXOP_CTRL`), and the RF hears and sends only while
//! `RTC_CNTL_ANA_CONF_REG` powers it ([`ANA_CONF`]). What the model does not run is refused by
//! name: a cipher other than CCMP-128, a protected Management frame, a coexistence timer turned on
//! or forced (`coex_timer`, `coex_force`), external coexistence (`extern_coex`), the interrupt of
//! a TBTT other than 0 or of a TSF timer's alarm (`tbtt_intr`, `tsf_timer`), a Beacon the filter
//! cannot judge (`beacon_filter_tim`, `beacon_filter_group`), a frame sent with the RF powered
//! down, the RF powered down under a frame, `SAR_I2C_PU` cleared, or an unknown bit in
//! `RTC_CNTL_ANA_CONF_REG` (`tx_rf_off`, `rf_off_tx`, `sar_i2c_off`, `ana_conf`), any TXOP queue
//! above 2 (`txop_queue`), and any store to the MAC, key table, TSF, TBTT, power interrupt or
//! coexistence words that no blob function makes (`key_entry`, `key_valid`, `crypto_control`,
//! `crypto_init`, `tx_plcp1`, `txop_ctrl`, `events_enable`, `bssid_bits`, `beacon_filter`,
//! `beacon_filter_bits`, `tsf_control`, `tbtt_control`, `tbtt_start`, `tbtt_wake_ahead`,
//! `tsf_active`, `tsf_rtc_tick`, `pwr_intr`, `pwr_raw`, `pwr_events`, `coex_beacon_time`,
//! `coex_beacon_pti`, `coex_pti`, `coex_wait_time`, `coex_timer_bits`). The driver itself numbers
//! each frame and sets its Retry bit when it sends it again after an ACK timeout, so the MAC sends
//! the buffer as it is.
//!
//! Any other access in these blocks is refused or listed as on a chip with no radio.

use std::collections::VecDeque;
use std::sync::Arc;

use orbe_model::air::{Ack, Mcs, Ofdm, Preamble, Rate, SIFS_PS};

use super::clock::{TICK_HZ, Tick};
use super::keys::{self, Opened, Sealed};

/// The words kept, sorted.
const WORDS: [u32; 291] = [
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
    0x6003_308c,
    0x6003_3090,
    // `wdev_record_rx_linked_list` copies this word and the next, and `0x6003_3318` and
    // `0x6003_3320`, into the driver's debug record, which only `wdev_dump_rx_linked_list`
    // prints. Each reads 0 before a write, a guess.
    0x6003_3094,
    0x6003_3098,
    0x6003_309c,
    0x6003_30a0,
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
    0x6003_3318,
    0x6003_3320,
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
    0x6003_500c,
    0x6003_5010,
    0x6003_5014,
    0x6003_5018,
    0x6003_501c,
    0x6003_5020,
    0x6003_5024,
    0x6003_5028,
    0x6003_502c,
    0x6003_5030,
    0x6003_5058,
    0x6003_5080,
    0x6003_5084,
    0x6003_5088,
    0x6003_508c,
    0x6003_5090,
    0x6003_5094,
    0x6003_509c,
    0x6003_50a8,
    0x6003_50b0,
    0x6003_50b8,
    0x6003_50c0,
    0x6003_5110,
    0x6003_5114,
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
/// The bits of a PLCP1 word that `mac_tx_set_plcp1` sets beyond the length, the rate and the key
/// entry ([`keys::PLCP1_SHIFT`]): bit 25 with a rate from 16 to 31, an HT MCS, and bit 29 when
/// two flags of the driver's frame are set, which `mac_tx_set_htsig` clears again for an HT
/// frame. That bit 25 marks an HT rate, and that bit 29 changes nothing the model sends, are
/// guesses. A rate of 16 or more without bit 25 comes from a rate code above 31, which the model
/// does not run, `tx_rate`. No function sets bits 26 to 28, 30 or 31, or bit 25 with a rate below
/// 16, and a store that does is refused by name, `tx_plcp1`.
const PLCP1_HT: u32 = 1 << 25;
const PLCP1_BITS: u32 = 0x23ff_ffff;
const HT_RATES: u32 = 0x10;
/// Queue 0's status word, which `hal_mac_get_txq_pmd` reads. Bits 12 to 15 are the outcome that
/// `lmacProcessTxComplete` dispatches on: 0 runs `lmacProcessTxSuccess` and 5
/// `lmacProcessAckTimeout`. Bits 28 to 31 are the count of earlier TXOP frames that finished
/// before the final frame, as `lmacProcessTxopQComplete` (`libpp.a` `0x34`, offset `0x52`) reads
/// them. Every other bit reads 0, a guess.
const TX_STATUS: u32 = 0x6003_4320;
const TX_SUCCESS: u32 = 0;
const TX_ACK_TIMEOUT: u32 = 5 << 12;
const TX_STATUS_TXOP_SHIFT: u32 = 28;
/// Queue 0's TXOP control word (`0x6003_4314 - 0x4c q`), as `mac_tx_set_txop_q` (`libpp.a`
/// `0x1c`, ROM `0x4003_b924`) writes bits 24 to 27 with the count of extra frames (0 to 15) and
/// bits 28 and 29 with the hardware TXOP queue (0 to 2). `hal_attenna_init`, `mac_tx_set_plcp1`,
/// `mac_tx_set_plcp2` and `mac_tx_set_htsig` write bits 0 to 23. No blob function sets bits 30 or
/// 31, and a store that does is refused by name, `txop_ctrl`.
const TXOP_CTRL: u32 = 0x6003_4314;
const TXOP_COUNT_SHIFT: u32 = 24;
const TXOP_COUNT_MASK: u32 = 0xf;
const TXOP_QUEUE_SHIFT: u32 = 28;
const TXOP_QUEUE_MASK: u32 = 0x3;
const TXOP_CTRL_UNSET: u32 = 0xc000_0000;
/// The hardware TXOP table: 3 queues of 15 entries (`hal_mac_fill_hwtxop`, `libpp.a` `0x8`,
/// address `0x6003_437c + 60 * txop_q - 4 * (idx - 1)` for `txop_q` 0..3 and `idx` 1..=15),
/// spanning `0x6003_4344..=0x6003_43f4`. Bits 0 to 19 hold the DMA descriptor's SRAM offset and
/// bits 20 to 31 the PSDU length with FCS.
const TXOP_QUEUES: usize = 3;
const TXOP_SLOTS: usize = 15;
const TXOP_TABLE: usize = TXOP_QUEUES * TXOP_SLOTS;
const TXOP_BASE: u32 = 0x6003_4344;
const TXOP_FIRST: u32 = 0x6003_437c;
const TXOP_LENGTH_SHIFT: u32 = 20;

/// The TXOP table's word at `addr`, by its index in [`Radio::txop`].
fn txop_slot(addr: u32) -> Option<usize> {
    let at = addr.checked_sub(TXOP_BASE)? as usize / 4;
    (addr.is_multiple_of(4) && at < TXOP_TABLE).then_some(at)
}
/// The outcome of a protected frame whose key entry is not in use: outcome 4, which
/// `lmacProcessTxComplete` hands to `lmacProcessTxError` with bits 0 to 7, and 192 there, which
/// `lmacProcessTxError` hands to `lmacProcessTxseckiderr`. That the MAC reports it for an entry
/// not in use, sends nothing and is done when the frame would have started is a guess.
const TX_KEY_ERROR: u32 = 4 << 12 | 0xc0;
/// The MAC's event bits, which `hal_mac_interrupt_get_event` reads and `wDev_ProcessFiq`
/// dispatches on, the bits a store to [`EVENTS_CLEAR`] clears (W1C, reads 0;
/// `hal_mac_interrupt_clr_watchdog` RMWs it), and the word whose bits let an event raise the
/// interrupt. `hal_init` (`libpp.a` `0x5c`) writes `0` then `0x19a8_79e0`, `hal_deinit` (`0x20`)
/// clears those bits and writes `0`, `hal_enable_sta_beacon_filter` /
/// `hal_disable_sta_beacon_filter` toggle bit 15, and `hal_disable_mac` / `hal_enable_mac` /
/// `wdev_mac_sleep` / `wdev_mac_wakeup` save, zero and restore it. `hal_sniffer_rx_set_promis`
/// sets bit 2, which is not modelled. Any store whose non-modelled bits are neither 0 nor
/// `hal_init`'s fixed set is refused by name, `events_enable`.
const EVENTS: u32 = 0x6003_3c3c;
const EVENTS_CLEAR: u32 = 0x6003_3c40;
const EVENTS_ENABLE: u32 = 0x6003_3c34;
/// The event `wDev_ProcessFiq` answers with `lmacPostTxComplete`.
const EVENT_TX_DONE: u32 = 1 << 7;
const EVENTS_MODELLED: u32 = EVENT_TX_DONE | EVENT_BEACON | EVENT_RX;
const EVENTS_INIT_OTHER: u32 = 0x19a8_79e0 & !EVENTS_MODELLED;
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

/// The receive list. A store of an `lldesc_t`'s address to [`RX_BASE`] makes it the next one
/// the MAC fills, as `mac_rxbuf_init` and `hal_mac_rx_set_base` do. [`RX_NEXT`] reads the next
/// one's address bits 0 to 19, 0 once the list has run out, and [`RX_LAST`] those of the last
/// one filled, which `hal_mac_rx_get_last_dscr` puts above the SRAM bits of `0x6003_3c64`. A
/// store that sets bit 0 of [`RX_RELOAD`], as `hal_mac_rx_set_dscr_reload` does after
/// `wDev_AppendRxBlocks` has linked more onto the last one, makes the MAC read the last one's
/// next link again when the list has run out. The bit reads 0 once that is done, at once.
/// esp32-open-mac names the same four words at the ESP32's MAC base (`main/hardware.c`,
/// `WIFI_MAC_BITMASK_084`, `WIFI_BASE_RX_DSCR`, `WIFI_NEXT_RX_DSCR`, `WIFI_LAST_RX_DSCR`, commit
/// 20ce43d). That the S3 keeps them at the same offsets, the bits 0 to 19 and the reload's timing
/// are guesses from the driver's code.
const RX_RELOAD: u32 = 0x6003_3084;
const RX_RELOAD_START: u32 = 1 << 0;
const RX_BASE: u32 = 0x6003_3088;
const RX_NEXT: u32 = 0x6003_308c;
const RX_LAST: u32 = 0x6003_3090;
const RX_ADDRESS: u32 = 0xf_ffff;
/// The event `wDev_ProcessFiq` answers with `lmacProcessRxSucData`, one of the two it tests with
/// `0x401 << 14`. Bit 24 is also the receive bit esp32-open-mac tests on the ESP32
/// (`cause & 0x1000024`). That the S3's MAC sets bit 24 for a frame received, and not bit 14, is
/// a guess.
const EVENT_RX: u32 = 1 << 24;

/// The two address filter slots, `n` 0 and 1, as `hal_mac_set_addr`, `hal_mac_set_bssid` and
/// `hal_mac_rx_set_policy` write them: the receiver address at [`RA`] `+ 8 n`, its mask at
/// [`RA_MASK`] `+ 8 n` with the enable bit 16 in the second word, and the BSSID and its mask the
/// same way at [`BSSID`] and [`BSSID_MASK`]. Slot 0 is the station's interface and slot 1 the
/// access point's, as `wDev_ProcessRxSucData` reads the match bits. esp32-open-mac's
/// `esp32-wifi.yaml` and its write-up "RX filter" (2024-09-26) name the same words on the ESP32.
const RA: u32 = 0x6003_3040;
const RA_MASK: u32 = 0x6003_3060;
const BSSID: u32 = 0x6003_3000;
const BSSID_MASK: u32 = 0x6003_3020;
const SLOT_ENABLE: u32 = 1 << 16;
/// The slots `rx_ctrl` word 0 says a frame matched, bit 28 for slot 0, as `wDev_ProcessRxSucData`
/// tests them; esp32-open-mac's `filter_match`.
const FILTER_MATCH_SHIFT: u32 = 28;

/// The `rx_ctrl` header before each frame the MAC receives, `wifi_pkt_rx_ctrl_t` of IDF v5.5.5,
/// `components/esp_wifi/include/local/esp_wifi_types_native.h`, for the S3: 12 words. The MPDU
/// follows it, then its FCS, as `wDev_ProcessRxSucData` reads the frame at 48 bytes in.
const RX_CTRL: usize = 48;

/// The MAC's clock, which `hal_now` and `esp_wifi_internal_get_mac_clock_time` read: "32-bit
/// time value, unit: microsecond" (IDF v5.5.5, `components/esp_wifi/include/esp_private/wifi.h`).
/// `ic_update_sta_tsf` subtracts a frame's `rx_ctrl` timestamp from it, so the model reads the
/// same count, [`tsf`]'s low 32 bits.
const MAC_CLOCK: u32 = 0x6003_5000;

/// The TSF timers' control word, as the Arduino-ESP32 3.3.12 blobs set its bits, timer or TBTT
/// `n` from 0 to 3:
/// - Bit `n` copies timer `n`'s count into [`TSF_LOW`] and [`TSF_HIGH`]: `tsf_hal_get_counter_value`,
///   `hal_get_tsf_time`, `hal_get_sta_tsf` and `hal_mac_tsf_get_time` set it, read the two and
///   clear it.
/// - Bit `4 + n` loads timer `n` with the count in [`TSF_LOAD_LOW`] and [`TSF_LOAD_HIGH`], and the
///   timer counts on from it: `tsf_hal_set_counter_value` writes the count there and sets the
///   bit, `hal_set_sta_tsf` and `hal_mac_tsf_set_time` do so for timers 0 and 1, and
///   `hal_mac_tsf_reset` loads 0 into them.
/// - Bit `8 + n` latches the start in [`TBTT`] as TBTT `n`'s: `tsf_hal_set_tbtt_start_time` and
///   `hal_set_sta_tbtt` write the word and then set the bit.
///
/// No function clears a load or latch bit, so the model clears them at once and they read 0
/// after the store, a guess. No function sets bits 12 to 31, and a store that does is refused by
/// name, `tsf_control`.
const TSF_CONTROL: u32 = 0x6003_500c;
const TSF_LATCH: u32 = 0xf;
const TSF_LOAD_SHIFT: u32 = 4;
const TBTT_LATCH_SHIFT: u32 = 8;
const TSF_CONTROL_BITS: u32 = 0xfff;
/// The TSF timers, and the TBTTs, each one's own.
const TSF_TIMERS: usize = 4;
const TSF_LOAD_LOW: u32 = 0x6003_5010;
const TSF_LOAD_HIGH: u32 = 0x6003_5014;
const TSF_LOW: u32 = 0x6003_5018;
const TSF_HIGH: u32 = 0x6003_501c;

/// The coexistence timers' control words, timer `n`'s at [`COEX_TIMERS`] `+ 8 n`, as the ROM's
/// `coex_hw_timer_set`, `coex_hw_timer_enable` and `coex_hw_timer_disable` and libcoexist's
/// `coex_hw_timer_force` and `coex_hw_timer_unforce` write them. Bit 31 runs the timer to the
/// alarm `coex_hw_timer_set` writes into the next word from [`MAC_CLOCK`]. The alarm is not
/// modelled: a store that sets bit 31 is refused by name, `coex_timer`. Bit 28 forces the timer's
/// signal, which is not modelled either, `coex_force`. `coex_hw_timer_set` writes bits 0 to 23, a
/// 4-bit field in bits 24 to 27 and bit 30, which do nothing while the timer does not run. No
/// function sets bit 29, and a store that does is refused by name, `coex_timer_bits`.
const COEX_TIMERS: u32 = 0x6003_50a8;
const COEX_TIMER_COUNT: u32 = 4;
const COEX_TIMER_ON: u32 = 1 << 31;
const COEX_TIMER_FORCE: u32 = 1 << 28;
const COEX_TIMER_UNSET: u32 = 1 << 29;

/// Whether `mac_tx_set_plcp1` writes a PLCP1 word with `value`'s bits.
fn plcp1(value: u32) -> bool {
    let code = (value >> PLCP1_RATE_SHIFT) & PLCP1_RATE;
    value & !PLCP1_BITS == 0 && (value & PLCP1_HT == 0 || code >= HT_RATES)
}

/// Whether `addr` is a coexistence timer's control word.
fn coex_timer(addr: u32) -> bool {
    addr.checked_sub(COEX_TIMERS)
        .is_some_and(|at| at.is_multiple_of(8) && at / 8 < COEX_TIMER_COUNT)
}

/// The power interrupt, `ETS_WIFI_PWR_INTR_SOURCE` (2 in IDF v5.5.5,
/// `components/soc/esp32s3/include/soc/interrupts.h`), and its enable word. `tsf_hal_set_tbtt_intr_enable`
/// sets bit `4 - n` for TBTT `n`, and `hal_enable_sta_tbtt` bit 4 for TBTT 0.
/// `tsf_hal_set_timer_intr_enable` and `hal_enable_tsf_timer` set bit `8 - n` for timer `n`'s
/// alarm. Each first sets the same bit of [`PWR_CLEAR`]. While bit 4 is set and TBTT 0 is on
/// ([`TBTT_CONTROL`]), TBTT 0 sets bit 4 of [`PWR_EVENTS`] where [`Radio::tbtt`] reckons it, and
/// the interrupt is high while a bit is set there and here. A store that sets the bit of another
/// TBTT is refused by name, `tbtt_intr`, and one of a timer's alarm, whose target words the
/// model does not keep, `tsf_timer`. No function sets bit 0 or bits 9 to 31, and a store that
/// does is refused by name, `pwr_intr`.
pub(crate) const PWR_SOURCE: u32 = 2;
const PWR_ENABLE: u32 = 0x6003_5110;
const PWR_TBTT: u32 = 1 << 4;
const PWR_TBTT_OTHERS: u32 = 0b1110;
const PWR_TSF_TIMERS: u32 = 0xf << 5;
const PWR_BITS: u32 = 0x1fe;
/// The power interrupt's raw status (`0x6003_5114`, `pwr_hal_get_intr_raw_signal` in `libpp.a`
/// `0x4`), masked status (`0x6003_5118`, `pwr_hal_get_intr_status`, `hal_pwr_interrupt_get_event`
/// in `libpp.a` `0x4` and ROM `0x4004_06c8`, and `wDev_ProcessFiq` ROM `0x4004_08c2`), and
/// write-1-to-clear word (`0x6003_511c`, `hal_pwr_interrupt_clr_event` ROM `0x4004_06dc` and
/// `hal_enable_sta_tbtt` `libpp.a` `0xc` which RMWs it, so it reads 0). No function stores to
/// [`PWR_RAW`] or [`PWR_EVENTS`], and a store that does is refused by name, `pwr_raw` or
/// `pwr_events`.
const PWR_RAW: u32 = 0x6003_5114;
const PWR_EVENTS: u32 = 0x6003_5118;
const PWR_CLEAR: u32 = 0x6003_511c;
/// The start of a TBTT, bits 0 to 25 of its TSF count, which a latch bit of [`TSF_CONTROL`] makes
/// TBTT `n`'s. `tsf_hal_set_tbtt_start_time` keeps bits 26 to 31 as they read, and no function
/// sets them: a store that does is refused by name, `tbtt_start`.
const TBTT: u32 = 0x6003_5020;
const TBTT_FIELD: u32 = (1 << 26) - 1;
/// TBTT 0's wake-ahead time and RTC TSF update enable (`0x6003_5024`): bits 0 to 15 written by
/// `hal_set_sta_tbtt` (`libpp.a` `0x18`, `0x79-0x95`), `hal_set_sta_light_sleep_wake_ahead_time`
/// (`0x4`) and `tsf_hal_set_modem_wakeup_early_time` (`0x4`), and bit 25 written by
/// `hal_timer_update_by_rtc` (`libpp.a` `0x10`). No blob function sets bits 16 to 24 or 26 to 31,
/// and a store that does is refused by name, `tbtt_wake_ahead`.
const TBTT_WAKE_AHEAD: u32 = 0x6003_5024;
const TBTT_WAKE_AHEAD_BITS: u32 = 0x0200_ffff;
/// TBTT 0's control word, the first of the four TBTT and TSF groups 12 bytes apart
/// (`tsf_hal_set_tbtt_enable` and `tsf_hal_set_tsf_enable` pick `0x6003_5028`, `5034`, `5040` or
/// `504c`). The model keeps only TBTT 0's, whose bits the blobs set as:
/// - Bit 26 turns TBTT 0 on (`tsf_hal_set_tbtt_enable`, `hal_enable_sta_tbtt`).
/// - Bits 27, 28 and 31 turn TSF timer 0 on (`hal_enable_sta_tsf` `libpp.a` `0x4` and
///   `tsf_hal_set_tsf_enable` `0x10` OR `0x9800_0000`; `hal_disable_sta_tsf` `0x8` and
///   `tsf_hal_set_tsf_disable` `0x14` clear all three; `hal_mac_tsf_reset` `0x20` clears and
///   sets bit 31 around loading 0; `tsf_hal_is_tsf_enabled` `0x10` reads bit 31). Bit 31 runs or
///   freezes TSF timer 0 and gates TBTT 0. Bits 27 and 28 are always equal, and setting only one
///   is refused as `tbtt_control`.
/// - Bit 29 lets TBTT 0 wake the SoC (`tsf_hal_set_tbtt_soc_wakeup_enable`,
///   `hal_set_sta_tsf_wakeup` `libpp.a` `0x8`), which `pm_start` sets unconditionally; SoC light
///   sleep itself is refused at `RTC_CNTL_SLEEP_EN`.
///
/// No function sets bits 0 to 25 or bit 30 of this word, and a store that does is refused by
/// name, `tbtt_control`.
const TBTT_CONTROL: u32 = 0x6003_5028;
const TBTT_ON: u32 = 1 << 26;
const TSF0_MODE: u32 = 3 << 27;
const TBTT_SOC_WAKEUP: u32 = 1 << 29;
const TSF0_ON: u32 = 1 << 31;
const TBTT_CONTROL_BITS: u32 = TSF0_ON | TBTT_SOC_WAKEUP | TSF0_MODE | TBTT_ON;
/// Station TSF active status and W1C clear (`0x6003_502c`): `hal_is_sta_tsf_active` (`libpp.a`
/// `0x4`) reads bit 1, and `hal_clear_sta_tsf_active_state` (`libpp.a`, ROM `pm_tbtt_process`
/// `0x4003_c93e`, `0x4003_c9cc`) RMWs bit 0 to clear it. TBTT 0 sets bit 1 when it fires, bit 0
/// clears bit 1 and reads 0. Any store setting bits 2 to 31, or setting bit 1 when inactive, is
/// refused by name, `tsf_active`.
const TSF_ACTIVE: u32 = 0x6003_502c;
const TSF_ACTIVE_CLEAR: u32 = 1 << 0;
const TSF_ACTIVE_STATUS: u32 = 1 << 1;
/// TBTT 0's timing: bits 0 to 15 the beacon interval in TU, and bits 16 to 31 how many
/// microseconds before the TBTT the interrupt comes, as `hal_set_sta_tbtt`,
/// `hal_set_sta_tbtt_interval`, `hal_set_sta_tbtt_ahead_time`, `tsf_hal_set_tbtt_interval` and
/// `tsf_hal_set_tbtt_early_time` write them.
const TBTT_TIMING: u32 = 0x6003_5030;
/// RTC slow-clock tick interval for light-sleep TSF updates (`0x6003_5058`): bits 0 to 17 written
/// by `hal_timer_update_by_rtc` (`libpp.a` `0x10`, `0x28-0x49`). Bits 18 to 31 are never set and
/// refused as `tsf_rtc_tick`.
const TSF_RTC_TICK: u32 = 0x6003_5058;
const TSF_RTC_TICK_BITS: u32 = 0x0003_ffff;
/// Coexistence PTI and timing words (`0x6003_5080..=0x6003_509c`):
/// - `0x6003_5080`: bits 0 to 15 written by `hal_set_rx_beacon_time` (`libpp.a` `0x8`).
/// - `0x6003_5084`: bit 1 set by `hal_coex_pti_init` (`libpp.a` `0x4`); bit 0 is external
///   coexistence (`hal_enable_extern_coex` in `libcoexist.a` `0xc`), unmodelled and refused as
///   `extern_coex`; bits 2 to 31 never set, refused as `coex_pti`.
/// - `0x6003_5088`: bit 0 W1C strobe set by `hal_clear_rx_beacon_pti` (`libpp.a` `0x4`, ROM
///   `0x4003_c9e4`, `0x4003_cee9`), reads 0; bits 1 to 31 refused as `coex_beacon_pti`.
/// - `0x6003_508c`: bits 0 to 7 written by `hal_set_rx_beacon_time` (`libpp.a` `0x8`, `0x29-0x3d`).
/// - `0x6003_5090`: bits 0 to 3 written by `hal_set_rx_beacon_pti` (`libpp.a` `0xc`, `0x31-0x44`).
/// - `0x6003_5094`: bits 8 to 11 written by `hal_set_wifi_default_pti` (`libpp.a` `0x8`) and bits
///   12 to 15 by `hal_set_rx_beacon_pti` (`libpp.a` `0xc`).
/// - `0x6003_509c`: bits 16 to 31 written by `set_chan_freq_hw_init` (`libphy.a` `0x30`,
///   `0x4e-0x6b`) and `hal_coex_set_wait_time` (`libcoexist.a` `0x8`, `0x24-0x35`).
const COEX_BEACON_TIME: u32 = 0x6003_5080;
const COEX_PTI_CTRL: u32 = 0x6003_5084;
const COEX_EXTERN: u32 = 1 << 0;
const COEX_PTI_CTRL_BITS: u32 = 0x3;
const COEX_BEACON_PTI_CLEAR: u32 = 0x6003_5088;
const COEX_BEACON_TIME_EXT: u32 = 0x6003_508c;
const COEX_BEACON_PTI: u32 = 0x6003_5090;
const COEX_DEFAULT_PTI: u32 = 0x6003_5094;
const COEX_WAIT_TIME: u32 = 0x6003_509c;
/// A time unit, 1024 microseconds (IEEE 802.11-2020, 3.1).
const TU: u64 = 1024;

/// The station's beacon filter, which `hal_enable_sta_beacon_filter` turns on by setting bits 0
/// to 2, with [`EVENT_BEACON`] in [`EVENTS_ENABLE`], and `hal_disable_sta_beacon_filter` turns
/// off by clearing them. `pm_set_beacon_filter` first writes the station's AID into bits 16 to 26
/// of slot 0's BSSID word ([`BEACON_AID`], `hal_set_sta_beacon_filter`). While the filter is on, a
/// Beacon slot 0's BSSID takes whose TIM (IEEE 802.11-2020, 9.4.2.5) does not set that AID's bit
/// does not land: the MAC raises [`EVENT_BEACON`] on the next tick instead, which
/// `wdev_process_beacon_filter` posts to `pm_on_beacon_rx` with no frame. One that sets the bit
/// lands as any frame does. That the filter reads the TIM this way is a guess from those writes.
/// A Beacon with no TIM, or one whose TIM says group frames are held, which the filter might
/// pass or drop, ends the run by name, `beacon_filter_tim` or `beacon_filter_group`. A store that
/// sets only some of bits 0 to 2 ends it as `beacon_filter`, and one that sets bits 3 to 31 as
/// `beacon_filter_bits`, since no blob function does.
const BEACON_FILTER: u32 = 0x6003_30a0;
const BEACON_FILTER_ON: u32 = 0x7;
/// The event `wDev_ProcessFiq` answers with `wdev_process_beacon_filter`.
const EVENT_BEACON: u32 = 1 << 15;
/// The station's AID in slot 0's BSSID word (`0x6003_3004`), as `hal_set_sta_beacon_filter`
/// (`libpp.a` `0x8`, ROM `0x4003_c508`) writes bits 16 to 26 while `hal_mac_set_bssid` (`libpp.a`
/// `0x18`, `0x5f-0x85`) writes bits 0 to 15. No blob function sets bits 27 to 31, and a store
/// that does is refused by name, `bssid_bits`.
const BEACON_AID: u32 = BSSID + 4;
const BEACON_AID_SHIFT: u32 = 16;
const BEACON_AID_BITS: u32 = 0x7ff;
const BEACON_AID_UNSET: u32 = 0xf800_0000;

/// The key table: [`keys::ENTRIES`] entries of [`keys::WORDS`] words from [`keys::KEYS`], as
/// `hal_crypto_set_key_entry` writes entry `n` at `KEYS + 40 n` and `hal_crypto_clr_key_entry`
/// clears it, and the kept word [`keys::VALID`], whose bit `n` those two set and clear and
/// `hal_crypto_is_key_valid` reads. `wDev_remove_KeyEntry_all_cnx` walks entries 0 to 24 and
/// `hal_crypto_init` ORs 25 into `0x6003_3840`. The words read the last write, 0 before it, a
/// guess. [`keys`] lays them out, says how the MAC uses them, and names the stores the blobs
/// never make, which are refused ([`keys::refused`]).
const KEY_TABLE: usize = keys::ENTRIES * keys::WORDS;

/// The key table's word at `addr`, by its index in [`Radio::keys`].
fn key_slot(addr: u32) -> Option<usize> {
    let at = addr.checked_sub(keys::KEYS)? as usize / 4;
    (addr.is_multiple_of(4) && at < KEY_TABLE).then_some(at)
}

/// `rx_ctrl`'s `rx_state`, bits 24 to 31 of word 11, for a protected frame the MAC kept as it
/// came: no key entry takes it, or its MIC fails. `wDev_ProcessRxSucData` and `ppRxPkt` drop a
/// frame whose `rx_state` is neither 0 nor 245, and hand one with 245 to the TKIP Michael MIC
/// failure callback (`wpa_michael_mic_failure`, "TKIP MIC failure occur"). `wDev_RxSecError`
/// counts 225 to 230 and 240 to 252 as security errors. Which of them the MAC writes for these
/// two is a guess: two of the security errors that are not 245, so the driver drops the frame.
const RX_NO_KEY: u32 = 0xf0;
const RX_MIC_FAILURE: u32 = 0xf1;
const RX_STATE_SHIFT: u32 = 24;

/// The microseconds from the chip's start to `now`, the unit IEEE 802.11-2020 counts the TSF in
/// (11.1.3.1). Every TSF timer counts it until a load: the origin, and that the four timers
/// agree, are guesses. `rx_ctrl`'s `timestamp`, which IDF gives in microseconds, is its low 32
/// bits, whatever the loads, a guess.
fn tsf(now: Tick) -> u64 {
    now.0 / (TICK_HZ / 1_000_000)
}

/// When the frame starts after the store that starts its queue: DIFS, aSIFSTime (10 us) and two
/// of the long aSlotTime (20 us) of the DSSS and ERP PHYs (IEEE 802.11-2020, 10.3.2.3, 15.4.4
/// and 18.5.4). The MAC draws no backoff and senses no carrier, a guess.
const DIFS: Tick = Tick(50 * TICK_HZ / 1_000_000);

/// `RTC_CNTL_ANA_CONF_REG`, by its offset in RTC_CNTL, whose bits power the analog blocks
/// (IDF v5.5.5, `components/soc/esp32s3/register/soc/rtc_cntl_reg.h` and `rtc_cntl_struct.h`):
/// - Bit 31: `RTC_CNTL_PLL_I2C_PU`
/// - Bit 30: `RTC_CNTL_CKGEN_I2C_PU`
/// - Bit 29: `reserved29` in `rtc_cntl_struct.h`, set with bits 27, 28, 30 and 31 by
///   `rom_open_i2c_xpd` (ROM `0x4003_5e7c`, `libphy.a` `0x390`: `|= 0xf800_0000`) and left set by
///   `rom_phy_xpd_rf` (ROM `0x4003_8eac`, `libphy.a` `0x64`: `&= 0x27ff_ffff`, writing
///   `0x2040_0000` for modem sleep while wake writes `0xf840_0000`)
/// - Bit 28: `RTC_CNTL_RFRX_PBUS_PU`
/// - Bit 27: `RTC_CNTL_TXRF_I2C_PU`
/// - Bit 22: `RTC_CNTL_SAR_I2C_PU` (1 at boot, set by `tsens_read_init_new` and `adc_ll.h`)
///
/// The radio receives only while bits 31, 30, 29 and 28 are all set, and transmits only while
/// bits 31, 30, 29 and 27 are all set (`tx_rf_off`). Clearing any of bits 27 to 31 while a frame
/// is not done ends the run as `rf_off_tx`. Clearing bit 22 is refused as `sar_i2c_off`, and
/// setting any other bit as `ana_conf`.
pub(crate) const ANA_CONF: u32 = 0x34;
const PLL_I2C_PU: u32 = 1 << 31;
const CKGEN_I2C_PU: u32 = 1 << 30;
const RF_I2C_PU: u32 = 1 << 29;
const RFRX_PBUS_PU: u32 = 1 << 28;
const TXRF_I2C_PU: u32 = 1 << 27;
const SAR_I2C_PU: u32 = 1 << 22;
const RF_COMMON_PU: u32 = PLL_I2C_PU | CKGEN_I2C_PU | RF_I2C_PU;
const RF_RX_PU: u32 = RF_COMMON_PU | RFRX_PBUS_PU;
const RF_TX_PU: u32 = RF_COMMON_PU | TXRF_I2C_PU;
const RF_ALL_PU: u32 = RF_RX_PU | RF_TX_PU;
const ANA_CONF_BITS: u32 = RF_ALL_PU | SAR_I2C_PU;

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
    /// Additional frames queued for the current hardware TXOP burst, sent SIFS after each ACK.
    txop_rest: VecDeque<QueuedFrame>,
    /// The hardware TXOP table (`0x6003_4344..=0x6003_43f4`).
    txop: [u32; TXOP_TABLE],
    /// The bits of [`EVENTS`].
    events: u32,
    /// The bits of [`TX_DONE`].
    done: u32,
    /// Each queue's [`TX_STATUS`] word.
    status: [u32; QUEUES as usize],
    /// The address of the next receive descriptor, 0 once the list has run out.
    rx_next: u32,
    /// The address of the last receive descriptor filled, 0 before the first.
    rx_last: u32,
    /// When [`EVENT_RX`] rises for the frames received and not yet raised.
    rx_at: Option<Tick>,
    /// When [`EVENT_BEACON`] rises for the beacons the filter took and not yet raised.
    beacon_at: Option<Tick>,
    /// The TSF the last latch copied, which [`TSF_LOW`] and [`TSF_HIGH`] read.
    tsf_latched: u64,
    /// What each TSF timer adds to [`tsf`], from its last load.
    tsf_offset: [u64; TSF_TIMERS],
    /// Timer 0's frozen count while bit 31 ([`TSF0_ON`]) of [`TBTT_CONTROL`] is clear.
    tsf0_frozen: Option<u64>,
    /// Each TBTT's start, from its last latch.
    tbtt_start: [u32; TSF_TIMERS],
    /// The key table's words.
    keys: [u32; KEY_TABLE],
    /// While TBTT 0 and timer 0 are on, the tick its next TBTT counts from: the store that armed
    /// it, a later one that latched its start, set its timing or loaded timer 0, or the
    /// microsecond after the last TBTT it raised.
    tbtt_since: Option<Tick>,
    /// Station TSF active flag, bit 1 of [`TSF_ACTIVE`]: set on TBTT 0, cleared by bit 0.
    tsf_active: bool,
    /// The raw bits of [`PWR_RAW`], masked by [`PWR_ENABLE`] in [`PWR_EVENTS`].
    pwr: u32,
    /// `RTC_CNTL_ANA_CONF_REG` as last written ([`ANA_CONF`]).
    rf: u32,
}

/// A frame the radio hears on the channel it is tuned to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Heard<'a> {
    /// The first preamble symbol on the chip's timeline.
    pub(crate) start: Tick,
    /// Without the FCS.
    pub(crate) mpdu: &'a [u8],
    pub(crate) rate: Rate,
    pub(crate) rssi_dbm: i8,
    pub(crate) noise_dbm: i8,
}

/// What the radio did with a frame it heard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Received {
    /// No filter slot took it, or no descriptor was free for it.
    Missed,
    /// It wrote `bytes` at each address into SRAM, and acknowledges the frame when `ack`.
    Kept {
        writes: Vec<(u32, Vec<u8>)>,
        ack: bool,
    },
    /// It took the frame, which the model can't put in the list, by name.
    Refused(&'static str),
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

/// A follow-up frame prepared for a hardware TXOP burst.
#[derive(Clone, Debug)]
struct QueuedFrame {
    channel: u8,
    rate: Rate,
    /// `None` when a protected frame's key entry is not in use (`TX_KEY_ERROR`).
    mpdu: Option<Arc<[u8]>>,
    /// PSDU length with FCS.
    length: usize,
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
    /// Number of earlier frames in the TXOP burst already completed.
    completed: u32,
    /// The outcome the MAC reports whatever the ACK, for a frame it did not send.
    outcome: Option<u32>,
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
            txop_rest: VecDeque::new(),
            txop: [0; TXOP_TABLE],
            events: 0,
            done: 0,
            status: [0; QUEUES as usize],
            rx_next: 0,
            rx_last: 0,
            rx_at: None,
            beacon_at: None,
            tsf_latched: 0,
            tsf_offset: [0; TSF_TIMERS],
            tsf0_frozen: Some(0),
            tbtt_start: [0; TSF_TIMERS],
            keys: [0; KEY_TABLE],
            tbtt_since: None,
            tsf_active: false,
            pwr: 0,
            rf: 0,
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

    /// The word of TXOP queue `txop_q` (0..3) at 1-based slot `idx` (1..=15), matching
    /// `hal_mac_fill_hwtxop` (`libpp.a` `0x8`).
    fn txop_word(&self, txop_q: u32, idx: u32) -> u32 {
        let addr = TXOP_FIRST + 60 * txop_q - 4 * (idx - 1);
        txop_slot(addr).map_or(0, |at| self.txop[at])
    }

    /// The word at `addr`, a multiple of 4, at `now`, when the radio keeps it.
    pub(crate) fn read(&mut self, now: Tick, addr: u32) -> Option<u32> {
        if let Some(at) = key_slot(addr) {
            return Some(self.keys[at]);
        }
        if let Some(at) = txop_slot(addr) {
            return Some(self.txop[at]);
        }
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
            EVENTS_CLEAR => 0,
            TX_DONE => self.done,
            TX_DONE_CLEAR => 0,
            RX_RELOAD => word & !RX_RELOAD_START,
            RX_NEXT => self.rx_next & RX_ADDRESS,
            RX_LAST => self.rx_last & RX_ADDRESS,
            MAC_CLOCK => tsf(now) as u32,
            TSF_LOW => self.tsf_latched as u32,
            TSF_HIGH => (self.tsf_latched >> 32) as u32,
            TSF_ACTIVE => {
                if self.tsf_active {
                    TSF_ACTIVE_STATUS
                } else {
                    0
                }
            }
            COEX_BEACON_PTI_CLEAR => 0,
            PWR_RAW => self.pwr,
            PWR_EVENTS => self.pwr & self.word(PWR_ENABLE),
            PWR_CLEAR => 0,
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
        if let Some(at) = key_slot(addr) {
            self.keys[at] = value;
            return keys::refused(addr, value).map_or(Store::Kept, Store::Refused);
        }
        if let Some(at) = txop_slot(addr) {
            self.txop[at] = value;
            return Store::Kept;
        }
        let Some(slot) = Self::slot(addr) else {
            return Store::Unkept;
        };
        self.settle(now);
        let kept = std::mem::replace(&mut self.words[slot], value);
        if let Some(setting) = keys::refused(addr, value) {
            return Store::Refused(setting);
        }
        let starts = |bit: u32| value & bit != 0 && kept & bit == 0;
        match addr {
            EVENTS_CLEAR => self.events &= !value,
            EVENTS_ENABLE if !matches!(value & !EVENTS_MODELLED, 0 | EVENTS_INIT_OTHER) => {
                return Store::Refused("events_enable");
            }
            TX_DONE_CLEAR => self.done &= !value,
            FREQ if starts(FREQ_START) => self.channel = channel(value & FREQ_HALF_MHZ),
            RX_BASE => self.rx_next = value,
            RX_RELOAD if value & RX_RELOAD_START != 0 && self.rx_next == 0 && self.rx_last != 0 => {
                self.rx_next = read_word(self.rx_last + 8, &byte).unwrap_or(0);
            }
            BEACON_AID if value & BEACON_AID_UNSET != 0 => {
                return Store::Refused("bssid_bits");
            }
            BEACON_FILTER if value & !BEACON_FILTER_ON != 0 => {
                return Store::Refused("beacon_filter_bits");
            }
            BEACON_FILTER if value != 0 && value != BEACON_FILTER_ON => {
                return Store::Refused("beacon_filter");
            }
            _ if coex_timer(addr) && value & COEX_TIMER_UNSET != 0 => {
                return Store::Refused("coex_timer_bits");
            }
            _ if coex_timer(addr) && starts(COEX_TIMER_ON) => return Store::Refused("coex_timer"),
            _ if coex_timer(addr) && starts(COEX_TIMER_FORCE) => {
                return Store::Refused("coex_force");
            }
            _ if Self::queue(addr, PLCP1, STRIDE).is_some() && !plcp1(value) => {
                return Store::Refused("tx_plcp1");
            }
            _ if Self::queue(addr, TXOP_CTRL, STRIDE).is_some() && value & TXOP_CTRL_UNSET != 0 => {
                return Store::Refused("txop_ctrl");
            }
            COEX_BEACON_TIME if value & !0xffff != 0 => {
                return Store::Refused("coex_beacon_time");
            }
            COEX_PTI_CTRL if value & COEX_EXTERN != 0 => return Store::Refused("extern_coex"),
            COEX_PTI_CTRL if value & !COEX_PTI_CTRL_BITS != 0 => {
                return Store::Refused("coex_pti");
            }
            COEX_BEACON_PTI_CLEAR if value & !1 != 0 => {
                return Store::Refused("coex_beacon_pti");
            }
            COEX_BEACON_TIME_EXT if value & !0xff != 0 => {
                return Store::Refused("coex_beacon_time");
            }
            COEX_BEACON_PTI if value & !0xf != 0 => {
                return Store::Refused("coex_beacon_pti");
            }
            COEX_DEFAULT_PTI if value & !0xff00 != 0 => {
                return Store::Refused("coex_pti");
            }
            COEX_WAIT_TIME if value & !0xffff_0000 != 0 => {
                return Store::Refused("coex_wait_time");
            }
            TBTT_WAKE_AHEAD if value & !TBTT_WAKE_AHEAD_BITS != 0 => {
                return Store::Refused("tbtt_wake_ahead");
            }
            TSF_RTC_TICK if value & !TSF_RTC_TICK_BITS != 0 => {
                return Store::Refused("tsf_rtc_tick");
            }
            TSF_ACTIVE
                if value & !(TSF_ACTIVE_CLEAR | TSF_ACTIVE_STATUS) != 0
                    || (value & TSF_ACTIVE_STATUS != 0 && !self.tsf_active) =>
            {
                return Store::Refused("tsf_active");
            }
            TSF_ACTIVE => {
                if value & TSF_ACTIVE_CLEAR != 0 {
                    self.tsf_active = false;
                }
            }
            PWR_CLEAR => self.pwr &= !value,
            PWR_RAW => return Store::Refused("pwr_raw"),
            PWR_EVENTS => return Store::Refused("pwr_events"),
            PWR_ENABLE if value & !PWR_BITS != 0 => return Store::Refused("pwr_intr"),
            PWR_ENABLE if value & PWR_TBTT_OTHERS != 0 => return Store::Refused("tbtt_intr"),
            PWR_ENABLE if value & PWR_TSF_TIMERS != 0 => return Store::Refused("tsf_timer"),
            TBTT_CONTROL
                if value & !TBTT_CONTROL_BITS != 0
                    || !matches!(value & TSF0_MODE, 0 | TSF0_MODE) =>
            {
                return Store::Refused("tbtt_control");
            }
            TBTT_CONTROL => {
                let was_on = kept & TSF0_ON != 0;
                let is_on = value & TSF0_ON != 0;
                if was_on && !is_on {
                    self.tsf0_frozen = Some(self.timer(now, 0));
                } else if !was_on && is_on {
                    let frozen = self.tsf0_frozen.take().unwrap_or(0);
                    self.tsf_offset[0] = frozen.wrapping_sub(tsf(now));
                }
            }
            TBTT if value & !TBTT_FIELD != 0 => return Store::Refused("tbtt_start"),
            TSF_CONTROL if value & !TSF_CONTROL_BITS != 0 => return Store::Refused("tsf_control"),
            TSF_CONTROL => {
                let loaded = (u64::from(self.word(TSF_LOAD_HIGH)) << 32)
                    | u64::from(self.word(TSF_LOAD_LOW));
                for n in 0..TSF_TIMERS {
                    if value & 1 << (TSF_LOAD_SHIFT + n as u32) != 0 {
                        if n == 0 && self.tsf0_frozen.is_some() {
                            self.tsf0_frozen = Some(loaded);
                        } else {
                            self.tsf_offset[n] = loaded.wrapping_sub(tsf(now));
                        }
                    }
                    if value & 1 << (TBTT_LATCH_SHIFT + n as u32) != 0 {
                        self.tbtt_start[n] = self.word(TBTT);
                    }
                }
                self.words[slot] = value & TSF_LATCH;
                let latched = value & !kept & TSF_LATCH;
                if latched != 0 {
                    self.tsf_latched = self.timer(now, latched.trailing_zeros() as usize);
                }
            }
            _ => {}
        }
        // TBTT 0's settings: its start, its timing and timer 0.
        let retimed = addr == TBTT_TIMING
            || addr == TSF_CONTROL && value & (1 << TSF_LOAD_SHIFT | 1 << TBTT_LATCH_SHIFT) != 0;
        let on = self.word(TBTT_CONTROL) & (TBTT_ON | TSF0_ON) == (TBTT_ON | TSF0_ON);
        self.tbtt_since = match self.tbtt_since {
            _ if !on => None,
            Some(since) if !retimed => Some(since),
            _ => Some(now),
        };
        match Self::queue(addr, PLCP0, 8) {
            Some(queue) if starts(PLCP0_START) => match self.send(now, queue, value, byte) {
                Ok(true) => Store::Sent,
                Ok(false) => Store::Kept,
                Err(setting) => Store::Refused(setting),
            },
            _ => Store::Kept,
        }
    }

    /// Gathers a transmit descriptor chain from `descriptor`, checks its PSDU length `length`,
    /// and seals protected frames with `plcp1`'s key entry. Returns `Ok(None)` when the key entry
    /// is not in use (`TX_KEY_ERROR`).
    fn prepare_frame(
        &self,
        plcp1: u32,
        descriptor: u32,
        length: usize,
        byte: &impl Fn(u32) -> Option<u8>,
    ) -> Result<Option<Arc<[u8]>>, &'static str> {
        let psdu = gather(descriptor, byte).ok_or("tx_descriptor")?;
        // An MPDU holds at least Frame Control, Duration and Address 1 (9.3.1).
        if psdu.len() != length || length < FCS + 10 {
            return Err("tx_length");
        }
        let mut mpdu: Arc<[u8]> = Arc::from(&psdu[..length - FCS]);
        // The Protected Frame bit (9.2.4.1.9): the MAC encrypts the frame with the key entry
        // PLCP1 names.
        if mpdu[1] & PROTECTED != 0 {
            let entry = (plcp1 >> keys::PLCP1_SHIFT) & keys::PLCP1_ENTRY;
            match keys::seal(
                &self.keys,
                self.word(keys::VALID),
                |n| self.word(keys::CONTROL + 4 * n),
                entry,
                &mpdu,
            )? {
                Sealed::Mpdu(sealed) => mpdu = Arc::from(sealed),
                Sealed::NoKey => return Ok(None),
            }
        }
        Ok(Some(mpdu))
    }

    /// Puts queue `queue`'s frame (and any hardware TXOP follow-ups) on the air, from the store at
    /// `now` of `plcp0`, and whether it did: a protected frame without its key entry is done
    /// without going on the air.
    fn send(
        &mut self,
        now: Tick,
        queue: u32,
        plcp0: u32,
        byte: impl Fn(u32) -> Option<u8>,
    ) -> Result<bool, &'static str> {
        if self.airing.is_some() || self.pending.is_some() {
            return Err("tx_busy");
        }
        if self.rf & RF_TX_PU != RF_TX_PU {
            return Err("tx_rf_off");
        }
        let channel = self.channel.ok_or("tx_channel")?;
        let plcp1 = self.word(PLCP1 - STRIDE * queue);
        let code = (plcp1 >> PLCP1_RATE_SHIFT) & PLCP1_RATE;
        if (plcp1 & PLCP1_HT != 0) != (code >= HT_RATES) {
            return Err("tx_rate");
        }
        let rate = rate(code).ok_or("tx_rate")?;
        let txop_ctrl = self.word(TXOP_CTRL - STRIDE * queue);
        let extra = (txop_ctrl >> TXOP_COUNT_SHIFT) & TXOP_COUNT_MASK;
        let txop_q = (txop_ctrl >> TXOP_QUEUE_SHIFT) & TXOP_QUEUE_MASK;
        if extra > 0 && txop_q >= TXOP_QUEUES as u32 {
            return Err("txop_queue");
        }
        let length = (plcp1 & PLCP1_LENGTH) as usize;
        let first =
            self.prepare_frame(plcp1, SRAM_DATA | (plcp0 & PLCP0_DESCRIPTOR), length, &byte)?;
        let mut rest = VecDeque::with_capacity(extra as usize);
        for idx in 1..=extra {
            let word = self.txop_word(txop_q, idx);
            let desc = SRAM_DATA | (word & PLCP0_DESCRIPTOR);
            let len = ((word >> TXOP_LENGTH_SHIFT) & PLCP1_LENGTH) as usize;
            let mpdu = self.prepare_frame(plcp1, desc, len, &byte)?;
            rest.push_back(QueuedFrame {
                channel,
                rate,
                mpdu,
                length: len,
            });
        }
        let Some(mpdu) = first else {
            self.pending = Some(Pending {
                queue,
                at: Tick(now.0 + DIFS.0),
                waits: false,
                acked: false,
                completed: 0,
                outcome: Some(TX_KEY_ERROR),
            });
            return Ok(false);
        };
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
            completed: 0,
            outcome: None,
        });
        self.txop_rest = rest;
        Ok(true)
    }

    /// When TBTT 0 next reaches its wake point, regardless of [`PWR_ENABLE`].
    fn tbtt_due(&self) -> Option<Tick> {
        let armed = self.tbtt_since?;
        let timing = self.word(TBTT_TIMING);
        let interval = u64::from(timing & 0xffff) * TU;
        let ahead = u64::from(timing >> 16);
        let period = if interval == 0 {
            u64::from(TBTT_FIELD) + 1
        } else {
            interval
        };
        let offset = self.tsf_offset[0];
        let first = self.timer(armed, 0).wrapping_add(ahead);
        let target = u64::from(self.tbtt_start[0]) % period;
        let tbtt = first.wrapping_add((target + period - first % period) % period);
        let us = tbtt.wrapping_sub(ahead).wrapping_sub(offset);
        Some(Tick(us.saturating_mul(TICK_HZ / 1_000_000)).max(armed))
    }

    /// When TBTT 0 next raises the power interrupt: the first TBTT at least the ahead time after
    /// [`Radio::tbtt_since`], less that time, while its interrupt is enabled in [`PWR_ENABLE`].
    pub(crate) fn tbtt(&self) -> Option<Tick> {
        (self.word(PWR_ENABLE) & PWR_TBTT != 0)
            .then(|| self.tbtt_due())
            .flatten()
    }

    /// TSF timer `n`'s count at `now`.
    fn timer(&self, now: Tick, n: usize) -> u64 {
        if n == 0
            && let Some(frozen) = self.tsf0_frozen
        {
            return frozen;
        }
        let offset = self.tsf_offset.get(n).copied().unwrap_or(0);
        tsf(now).wrapping_add(offset)
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

    /// Whether a station acknowledged the frame that just ended. In a hardware TXOP burst,
    /// an acknowledged (or unacknowledged group) frame starts the next queued frame SIFS after
    /// the ACK; an ACK timeout stops the burst so `TX_STATUS` reports only the frames finished
    /// before the failed one.
    pub(crate) fn acked(&mut self, ack: Ack) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        let ok = !pending.waits || ack == Ack::Acked;
        pending.acked = ack == Ack::Acked;
        if !ok {
            self.txop_rest.clear();
            return;
        }
        let Some(next) = self.txop_rest.pop_front() else {
            return;
        };
        pending.completed += 1;
        let start = after(pending.at, SIFS_PS);
        match next.mpdu {
            Some(mpdu) => {
                let waits = mpdu[4] & 1 == 0;
                let end = after(start, next.rate.airtime(next.length));
                let done = if waits {
                    after(end, next.rate.acked_after())
                } else {
                    end
                };
                self.airing = Some(Frame {
                    start,
                    end,
                    channel: next.channel,
                    rate: next.rate,
                    mpdu,
                });
                pending.at = done;
                pending.waits = waits;
                pending.acked = false;
                pending.outcome = None;
            }
            None => {
                self.txop_rest.clear();
                pending.at = start;
                pending.waits = false;
                pending.acked = false;
                pending.outcome = Some(TX_KEY_ERROR);
            }
        }
    }

    /// The channel the radio is tuned to, while its RF receives ([`ANA_CONF`]).
    pub(crate) fn tuned(&self) -> Option<u8> {
        self.channel.filter(|_| self.rf & RF_RX_PU == RF_RX_PU)
    }

    /// The run stored `ana_conf` to `RTC_CNTL_ANA_CONF_REG` at `now`. Unmodelled bits, clearing
    /// `SAR_I2C_PU`, or powering down any RF bit while a frame is not done is refused by name
    /// ([`ANA_CONF`]).
    pub(crate) fn power(&mut self, now: Tick, ana_conf: u32) -> Option<&'static str> {
        self.settle(now);
        if ana_conf & !ANA_CONF_BITS != 0 {
            return Some("ana_conf");
        }
        if ana_conf & SAR_I2C_PU == 0 {
            return Some("sar_i2c_off");
        }
        let busy = self.airing.is_some() || self.pending.is_some();
        self.rf = ana_conf;
        (busy && ana_conf & RF_ALL_PU != RF_ALL_PU).then_some("rf_off_tx")
    }

    /// Hears `frame`, which ended at `now` on the channel the radio is tuned to. When a filter
    /// slot takes it and the next receive descriptor is free, the MAC writes its `rx_ctrl`, the
    /// MPDU and the FCS into that descriptor's buffer, hands the descriptor back and raises
    /// [`EVENT_RX`] on the next tick. It acknowledges a frame to one station that a slot's
    /// receiver address took, as esp32-open-mac's write-up says the RA filter does, and only when
    /// the frame found a descriptor (its `wifi_hardware_task`: "acking will only happen if the
    /// hardware puts the packet in an RX buffer"). A protected frame lands opened in place, or as
    /// it came with its `rx_state`, as [`keys::open`] says. `byte` reads SRAM on the data bus.
    pub(crate) fn hear(
        &mut self,
        now: Tick,
        frame: Heard<'_>,
        byte: impl Fn(u32) -> Option<u8>,
    ) -> Received {
        self.settle(now);
        let (slots, ack) = self.filter(frame.mpdu);
        if slots == 0 {
            return Received::Missed;
        }
        match self.beacon_filtered(frame.mpdu, slots) {
            Ok(true) => {
                self.beacon_at.get_or_insert(Tick(now.0 + 1));
                return Received::Missed;
            }
            Ok(false) => {}
            Err(setting) => return Received::Refused(setting),
        }
        // The Protected Frame bit: the MAC decrypts the frame with the key entry that takes it,
        // for the interface of the lowest slot that took it, or keeps it as it came.
        let mut opened = None;
        let mut rx_state = 0;
        if frame.mpdu[1] & PROTECTED != 0 {
            match keys::open(
                &self.keys,
                self.word(keys::VALID),
                |n| self.word(keys::CONTROL + 4 * n),
                slots.trailing_zeros(),
                frame.mpdu,
            ) {
                Ok(Opened::Mpdu(plain)) => opened = Some(plain),
                Ok(Opened::NoKey) => rx_state = RX_NO_KEY,
                Ok(Opened::Failed) => rx_state = RX_MIC_FAILURE,
                Err(setting) => return Received::Refused(setting),
            }
        }
        // A descriptor is free while the CPU has handed it to the MAC: bit 31 of its first word,
        // which `wDev_AppendRxBlocks` sets and the MAC clears, a guess.
        let descriptor = self.rx_next;
        let Some(head) = (descriptor != 0)
            .then(|| read_word(descriptor, &byte))
            .flatten()
            .filter(|head| head & OWNER != 0)
        else {
            return if descriptor == 0 || read_word(descriptor, &byte).is_some() {
                Received::Missed
            } else {
                Received::Refused("rx_descriptor")
            };
        };
        let (Some(buffer), Some(next)) = (
            read_word(descriptor + 4, &byte),
            read_word(descriptor + 8, &byte),
        ) else {
            return Received::Refused("rx_descriptor");
        };
        let psdu = frame.mpdu.len() + FCS;
        let length = RX_CTRL + psdu;
        // The frame fits one descriptor's buffer: `size`, bits 0 to 11. A frame across several
        // descriptors is not modelled.
        if length > (head & 0xfff) as usize {
            return Received::Refused("rx_length");
        }
        let Some(code) = rx_rate(frame.rate) else {
            return Received::Refused("rx_rate");
        };
        let mut bytes = Vec::with_capacity(length);
        let mut words = self.rx_ctrl(now, frame, code, slots, psdu);
        words[11] |= rx_state << RX_STATE_SHIFT;
        for word in words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(opened.as_deref().unwrap_or(frame.mpdu));
        bytes.extend_from_slice(&fcs(frame.mpdu).to_le_bytes());
        // `length`, bits 12 to 23, holds what the MAC wrote, the EOF bit 30 marks the descriptor
        // done, as `wdevProcessRxSucDataAll` tests it, and the owner bit 31 goes back to the CPU.
        let head = (head & !(OWNER | 0xfff << 12)) | (length as u32) << 12 | EOF;
        self.rx_last = descriptor;
        self.rx_next = next;
        self.rx_at.get_or_insert(Tick(now.0 + 1));
        Received::Kept {
            writes: vec![(buffer, bytes), (descriptor, head.to_le_bytes().to_vec())],
            ack,
        }
    }

    /// The slots that take a frame whose MPDU is `mpdu`, as bits, and whether the MAC
    /// acknowledges it. A slot's receiver address takes a frame whose Address 1 matches it under
    /// its mask. Its BSSID takes one whose Address 1 is the BSSID or broadcast and whose BSSID is
    /// the BSSID or broadcast (esp32-open-mac, "RX filter"). The BSSID is Address 3 of a frame
    /// with neither DS bit, Address 2 of one from the DS and Address 1 of one to it (IEEE
    /// 802.11-2020, 9.3.2.1). The MAC acknowledges a frame to one station that a slot's
    /// receiver address took (10.3.2.11). Which bits of `0x6003_30d8` let other frames through,
    /// and the case esp32-open-mac names for a frame from the DS when both filters run, are not
    /// modelled: a guess that the stock station needs neither.
    fn filter(&self, mpdu: &[u8]) -> (u32, bool) {
        // Frame Control, Duration and Address 1 (9.3.1).
        let Some(ra) = mpdu.get(4..10) else {
            return (0, false);
        };
        let address = |at: usize| mpdu.get(at..at + 6);
        let ds = mpdu[1] & 3;
        let bssid = match (mpdu[0] >> 2) & 3 {
            // A Management frame's (9.3.3.1) and a Data frame's.
            0 => address(16),
            2 => match ds {
                0 => address(16),
                1 => Some(ra),
                2 => address(10),
                _ => None,
            },
            _ => None,
        };
        let broadcast = [0xff; 6];
        let mut slots = 0;
        let mut ack = false;
        for slot in 0..2 {
            let ra_slot = self.mac(RA + 8 * slot, RA_MASK + 8 * slot);
            let bssid_slot = self.mac(BSSID + 8 * slot, BSSID_MASK + 8 * slot);
            let to_ra = ra_slot.is_some_and(|(mac, mask)| masked(ra, &mac, &mask));
            let in_bss = bssid_slot.is_some_and(|(mac, mask)| {
                (ra == broadcast || masked(ra, &mac, &mask))
                    && bssid.is_some_and(|b| b == broadcast || masked(b, &mac, &mask))
            });
            if to_ra || in_bss {
                slots |= 1 << slot;
            }
            ack |= to_ra && ra[0] & 1 == 0;
        }
        (slots, ack)
    }

    /// Whether the beacon filter takes `mpdu`, which the slots `slots` took: a Beacon that slot
    /// 0's BSSID takes while the filter is on, whose TIM does not set the bit of the AID in
    /// [`BEACON_AID`] ([`BEACON_FILTER`]).
    fn beacon_filtered(&self, mpdu: &[u8], slots: u32) -> Result<bool, &'static str> {
        // A Beacon (9.3.3.2): Frame Control's type 0 and subtype 8, from slot 0's BSS.
        let beacon = mpdu[0] == 0x80
            && slots & 1 != 0
            && self.mac(BSSID, BSSID_MASK).is_some_and(|(mac, mask)| {
                mpdu.get(16..22)
                    .is_some_and(|bssid| masked(bssid, &mac, &mask))
            });
        if !beacon || self.word(BEACON_FILTER) & BEACON_FILTER_ON == 0 {
            return Ok(false);
        }
        // The Timestamp, Beacon Interval and Capability Information, then the elements.
        let mut elements = mpdu.get(36..).unwrap_or(&[]);
        let tim = loop {
            match elements {
                [5, len, rest @ ..] if rest.len() >= usize::from(*len) && *len >= 4 => {
                    break &rest[..usize::from(*len)];
                }
                [_, len, rest @ ..] if rest.len() >= usize::from(*len) => {
                    elements = &rest[usize::from(*len)..];
                }
                _ => return Err("beacon_filter_tim"),
            }
        };
        // DTIM Count, DTIM Period, Bitmap Control and the Partial Virtual Bitmap, whose first
        // octet is octet N1 of the bitmap, with N1 / 2 in Bitmap Control's upper seven bits.
        let control = tim[2];
        if control & 1 != 0 {
            return Err("beacon_filter_group");
        }
        let aid = (self.word(BEACON_AID) >> BEACON_AID_SHIFT) & BEACON_AID_BITS;
        let octet = (aid / 8) as usize;
        let held = octet
            .checked_sub(usize::from(control & 0xfe))
            .and_then(|k| tim[3..].get(k))
            .is_some_and(|bits| bits & 1 << (aid % 8) != 0);
        Ok(!held)
    }

    /// The address and mask of the filter whose words start at `at` and `mask`, while its enable
    /// bit is set.
    fn mac(&self, at: u32, mask: u32) -> Option<([u8; 6], [u8; 6])> {
        let high = self.word(mask + 4);
        if high & SLOT_ENABLE == 0 {
            return None;
        }
        let six = |low: u32, high: u32| {
            let mut bytes = [0; 6];
            bytes[..4].copy_from_slice(&low.to_le_bytes());
            bytes[4..].copy_from_slice(&(high as u16).to_le_bytes());
            bytes
        };
        Some((
            six(self.word(at), self.word(at + 4)),
            six(self.word(mask), high),
        ))
    }

    /// The `rx_ctrl` words before a frame received at `now` whose PSDU, with the FCS, is `psdu`
    /// bytes. The MAC fills, as `wDev_ProcessRxSucData` and IDF's field names read them: the
    /// RSSI, the rate and the signal mode in word 0 with the slots that took it in bits 28 and
    /// 29; the MCS of an HT frame in word 1; the MAC clock in microseconds when the first bit of
    /// the frame body (the Timestamp field of a Beacon or Probe Response, IEEE 802.11-2020
    /// 9.4.1.10 and 11.1.3.2, which `ic_update_sta_tsf` adds directly to `MAC_CLOCK - timestamp`)
    /// arrived in word 3; the noise floor in word 5; the length with the FCS in word 11, with
    /// `rx_state` 0. The driver writes the channel into word 2 itself. Every other bit is 0.
    fn rx_ctrl(
        &self,
        now: Tick,
        frame: Heard<'_>,
        code: RxRate,
        slots: u32,
        psdu: usize,
    ) -> [u32; 12] {
        let mut words = [0; 12];
        words[0] = u32::from(frame.rssi_dbm as u8)
            | code.rate << 8
            | code.sig_mode << 14
            | slots << FILTER_MATCH_SHIFT;
        words[1] = code.mcs;
        words[3] = tsf(rx_timestamp(frame.start, now, frame.rate, frame.mpdu.len())) as u32;
        words[5] = u32::from(frame.noise_dbm as u8);
        words[11] = psdu as u32;
        words
    }

    /// Finishes the frame that is done by `now`, and raises the receive event and the TBTT that
    /// are due. A TBTT counts the next one from the microsecond after it.
    fn settle(&mut self, now: Tick) {
        while let Some(at) = self.tbtt_due().filter(|&at| at <= now) {
            self.pwr |= PWR_TBTT;
            self.tsf_active = true;
            self.tbtt_since = Some(Tick(at.0 + TICK_HZ / 1_000_000));
        }
        if self.rx_at.is_some_and(|at| at <= now) {
            self.rx_at = None;
            self.events |= EVENT_RX;
        }
        if self.beacon_at.is_some_and(|at| at <= now) {
            self.beacon_at = None;
            self.events |= EVENT_BEACON;
        }
        let Some(pending) = self.pending.filter(|pending| pending.at <= now) else {
            return;
        };
        self.pending = None;
        let outcome = match pending.outcome {
            Some(outcome) => outcome,
            None if pending.waits && !pending.acked => TX_ACK_TIMEOUT,
            None => TX_SUCCESS,
        };
        self.status[pending.queue as usize] = (pending.completed << TX_STATUS_TXOP_SHIFT) | outcome;
        self.done |= 1 << pending.queue;
        self.events |= EVENT_TX_DONE;
    }

    /// The MAC interrupt's level at `now`.
    pub(crate) fn line(&mut self, now: Tick) -> bool {
        self.settle(now);
        self.events & self.word(EVENTS_ENABLE) != 0
    }

    /// The power interrupt's level at `now`.
    pub(crate) fn pwr_line(&mut self, now: Tick) -> bool {
        self.settle(now);
        self.pwr & self.word(PWR_ENABLE) != 0
    }

    /// The first tick after `now` at which the MAC or the power interrupt can rise.
    pub(crate) fn next_event(&mut self, now: Tick) -> Option<Tick> {
        self.settle(now);
        [
            self.pending.map(|pending| pending.at),
            self.rx_at,
            self.beacon_at,
            self.tbtt(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

/// An `lldesc_t`'s owner bit, set while the MAC may fill it, and its EOF bit.
const OWNER: u32 = 1 << 31;
const EOF: u32 = 1 << 30;

/// The word at `at` in SRAM, little-endian.
fn read_word(at: u32, byte: impl Fn(u32) -> Option<u8>) -> Option<u32> {
    let bytes = [byte(at)?, byte(at + 1)?, byte(at + 2)?, byte(at + 3)?];
    Some(u32::from_le_bytes(bytes))
}

/// Whether `a` and `b` agree on every bit `mask` sets.
fn masked(a: &[u8], b: &[u8; 6], mask: &[u8; 6]) -> bool {
    a.iter()
        .zip(b)
        .zip(mask)
        .all(|((a, b), m)| (a ^ b) & m == 0)
}

/// The FCS: the CRC-32 of IEEE 802.11-2020, 9.2.4.8, sent low byte first.
fn fcs(mpdu: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in mpdu {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// How `rx_ctrl` names a frame's rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RxRate {
    /// The `wifi_phy_rate_t` of a non-HT frame, 0 for an HT one.
    rate: u32,
    /// 0 for non-HT, 1 for HT.
    sig_mode: u32,
    mcs: u32,
}

/// The `rx_ctrl` rate fields of a frame at `rate`: the code [`rate`] reads from a PLCP1 word, a
/// guess that the MAC writes the same codes. `None` for a rate [`rate`] does not run.
fn rx_rate(rate: Rate) -> Option<RxRate> {
    if let Rate::Ht(mcs) = rate {
        return Some(RxRate {
            rate: 0,
            sig_mode: 1,
            mcs: u32::from(mcs.index()),
        });
    }
    let code = (0..0x10).find(|&code| self::rate(code) == Some(rate))?;
    Some(RxRate {
        rate: code,
        sig_mode: 0,
        mcs: 0,
    })
}

/// The Protected Frame bit of Frame Control's second octet (IEEE 802.11-2020, 9.2.4.1.9).
const PROTECTED: u8 = 0x40;

/// The FCS after the MPDU.
const FCS: usize = 4;
/// The most descriptors a frame's chain may have before the model calls it a loop.
const DESCRIPTORS: usize = 64;

/// The bytes of the chain of `lldesc_t` from `first`: `size` in bits 0 to 11 of the first word,
/// `length` in bits 12 to 23 and `eof` in bit 30, then the buffer's address and the next
/// descriptor's. IDF v5.5.5, `components/esp_rom/include/esp_rom_lldesc.h`. `None` when a word or
/// a byte lies outside SRAM, or the chain does not end.
fn gather(first: u32, byte: impl Fn(u32) -> Option<u8>) -> Option<Vec<u8>> {
    let word = |at: u32| read_word(at, &byte);
    let mut bytes = Vec::new();
    let mut at = first;
    for _ in 0..DESCRIPTORS {
        let (head, buffer, next) = (word(at)?, word(at + 4)?, word(at + 8)?);
        let length = (head >> 12) & 0xfff;
        for k in 0..length {
            bytes.push(byte(buffer + k)?);
        }
        if head & EOF != 0 {
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

/// The tick when the first bit of the frame body (the Timestamp field of a Beacon or Probe
/// Response, after the PLCP and 24-octet MAC header) arrives on the air (IEEE 802.11-2020
/// 9.4.1.10 and 11.1.3.2), clamped to `end` for frames shorter than 24 octets.
fn rx_timestamp(start: Tick, end: Tick, rate: Rate, mpdu_len: usize) -> Tick {
    let bits = 8 * mpdu_len.min(24) as u64;
    let preamble = |p: Preamble| match p {
        Preamble::Long => 192,
        Preamble::Short => 96,
    };
    let us = match rate {
        Rate::Dsss1 => 192 + bits,
        Rate::Dsss2(p) => preamble(p) + bits.div_ceil(2),
        Rate::Cck5_5(p) => preamble(p) + (2 * bits).div_ceil(11),
        Rate::Cck11(p) => preamble(p) + bits.div_ceil(11),
        Rate::Ofdm(r) => {
            let dbps = match r {
                Ofdm::M6 => 24,
                Ofdm::M9 => 36,
                Ofdm::M12 => 48,
                Ofdm::M18 => 72,
                Ofdm::M24 => 96,
                Ofdm::M36 => 144,
                Ofdm::M48 => 192,
                Ofdm::M54 => 216,
            };
            20 + 4 * (16 + bits).div_ceil(dbps)
        }
        Rate::Ht(m) => {
            let dbps = match m.index() {
                0 => 26,
                1 => 52,
                2 => 78,
                3 => 104,
                4 => 156,
                5 => 208,
                6 => 234,
                _ => 260,
            };
            36 + 4 * (16 + bits).div_ceil(dbps)
        }
    };
    Tick(start.0 + us * (TICK_HZ / 1_000_000)).min(end)
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

    /// `RTC_CNTL_ANA_CONF_REG` as `rom_open_i2c_xpd` leaves it, with the RF powered.
    const RF_ON: u32 = 0xf840_0000;

    /// A radio whose RF is powered, as the PHY's start leaves it.
    fn powered() -> Radio {
        let mut radio = Radio::new();
        assert_eq!(radio.power(Tick(0), RF_ON), None);
        radio
    }

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
        let mut radio = powered();
        assert_eq!(read(&mut radio, 0x6003_3000), Some(0));
        assert!(write(&mut radio, 0x6003_3000, 0x1234_5678));
        assert_eq!(read(&mut radio, 0x6003_3000), Some(0x1234_5678));
        assert_eq!(read(&mut radio, 0x6000_6000), Some(0));
    }

    #[test]
    fn a_word_the_radio_does_not_keep_is_left_to_its_block() {
        let mut radio = powered();
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
        let mut radio = powered();
        assert_eq!(read(&mut radio, 0x6000_e050), Some(0x0700_0000));
        assert!(write(&mut radio, 0x6000_e050, 0x2));
        assert_eq!(read(&mut radio, 0x6000_e050), Some(0x0700_0002));
        assert_eq!(read(&mut radio, 0x6000_e04c), Some(0x0100_0000));
        assert_eq!(read(&mut radio, 0x6000_6174), Some(0x0001_0000));
    }

    #[test]
    fn hal_init_reads_done_once_it_has_set_start() {
        let mut radio = powered();
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
        for addr in [
            RX_RELOAD,
            RX_BASE,
            RX_NEXT,
            RX_LAST,
            TSF_CONTROL,
            TSF_LOW,
            TSF_HIGH,
        ] {
            assert!(Radio::slot(addr).is_some(), "{addr:#x}");
        }
        for slot in 0..2 {
            for addr in [RA, RA_MASK, BSSID, BSSID_MASK] {
                for word in [addr + 8 * slot, addr + 8 * slot + 4] {
                    // Slot 1's BSSID, which the stock station never sets, reads 0.
                    if slot == 0 || addr == RA || addr == RA_MASK {
                        assert!(Radio::slot(word).is_some(), "{word:#x}");
                    }
                }
            }
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

        let mut radio = powered();
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
        let mut radio = powered();
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
        let mut untuned = powered();
        assert!(write(&mut untuned, PLCP1, 0x31));
        assert_eq!(refused(&mut untuned, 49), Store::Refused("tx_channel"));
        // The reserved code 4, MCS 0 with the short guard interval, and MCS 0 without bit 25.
        for (code, ht) in [(0x04, 0), (0x18, PLCP1_HT), (0x10, 0)] {
            let mut radio = ready(code << PLCP1_RATE_SHIFT | ht | 0x31);
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

    /// SRAM as a map of the bytes written, for the receive path.
    #[derive(Default)]
    struct Ram(std::collections::BTreeMap<u32, u8>);

    impl Ram {
        fn poke(&mut self, at: u32, bytes: &[u8]) {
            for (k, &byte) in bytes.iter().enumerate() {
                self.0.insert(at + k as u32, byte);
            }
        }

        fn word(&self, at: u32) -> u32 {
            read_word(at, |at| self.0.get(&at).copied()).unwrap()
        }

        fn bytes(&self, at: u32, len: usize) -> Vec<u8> {
            (0..len as u32).map(|k| self.0[&(at + k)]).collect()
        }

        /// One free descriptor at `at` with a buffer of `size` bytes at `buffer`, linked to
        /// `next`, as `wDev_AppendRxBlocks` leaves it: the owner bit, `length` = `size`.
        fn descriptor(&mut self, at: u32, buffer: u32, size: u32, next: u32) {
            let head = OWNER | size << 12 | size;
            for (k, word) in [head, buffer, next].into_iter().enumerate() {
                self.poke(at + 4 * k as u32, &word.to_le_bytes());
            }
            self.poke(buffer, &vec![0; size as usize]);
        }

        fn apply(&mut self, received: Received) -> bool {
            let Received::Kept { writes, ack } = received else {
                panic!("{received:?}");
            };
            for (at, bytes) in writes {
                self.poke(at, &bytes);
            }
            ack
        }
    }

    /// The station's MAC, the access point's, and two descriptors in a list.
    const STA: [u8; 6] = [0x02, 0, 0, 0, 0, 0x01];
    const AP: [u8; 6] = [0x02, 0x6f, 0x72, 0x62, 0x65, 0x01];
    const RX0: u32 = 0x3fca_ffb0;
    const RX1: u32 = 0x3fca_ffbc;
    const BUF0: u32 = 0x3fcb_0000;
    const BUF1: u32 = 0x3fcb_1000;
    const SIZE: u32 = 1600;

    /// A Management frame of `subtype` from the access point to `ra`, with the access point as
    /// BSSID and a short body.
    fn management(subtype: u8, ra: [u8; 6]) -> Vec<u8> {
        let mut mpdu = vec![subtype << 4, 0, 0, 0];
        mpdu.extend(ra);
        mpdu.extend(AP);
        mpdu.extend(AP);
        mpdu.extend([0x10, 0x00, 0, 1, 2, 3]);
        mpdu
    }

    fn heard(mpdu: &[u8]) -> Heard<'_> {
        Heard {
            start: Tick(0),
            mpdu,
            rate: Rate::Dsss1,
            rssi_dbm: -50,
            noise_dbm: -95,
        }
    }

    /// Writes `mac` and an all-ones mask, enabled, into the filter at `at` and `mask`, as
    /// `hal_mac_set_addr` and `hal_mac_set_bssid` do.
    fn filter(radio: &mut Radio, at: u32, mask: u32, mac: [u8; 6]) {
        assert!(write(
            radio,
            at,
            u32::from_le_bytes(mac[..4].try_into().unwrap())
        ));
        assert!(write(
            radio,
            at + 4,
            u32::from(u16::from_le_bytes([mac[4], mac[5]]))
        ));
        assert!(write(radio, mask, !0));
        assert!(write(radio, mask + 4, SLOT_ENABLE | 0xffff));
    }

    /// A radio scanning as the stock station does: slot 0's receiver address and BSSID are both
    /// the station's own MAC, and the list holds two free descriptors.
    fn listening(ram: &mut Ram) -> Radio {
        let mut radio = powered();
        assert!(write(&mut radio, EVENTS_ENABLE, 0x19a8_79e0));
        filter(&mut radio, RA, RA_MASK, STA);
        filter(&mut radio, BSSID, BSSID_MASK, STA);
        ram.descriptor(RX0, BUF0, SIZE, RX1);
        ram.descriptor(RX1, BUF1, SIZE, 0);
        assert!(write(&mut radio, RX_BASE, RX0));
        radio
    }

    #[test]
    fn the_fcs_is_the_crc_32_of_the_mpdu() {
        // The CRC-32 check value (ITU-T V.42, IEEE 802.3), on "123456789".
        assert_eq!(fcs(b"123456789"), 0xcbf4_3926);
    }

    /// A Probe Response to the station lands in the first descriptor as `rx_ctrl`, the MPDU and
    /// the FCS. The descriptor is done and the CPU's, the list moves on, the MAC acknowledges the
    /// frame, and the receive event rises a tick later.
    #[test]
    fn a_frame_to_the_station_lands_in_the_next_descriptor() {
        let mut ram = Ram::default();
        let mut radio = listening(&mut ram);
        let mpdu = management(5, STA);
        let start = Tick(480_000 * 579);
        let now = after(start, Rate::Dsss1.airtime(mpdu.len() + 4));
        let mut frame = heard(&mpdu);
        frame.start = start;
        let ack = ram.apply(radio.hear(now, frame, |at| ram.0.get(&at).copied()));
        assert!(ack);

        let length = RX_CTRL as u32 + mpdu.len() as u32 + 4;
        assert_eq!(ram.word(RX0), EOF | length << 12 | SIZE);
        let words: Vec<u32> = (0..12).map(|k| ram.word(BUF0 + 4 * k)).collect();
        // RSSI -50, rate 0 (1 Mb/s), non-HT, slot 0 matched.
        assert_eq!(words[0], 0x1000_00ce);
        assert_eq!(
            words[3],
            579_000 + 384,
            "first bit of Timestamp field (192 us PLCP + 24 * 8 us header)"
        );
        assert_eq!(words[5], 0xa1, "a noise floor of -95 dBm");
        assert_eq!(words[11], mpdu.len() as u32 + 4);
        for k in [1, 2, 4, 6, 7, 8, 9, 10] {
            assert_eq!(words[k], 0, "word {k}");
        }
        let at = BUF0 + RX_CTRL as u32;
        assert_eq!(ram.bytes(at, mpdu.len()), mpdu);
        let fcs = u32::from_le_bytes(ram.bytes(at + mpdu.len() as u32, 4).try_into().unwrap());
        assert_eq!(fcs, super::fcs(&mpdu));

        assert_eq!(radio.read(now, RX_LAST), Some(RX0 & RX_ADDRESS));
        assert_eq!(radio.read(now, RX_NEXT), Some(RX1 & RX_ADDRESS));
        assert_eq!(radio.read(now, EVENTS), Some(0));
        assert!(!radio.line(now));
        let next = Tick(now.0 + 1);
        assert_eq!(radio.next_event(now), Some(next));
        assert!(radio.line(next));
        assert_eq!(radio.read(next, EVENTS), Some(EVENT_RX));
        radio.write(next, EVENTS_CLEAR, EVENT_RX, |_| None);
        assert_eq!(radio.read(next, EVENTS_CLEAR), Some(0));
        assert!(!radio.line(next));
    }

    /// `ic_update_sta_tsf` (`libpp.a` `0x8-0x28`) loads TSF timer 0 with
    /// `frame_tsf + (MAC_CLOCK - rx_ctrl.timestamp)`. Because `orbe-net` writes the AP's TSF at
    /// `start + 384 us` (first bit of the Timestamp field at 1 Mb/s, IEEE 802.11-2020 9.4.1.10)
    /// and `rx_ctrl.timestamp` stamps the same point, timer 0 matches the AP's TSF with zero lag
    /// and TBTT fires exactly 3 000 us before the next beacon.
    #[test]
    fn tbtt_fires_with_the_programmed_3000_us_lead_after_probe_response_tsf_sync() {
        let at = |us: u64| Tick(480 * us);
        let mut ram = Ram::default();
        let mut radio = listening(&mut ram);
        assert_eq!(
            radio.write(at(0), TBTT_CONTROL, TSF0_ON | TSF0_MODE, |_| None),
            Store::Kept
        );
        // A 86-octet Probe Response MPDU (90 octets with FCS, 912 us airtime at 1 Mb/s).
        let mut mpdu = management(5, STA);
        mpdu.resize(86, 0);
        let start_us = 3_180_705u64;
        let frame_tsf = start_us + 192 + 24 * 8;
        let start = at(start_us);
        let end = after(start, Rate::Dsss1.airtime(mpdu.len() + 4));
        let mut frame = heard(&mpdu);
        frame.start = start;
        assert!(ram.apply(radio.hear(end, frame, |a| ram.0.get(&a).copied())));
        let rx_ts = u64::from(ram.word(BUF0 + 12));
        assert_eq!(rx_ts, start_us + 384);

        // `ic_update_sta_tsf` runs later at `sync_us`: `frame_tsf + (MAC_CLOCK - rx_ts)`.
        let sync_us = 3_181_617u64;
        let mac_clock = u64::from(radio.read(at(sync_us), MAC_CLOCK).unwrap());
        let loaded_tsf = frame_tsf + mac_clock.wrapping_sub(rx_ts);
        assert_eq!(loaded_tsf, sync_us, "zero lag against AP TSF");
        assert!(write(&mut radio, TSF_LOAD_LOW, loaded_tsf as u32));
        assert!(write(&mut radio, TSF_LOAD_HIGH, (loaded_tsf >> 32) as u32));
        assert_eq!(
            radio.write(at(sync_us), TSF_CONTROL, 1 << 4, |_| None),
            Store::Kept
        );

        // Arm TBTT 0 with 100 TU interval, start at 3 072 000 us, 3 000 us ahead.
        let set = at(3_207_713);
        assert_eq!(
            radio.write(set, PWR_ENABLE, PWR_TBTT, |_| None),
            Store::Kept
        );
        assert_eq!(
            radio.write(set, TBTT_CONTROL, 0xbc00_0000, |_| None),
            Store::Kept
        );
        assert_eq!(radio.write(set, TBTT, 3_072_000, |_| None), Store::Kept);
        assert_eq!(radio.write(set, TSF_CONTROL, 1 << 8, |_| None), Store::Kept);
        assert_eq!(
            radio.write(set, TBTT_TIMING, (3_000 << 16) | 100, |_| None),
            Store::Kept
        );
        // Next AP beacon is at 3 072 000 + 2 * 102 400 = 3 276 800 us.
        let next_beacon_us = 3_276_800u64;
        assert_eq!(radio.tbtt(), Some(at(next_beacon_us - 3_000)));
    }

    /// A Beacon is to everyone, and its BSSID is the access point's, not the station's own MAC
    /// that the scan sets as slot 0's BSSID, so no slot takes it. Once slot 0's BSSID is the
    /// access point's, it does, unacknowledged.
    #[test]
    fn a_beacon_lands_only_once_a_slot_has_its_bssid() {
        let mut ram = Ram::default();
        let mut radio = listening(&mut ram);
        let beacon = management(8, [0xff; 6]);
        let now = Tick(1_000);
        let byte = |ram: &Ram| {
            let map = ram.0.clone();
            move |at| map.get(&at).copied()
        };
        assert_eq!(
            radio.hear(now, heard(&beacon), byte(&ram)),
            Received::Missed
        );
        filter(&mut radio, BSSID, BSSID_MASK, AP);
        assert!(!ram.apply(radio.hear(now, heard(&beacon), byte(&ram))));
        assert_eq!(ram.word(BUF0) >> FILTER_MATCH_SHIFT, 1);
        // A frame to another station is taken by no slot.
        let other = management(5, [0x02, 0, 0, 0, 0, 0x02]);
        let mut radio = listening(&mut ram);
        assert_eq!(radio.hear(now, heard(&other), byte(&ram)), Received::Missed);
        // Nor is one to the station while slot 0 is off.
        let mut radio = listening(&mut ram);
        assert!(write(&mut radio, RA_MASK + 4, 0xffff));
        assert!(write(&mut radio, BSSID_MASK + 4, 0xffff));
        let probe = management(5, STA);
        assert_eq!(radio.hear(now, heard(&probe), byte(&ram)), Received::Missed);
    }

    /// The list runs out after its last descriptor, and a frame then is lost, unacknowledged.
    /// Once `wDev_AppendRxBlocks` links a descriptor onto the last one and sets the reload bit,
    /// the MAC fills it.
    #[test]
    fn the_list_runs_out_until_a_reload() {
        let mut ram = Ram::default();
        let mut radio = listening(&mut ram);
        let probe = management(5, STA);
        let now = Tick(1_000);
        for _ in 0..2 {
            let map = ram.0.clone();
            ram.apply(radio.hear(now, heard(&probe), |at| map.get(&at).copied()));
        }
        assert_eq!(radio.read(now, RX_NEXT), Some(0));
        assert_eq!(radio.read(now, RX_LAST), Some(RX1 & RX_ADDRESS));
        let map = ram.0.clone();
        assert_eq!(
            radio.hear(now, heard(&probe), |at| map.get(&at).copied()),
            Received::Missed
        );

        // The driver hands RX0 back and links it after RX1.
        ram.descriptor(RX0, BUF0, SIZE, 0);
        ram.poke(RX1 + 8, &RX0.to_le_bytes());
        let map = ram.0.clone();
        let reload = radio.write(now, RX_RELOAD, 0x8000_0001, |at| map.get(&at).copied());
        assert_eq!(reload, Store::Kept);
        assert_eq!(
            radio.read(now, RX_RELOAD),
            Some(0x8000_0000),
            "reloaded at once"
        );
        assert_eq!(radio.read(now, RX_NEXT), Some(RX0 & RX_ADDRESS));
        let map = ram.0.clone();
        assert!(ram.apply(radio.hear(now, heard(&probe), |at| map.get(&at).copied())));
    }

    #[test]
    fn a_frame_the_model_cannot_receive_is_refused_by_name() {
        let mut ram = Ram::default();
        let mut radio = listening(&mut ram);
        ram.descriptor(RX0, BUF0, 64, RX1);
        let probe = management(5, STA);
        let map = ram.0.clone();
        let byte = |at| map.get(&at).copied();
        assert_eq!(
            radio.hear(Tick(0), heard(&probe), byte),
            Received::Refused("rx_length")
        );
        let mut radio = listening(&mut ram);
        assert!(write(&mut radio, RX_BASE, 0x3fc0_0000));
        assert_eq!(
            radio.hear(Tick(0), heard(&probe), |_| None),
            Received::Refused("rx_descriptor")
        );
    }

    /// A latch bit copies the TSF, in microseconds, into the two words, and a load sets timer 0
    /// or 1 to count on from the loaded count. Timer 0 counts only while bit 31 of
    /// [`TBTT_CONTROL`] is set.
    #[test]
    fn a_latch_reads_the_tsf_and_a_load_sets_it() {
        let mut radio = powered();
        let now = Tick(480 * ((5 << 32) + 7));
        // Timer 0 starts frozen at 0 until `hal_enable_sta_tsf` enables it.
        assert_eq!(radio.write(now, TSF_CONTROL, 1, |_| None), Store::Kept);
        assert_eq!(radio.read(now, TSF_LOW), Some(0));
        assert_eq!(radio.read(now, TSF_HIGH), Some(0));
        assert_eq!(radio.write(now, TSF_CONTROL, 0, |_| None), Store::Kept);
        assert_eq!(
            radio.write(Tick(0), TBTT_CONTROL, TSF0_ON | TSF0_MODE, |_| None),
            Store::Kept
        );
        // `hal_get_tsf_time(0)`: set bit 0, read the two words, clear bit 0.
        assert_eq!(radio.write(now, TSF_CONTROL, 1, |_| None), Store::Kept);
        assert_eq!(radio.read(Tick(now.0 + 480), TSF_LOW), Some(7));
        assert_eq!(radio.read(now, TSF_HIGH), Some(5));
        assert_eq!(radio.write(now, TSF_CONTROL, 0, |_| None), Store::Kept);

        // `hal_set_sta_tsf`: the AP's count, then bit 4, which reads 0 at once.
        assert!(write(&mut radio, TSF_LOAD_LOW, 0x8000_0000));
        assert!(write(&mut radio, TSF_LOAD_HIGH, 9));
        assert_eq!(radio.write(now, TSF_CONTROL, 1 << 4, |_| None), Store::Kept);
        assert_eq!(radio.read(now, TSF_CONTROL), Some(0));
        // 3 us later timer 0 reads the loaded count and 3, and timer 2 the chip's own.
        let later = Tick(now.0 + 3 * 480);
        assert_eq!(radio.write(later, TSF_CONTROL, 1, |_| None), Store::Kept);
        assert_eq!(radio.read(later, TSF_LOW), Some(0x8000_0003));
        assert_eq!(radio.read(later, TSF_HIGH), Some(9));
        assert_eq!(
            radio.write(later, TSF_CONTROL, 1 << 2, |_| None),
            Store::Kept
        );
        assert_eq!(radio.read(later, TSF_LOW), Some(10));
        assert_eq!(radio.read(later, TSF_HIGH), Some(5));
        // `hal_mac_tsf_reset`: clear bit 31, load 0 into timers 0 and 1, set bit 31.
        assert_eq!(
            radio.write(later, TBTT_CONTROL, TSF0_MODE, |_| None),
            Store::Kept
        );
        assert!(write(&mut radio, TSF_LOAD_LOW, 0));
        assert!(write(&mut radio, TSF_LOAD_HIGH, 0));
        assert_eq!(radio.write(later, TSF_CONTROL, 0x30, |_| None), Store::Kept);
        assert_eq!(
            radio.write(later, TBTT_CONTROL, TSF0_ON | TSF0_MODE, |_| None),
            Store::Kept
        );
        assert_eq!(radio.write(later, TSF_CONTROL, 1, |_| None), Store::Kept);
        assert_eq!(radio.read(later, TSF_LOW), Some(0));
        assert_eq!(radio.read(later, TSF_HIGH), Some(0));
        assert_eq!(radio.write(later, TSF_CONTROL, 2, |_| None), Store::Kept);
        assert_eq!(radio.read(Tick(later.0 + 480), TSF_LOW), Some(0));
        assert_eq!(radio.read(later, TSF_HIGH), Some(0));
    }

    /// `tsf_hal_set_counter_value` loads timer `n` with bit `4 + n`, timers 2 and 3 too, and
    /// each counts on from its own load.
    #[test]
    fn a_load_sets_each_of_the_four_timers() {
        let mut radio = powered();
        assert_eq!(
            radio.write(Tick(0), TBTT_CONTROL, TSF0_ON | TSF0_MODE, |_| None),
            Store::Kept
        );
        let now = Tick(480 * 1_000);
        for n in 0..4 {
            assert!(write(&mut radio, TSF_LOAD_LOW, 100 * (n + 1)));
            assert!(write(&mut radio, TSF_LOAD_HIGH, n));
            assert_eq!(
                radio.write(now, TSF_CONTROL, 1 << (4 + n), |_| None),
                Store::Kept
            );
            assert_eq!(radio.read(now, TSF_CONTROL), Some(0));
        }
        let later = Tick(now.0 + 5 * 480);
        for n in 0..4 {
            assert_eq!(
                radio.write(later, TSF_CONTROL, 1 << n, |_| None),
                Store::Kept
            );
            assert_eq!(
                radio.read(later, TSF_LOW),
                Some(100 * (n + 1) + 5),
                "timer {n}"
            );
            assert_eq!(radio.read(later, TSF_HIGH), Some(n), "timer {n}");
            assert_eq!(radio.write(later, TSF_CONTROL, 0, |_| None), Store::Kept);
        }
    }

    /// Every bit of the TSF control word, TBTT 0's control word, the TBTT start and the power
    /// interrupt's enable word that a blob function sets is kept, and each other bit is refused
    /// by name. So is the interrupt of TBTTs 1 to 3 and of the TSF timers' alarms.
    #[test]
    fn a_tsf_or_tbtt_bit_no_blob_function_sets_is_refused() {
        let kept = [
            (TSF_CONTROL, 0xfff),
            (TBTT_CONTROL, 0xbc00_0000),
            (TBTT, 0x03ff_ffff),
            (PWR_ENABLE, PWR_TBTT),
        ];
        for (at, bits) in kept {
            let mut radio = powered();
            assert_eq!(
                radio.write(Tick(0), at, bits, |_| None),
                Store::Kept,
                "{at:x}"
            );
        }
        let refused = [
            (TSF_CONTROL, 1 << 12, "tsf_control"),
            (TSF_CONTROL, 1 << 31, "tsf_control"),
            (TBTT_CONTROL, 1, "tbtt_control"),
            (TBTT_CONTROL, 1 << 25, "tbtt_control"),
            (TBTT_CONTROL, 1 << 30, "tbtt_control"),
            (TBTT, 1 << 26, "tbtt_start"),
            (PWR_ENABLE, 1, "pwr_intr"),
            (PWR_ENABLE, 1 << 9, "pwr_intr"),
            (PWR_ENABLE, 1 << 3, "tbtt_intr"),
            (PWR_ENABLE, 1 << 2, "tbtt_intr"),
            (PWR_ENABLE, 1 << 1, "tbtt_intr"),
            (PWR_ENABLE, 1 << 8, "tsf_timer"),
            (PWR_ENABLE, 1 << 5, "tsf_timer"),
        ];
        for (at, bits, name) in refused {
            let mut radio = powered();
            assert_eq!(
                radio.write(Tick(0), at, bits, |_| None),
                Store::Refused(name),
                "{at:x} {bits:x}"
            );
        }
    }

    /// The key table holds 25 entries of 10 words from `0x6003_4400`, each reading what was
    /// written, and its valid bits beside the other kept words. `hal_crypto_clr_key_entry`
    /// writes 0 over an entry's ten words.
    #[test]
    fn the_key_table_is_storage() {
        let mut radio = powered();
        assert!(Radio::slot(0x6003_3814).is_some(), "the valid bits");
        let last = keys::KEYS + 4 * (KEY_TABLE - 1) as u32;
        for addr in [keys::KEYS, last] {
            assert!(blocks::name(addr).starts_with("RADIO"), "{addr:#x}");
        }
        for entry in [0, 24] {
            let at = keys::KEYS + 40 * entry;
            for k in 0..keys::WORDS as u32 {
                assert_eq!(read(&mut radio, at + 4 * k), Some(0));
                assert!(write(&mut radio, at + 4 * k, 0x1000 * entry + k));
            }
            assert_eq!(read(&mut radio, at + 4), Some(0x1000 * entry + 1));
            for k in 0..keys::WORDS as u32 {
                assert!(write(&mut radio, at + 4 * k, 0));
                assert_eq!(read(&mut radio, at + 4 * k), Some(0));
            }
        }
        assert_eq!(read(&mut radio, keys::KEYS + 40 * 25), None);
        assert_eq!(read(&mut radio, keys::KEYS - 4), None);
    }

    /// After the join the driver leaves slot 0's receiver address on the station and puts its
    /// BSSID on the access point's, with these words. The access point's Beacons and its frames
    /// to every station land, unacknowledged, and its frames to the station land and are
    /// acknowledged. Another network's Beacon does not land. `hal_set_sta_beacon_filter` later
    /// sets bit 16 of the BSSID's second word, which the filter does not read.
    #[test]
    fn after_the_join_the_access_points_frames_land_and_another_networks_do_not() {
        for high in [0x165, 0x1_0165] {
            let mut ram = Ram::default();
            let mut radio = listening(&mut ram);
            for (at, value) in [
                (RA, 0x2),
                (RA + 4, 0x100),
                (RA_MASK, !0),
                (RA_MASK + 4, 0x1_ffff),
                (BSSID, 0x6272_6f02),
                (BSSID + 4, high),
                (BSSID_MASK, !0),
                (BSSID_MASK + 4, 0x1_ffff),
            ] {
                assert!(write(&mut radio, at, value));
            }
            let mut land = |mpdu: &[u8]| {
                ram.descriptor(RX0, BUF0, SIZE, RX1);
                assert!(write(&mut radio, RX_BASE, RX0));
                let map = ram.0.clone();
                match radio.hear(Tick(0), heard(mpdu), |at| map.get(&at).copied()) {
                    Received::Kept { ack, .. } => Some(ack),
                    Received::Missed => None,
                    refused => panic!("{refused:?}"),
                }
            };
            assert_eq!(land(&management(8, [0xff; 6])), Some(false), "{high:#x}");
            // A Data frame from the DS: Address 1 the receiver, Address 2 the BSSID.
            let data = |ra: [u8; 6]| {
                let mut mpdu = vec![0x08, 0x02, 0, 0];
                mpdu.extend(ra);
                mpdu.extend(AP);
                mpdu.extend([0x02, 0, 0, 0, 0, 0x09, 0x10, 0x00, 0xaa, 0xaa]);
                mpdu
            };
            assert_eq!(land(&data(STA)), Some(true));
            assert_eq!(land(&data([0xff; 6])), Some(false));
            let mut other = management(8, [0xff; 6]);
            other[10..22].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x07, 0x02, 0, 0, 0, 0, 0x07]);
            assert_eq!(land(&other), None);
        }
    }

    /// The MAC's clock reads the microseconds the `rx_ctrl` timestamp counts.
    #[test]
    fn the_mac_clock_counts_microseconds() {
        let mut radio = powered();
        assert_eq!(
            radio.read(Tick(480 * 3_207_712 + 479), MAC_CLOCK),
            Some(3_207_712)
        );
        assert_eq!(radio.read(Tick(480 << 32), MAC_CLOCK), Some(0));
    }

    /// IEEE 802.11-2020 J.6.4's CCMP test vector: the TK, a Data frame's MAC header with Retry
    /// and Protected set, its CCMP header of PN 0xb5039776e70c, its plaintext, and the ciphertext
    /// and MIC.
    const TK: [u8; 16] = [
        0xc9, 0x7c, 0x1f, 0x67, 0xce, 0x37, 0x11, 0x85, 0x51, 0x4a, 0x8a, 0x19, 0xf2, 0xbd, 0xd5,
        0x2f,
    ];
    const VECTOR_HEADER: [u8; 24] = [
        0x08, 0x48, 0xc3, 0x2c, 0x0f, 0xd2, 0xe1, 0x28, 0xa5, 0x7c, 0x50, 0x30, 0xf1, 0x84, 0x44,
        0x08, 0xab, 0xae, 0xa5, 0xb8, 0xfc, 0xba, 0x80, 0x33,
    ];
    const VECTOR_CCMP: [u8; 8] = [0x0c, 0xe7, 0x00, 0x20, 0x76, 0x97, 0x03, 0xb5];
    const VECTOR_PLAINTEXT: [u8; 20] = [
        0xf8, 0xba, 0x1a, 0x55, 0xd0, 0x2f, 0x85, 0xae, 0x96, 0x7b, 0xb6, 0x2f, 0xb6, 0xcd, 0xa8,
        0xeb, 0x7e, 0x78, 0xa0, 0x50,
    ];
    const VECTOR_SEALED: [u8; 28] = [
        0xf3, 0xd0, 0xa2, 0xfe, 0x9a, 0x3d, 0xbf, 0x23, 0x42, 0xa6, 0x43, 0xe4, 0x32, 0x46, 0xe8,
        0x0c, 0x3c, 0x04, 0xd0, 0x19, 0x78, 0x45, 0xce, 0x0b, 0x16, 0xf9, 0x76, 0x23,
    ];

    /// The words the stock station's driver writes for its pairwise key: entry 4, CCMP, bits 21,
    /// 22 and 27, interface 0, key ID 0, for `peer`, or bits 21 to 23 for WEP. Valid, and interface 0's crypto on, as
    /// `hal_crypto_enable` leaves it.
    fn install(radio: &mut Radio, peer: [u8; 6], key: [u8; 16], cipher: u32) {
        let at = keys::KEYS + 40 * 4;
        let high = u32::from(u16::from_le_bytes([peer[4], peer[5]]));
        assert!(write(
            radio,
            at,
            u32::from_le_bytes(peer[..4].try_into().unwrap())
        ));
        let kind = if cipher == 1 { 7 } else { 3 };
        assert_eq!(
            radio.write(
                Tick(0),
                at + 4,
                1 << 27 | kind << 21 | cipher << 18 | high,
                |_| None
            ),
            Store::Kept
        );
        for k in 0..4 {
            let word = u32::from_le_bytes(key[4 * k..4 * k + 4].try_into().unwrap());
            assert!(write(radio, at + 8 + 4 * k as u32, word));
        }
        assert!(write(radio, keys::VALID, 1 << 4));
        assert!(write(radio, keys::CONTROL, 0x30103));
    }

    /// The vector's frame as the driver leaves it in a queue's buffer: the CCMP header it filled,
    /// the plaintext, the 8 octets for the MIC with `ppProcTxSecFrame`'s marks, and the FCS.
    fn protected_buffer() -> Vec<u8> {
        let mut buffer = VECTOR_HEADER.to_vec();
        buffer.extend(VECTOR_CCMP);
        buffer.extend(VECTOR_PLAINTEXT);
        buffer.extend([0xef, 0xbe, 0xad, 0xde, 0, 0, 0x78, 0x56]);
        buffer.extend([0; 4]);
        buffer
    }

    /// A protected frame goes on the air under the key entry PLCP1's bits 17 to 24 name, with
    /// its ciphertext and MIC over the plaintext and the 8 octets left for the MIC, as the
    /// standard's vector has them. Its PN is the one in the driver's CCMP header.
    #[test]
    fn a_protected_frame_goes_on_the_air_sealed_under_the_entry_plcp1_names() {
        let buffer = protected_buffer();
        let length = buffer.len() as u32;
        let mut radio = ready(4 << keys::PLCP1_SHIFT | length);
        install(&mut radio, VECTOR_HEADER[4..10].try_into().unwrap(), TK, 3);
        assert_eq!(start(&mut radio, Tick(0), &buffer, length), Store::Sent);
        let end = radio.end().unwrap();
        radio.report(end);
        let frame = radio.take_aired().unwrap();
        let mut want = VECTOR_HEADER.to_vec();
        want.extend(VECTOR_CCMP);
        want.extend(VECTOR_SEALED);
        assert_eq!(*frame.mpdu, want[..]);
        assert_eq!(
            frame.end,
            after(frame.start, Rate::Dsss1.airtime(length as usize))
        );
    }

    /// When the entry PLCP1 names is not in use, or is beyond the table, the MAC sends nothing.
    /// It reports the outcome `lmacProcessTxError` hands to `lmacProcessTxseckiderr`, when the
    /// frame would have started.
    #[test]
    fn a_protected_frame_without_its_entry_reports_a_key_error_and_is_not_sent() {
        let buffer = protected_buffer();
        let length = buffer.len() as u32;
        for entry in [4, 5, 25, 0x84] {
            let mut radio = ready(entry << keys::PLCP1_SHIFT | length);
            install(&mut radio, VECTOR_HEADER[4..10].try_into().unwrap(), TK, 3);
            if entry == 4 {
                assert!(write(&mut radio, keys::VALID, 0));
            }
            assert_eq!(start(&mut radio, Tick(0), &buffer, length), Store::Kept);
            assert_eq!(radio.end(), None);
            let done = Tick(50 * 480);
            assert_eq!(radio.next_event(Tick(0)), Some(done));
            assert!(!radio.line(Tick(done.0 - 1)));
            assert!(radio.line(done));
            assert_eq!(radio.read(done, TX_STATUS), Some(TX_KEY_ERROR));
            assert_eq!(radio.read(done, TX_DONE), Some(1));
        }
    }

    /// A cipher other than CCMP-128 and a protected Management frame are refused by name.
    #[test]
    fn a_protected_frame_the_model_cannot_seal_is_refused_by_name() {
        let buffer = protected_buffer();
        let length = buffer.len() as u32;
        for (cipher, name) in [(1, "tx_wep"), (2, "tx_tkip"), (5, "tx_gcmp")] {
            let mut radio = ready(4 << keys::PLCP1_SHIFT | length);
            install(&mut radio, [0; 6], TK, cipher);
            assert_eq!(
                start(&mut radio, Tick(0), &buffer, length),
                Store::Refused(name)
            );
        }
        let mut management = buffer.clone();
        management[0] = 0xd0;
        let mut radio = ready(4 << keys::PLCP1_SHIFT | length);
        install(&mut radio, [0; 6], TK, 3);
        assert_eq!(
            start(&mut radio, Tick(0), &management, length),
            Store::Refused("tx_mfp")
        );
    }

    /// A listening station whose pairwise entry holds the access point's address.
    fn keyed(ram: &mut Ram) -> Radio {
        let mut radio = listening(ram);
        install(&mut radio, AP, TK, 3);
        radio
    }

    /// The vector's frame from the access point to the station, sealed as the station's MAC
    /// seals the vector.
    fn sealed_vector() -> Vec<u8> {
        let mut buffer = protected_buffer();
        buffer[1] = 0x42;
        buffer[4..10].copy_from_slice(&STA);
        buffer[10..16].copy_from_slice(&AP);
        let length = buffer.len() as u32;
        let mut radio = ready(4 << keys::PLCP1_SHIFT | length);
        install(&mut radio, AP, TK, 3);
        assert_eq!(start(&mut radio, Tick(0), &buffer, length), Store::Sent);
        let end = radio.end().unwrap();
        radio.report(end);
        radio.take_aired().unwrap().mpdu.to_vec()
    }

    /// A protected frame that the pairwise entry of its Address 2 takes lands with its plaintext
    /// over its ciphertext. The MAC header, the CCMP header and the MIC stay, for the driver's
    /// `ccmp_decap` to read and strip, and `rx_state` is 0.
    #[test]
    fn a_protected_frame_lands_decrypted_in_place() {
        let mut ram = Ram::default();
        let mut radio = keyed(&mut ram);
        let mpdu = sealed_vector();
        let ack = ram.apply(radio.hear(Tick(0), heard(&mpdu), |at| ram.0.get(&at).copied()));
        assert!(ack);
        let at = BUF0 + RX_CTRL as u32;
        let mut want = mpdu.clone();
        want[32..52].copy_from_slice(&VECTOR_PLAINTEXT);
        assert_eq!(ram.bytes(at, mpdu.len()), want);
        assert_eq!(ram.word(BUF0 + 44), mpdu.len() as u32 + 4, "rx_state 0");
        let length = RX_CTRL as u32 + mpdu.len() as u32 + 4;
        assert_eq!(ram.word(RX0), EOF | length << 12 | SIZE);
    }

    /// A protected frame whose MIC fails, or that no entry takes, lands as it came with an
    /// `rx_state` the driver drops, and is still acknowledged, as the ACK does not wait for the
    /// MIC.
    #[test]
    fn a_protected_frame_the_mac_cannot_open_lands_as_it_came_with_its_rx_state() {
        let mut tampered = sealed_vector();
        tampered[40] ^= 1;
        let mut short = sealed_vector();
        short[27] &= !0x20;
        for (mpdu, valid, state) in [
            (tampered, 1 << 4, RX_MIC_FAILURE),
            (short, 1 << 4, RX_MIC_FAILURE),
            (sealed_vector(), 0, RX_NO_KEY),
        ] {
            let mut ram = Ram::default();
            let mut radio = keyed(&mut ram);
            assert!(write(&mut radio, keys::VALID, valid));
            let ack = ram.apply(radio.hear(Tick(0), heard(&mpdu), |at| ram.0.get(&at).copied()));
            assert!(ack);
            assert_eq!(ram.bytes(BUF0 + RX_CTRL as u32, mpdu.len()), mpdu);
            assert_eq!(
                ram.word(BUF0 + 44),
                state << RX_STATE_SHIFT | (mpdu.len() as u32 + 4)
            );
            assert_ne!(state, 245, "not the TKIP Michael MIC failure");
        }
    }

    /// A protected frame the model cannot open is refused by name, once a slot took it.
    #[test]
    fn a_protected_frame_the_model_cannot_open_is_refused_by_name() {
        let mut ram = Ram::default();
        let mut radio = listening(&mut ram);
        install(&mut radio, AP, TK, 2);
        let mut mpdu = management(5, STA);
        mpdu[1] = PROTECTED;
        let map = ram.0.clone();
        assert_eq!(
            radio.hear(Tick(0), heard(&mpdu), |at| map.get(&at).copied()),
            Received::Refused("rx_mfp")
        );
        let mut data = sealed_vector();
        data[4..10].copy_from_slice(&STA);
        data[10..16].copy_from_slice(&AP);
        assert_eq!(
            radio.hear(Tick(0), heard(&data), |at| map.get(&at).copied()),
            Received::Refused("rx_tkip")
        );
        // One no slot takes is dropped as before.
        mpdu[4..10].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x02]);
        assert_eq!(
            radio.hear(Tick(0), heard(&mpdu), |at| map.get(&at).copied()),
            Received::Missed
        );
    }

    /// Turning on a coexistence timer, forcing it or turning on the beacon filter is refused by
    /// name. Turning either off, as the stock station does, is kept, and so are the bits
    /// `coex_hw_timer_set` writes. A bit no blob function sets is refused by name.
    #[test]
    fn a_coexistence_timer_turned_on_or_a_beacon_filter_store_no_blob_function_makes_is_refused() {
        for timer in [
            COEX_TIMERS,
            COEX_TIMERS + 8,
            COEX_TIMERS + 16,
            COEX_TIMERS + 24,
        ] {
            for (value, store) in [
                (0, Store::Kept),
                (0x4fff_ffff, Store::Kept),
                (COEX_TIMER_ON, Store::Refused("coex_timer")),
                (COEX_TIMER_FORCE, Store::Refused("coex_force")),
                (1 << 29, Store::Refused("coex_timer_bits")),
            ] {
                let mut radio = powered();
                assert_eq!(
                    radio.write(Tick(0), timer, value, |_| None),
                    store,
                    "{timer:#x} {value:#x}"
                );
            }
        }
        for (value, store) in [
            (0, Store::Kept),
            (7, Store::Kept),
            (1, Store::Refused("beacon_filter")),
            (6, Store::Refused("beacon_filter")),
            (1 << 3, Store::Refused("beacon_filter_bits")),
            (1 << 31, Store::Refused("beacon_filter_bits")),
        ] {
            let mut radio = powered();
            assert_eq!(
                radio.write(Tick(0), BEACON_FILTER, value, |_| None),
                store,
                "{value:#x}"
            );
        }
    }

    /// Each PLCP1 bit `mac_tx_set_plcp1` sets is kept: the length, the rate, the key entry's byte
    /// in bits 17 to 24, bit 25 with an HT rate and bit 29. Each other bit, and bit 25 with a rate
    /// below 16, is refused by name.
    #[test]
    fn a_plcp1_bit_no_blob_function_sets_is_refused() {
        for value in [
            0x0008_b1ff,
            0xff << keys::PLCP1_SHIFT | 0x31,
            PLCP1_HT | 0x10 << PLCP1_RATE_SHIFT | 0x31,
            1 << 29 | 0x31,
        ] {
            let mut radio = powered();
            assert_eq!(
                radio.write(Tick(0), PLCP1, value, |_| None),
                Store::Kept,
                "{value:#x}"
            );
        }
        for value in [1 << 26, 1 << 27, 1 << 28, 1 << 30, 1 << 31, PLCP1_HT | 0x31] {
            let mut radio = powered();
            assert_eq!(
                radio.write(Tick(0), PLCP1, value, |_| None),
                Store::Refused("tx_plcp1"),
                "{value:#x}"
            );
        }
        // MCS 0 with bit 25 goes on the air, and so does a frame with bit 29.
        for plcp1 in [PLCP1_HT | 0x10 << PLCP1_RATE_SHIFT | 0x31, 1 << 29 | 0x31] {
            let mut radio = ready(plcp1);
            assert_eq!(start(&mut radio, Tick(0), &PROBE, 49), Store::Sent);
        }
    }

    /// The radio refuses a store to the key table or its control words that no blob function
    /// makes, as [`keys::refused`] names it, and keeps the stock station's.
    #[test]
    fn a_key_table_store_no_blob_function_makes_is_refused() {
        let mut radio = powered();
        let word1 = keys::KEYS + 40 * 4 + 4;
        for (addr, value) in [
            (word1, 0x086c_0165),
            (keys::VALID, 0x11),
            (keys::CONTROL, 0x30103),
        ] {
            assert_eq!(radio.write(Tick(0), addr, value, |_| None), Store::Kept);
        }
        for (addr, value, name) in [
            (word1, 0x086e_0165, "key_entry"),
            (keys::VALID, 1 << 25, "key_valid"),
            (keys::CONTROL, 0x103, "crypto_control"),
            (0x6003_3840, 24, "crypto_init"),
        ] {
            assert_eq!(
                radio.write(Tick(0), addr, value, |_| None),
                Store::Refused(name)
            );
        }
    }

    /// The stock station's stores, in its order: timer 0 loaded with the access point's TSF
    /// 528 us behind the chip's, the TBTT's interrupt and then TBTT 0 turned on at 3.2077 s,
    /// as `hal_enable_sta_tbtt` does, and `hal_set_sta_tbtt`'s interval of 100 TU, start at
    /// 3 072 000 us and its latch, and ahead time of 3000 us. Its first TBTT after those is at
    /// 3 276 800 us of timer 0, so its interrupt would come at 3 273 800 us, 3 274 328 us of the
    /// chip's time. Turned off either way, none is due.
    #[test]
    fn the_first_armed_tbtt_falls_on_a_whole_beacon_interval() {
        let at = |us: u64| Tick(480 * us);
        let store = |radio: &mut Radio, now: Tick, addr: u32, value: u32| {
            assert_eq!(radio.write(now, addr, value, |_| None), Store::Kept);
        };
        let stock = || {
            let mut radio = powered();
            store(&mut radio, at(0), TBTT_CONTROL, TSF0_ON | TSF0_MODE);
            assert!(write(&mut radio, TSF_LOAD_LOW, 0x30_8a21));
            store(&mut radio, at(3_181_617), TSF_CONTROL, 1 << 4);
            store(&mut radio, at(3_207_710), TBTT_CONTROL, 0xb800_0000);
            store(&mut radio, at(3_207_712), PWR_ENABLE, PWR_TBTT);
            assert_eq!(radio.tbtt(), None, "TBTT 0 is off");
            store(&mut radio, at(3_207_712), TBTT_CONTROL, 0xbc00_0000);
            assert!(radio.tbtt().is_some(), "armed");
            let set = at(3_207_713);
            store(&mut radio, set, TBTT_TIMING, 0x64);
            store(&mut radio, set, TBTT, 0x2e_e000);
            store(&mut radio, set, TSF_CONTROL, 1 << 8);
            assert_eq!(radio.read(set, TSF_CONTROL), Some(0));
            store(&mut radio, set, TBTT_TIMING, 0x0bb8_0064);
            radio
        };
        let mut radio = stock();
        assert_eq!(radio.tbtt(), Some(at(3_274_328)));
        // A store to the start that no latch follows moves nothing.
        store(&mut radio, at(3_207_714), TBTT, 0x2e_e000 + 512);
        assert_eq!(radio.tbtt(), Some(at(3_274_328)));
        // A later store that keeps bit 4 set keeps the arming.
        store(&mut radio, at(3_207_800), PWR_ENABLE, PWR_TBTT);
        assert_eq!(radio.tbtt(), Some(at(3_274_328)));
        store(&mut radio, at(3_207_900), PWR_ENABLE, 0);
        assert_eq!(radio.tbtt(), None);
        // `hal_disable_sta_tbtt` clears TBTT 0's bit first.
        let mut radio = stock();
        store(&mut radio, at(3_207_900), TBTT_CONTROL, 0xb800_0000);
        assert_eq!(radio.tbtt(), None);
    }

    /// A TBTT's settings that change after it is armed count from the change: a start latched
    /// after its first interrupt since the arming would have come brings the interrupt at the
    /// next TBTT, not at once, and so does a load of timer 0.
    #[test]
    fn a_tbtt_set_after_its_arming_counts_from_the_store_that_set_it() {
        let mut radio = powered();
        let at = |us: u64| Tick(480 * us);
        assert!(write(&mut radio, TBTT_TIMING, 10 << 16 | 1));
        assert!(write(
            &mut radio,
            TBTT_CONTROL,
            TSF0_ON | TSF0_MODE | TBTT_ON
        ));
        assert_eq!(
            radio.write(at(0), PWR_ENABLE, PWR_TBTT, |_| None),
            Store::Kept
        );
        assert_eq!(radio.tbtt(), Some(at(1014)));
        // At 1000 us a start 500 us into each TU is latched: its 500 us has passed, so the
        // first is at 1524 us.
        assert!(write(&mut radio, TBTT, 500));
        assert_eq!(
            radio.write(at(1000), TSF_CONTROL, 1 << 8, |_| None),
            Store::Kept
        );
        assert_eq!(radio.tbtt(), Some(at(1514)));
        // A load of timer 0 to 0 at 1100 us puts the next TBTT at its 500 us.
        assert!(write(&mut radio, TSF_LOAD_LOW, 0));
        assert_eq!(
            radio.write(at(1100), TSF_CONTROL, 1 << 4, |_| None),
            Store::Kept
        );
        assert_eq!(radio.tbtt(), Some(at(1590)));
    }

    /// The stock station's TBTT, as `the_first_armed_tbtt_falls_on_a_whole_beacon_interval` sets
    /// it, raises bit 4 of the power interrupt's raw and masked status at 3 274 328 us of the
    /// chip's time, and sets bit 1 of `TSF_ACTIVE`. `hal_pwr_interrupt_clr_event`'s store to
    /// `PWR_CLEAR` (which reads 0) clears it, and `hal_clear_sta_tsf_active_state`'s bit 0 clears
    /// `TSF_ACTIVE`. With `PWR_ENABLE` cleared, `PWR_EVENTS` reads 0 while `PWR_RAW` keeps the
    /// bit until cleared.
    #[test]
    fn the_armed_tbtt_raises_the_power_interrupt_each_beacon_interval() {
        let at = |us: u64| Tick(480 * us);
        let store = |radio: &mut Radio, now: Tick, addr: u32, value: u32| {
            assert_eq!(radio.write(now, addr, value, |_| None), Store::Kept);
        };
        let mut radio = powered();
        store(&mut radio, at(0), TBTT_CONTROL, TSF0_ON | TSF0_MODE);
        assert!(write(&mut radio, TSF_LOAD_LOW, 0x30_8a21));
        store(&mut radio, at(3_181_617), TSF_CONTROL, 1 << 4);
        store(&mut radio, at(3_207_712), PWR_ENABLE, PWR_TBTT);
        store(&mut radio, at(3_207_712), TBTT_CONTROL, 0xbc00_0000);
        let set = at(3_207_713);
        store(&mut radio, set, TBTT_TIMING, 0x64);
        store(&mut radio, set, TBTT, 0x2e_e000);
        store(&mut radio, set, TSF_CONTROL, 1 << 8);
        store(&mut radio, set, TBTT_TIMING, 0x0bb8_0064);

        let first = at(3_274_328);
        assert_eq!(radio.next_event(set), Some(first));
        assert!(!radio.pwr_line(Tick(first.0 - 1)));
        assert_eq!(radio.read(Tick(first.0 - 1), PWR_RAW), Some(0));
        assert_eq!(radio.read(Tick(first.0 - 1), PWR_EVENTS), Some(0));
        assert_eq!(radio.read(Tick(first.0 - 1), TSF_ACTIVE), Some(0));
        assert!(radio.pwr_line(first));
        assert!(!radio.line(first), "the MAC interrupt stays low");
        assert_eq!(radio.read(first, PWR_RAW), Some(PWR_TBTT));
        assert_eq!(radio.read(first, PWR_EVENTS), Some(PWR_TBTT));
        assert_eq!(radio.read(first, TSF_ACTIVE), Some(TSF_ACTIVE_STATUS));
        // `hal_clear_sta_tsf_active_state` RMWs bit 0 over the active status bit 1.
        store(
            &mut radio,
            first,
            TSF_ACTIVE,
            TSF_ACTIVE_STATUS | TSF_ACTIVE_CLEAR,
        );
        assert_eq!(radio.read(first, TSF_ACTIVE), Some(0));
        let next = at(3_274_328 + 102_400);
        assert_eq!(radio.next_event(first), Some(next));
        store(&mut radio, at(3_274_329), PWR_CLEAR, PWR_TBTT);
        assert_eq!(radio.read(at(3_274_329), PWR_CLEAR), Some(0));
        assert!(!radio.pwr_line(at(3_274_329)));
        assert!(!radio.pwr_line(Tick(next.0 - 1)));
        assert!(radio.pwr_line(next));
        // Two intervals without a poll raise the bit once.
        store(&mut radio, next, PWR_CLEAR, PWR_TBTT);
        let later = at(3_274_328 + 3 * 102_400);
        assert!(radio.pwr_line(later));
        assert_eq!(radio.next_event(later), Some(at(3_274_328 + 4 * 102_400)));

        // Masking `PWR_ENABLE` masks `PWR_EVENTS` (`INT_ST`) while `PWR_RAW` (`INT_RAW`) stays
        // set until cleared, and re-enabling before clearing exposes it again.
        store(&mut radio, later, PWR_ENABLE, 0);
        assert!(!radio.pwr_line(later));
        assert_eq!(radio.read(later, PWR_EVENTS), Some(0));
        assert_eq!(radio.read(later, PWR_RAW), Some(PWR_TBTT));
        store(&mut radio, later, PWR_ENABLE, PWR_TBTT);
        assert!(radio.pwr_line(later));
        assert_eq!(radio.read(later, PWR_EVENTS), Some(PWR_TBTT));
        store(&mut radio, later, PWR_ENABLE, 0);
        store(&mut radio, later, PWR_CLEAR, PWR_TBTT);
        assert_eq!(radio.read(later, PWR_RAW), Some(0));
        assert_eq!(radio.read(later, PWR_CLEAR), Some(0));
        assert_eq!(radio.tbtt(), None);
        assert_eq!(radio.next_event(later), None);
        assert!(!radio.pwr_line(at(4_000_000)));
        assert_eq!(
            radio.write(later, PWR_EVENTS, PWR_TBTT, |_| None),
            Store::Refused("pwr_events")
        );
        assert_eq!(
            radio.write(later, PWR_RAW, PWR_TBTT, |_| None),
            Store::Refused("pwr_raw")
        );
    }

    /// A Beacon from the access point with a TIM whose Partial Virtual Bitmap starts at octet
    /// `n1` and holds `bitmap`, with the group bit `group`.
    fn beacon(n1: u8, bitmap: &[u8], group: bool) -> Vec<u8> {
        let mut mpdu = management(8, [0xff; 6]);
        mpdu.truncate(24);
        mpdu.extend([0; 8]);
        mpdu.extend(100u16.to_le_bytes());
        mpdu.extend([0x11, 0x04]);
        mpdu.extend([0, 4]);
        mpdu.extend(*b"orbe");
        mpdu.extend([5, 3 + bitmap.len() as u8, 0, 1, n1 | u8::from(group)]);
        mpdu.extend(bitmap);
        mpdu
    }

    /// With the filter on and the station's AID 9 in bits 16 to 26 of slot 0's BSSID word, as
    /// `pm_set_beacon_filter` leaves them, a Beacon whose TIM leaves bit 9 clear lands nowhere and
    /// raises bit 15 of the events a tick later, and one that sets it lands as before. With the
    /// filter off every Beacon lands.
    #[test]
    fn the_beacon_filter_lets_through_a_beacon_whose_tim_names_the_station() {
        let now = Tick(1_000);
        let byte = |ram: &Ram| {
            let map = ram.0.clone();
            move |at| map.get(&at).copied()
        };
        let filtered = |ram: &mut Ram| {
            let mut radio = listening(ram);
            filter(&mut radio, BSSID, BSSID_MASK, AP);
            let high = u32::from(u16::from_le_bytes([AP[4], AP[5]]));
            assert!(write(&mut radio, BEACON_AID, 9 << BEACON_AID_SHIFT | high));
            assert!(write(&mut radio, EVENTS_ENABLE, 0x19a8_f9e0));
            assert!(write(&mut radio, BEACON_FILTER, BEACON_FILTER_ON));
            radio
        };
        let mut ram = Ram::default();
        let mut radio = filtered(&mut ram);
        for quiet in [beacon(0, &[0], false), beacon(0, &[0, 1], false)] {
            assert_eq!(radio.hear(now, heard(&quiet), byte(&ram)), Received::Missed);
        }
        assert_eq!(radio.read(now, RX_NEXT), Some(RX0 & RX_ADDRESS));
        assert!(!radio.line(now));
        assert_eq!(radio.next_event(now), Some(Tick(now.0 + 1)));
        assert!(radio.line(Tick(now.0 + 1)));
        assert_eq!(radio.read(Tick(now.0 + 1), EVENTS), Some(EVENT_BEACON));

        // Octet 1, bit 1 is AID 9, whether the bitmap starts at octet 0 or is cut to start there.
        for named in [beacon(0, &[0, 2], false), beacon(0, &[1, 2, 0], false)] {
            let mut ram = Ram::default();
            let mut radio = filtered(&mut ram);
            assert!(!ram.apply(radio.hear(now, heard(&named), byte(&ram))));
            assert_eq!(ram.bytes(BUF0 + RX_CTRL as u32, named.len()), named);
        }
        let mut ram = Ram::default();
        let mut radio = filtered(&mut ram);
        assert!(write(&mut radio, BEACON_FILTER, 0));
        assert!(!ram.apply(radio.hear(now, heard(&beacon(0, &[0], false)), byte(&ram))));
    }

    /// A Beacon the filter cannot judge ends the run by name: one with no TIM, and one whose TIM
    /// says group frames are held, which the filter might pass or drop.
    #[test]
    fn a_beacon_the_filter_cannot_judge_is_refused_by_name() {
        let mut ram = Ram::default();
        let mut radio = listening(&mut ram);
        filter(&mut radio, BSSID, BSSID_MASK, AP);
        assert!(write(&mut radio, BEACON_FILTER, BEACON_FILTER_ON));
        let map = ram.0.clone();
        let byte = move |at| map.get(&at).copied();
        let mut bare = management(8, [0xff; 6]);
        bare.truncate(24);
        bare.extend([0; 12]);
        bare.extend([0, 4]);
        bare.extend(*b"orbe");
        for (mpdu, setting) in [
            (bare, "beacon_filter_tim"),
            (beacon(0, &[0], true), "beacon_filter_group"),
        ] {
            assert_eq!(
                radio.hear(Tick(1_000), heard(&mpdu), &byte),
                Received::Refused(setting)
            );
        }
    }

    /// `rom_phy_xpd_rf` clears RFRX_PBUS and TXRF in `RTC_CNTL_ANA_CONF_REG` for modem sleep:
    /// the radio then hears on no channel, and a frame it would send ends the run by name. The
    /// PHY's wake powers them again and the radio hears on the channel it was on. A power-down
    /// while a frame is not done ends the run by name too.
    #[test]
    fn with_its_rf_powered_down_the_radio_hears_nothing_and_sends_nothing() {
        let mut radio = ready(24);
        assert_eq!(radio.tuned(), Some(1));
        assert_eq!(radio.power(Tick(0), 0x2040_0000), None);
        assert_eq!(radio.tuned(), None);
        let mut ram = Ram::default();
        ram.poke(DESCRIPTOR, &[0; 12]);
        let frame = [0u8; 24];
        assert_eq!(
            start(&mut radio, Tick(10), &frame, 24),
            Store::Refused("tx_rf_off")
        );
        let mut radio = ready(24);
        assert_eq!(radio.power(Tick(0), 0x2040_0000), None);
        assert_eq!(radio.power(Tick(0), RF_ON), None);
        assert_eq!(radio.tuned(), Some(1));
        let mut broadcast = frame;
        broadcast[4..10].fill(0xff);
        assert_eq!(start(&mut radio, Tick(10), &broadcast, 24), Store::Sent);
        assert_eq!(radio.power(Tick(20), 0x2040_0000), Some("rf_off_tx"));
        // Powered from boot's value, the radio hears nothing.
        let mut radio = Radio::new();
        assert!(write(&mut radio, FREQ, FREQ_START | (2 * 12)));
        assert_eq!(radio.power(Tick(0), 0x0040_0000), None);
        assert_eq!(radio.tuned(), None);

        // Each RF bit (31, 30, 29, 28, 27) gates RX and/or TX; clearing bit 22 (`SAR_I2C_PU`)
        // or setting any other bit is refused by name.
        for (bit, rx_off, tx_off) in [
            (PLL_I2C_PU, true, true),
            (CKGEN_I2C_PU, true, true),
            (RF_I2C_PU, true, true),
            (RFRX_PBUS_PU, true, false),
            (TXRF_I2C_PU, false, true),
        ] {
            let mut radio = ready(24);
            assert_eq!(radio.power(Tick(0), RF_ON & !bit), None, "{bit:#x}");
            assert_eq!(radio.tuned().is_none(), rx_off, "{bit:#x}");
            let sent = start(&mut radio, Tick(10), &broadcast, 24);
            if tx_off {
                assert_eq!(sent, Store::Refused("tx_rf_off"), "{bit:#x}");
            } else {
                assert_eq!(sent, Store::Sent, "{bit:#x}");
            }
            let mut busy = ready(24);
            assert_eq!(start(&mut busy, Tick(10), &broadcast, 24), Store::Sent);
            assert_eq!(busy.power(Tick(20), RF_ON & !bit), Some("rf_off_tx"));
        }
        let mut radio = powered();
        assert_eq!(radio.power(Tick(0), 0), Some("sar_i2c_off"));
        assert_eq!(
            radio.power(Tick(0), RF_ON & !SAR_I2C_PU),
            Some("sar_i2c_off")
        );
        for bad in [1 << 0, 1 << 20, 1 << 23, 1 << 26] {
            assert_eq!(
                radio.power(Tick(0), RF_ON | bad),
                Some("ana_conf"),
                "{bad:#x}"
            );
        }
    }

    /// Hardware TXOP (`mac_tx_set_txop_q` and `hal_mac_fill_hwtxop`) chains additional frames
    /// from `0x6003_4344..=0x6003_43f4` on SIFS spacing after each ACK and reports the count of
    /// earlier completed frames in bits 28 to 31 of `TX_STATUS`. An ACK timeout stops the burst
    /// and reports only the frames completed before the timed-out frame.
    #[test]
    fn hardware_txop_chains_frames_on_sifs_and_reports_completed_count() {
        let mut ram = Ram::default();
        let mut f0 = PROBE;
        f0[4..10].copy_from_slice(&AP);
        f0[22] = 0x10;
        let mut f1 = f0;
        f1[22] = 0x20;
        let mut f2 = f0;
        f2[22] = 0x30;
        let d0 = DESCRIPTOR;
        let d1 = DESCRIPTOR + 16;
        let d2 = DESCRIPTOR + 32;
        let b0 = BUFFER;
        let b1 = BUFFER + 0x100;
        let b2 = BUFFER + 0x200;
        let head = 1 << 31 | 1 << 30 | (49u32) << 12 | 0x50;
        for (d, b, f) in [(d0, b0, &f0), (d1, b1, &f1), (d2, b2, &f2)] {
            for (k, w) in [head, b, 0].into_iter().enumerate() {
                ram.poke(d + 4 * k as u32, &w.to_le_bytes());
            }
            ram.poke(b, f);
        }

        let setup = |extra: u32, txop_q: u32| {
            let mut radio = ready(49);
            let slot1 = TXOP_FIRST + 60 * txop_q;
            let slot2 = slot1 - 4;
            assert!(write(
                &mut radio,
                slot1,
                (49 << 20) | (d1 & PLCP0_DESCRIPTOR)
            ));
            assert!(write(
                &mut radio,
                slot2,
                (49 << 20) | (d2 & PLCP0_DESCRIPTOR)
            ));
            assert_eq!(
                radio.write(
                    Tick(0),
                    TXOP_CTRL,
                    (txop_q << TXOP_QUEUE_SHIFT) | (extra << TXOP_COUNT_SHIFT),
                    |_| None
                ),
                Store::Kept
            );
            radio
        };

        // Two frames, both ACKed: completed count is 1 (frame 0 finished before final frame 1).
        let mut radio = setup(1, 0);
        let map = ram.0.clone();
        assert_eq!(
            radio.write(Tick(0), PLCP0, 0xc060_0000 | (d0 & PLCP0_DESCRIPTOR), |a| {
                map.get(&a).copied()
            }),
            Store::Sent
        );
        let end0 = radio.end().unwrap();
        radio.report(end0);
        let aired0 = radio.take_aired().unwrap();
        assert_eq!(aired0.mpdu[22], 0x10);
        radio.acked(Ack::Acked);
        // Frame 1 starts SIFS after frame 0's ACK (SIFS + ACK + SIFS).
        let end1 = radio.end().unwrap();
        assert!(!radio.line(end0));
        radio.report(end1);
        let aired1 = radio.take_aired().unwrap();
        assert_eq!(aired1.mpdu[22], 0x20);
        assert_eq!(
            aired1.start,
            after(after(end0, Rate::Dsss1.acked_after()), SIFS_PS)
        );
        radio.acked(Ack::Acked);
        assert_eq!(radio.end(), None);
        let done = after(end1, Rate::Dsss1.acked_after());
        assert!(!radio.line(Tick(done.0 - 1)));
        assert!(radio.line(done));
        assert_eq!(radio.read(done, TX_STATUS), Some((1 << 28) | TX_SUCCESS));

        // Three frames, frame 0 ACKed, frame 1 times out: frame 2 never airs, completed = 1.
        let mut radio = setup(2, 2);
        assert_eq!(
            radio.write(Tick(0), PLCP0, 0xc060_0000 | (d0 & PLCP0_DESCRIPTOR), |a| {
                map.get(&a).copied()
            }),
            Store::Sent
        );
        let end0 = radio.end().unwrap();
        radio.report(end0);
        radio.acked(Ack::Acked);
        let end1 = radio.end().unwrap();
        radio.report(end1);
        radio.acked(Ack::None);
        assert_eq!(radio.end(), None, "frame 2 does not air after timeout");
        let done = after(end1, Rate::Dsss1.acked_after());
        assert!(radio.line(done));
        assert_eq!(
            radio.read(done, TX_STATUS),
            Some((1 << 28) | TX_ACK_TIMEOUT)
        );

        // Invalid TXOP queue 3 and unset TXOP_CTRL bits 30..31 are refused by name.
        let mut radio = setup(1, 3);
        assert_eq!(
            radio.write(Tick(0), PLCP0, 0xc060_0000 | (d0 & PLCP0_DESCRIPTOR), |a| {
                map.get(&a).copied()
            }),
            Store::Refused("txop_queue")
        );
        let mut radio = powered();
        assert_eq!(
            radio.write(Tick(0), TXOP_CTRL, 1 << 30, |_| None),
            Store::Refused("txop_ctrl")
        );
        assert_eq!(
            radio.write(Tick(0), TXOP_CTRL, 1 << 31, |_| None),
            Store::Refused("txop_ctrl")
        );
    }

    /// Unmodelled bits of `EVENTS_ENABLE`, `TBTT_CONTROL`, `BEACON_AID`, `0x6003_5024`,
    /// `0x6003_502c`, `0x6003_5058` and the coexistence PTI/timing words `0x6003_5080..=509c`
    /// are refused by name, while every store the blobs make is kept.
    #[test]
    fn unmodelled_mac_tsf_and_coex_bits_are_refused_by_name() {
        let kept = [
            (EVENTS_ENABLE, 0),
            (EVENTS_ENABLE, EVENT_TX_DONE | EVENT_RX),
            (EVENTS_ENABLE, 0x19a8_79e0),
            (EVENTS_ENABLE, 0x19a8_f9e0),
            (BEACON_AID, 0x07ff_ffff),
            (TBTT_WAKE_AHEAD, 0x0200_0bb8),
            (TSF_ACTIVE, 0),
            (TSF_ACTIVE, TSF_ACTIVE_CLEAR),
            (TSF_RTC_TICK, 0x0003_ffff),
            (COEX_BEACON_TIME, 0xffff),
            (COEX_PTI_CTRL, 2),
            (COEX_BEACON_PTI_CLEAR, 1),
            (COEX_BEACON_TIME_EXT, 0xff),
            (COEX_BEACON_PTI, 0xf),
            (COEX_DEFAULT_PTI, 0xff00),
            (COEX_WAIT_TIME, 0x0c80_0000),
        ];
        for (addr, value) in kept {
            let mut radio = powered();
            assert_eq!(
                radio.write(Tick(0), addr, value, |_| None),
                Store::Kept,
                "{addr:#x} {value:#x}"
            );
        }
        // `COEX_BEACON_PTI_CLEAR` is W1C and reads 0.
        let mut radio = powered();
        assert_eq!(
            radio.write(Tick(0), COEX_BEACON_PTI_CLEAR, 1, |_| None),
            Store::Kept
        );
        assert_eq!(radio.read(Tick(0), COEX_BEACON_PTI_CLEAR), Some(0));

        let refused = [
            (EVENTS_ENABLE, 1 << 2, "events_enable"),
            (EVENTS_ENABLE, 0x19a8_79e4, "events_enable"),
            (EVENTS_ENABLE, 1 << 5, "events_enable"),
            (EVENTS_ENABLE, 1 << 31, "events_enable"),
            (TBTT_CONTROL, 1 << 27, "tbtt_control"),
            (TBTT_CONTROL, 1 << 28, "tbtt_control"),
            (TBTT_CONTROL, TSF0_ON | (1 << 27), "tbtt_control"),
            (BEACON_AID, 1 << 27, "bssid_bits"),
            (BEACON_AID, 1 << 31, "bssid_bits"),
            (TBTT_WAKE_AHEAD, 1 << 16, "tbtt_wake_ahead"),
            (TBTT_WAKE_AHEAD, 1 << 26, "tbtt_wake_ahead"),
            (TSF_ACTIVE, 1 << 1, "tsf_active"),
            (TSF_ACTIVE, 1 << 2, "tsf_active"),
            (TSF_RTC_TICK, 1 << 18, "tsf_rtc_tick"),
            (COEX_BEACON_TIME, 1 << 16, "coex_beacon_time"),
            (COEX_PTI_CTRL, COEX_EXTERN, "extern_coex"),
            (COEX_PTI_CTRL, 1 << 2, "coex_pti"),
            (COEX_BEACON_PTI_CLEAR, 1 << 1, "coex_beacon_pti"),
            (COEX_BEACON_TIME_EXT, 1 << 8, "coex_beacon_time"),
            (COEX_BEACON_PTI, 1 << 4, "coex_beacon_pti"),
            (COEX_DEFAULT_PTI, 1 << 0, "coex_pti"),
            (COEX_DEFAULT_PTI, 1 << 16, "coex_pti"),
            (COEX_WAIT_TIME, 1 << 0, "coex_wait_time"),
        ];
        for (addr, value, name) in refused {
            let mut radio = powered();
            assert_eq!(
                radio.write(Tick(0), addr, value, |_| None),
                Store::Refused(name),
                "{addr:#x} {value:#x}"
            );
        }
    }
}
