//! The subnet behind the access point, through the air: a station joins, gets its keys, leases
//! an address, finds the router, looks up a time server and gets the time.

mod common;

use common::lan::{self, LEASE, ROUTER};
use common::{
    ANONCE, Air, BSSID, PASSPHRASE, STA, Script, Supplicant, at, by, clear, data_sent,
    keyed_network, ms, network, open_subnet_session, subnet_session,
};
use orbe_model::Time;
use orbe_net::{Config, Mac, Network, Security, Subnet};

fn run(script: Script, until: Time, mut net: Network) -> Air {
    let mut air = Air::new(script);
    air.run_through(&mut net, until);
    air
}

/// The router's answers, in order, as (start, MSDU).
fn answers(air: &Air, keyed: bool) -> Vec<(Time, Vec<u8>)> {
    let sta = Supplicant::new(PASSPHRASE);
    let mut pns = Vec::new();
    let out = (data_sent(air).into_iter())
        .map(|f| {
            assert_eq!(f.mpdu[4..10], STA.0);
            assert_eq!(f.mpdu[10..22], [BSSID.0, BSSID.0].concat());
            let msdu = if keyed {
                let (pn, msdu) = sta.open(&ANONCE, &f.mpdu).expect("a frame under the TK");
                pns.push(pn);
                msdu
            } else {
                assert_eq!(f.mpdu[1], 0x02);
                f.mpdu[24..].to_vec()
            };
            (f.start, msdu)
        })
        .collect();
    if keyed {
        assert_eq!(pns, (1..=pns.len() as u64).collect::<Vec<_>>());
    }
    out
}

