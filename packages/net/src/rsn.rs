//! WPA2-PSK keys and the EAPOL-Key frames of the 4-way handshake, after IEEE 802.11-2020 12.7.

use orbe_crypto::aes::AesKey;
use orbe_crypto::hmac::hmac_sha1;
use orbe_crypto::key_wrap;
use orbe_crypto::pbkdf2::pbkdf2_sha1;
use orbe_crypto::prf::prf;

use crate::Mac;

/// LLC/SNAP before an EtherType in a Data frame's body (IEEE 802.1H).
pub(crate) const LLC_SNAP: [u8; 6] = [0xaa, 0xaa, 0x03, 0, 0, 0];
/// The EtherType of IEEE 802.1X, which carries EAPOL.
pub(crate) const EAPOL: [u8; 2] = [0x88, 0x8e];

// Key Information (12.7.2, Figure 12-33). Version 2 is the HMAC-SHA1-128 MIC and AES key wrap
// that the PSK AKM with CCMP-128 uses.
const VERSION: u16 = 7;
const VERSION_2: u16 = 2;
const PAIRWISE: u16 = 1 << 3;
const INSTALL: u16 = 1 << 6;
const ACK: u16 = 1 << 7;
const MIC: u16 = 1 << 8;
const SECURE: u16 = 1 << 9;
const ERROR: u16 = 1 << 10;
const REQUEST: u16 = 1 << 11;
const ENCRYPTED: u16 = 1 << 12;

/// The Key Information of messages 2 and 4, as far as the access point checks it.
const CHECKED: u16 = VERSION | PAIRWISE | INSTALL | ACK | MIC | SECURE | ERROR | REQUEST;
pub(crate) const MESSAGE_2: u16 = VERSION_2 | PAIRWISE | MIC;
pub(crate) const MESSAGE_4: u16 = VERSION_2 | PAIRWISE | MIC | SECURE;

/// Where the EAPOL-Key fields start in an EAPOL frame: the 802.1X header of version, type and
/// body length, then the descriptor type.
const FIELDS: usize = 5;
const NONCE: usize = FIELDS + 2 + 2 + 8;
const MIC_AT: usize = NONCE + 32 + 16 + 8 + 8;
const DATA: usize = MIC_AT + 16 + 2;

/// The PMK of a passphrase (J.4.1): PBKDF2-HMAC-SHA1 over the SSID, 4096 iterations, 256 bits.
pub(crate) fn pmk(passphrase: &[u8], ssid: &[u8]) -> [u8; 32] {
    let mut pmk = [0; 32];
    pbkdf2_sha1(passphrase, ssid, 4096, &mut pmk);
    pmk
}

/// The PMKID of the PMK between the access point `aa` and station `spa` (12.7.1.3).
pub(crate) fn pmkid(pmk: &[u8; 32], aa: Mac, spa: Mac) -> [u8; 16] {
    let mut data = b"PMK Name".to_vec();
    data.extend(aa.0);
    data.extend(spa.0);
    hmac_sha1(pmk, &data)[..16].try_into().expect("16 octets")
}

/// The PTK of CCMP-128 (12.7.1.3): the KCK, the KEK and the TK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ptk {
    pub(crate) kck: [u8; 16],
    pub(crate) kek: [u8; 16],
    pub(crate) tk: [u8; 16],
}

impl Ptk {
    /// PRF-384 of the PMK over both addresses and both nonces, each pair lower first.
    pub(crate) fn derive(pmk: &[u8; 32], aa: Mac, spa: Mac, anonce: &[u8], snonce: &[u8]) -> Ptk {
        let mut data = Vec::with_capacity(76);
        data.extend(aa.0.min(spa.0));
        data.extend(aa.0.max(spa.0));
        data.extend(anonce.min(snonce));
        data.extend(anonce.max(snonce));
        let mut ptk = [0; 48];
        prf(pmk, b"Pairwise key expansion", &data, &mut ptk);
        let part = |at: usize| ptk[at..at + 16].try_into().expect("16 octets");
        Ptk {
            kck: part(0),
            kek: part(16),
            tk: part(32),
        }
    }
}

