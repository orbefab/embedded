//! When the access point may send: the interframe spaces, the acknowledgement it waits for, and
//! the beacon interval.

use orbe_model::Time;
use orbe_model::air::{Ofdm, Preamble, Rate};

pub(crate) const US: u64 = 1_000_000;
const MS: u64 = 1000 * US;

/// aSIFSTime of the ERP PHY, 10 us (clause 18).
pub(crate) const SIFS: u64 = 10 * US;
/// aSlotTime of the ERP PHY with short slots, 9 us (clause 18), which the access point's
/// Capability Information advertises.
const SLOT: u64 = 9 * US;
/// DIFS, aSIFSTime plus two slots (10.3.2.3). The access point waits it after the medium goes
/// idle, with no random backoff, so its sends are fixed in time.
pub(crate) const DIFS: u64 = SIFS + 2 * SLOT;

/// A time unit, 1024 us (3.1).
const TU: u64 = 1024 * US;
/// The Beacon Interval field, in TU (9.4.1.3).
pub(crate) const BEACON_INTERVAL_TU: u16 = 100;
pub(crate) const BEACON_INTERVAL: u64 = BEACON_INTERVAL_TU as u64 * TU;
/// Every beacon is a DTIM, as with the access point in Wireshark's `wpa-Induction.pcap`.
pub(crate) const DTIM_PERIOD: u8 = 1;

/// When the access point sends an EAPOL-Key message again that has no answer: 100 ms after its
/// first send and 1 s after each later one. After its fourth send it gives up. These are
/// hostapd's defaults, `eapol_key_timeout_first`, `eapol_key_timeout_subseq` and
/// `wpa_pairwise_update_count`.
pub(crate) const EAPOL_FIRST: u64 = 100 * MS;
pub(crate) const EAPOL_NEXT: u64 = 1000 * MS;
pub(crate) const EAPOL_SENDS: u8 = 4;

/// The lowest basic rate, 1 Mb/s with the long preamble, which every station in the BSS takes.
/// The access point sends at it what carries no MSDU to one station: Beacons, Probe Responses
/// and every other Management frame, group frames and Null frames, and also EAPOL-Key frames,
/// as Linux mac80211 sends a frame of the control port protocol at its lowest basic rate
/// (`IEEE80211_TX_CTL_USE_MINRATE`). The access point of Wireshark's `wpa-Induction.pcap`, a
/// 2005 Linksys, sent its EAPOL-Key frames at 54 Mb/s instead.
pub(crate) const BASIC: Rate = Rate::Dsss1;

/// The first target beacon transmission time after `at`, or `None` past the end of [`Time`].
pub(crate) fn next_tbtt(at: Time) -> Option<Time> {
    (at.as_ps() / BEACON_INTERVAL + 1)
        .checked_mul(BEACON_INTERVAL)
        .map(Time::from_ps)
}

/// The rate the access point sends a station its Data frames with a body at: the fastest of
/// [`element::RATES`](crate::element::RATES) and
/// [`element::EXTENDED`](crate::element::EXTENDED) that the station's Supported Rates and
/// Extended Supported Rates also name, given as `rates`, in 500 kb/s, the basic bit masked
/// off. The long preamble goes with a DSSS or CCK rate, since the access point's Capability
/// Information does not offer Short Preamble. With no rate in common it is [`BASIC`], which a
/// station that joined takes.
pub(crate) fn data_rate(rates: impl Iterator<Item = u8>) -> Rate {
    let ours = |rate: &u8| {
        let mut all = crate::element::RATES
            .iter()
            .chain(&crate::element::EXTENDED);
        all.any(|r| r & 0x7f == *rate)
    };
    // A rate in 500 kb/s orders as its speed.
    match rates.filter(ours).max() {
        Some(4) => Rate::Dsss2(Preamble::Long),
        Some(11) => Rate::Cck5_5(Preamble::Long),
        Some(22) => Rate::Cck11(Preamble::Long),
        Some(12) => Rate::Ofdm(Ofdm::M6),
        Some(18) => Rate::Ofdm(Ofdm::M9),
        Some(24) => Rate::Ofdm(Ofdm::M12),
        Some(36) => Rate::Ofdm(Ofdm::M18),
        Some(48) => Rate::Ofdm(Ofdm::M24),
        Some(72) => Rate::Ofdm(Ofdm::M36),
        Some(96) => Rate::Ofdm(Ofdm::M48),
        Some(108) => Rate::Ofdm(Ofdm::M54),
        _ => BASIC,
    }
}

/// An ACK frame's PSDU: Frame Control, Duration, RA and the FCS (9.3.1.3).
const ACK_LEN: usize = 14;

