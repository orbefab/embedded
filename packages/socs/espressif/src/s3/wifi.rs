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
//! `ic_update_sta_tsf`, `pm_update_next_tbtt` and the ROM's `coex_hw_timer_*`, and each meaning
//! is a guess: the MAC encrypts and decrypts CCMP-128 with the key table, as [`keys`] says, a
//! TSF load sets a timer and a TBTT latch sets a TBTT's start ([`TSF_CONTROL`]), and the MAC's
//! clock counts microseconds ([`MAC_CLOCK`]). What the model does not run is refused by name: a
//! cipher other than CCMP-128, a protected Management frame, a coexistence timer turned on or
//! forced (`coex_timer`, `coex_force`), the beacon filter turned on (`beacon_filter`), the
//! interrupt of a TBTT other than 0 or of a TSF timer's alarm (`tbtt_intr`, `tsf_timer`), any bit
//! of the TSF and TBTT words that no blob function sets (`tsf_control`, `tbtt_control`,
//! `tbtt_start`, `pwr_intr`), any store to the key table, its crypto words, PLCP1, the beacon
//! filter or a coexistence timer that no blob function makes (`key_entry`, `key_valid`,
//! `crypto_control`, `crypto_init`, `tx_plcp1`, `beacon_filter_bits`, `coex_timer_bits`), and an
//! armed TBTT, where the run ends as `tbtt` ([`Radio::tbtt`]). The driver itself numbers each
//! frame and sets its Retry bit when it sends it again after an ACK timeout, so the MAC sends the
//! buffer as it is.
//!
//! Any other access in these blocks is refused or listed as on a chip with no radio.

use std::sync::Arc;

use orbe_model::air::{Ack, Mcs, Ofdm, Preamble, Rate};

use super::clock::{TICK_HZ, Tick};
use super::keys::{self, Opened, Sealed};

