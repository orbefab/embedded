//! When the access point may send: the interframe spaces, the acknowledgement it waits for, and
//! the beacon interval.

use orbe_model::air::{Ofdm, Rate};

pub(crate) const US: u64 = 1_000_000;

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

/// The rate the access point sends every frame at, 1 Mb/s with the long preamble.
pub(crate) const RATE: Rate = Rate::Dsss1;

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

/// The Duration field of an individually addressed frame the access point sends: the
/// microseconds of SIFS and the ACK after it, rounded up (9.2.4.2).
pub(crate) fn duration_us() -> u16 {
    acked_after(RATE).div_ceil(US) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbe_model::air::{Mcs, Preamble};

    // Wireshark's wpa-Induction.pcap: every individually addressed frame the 1 Mb/s access
    // point sends has Duration 0x013a, 314 us: SIFS and a 304 us ACK.
    #[test]
    fn a_frame_at_1_mbps_reserves_its_ack() {
        assert_eq!(duration_us(), 314);
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
