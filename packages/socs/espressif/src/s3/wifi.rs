//! The Wi-Fi radio's registers, which a chip built with a radio keeps.
//!
//! IDF documents none of them. The model keeps the words a stock Arduino-ESP32 3.3.12 station
//! sketch touches through `esp_wifi_init`, `esp_wifi_start`, a scan and a join: the PHY's in FE2,
//! FE, the analog I2C master past the words [`Analog`](super::analog::Analog) keeps, BT, the range
//! below NRX that IDF names no block for, NRX and BB, and the MAC's in RADIO. Each reads the word
//! last written, 0 before the first write. Four reads also carry a done bit that the driver
//! polls for, in [`STATUS`] and [`HAL_INIT`]. Each of those values, and each 0 before a write, was
//! chosen so the driver proceeds, and is not yet read from hardware.
//!
//! The words move nothing: no frame goes on the air, no interrupt is raised and no MAC timer
//! counts. Any other access in these blocks is refused or listed as on a chip with no radio.

/// The words kept, sorted.
const WORDS: [u32; 262] = [
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

/// The radio's words, in [`WORDS`] order.
#[derive(Clone, Debug)]
pub(crate) struct Radio {
    words: [u32; WORDS.len()],
}

impl Radio {
    pub(crate) fn new() -> Radio {
        Radio {
            words: [0; WORDS.len()],
        }
    }

    fn slot(addr: u32) -> Option<usize> {
        WORDS.binary_search(&addr).ok()
    }

    /// The word at `addr`, a multiple of 4, when the radio keeps it.
    pub(crate) fn read(&self, addr: u32) -> Option<u32> {
        let word = self.words[Self::slot(addr)?];
        let status = STATUS
            .iter()
            .find(|&&(at, _)| at == addr)
            .map_or(0, |&(_, bits)| bits);
        let done = if addr == HAL_INIT && word & HAL_INIT_START != 0 {
            HAL_INIT_DONE
        } else {
            0
        };
        Some(word | status | done)
    }

    /// Keeps `value` at `addr`, a multiple of 4, and says whether the radio keeps that word.
    pub(crate) fn write(&mut self, addr: u32, value: u32) -> bool {
        let Some(slot) = Self::slot(addr) else {
            return false;
        };
        self.words[slot] = value;
        true
    }
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

    #[test]
    fn a_word_reads_what_was_written_and_0_before() {
        let mut radio = Radio::new();
        assert_eq!(radio.read(0x6003_3000), Some(0));
        assert!(radio.write(0x6003_3000, 0x1234_5678));
        assert_eq!(radio.read(0x6003_3000), Some(0x1234_5678));
        assert_eq!(radio.read(0x6000_6000), Some(0));
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
            assert_eq!(radio.read(addr), None, "{addr:#x}");
            assert!(!radio.write(addr, 1), "{addr:#x}");
        }
    }

    #[test]
    fn each_polled_word_reads_its_done_bits() {
        let mut radio = Radio::new();
        assert_eq!(radio.read(0x6000_e050), Some(0x0700_0000));
        assert!(radio.write(0x6000_e050, 0x2));
        assert_eq!(radio.read(0x6000_e050), Some(0x0700_0002));
        assert_eq!(radio.read(0x6000_e04c), Some(0x0100_0000));
        assert_eq!(radio.read(0x6000_6174), Some(0x0001_0000));
    }

    #[test]
    fn hal_init_reads_done_once_it_has_set_start() {
        let mut radio = Radio::new();
        assert_eq!(radio.read(HAL_INIT), Some(0));
        assert!(radio.write(HAL_INIT, HAL_INIT_START));
        assert_eq!(radio.read(HAL_INIT), Some(HAL_INIT_START | HAL_INIT_DONE));
        assert!(radio.write(HAL_INIT, 0));
        assert_eq!(radio.read(HAL_INIT), Some(0));
    }
}
