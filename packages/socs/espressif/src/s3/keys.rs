//! The Wi-Fi MAC's key table, and the CCMP-128 it runs with it on the frames it sends and
//! receives (IEEE 802.11-2020, 12.5.3).
//!
//! The layout of an entry comes from the Arduino-ESP32 3.3.12 driver, `libpp.a`:
//! `hal_crypto_set_key_entry` writes entry `n` at [`KEYS`] `+ 40 n`, and
//! `hal_crypto_get_key_entry` reads it back the same way. Word 0 holds octets 0 to 3 of the peer's
//! MAC address. Word 1 holds:
//! - bits 0 to 15: octets 4 and 5 of the address;
//! - bits 18 to 20: the cipher, the driver's `wpa_alg` (`wpa_cipher_to_alg` in
//!   `libwpa_supplicant.a`) as `hal_crypto_set_key_entry` folds it: 1 WEP, 2 TKIP, 3 CCMP, 4 SMS4,
//!   5 GCMP;
//! - bit 16: WEP-104, only with cipher 1;
//! - bit 26: a 256-bit key, only with cipher 5;
//! - bits 21 to 23: 7 with cipher 1, and otherwise 3 (bits 21 and 22) for entries 4 to 24 and 6
//!   (bits 22 and 23) for entries 0 to 3;
//! - bits 24 and 25: the interface;
//! - bit 27: set unless the key ID is 3, with a meaning the model does not know and does not read;
//! - bits 30 and 31: the key ID.
//!
//! `hal_crypto_set_key_entry` sets no other bit of word 1. `hal_crypto_clr_key_entry` writes 0 over
//! the entry, and `ppInstallKey` clears an entry before it sets it. A store of any other word 1 is
//! refused by name, `key_entry` ([`refused`]).
//!
//! Words 2 to 9 hold the key, low octet first. A CCMP-128 key fills words 2 to 5, and the driver
//! leaves words 6 to 9 at 0: a frame that uses a CCMP-128 entry whose words 6 to 9 are not 0 is
//! refused by name, `tx_key_words` or `rx_key_words`. `esp_wifi_get_sta_hw_key_idx_internal` puts a station's pairwise key
//! in entry 4 and its group key with key ID 1 or 2 in entry 0 or 1, so bit 21 marks a pairwise key
//! and bit 23 a group key. esp32-open-mac's `esp32` PAC names the same two bits so
//! (`src/wifi/crypto_key_slot/addr_high.rs`, crate 0.40.2), and bit 22 "set to 1 for all
//! algorithms", with a meaning it does not know. The model does not read bit 22. The supplicant
//! installs every pairwise key with key ID 0 (`wpa_supplicant_install_ptk` and `eapol_txcb` pass 0
//! to `wpa_sm_set_key`). Bit `n` of [`VALID`] marks entry `n` in use, as
//! `hal_crypto_set_key_entry` sets it and `hal_crypto_is_key_valid` reads it. No function sets
//! bits 25 to 31 of it, and a store that does is refused by name, `key_valid`.
//!
//! How the MAC uses the table is a guess, from the driver and from esp32-open-mac's write-up
//! "Reverse engineering WPA crypto acceleration on ESP32" (2025-03-07) of the ESP32's MAC:
//! - It encrypts a frame it sends whose Protected Frame bit is set, with the entry that bits 17
//!   to 24 of the queue's PLCP1 word name. `mac_tx_set_plcp1` writes them there, and the
//!   write-up says the driver names the slot in PLCP1. The driver fills the CCMP header itself
//!   (`ccmp_encap`, from the PN it counts in its own key) and leaves 8 octets for the MIC before
//!   the FCS (`ppProcTxSecFrame`). The MAC takes the PN from that header and writes the
//!   ciphertext and the MIC over the plaintext and those 8 octets.
//! - It decrypts a frame it receives whose Protected Frame bit is set. An individually addressed
//!   frame takes the pairwise entry of the interface whose address is the frame's Address 2,
//!   as the write-up says. Whether the MAC also compares the CCMP header's key ID with the
//!   entry's is not known, so one whose key ID is not the entry's is refused by name,
//!   `rx_key_id`. A group-addressed frame takes the interface's group entry with the
//!   CCMP header's key ID. The write-up found that the address of a group entry does not matter.
//!   The MAC writes the plaintext over the ciphertext and keeps the MAC header, the CCMP header and
//!   the MIC, because the driver's `ccmp_decap` reads the CCMP header and strips it and the MIC.
//! - It checks the MIC and not the PN. `ccmp_decap` runs the replay check itself, against the
//!   counters the driver keeps for each TID.

