//! The air between radio stations: the channel a station tunes, the rate a frame goes at and
//! how long it takes on the air, the frame itself, the signal a receiver hears it at, and whether
//! the receiver acknowledged it.
//!
//! Only the 2.4 GHz band at 20 MHz is modelled. Times follow IEEE 802.11-2020: clause 15 (DSSS),
//! clause 16 (HR/DSSS), clause 17 (OFDM), clause 18 (ERP) and clause 19 (HT).

use std::sync::Arc;

use crate::Time;

/// A 2.4 GHz channel, 1 to 14, 20 MHz wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Channel(u8);

impl Channel {
    /// Channel `number`, or `None` outside 1 to 14.
    pub const fn new(number: u8) -> Option<Channel> {
        match number {
            1..=14 => Some(Channel(number)),
            _ => None,
        }
    }

    pub const fn number(self) -> u8 {
        self.0
    }
}

/// The PLCP preamble of a 2, 5.5 or 11 Mb/s frame. A 1 Mb/s frame always takes the long one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preamble {
    /// 144 us of preamble and a 48 us header at 1 Mb/s, clause 15.
    Long,
    /// 72 us of preamble and a 24 us header at 2 Mb/s, clause 16.
    Short,
}

/// An ERP-OFDM rate, in Mb/s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ofdm {
    M6,
    M9,
    M12,
    M18,
    M24,
    M36,
    M48,
    M54,
}

impl Ofdm {
    /// Data bits per 4 us symbol, N_DBPS, at 20 MHz (clause 17).
    const fn bits_per_symbol(self) -> u64 {
        match self {
            Ofdm::M6 => 24,
            Ofdm::M9 => 36,
            Ofdm::M12 => 48,
            Ofdm::M18 => 72,
            Ofdm::M24 => 96,
            Ofdm::M36 => 144,
            Ofdm::M48 => 192,
            Ofdm::M54 => 216,
        }
    }
}

/// An HT modulation and coding scheme, 0 to 7: one spatial stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mcs(u8);

impl Mcs {
    /// MCS `index`, or `None` above 7.
    pub const fn new(index: u8) -> Option<Mcs> {
        match index {
            0..=7 => Some(Mcs(index)),
            _ => None,
        }
    }

    pub const fn index(self) -> u8 {
        self.0
    }

    /// Data bits per 4 us symbol, N_DBPS, at 20 MHz with one stream (clause 19).
    const fn bits_per_symbol(self) -> u64 {
        match self.0 {
            0 => 26,
            1 => 52,
            2 => 78,
            3 => 104,
            4 => 156,
            5 => 208,
            6 => 234,
            _ => 260,
        }
    }
}

/// The rate a frame goes at, with what sets its PLCP timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rate {
    /// 1 Mb/s DSSS, clause 15.
    Dsss1,
    /// 2 Mb/s DSSS, clauses 15 and 16.
    Dsss2(Preamble),
    /// 5.5 Mb/s CCK, clause 16.
    Cck5_5(Preamble),
    /// 11 Mb/s CCK, clause 16.
    Cck11(Preamble),
    /// ERP-OFDM, clause 18.
    Ofdm(Ofdm),
    /// HT-mixed format, clause 19: 20 MHz, BCC, the 800 ns guard interval.
    Ht(Mcs),
}

/// The signal extension after an ERP-OFDM or HT frame in the 2.4 GHz band (clauses 18 and 19).
const SIGNAL_EXTENSION_US: u64 = 6;
/// SERVICE and tail bits around an OFDM or HT frame's data (clauses 17 and 19).
const SERVICE_TAIL_BITS: u64 = 16 + 6;

impl Rate {
    /// The picoseconds a frame whose PSDU is `len` octets, its MPDU and the 4-octet FCS, takes on
    /// the air: TXTIME, with the signal extension of the 2.4 GHz band after an OFDM or HT frame.
    pub const fn airtime(self, len: usize) -> u64 {
        let bits = 8 * len as u64;
        let us = match self {
            // TXTIME = preamble + PLCP header + ceil(8 LENGTH / rate) (15.3.4, 16.3.4).
            Rate::Dsss1 => preamble_us(Preamble::Long) + bits,
            Rate::Dsss2(p) => preamble_us(p) + bits.div_ceil(2),
            Rate::Cck5_5(p) => preamble_us(p) + (2 * bits).div_ceil(11),
            Rate::Cck11(p) => preamble_us(p) + bits.div_ceil(11),
            // 16 us preamble, 4 us SIGNAL, 4 us symbols (17.4.3, 18.5.2).
            Rate::Ofdm(r) => {
                16 + 4
                    + 4 * (SERVICE_TAIL_BITS + bits).div_ceil(r.bits_per_symbol())
                    + SIGNAL_EXTENSION_US
            }
            // L-STF and L-LTF 16 us, L-SIG 4 us, HT-SIG 8 us, HT-STF 4 us and one HT-LTF 4 us
            // (19.4.3).
            Rate::Ht(m) => {
                16 + 4
                    + 8
                    + 4
                    + 4
                    + 4 * (SERVICE_TAIL_BITS + bits).div_ceil(m.bits_per_symbol())
                    + SIGNAL_EXTENSION_US
            }
        };
        us * 1_000_000
    }
}