/// The rate of the ACK that answers a frame at `rate` (10.6.6.5): the highest basic rate at or
/// below it in its own modulation. The basic rates are every DSSS and CCK rate, so a DSSS or
/// CCK frame's ACK goes at the frame's own rate and preamble. No OFDM rate is basic, so an
/// OFDM or HT frame's ACK takes the highest mandatory OFDM rate, 6, 12 or 24 Mb/s, at or below
/// the frame's rate, or below its non-HT reference rate for HT.
fn ack_rate(rate: Rate) -> Rate {
    let mbps = match rate {
        Rate::Dsss1 | Rate::Dsss2(_) | Rate::Cck5_5(_) | Rate::Cck11(_) => return rate,
        Rate::Ofdm(Ofdm::M6 | Ofdm::M9) => 6,
        Rate::Ofdm(Ofdm::M12 | Ofdm::M18) => 12,
        Rate::Ofdm(_) => 24,
        // MCS 0 is 6.5 Mb/s, 1 is 13, 2 is 19.5, and 3 and up are 26 or more.
        Rate::Ht(mcs) => match mcs.index() {
            0 => 6,
            1 | 2 => 12,
            _ => 24,
        },
    };
    Rate::Ofdm(match mbps {
        6 => Ofdm::M6,
        12 => Ofdm::M12,
        _ => Ofdm::M24,
    })
}

/// The picoseconds from the end of a frame at `rate` to the end of its ACK: SIFS and the ACK.
pub(crate) fn acked_after(rate: Rate) -> u64 {
    SIFS + ack_rate(rate).airtime(ACK_LEN)
}

/// The Duration field of an individually addressed frame the access point sends at `rate`:
/// the microseconds of SIFS and the ACK after it, rounded up (9.2.4.2).
pub(crate) fn duration_us(rate: Rate) -> u16 {
    acked_after(rate).div_ceil(US) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbe_model::air::Mcs;

    // Wireshark's wpa-Induction.pcap: every individually addressed Management frame the access
    // point sends at 1 Mb/s has Duration 0x013a, 314 us: SIFS and a 304 us ACK.
    #[test]
    fn a_frame_at_1_mbps_reserves_its_ack() {
        assert_eq!(duration_us(BASIC), 314);
    }

    // At 54 Mb/s the ACK goes at 24 Mb/s: 20 us of preamble and SIGNAL, two symbols and the
    // 6 us signal extension. Every Data frame the capture's access point sends at 36, 48 or 54
    // Mb/s has Duration 44.
    #[test]
    fn a_frame_at_54_mbps_reserves_its_ack_at_24() {
        assert_eq!(duration_us(Rate::Ofdm(Ofdm::M54)), 10 + 34);
    }

    #[test]
    fn data_goes_at_the_fastest_rate_both_ends_name() {
        // The capture's station: every ERP rate, the basic bit on the DSSS and CCK ones.
        let induction = [
            0x82, 0x84, 0x8b, 0x96, 0x24, 0x30, 0x48, 0x6c, 0x0c, 0x12, 0x18, 0x60,
        ];
        let masked = |rates: &[u8]| rates.iter().map(|r| r & 0x7f).collect::<Vec<_>>();
        assert_eq!(
            data_rate(masked(&induction).into_iter()),
            Rate::Ofdm(Ofdm::M54)
        );
        assert_eq!(data_rate([2, 4].into_iter()), Rate::Dsss2(Preamble::Long));
        assert_eq!(
            data_rate([2, 4, 11, 22].into_iter()),
            Rate::Cck11(Preamble::Long)
        );
        assert_eq!(data_rate([12, 24, 48].into_iter()), Rate::Ofdm(Ofdm::M24));
        // An HT selector (127) and a rate the access point lacks (3 Mb/s) are not rates in
        // common, so a station that names only those gets the basic rate.
        assert_eq!(data_rate([127, 6].into_iter()), BASIC);
        assert_eq!(data_rate(std::iter::empty()), BASIC);
    }

    #[test]
    fn an_ack_goes_at_a_basic_or_mandatory_rate_at_or_below_the_frame() {
        let short = Rate::Cck11(Preamble::Short);
        assert_eq!(ack_rate(short), short);
        assert_eq!(ack_rate(Rate::Ofdm(Ofdm::M9)), Rate::Ofdm(Ofdm::M6));
        assert_eq!(ack_rate(Rate::Ofdm(Ofdm::M18)), Rate::Ofdm(Ofdm::M12));
        assert_eq!(ack_rate(Rate::Ofdm(Ofdm::M54)), Rate::Ofdm(Ofdm::M24));
        assert_eq!(
            ack_rate(Rate::Ht(Mcs::new(0).unwrap())),
            Rate::Ofdm(Ofdm::M6)
        );
        assert_eq!(
            ack_rate(Rate::Ht(Mcs::new(2).unwrap())),
            Rate::Ofdm(Ofdm::M12)
        );
        assert_eq!(
            ack_rate(Rate::Ht(Mcs::new(7).unwrap())),
            Rate::Ofdm(Ofdm::M24)
        );
    }
}