use orbe_crypto::aes::AesKey;
use orbe_crypto::ccm;

/// The table's first word, and its size.
pub(crate) const KEYS: u32 = 0x6003_4400;
pub(crate) const ENTRIES: usize = 25;
pub(crate) const WORDS: usize = 10;
/// The word whose bit `n` marks entry `n` in use.
pub(crate) const VALID: u32 = 0x6003_3814;
const VALID_BITS: u32 = (1 << ENTRIES) - 1;
/// Interface `n`'s crypto word, at `+ 4 n`, as the blobs write it:
/// - `hal_crypto_init` writes `0x30000` to the words of interfaces 0 and 1 and 0 to those of 2
///   and 3, and `hal_crypto_disable` writes `0x30000`. `hal_crypto_mgmt_rx_enabled` reads bit 17.
///   The model reads neither bit 16 nor bit 17: protected Management frames are refused.
/// - `hal_crypto_enable`, which only ever writes interface 0's or 1's word, writes `0x30103`. It
///   adds bit 28 for WEP and bit 9 when `ieee80211_get_spp` says the peer and the station both
///   protect the A-MSDU Present bit, and then sets bit 31 for SMS4 and clears bits 30 and 31
///   otherwise. WEP and SMS4 are refused, so bits 28 and 31 do nothing.
///
/// That the MAC encrypts and decrypts for the interface only while bits 0, 1 and 8 are set, and
/// that bit 9 keeps the A-MSDU Present bit in the AAD (IEEE 802.11-2020, 12.5.3.3.3), are
/// guesses. A store of any other word is refused by name, `crypto_control`.
pub(crate) const CONTROL: u32 = 0x6003_3800;
const CONTROL_ON: u32 = 0x103;
const CONTROL_INIT: u32 = 0x30000;
const CONTROL_SPP: u32 = 1 << 9;
const CONTROL_WITH_ON: u32 = 1 << 28 | CONTROL_SPP | 1 << 31;
/// The word `hal_crypto_enable` sets bits 6 to 21 of for SMS4 and clears them otherwise, and
/// `hal_crypto_init` writes 0. SMS4 is refused, so the bits do nothing. A store that sets another
/// bit is refused by name, `crypto_control`.
const SMS4: u32 = 0x6003_3810;
const SMS4_BITS: u32 = 0x003f_ffc0;
/// The word `hal_crypto_init` ORs 25, the number of entries, into. A store of anything but 0 or
/// 25 is refused by name, `crypto_init`.
const INIT: u32 = 0x6003_3840;

/// The queue's PLCP1 bits that name the entry a frame is sent with: `mac_tx_set_plcp1` writes a
/// byte of the driver's frame there, bits 17 to 24. An entry beyond 24 is not in use.
pub(crate) const PLCP1_SHIFT: u32 = 17;
pub(crate) const PLCP1_ENTRY: u32 = 0xff;

const PAIRWISE: u32 = 1 << 21;
const GROUP: u32 = 1 << 23;
const WEP104: u32 = 1 << 16;
const LONG: u32 = 1 << 26;
const KEY_ID_NOT_3: u32 = 1 << 27;
const WEP: u32 = 1;
const CCMP: u32 = 3;
const GCMP: u32 = 5;
/// The bits of word 1 that `hal_crypto_set_key_entry` can set.
const WORD1_BITS: u32 = !(1 << 17 | 1 << 28 | 1 << 29);

/// The CCMP header and the MIC (12.5.3.2).
const HEADER: usize = 8;
const MIC: usize = 8;
/// The Extended IV bit of the CCMP header's Key ID octet.
const EXT_IV: u8 = 0x20;

/// A Data frame (9.2.4.1.3), and Frame Control's DS bits and Order bit.
const DATA: u8 = 2;
const TO_DS: u8 = 0x01;
const FROM_DS: u8 = 0x02;
const ORDER: u8 = 0x80;

/// The entry at `n` in `keys`, the table's words in order.
fn entry(keys: &[u32], n: usize) -> &[u32] {
    &keys[n * WORDS..(n + 1) * WORDS]
}

/// The entry's peer address, from words 0 and 1.
fn address(entry: &[u32]) -> [u8; 6] {
    let mut mac = [0; 6];
    mac[..4].copy_from_slice(&entry[0].to_le_bytes());
    mac[4..].copy_from_slice(&(entry[1] as u16).to_le_bytes());
    mac
}