/// Checks the answers to [`common::requests`]: one each but for the ARP probe, each started
/// within a millisecond of its request, and the time the station sets its clock to.
fn check_exchange(air: &Air, keyed: bool) {
    let answers = answers(air, keyed);
    let times = [
        at::DISCOVER,
        at::REQUEST,
        at::ARP,
        at::DNS,
        at::SNTP,
        at::PING,
        at::NXDOMAIN,
    ];
    assert_eq!(answers.len(), times.len(), "{answers:02x?}");
    for ((start, _), at) in answers.iter().zip(times) {
        assert!(
            ms(at) < *start && *start < ms(at + 1.0),
            "{start:?} for the request at {at} ms"
        );
    }
    let mut answers = answers.into_iter().map(|(_, msdu)| msdu);

    // DHCP (RFC 2131): the OFFER and the ACK of the first lease, to that address, with the
    // server, the lease time, the mask, the router and the DNS server.
    for kind in [lan::OFFER, lan::ACK] {
        let (sport, dst, dport, reply) = lan::read_udp(&answers.next().unwrap());
        assert_eq!((sport, dst, dport), (67, LEASE, 68));
        assert_eq!(reply[..8], [2, 1, 6, 0, 0xab, 0xcd, 0, 1]);
        assert_eq!(reply[16..20], LEASE);
        assert_eq!(reply[28..34], STA.0);
        let (got, options) = lan::dhcp_options(&reply);
        assert_eq!(got, kind);
        assert_eq!(
            options,
            vec![
                (54, ROUTER.to_vec()),
                (51, 86_400u32.to_be_bytes().to_vec()),
                (1, vec![255, 255, 255, 0]),
                (3, ROUTER.to_vec()),
                (6, ROUTER.to_vec()),
            ]
        );
    }

    // ARP (RFC 826): the router's address is at the BSSID.
    let reply = answers.next().unwrap();
    assert_eq!(reply[..8], [0xaa, 0xaa, 3, 0, 0, 0, 8, 6]);
    let mut want = vec![0, 1, 8, 0, 6, 4, 0, 2];
    want.extend(BSSID.0);
    want.extend(ROUTER);
    want.extend(STA.0);
    want.extend(LEASE);
    assert_eq!(reply[8..], want);

    // DNS (RFC 1035): pool.ntp.org is the router.
    let (sport, dst, dport, reply) = lan::read_udp(&answers.next().unwrap());
    assert_eq!((sport, dst, dport), (53, LEASE, 0xc001));
    assert_eq!(
        reply[..12],
        [0x4e, 0x21, 0x85, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]
    );
    assert_eq!(reply[reply.len() - 4..], ROUTER);

    // SNTP (RFC 4330): a stratum 1 server whose Transmit Timestamp is the network's clock when
    // the request landed, 100 ms past 2026-01-01T00:00:00Z.
    let (sport, dst, dport, reply) = lan::read_udp(&answers.next().unwrap());
    assert_eq!((sport, dst, dport), (123, LEASE, 123));
    assert_eq!(reply.len(), 48);
    assert_eq!(reply[..2], [0x24, 1]);
    assert_eq!(reply[24..32], lan::NTP_SENT);
    let ntp = |at: usize| {
        let s = u64::from(u32::from_be_bytes(reply[at..at + 4].try_into().unwrap()));
        let f = u64::from(u32::from_be_bytes(
            reply[at + 4..at + 8].try_into().unwrap(),
        ));
        // Microseconds from 1970, as a station sets its clock.
        (s - 2_208_988_800) * 1_000_000 + ((f * 1_000_000) >> 32)
    };
    let sent = ntp(40);
    assert_eq!(sent, lan::EPOCH * 1_000_000 + 99_999);
    assert_eq!(ntp(40), ntp(32));
    assert_eq!(ntp(16), lan::EPOCH * 1_000_000);

    // ICMP (RFC 792): the echo comes back.
    let (protocol, dst, echo) = lan::read_ipv4(&answers.next().unwrap());
    assert_eq!((protocol, dst), (1, LEASE));
    assert_eq!(echo[..2], [0, 0]);
    assert_eq!(lan::checksum(&echo), 0);
    assert_eq!(echo[4..], *b"\x01\x01\x00\x07orbe ping");

    // An unknown name is NXDOMAIN.
    let (_, _, _, reply) = lan::read_udp(&answers.next().unwrap());
    assert_eq!(reply[..8], [0x4e, 0x22, 0x85, 0x83, 0, 1, 0, 0]);

    let rows: Vec<_> = (air.rows_seen().into_iter())
        .filter(|row| !row.contains("\"auth\"") && !row.contains("\"assoc\""))
        .filter(|row| !row.contains("\"handshake\""))
        .collect();
    let sta = "00:0d:93:82:36:3a";
    assert_eq!(
        rows,
        [
            format!(
                r#"{{"t_ps":60000000000,"event":"dhcp_ack","sta":"{sta}","ip":"192.168.1.100"}}"#
            ),
            format!(
                r#"{{"t_ps":90000000000,"event":"dns","sta":"{sta}","name":"pool.ntp.org","answer":"192.168.1.1"}}"#
            ),
            format!(
                r#"{{"t_ps":100000000000,"event":"sntp","sta":"{sta}","sent":"2026-01-01T00:00:00.100Z"}}"#
            ),
            format!(
                r#"{{"t_ps":115000000000,"event":"nxdomain","sta":"{sta}","name":"pool.ntp.orq"}}"#
            ),
        ]
    );
}

// Association, the 4-way handshake, DHCP, ARP, a DNS lookup of a time server and SNTP, every
// answer under CCMP-128 with rising PNs.
#[test]
fn a_station_joins_wpa2_and_gets_the_time() {
    let (script, until) = subnet_session();
    let air = run(script, until, keyed_network());
    assert!(air.rows_seen().iter().any(|row| row.contains("handshake")));
    check_exchange(&air, true);
}

#[test]
fn a_station_on_an_open_network_gets_the_same_answers_in_the_clear() {
    let (script, until) = open_subnet_session();
    let air = run(script, until, network(Security::Open));
    check_exchange(&air, false);
}

