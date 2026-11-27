//! The 4-way handshake and CCMP-128 against a real one: Wireshark's `wpa-Induction.pcap`
//! (https://wiki.wireshark.org/SampleCaptures), SSID "Coherer", passphrase "Induction". The
//! repo does not commit it: `packages/engine/scripts/blobs.sh --induction` fetches it, and the
//! test skips without it.
//!
//! The network takes the capture's ANonce as its key counter, and its RSNE and GTK, which are
//! for a TKIP group key, from the capture. The station's frames go in through `hear` as the
//! capture has them. Messages 1 and 3 are then the capture's, octet for octet, but for their
//! Duration and Sequence Control, and for the PMKID in message 1, which in the capture is not
//! the one IEEE 802.11-2020 12.7.1.3 derives. The Duration differs because this access point
//! sends EAPOL-Key frames at 1 Mb/s, its lowest basic rate, as Linux mac80211 does, where the
//! capture's 2005 Linksys sent them at 54. The network's sending message 3 is the check of
//! message 2's MIC, its Handshake row the check of message 4's, and every Data frame the
//! station protected comes out with its MIC good.

mod common;

use std::path::PathBuf;

use common::{Air, BSSID, STA, Script, ms};
use orbe_crypto::aes::AesKey;
use orbe_model::Sha256;
use orbe_model::air::{AirFrame, Rate};
use orbe_net::{Config, Network};

/// The capture's SHA-256, its name in the blob cache.
const SHA256: &str = "2b57dca7fa2c3bd0e942060b546028d961bfb698fb12ed8b2947b13f88d170c8";

/// The capture from the folder `blobs.sh` fills: `$ORBE_BLOBS`, else
/// `$XDG_CACHE_HOME/orbe/blobs`, else `~/.cache/orbe/blobs`.
fn capture() -> Option<Vec<u8>> {
    let set = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    let folder = set("ORBE_BLOBS").map(PathBuf::from).unwrap_or_else(|| {
        set("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(set("HOME").expect("HOME")).join(".cache"))
            .join("orbe/blobs")
    });
    let path = folder.join(SHA256);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!(
            "skipped: wpa-Induction.pcap ({SHA256}) is not in {}; run \
             packages/engine/scripts/blobs.sh --induction",
            folder.display()
        );
        return None;
    };
    let mut sha = Sha256::new();
    sha.update(&bytes);
    let hex: String = sha.finish().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, SHA256, "{}", path.display());
    Some(bytes)
}