/// An EAPOL-Key frame a station sent, as far as the access point reads it.
pub(crate) struct Key<'a> {
    /// Key Information with the bits the access point checks.
    pub(crate) info: u16,
    pub(crate) replay: u64,
    pub(crate) nonce: &'a [u8],
    pub(crate) data: &'a [u8],
    /// The frame up to the end of its body, which its MIC covers.
    frame: &'a [u8],
}

impl<'a> Key<'a> {
    /// The EAPOL-Key frame in `eapol`, or `None` when it is not an RSN EAPOL-Key frame whose
    /// fields fit its body.
    pub(crate) fn parse(eapol: &'a [u8]) -> Option<Key<'a>> {
        let [_version, 3, len0, len1, 2, ..] = *eapol else {
            return None;
        };
        let frame = eapol.get(..4 + usize::from(u16::from_be_bytes([len0, len1])))?;
        let data_len = usize::from(u16::from_be_bytes(
            frame.get(DATA - 2..DATA)?.try_into().ok()?,
        ));
        Some(Key {
            info: u16::from_be_bytes([frame[FIELDS], frame[FIELDS + 1]]) & CHECKED,
            replay: u64::from_be_bytes(frame[FIELDS + 4..NONCE].try_into().ok()?),
            nonce: &frame[NONCE..NONCE + 32],
            data: frame.get(DATA..DATA + data_len)?,
            frame,
        })
    }

    /// Whether its MIC is the HMAC-SHA1-128 under `kck` of the frame with the MIC zeroed.
    pub(crate) fn mic_ok(&self, kck: &[u8; 16]) -> bool {
        let mut zeroed = self.frame.to_vec();
        zeroed[MIC_AT..MIC_AT + 16].fill(0);
        hmac_sha1(kck, &zeroed)[..16] == self.frame[MIC_AT..MIC_AT + 16]
    }
}

/// The fields of an EAPOL-Key frame the access point sends that differ between messages.
struct Fields<'a> {
    info: u16,
    replay: u64,
    nonce: &'a [u8; 32],
    iv: [u8; 16],
    rsc: u64,
    data: &'a [u8],
}

/// A Data frame body of LLC/SNAP and an EAPOL-Key frame of 802.1X-2004, version 2, with Key
/// Length 16 for CCMP-128 (12.7.2). With `kck`, its MIC is set.
fn key_frame(fields: &Fields, kck: Option<&[u8; 16]>) -> Vec<u8> {
    let body = DATA - 4 + fields.data.len();
    let mut out = LLC_SNAP.to_vec();
    out.extend(EAPOL);
    let start = out.len();
    out.extend([2, 3]);
    out.extend((body as u16).to_be_bytes());
    out.push(2);
    out.extend(fields.info.to_be_bytes());
    out.extend(16u16.to_be_bytes());
    out.extend(fields.replay.to_be_bytes());
    out.extend(fields.nonce);
    out.extend(fields.iv);
    out.extend(&fields.rsc.to_le_bytes()[..6]);
    out.extend([0; 2 + 8 + 16]);
    out.extend((fields.data.len() as u16).to_be_bytes());
    out.extend(fields.data);
    if let Some(kck) = kck {
        let mic = hmac_sha1(kck, &out[start..]);
        out[start + MIC_AT..start + MIC_AT + 16].copy_from_slice(&mic[..16]);
    }
    out
}

/// Message 1 (12.7.6.2): the ANonce, and the PMKID in a KDE.
pub(crate) fn message_1(replay: u64, anonce: &[u8; 32], pmkid: &[u8; 16]) -> Vec<u8> {
    let mut data = vec![0xdd, 20, 0x00, 0x0f, 0xac, 4];
    data.extend(pmkid);
    key_frame(
        &Fields {
            info: VERSION_2 | PAIRWISE | ACK,
            replay,
            nonce: anonce,
            iv: [0; 16],
            rsc: 0,
            data: &data,
        },
        None,
    )
}

