//! What the access point does, frame by frame. Clauses are IEEE 802.11-2020's.

mod common;

use common::{Air, BSSID, STA, Script, by, hex, ms, network};
use orbe_model::Time;
use orbe_model::air::{Ack, Rate};
use orbe_net::{Mac, Security};

const OTHER: Mac = Mac([0x02, 0, 0, 0, 0, 0x02]);

fn run(script: Script, until: Time, security: Security) -> Air {
    let mut air = Air::new(script);
    air.run_through(&mut network(security), until);
    air
}

fn row(at: Time, rest: &str) -> String {
    format!(r#"{{"t_ps":{},"event":{rest}}}"#, at.as_ps())
}

/// The Status Code of an Authentication or Association Response.
fn status(mpdu: &[u8]) -> u16 {
    let at = if mpdu[0] == 0xb0 { 28 } else { 26 };
    u16::from_le_bytes([mpdu[at], mpdu[at + 1]])
}

// 11.1.3 and 9.4.1.10: a beacon at each TBTT, 100 TU apart, whose Timestamp is the TSF when its
// first bit is on the air, after the 192 us PLCP and 24 octets of header at 1 Mb/s.
#[test]
fn it_beacons_every_100_tu() {
    let air = run(Script::default(), ms(350.0), common::WPA2);
    let beacons = air.aired();
    assert_eq!(beacons.len(), 4);
    for (k, beacon) in beacons.iter().enumerate() {
        let start_us = 102_400 * k as u64;
        assert_eq!(beacon.start.as_ps(), start_us * 1_000_000);
        assert_eq!(
            beacon.end.as_ps() - beacon.start.as_ps(),
            Rate::Dsss1.airtime(beacon.mpdu.len() + 4)
        );
        assert_eq!(beacon.mpdu[24..32], (start_us + 384).to_le_bytes());
        assert_eq!(beacon.mpdu[22..24], ((k as u16) << 4).to_le_bytes());
    }
}

// 10.3.2.3: the access point answers after SIFS, the ACK it gives, and DIFS, with no backoff.
// A 1 Mb/s frame's ACK is 304 us, and DIFS with short slots is 28 us.
#[test]
fn it_answers_after_the_ack_and_difs() {
    let air = run(
        Script::default().send(ms(20.0), hex(common::capture::AUTH_78)),
        ms(30.0),
        common::WPA2,
    );
    assert_eq!(air.seen[1], common::Seen::Heard(ms(20.0), Ack::Acked));
    let answer = air.sent()[0];
    assert_eq!(
        answer.start.as_ps(),
        ms(20.0).as_ps() + (10 + 304 + 28) * 1_000_000
    );
}

// 11.3.5.3: each station that associates takes the lowest free AID, and keeps it when it
// associates again.
#[test]
fn stations_take_the_lowest_free_aid() {
    let script = Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .send(ms(11.0), by(OTHER, common::auth(0, 1)))
        .send(ms(12.0), by(OTHER, common::assoc_ccmp()))
        .send(ms(13.0), common::assoc_ccmp())
        .send(ms(14.0), common::assoc_ccmp());
    let air = run(script, ms(20.0), common::WPA2);
    let assoc = |at, sta: Mac, aid| {
        row(
            at,
            &format!(r#""assoc","sta":"{sta}","status":0,"aid":{aid}"#),
        )
    };
    assert_eq!(
        air.rows_seen(),
        [
            row(ms(10.0), &format!(r#""auth","sta":"{STA}","status":0"#)),
            row(ms(11.0), &format!(r#""auth","sta":"{OTHER}","status":0"#)),
            assoc(ms(12.0), OTHER, 1),
            assoc(ms(13.0), STA, 2),
            assoc(ms(14.0), STA, 2),
        ]
    );
}

// Status codes of Table 9-50 and reason codes of Table 9-49.
#[test]
fn what_it_refuses() {
    let script = Script::default()
        // SAE, algorithm 3: not supported, 13.
        .send(ms(10.0), common::auth(3, 1))
        // Open System out of sequence: 14.
        .send(ms(11.0), common::auth(0, 3))
        // Association before authentication: Deauthentication, reason 6 (11.3.3).
        .send(ms(12.0), common::assoc_ccmp())
        .send(ms(13.0), common::auth(0, 1))
        // Another SSID: 1.
        .send(ms(14.0), common::assoc(b"Other", Some(&common::RSN_PSK)))
        // No RSNE under WPA2-PSK: 40.
        .send(ms(15.0), common::assoc(common::SSID, None));
    let air = run(script, ms(20.0), common::WPA2);
    let sent = air.sent();
    assert_eq!(status(&sent[0].mpdu), 13);
    assert_eq!(sent[0].mpdu[26..28], 2u16.to_le_bytes());
    assert_eq!(status(&sent[1].mpdu), 14);
    assert_eq!(sent[2].mpdu[0], 0xc0);
    assert_eq!(sent[2].mpdu[24..], 6u16.to_le_bytes());
    assert_eq!(status(&sent[3].mpdu), 0);
    assert_eq!(status(&sent[4].mpdu), 1);
    assert_eq!(status(&sent[5].mpdu), 40);
    // A refused association carries AID 0.
    assert_eq!(sent[5].mpdu[28..30], [0, 0]);

    // An open network asks for no RSNE and advertises none, and its Capability has no Privacy.
    let script = Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .send(ms(11.0), common::assoc(common::SSID, None));
    let air = run(script, ms(20.0), Security::Open);
    assert_eq!(status(&air.sent()[1].mpdu), 0);
    assert_eq!(air.sent()[1].mpdu[24..26], 0x0401u16.to_le_bytes());
    let beacon = &air.aired()[0].mpdu;
    assert!(common::elements(beacon).iter().all(|(id, _)| *id != 48));
}

// Status 17 once it keeps as many stations as it can.
#[test]
fn a_full_access_point_refuses_another_station() {
    let mut script = Script::default();
    for k in 0..=orbe_net::MAX_STATIONS {
        let sta = Mac([0x02, 0, 0, 0, 1, k as u8]);
        script = script.send(ms(10.0 + k as f64), by(sta, common::auth(0, 1)));
    }
    let air = run(script, ms(30.0), common::WPA2);
    let statuses: Vec<u16> = air.sent().iter().map(|f| status(&f.mpdu)).collect();
    let mut wanted = vec![0; orbe_net::MAX_STATIONS];
    wanted.push(17);
    assert_eq!(statuses, wanted);
}

// 9.6.4.3: the ADDBA Response echoes the Dialog Token, the Block Ack Parameter Set and the
// timeout, with status 37, so no block ack agreement is made.
#[test]
fn addba_is_declined() {
    let script = Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .send(ms(11.0), common::assoc_ccmp())
        .send(ms(12.0), common::addba(7, 5, false));
    let air = run(script, ms(20.0), Security::Open);
    let response = &air.sent()[2].mpdu;
    let mut wanted = hex("d0003a01");
    wanted.extend(STA.0);
    wanted.extend(BSSID.0);
    wanted.extend(BSSID.0);
    wanted.extend(&response[22..24]);
    wanted.extend(hex("0301 07 2500 1610 0000"));
    assert_eq!(response[..], wanted[..]);
    assert_eq!(
        air.rows_seen()[2],
        row(
            ms(12.0),
            &format!(r#""addba_declined","sta":"{STA}","tid":5"#)
        )
    );
}

// 11.3.3: a Class 3 frame from a station that has authenticated and not associated gets a
// Disassociation, and from one that has not authenticated a Deauthentication, reason 7.
#[test]
fn class_3_frames_from_a_station_not_associated_are_refused() {
    let script = Script::default()
        .send(ms(10.0), common::null(false))
        .send(ms(11.0), common::auth(0, 1))
        .send(ms(12.0), common::null(false))
        .send(ms(13.0), common::ps_poll(1));
    let air = run(script, ms(20.0), common::WPA2);
    let sent = air.sent();
    let kinds: Vec<(u8, Vec<u8>)> = sent
        .iter()
        .map(|f| (f.mpdu[0], f.mpdu[24..].to_vec()))
        .collect();
    assert_eq!(
        kinds,
        [
            (0xc0, vec![7, 0]),
            (0xb0, hex("0000 0200 0000")),
            (0xa0, vec![7, 0]),
            (0xa0, vec![7, 0])
        ]
    );
}

/// The TIM element of a beacon.
fn tim(beacon: &[u8]) -> Vec<u8> {
    common::elements(beacon)
        .into_iter()
        .find(|(id, _)| *id == 5)
        .unwrap()
        .1
}

// 11.2.3: the access point holds bufferable frames for a station in power save, shows its AID
// in the TIM, and sends one frame per PS-Poll, with More Data while it holds more, then a Null
// frame. When the station wakes it sends the rest.
#[test]
fn it_holds_frames_for_a_station_in_power_save() {
    let script = Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .send(ms(11.0), common::assoc_ccmp())
        .send(ms(20.0), common::addba(1, 0, true))
        .send(ms(21.0), common::addba(2, 1, true))
        .send(ms(22.0), common::addba(3, 2, true))
        .doze(ms(20.5), ms(102.0))
        .send(ms(104.0), common::ps_poll(1))
        .send(ms(106.0), common::ps_poll(1))
        .doze(ms(108.0), ms(204.0))
        .send(ms(205.0), common::null(false))
        .send(ms(210.0), common::ps_poll(1));
    let air = run(script, ms(220.0), Security::Open);
    let beacons: Vec<_> = air
        .aired()
        .into_iter()
        .filter(|f| f.mpdu[0] == 0x80)
        .collect();
    // DTIM Count 0, DTIM Period 1, Bitmap Control 0, AID 1's bit.
    assert_eq!(tim(&beacons[0].mpdu), [0, 1, 0, 0]);
    assert_eq!(tim(&beacons[1].mpdu), [0, 1, 0, 0x02]);
    assert_eq!(tim(&beacons[2].mpdu), [0, 1, 0, 0x02]);

    let sent = air.sent();
    let after = |t: Time| -> Vec<(u8, u8, u8)> {
        (sent.iter())
            .filter(|f| f.start > t)
            .map(|f| (f.mpdu[0], f.mpdu[1], *f.mpdu.get(26).unwrap_or(&0)))
            .collect()
    };
    // ADDBA Responses by Dialog Token: the first two polled, each with More Data, the third
    // once the station wakes, and a Null frame from the DS for a poll with nothing held.
    assert_eq!(
        after(ms(100.0)),
        [
            (0xd0, 0x20, 1),
            (0xd0, 0x20, 2),
            (0xd0, 0x00, 3),
            (0x48, 0x02, 0)
        ]
    );
    let rows = air.rows_seen();
    assert!(rows.contains(&row(ms(20.0), &format!(r#""doze","sta":"{STA}""#)).as_str()));
    assert!(rows.contains(&row(ms(205.0), &format!(r#""wake","sta":"{STA}""#)).as_str()));
    assert!(!rows.iter().any(|r| r.contains("lost")));
}

// A frame the station does not acknowledge is lost: here the station dozes without saying so.
#[test]
fn a_frame_with_no_ack_is_lost() {
    let script = Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .doze(ms(10.1), ms(20.0));
    let air = run(script, ms(20.0), common::WPA2);
    let answer = air.sent()[0];
    assert_eq!(
        air.seen.last(),
        Some(&common::Seen::Row(row(
            answer.end,
            &format!(r#""lost","sta":"{STA}""#)
        )))
    );
}

// Going down, it sends every station a Deauthentication, reason 3, stops beaconing and stops
// listening. Coming up, it beacons from the next TBTT and knows no station.
#[test]
fn the_access_point_goes_down_and_comes_up() {
    let (script, until) = common::session();
    let mut air = Air::new(script);
    let mut net = network(common::WPA2);
    air.run_through(&mut net, ms(350.0));
    assert_eq!(net.tuned(), None);
    air.run_through(&mut net, until);
    assert_eq!(net.tuned(), Some(common::channel()));

    let aired = air.aired();
    let down: Vec<_> = aired
        .iter()
        .filter(|f| f.start >= ms(300.0) && f.start < ms(500.0))
        .collect();
    assert_eq!(down.len(), 1);
    let mut deauth = hex("c0000000ffffffffffff");
    deauth.extend(BSSID.0);
    deauth.extend(BSSID.0);
    deauth.extend(&down[0].mpdu[22..24]);
    deauth.extend([3, 0]);
    assert_eq!(down[0].mpdu[..], deauth[..]);
    let first = aired.iter().find(|f| f.start > ms(500.0)).unwrap();
    assert_eq!(first.start, Time::from_ps(5 * 102_400_000_000));
    assert_eq!(first.mpdu[0], 0x80);
    assert!(
        air.seen
            .contains(&common::Seen::Heard(ms(400.0), Ack::None))
    );
    let rows = air.rows_seen();
    assert!(rows.contains(&row(ms(300.0), r#""ap_down""#).as_str()));
    assert!(rows.contains(&row(ms(500.0), r#""ap_up""#).as_str()));
    assert_eq!(
        rows.last(),
        Some(
            &row(
                ms(520.0),
                &format!(r#""deauth","sta":"{STA}","reason":7,"by":"ap""#)
            )
            .as_str()
        )
    );
}

// Frames to another BSSID, and control frames other than PS-Poll, are not acknowledged. A
// group-addressed Probe Request is answered but not acknowledged.
#[test]
fn it_acknowledges_only_frames_to_it() {
    let mut elsewhere = common::auth(0, 1);
    elsewhere[4..10].copy_from_slice(&OTHER.0);
    let mut rts = common::ps_poll(1);
    rts[0] = 0xb4;
    let script = Script::default()
        .send(ms(10.0), elsewhere)
        .send(ms(11.0), rts)
        .send(ms(12.0), hex(common::capture::PROBE_58));
    let air = run(script, ms(20.0), common::WPA2);
    let heard: Vec<_> = (air.seen.iter())
        .filter_map(|s| match s {
            common::Seen::Heard(_, ack) => Some(*ack),
            _ => None,
        })
        .collect();
    assert_eq!(heard, [Ack::None; 3]);
    assert_eq!(air.sent().len(), 1);
}