fn interface(entry: &[u32]) -> u32 {
    (entry[1] >> 24) & 3
}

fn key_id(entry: &[u32]) -> u8 {
    (entry[1] >> 30) as u8
}

/// Whether `word` is a word 1 that `hal_crypto_set_key_entry` writes for entry `n`, or 0.
fn is_word1(n: usize, word: u32) -> bool {
    let cipher = (word >> 18) & 7;
    let kind = match cipher {
        WEP => 7,
        _ if n >= 4 => 3,
        _ => 6,
    };
    word == 0
        || word & !WORD1_BITS == 0
            && (word >> 21) & 7 == kind
            && (word & WEP104 == 0 || cipher == WEP)
            && (word & LONG == 0 || cipher == GCMP)
            && (word & KEY_ID_NOT_3 != 0) == (word >> 30 != 3)
}

/// Whether `word` is a value the blobs leave in interface `n`'s crypto word.
fn is_control(n: u32, word: u32) -> bool {
    let on = word & CONTROL_ON;
    word == 0
        || n < 2
            && word & !(CONTROL_INIT | CONTROL_ON | CONTROL_WITH_ON) == 0
            && word & CONTROL_INIT == CONTROL_INIT
            && (on == CONTROL_ON || on == 0 && word & CONTROL_WITH_ON == 0)
}

/// The name a store of `value` to `addr` is refused by, when `addr` is a word of the key table or
/// of its control words and `value` is one no blob function writes there.
pub(crate) fn refused(addr: u32, value: u32) -> Option<&'static str> {
    let ok = match addr {
        VALID => value & !VALID_BITS == 0,
        SMS4 => value & !SMS4_BITS == 0,
        INIT => return (value != 0 && value != ENTRIES as u32).then_some("crypto_init"),
        _ if (CONTROL..CONTROL + 16).contains(&addr) => is_control((addr - CONTROL) / 4, value),
        _ => {
            let at = addr.checked_sub(KEYS)? as usize / 4;
            if !addr.is_multiple_of(4) || at >= ENTRIES * WORDS || at % WORDS != 1 {
                return None;
            }
            return (!is_word1(at / WORDS, value)).then_some("key_entry");
        }
    };
    (!ok).then_some(if addr == VALID {
        "key_valid"
    } else {
        "crypto_control"
    })
}

/// The entry's AES-128 key, or the name of the cipher the model does not run.
fn aes(entry: &[u32], tx: bool) -> Result<AesKey, &'static str> {
    let word = entry[1];
    let cipher = (word >> 18) & 7;
    let name = |tx_name, rx_name| if tx { tx_name } else { rx_name };
    if cipher != CCMP {
        return Err(match cipher {
            1 => name("tx_wep", "rx_wep"),
            2 => name("tx_tkip", "rx_tkip"),
            4 => name("tx_sms4", "rx_sms4"),
            5 => name("tx_gcmp", "rx_gcmp"),
            _ => name("tx_cipher", "rx_cipher"),
        });
    }
    if entry[6..].iter().any(|&word| word != 0) {
        return Err(name("tx_key_words", "rx_key_words"));
    }
    let mut key = [0; 16];
    for (k, word) in entry[2..6].iter().enumerate() {
        key[4 * k..4 * k + 4].copy_from_slice(&word.to_le_bytes());
    }
    Ok(AesKey::new(&key).expect("a 128-bit key"))
}

/// Where a protected Data frame's MAC header ends, or the name of a frame the model does not
/// run: a protected Management frame, which management frame protection sends (12.6.18), a
/// Control or Extension frame with the Protected Frame bit, and a QoS Data frame with an HT
/// Control field.
fn body(mpdu: &[u8], tx: bool) -> Result<usize, &'static str> {
    let name = |tx_name, rx_name| if tx { tx_name } else { rx_name };
    match (mpdu[0] >> 2) & 3 {
        DATA => {}
        0 => return Err(name("tx_mfp", "rx_mfp")),
        _ => return Err(name("tx_protected_control", "rx_protected_control")),
    }
    let four = mpdu[1] & (TO_DS | FROM_DS) == TO_DS | FROM_DS;
    let qos = mpdu[0] & 0x80 != 0;
    if qos && mpdu[1] & ORDER != 0 {
        return Err(name("tx_ht_control", "rx_ht_control"));
    }
    Ok(24 + if four { 6 } else { 0 } + if qos { 2 } else { 0 })
}

