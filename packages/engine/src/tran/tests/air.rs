//! The air: a script station joins the netlist's network through the engine, with no chip. It
//! associates, runs the 4-way handshake, leases an address, looks up a time server and gets the
//! time, every frame landing at its end and every answer an `Ack` there. A run in any chunks,
//! with rewinds or without, is the same run.

use orbe_model::Drive;
use orbe_model::air::{Ack, Channel};

use crate::netlist::Netlist;

use super::super::*;
use super::active::scripted;
use super::fixtures::*;
use super::session::Rng;
use crate::active::Core;
use crate::active::script::station::{Log, Station};
use crate::active::script::{self, Op};
use crate::diagnostic::Diagnostic;
use crate::session::State;

const MS: u64 = 1_000_000_000;
const STOP: u64 = 50 * MS;
const BSSID: [u8; 6] = [0x02, 0x6f, 0x72, 0x62, 0x65, 0x01];
const SSID: &str = "orbe-lab";
const PASSPHRASE: &str = "breadboard";
/// 2026-03-14T09:26:00Z.
const EPOCH_S: u64 = 1_773_480_360;
const ROUTER: [u8; 4] = [192, 168, 1, 1];

fn network_json(network: &str) -> String {
    format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {STOP}, "max_step_ps": 100000000, "grid_ps": {MS} }} }},
          "ground": "gnd",
          "nets": ["gnd", "vcc", "o1", "o2", "x"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
              "voltage": {{ "value": 5.0, "unit": "V" }} }},
            {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "o1", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
            {{ "name": "r2", "kind": "resistor", "pins": {{ "a": "o2", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
            {{ "name": "rx", "kind": "resistor", "pins": {{ "a": "x", "b": "gnd" }},
              "resistance": {{ "value": 1000.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }}
          ]{network}
        }}"#
    )
}

/// The WPA2-PSK network "orbe-lab" on channel 6.
fn netlist() -> Netlist {
    let network = format!(
        r#", "network": {{ "ssid": "{SSID}", "bssid": "02:6f:72:62:65:01", "channel": 6,
              "passphrase": "{PASSPHRASE}", "epoch_s": {EPOCH_S} }}"#
    );
    crate::netlist::parse(network_json(&network).as_bytes()).unwrap()
}

fn mac(last: u8) -> [u8; 6] {
    [0x02, 0, 0, 0, 0, last]
}

/// A script on `out` that plays a station at `mac(last)`, which probes from `start_ms`.
fn station(netlist: &Netlist, out: &str, last: u8, start_ms: u64) -> crate::active::Chip {
    let channel = Channel::new(6).unwrap();
    let start = Time::from_ps(start_ms * MS);
    let station = Station::new(
        mac(last),
        BSSID,
        SSID.as_bytes(),
        PASSPHRASE,
        channel,
        start,
    );
    script::radio(scripted(netlist, out, "x", &[], None), station)
}

fn log(session: &Session, k: usize) -> Vec<Log> {
    let Core::Script(core) = &session.actives.cores()[k] else {
        panic!("chip {k} is a script");
    };
    core.station().expect("a station").log.clone()
}

fn rows(session: &Session) -> Vec<String> {
    session
        .network_rows()
        .iter()
        .map(|row| row.json())
        .collect()
}

/// Each row's `event`, for the station at `mac(last)` or for no station.
fn events(session: &Session, last: u8) -> Vec<String> {
    let sta = format!("02:00:00:00:00:{last:02x}");
    (rows(session).iter())
        .filter(|row| !row.contains("\"sta\"") || row.contains(&sta))
        .map(|row| row.split('"').nth(5).unwrap().to_string())
        .collect()
}

/// The station joined, got the lease `lease` and the time, and every frame it sent but its
/// broadcast probe was acknowledged.
fn check_joined(log: &[Log], lease: [u8; 4]) {
    let learned: Vec<&Log> = log
        .iter()
        .filter(|log| !matches!(log, Log::Acked(..)))
        .collect();
    let [
        Log::Associated(_),
        Log::Keyed,
        Log::Leased(leased),
        Log::Resolved(resolved),
        Log::Time { asked, utc_ms },
    ] = learned[..]
    else {
        panic!("{log:?}");
    };
    assert_eq!((*leased, *resolved), (lease, ROUTER));
    // The router answers in the simulated millisecond the request lands, from its clock.
    assert_eq!(*utc_ms, EPOCH_S * 1000 + asked.as_ps() / MS, "{log:?}");
    let acks: Vec<Ack> = (log.iter())
        .filter_map(|log| match log {
            Log::Acked(_, ack) => Some(*ack),
            _ => None,
        })
        .collect();
    assert_eq!(acks[0], Ack::None, "the probe is to every station");
    assert!(acks[1..].iter().all(|&ack| ack == Ack::Acked), "{log:?}");
    // Authentication, association, messages 2 and 4, DISCOVER, REQUEST, DNS and SNTP.
    assert_eq!(acks.len(), 9, "{log:?}");
}