const fn preamble_us(preamble: Preamble) -> u64 {
    match preamble {
        Preamble::Long => 144 + 48,
        Preamble::Short => 72 + 24,
    }
}

/// A frame on the air. `mpdu` is its header and body as a sniffer sees them, ciphertext
/// included, without the FCS. Copies and receivers share the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirFrame {
    /// The first preamble symbol.
    pub start: Time,
    /// Where its [`airtime`](Rate::airtime) ends.
    pub end: Time,
    pub channel: Channel,
    pub rate: Rate,
    pub mpdu: Arc<[u8]>,
}

/// How a receiver hears a frame, which the air adds for each receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Signal {
    pub rssi_dbm: i8,
    pub noise_dbm: i8,
}

/// Whether a frame was acknowledged. The acknowledgement is an answer, not a frame on the air.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ack {
    Acked,
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Airtime in whole microseconds, which every case here is.
    fn us(rate: Rate, len: usize) -> u64 {
        let ps = rate.airtime(len);
        assert_eq!(ps % 1_000_000, 0);
        ps / 1_000_000
    }

    #[test]
    fn channels_run_from_1_to_14() {
        assert_eq!(Channel::new(0), None);
        assert_eq!(Channel::new(1).map(Channel::number), Some(1));
        assert_eq!(Channel::new(14).map(Channel::number), Some(14));
        assert_eq!(Channel::new(15), None);
        assert_eq!(Mcs::new(7).map(Mcs::index), Some(7));
        assert_eq!(Mcs::new(8), None);
    }

    // ns-3 src/wifi/test/tx-duration-test.cc, TxDurationTest: a 14-octet ACK at 1 Mb/s is 304 us,
    // and 1023 to 1026 octets are 8184 to 8208 us after the 192 us long PLCP.
    #[test]
    fn dsss_1_mbps() {
        assert_eq!(us(Rate::Dsss1, 14), 304);
        assert_eq!(us(Rate::Dsss1, 1023), 8184 + 192);
        assert_eq!(us(Rate::Dsss1, 1026), 8208 + 192);
    }

    // ns-3 TxDurationTest, after IEEE Std 802.11-2007 Table 18-2, "Example of LENGTH
    // calculations for CCK": 1023 to 1026 octets at 11 Mb/s are 744 to 747 us after the PLCP,
    // 192 us long or 96 us short. A 14-octet ACK at 11 Mb/s with the long preamble is 203 us.
    #[test]
    fn cck_11_mbps() {
        for (len, data) in [(1023, 744), (1024, 745), (1025, 746), (1026, 747)] {
            assert_eq!(us(Rate::Cck11(Preamble::Long), len), data + 192);
            assert_eq!(us(Rate::Cck11(Preamble::Short), len), data + 96);
        }
        assert_eq!(us(Rate::Cck11(Preamble::Long), 14), 203);
    }

    // sarwiki.informatik.hu-berlin.de, "Packet transmission time in 802.11", 802.11g at 6 Mb/s:
    // a 1528-octet PSDU (28 octets of header and FCS around 1500) is 16 us of preamble, 4 us of
    // PLCP header, 2044 us of data and 6 us of signal extension, and a 14-octet ACK is
    // 16 + 4 + 24 + 6 us.
    #[test]
    fn ofdm_6_mbps() {
        assert_eq!(us(Rate::Ofdm(Ofdm::M6), 1528), 16 + 4 + 2044 + 6);
        assert_eq!(us(Rate::Ofdm(Ofdm::M6), 14), 16 + 4 + 24 + 6);
    }

    // ns-3 TxDurationTest, "802.11g durations are same as 802.11a durations but with 6 us signal
    // extension": 1536, 76 and 14 octets at ERP-OFDM 54 Mb/s are 254, 38 and 30 us.
    #[test]
    fn ofdm_54_mbps() {
        assert_eq!(us(Rate::Ofdm(Ofdm::M54), 1536), 254);
        assert_eq!(us(Rate::Ofdm(Ofdm::M54), 76), 38);
        assert_eq!(us(Rate::Ofdm(Ofdm::M54), 14), 30);
    }

    // ns-3 TxDurationTest, "802.11n durations": HT-mixed MCS 7, 20 MHz, 800 ns guard interval,
    // 1536, 76 and 14 octets are 228, 48 and 40 us at 5 GHz, and the 2.4 GHz band adds 6 us.
    #[test]
    fn ht_mcs_7() {
        let mcs7 = Rate::Ht(Mcs::new(7).unwrap());
        assert_eq!(us(mcs7, 1536), 228 + 6);
        assert_eq!(us(mcs7, 76), 48 + 6);
        assert_eq!(us(mcs7, 14), 40 + 6);
    }
}