/// The words kept, sorted.
const WORDS: [u32; 289] = [
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
    0x6003_50b8,
    0x6003_50c0,
    0x6003_5110,
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
/// `lmacProcessAckTimeout`. Every other bit reads 0, a guess.
const TX_STATUS: u32 = 0x6003_4320;
const TX_SUCCESS: u32 = 0;
const TX_ACK_TIMEOUT: u32 = 5 << 12;
/// The outcome of a protected frame whose key entry is not in use: outcome 4, which
/// `lmacProcessTxComplete` hands to `lmacProcessTxError` with bits 0 to 7, and 192 there, which
/// `lmacProcessTxError` hands to `lmacProcessTxseckiderr`. That the MAC reports it for an entry
/// not in use, sends nothing and is done when the frame would have started is a guess.
const TX_KEY_ERROR: u32 = 4 << 12 | 0xc0;
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

/// The power interrupt's enable word, `ETS_WIFI_PWR_INTR_SOURCE` (2 in IDF v5.5.5,
/// `components/soc/esp32s3/include/soc/interrupts.h`). `tsf_hal_set_tbtt_intr_enable` sets bit
/// `4 - n` for TBTT `n`, and `hal_enable_sta_tbtt` bit 4 for TBTT 0. `tsf_hal_set_timer_intr_enable`
/// and `hal_enable_tsf_timer` set bit `8 - n` for timer `n`'s alarm. Each first sets the same bit
/// of `0x6003_511c`, which `pwr_hal_clear_intr_status` writes too, the interrupt's clear word,
/// which with no interrupt raised clears nothing. While bit 4 is set and TBTT 0 is on
/// ([`TBTT_CONTROL`]), TBTT 0 would raise the interrupt, which the model does not run: the chip
/// ends the run by name, `tbtt`, where it first would, as [`Radio::tbtt`] reckons it. A store
/// that sets the bit of another TBTT is refused by name, `tbtt_intr`, and one of a timer's
/// alarm, whose target words the model does not keep, `tsf_timer`. No function sets bit 0 or
/// bits 9 to 31, and a store that does is refused by name, `pwr_intr`.
const PWR_ENABLE: u32 = 0x6003_5110;
const PWR_TBTT: u32 = 1 << 4;
const PWR_TBTT_OTHERS: u32 = 0b1110;
const PWR_TSF_TIMERS: u32 = 0xf << 5;
const PWR_BITS: u32 = 0x1fe;
/// The start of a TBTT, bits 0 to 25 of its TSF count, which a latch bit of [`TSF_CONTROL`] makes
/// TBTT `n`'s. `tsf_hal_set_tbtt_start_time` keeps bits 26 to 31 as they read, and no function
/// sets them: a store that does is refused by name, `tbtt_start`.
const TBTT: u32 = 0x6003_5020;
const TBTT_FIELD: u32 = (1 << 26) - 1;
/// TBTT 0's control word, the first of the four TBTT and TSF groups 12 bytes apart
/// (`tsf_hal_set_tbtt_enable` and `tsf_hal_set_tsf_enable` pick `0x6003_5028`, `5034`, `5040` or
/// `504c`). The model keeps only TBTT 0's, whose bits the blobs set as:
/// - Bit 26 turns TBTT 0 on (`tsf_hal_set_tbtt_enable`, `hal_enable_sta_tbtt`).
/// - Bit 29 lets it wake the SoC (`tsf_hal_set_tbtt_soc_wakeup_enable`, `hal_set_sta_tsf_wakeup`).
///   The model has no light sleep for it to wake the SoC from, so the bit does nothing.
/// - Bits 27, 28 and 31 turn TSF timer 0 on (`hal_enable_sta_tsf`, `tsf_hal_set_tsf_enable`,
///   `hal_mac_tsf_reset`). Every timer counts whatever they say, a guess.
///
/// No function sets bits 0 to 25 or bit 30 of this word, and a store that does is refused by
/// name, `tbtt_control`.
const TBTT_CONTROL: u32 = 0x6003_5028;
const TBTT_ON: u32 = 1 << 26;
const TBTT_CONTROL_BITS: u32 = 0xbc00_0000;
/// TBTT 0's timing: bits 0 to 15 the beacon interval in TU, and bits 16 to 31 how many
/// microseconds before the TBTT the interrupt comes, as `hal_set_sta_tbtt`,
/// `hal_set_sta_tbtt_interval`, `hal_set_sta_tbtt_ahead_time`, `tsf_hal_set_tbtt_interval` and
/// `tsf_hal_set_tbtt_early_time` write them.
const TBTT_TIMING: u32 = 0x6003_5030;
/// A time unit, 1024 microseconds (IEEE 802.11-2020, 3.1).
const TU: u64 = 1024;

/// The station's beacon filter, which `hal_enable_sta_beacon_filter` turns on by setting bits 0
/// to 2, with event bit 15 in [`EVENTS_ENABLE`], and `hal_disable_sta_beacon_filter` off. The
/// filter, which by `hal_beacon_ie_crc_get` and `hal_beacon_ie_crc_set` compares a CRC of each
/// beacon's elements, is not modelled: a store that turns it on is refused by name,
/// `beacon_filter`. No function sets bits 3 to 31, and a store that does is refused by name,
/// `beacon_filter_bits`.
const BEACON_FILTER: u32 = 0x6003_30a0;
const BEACON_FILTER_ON: u32 = 0x7;

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
    /// The address of the next receive descriptor, 0 once the list has run out.
    rx_next: u32,
    /// The address of the last receive descriptor filled, 0 before the first.
    rx_last: u32,
    /// When [`EVENT_RX`] rises for the frames received and not yet raised.
    rx_at: Option<Tick>,
    /// The TSF the last latch copied, which [`TSF_LOW`] and [`TSF_HIGH`] read.
    tsf_latched: u64,
    /// What each TSF timer adds to [`tsf`], from its last load.
    tsf_offset: [u64; TSF_TIMERS],
    /// Each TBTT's start, from its last latch.
    tbtt_start: [u32; TSF_TIMERS],
    /// The key table's words.
    keys: [u32; KEY_TABLE],
    /// While TBTT 0 would raise the power interrupt, when its settings last changed: the store
    /// that armed it, or a later one that latched its start, set its timing or loaded timer 0.
    tbtt_since: Option<Tick>,
}

/// A frame the radio hears on the channel it is tuned to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Heard<'a> {
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

