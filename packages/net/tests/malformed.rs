//! No frame on the air panics the network: every cut of every frame it reads, and frames
//! corrupted at random, heard in every state the session reaches.

mod common;

use common::{ANONCE, Air, BSSID, Rng, Supplicant, from_station, hex, keyed_network, ms, session};
use orbe_model::Time;
use orbe_model::air::{AirFrame, Mcs, Rate};
use orbe_net::Network;

/// Every frame the tests send, from the station and from the capture.
fn frames() -> Vec<Vec<u8>> {
    let mut frames = vec![
        common::auth(0, 1),
        common::auth(3, 1),
        common::assoc_ccmp(),
        common::assoc(common::SSID, Some(&common::RSN_PSK)),
        common::assoc(common::SSID, Some(&[1, 0, 0, 0x0f, 0xac])),
        common::addba(1, 3, true),
        common::null(true),
        common::null(false),
        common::ps_poll(1),
        common::deauth(3),
    ];
    for frame in [
        common::capture::BEACON_1,
        common::capture::PROBE_58,
        common::capture::PROBE_RESPONSE_59,
        common::capture::AUTH_78,
        common::capture::ASSOC_82,
        common::capture::PROBE_999,
        common::capture::DISASSOC_1050,
    ] {
        frames.push(hex(frame));
    }
    // The handshake and the Data frames after it, with a key data of every element the access
    // point reads, and Data frames under the wrong key.
    let sta = Supplicant::new(common::PASSPHRASE);
    frames.push(sta.message_2(&ANONCE, 0, &common::RSN_PSK));
    frames.push(sta.message_4(&ANONCE, 1));
    frames.push(sta.data(&ANONCE, 9, b"data"));
    frames.push(Supplicant::new("not the passphrase").data(&ANONCE, 10, b"data"));
    let mut qos = sta.data(&ANONCE, 11, b"qos");
    qos[0] = 0x88;
    qos.splice(24..24, [5, 0]);
    frames.push(qos);
    // A Reassociation Request: the Association Request with the current AP's address.
    let mut reassoc = common::assoc_ccmp();
    reassoc[0] = 0x20;
    reassoc.splice(28..28, BSSID.0);
    frames.push(reassoc);
    frames
}

/// The network at each of the session's steps, so frames land on every state it reaches.
fn states() -> Vec<(Network, Time)> {
    let (script, until) = session();
    let mut air = Air::new(script);
    let mut net = keyed_network();
    let mut states = vec![(net.clone(), Time::ZERO)];
    let mut at = Time::ZERO;
    while at < until {
        at = Time::from_ps(at.as_ps() + 5_000_000_000);
        air.run_through(&mut net, at);
        states.push((net.clone(), at));
    }
    states
}

/// Hears `mpdu` at `at`, at `rate`, then runs on 2 ms, answering every frame it sends.
fn land(net: &mut Network, at: Time, mpdu: &[u8], rate: Rate) -> Time {
    let mut frame = from_station(at, mpdu);
    frame.rate = rate;
    net.run_until(at);
    net.hear(at, &frame, common::SIGNAL);
    let until = Time::from_ps(at.as_ps() + 2_000_000_000);
    while let Some(stop) = net.run_until(until) {
        net.take_aired(|_: AirFrame| {});
        net.acked(stop.at, orbe_model::air::Ack::Acked);
    }
    net.take_rows(|_| {});
    until
}

#[test]
fn every_cut_of_every_frame_is_heard_without_a_panic() {
    for (state, at) in states() {
        for frame in frames() {
            let mut net = state.clone();
            let mut t = Time::from_ps(at.as_ps() + 1);
            for len in 0..=frame.len() {
                t = land(&mut net, t, &frame[..len], Rate::Dsss1);
            }
        }
    }
}

#[test]
fn corrupted_frames_are_heard_without_a_panic() {
    let frames = frames();
    let states = states();
    let rates = [
        Rate::Dsss1,
        Rate::Ofdm(orbe_model::air::Ofdm::M54),
        Rate::Ht(Mcs::new(7).unwrap()),
    ];
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for _ in 0..20_000 {
        let (state, at) = &states[rng.below(states.len() as u64) as usize];
        let mut net = state.clone();
        let mut t = Time::from_ps(at.as_ps() + 1);
        for _ in 0..4 {
            let mut frame = match rng.below(4) {
                0 => (0..rng.below(300)).map(|_| rng.next() as u8).collect(),
                _ => frames[rng.below(frames.len() as u64) as usize].clone(),
            };
            for _ in 0..rng.below(6) {
                if frame.is_empty() {
                    break;
                }
                let k = rng.below(frame.len() as u64) as usize;
                frame[k] ^= 1 << rng.below(8);
            }
            match rng.below(4) {
                0 => frame.truncate(rng.below(frame.len() as u64 + 1) as usize),
                1 => frame.extend((0..rng.below(40)).map(|_| rng.next() as u8)),
                _ => {}
            }
            // Mostly to the access point, so it reads past the header.
            if frame.len() >= 10 && rng.below(4) != 0 {
                frame[4..10].copy_from_slice(&BSSID.0);
            }
            let rate = rates[rng.below(rates.len() as u64) as usize];
            t = land(&mut net, t, &frame, rate);
        }
        // It still answers a Probe Request.
        let mut answered = false;
        let probe = from_station(
            Time::from_ps(t.as_ps() + 1),
            &hex(common::capture::PROBE_999),
        );
        net.run_until(probe.end);
        net.hear(probe.end, &probe, common::SIGNAL);
        while let Some(stop) = net.run_until(ms(10_000.0)) {
            net.take_aired(|frame| answered |= frame.mpdu[0] == 0x50);
            net.acked(stop.at, orbe_model::air::Ack::Acked);
            if answered {
                break;
            }
        }
        assert!(answered || net.tuned().is_none());
    }
}