/// Test 4: the station joins through the engine's air, not by calling `orbe-net`.
#[test]
fn a_station_joins_wpa2_through_the_air_and_gets_the_time() {
    let netlist = netlist();
    let mut session = open(&netlist, vec![station(&netlist, "o1", 1, 1)], BUDGET);
    session.advance_to(Time::from_ps(STOP));
    assert!(matches!(session.state, State::Ended { .. }));
    check_joined(&log(&session, 0), [192, 168, 1, 100]);
    assert_eq!(
        events(&session, 1),
        ["auth", "assoc", "handshake", "dhcp_ack", "dns", "sntp"],
        "{:#?}",
        rows(&session)
    );
    let rows = rows(&session);
    assert!(
        rows[5].contains(r#""sent":"2026-03-14T09:26:00.0"#),
        "{rows:?}"
    );
    // A frame stop lands without a restart: the air changes no image.
    assert_eq!(session.stats.restarts, 0);
}

/// Two stations share the one access point and its DHCP pool, in station order, and each
/// hears the other's frames without answering them.
#[test]
fn two_stations_share_the_access_point_and_its_leases() {
    let netlist = netlist();
    let chips = vec![station(&netlist, "o1", 2, 3), station(&netlist, "o2", 1, 1)];
    let mut session = open(&netlist, chips, BUDGET);
    session.advance_to(Time::from_ps(STOP));
    check_joined(&log(&session, 1), [192, 168, 1, 100]);
    check_joined(&log(&session, 0), [192, 168, 1, 101]);
    for last in [1, 2] {
        let events = events(&session, last);
        assert_eq!(
            events.last().map(String::as_str),
            Some("sntp"),
            "{events:?}"
        );
    }
}

/// What a run leaves: each station's log and the network's rows.
fn outcome(session: &Session, stations: usize) -> (Vec<Vec<Log>>, Vec<String>) {
    let logs = (0..stations).map(|k| log(session, k)).collect();
    (logs, rows(session))
}

/// A run in one call, one in seeded chunks, and one that stops at every landing, so every
/// core is kept, and copied over its last commit, at each one mid-exchange, are the same run.
#[test]
fn a_run_split_anywhere_or_kept_at_every_landing_is_the_same_run() {
    let netlist = netlist();
    let chips = || vec![station(&netlist, "o1", 2, 3), station(&netlist, "o2", 1, 1)];
    let mut whole = open(&netlist, chips(), BUDGET);
    whole.advance_to(Time::from_ps(STOP));
    let want = outcome(&whole, 2);

    for seed in [1, 7, 42, 1234] {
        let mut rng = Rng(seed);
        let mut session = open(&netlist, chips(), BUDGET);
        while let State::Running { now, next, .. } = &session.state {
            let (now, landing) = (now.as_ps(), next.at.as_ps());
            let to = match rng.below(5) {
                0 => landing,
                1 => landing - 1,
                2 => landing + 1,
                3 => now + 1 + rng.below(1_000_000),
                _ => now + 1 + rng.below(STOP / 20),
            };
            session.advance_to(Time::from_ps(to));
        }
        assert_eq!(outcome(&session, 2), want, "seed {seed}");
    }

    let mut session = open(&netlist, chips(), BUDGET);
    while let State::Running { next, .. } = &session.state {
        let landing = next.at;
        session.advance_to(landing);
    }
    assert_eq!(outcome(&session, 2), want, "kept at every landing");
}

/// A driver flips `x`, which the station watches, every 157 us, so the station is run again
/// from its last commit over and over, and the network is put back at each frame's landing.
/// Each frame still lands once and the run is the one without the flips.
#[test]
fn a_station_run_again_after_rewinds_joins_the_same_way() {
    let netlist = netlist();
    let mut whole = open(&netlist, vec![station(&netlist, "o1", 1, 1)], BUDGET);
    whole.advance_to(Time::from_ps(STOP));
    let flips: Vec<(u64, Op)> = (1..STOP / 157_000_000)
        .zip([Drive::High, Drive::Low].iter().cycle())
        .map(|(k, &drive)| (k * 157_000_000, Op::Drive(drive)))
        .collect();
    let driver = scripted(&netlist, "x", "o2", &flips, None);
    let chips = vec![station(&netlist, "o1", 1, 1), driver];
    let mut session = open(&netlist, chips, BUDGET);
    while let State::Running { next, .. } = &session.state {
        let landing = next.at;
        session.advance_to(landing);
    }
    assert_eq!(outcome(&session, 1), outcome(&whole, 1));
}

/// A netlist without `network` builds no air, and a frame of a station there goes nowhere.
#[test]
fn a_netlist_without_a_network_builds_no_air() {
    let netlist = crate::netlist::parse(network_json("").as_bytes()).unwrap();
    let mut session = open(&netlist, vec![station(&netlist, "o1", 1, 1)], BUDGET);
    session.advance_to(Time::from_ps(STOP));
    assert!(session.network_rows().is_empty());
    assert_eq!(session.actives.cores().len(), 1);
}

#[test]
fn a_network_field_out_of_range_is_a_syntax_error() {
    for network in [
        r#""ssid": "orbe-lab", "bssid": "03:00:00:00:00:01", "channel": 6"#,
        r#""ssid": "orbe-lab", "bssid": "02:00:00:00:00:01", "channel": 15"#,
        r#""ssid": "", "bssid": "02:00:00:00:00:01", "channel": 6"#,
        r#""ssid": "orbe-lab", "bssid": "02:00:00:00:00:01", "channel": 6, "passphrase": "short""#,
        r#""ssid": "orbe-lab", "bssid": "02:00:00:00:00:01", "channel": 6, "epoch_s": 253383854056"#,
        r#""ssid": "orbe-lab", "bssid": "02:00:00:00:00:01", "channel": 6, "rssi": -50"#,
    ] {
        let json = network_json(&format!(r#", "network": {{ {network} }}"#));
        let Err(found) = crate::netlist::parse(json.as_bytes()) else {
            panic!("{network} parsed");
        };
        assert!(
            matches!(&found[..], [Diagnostic::NetlistSyntax { path, .. }] if path.starts_with("network")),
            "{network}: {found:?}"
        );
    }
}