/// A frame that is not done yet.
#[derive(Clone, Copy, Debug)]
struct Pending {
    queue: u32,
    /// When the MAC is done with it.
    at: Tick,
    /// Whether it waits for an ACK.
    waits: bool,
    acked: bool,
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
            events: 0,
            done: 0,
            status: [0; QUEUES as usize],
            rx_next: 0,
            rx_last: 0,
            rx_at: None,
            tsf_latched: 0,
            tsf_offset: [0; TSF_TIMERS],
            tbtt_start: [0; TSF_TIMERS],
            keys: [0; KEY_TABLE],
            tbtt_since: None,
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
        if let Some(at) = key_slot(addr) {
            return Some(self.keys[at]);
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
            TX_DONE => self.done,
            TX_DONE_CLEAR => 0,
            RX_RELOAD => word & !RX_RELOAD_START,
            RX_NEXT => self.rx_next & RX_ADDRESS,
            RX_LAST => self.rx_last & RX_ADDRESS,
            MAC_CLOCK => tsf(now) as u32,
            TSF_LOW => self.tsf_latched as u32,
            TSF_HIGH => (self.tsf_latched >> 32) as u32,
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
            TX_DONE_CLEAR => self.done &= !value,
            FREQ if starts(FREQ_START) => self.channel = channel(value & FREQ_HALF_MHZ),
            RX_BASE => self.rx_next = value,
            RX_RELOAD if value & RX_RELOAD_START != 0 && self.rx_next == 0 && self.rx_last != 0 => {
                self.rx_next = read_word(self.rx_last + 8, &byte).unwrap_or(0);
            }
            BEACON_FILTER if value & !BEACON_FILTER_ON != 0 => {
                return Store::Refused("beacon_filter_bits");
            }
            BEACON_FILTER if value & BEACON_FILTER_ON != 0 => {
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
            PWR_ENABLE if value & !PWR_BITS != 0 => return Store::Refused("pwr_intr"),
            PWR_ENABLE if value & PWR_TBTT_OTHERS != 0 => return Store::Refused("tbtt_intr"),
            PWR_ENABLE if value & PWR_TSF_TIMERS != 0 => return Store::Refused("tsf_timer"),
            TBTT_CONTROL if value & !TBTT_CONTROL_BITS != 0 => {
                return Store::Refused("tbtt_control");
            }
            TBTT if value & !TBTT_FIELD != 0 => return Store::Refused("tbtt_start"),
            TSF_CONTROL if value & !TSF_CONTROL_BITS != 0 => return Store::Refused("tsf_control"),
            TSF_CONTROL => {
                let loaded = (u64::from(self.word(TSF_LOAD_HIGH)) << 32)
                    | u64::from(self.word(TSF_LOAD_LOW));
                for n in 0..TSF_TIMERS {
                    if value & 1 << (TSF_LOAD_SHIFT + n as u32) != 0 {
                        self.tsf_offset[n] = loaded.wrapping_sub(tsf(now));
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
        let on = self.word(TBTT_CONTROL) & TBTT_ON != 0 && self.word(PWR_ENABLE) & PWR_TBTT != 0;
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

    /// Puts queue `queue`'s frame on the air, from the store at `now` of `plcp0`, and whether
    /// it did: a protected frame without its key entry is done without going on the air.
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
        let channel = self.channel.ok_or("tx_channel")?;
        let plcp1 = self.word(PLCP1 - STRIDE * queue);
        let code = (plcp1 >> PLCP1_RATE_SHIFT) & PLCP1_RATE;
        if (plcp1 & PLCP1_HT != 0) != (code >= HT_RATES) {
            return Err("tx_rate");
        }
        let rate = rate(code).ok_or("tx_rate")?;
        let length = (plcp1 & PLCP1_LENGTH) as usize;
        let psdu = gather(SRAM_DATA | plcp0 & PLCP0_DESCRIPTOR, byte).ok_or("tx_descriptor")?;
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
                Sealed::NoKey => {
                    self.pending = Some(Pending {
                        queue,
                        at: Tick(now.0 + DIFS.0),
                        waits: false,
                        acked: false,
                        outcome: Some(TX_KEY_ERROR),
                    });
                    return Ok(false);
                }
            }
        }
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
            outcome: None,
        });
        Ok(true)
    }

