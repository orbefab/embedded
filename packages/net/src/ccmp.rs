//! CCMP-128 on Data frames (IEEE 802.11-2020 12.5.3): CCM over AES-128 with a 13-octet nonce
//! and an 8-octet MIC, after an 8-octet CCMP header that carries the PN.

use orbe_crypto::aes::AesKey;
use orbe_crypto::ccm;

use crate::frame::{DATA, FROM_DS, TO_DS};

const HEADER: usize = 8;
const MIC: usize = 8;
/// The Extended IV bit of the CCMP header's Key ID octet, always set.
const EXT_IV: u8 = 0x20;

/// The replay counter a received frame counts against (12.5.3.4.4): one per TID for QoS Data,
/// and one more for the rest.
pub(crate) const COUNTERS: usize = 17;

/// What [`open`] found in a protected Data frame.
pub(crate) struct Opened {
    pub(crate) pn: u64,
    /// Its replay counter, below [`COUNTERS`].
    pub(crate) counter: usize,
    pub(crate) plaintext: Vec<u8>,
}

/// The PN and the plaintext of the protected Data frame `mpdu`, whose header ends at `body`,
/// under key `key_id` of `tk`. `None` when it is too short, its Key ID or Extended IV is wrong,
/// or its MIC fails.
pub(crate) fn open(tk: &[u8; 16], key_id: u8, mpdu: &[u8], body: usize) -> Option<Opened> {
    let ccmp = mpdu.get(body..body + HEADER)?;
    if ccmp[3] != EXT_IV | key_id << 6 || mpdu.len() < body + HEADER + MIC {
        return None;
    }
    let pn = u64::from_le_bytes([ccmp[0], ccmp[1], ccmp[4], ccmp[5], ccmp[6], ccmp[7], 0, 0]);
    let (aad, nonce, counter) = aad_and_nonce(&mpdu[..body], pn);
    let key = AesKey::new(tk).expect("a 128-bit TK");
    let plaintext = ccm::open(&key, &nonce, &aad, &mpdu[body + HEADER..], MIC)?;
    Some(Opened {
        pn,
        counter,
        plaintext,
    })
}

/// `header`, the Data frame's MAC header with Protected set, then the CCMP header of `pn` and
/// key `key_id`, and `plaintext` sealed under `tk`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the subnet's frames to a station use it")
)]
pub(crate) fn seal(tk: &[u8; 16], key_id: u8, pn: u64, header: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let (aad, nonce, _) = aad_and_nonce(header, pn);
    let key = AesKey::new(tk).expect("a 128-bit TK");
    let p = pn.to_le_bytes();
    let mut out = header.to_vec();
    out.extend([p[0], p[1], 0, EXT_IV | key_id << 6, p[2], p[3], p[4], p[5]]);
    out.extend(ccm::seal(&key, &nonce, &aad, plaintext, MIC));
    out
}

/// The AAD and the nonce of a Data frame's MAC header (12.5.3.3.3, 12.5.3.3.4), and its replay
/// counter. The AAD keeps Frame Control with its subtype's low bits, Retry, Power Management
/// and More Data cleared and Protected set, the addresses, the fragment number, and the TID of
/// QoS Control. The nonce is the TID, the transmitter and the PN.
fn aad_and_nonce(header: &[u8], pn: u64) -> (Vec<u8>, [u8; 13], usize) {
    let qos = (header[0] >> 2) & 3 == DATA && header[0] & 0x80 != 0;
    let four = header[1] & (TO_DS | FROM_DS) == TO_DS | FROM_DS;
    let mut aad = Vec::with_capacity(30);
    aad.push(header[0] & 0x8f);
    let mut flags = (header[1] & !0x38) | 0x40;
    if qos {
        flags &= !0x80;
    }
    aad.push(flags);
    aad.extend(&header[4..22]);
    aad.extend([header[22] & 0x0f, 0]);
    let mut at = 24;
    if four {
        aad.extend(&header[24..30]);
        at = 30;
    }
    let tid = if qos { header[at] & 0x0f } else { 0 };
    if qos {
        aad.extend([tid, 0]);
    }
    let mut nonce = [0; 13];
    nonce[0] = tid;
    nonce[1..7].copy_from_slice(&header[10..16]);
    nonce[7..].copy_from_slice(&pn.to_be_bytes()[2..]);
    let counter = if qos { usize::from(tid) } else { COUNTERS - 1 };
    (aad, nonce, counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // IEEE 802.11-2020 J.6.4, the CCMP test vector: a Data frame with Retry set and PN
    // 0xb5039776e70c.
    #[test]
    fn the_standard_s_vector_seals_and_opens() {
        let tk: [u8; 16] = hex("c97c1f67ce371185514a8a19f2bdd52f").try_into().unwrap();
        let header = hex("08 48 c3 2c 0f d2 e1 28 a5 7c 50 30 f1 84 44 08 ab ae a5 b8 fc ba 80 33");
        let plaintext = hex("f8 ba 1a 55 d0 2f 85 ae 96 7b b6 2f b6 cd a8 eb 7e 78 a0 50");
        let want = hex(
            "08 48 c3 2c 0f d2 e1 28 a5 7c 50 30 f1 84 44 08 ab ae a5 b8 fc ba 80 33
             0c e7 00 20 76 97 03 b5
             f3 d0 a2 fe 9a 3d bf 23 42 a6 43 e4 32 46 e8 0c 3c 04 d0 19
             78 45 ce 0b 16 f9 76 23",
        );
        let sealed = seal(&tk, 0, 0xb503_9776_e70c, &header, &plaintext);
        assert_eq!(sealed, want);
        let opened = open(&tk, 0, &want, 24).unwrap();
        assert_eq!(opened.pn, 0xb503_9776_e70c);
        assert_eq!(opened.plaintext, plaintext);
        assert_eq!(opened.counter, COUNTERS - 1);

        let mut tampered = want.clone();
        tampered[30] ^= 1;
        assert!(open(&tk, 0, &tampered, 24).is_none());
        assert!(open(&tk, 1, &want, 24).is_none());
        assert!(open(&tk, 0, &want[..24 + 8 + 7], 24).is_none());
    }

    #[test]
    fn a_qos_frame_counts_against_its_tid() {
        let tk = [5; 16];
        let mut header =
            hex("88 41 00 00 0a 0a 0a 0a 0a 0a 0b 0b 0b 0b 0b 0b 0c 0c 0c 0c 0c 0c 00 00");
        header.extend([0x86, 0x00]);
        let sealed = seal(&tk, 0, 9, &header, b"payload");
        let opened = open(&tk, 0, &sealed, 26).unwrap();
        assert_eq!(
            (opened.counter, opened.plaintext.as_slice()),
            (6, &b"payload"[..])
        );
        // The TID is in the nonce and the AAD.
        let mut moved = sealed.clone();
        moved[24] = 0x85;
        assert!(open(&tk, 0, &moved, 26).is_none());
        // Retry and Power Management are not.
        let mut retried = sealed;
        retried[1] |= 0x18;
        assert!(open(&tk, 0, &retried, 26).is_some());
    }
}