/// The AAD and the nonce of the Data frame whose MAC header is `header` (12.5.3.3.3 and
/// 12.5.3.3.4). The AAD keeps Frame Control with subtype bits 4 to 6, Retry, Power Management
/// and More Data cleared and Protected set, with Order cleared in a QoS frame, then the three
/// addresses, Sequence Control with only the fragment number, Address 4 when the frame has it,
/// and of QoS Control only the TID, and the A-MSDU Present bit when `spp`. The nonce is the TID,
/// Address 2 and the PN.
fn aad_and_nonce(header: &[u8], pn: [u8; 6], spp: bool) -> (Vec<u8>, [u8; 13]) {
    let qos = header[0] & 0x80 != 0;
    let four = header[1] & (TO_DS | FROM_DS) == TO_DS | FROM_DS;
    let mut aad = Vec::with_capacity(30);
    aad.push(header[0] & 0x8f);
    let mut flags = (header[1] & !0x38) | 0x40;
    if qos {
        flags &= !ORDER;
    }
    aad.push(flags);
    aad.extend(&header[4..22]);
    aad.extend([header[22] & 0x0f, 0]);
    if four {
        aad.extend(&header[24..30]);
    }
    let tid = if qos {
        header[header.len() - 2] & 0x0f
    } else {
        0
    };
    if qos {
        let amsdu = if spp {
            header[header.len() - 2] & 0x80
        } else {
            0
        };
        aad.extend([tid | amsdu, 0]);
    }
    let mut nonce = [0; 13];
    nonce[0] = tid;
    nonce[1..7].copy_from_slice(&header[10..16]);
    for (k, octet) in pn.iter().rev().enumerate() {
        nonce[7 + k] = *octet;
    }
    (aad, nonce)
}

/// The PN of a CCMP header, PN0 first (12.5.3.2).
fn pn(ccmp: &[u8]) -> [u8; 6] {
    [ccmp[0], ccmp[1], ccmp[4], ccmp[5], ccmp[6], ccmp[7]]
}

/// What the MAC does with a protected frame it sends.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Sealed {
    /// The MPDU it sends, the same length as the buffer's.
    Mpdu(Vec<u8>),
    /// The entry PLCP1 names is not in use. The MAC sends nothing and reports a key error.
    NoKey,
}

/// Encrypts `mpdu`, the protected Data frame in a queue's buffer without its FCS, whose CCMP
/// header the driver filled and whose last 8 octets the MIC takes, with entry `n` of `keys`.
/// `valid` is [`VALID`]'s word and `control` reads an interface's crypto word.
pub(crate) fn seal(
    keys: &[u32],
    valid: u32,
    control: impl Fn(u32) -> u32,
    n: u32,
    mpdu: &[u8],
) -> Result<Sealed, &'static str> {
    let body = body(mpdu, true)?;
    if mpdu.len() < body + HEADER + MIC {
        return Err("tx_length");
    }
    let n = n as usize;
    if n >= ENTRIES || valid & 1 << n == 0 {
        return Ok(Sealed::NoKey);
    }
    let entry = entry(keys, n);
    let control = control(interface(entry));
    if control & CONTROL_ON != CONTROL_ON {
        return Err("tx_crypto_off");
    }
    let key = aes(entry, true)?;
    let (header, rest) = mpdu.split_at(body);
    let (aad, nonce) = aad_and_nonce(header, pn(rest), control & CONTROL_SPP != 0);
    let plaintext = &rest[HEADER..rest.len() - MIC];
    let mut out = mpdu[..body + HEADER].to_vec();
    out.extend(ccm::seal(&key, &nonce, &aad, plaintext, MIC));
    Ok(Sealed::Mpdu(out))
}

/// What the MAC does with a protected frame it receives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// The MPDU with its plaintext over its ciphertext, the same length.
    Mpdu(Vec<u8>),
    /// No entry in use takes it. The MAC keeps it as it came.
    NoKey,
    /// Its CCMP header lacks the Extended IV bit, it is too short for the MIC, or the MIC fails.
    /// The MAC keeps it as it came.
    Failed,
}