    /// When the TBTT the driver armed would first raise the power interrupt: the first TBTT at
    /// least the ahead time after the store that armed it or last changed its settings, less
    /// that time. The TBTTs are the counts of TSF timer 0 that agree with TBTT 0's latched start
    /// modulo the beacon interval, as a TBTT falls where the TSF is a whole number of beacon
    /// intervals (IEEE 802.11-2020, 11.1.3.2). That the MAC compares timer 0 this way, raises
    /// the interrupt the ahead time before, and after a change of the settings waits for the
    /// next such count, is a guess. `None` while the driver has not armed it.
    pub(crate) fn tbtt(&self) -> Option<Tick> {
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

    /// TSF timer `n`'s count at `now`.
    fn timer(&self, now: Tick, n: usize) -> u64 {
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
    /// 29; the MCS of an HT frame in word 1; the time in microseconds in word 3; the noise floor
    /// in word 5; the length with the FCS in word 11, with `rx_state` 0. The driver writes the
    /// channel into word 2 itself. Every other bit is 0. Which words the MAC fills, and the time's
    /// origin, which is the chip's start, are guesses.
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
        words[3] = tsf(now) as u32;
        words[5] = u32::from(frame.noise_dbm as u8);
        words[11] = psdu as u32;
        words
    }

    /// Finishes the frame that is done by `now`, and raises the receive event that is due.
    fn settle(&mut self, now: Tick) {
        if self.rx_at.is_some_and(|at| at <= now) {
            self.rx_at = None;
            self.events |= EVENT_RX;
        }
        let Some(pending) = self.pending.filter(|pending| pending.at <= now) else {
            return;
        };
        self.pending = None;
        self.status[pending.queue as usize] = match pending.outcome {
            Some(outcome) => outcome,
            None if pending.waits && !pending.acked => TX_ACK_TIMEOUT,
            None => TX_SUCCESS,
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
        [self.pending.map(|pending| pending.at), self.rx_at]
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
        let mut radio = Radio::new();
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
        let now = Tick(480_000 * 579);
        let ack = ram.apply(radio.hear(now, heard(&mpdu), |at| ram.0.get(&at).copied()));
        assert!(ack);

        let length = RX_CTRL as u32 + mpdu.len() as u32 + 4;
        assert_eq!(ram.word(RX0), EOF | length << 12 | SIZE);
        let words: Vec<u32> = (0..12).map(|k| ram.word(BUF0 + 4 * k)).collect();
        // RSSI -50, rate 0 (1 Mb/s), non-HT, slot 0 matched.
        assert_eq!(words[0], 0x1000_00ce);
        assert_eq!(words[3], 579_000, "the time in microseconds");
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
        assert!(!radio.line(next));
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
    /// or 1 to count on from the loaded count.
    #[test]
    fn a_latch_reads_the_tsf_and_a_load_sets_it() {
        let mut radio = Radio::new();
        let now = Tick(480 * ((5 << 32) + 7));
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
        // `hal_mac_tsf_reset`: 0 into timers 0 and 1.
        assert!(write(&mut radio, TSF_LOAD_LOW, 0));
        assert!(write(&mut radio, TSF_LOAD_HIGH, 0));
        assert_eq!(radio.write(later, TSF_CONTROL, 0x30, |_| None), Store::Kept);
        assert_eq!(radio.write(later, TSF_CONTROL, 2, |_| None), Store::Kept);
        assert_eq!(radio.read(Tick(later.0 + 480), TSF_LOW), Some(0));
        assert_eq!(radio.read(later, TSF_HIGH), Some(0));
    }

    /// `tsf_hal_set_counter_value` loads timer `n` with bit `4 + n`, timers 2 and 3 too, and
    /// each counts on from its own load.
    #[test]
    fn a_load_sets_each_of_the_four_timers() {
        let mut radio = Radio::new();
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
            let mut radio = Radio::new();
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
            let mut radio = Radio::new();
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
        let mut radio = Radio::new();
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
        let mut radio = Radio::new();
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
    fn a_coexistence_timer_or_the_beacon_filter_turned_on_is_refused() {
        for timer in [COEX_TIMERS, COEX_TIMERS + 16, COEX_TIMERS + 24] {
            for (value, store) in [
                (0, Store::Kept),
                (0x4fff_ffff, Store::Kept),
                (COEX_TIMER_ON, Store::Refused("coex_timer")),
                (COEX_TIMER_FORCE, Store::Refused("coex_force")),
                (1 << 29, Store::Refused("coex_timer_bits")),
            ] {
                let mut radio = Radio::new();
                assert_eq!(
                    radio.write(Tick(0), timer, value, |_| None),
                    store,
                    "{timer:#x} {value:#x}"
                );
            }
        }
        for (value, store) in [
            (0, Store::Kept),
            (7, Store::Refused("beacon_filter")),
            (1 << 3, Store::Refused("beacon_filter_bits")),
            (1 << 31, Store::Refused("beacon_filter_bits")),
        ] {
            let mut radio = Radio::new();
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
            let mut radio = Radio::new();
            assert_eq!(
                radio.write(Tick(0), PLCP1, value, |_| None),
                Store::Kept,
                "{value:#x}"
            );
        }
        for value in [1 << 26, 1 << 27, 1 << 28, 1 << 30, 1 << 31, PLCP1_HT | 0x31] {
            let mut radio = Radio::new();
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
        let mut radio = Radio::new();
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
            let mut radio = Radio::new();
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
        let mut radio = Radio::new();
        let at = |us: u64| Tick(480 * us);
        assert!(write(&mut radio, TBTT_TIMING, 10 << 16 | 1));
        assert!(write(&mut radio, TBTT_CONTROL, TBTT_ON));
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
}