#[test]
fn the_clock_reads_the_configured_epoch_plus_the_simulated_time() {
    // 2026-03-14T09:26:00Z at time 0, and the request at 100 ms.
    let subnet = Subnet {
        epoch: 1_773_480_360,
        ..Subnet::default()
    };
    let config = Config::new(common::SSID, BSSID, common::channel(), Security::Open)
        .unwrap()
        .with_subnet(subnet)
        .unwrap();
    let (script, until) = open_subnet_session();
    let air = run(script, until, Network::new(config));
    assert!(
        (air.rows_seen().iter()).any(|row| row.ends_with(
            r#""event":"sntp","sta":"00:0d:93:82:36:3a","sent":"2026-03-14T09:26:00.100Z"}"#
        )),
        "{:?}",
        air.rows_seen()
    );
}

#[test]
fn a_subnet_whose_leases_leave_it_is_refused() {
    let config = Config::new(common::SSID, BSSID, common::channel(), Security::Open).unwrap();
    let subnet = Subnet {
        leases: 200,
        ..Subnet::default()
    };
    assert!(config.with_subnet(subnet).is_none());
}

/// An open station `sta` joins at `t` ms and sends a DISCOVER at `t + 2`.
fn join_and_discover(script: Script, sta: Mac, t: f64) -> Script {
    let discover = lan::dhcp(lan::DISCOVER, [0; 4], None, None);
    script
        .send(ms(t), by(sta, common::auth(0, 1)))
        .send(ms(t + 1.0), by(sta, common::assoc(common::SSID, None)))
        .send(ms(t + 2.0), clear(sta, Mac::BROADCAST, &discover))
}

// A station keeps its address when it leaves and joins again, and the next station gets the
// next one. A REQUEST for another station's address gets a NAK (RFC 2131 4.3.2).
#[test]
fn each_station_keeps_its_own_address() {
    let other = Mac([0x02, 0, 0, 0, 0, 2]);
    let script = join_and_discover(Script::default(), STA, 10.0).send(ms(20.0), common::deauth(3));
    let script = join_and_discover(script, STA, 30.0);
    let script = join_and_discover(script, other, 40.0);
    let request = lan::dhcp(lan::REQUEST, [0; 4], Some(LEASE), Some(ROUTER));
    let script = script.send(ms(50.0), clear(other, Mac::BROADCAST, &request));
    let air = run(script, ms(60.0), network(Security::Open));

    let offers: Vec<_> = (air.sent().into_iter())
        .filter(|f| f.mpdu[0] == 0x08)
        .map(|f| {
            let (_, dst, _, reply) = lan::read_udp(&f.mpdu[24..]);
            let (kind, _) = lan::dhcp_options(&reply);
            (Mac(f.mpdu[4..10].try_into().unwrap()), kind, dst)
        })
        .collect();
    assert_eq!(
        offers,
        [
            (STA, lan::OFFER, LEASE),
            (STA, lan::OFFER, LEASE),
            (other, lan::OFFER, lan::SECOND_LEASE),
            (other, lan::NAK, [255; 4]),
        ]
    );
    assert!((air.rows_seen().iter()).any(|row| {
        row.ends_with(r#""event":"dhcp_nak","sta":"02:00:00:00:00:02","ip":"192.168.1.100"}"#)
    }));
}

// A station that is not associated gets no answer, and neither does a frame in the clear on a
// WPA2 network.
#[test]
fn only_an_associated_station_past_the_controlled_port_is_answered() {
    let discover = lan::dhcp(lan::DISCOVER, [0; 4], None, None);
    let script = Script::default()
        .send(ms(10.0), common::auth(0, 1))
        .send(ms(20.0), clear(STA, Mac::BROADCAST, &discover));
    let air = run(script, ms(30.0), network(Security::Open));
    assert!(data_sent(&air).is_empty());

    let (mut script, _) = subnet_session();
    script.steps.truncate(4);
    let script = script.send(ms(50.0), clear(STA, Mac::BROADCAST, &discover));
    let air = run(script, ms(60.0), keyed_network());
    assert!(data_sent(&air).is_empty());
}
