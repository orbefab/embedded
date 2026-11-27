//! WPA2-PSK: the 4-way handshake with the access point as the authenticator, then CCMP-128.
//! Clauses are IEEE 802.11-2020's.

mod common;

use common::{
    ANONCE, Air, BSSID, RSN_PSK, STA, Script, Supplicant, eapol_sent, hex, keyed_network, ms,
};
use orbe_model::Time;
use orbe_net::{Config, Mac, Network, Security};

fn run(script: Script, until: Time, mut net: Network) -> (Air, Network) {
    let mut air = Air::new(script);
    air.run_through(&mut net, until);
    (air, net)
}

fn delivered(net: &mut Network) -> Vec<(Mac, Vec<u8>)> {
    let mut out = Vec::new();
    net.take_delivered(|sta, msdu| out.push((sta, msdu)));
    out
}

/// The station joins: authentication at 10 ms and association at 20 ms.
fn joined() -> Script {
    Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .send(ms(20.0), common::assoc_ccmp())
}

fn replay(eapol: &[u8]) -> u64 {
    u64::from_be_bytes(eapol[9..17].try_into().unwrap())
}

// 12.7.6: messages 1 to 4, then the station's Data frames under its TK. A frame whose PN is
// not past the last one taken, a frame that fails its MIC, and a frame sent in the clear are
// dropped (12.5.3.4.4).
#[test]
fn a_station_with_the_passphrase_gets_its_keys() {
    let sta = Supplicant::new(common::PASSPHRASE);
    let mut tampered = sta.data(&ANONCE, 3, b"three");
    *tampered.last_mut().unwrap() ^= 1;
    let mut clear = hex("08013a01");
    clear.extend(BSSID.0);
    clear.extend(STA.0);
    clear.extend(Mac::BROADCAST.0);
    clear.extend(hex("0000 aaaa030000000800"));
    clear.extend(b"clear");
    let script = joined()
        .send(ms(30.0), sta.message_2(&ANONCE, 0, &RSN_PSK))
        .send(ms(40.0), sta.message_4(&ANONCE, 1))
        .send(ms(50.0), sta.data(&ANONCE, 1, b"one"))
        .send(ms(60.0), sta.data(&ANONCE, 2, b"two"))
        .send(ms(70.0), sta.data(&ANONCE, 2, b"two"))
        .send(ms(80.0), sta.data(&ANONCE, 1, b"one"))
        .send(ms(90.0), tampered)
        .send(ms(100.0), clear)
        .send(ms(110.0), sta.data(&ANONCE, 3, b"three"));
    let (air, mut net) = run(script, ms(120.0), keyed_network());

    let eapol = eapol_sent(&air);
    assert_eq!(eapol.len(), 2);
    // Message 1: Pairwise and Ack, Key Length 16, Key Replay Counter 0, the ANonce, and the
    // PMKID KDE.
    assert_eq!(eapol[0][..9], hex("02 03 0075 02 008a 0010"));
    assert_eq!(replay(&eapol[0]), 0);
    assert_eq!(eapol[0][17..49], ANONCE);
    assert_eq!(eapol[0][97..99], [0, 22]);
    assert_eq!(eapol[0][99..105], hex("dd14000fac04"));

    // Message 3: Install, Ack, MIC, Secure and Encrypted Key Data, the next counter, the
    // ANonce again, the Key IV from the key counter, and a MIC under the KCK.
    let m3 = &eapol[1];
    assert_eq!(m3[5..7], hex("13ca"));
    assert_eq!(replay(m3), 1);
    assert_eq!(m3[17..49], ANONCE);
    assert_eq!(m3[49..65], common::anonce_plus(1)[16..]);
    let ptk = sta.ptk(&ANONCE);
    let mut zeroed = m3.clone();
    zeroed[81..97].fill(0);
    assert_eq!(
        orbe_crypto::hmac::hmac_sha1(&ptk[..16], &zeroed)[..16],
        m3[81..97]
    );
    // Its key data, unwrapped under the KEK: the RSNE of the beacons, then a GTK KDE with Key
    // ID 1 and a 16-octet GTK, padded to a multiple of 8.
    let kek = orbe_crypto::aes::AesKey::new(&ptk[16..32]).unwrap();
    let data = orbe_crypto::key_wrap::unwrap(&kek, &m3[99..]).unwrap();
    assert_eq!(data[..2], [48, 20]);
    assert_eq!(data[2..22], RSN_PSK);
    assert_eq!(data[22..30], hex("dd16000fac010100"));
    assert_eq!(data[46..], [0xdd, 0]);

    let handshake = format!(r#"{{"t_ps":40000000000,"event":"handshake","sta":"{STA}"}}"#);
    assert!(air.rows_seen().contains(&handshake.as_str()));
    let payloads: Vec<_> = (delivered(&mut net).into_iter())
        .map(|(from, msdu)| {
            assert_eq!(from, STA);
            assert_eq!(msdu[..8], hex("aaaa030000000800"));
            msdu[8..].to_vec()
        })
        .collect();
    assert_eq!(payloads, [&b"one"[..], b"two", b"three"]);
}

// 12.7.5: with no counter forced, the ANonce comes from the network's fixed seed, so every run
// sends the same one, and a station that reads it from message 1 gets its keys.
#[test]
fn the_default_anonce_is_fixed_and_works() {
    let network =
        || Network::new(Config::new(common::SSID, BSSID, common::channel(), common::WPA2).unwrap());
    let (air, _) = run(joined(), ms(30.0), network());
    let anonce: [u8; 32] = eapol_sent(&air)[0][17..49].try_into().unwrap();
    assert_ne!(anonce, [0; 32]);
    let (again, _) = run(joined(), ms(30.0), network());
    assert_eq!(eapol_sent(&again)[0][17..49], anonce);

    let sta = Supplicant::new(common::PASSPHRASE);
    let script = joined()
        .send(ms(30.0), sta.message_2(&anonce, 0, &RSN_PSK))
        .send(ms(40.0), sta.message_4(&anonce, 1))
        .send(ms(50.0), sta.data(&anonce, 1, b"one"));
    let (air, mut net) = run(script, ms(60.0), network());
    assert!(air.rows_seen().iter().any(|r| r.contains("handshake")));
    assert_eq!(delivered(&mut net).len(), 1);
}

// A wrong passphrase, as on a real access point: message 2's MIC fails, so the access point
// sends message 1 again 100 ms after its first send and 1 s after each later one, and 1 s after
// its fourth send it deauthenticates the station with reason 15, 4-way handshake timeout. The
// station never gets message 3.
#[test]
fn a_wrong_passphrase_times_out_with_reason_15() {
    let sta = Supplicant::new("Inductiom");
    let mut script = joined();
    for (k, at) in [25.0, 125.0, 1125.0, 2125.0].into_iter().enumerate() {
        script = script.send(ms(at), sta.message_2(&ANONCE, k as u64, &RSN_PSK));
    }
    let script = script.send(ms(3200.0), sta.data(&ANONCE, 1, b"one"));
    let (air, mut net) = run(script, ms(3300.0), keyed_network());

    let sent = air.sent();
    let message_1: Vec<_> = (sent.iter())
        .filter(|f| f.mpdu.get(32..34) == Some(&[2, 3]))
        .collect();
    assert_eq!(message_1.len(), 4);
    for (k, m1) in message_1.iter().enumerate() {
        assert_eq!(m1.mpdu[37..39], hex("008a"));
        assert_eq!(replay(&m1.mpdu[32..]), k as u64);
        assert_eq!(m1.mpdu[49..81], ANONCE);
    }
    // The first goes after the Association Response, and the others on their timeouts, with
    // the medium idle.
    assert!(message_1[0].start > ms(20.0) && message_1[0].end < ms(25.0));
    let starts: Vec<Time> = message_1[1..].iter().map(|f| f.start).collect();
    assert_eq!(starts, [ms(120.0), ms(1120.0), ms(2120.0)]);

    let rows = air.rows_seen();
    assert!(!rows.iter().any(|r| r.contains("handshake")));
    // Its Data frame after that is a Class 3 frame from a station that is not authenticated.
    let deauth = |at: Time, reason| {
        let t = at.as_ps();
        format!(r#"{{"t_ps":{t},"event":"deauth","sta":"{STA}","reason":{reason},"by":"ap"}}"#)
    };
    let deauths: Vec<&str> = rows.into_iter().filter(|r| r.contains("deauth")).collect();
    assert_eq!(deauths, [deauth(ms(3120.0), 15), deauth(ms(3200.0), 7)]);
    let reasons: Vec<&[u8]> = (sent.iter())
        .filter(|f| f.mpdu[0] == 0xc0)
        .map(|f| &f.mpdu[24..])
        .collect();
    assert_eq!(reasons, [[15, 0], [7, 0]]);
    assert!(delivered(&mut net).is_empty());
}

// 12.7.6.3: a message 2 with a good MIC whose RSNE is not the one of the Association Request
// ends the association with reason 17. One that answers no message 1 is dropped.
#[test]
fn message_2_must_repeat_the_rsne_and_answer_message_1() {
    let sta = Supplicant::new(common::PASSPHRASE);
    let mut other = RSN_PSK;
    other[18] = 0x80;
    let script = joined()
        .send(ms(30.0), sta.message_2(&ANONCE, 1, &RSN_PSK))
        .send(ms(40.0), sta.message_2(&ANONCE, 0, &other));
    let (air, _) = run(script, ms(50.0), keyed_network());
    assert_eq!(eapol_sent(&air).len(), 1);
    let deauth =
        format!(r#"{{"t_ps":40000000000,"event":"deauth","sta":"{STA}","reason":17,"by":"ap"}}"#);
    assert_eq!(air.rows_seen().last(), Some(&deauth.as_str()));
}

// 12.7.6.5: a message 4 that answers no message 3, or fails its MIC, is dropped, and message 3
// goes again with the next Key Replay Counter and the next Key IV.
#[test]
fn message_3_goes_again_until_message_4_checks() {
    let sta = Supplicant::new(common::PASSPHRASE);
    let mut bad = sta.message_4(&ANONCE, 1);
    bad[32 + 81] ^= 1;
    let script = joined()
        .send(ms(30.0), sta.message_2(&ANONCE, 0, &RSN_PSK))
        .send(ms(40.0), sta.message_4(&ANONCE, 0))
        .send(ms(50.0), bad)
        .send(ms(140.0), sta.message_4(&ANONCE, 2));
    let (air, _) = run(script, ms(150.0), keyed_network());
    let eapol = eapol_sent(&air);
    assert_eq!(eapol.len(), 3);
    assert_eq!(replay(&eapol[2]), 2);
    assert_eq!(eapol[2][49..65], common::anonce_plus(2)[16..]);
    let rows = air.rows_seen();
    let handshake = format!(r#"{{"t_ps":140000000000,"event":"handshake","sta":"{STA}"}}"#);
    assert_eq!(
        rows.iter()
            .filter(|r| r.contains("handshake"))
            .collect::<Vec<_>>(),
        [&handshake]
    );
}

/// The status of the Association Response to an Association Request with RSNE body `rsne`.
fn assoc_status(rsne: &[u8]) -> u16 {
    let script = Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .send(ms(20.0), common::assoc(common::SSID, Some(rsne)));
    let (air, _) = run(script, ms(30.0), keyed_network());
    let response = air.sent()[1].mpdu.clone();
    assert_eq!(response[0], 0x10);
    u16::from_le_bytes([response[26], response[27]])
}

// 12.6.3 and Table 9-50: the access point does neither WPA3-SAE nor PMF. SAE's AKM gets 43,
// invalid AKMP, and a station that requires PMF gets 31, robust management frame policy
// violation. One that is only capable of PMF associates without it. SAE Authentication itself
// gets 13, in ap.rs.
#[test]
fn sae_and_pmf_are_refused_with_their_status_codes() {
    let mut sae = RSN_PSK;
    sae[17] = 8;
    assert_eq!(assoc_status(&sae), 43);
    let mut required = RSN_PSK;
    required[18] = 0xc0;
    assert_eq!(assoc_status(&required), 31);
    let mut capable = RSN_PSK;
    capable[18] = 0x80;
    assert_eq!(assoc_status(&capable), 0);
}

// J.4.1: a passphrase is 8 to 63 ASCII characters from space to `~`.
#[test]
fn a_passphrase_out_of_range_is_refused() {
    fn config(passphrase: &str) -> Option<Config> {
        let security = Security::Wpa2Psk { passphrase };
        Config::new(common::SSID, BSSID, common::channel(), security)
    }
    assert!(config("seven77").is_none());
    assert!(config("eight888").is_some());
    assert!(config(&"x".repeat(63)).is_some());
    assert!(config(&"x".repeat(64)).is_none());
    assert!(config("tab\there!").is_none());
    assert!(config("caf\u{e9}caf\u{e9}").is_none());
}
