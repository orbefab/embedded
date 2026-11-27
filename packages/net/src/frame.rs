//! MAC frame headers, after IEEE 802.11-2020 9.2 and 9.3.

use crate::Mac;

// Frame types, in Frame Control (9.2.4.1).
pub(crate) const MANAGEMENT: u8 = 0;
pub(crate) const CONTROL: u8 = 1;
pub(crate) const DATA: u8 = 2;

// Management subtypes (9.3.3).
pub(crate) const ASSOC_REQUEST: u8 = 0;
pub(crate) const ASSOC_RESPONSE: u8 = 1;
pub(crate) const REASSOC_REQUEST: u8 = 2;
pub(crate) const REASSOC_RESPONSE: u8 = 3;
pub(crate) const PROBE_REQUEST: u8 = 4;
pub(crate) const PROBE_RESPONSE: u8 = 5;
pub(crate) const BEACON: u8 = 8;
pub(crate) const DISASSOC: u8 = 10;
pub(crate) const AUTH: u8 = 11;
pub(crate) const DEAUTH: u8 = 12;
pub(crate) const ACTION: u8 = 13;

/// The PS-Poll control subtype (9.3.1.5).
pub(crate) const PS_POLL: u8 = 10;
/// The Null data subtype, a Data frame with no body (9.3.2.1).
pub(crate) const NULL: u8 = 4;
/// The bit of a data subtype that makes it a QoS Data frame, with a QoS Control field.
const QOS: u8 = 8;
/// The bit of a data subtype whose frame has no body, as a Null frame.
pub(crate) const NO_DATA: u8 = 4;

// The flags, the second octet of Frame Control (9.2.4.1).
pub(crate) const TO_DS: u8 = 0x01;
pub(crate) const FROM_DS: u8 = 0x02;
pub(crate) const PWR_MGT: u8 = 0x10;
pub(crate) const MORE_DATA: u8 = 0x20;
pub(crate) const PROTECTED: u8 = 0x40;
/// In a management or QoS Data frame, a 4-octet HT Control field follows the header.
const ORDER: u8 = 0x80;

/// The header of a received frame, as far as the access point reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Header {
    pub(crate) kind: u8,
    pub(crate) subtype: u8,
    pub(crate) flags: u8,
    /// Duration/ID (9.2.4.2). In a PS-Poll it is the AID with its two top bits set.
    pub(crate) duration: u16,
    pub(crate) addr1: Mac,
    /// The transmitter, on every frame the access point reads.
    pub(crate) addr2: Mac,
    /// The BSSID of a management frame, or `None` on a control frame.
    pub(crate) addr3: Option<Mac>,
    /// Where the frame body starts, at most the frame's length.
    pub(crate) body: usize,
}

impl Header {
    /// The header of `mpdu`, or `None` when it is not a frame of protocol version 0 with the
    /// fields its type needs.
    pub(crate) fn parse(mpdu: &[u8]) -> Option<Header> {
        let fc = *mpdu.first()?;
        if fc & 3 != 0 {
            return None;
        }
        let kind = (fc >> 2) & 3;
        let subtype = fc >> 4;
        let flags = *mpdu.get(1)?;
        let duration = u16::from_le_bytes([*mpdu.get(2)?, *mpdu.get(3)?]);
        let addr1 = mac(mpdu, 4)?;
        let addr2 = mac(mpdu, 10)?;
        let (addr3, body) = match kind {
            MANAGEMENT => (Some(mac(mpdu, 16)?), 24 + ht_control(flags)),
            // A control frame with a TA: PS-Poll, RTS, BlockAckReq, BlockAck (9.3.1).
            CONTROL => (None, 16),
            DATA => {
                let four = if flags & (TO_DS | FROM_DS) == TO_DS | FROM_DS {
                    6
                } else {
                    0
                };
                let qos = if subtype & QOS != 0 {
                    2 + ht_control(flags)
                } else {
                    0
                };
                (Some(mac(mpdu, 16)?), 24 + four + qos)
            }
            _ => return None,
        };
        (body <= mpdu.len()).then_some(Header {
            kind,
            subtype,
            flags,
            duration,
            addr1,
            addr2,
            addr3,
            body,
        })
    }
}

fn ht_control(flags: u8) -> usize {
    if flags & ORDER != 0 { 4 } else { 0 }
}

fn mac(bytes: &[u8], at: usize) -> Option<Mac> {
    Some(Mac(bytes.get(at..at + 6)?.try_into().ok()?))
}

/// A frame the access point sends: Frame Control, then a Duration and a Sequence Control of 0
/// that [`set_duration`] and [`set_sequence`] fill when it goes on the air, around `addr1` and
/// the BSSID twice. This is the management header of 9.3.3.1, and the header of a Data frame
/// from the DS of 9.3.2.1, where the BSSID is also the source.
pub(crate) fn header(kind: u8, subtype: u8, flags: u8, addr1: Mac, bssid: Mac) -> Vec<u8> {
    let mut f = Vec::with_capacity(64);
    f.push((subtype << 4) | (kind << 2));
    f.push(flags);
    f.extend([0, 0]);
    f.extend(addr1.0);
    f.extend(bssid.0);
    f.extend(bssid.0);
    f.extend([0, 0]);
    f
}

/// Sets the Duration field, in microseconds (9.2.4.2).
pub(crate) fn set_duration(mpdu: &mut [u8], us: u16) {
    mpdu[2..4].copy_from_slice(&us.to_le_bytes());
}

/// Whether `mpdu` is a Data frame with a body (9.3.2.1), which carries an MSDU.
pub(crate) fn has_msdu(mpdu: &[u8]) -> bool {
    let kind = (mpdu[0] >> 2) & 3;
    let subtype = mpdu[0] >> 4;
    kind == DATA && subtype & NO_DATA == 0
}

/// Sets the sequence number, 12 bits with fragment 0 (9.2.4.4).
pub(crate) fn set_sequence(mpdu: &mut [u8], sequence: u16) {
    mpdu[22..24].copy_from_slice(&((sequence & 0xfff) << 4).to_le_bytes());
}

/// Whether the access point buffers `mpdu` for a station in power save: a Data frame, or a
/// Disassociation, Deauthentication or Action frame (11.2.2, Table 11-2).
pub(crate) fn bufferable(mpdu: &[u8]) -> bool {
    let kind = (mpdu[0] >> 2) & 3;
    let subtype = mpdu[0] >> 4;
    kind == DATA || (kind == MANAGEMENT && matches!(subtype, DISASSOC | DEAUTH | ACTION))
}

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}