/// The CRC-32 of IEEE 802.3, the FCS.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// The MPDUs of a little-endian pcap of radiotap frames, each without its FCS, by frame number
/// from 1. A frame whose FCS fails is `None`: no receiver took it.
fn mpdus(pcap: &[u8]) -> Vec<Option<Vec<u8>>> {
    let u16_at = |at: usize| u16::from_le_bytes([pcap[at], pcap[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(pcap[at..at + 4].try_into().unwrap());
    assert_eq!(u32_at(0), 0xa1b2_c3d4);
    // LINKTYPE_IEEE802_11_RADIOTAP.
    assert_eq!(u32_at(20), 127);
    let mut out = vec![None];
    let mut at = 24;
    while at < pcap.len() {
        let len = u32_at(at + 8) as usize;
        let packet = at + 16;
        at = packet + len;
        // Its radiotap header has no TSFT, so Flags is its first field, and it says the FCS is
        // at the end.
        let radiotap = usize::from(u16_at(packet + 2));
        assert_eq!(u32_at(packet + 4) & 3, 2);
        assert_ne!(pcap[packet + 8] & 0x10, 0);
        let mpdu = &pcap[packet + radiotap..at - 4];
        let fcs = u32_at(at - 4);
        out.push((crc32(mpdu) == fcs).then(|| mpdu.to_vec()));
    }
    out
}

/// The EAPOL frame of a Data frame carrying one.
fn eapol(mpdu: &[u8]) -> Option<&[u8]> {
    (mpdu[0] == 0x08 && mpdu.get(24..32)? == common::hex("aaaa03000000888e")).then(|| &mpdu[32..])
}

#[test]
fn the_handshake_and_data_of_wpa_induction_replay() {
    let Some(pcap) = capture() else {
        return;
    };
    let frames = mpdus(&pcap);
    let frame = |n: usize| frames[n].as_deref().expect("a frame with a good FCS");
    // Frames 87 and 92 are messages 1 and 3 from the access point, 89 and 94 messages 2 and 4.
    let (theirs_1, theirs_3) = (frame(87), frame(92));
    let (m1, m2, m3) = (
        eapol(theirs_1).unwrap(),
        eapol(frame(89)).unwrap(),
        eapol(theirs_3).unwrap(),
    );
    assert_eq!(eapol(frame(94)).unwrap()[5..7], [0x03, 0x0a]);
    let anonce: [u8; 32] = m1[17..49].try_into().unwrap();
    let snonce = &m2[17..49];

    // The GTK and its Key ID from message 3's key data, unwrapped under the KEK, and its PN
    // from the Key RSC. Its RSNE is the beacons' (frame 1).
    let mut pmk = [0; 32];
    orbe_crypto::pbkdf2::pbkdf2_sha1(common::PASSPHRASE.as_bytes(), common::SSID, 4096, &mut pmk);
    let mut data = Vec::new();
    data.extend(BSSID.0.min(STA.0));
    data.extend(BSSID.0.max(STA.0));
    data.extend(anonce.min(snonce.try_into().unwrap()));
    data.extend(anonce.max(snonce.try_into().unwrap()));
    let mut ptk = [0; 48];
    orbe_crypto::prf::prf(&pmk, b"Pairwise key expansion", &data, &mut ptk);
    let kek = AesKey::new(&ptk[16..32]).unwrap();
    let key_data = orbe_crypto::key_wrap::unwrap(&kek, &m3[99..]).unwrap();
    let rsne = &key_data[2..2 + usize::from(key_data[1])];
    let beacon = common::elements(frame(1));
    assert!(beacon.iter().any(|(id, body)| *id == 48 && body == rsne));
    let kde = &key_data[2 + rsne.len()..];
    assert_eq!(kde[2..6], [0x00, 0x0f, 0xac, 1]);
    let gtk = &kde[8..2 + usize::from(kde[1])];
    let index = kde[6] & 3;
    let pn = u64::from_le_bytes(m3[65..73].try_into().unwrap());
    let config = Config::new(common::SSID, BSSID, common::channel(), common::WPA2)
        .unwrap()
        .with_counter(anonce)
        .with_group(rsne, gtk, index, pn);

    // Every frame the station sent that a receiver took, 10 ms apart: its probes, its
    // authentication and association, messages 2 and 4, its Data frames and its leaving.
    let from_station: Vec<&[u8]> = (frames.iter().flatten())
        .filter(|f| f.len() >= 16 && f[10..16] == STA.0)
        .map(|f| f.as_slice())
        .collect();
    let mut script = Script::default();
    for (k, f) in from_station.iter().enumerate() {
        script = script.send(ms(10.0 * (k + 1) as f64), f.to_vec());
    }
    let mut air = Air::new(script);
    let mut net = Network::new(config);
    air.run_through(&mut net, ms(10.0 * (from_station.len() + 1) as f64));

    let ours: Vec<&AirFrame> = (air.sent().into_iter())
        .filter(|f| eapol(&f.mpdu).is_some())
        .collect();
    assert_eq!(ours.len(), 2, "messages 1 and 3, each once");
    // Ours go at 1 Mb/s, the lowest basic rate, as Linux mac80211 sends EAPOL-Key frames, with
    // Duration 314 for SIFS and the 1 Mb/s ACK. The capture's went at 54 Mb/s with Duration 44.
    for f in &ours {
        assert_eq!(
            (f.rate, &f.mpdu[2..4]),
            (Rate::Dsss1, &314u16.to_le_bytes()[..])
        );
    }
    for theirs in [theirs_1, theirs_3] {
        assert_eq!(theirs[2..4], 44u16.to_le_bytes());
    }
    let ours: Vec<&[u8]> = ours.iter().map(|f| &f.mpdu[..]).collect();
    // Message 1's last 16 octets are the PMKID of its PMKID KDE. Ours is the PMKID of
    // 12.7.1.3, HMAC-SHA1-128(PMK, "PMK Name" || AA || SPA). The capture's is not, so it is not
    // compared.
    let pmkid = {
        let mut name = b"PMK Name".to_vec();
        name.extend(BSSID.0);
        name.extend(STA.0);
        orbe_crypto::hmac::hmac_sha1(&pmk, &name)[..16].to_vec()
    };
    let at = theirs_1.len() - 16;
    assert_eq!(ours[0][at..], pmkid);
    assert_ne!(theirs_1[at..], pmkid);
    let mut ours_1 = ours[0].to_vec();
    ours_1[at..].copy_from_slice(&theirs_1[at..]);
    for (ours, theirs) in [(&ours_1[..], theirs_1), (ours[1], theirs_3)] {
        assert_eq!(ours.len(), theirs.len());
        // All but the Duration and the Sequence Control.
        assert_eq!(ours[..2], theirs[..2]);
        assert_eq!(ours[4..22], theirs[4..22]);
        assert_eq!(ours[24..], theirs[24..]);
    }
    let rows = air.rows_seen();
    assert!(
        rows.iter()
            .any(|r| r.contains(r#""event":"assoc""#) && r.contains(r#""status":0"#))
    );
    assert_eq!(rows.iter().filter(|r| r.contains("handshake")).count(), 1);

    // Each protected Data frame to the access point once, by PN: a retry the capture also
    // holds is the same frame again.
    let mut pns: Vec<u64> = (from_station.iter())
        .filter(|f| f[0] == 0x08 && f[1] & 0x43 == 0x41 && f[4..10] == BSSID.0)
        .map(|f| u64::from_le_bytes([f[24], f[25], f[28], f[29], f[30], f[31], 0, 0]))
        .collect();
    pns.dedup();
    assert!(pns.len() > 100);
    let mut delivered = Vec::new();
    net.take_delivered(|sta, msdu| delivered.push((sta, msdu)));
    assert_eq!(delivered.len(), pns.len());
    for (sta, msdu) in delivered {
        assert_eq!(sta, STA);
        // LLC with SNAP, most with EtherTypes and some with AppleTalk's OUI.
        assert_eq!(msdu[..3], [0xaa, 0xaa, 0x03]);
    }
}
