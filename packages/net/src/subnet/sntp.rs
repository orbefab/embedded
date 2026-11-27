//! The router's SNTP server (RFC 4330): a primary server whose clock reads the subnet's epoch
//! plus the simulated time.

use orbe_model::Time;

use crate::{Event, Mac};

const LEN: usize = 48;
const CLIENT: u8 = 3;
const SERVER: u8 = 4;
/// A primary server's stratum (RFC 4330 4).
const STRATUM: u8 = 1;
/// Its clock's precision, 2^-20 s, about a microsecond.
const PRECISION: i8 = -20;
/// The Reference Identifier of a primary server's clock: its own, "LOCL" (RFC 4330 4).
const REFERENCE: [u8; 4] = *b"LOCL";
/// The seconds from 1900, the NTP era 0 epoch, to 1970 (RFC 4330 3).
const FROM_1900: u64 = 2_208_988_800;
const PS: u128 = 1_000_000_000_000;

/// The answer to the NTP message `request` from station `sta` that landed at `at`, and its row.
/// A client request of version 1 to 4 gets a server reply of the same version, whose
/// Originate Timestamp is the request's Transmit Timestamp, and whose Receive and Transmit
/// Timestamps are `at` on the network's clock. Its Reference Timestamp is the start of that
/// second, when the clock last ticked to a new second. Anything else is dropped.
pub(super) fn answer(epoch: u64, at: Time, sta: Mac, request: &[u8]) -> Option<(Vec<u8>, Event)> {
    let first = *request.first()?;
    let version = (first >> 3) & 7;
    if request.len() < LEN || first & 7 != CLIENT || !(1..=4).contains(&version) {
        return None;
    }
    let ps = u128::from(epoch) * PS + u128::from(at.as_ps());
    let now = timestamp(ps);
    let mut reply = vec![version << 3 | SERVER, STRATUM, request[2], PRECISION as u8];
    // Root Delay and Root Dispersion: 0 for a primary server whose clock is the reference.
    reply.extend([0; 8]);
    reply.extend(REFERENCE);
    reply.extend(timestamp(ps / PS * PS));
    reply.extend(&request[40..48]);
    reply.extend(now);
    reply.extend(now);
    let event = Event::Sntp {
        sta,
        utc_ms: (ps / 1_000_000_000) as u64,
    };
    Some((reply, event))
}

/// The NTP timestamp of `ps` picoseconds from 1970 (RFC 4330 3): seconds from 1900 in its era,
/// then the fraction of a second in 2^-32 units, rounded down.
fn timestamp(ps: u128) -> [u8; 8] {
    let seconds = (ps / PS) as u64 + FROM_1900;
    let fraction = ((ps % PS) << 32) / PS;
    let mut out = [0; 8];
    out[..4].copy_from_slice(&(seconds as u32).to_be_bytes());
    out[4..].copy_from_slice(&(fraction as u32).to_be_bytes());
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::tests::STA;
    use super::*;

    /// An SNTP version 4 client request with Transmit Timestamp `0x0102030405060708`.
    pub(crate) fn request() -> Vec<u8> {
        let mut r = vec![0x23];
        r.extend([0; 39]);
        r.extend(1..=8);
        r
    }

    fn seconds(ts: &[u8]) -> u32 {
        u32::from_be_bytes(ts[..4].try_into().unwrap())
    }

    // RFC 4330 3: era 0 began 1900-01-01, and its seconds wrap in 2036, at second 2^32, which
    // is 2036-02-07T06:28:16Z, 2085978496 s after 1970. Half a second is 2^31.
    #[test]
    fn timestamps_are_seconds_from_1900_in_the_era_and_a_fraction() {
        assert_eq!(timestamp(0), [0x83, 0xaa, 0x7e, 0x80, 0, 0, 0, 0]);
        let wrap = 2_085_978_496 * PS;
        assert_eq!(timestamp(wrap), [0; 8]);
        assert_eq!(
            timestamp(wrap - PS / 2),
            [0xff, 0xff, 0xff, 0xff, 0x80, 0, 0, 0]
        );
        // The largest fraction is just under a whole second.
        assert_eq!(timestamp(PS - 1)[4..], [0xff; 4]);
    }

    #[test]
    fn a_reply_carries_the_network_clock() {
        // 2026-01-01T00:00:00Z, then 1.25 s.
        let epoch = 1_767_225_600;
        let at = Time::from_ps(1_250_000_000_000);
        let (reply, event) = answer(epoch, at, STA, &request()).unwrap();
        assert_eq!(reply.len(), LEN);
        // LI 0, VN 4, mode 4, stratum 1, the client's poll, precision -20.
        assert_eq!(reply[..4], [0x24, 1, 0, 0xec]);
        assert_eq!(reply[4..12], [0; 8]);
        assert_eq!(reply[12..16], *b"LOCL");
        let secs = (epoch + FROM_1900 + 1) as u32;
        assert_eq!(seconds(&reply[16..]), secs);
        assert_eq!(reply[20..24], [0; 4]);
        assert_eq!(reply[24..32], [1, 2, 3, 4, 5, 6, 7, 8]);
        for at in [32, 40] {
            assert_eq!(seconds(&reply[at..]), secs);
            assert_eq!(reply[at + 4..at + 8], [0x40, 0, 0, 0]);
        }
        assert_eq!(
            event,
            Event::Sntp {
                sta: STA,
                utc_ms: 1_767_225_601_250
            }
        );
        // Version 3 gets version 3.
        let mut v3 = request();
        v3[0] = 0x1b;
        assert_eq!(answer(epoch, at, STA, &v3).unwrap().0[0], 0x1c);
    }

    #[test]
    fn only_a_client_request_is_answered() {
        for first in [0x24, 0x21, 0x25, 0x03, 0x2b] {
            let mut r = request();
            r[0] = first;
            assert!(answer(0, Time::ZERO, STA, &r).is_none(), "{first:#x}");
        }
        assert!(answer(0, Time::ZERO, STA, &request()[..47]).is_none());
        // A longer request, with an extension or a MAC, is still answered.
        let mut long = request();
        long.extend([0; 20]);
        assert_eq!(answer(0, Time::ZERO, STA, &long).unwrap().0.len(), LEN);
    }
}