/// Decrypts `mpdu`, a protected frame the filter slot of interface `interface` took, without
/// its FCS.
pub(crate) fn open(
    keys: &[u32],
    valid: u32,
    control: impl Fn(u32) -> u32,
    interface: u32,
    mpdu: &[u8],
) -> Result<Opened, &'static str> {
    let body = body(mpdu, false)?;
    let Some(ccmp) = mpdu.get(body..body + HEADER) else {
        return Ok(Opened::Failed);
    };
    let group = mpdu[4] & 1 != 0;
    let id = ccmp[3] >> 6;
    let peer = &mpdu[10..16];
    let found = (0..ENTRIES).find(|&n| {
        let entry = entry(keys, n);
        valid & 1 << n != 0
            && self::interface(entry) == interface
            && if group {
                entry[1] & GROUP != 0 && key_id(entry) == id
            } else {
                entry[1] & PAIRWISE != 0 && address(entry) == peer
            }
    });
    let Some(n) = found else {
        return Ok(Opened::NoKey);
    };
    let control = control(interface);
    if control & CONTROL_ON != CONTROL_ON {
        return Err("rx_crypto_off");
    }
    let entry = entry(keys, n);
    let key = aes(entry, false)?;
    if !group && key_id(entry) != id {
        return Err("rx_key_id");
    }
    if ccmp[3] & EXT_IV == 0 || mpdu.len() < body + HEADER + MIC {
        return Ok(Opened::Failed);
    }
    let (header, rest) = mpdu.split_at(body);
    let (aad, nonce) = aad_and_nonce(header, pn(rest), control & CONTROL_SPP != 0);
    let Some(plaintext) = ccm::open(&key, &nonce, &aad, &rest[HEADER..], MIC) else {
        return Ok(Opened::Failed);
    };
    let mut out = mpdu.to_vec();
    out[body + HEADER..mpdu.len() - MIC].copy_from_slice(&plaintext);
    Ok(Opened::Mpdu(out))
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

    /// IEEE 802.11-2020 J.6.4's CCMP test vector: a Data frame with Retry set, PN
    /// 0xb5039776e70c, key ID 0.
    const TK: &str = "c97c1f67ce371185514a8a19f2bdd52f";
    const HEADER_24: &str =
        "08 48 c3 2c 0f d2 e1 28 a5 7c 50 30 f1 84 44 08 ab ae a5 b8 fc ba 80 33";
    const CCMP_HEADER: &str = "0c e7 00 20 76 97 03 b5";
    const PLAINTEXT: &str = "f8 ba 1a 55 d0 2f 85 ae 96 7b b6 2f b6 cd a8 eb 7e 78 a0 50";
    const SEALED: &str = "f3 d0 a2 fe 9a 3d bf 23 42 a6 43 e4 32 46 e8 0c 3c 04 d0 19
                          78 45 ce 0b 16 f9 76 23";

    /// A table with `n` set as the driver sets it: the address in words 0 and 1, `bits` in
    /// word 1's high half, and the key from word 2.
    fn table(n: usize, peer: &[u8], bits: u32, key: &[u8]) -> Vec<u32> {
        let mut keys = vec![0; ENTRIES * WORDS];
        let at = n * WORDS;
        keys[at] = u32::from_le_bytes(peer[..4].try_into().unwrap());
        keys[at + 1] = u32::from(u16::from_le_bytes([peer[4], peer[5]])) | bits;
        for (k, chunk) in key.chunks(4).enumerate() {
            keys[at + 2 + k] = u32::from_le_bytes(chunk.try_into().unwrap());
        }
        keys
    }

    /// The pairwise entry the stock station's driver wrote at entry 4: CCMP, bits 21 and 22,
    /// bit 27, interface 0 and key ID 0 (`0x086c` in word 1's high half).
    const STOCK_PAIRWISE: u32 = 0x086c << 16;
    /// Its group entry at 0, key ID 1 (`0x48cc`).
    const STOCK_GROUP: u32 = 0x48cc << 16;

    fn on(_: u32) -> u32 {
        0x30103
    }

    #[test]
    fn the_stock_entries_decode_as_pairwise_and_group_ccmp() {
        for (bits, pairwise, group, id) in [
            (STOCK_PAIRWISE, true, false, 0),
            (STOCK_GROUP, false, true, 1),
        ] {
            let keys = table(0, &[2, 0x6f, 0x72, 0x62, 0x65, 1], bits, &[0; 16]);
            let entry = entry(&keys, 0);
            assert_eq!(
                (entry[1] & PAIRWISE != 0, entry[1] & GROUP != 0),
                (pairwise, group)
            );
            assert_eq!((interface(entry), key_id(entry)), (0, id));
            assert_eq!(address(entry), [2, 0x6f, 0x72, 0x62, 0x65, 1]);
            assert!(aes(entry, true).is_ok());
        }
    }

    #[test]
    fn a_sent_frame_is_sealed_as_the_standard_s_vector() {
        let header = hex(HEADER_24);
        let tk = hex(TK);
        let keys = table(4, &header[4..10], STOCK_PAIRWISE, &tk);
        let mut buffer = header.clone();
        buffer.extend(hex(CCMP_HEADER));
        buffer.extend(hex(PLAINTEXT));
        buffer.extend([0xef, 0xbe, 0xad, 0xde, 0, 0, 0x78, 0x56]);
        let mut want = header.clone();
        want.extend(hex(CCMP_HEADER));
        want.extend(hex(SEALED));
        assert_eq!(seal(&keys, 1 << 4, on, 4, &buffer), Ok(Sealed::Mpdu(want)));
        assert_eq!(seal(&keys, 1 << 4, on, 3, &buffer), Ok(Sealed::NoKey));
        assert_eq!(seal(&keys, 0, on, 4, &buffer), Ok(Sealed::NoKey));
        assert_eq!(seal(&keys, 1 << 4, on, 25, &buffer), Ok(Sealed::NoKey));
        assert_eq!(
            seal(&keys, 1 << 4, |_| 0x30000, 4, &buffer),
            Err("tx_crypto_off")
        );
        assert_eq!(
            seal(&keys, 1 << 4, on, 4, &buffer[..24 + 8 + 7]),
            Err("tx_length")
        );
    }

    /// The vector's frame made individually addressed, as the vector's Address 1 is a group
    /// address, sealed under entry 4 of `keys`.
    fn unicast(keys: &[u32]) -> Vec<u8> {
        let mut buffer = hex(HEADER_24);
        buffer[4] = 0x0e;
        buffer.extend(hex(CCMP_HEADER));
        buffer.extend(hex(PLAINTEXT));
        buffer.extend([0; MIC]);
        let Ok(Sealed::Mpdu(mpdu)) = seal(keys, 1 << 4, on, 4, &buffer) else {
            panic!();
        };
        mpdu
    }

    #[test]
    fn a_received_frame_is_opened_in_place_with_its_headers_and_mic() {
        let header = hex(HEADER_24);
        let tk = hex(TK);
        // The frame's Address 2 is the peer.
        let keys = table(4, &header[10..16], STOCK_PAIRWISE, &tk);
        let mpdu = unicast(&keys);
        let mut want = mpdu.clone();
        want[32..52].copy_from_slice(&hex(PLAINTEXT));
        assert_eq!(open(&keys, 1 << 4, on, 0, &mpdu), Ok(Opened::Mpdu(want)));

        let mut tampered = mpdu.clone();
        tampered[40] ^= 1;
        assert_eq!(open(&keys, 1 << 4, on, 0, &tampered), Ok(Opened::Failed));
        assert_eq!(
            open(&keys, 1 << 4, on, 0, &mpdu[..24 + 8 + 7]),
            Ok(Opened::Failed)
        );
        assert_eq!(
            open(&keys, 1 << 4, |_| 0x30000, 0, &mpdu),
            Err("rx_crypto_off")
        );
        // Another interface's, another peer's, or a group entry does not take it.
        assert_eq!(open(&keys, 1 << 4, on, 1, &mpdu), Ok(Opened::NoKey));
        let other = table(4, &header[4..10], STOCK_PAIRWISE, &tk);
        assert_eq!(open(&other, 1 << 4, on, 0, &mpdu), Ok(Opened::NoKey));
        let group = table(4, &header[10..16], STOCK_GROUP, &tk);
        assert_eq!(open(&group, 1 << 4, on, 0, &mpdu), Ok(Opened::NoKey));
        assert_eq!(open(&keys, 0, on, 0, &mpdu), Ok(Opened::NoKey));
        // The pairwise entry's key ID is 0, as the supplicant installs it. One under key ID 1 or
        // 2, whose MIC the key ID does not change, is refused.
        for id in [1, 2, 3] {
            let mut other = mpdu.clone();
            other[27] |= id << 6;
            assert_eq!(open(&keys, 1 << 4, on, 0, &other), Err("rx_key_id"));
        }
    }

    /// With bit 9 of the interface's crypto word, which `hal_crypto_enable` sets for SPP A-MSDU,
    /// a QoS frame's AAD keeps its A-MSDU Present bit. Without it, the bit is masked.
    #[test]
    fn spp_keeps_the_a_msdu_present_bit_in_the_aad() {
        let mut header = hex(HEADER_24);
        header[0] = 0x88;
        header[4] = 0x0e;
        header.extend([0x83, 0]);
        let pn = [1, 0, 0, 0, 0, 0];
        let (plain, _) = aad_and_nonce(&header, pn, false);
        let (spp, _) = aad_and_nonce(&header, pn, true);
        assert_eq!(plain[plain.len() - 2..], [3, 0]);
        assert_eq!(spp[spp.len() - 2..], [0x83, 0]);

        let tk = hex(TK);
        let keys = table(4, &header[10..16], STOCK_PAIRWISE, &tk);
        let mut buffer = header.clone();
        buffer.extend([1, 0, 0, EXT_IV, 0, 0, 0, 0]);
        buffer.extend(hex(PLAINTEXT));
        buffer.extend([0; MIC]);
        let with = |_: u32| 0x30303;
        let Ok(Sealed::Mpdu(sealed)) = seal(&keys, 1 << 4, with, 4, &buffer) else {
            panic!();
        };
        assert_ne!(
            Ok(Sealed::Mpdu(sealed.clone())),
            seal(&keys, 1 << 4, on, 4, &buffer)
        );
        assert!(matches!(
            open(&keys, 1 << 4, with, 0, &sealed),
            Ok(Opened::Mpdu(_))
        ));
        assert_eq!(open(&keys, 1 << 4, on, 0, &sealed), Ok(Opened::Failed));
    }

    /// Each word 1 `hal_crypto_set_key_entry` writes, and each crypto, valid and init word the
    /// `hal_crypto_*` functions write, is kept. Any other is refused by name.
    #[test]
    fn a_store_no_blob_function_makes_to_the_key_table_or_its_words_is_refused() {
        let word1 = |n: u32| KEYS + 40 * n + 4;
        let wep = 1 << 27 | 7 << 21 | 1 << 18;
        let gcmp = 1 << 27 | 3 << 21 | 5 << 18;
        let kept = [
            (word1(4), STOCK_PAIRWISE | 0x0165),
            (word1(0), STOCK_GROUP | 0x0165),
            (word1(24), STOCK_PAIRWISE | 1 << 24),
            (word1(4), 0),
            (word1(0), wep | WEP104),
            (word1(4), wep),
            (word1(4), gcmp | LONG),
            (word1(4), 2 << 18 | 1 << 27 | 3 << 21),
            // A group key with key ID 3 has bit 27 clear.
            (word1(1), 3 << 30 | 6 << 21 | 3 << 18),
            (CONTROL, 0),
            (CONTROL, 0x30000),
            (CONTROL, 0x30103),
            (CONTROL + 4, 0x30103),
            (CONTROL, 0x30303),
            (CONTROL, 0x1003_0103),
            (CONTROL, 0x8003_0103),
            (CONTROL + 8, 0),
            (CONTROL + 12, 0),
            (SMS4, 0),
            (SMS4, 0x003f_ffc0),
            (VALID, 0x11),
            (VALID, VALID_BITS),
            (INIT, 0),
            (INIT, 25),
        ];
        for (addr, value) in kept {
            assert_eq!(refused(addr, value), None, "{addr:#x} {value:#x}");
        }
        let refusals = [
            (word1(4), STOCK_PAIRWISE | 1 << 17, "key_entry"),
            (word1(4), STOCK_PAIRWISE | 1 << 28, "key_entry"),
            (word1(4), STOCK_PAIRWISE | 1 << 29, "key_entry"),
            (word1(4), STOCK_PAIRWISE & !(1 << 21), "key_entry"),
            (word1(4), STOCK_PAIRWISE & !(1 << 22), "key_entry"),
            (word1(4), STOCK_PAIRWISE & !(1 << 27), "key_entry"),
            (word1(4), STOCK_PAIRWISE | GROUP, "key_entry"),
            (word1(4), STOCK_PAIRWISE & !(1 << 22) | GROUP, "key_entry"),
            (word1(4), STOCK_PAIRWISE | LONG, "key_entry"),
            (word1(4), STOCK_PAIRWISE | WEP104, "key_entry"),
            (word1(4), STOCK_PAIRWISE | 3 << 30, "key_entry"),
            (word1(4), STOCK_GROUP, "key_entry"),
            (word1(0), STOCK_PAIRWISE, "key_entry"),
            (word1(0), 0xffff_ffff, "key_entry"),
            (CONTROL, 0x103, "crypto_control"),
            (CONTROL, 0xffff_ffff, "crypto_control"),
            (CONTROL, 0x30107, "crypto_control"),
            (CONTROL, 0x30101, "crypto_control"),
            (CONTROL, 0x30200, "crypto_control"),
            (CONTROL, 0x8003_0000, "crypto_control"),
            (CONTROL, 0x4003_0103, "crypto_control"),
            (CONTROL + 4, 0x3_0103 | 1 << 18, "crypto_control"),
            (CONTROL + 8, 0x30103, "crypto_control"),
            (CONTROL + 12, 0x30000, "crypto_control"),
            (SMS4, 1, "crypto_control"),
            (SMS4, 1 << 22, "crypto_control"),
            (VALID, 1 << 25, "key_valid"),
            (VALID, 1 << 31, "key_valid"),
            (INIT, 24, "crypto_init"),
            (INIT, 1 << 31, "crypto_init"),
        ];
        for (addr, value, name) in refusals {
            assert_eq!(refused(addr, value), Some(name), "{addr:#x} {value:#x}");
        }
        // The address and the key words take any value, and so do words past the table.
        for addr in [KEYS, KEYS + 8, KEYS + 40 * 4 + 24, KEYS + 40 * 25 + 4] {
            assert_eq!(refused(addr, 0xdead_beef), None, "{addr:#x}");
        }
    }

    #[test]
    fn a_group_frame_takes_the_group_entry_of_its_key_id_whatever_its_address() {
        let tk = [7; 16];
        let keys = table(0, &[9; 6], STOCK_GROUP, &tk);
        // From the DS to broadcast, sealed under key ID 1.
        let mut header =
            hex("08 42 00 00 ff ff ff ff ff ff 02 6f 72 62 65 01 02 6f 72 62 65 01 10 00");
        let pn = [5, 0, 0, 0, 0, 0];
        let (aad, nonce) = aad_and_nonce(&header, pn, false);
        let sealed = ccm::seal(&AesKey::new(&tk).unwrap(), &nonce, &aad, b"group", MIC);
        header.extend([5, 0, 0, EXT_IV | 1 << 6, 0, 0, 0, 0]);
        let mut mpdu = header.clone();
        mpdu.extend(&sealed);
        let Ok(Opened::Mpdu(opened)) = open(&keys, 1, on, 0, &mpdu) else {
            panic!();
        };
        assert_eq!(&opened[32..37], b"group");
        // Key ID 2 is not entry 0's.
        mpdu[27] = EXT_IV | 2 << 6;
        assert_eq!(open(&keys, 1, on, 0, &mpdu), Ok(Opened::NoKey));
    }

    #[test]
    fn a_cipher_or_a_frame_the_model_does_not_run_is_named() {
        let mut header = hex(HEADER_24);
        header[4] = 0x0e;
        let mut buffer = header.clone();
        buffer.extend([0; 8 + 4 + 8]);
        for (cipher, tx, rx) in [
            (1, "tx_wep", "rx_wep"),
            (2, "tx_tkip", "rx_tkip"),
            (4, "tx_sms4", "rx_sms4"),
            (5, "tx_gcmp", "rx_gcmp"),
        ] {
            let keys = table(4, &header[10..16], 1 << 21 | cipher << 18, &[0; 16]);
            assert_eq!(seal(&keys, 1 << 4, on, 4, &buffer), Err(tx));
            assert_eq!(open(&keys, 1 << 4, on, 0, &buffer), Err(rx));
        }
        // A CCMP-128 key fills words 2 to 5 only.
        let mut long = table(4, &header[10..16], STOCK_PAIRWISE, &[0; 16]);
        long[4 * WORDS + 6] = 0xdead_beef;
        assert_eq!(seal(&long, 1 << 4, on, 4, &buffer), Err("tx_key_words"));
        assert_eq!(open(&long, 1 << 4, on, 0, &buffer), Err("rx_key_words"));
        let keys = table(4, &header[10..16], STOCK_PAIRWISE, &[0; 16]);
        let mut management = buffer.clone();
        management[0] = 0xd0;
        assert_eq!(seal(&keys, 1 << 4, on, 4, &management), Err("tx_mfp"));
        assert_eq!(open(&keys, 1 << 4, on, 0, &management), Err("rx_mfp"));
        let mut ht = buffer.clone();
        ht[0] = 0x88;
        ht[1] |= ORDER;
        assert_eq!(seal(&keys, 1 << 4, on, 4, &ht), Err("tx_ht_control"));
    }
}