/// The group key message 3 carries, and the access point's RSNE.
pub(crate) struct Group<'a> {
    /// The RSNE's body.
    pub(crate) rsne: &'a [u8],
    pub(crate) gtk: &'a [u8],
    pub(crate) index: u8,
    /// The last PN the access point sent under the GTK, its Key RSC.
    pub(crate) pn: u64,
}

/// Message 3 (12.7.6.4): the ANonce again, the RSNE and the GTK KDE wrapped under the KEK, and
/// its MIC. `iv` is the Key IV, from the access point's key counter.
pub(crate) fn message_3(
    ptk: &Ptk,
    replay: u64,
    anonce: &[u8; 32],
    iv: [u8; 16],
    group: &Group,
) -> Vec<u8> {
    let mut data = vec![48, group.rsne.len() as u8];
    data.extend(group.rsne);
    // The GTK KDE (12.7.2, Figure 12-36): its Key ID, Tx 0, then the GTK.
    data.extend([0xdd, 6 + group.gtk.len() as u8, 0x00, 0x0f, 0xac, 1]);
    data.extend([group.index & 3, 0]);
    data.extend(group.gtk);
    // Key data under AES key wrap is padded to a multiple of 8, and at least 16, with 0xdd and
    // then zeros (12.7.2).
    if data.len() < 16 || !data.len().is_multiple_of(8) {
        data.push(0xdd);
        while data.len() < 16 || !data.len().is_multiple_of(8) {
            data.push(0);
        }
    }
    let kek = AesKey::new(&ptk.kek).expect("a 128-bit KEK");
    let wrapped = key_wrap::wrap(&kek, &data).expect("key data of 16 octets or more");
    key_frame(
        &Fields {
            info: VERSION_2 | PAIRWISE | INSTALL | ACK | MIC | SECURE | ENCRYPTED,
            replay,
            nonce: anonce,
            iv,
            rsc: group.pn,
            data: &wrapped,
        },
        Some(&ptk.kck),
    )
}

/// A 256-bit counter, big-endian, one more.
pub(crate) fn increment(counter: &mut [u8; 32]) {
    for octet in counter.iter_mut().rev() {
        *octet = octet.wrapping_add(1);
        if *octet != 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // IEEE 802.11-2020 J.4.2: passphrase "password", SSID "IEEE".
    #[test]
    fn the_pmk_of_a_passphrase_is_the_standard_s() {
        let want = "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e";
        let got: String = pmk(b"password", b"IEEE")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_counter_carries() {
        let mut counter = [0; 32];
        counter[30] = 0x12;
        counter[31] = 0xff;
        increment(&mut counter);
        assert_eq!(counter[30..], [0x13, 0]);
        let mut all = [0xff; 32];
        increment(&mut all);
        assert_eq!(all, [0; 32]);
    }

    #[test]
    fn message_3_pads_its_key_data_and_checks_under_its_own_mic() {
        let ptk = Ptk {
            kck: [1; 16],
            kek: [2; 16],
            tk: [3; 16],
        };
        let group = Group {
            rsne: &crate::element::RSN_PSK,
            gtk: &[9; 16],
            index: 1,
            pn: 0x0102,
        };
        let msdu = message_3(&ptk, 5, &[7; 32], [0; 16], &group);
        let key = Key::parse(&msdu[8..]).unwrap();
        assert!(key.mic_ok(&ptk.kck));
        assert!(!key.mic_ok(&ptk.tk));
        assert_eq!(key.replay, 5);
        let kek = AesKey::new(&ptk.kek).unwrap();
        let plain = key_wrap::unwrap(&kek, key.data).unwrap();
        // RSNE 22, GTK KDE 24, then 0xdd and a zero to 48.
        assert_eq!(plain.len(), 48);
        assert_eq!(plain[46..], [0xdd, 0]);
        assert_eq!(plain[22..30], [0xdd, 22, 0, 0x0f, 0xac, 1, 1, 0]);
    }
}
