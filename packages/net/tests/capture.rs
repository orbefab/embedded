//! The access point's frames against the frames of a real one, from Wireshark's
//! `wpa-Induction.pcap` (https://wiki.wireshark.org/SampleCaptures). That access point beacons
//! "Coherer" on channel 1 at 1 Mb/s and offers WPA and WPA2 with TKIP and CCMP. Where its frames
//! carry what this one sends, the bytes are the same. Frame numbers are the capture's.

mod common;

use common::{Air, STA, Script, capture, elements, hex, ms, network};

/// `frame` without the vendor element the capture's access point puts at its end.
fn without_vendor(frame: &str) -> Vec<u8> {
    let frame = hex(frame);
    let vendor = hex(capture::BROADCOM);
    assert!(frame.ends_with(&vendor));
    frame[..frame.len() - vendor.len()].to_vec()
}

/// `frame` with Sequence Control zeroed, since each access point counts its own.
fn unsequenced(frame: &[u8]) -> Vec<u8> {
    let mut frame = frame.to_vec();
    frame[22..24].fill(0);
    frame
}

/// The frames the network sends a station that sends `frames`, 10 ms apart, beacons left out.
fn answers(frames: &[Vec<u8>]) -> Air {
    let mut script = Script::default();
    for (k, frame) in frames.iter().enumerate() {
        script = script.send(ms(10.0 * (k + 1) as f64), frame.clone());
    }
    let mut air = Air::new(script);
    air.run_through(
        &mut network(common::WPA2),
        ms(10.0 * (frames.len() + 1) as f64),
    );
    air
}

// Frames 78 and 80: the station's Authentication and the access point's answer.
#[test]
fn open_system_authentication_matches_frame_80() {
    let air = answers(&[hex(capture::AUTH_78)]);
    let sent = air.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        unsequenced(&sent[0].mpdu),
        unsequenced(&without_vendor(capture::AUTH_80))
    );
}

// Frames 82 and 84. The capture's station asks for a TKIP group key, which this access point
// does not offer, so it is refused with status 41. With CCMP-128 asked for instead, the answer
// is frame 84: status 0, AID 1 with its two top bits set, and the same rates.
#[test]
fn association_matches_frame_84() {
    let air = answers(&[hex(capture::AUTH_78), hex(capture::ASSOC_82)]);
    let refused = &air.sent()[1].mpdu;
    assert_eq!(refused[26..28], 41u16.to_le_bytes());

    let air = answers(&[hex(capture::AUTH_78), common::assoc_ccmp()]);
    assert_eq!(
        unsequenced(&air.sent()[1].mpdu),
        unsequenced(&without_vendor(capture::ASSOC_RESPONSE_84))
    );
    assert_eq!(
        air.rows_seen()[1],
        format!(r#"{{"t_ps":20000000000,"event":"assoc","sta":"{STA}","status":0,"aid":1}}"#)
    );
}

/// The elements both frames carry, by ID, in this network's order.
fn shared(ours: &[u8], theirs: &[u8], ids: &[u8]) {
    let (ours, theirs) = (elements(ours), elements(theirs));
    for &id in ids {
        let find = |list: &[(u8, Vec<u8>)]| list.iter().find(|(e, _)| *e == id).cloned();
        assert_eq!(find(&ours), find(&theirs), "element {id}");
    }
}

// Frames 1 and 59: a beacon and a Probe Response. The header, the Beacon Interval of 100 TU,
// the Capability Information (ESS, Privacy, Short Slot Time), the SSID, both rate elements, the
// DSSS Parameter Set and, in the beacon, the TIM are the capture's. The Timestamp is this
// network's own clock. The capture's ERP element sets Use_Protection, and its RSNE offers TKIP.
#[test]
fn beacon_and_probe_response_match_frames_1_and_59() {
    let air = answers(&[hex(capture::PROBE_58)]);
    let beacon = &air.aired()[0].mpdu;
    let theirs = hex(capture::BEACON_1);
    assert_eq!(unsequenced(&beacon[..24]), unsequenced(&theirs[..24]));
    assert_eq!(beacon[32..36], theirs[32..36]);
    shared(beacon, &theirs, &[0, 1, 3, 5, 50]);

    let response = &air.sent()[0].mpdu;
    let theirs = hex(capture::PROBE_RESPONSE_59);
    assert_eq!(unsequenced(&response[..24]), unsequenced(&theirs[..24]));
    assert_eq!(response[32..36], theirs[32..36]);
    shared(response, &theirs, &[0, 1, 3, 50]);
    assert!(elements(response).iter().all(|(id, _)| *id != 5));

    // IEEE 802.11-2020 9.3.3.2 and 9.3.3.10: the elements in order, and the RSNE for WPA2-PSK.
    let ids: Vec<u8> = elements(beacon).iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, [0, 1, 3, 5, 42, 50, 48]);
    let ids: Vec<u8> = elements(response).iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, [0, 1, 3, 42, 50, 48]);
    assert_eq!(
        elements(beacon)[6].1,
        hex("0100 000fac04 0100 000fac04 0100 000fac02 0000")
    );
}

// Frame 999 asks for any SSID, and the capture's access point answers it in frame 1000.
#[test]
fn a_wildcard_probe_is_answered() {
    let air = answers(&[hex(capture::PROBE_999)]);
    let sent = air.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].mpdu[0], 0x50);
    assert_eq!(sent[0].mpdu[4..10], STA.0);
}

// Frame 1050: the station leaves with reason 8.
#[test]
fn the_station_s_disassociation_ends_its_association() {
    let air = answers(&[
        hex(capture::AUTH_78),
        common::assoc_ccmp(),
        hex(capture::DISASSOC_1050),
    ]);
    assert_eq!(
        air.rows_seen()[2],
        format!(
            r#"{{"t_ps":30000000000,"event":"disassoc","sta":"{STA}","reason":8,"by":"station"}}"#
        )
    );
    // The Authentication, the Association Response and message 1 of the handshake. Nothing
    // answers the Disassociation.
    assert_eq!(air.sent().len(), 3);
}
