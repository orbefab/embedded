//! An ESP32-S3 in a netlist with a network is built with its radio. The stock Arduino station
//! sketch then gets through `esp_wifi_init` and `esp_wifi_start`, which `WiFi.mode(WIFI_STA)`
//! runs, with no diagnostic, and its scan puts a probe request on the air on each channel and
//! finds the network on its own. Its `WiFi.begin()` then authenticates, associates, runs the
//! 4-way handshake and leases an address over DHCP in frames its MAC seals and opens under
//! CCMP-128, and goes into power save, its RF powered down between the beacons its TBTT wakes it
//! for. It prints its address, then looks up `pool.ntp.org`, asks the network for the time over
//! SNTP and prints the date, and dozes on with no fault to the end of the run.
//! Without the network the same app stops at the radio, as before.
use std::net::Ipv4Addr;
use std::sync::Arc;

use orbe_crypto::aes::AesKey;
use orbe_crypto::{ccm, pbkdf2, prf};
use orbe_mcu::Setting;
use orbe_model::Time;
use orbe_model::air::{AirFrame, Rate};
use orbe_net::{Event, Mac};

use super::active::run_active;
use super::fixtures::open;
use crate::active::Core;
use crate::active::script::station::{aad, nonce};
use crate::blob::Blobs;
use crate::diagnostic::Diagnostic;
use crate::netlist::Netlist;
use crate::session::{BUDGET, NetworkSendRejected, SerialCursor, Session, Status};

const MS: u64 = 1_000_000_000;

/// The DevKitC-1 module's supply and the stock station app, run for `stop_ms`, with `network`
/// after the parts.
fn station(network: &str, stop_ms: u64) -> Netlist {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rom = std::fs::read(root.join("parts/esp32s3-qfn56/esp32s3_rev0_rom.elf")).unwrap();
    let fixture = root.join("packages/scene/tests/fixtures/esp32s3-stock-station");
    let partitions = std::fs::read(fixture.join("partitions.bin")).unwrap();
    let app = std::fs::read(fixture.join("app.bin")).unwrap();

    let mut blobs = Blobs::new();
    let rom_bytes = rom.len();
    let rom_sha = blobs.add(Arc::from(rom));
    let part_bytes = partitions.len();
    let part_sha = blobs.add(Arc::from(partitions));
    let app_bytes = app.len();
    let app_sha = blobs.add(Arc::from(app));

    let ohm = |value: f64| format!(r#"{{ "value": {value:?}, "unit": "ohm" }}"#);
    let json = format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {MS}, "grid_ps": {MS} }} }},
          "ground": "gnd",
          "nets": ["gnd", "vdd"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vdd", "n": "gnd" }},
              "voltage": {{ "value": 3.3, "unit": "V" }} }},
            {{ "name": "u1", "kind": "esp32s3",
              "pins": {{ "VCC": "vdd", "GND": "gnd" }},
              "rom": {{ "sha256": "{rom_sha}", "bytes": {rom_bytes} }},
              "firmware": {{
                "flash": [
                  {{ "offset": 32768, "sha256": "{part_sha}", "bytes": {part_bytes} }},
                  {{ "offset": 65536, "sha256": "{app_sha}", "bytes": {app_bytes} }}
                ]
              }},
              "output": {{ "high_drop": {{ "value": 0.0, "unit": "V" }}, "high_resistance": {r},
                          "low_drop": {{ "value": 0.0, "unit": "V" }}, "low_resistance": {r} }},
              "pull_up_resistance": {pull},
              "leakage_resistance": {leak},
              "input_threshold": {{ "low": {{ "value": 0.25, "unit": "1" }},
                                   "high": {{ "value": 0.75, "unit": "1" }} }} }}
          ]{network}
        }}"#,
        stop = stop_ms * MS,
        r = ohm(17.0),
        pull = ohm(45e3),
        leak = ohm(66e6),
    );
    crate::netlist::parse_with(json.as_bytes(), &blobs).unwrap()
}

/// What the chip sent on UART0.
fn uart0(netlist: &Netlist) -> (String, Vec<Diagnostic>) {
    let (trace, diagnostics) = run_active(netlist, Vec::new());
    let files = trace.files();
    let serial = files
        .iter()
        .find(|(name, _)| *name == "serial.csv")
        .map(|(_, bytes)| String::from_utf8(bytes.clone()).unwrap())
        .expect("serial.csv");
    let text = serial
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split(',').collect();
            (cols.get(2) == Some(&"UART0")).then(|| char::from(cols[3].parse::<u8>().unwrap()))
        })
        .collect();
    (text, diagnostics)
}

/// The WPA2 network "orbe" on channel 6.
const NETWORK: &str = r#", "network": { "ssid": "orbe", "bssid": "02:6f:72:62:65:01",
    "channel": 6, "passphrase": "orbe-password", "epoch_s": 1773480360 }"#;

#[test]
fn with_the_network_the_stock_station_starts_its_radio_with_no_diagnostic() {
    let (text, diagnostics) = uart0(&station(NETWORK, 100));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    // `WiFi.mode(WIFI_STA)` returns true only when `esp_wifi_init` and `esp_wifi_start` do.
    assert!(text.contains("ready\r\nmode 1\n"), "UART0 output:\n{text}");
}

#[test]
fn without_the_network_the_stock_station_stops_at_the_radio() {
    let (text, diagnostics) = uart0(&station("", 100));
    assert!(text.contains("ready"), "UART0 output:\n{text}");
    assert!(!text.contains("mode"), "UART0 output:\n{text}");
    assert!(
        matches!(
            diagnostics.as_slice(),
            [Diagnostic::ChipUnsupportedSetting {
                peripheral: "RADIO",
                setting: Setting::Unsimulated(_),
                ..
            }]
        ),
        "{diagnostics:?}"
    );
}

const BROADCAST: [u8; 6] = [0xff; 6];
/// The station's MAC, the chip's base MAC as the model's eFuse holds it.
const STA: [u8; 6] = [0x02, 0, 0, 0, 0, 0x01];

/// The Information Elements of a Management frame's body, each its ID and its bytes.
fn elements(mut body: &[u8]) -> Vec<(u8, &[u8])> {
    let mut found = Vec::new();
    while let [id, len, rest @ ..] = body {
        let (bytes, next) = rest.split_at(usize::from(*len));
        found.push((*id, bytes));
        body = next;
    }
    found
}

/// The stock station's `WiFi.scanNetworks()` sends a probe request on each channel it scans, and
/// the network on channel 6 hears its own and answers it. Each probe is a wildcard Probe Request
/// from the station's MAC to everyone, at 1 Mb/s, on the channel the chip tuned to and named in
/// its DS Parameter Set. The default country, "01", has channels 1 to 11 (IDF v5.5.5,
/// `esp_wifi.h`, `esp_wifi_set_country`), so the scan probes those, once each. It moves on
/// 100 ms after a probe, the sketch's minimum active time, from a channel where it hears no
/// Probe Response, and 300 ms after it, the maximum `WiFi.scanNetworks()` passes, from channel 6,
/// where it hears one (Arduino-ESP32 3.3.12, `WiFiScan.cpp`).
#[test]
fn the_stock_stations_scan_probes_each_channel_and_the_network_answers_on_its_own() {
    let netlist = station(NETWORK, 1400);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.advance_to(Time::from_ps(1400 * MS));
    assert!(
        session.diagnostics().is_empty(),
        "{:?}",
        session.diagnostics()
    );

    let chip = (session.actives.cores().iter())
        .position(|core| matches!(core, Core::Mcu(_)))
        .expect("the chip");
    let mut sent: Vec<&AirFrame> = Vec::new();
    let mut answers: Vec<&AirFrame> = Vec::new();
    for (sender, frame) in session.actives.aired() {
        // A landing run again after a rewind hands its frame on again.
        let seen = |frames: &[&AirFrame]| frames.iter().any(|f| f.end == frame.end);
        match *sender {
            k if k == chip && !seen(&sent) => sent.push(frame),
            k if k != chip && frame.mpdu[4..10] == STA && !seen(&answers) => answers.push(frame),
            _ => {}
        }
    }

    let channels: Vec<u8> = sent.iter().map(|frame| frame.channel.number()).collect();
    assert_eq!(channels, (1..=11).collect::<Vec<u8>>());
    for frame in &sent {
        let mpdu = &frame.mpdu;
        let channel = frame.channel.number();
        // Frame Control: a Management frame of subtype 4, Probe Request (9.2.4.1.3).
        assert_eq!(mpdu[..2], [0x40, 0x00], "channel {channel}");
        assert_eq!(mpdu[4..10], BROADCAST, "DA, channel {channel}");
        assert_eq!(mpdu[10..16], STA, "SA, channel {channel}");
        assert_eq!(mpdu[16..22], BROADCAST, "BSSID, channel {channel}");
        let elements = elements(&mpdu[24..]);
        // The wildcard SSID first (9.3.3.9), and the DS Parameter Set names the channel.
        assert_eq!(elements[0], (0, &[][..]), "channel {channel}");
        assert!(elements.contains(&(3, &[channel][..])), "channel {channel}");
        assert_eq!(frame.rate, Rate::Dsss1);
        // Its airtime with the FCS, to the chip's next 480 MHz tick.
        let airtime = frame.end.as_ps() - frame.start.as_ps();
        let want = Rate::Dsss1.airtime(mpdu.len() + 4);
        assert!(
            (want..want + 2_084).contains(&airtime),
            "{airtime} for {want}"
        );
    }
    let starts: Vec<u64> = sent.iter().map(|frame| frame.start.as_ps() / MS).collect();
    let gaps: Vec<u64> = starts.windows(2).map(|pair| pair[1] - pair[0]).collect();
    for (k, gap) in gaps.iter().enumerate() {
        let want = if k + 1 == 6 { 300 } else { 100 };
        assert!((want..=want + 1).contains(gap), "{starts:?}");
    }

    // The network heard only the probe on its own channel, and answered it with a Probe
    // Response DIFS after it ended.
    let probe = sent[5];
    let [answer] = answers[..] else {
        panic!("{answers:?}");
    };
    assert_eq!(answer.channel.number(), 6);
    assert_eq!(answer.mpdu[..2], [0x50, 0x00]);
    assert!(answer.start > probe.end, "{answer:?} after {probe:?}");
}

/// The stock station's chip receives the network's Probe Response on channel 6 and acknowledges
/// it, so `WiFi.scanNetworks()` finds the network there and the network logs no lost frame.
/// `scan 1` comes at about 2.46 s.
#[test]
fn the_stock_stations_scan_finds_the_network_on_its_channel() {
    let netlist = station(NETWORK, 2500);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.advance_to(Time::from_ps(2500 * MS));
    assert!(
        session.diagnostics().is_empty(),
        "{:?}",
        session.diagnostics()
    );
    let text = printed(&session);
    assert!(
        text.contains("mode 1\nscan 1\nap orbe ch 6\n"),
        "UART0 output:\n{text}"
    );
    let lost: Vec<_> = (session.network_rows().iter())
        .filter(|row| matches!(row.event, Event::Lost { .. }))
        .collect();
    assert!(lost.is_empty(), "{lost:?}");
}

/// The access point's BSSID in [`NETWORK`].
const AP: [u8; 6] = [0x02, 0x6f, 0x72, 0x62, 0x65, 0x01];

/// What UART0 printed.
fn printed(session: &Session) -> String {
    let uart0 = (session.usarts().iter())
        .position(|(_, usart)| *usart == "UART0")
        .expect("UART0");
    let (bytes, _) = session.serial_since(SerialCursor::default()).unwrap();
    (bytes.iter())
        .filter(|byte| usize::from(byte.usart) == uart0)
        .map(|byte| char::from(byte.byte))
        .collect()
}

/// Each frame the chip sent to the access point, once each.
fn sent_to_the_ap(session: &Session) -> Vec<AirFrame> {
    let chip = (session.actives.cores().iter())
        .position(|core| matches!(core, Core::Mcu(_)))
        .expect("the chip");
    let mut sent: Vec<AirFrame> = Vec::new();
    for (sender, frame) in session.actives.aired() {
        // A landing run again after a rewind hands its frame on again.
        if *sender == chip && frame.mpdu[4..10] == AP && !sent.iter().any(|f| f.end == frame.end) {
            sent.push(frame.clone());
        }
    }
    sent
}

/// The run reaches its stop with no fault and no diagnostic.
fn runs_to_its_stop(session: &Session, status: Status) {
    assert_eq!(status, Status::Stopped, "{:?}", session.fault());
    assert!(
        session.diagnostics().is_empty(),
        "{:?}",
        session.diagnostics()
    );
}

/// The network's rows other than the station's power-save ones.
fn joined(session: &Session) -> Vec<Event> {
    (session.network_rows().iter())
        .map(|row| row.event.clone())
        .filter(|event| !matches!(event, Event::Doze { .. } | Event::Wake { .. }))
        .collect()
}

/// The sequence number of a frame's Sequence Control field (IEEE 802.11-2020, 9.2.4.4).
fn sequence(mpdu: &[u8]) -> u16 {
    u16::from_le_bytes([mpdu[22], mpdu[23]]) >> 4
}

/// The Retry bit of Frame Control (9.2.4.1.6).
const RETRY: u8 = 0x08;

/// The network's frames to the station, once each.
fn sent_to_the_station(session: &Session) -> Vec<AirFrame> {
    let chip = (session.actives.cores().iter())
        .position(|core| matches!(core, Core::Mcu(_)))
        .expect("the chip");
    let mut sent: Vec<AirFrame> = Vec::new();
    for (sender, frame) in session.actives.aired() {
        if *sender != chip && frame.mpdu[4..10] == STA && !sent.iter().any(|f| f.end == frame.end) {
            sent.push(frame.clone());
        }
    }
    sent
}

/// LLC/SNAP and an EtherType (IEEE 802.1X-2020, 11.3; RFC 1042).
const LLC_EAPOL: [u8; 8] = [0xaa, 0xaa, 0x03, 0, 0, 0, 0x88, 0x8e];
const LLC_IPV4: [u8; 8] = [0xaa, 0xaa, 0x03, 0, 0, 0, 0x08, 0x00];

/// The Key Nonce of the EAPOL-Key frame in `mpdu`: after the LLC header, the EAPOL header's 4
/// octets, the Descriptor Type, Key Information, Key Length and Key Replay Counter (IEEE
/// 802.11-2020, 12.7.2).
fn key_nonce(mpdu: &[u8]) -> [u8; 32] {
    assert_eq!(mpdu[24..32], LLC_EAPOL);
    mpdu[49..81].try_into().unwrap()
}

/// The TK of the 4-way handshake on the air, derived apart from the chip and the network: the
/// PMK of "orbe-password" on "orbe" (12.7.1.3, PBKDF2-HMAC-SHA1 of 4096 rounds), then PRF-384
/// over the two addresses and the ANonce of message 1 and the SNonce of message 2.
fn tk(from_ap: &[AirFrame], to_ap: &[AirFrame]) -> AesKey {
    let is_eapol = |f: &&AirFrame| f.mpdu.len() > 81 && f.mpdu[24..32] == LLC_EAPOL;
    let anonce = key_nonce(&from_ap.iter().find(is_eapol).expect("message 1").mpdu);
    let snonce = key_nonce(&to_ap.iter().find(is_eapol).expect("message 2").mpdu);
    let mut pmk = [0; 32];
    pbkdf2::pbkdf2_sha1(b"orbe-password", b"orbe", 4096, &mut pmk);
    let mut data = Vec::new();
    data.extend(AP.min(STA));
    data.extend(AP.max(STA));
    data.extend(anonce.min(snonce));
    data.extend(anonce.max(snonce));
    let mut ptk = [0; 48];
    prf::prf(&pmk, b"Pairwise key expansion", &data, &mut ptk);
    AesKey::new(&ptk[32..]).unwrap()
}

/// The PN of a protected Data frame's CCMP header (12.5.3.2), and its MSDU opened under `tk`,
/// or `None` when its MIC fails.
fn opened(tk: &AesKey, mpdu: &[u8]) -> (u64, Option<Vec<u8>>) {
    let c = &mpdu[24..32];
    assert_eq!(c[3], 0x20, "Extended IV, key ID 0");
    let pn = u64::from_le_bytes([c[0], c[1], c[4], c[5], c[6], c[7], 0, 0]);
    let a2: [u8; 6] = mpdu[10..16].try_into().unwrap();
    let msdu = ccm::open(tk, &nonce(a2, pn), &aad(mpdu), &mpdu[32..], 8);
    (pn, msdu)
}

/// The DHCP message type of a DHCP message in an IPv4 UDP datagram after LLC/SNAP, from
/// `sport` to `dport`: option 53 (RFC 2132, 9.6) after the 236 fixed octets and the magic
/// cookie (RFC 2131, 3).
fn dhcp_type(msdu: &[u8], sport: u16, dport: u16) -> u8 {
    assert_eq!(msdu[..8], LLC_IPV4);
    let ip = &msdu[8..];
    assert_eq!(ip[9], 17, "UDP");
    let udp = &ip[usize::from(ip[0] & 0x0f) * 4..];
    assert_eq!(
        udp[..4],
        [
            (sport >> 8) as u8,
            sport as u8,
            (dport >> 8) as u8,
            dport as u8
        ]
    );
    let dhcp = &udp[8..];
    assert_eq!(dhcp[236..240], [99, 130, 83, 99], "the magic cookie");
    let mut options = &dhcp[240..];
    while let [code, len, rest @ ..] = options {
        if *code == 53 {
            return rest[0];
        }
        options = &rest[usize::from(*len)..];
    }
    panic!("no DHCP message type")
}

/// The stock station's `WiFi.begin("orbe", "orbe-password")` joins the network and leases an
/// address. The network logs its Open System authentication, its association as AID 1, the
/// end of the 4-way handshake and the DHCP ACK of 192.168.1.100, and loses no frame. The
/// station sends a Deauthentication, the Authentication and the Association Request, whose
/// sequence numbers the driver counts on from the scan, then the 4-way handshake's messages 2
/// and 4 as Data frames numbered 0 and 1, none with the Retry bit.
///
/// Its next Data frames are protected, sealed by its MAC under CCMP-128 with the pairwise key the
/// handshake installed. Opened under the TK this test derives from the handshake on the air,
/// the first is lwIP's DHCP Discover, with the PN the driver put in its CCMP header, and the
/// network's answer to it is a DHCP Offer. The station's DHCP Request follows the Offer, which
/// its MAC opened, and the network ACKs it. The station's PNs rise, and so do the network's.
/// Then the station goes into power save and prints its address. At about 8.995 s it looks up
/// `pool.ntp.org`, which the network answers with the router's address, asks the router for the
/// time over SNTP, which the network answers with its epoch and the time since the start, and
/// prints the date. The run then reaches its stop with no fault.
#[test]
fn the_stock_station_joins_the_network_and_prints_its_address_and_the_date_from_sntp() {
    let netlist = station(NETWORK, 9100);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let status = session.advance_to(Time::from_ps(9100 * MS));
    runs_to_its_stop(&session, status);
    let text = printed(&session);
    assert!(
        text.ends_with("scan 1\nap orbe ch 6\nip 192.168.1.100\ntime 2026-03-14\n"),
        "UART0 output:\n{text}"
    );

    let sta = Mac(STA);
    let router = Ipv4Addr::new(192, 168, 1, 1);
    let rows = joined(&session);
    let [.., Event::Sntp { utc_ms, .. }] = rows[..] else {
        panic!("{rows:?}");
    };
    assert_eq!(
        rows,
        [
            Event::Auth { sta, status: 0 },
            Event::Assoc {
                sta,
                status: 0,
                aid: Some(1)
            },
            Event::Handshake { sta },
            Event::DhcpAck {
                sta,
                ip: Ipv4Addr::new(192, 168, 1, 100)
            },
            Event::Dns {
                sta,
                name: "pool.ntp.org".into(),
                answer: Some(router)
            },
            Event::Sntp { sta, utc_ms },
        ]
    );
    let asked = (session.network_rows().iter())
        .find(|row| matches!(row.event, Event::Sntp { .. }))
        .map(|row| row.at.as_ps() / MS)
        .expect("the SNTP row");
    assert_eq!(utc_ms, 1_773_480_360_000 + asked);
    assert!((8990..9000).contains(&asked), "{asked} ms");
    assert!(
        (session.network_rows().iter()).any(|row| row.event == Event::Doze { sta }),
        "power save"
    );

    let sent = sent_to_the_ap(&session);
    let kinds: Vec<[u8; 2]> = sent.iter().map(|f| [f.mpdu[0], f.mpdu[1]]).collect();
    // Deauthentication, Authentication and Association Request (9.2.4.1.3), then two Data
    // frames to the DS, then protected ones.
    assert_eq!(
        kinds[..6],
        [
            [0xc0, 0],
            [0xb0, 0],
            [0x00, 0],
            [0x08, 0x01],
            [0x08, 0x01],
            [0x08, 0x41]
        ]
    );
    let numbers: Vec<u16> = sent[..5].iter().map(|f| sequence(&f.mpdu)).collect();
    let first = numbers[0];
    assert_eq!(numbers, [first, first + 1, first + 2, 0, 1]);
    for frame in &sent[3..5] {
        assert_eq!(frame.mpdu[33], 3, "EAPOL-Key");
    }

    let answers = sent_to_the_station(&session);
    let tk = tk(&answers, &sent[3..]);
    let protected = |frames: &[AirFrame]| -> Vec<(u64, Vec<u8>)> {
        (frames.iter())
            .filter(|f| f.mpdu[0] == 0x08 && f.mpdu[1] & 0x40 != 0)
            .map(|f| match opened(&tk, &f.mpdu) {
                (pn, Some(msdu)) => (pn, msdu),
                (pn, None) => panic!("the MIC of PN {pn} fails"),
            })
            .collect()
    };
    let from_sta = protected(&sent);
    let from_ap = protected(&answers);
    let (pn, discover) = &from_sta[0];
    // `ccmp_encap` counts the PN on from 0 in steps of 3.
    assert_eq!(*pn, 3);
    assert_eq!(dhcp_type(discover, 68, 67), 1, "DHCPDISCOVER");
    assert_eq!(dhcp_type(&from_ap[0].1, 67, 68), 2, "DHCPOFFER");
    assert_eq!(dhcp_type(&from_sta[1].1, 68, 67), 3, "DHCPREQUEST");
    assert_eq!(dhcp_type(&from_ap[1].1, 67, 68), 5, "DHCPACK");
    for pns in [&from_sta, &from_ap] {
        assert!(pns.windows(2).all(|pair| pair[0].0 < pair[1].0), "{pns:?}");
    }
}

/// When the station's Authentication reaches the network but the ACK does not reach the
/// station, its MAC reports an ACK timeout and the driver sends the same frame again, with the
/// same sequence number and the Retry bit set, after the ACK would have ended. The network
/// answers both, and the join goes on to the same lease, which the sketch prints.
#[test]
fn a_lost_ack_has_the_driver_send_the_authentication_again_with_the_retry_bit() {
    let netlist = station(NETWORK, 4300);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.actives.lose_acks(|frame| {
        frame.mpdu[0] == 0xb0 && frame.mpdu[1] & RETRY == 0 && frame.mpdu[10..16] == STA
    });
    let status = session.advance_to(Time::from_ps(4300 * MS));
    runs_to_its_stop(&session, status);
    let text = printed(&session);
    assert!(
        text.ends_with("ip 192.168.1.100\n"),
        "UART0 output:\n{text}"
    );

    let sent = sent_to_the_ap(&session);
    let auth: Vec<&AirFrame> = sent.iter().filter(|f| f.mpdu[0] == 0xb0).collect();
    let [first, again] = auth[..] else {
        panic!("{auth:?}");
    };
    assert_eq!(first.mpdu[1], 0);
    assert_eq!(again.mpdu[1], RETRY);
    assert_eq!(first.mpdu[2..], again.mpdu[2..], "the same frame");
    let acked = first.end.as_ps() + first.rate.acked_after();
    assert!(again.start.as_ps() > acked, "{again:?} after {first:?}");

    let sta = Mac(STA);
    assert_eq!(
        joined(&session),
        [
            Event::Auth { sta, status: 0 },
            Event::Auth { sta, status: 0 },
            Event::Assoc {
                sta,
                status: 0,
                aid: Some(1)
            },
            Event::Handshake { sta },
            Event::DhcpAck {
                sta,
                ip: Ipv4Addr::new(192, 168, 1, 100)
            },
        ]
    );
}

/// From the first doze at about 3.261 s, the station wakes at each TBTT, hears the beacon and
/// dozes again, and wakes to send lwIP's first ARP probes and its power-save frames. The run
/// gives the same frames on the air, the same network rows and the same output whether it
/// goes straight through or in steps of 1.013 ms from 3.2 s.
#[test]
fn the_station_dozes_and_wakes_the_same_however_the_run_splits() {
    let end = 3_800 * MS;
    let run = |step: Option<u64>| {
        let netlist = station(NETWORK, 3_800);
        let mut session = open(&netlist, Vec::new(), BUDGET);
        if let Some(step) = step {
            let mut at = 3_200 * MS;
            while at < end {
                assert_eq!(session.advance_to(Time::from_ps(at)), Status::Running);
                at += step;
            }
        }
        let status = session.advance_to(Time::from_ps(end));
        runs_to_its_stop(&session, status);
        let mut aired: Vec<AirFrame> = Vec::new();
        for (_, frame) in session.actives.aired() {
            if frame.start.as_ps() >= 3_200 * MS && !aired.iter().any(|f| f.end == frame.end) {
                aired.push(frame.clone());
            }
        }
        (printed(&session), session.network_rows().to_vec(), aired)
    };
    let straight = run(None);
    let sta = Mac(STA);
    let rows = &straight.1;
    let at = |event: &Event| {
        rows.iter()
            .find(|row| row.event == *event)
            .map(|row| row.at)
    };
    let doze = at(&Event::Doze { sta }).expect("a doze");
    assert!((3255..3265).contains(&(doze.as_ps() / MS)), "{doze:?}");
    assert!(
        rows.iter()
            .any(|row| row.at > doze && row.event == Event::Wake { sta })
    );
    let beacons = (straight.2.iter())
        .filter(|f| f.mpdu[0] == 0x80 && f.start > doze)
        .count();
    assert!(beacons >= 4, "{beacons} beacons");
    assert!(run(Some(1_013_000_000)) == straight, "in steps");
}

/// Internet checksum (RFC 1071) for constructing ICMP Echo Requests and checking Echo Replies.
fn inet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let (chunks, rem) = bytes.as_chunks::<2>();
    for chunk in chunks {
        sum += u32::from(u16::from_be_bytes(*chunk));
    }
    if let [last] = rem {
        sum += u32::from(*last) << 8;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// An LLC/SNAP IPv4 ICMP Echo Request from `src` to `dst` with `id`, `seq` and `payload`.
fn icmp_echo_request(src: Ipv4Addr, dst: Ipv4Addr, id: u16, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut icmp = vec![8, 0, 0, 0];
    icmp.extend(id.to_be_bytes());
    icmp.extend(seq.to_be_bytes());
    icmp.extend(payload);
    let icmp_sum = inet_checksum(&icmp);
    icmp[2..4].copy_from_slice(&icmp_sum.to_be_bytes());

    let total = (20 + icmp.len()) as u16;
    let mut ip = vec![0x45, 0];
    ip.extend(total.to_be_bytes());
    ip.extend([0, 0, 0x40, 0, 64, 1, 0, 0]);
    ip.extend(src.octets());
    ip.extend(dst.octets());
    let ip_sum = inet_checksum(&ip);
    ip[10..12].copy_from_slice(&ip_sum.to_be_bytes());

    let mut msdu = LLC_IPV4.to_vec();
    msdu.extend(ip);
    msdu.extend(icmp);
    msdu
}

/// After entering power save, the station wakes its RF 3 000 us before each beacon, matching the
/// ahead time `hal_set_sta_tbtt` programs (not 528 us late).
#[test]
fn the_dozing_station_wakes_3000_us_before_its_beacon() {
    let netlist = station(NETWORK, 3_600);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    // Advance just past a doze beacon so `committed[chip]` sits right after a beacon while
    // dozing, before the next beacon at 3 379 200 us.
    assert_eq!(
        session.advance_to(Time::from_ps(3_370 * MS)),
        Status::Running
    );
    let chip = (session.actives.cores().iter())
        .position(|core| matches!(core, Core::Mcu(_)))
        .expect("the chip");
    let beacon_ps = 3_379_200_000_000u64;
    let wake = session
        .actives
        .first_tuned_in(
            chip,
            Time::from_ps(beacon_ps - 5_000_000_000),
            Time::from_ps(beacon_ps),
            1_000_000,
        )
        .expect("station woke its RF before the beacon");
    // Advance past that beacon and verify it starts at `beacon_ps`.
    let status = session.advance_to(Time::from_ps(3_400 * MS));
    assert_eq!(status, Status::Running);
    let beacon = session
        .actives
        .aired()
        .iter()
        .map(|(_, f)| f)
        .find(|f| f.mpdu[0] == 0x80 && f.start.as_ps() == beacon_ps)
        .expect("the beacon at 3 379 200 us");
    let lead_us = (beacon.start.as_ps() - wake.as_ps()) / 1_000_000;
    // TBTT fires at 3 000 us ahead, and the ISR and `rom_open_i2c_xpd` power the RF ~44 us later.
    assert!(
        (2_900..=3_000).contains(&lead_us),
        "expected ~3000 us TBTT lead, got {lead_us} us (wake={} us, beacon={} us)",
        wake.as_ps() / 1_000_000,
        beacon.start.as_ps() / 1_000_000,
    );
}

/// Extracts decrypted ICMP Echo Replies matching `want_id` sent by the station to the AP,
/// verifying each reply's ICMP checksum.
fn icmp_echo_replies(session: &Session, want_id: u16) -> Vec<(u16, Vec<u8>, AirFrame)> {
    let sent = sent_to_the_ap(session);
    let answers = sent_to_the_station(session);
    let tk = tk(&answers, &sent[3..]);
    let mut replies = Vec::new();
    for frame in &sent {
        if frame.mpdu[0] == 0x08 && frame.mpdu[1] & 0x40 != 0 {
            let (_, Some(msdu)) = opened(&tk, &frame.mpdu) else {
                continue;
            };
            if msdu.starts_with(&LLC_IPV4) && msdu.len() >= 8 + 20 + 8 {
                let ip = &msdu[8..];
                let ihl = usize::from(ip[0] & 0x0f) * 4;
                let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
                if ip[9] == 1 && total <= ip.len() && total >= ihl + 8 {
                    let icmp = &ip[ihl..total];
                    if icmp[0] == 0 && icmp[1] == 0 {
                        assert_eq!(inet_checksum(icmp), 0, "ICMP reply checksum");
                        let id = u16::from_be_bytes([icmp[4], icmp[5]]);
                        let seq = u16::from_be_bytes([icmp[6], icmp[7]]);
                        if id == want_id {
                            replies.push((seq, icmp[8..].to_vec(), frame.clone()));
                        }
                    }
                }
            }
        }
    }
    replies
}

/// The last power-save event (`Doze` or `Wake`) recorded by the network.
fn last_power_save(session: &Session) -> Option<Event> {
    session
        .network_rows()
        .iter()
        .rev()
        .find(|row| matches!(row.event, Event::Doze { .. } | Event::Wake { .. }))
        .map(|row| row.event.clone())
}

/// Two ICMP Echo Requests buffered by the access point while the station dozes are delivered when
/// the station wakes on the TIM beacon, and lwIP sends both Echo Replies back-to-back through the
/// hardware TXOP table (`0x6003_437c`).
#[test]
fn two_buffered_pings_to_the_dozing_station_are_both_answered() {
    let netlist = station(NETWORK, 4_600);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    assert_eq!(
        session.advance_to(Time::from_ps(4_300 * MS)),
        Status::Running
    );
    let sta = Mac(STA);
    assert_eq!(
        last_power_save(&session),
        Some(Event::Doze { sta }),
        "station should be dozing"
    );

    let router = Ipv4Addr::new(192, 168, 1, 1);
    let sta_ip = Ipv4Addr::new(192, 168, 1, 100);
    let ping1 = icmp_echo_request(router, sta_ip, 0x1234, 1, b"first-ping");
    let ping2 = icmp_echo_request(router, sta_ip, 0x1234, 2, b"second-ping");
    let inject_at = Time::from_ps(4_300 * MS);
    session.send_to_station(inject_at, sta, &ping1).unwrap();
    session.send_to_station(inject_at, sta, &ping2).unwrap();

    let status = session.advance_to(Time::from_ps(4_600 * MS));
    runs_to_its_stop(&session, status);

    let replies = icmp_echo_replies(&session, 0x1234);
    let summary: Vec<(u16, Vec<u8>)> = replies
        .iter()
        .map(|(seq, body, _)| (*seq, body.clone()))
        .collect();
    assert_eq!(
        summary,
        vec![(1, b"first-ping".to_vec()), (2, b"second-ping".to_vec())],
        "both buffered pings must be answered"
    );
    // Both replies go in one hardware TXOP burst, so reply 2 starts SIFS (10 us) after reply 1's
    // ACK, to the chip's next 480 MHz tick.
    let after_ack = replies[0].2.end.as_ps() + replies[0].2.rate.acked_after();
    let gap_ps = replies[1].2.start.as_ps() - after_ack;
    assert!(
        (10_000_000..=10_004_168).contains(&gap_ps),
        "expected SIFS (10 us) TXOP gap after ACK, got {gap_ps} ps"
    );
}

/// An ICMP Echo Request injected while the station is already awake (during its ARP wake windows)
/// resettles the session's held landing instead of stepping past the new air frame, refuses a
/// time in the simulated past by name, delivers the frame immediately without waiting for a TIM
/// beacon, answers the post-IP ping during the wake window, and produces the same run however the
/// schedule is split.
#[test]
fn a_ping_arriving_during_the_wake_window_is_answered() {
    let sta = Mac(STA);
    let router = Ipv4Addr::new(192, 168, 1, 1);
    let sta_ip = Ipv4Addr::new(192, 168, 1, 100);

    // 1. Pre-IP ARP probe wake at 4.010 s (RF on from ~4.009 s to ~4.060 s): injecting at
    //    4 010 000 us must resettle the pre-solved 1 ms step instead of panicking, refuse a time
    //    in the past by name, and deliver/ACK the frame immediately on the air.
    {
        let netlist = station(NETWORK, 4_100);
        let mut session = open(&netlist, Vec::new(), BUDGET);
        assert_eq!(
            session.advance_to(Time::from_ps(4_010 * MS)),
            Status::Running
        );
        assert_eq!(
            last_power_save(&session),
            Some(Event::Wake { sta }),
            "station should be awake for its ARP probe at 4.010 s"
        );
        let ping = icmp_echo_request(router, sta_ip, 0x2001, 1, b"pre-ip-wake");
        assert_eq!(
            session.send_to_station(Time::from_ps(4_009 * MS), sta, &ping),
            Err(NetworkSendRejected::EventInPast),
            "event before now must be refused by name"
        );
        session
            .send_to_station(Time::from_ps(4_010 * MS), sta, &ping)
            .unwrap();
        let status = session.advance_to(Time::from_ps(4_100 * MS));
        runs_to_its_stop(&session, status);
        assert!(
            !session
                .network_rows()
                .iter()
                .any(|r| matches!(r.event, Event::Lost { .. } | Event::Dropped { .. })),
            "frame delivered during 4.010 s wake must be ACKed with no loss"
        );
        let delivered = sent_to_the_station(&session)
            .into_iter()
            .find(|f| f.start == Time::from_ps(4_010 * MS))
            .expect("frame aired immediately at 4.010 s");
        assert_eq!(delivered.end, Time::from_ps(4_010_042_000_000));
    }

    // 2. Post-IP ARP announcement wake at 5.010 s (Wake at ~5.0093 s, Doze at ~5.0596 s, next
    //    beacon at 5.0176 s): the ping airs immediately at 5.010 s, is answered before the next
    //    beacon, and gives the exact same run whether run straight through or split in 1.013 ms
    //    chunks.
    let run = |step: Option<u64>| {
        let end = 5_100 * MS;
        let inject = 5_010 * MS;
        let netlist = station(NETWORK, 5_100);
        let mut session = open(&netlist, Vec::new(), BUDGET);
        if let Some(step) = step {
            let mut at = 4_995 * MS;
            while at < inject {
                assert_eq!(session.advance_to(Time::from_ps(at)), Status::Running);
                at += step;
            }
        }
        assert_eq!(session.advance_to(Time::from_ps(inject)), Status::Running);
        assert_eq!(
            last_power_save(&session),
            Some(Event::Wake { sta }),
            "station should be awake for its ARP announcement at 5.010 s"
        );
        let ping = icmp_echo_request(router, sta_ip, 0x2002, 1, b"awake-ping");
        session
            .send_to_station(Time::from_ps(inject), sta, &ping)
            .unwrap();
        if let Some(step) = step {
            let mut at = inject + step;
            while at < end {
                assert_eq!(session.advance_to(Time::from_ps(at)), Status::Running);
                at += step;
            }
        }
        let status = session.advance_to(Time::from_ps(end));
        runs_to_its_stop(&session, status);
        let mut aired: Vec<AirFrame> = Vec::new();
        for (_, frame) in session.actives.aired() {
            if frame.start.as_ps() >= 4_995 * MS && !aired.iter().any(|f| f.end == frame.end) {
                aired.push(frame.clone());
            }
        }
        let replies = icmp_echo_replies(&session, 0x2002);
        (
            printed(&session),
            session.network_rows().to_vec(),
            aired,
            replies,
        )
    };

    let straight = run(None);
    assert!(
        !straight
            .1
            .iter()
            .any(|r| matches!(r.event, Event::Lost { .. } | Event::Dropped { .. })),
        "no lost or dropped frames: {:?}",
        straight.1
    );
    let [(seq, body, reply_frame)] = &straight.3[..] else {
        panic!("expected one Echo Reply, got {:?}", straight.3);
    };
    assert_eq!((*seq, body.as_slice()), (1, b"awake-ping".as_slice()));
    // The next beacon after 5.010 s is at 49 * 102.4 ms = 5 017 600 us; the reply finishes well
    // before it, confirming the ping was delivered immediately while awake.
    assert!(
        reply_frame.end < Time::from_ps(5_017_600_000_000),
        "expected reply before 5.0176 s beacon, ended at {} us",
        reply_frame.end.as_ps() / 1_000_000
    );
    assert!(run(Some(1_013_000_000)) == straight, "split schedule");
}

/// Four ICMP Echo Requests buffered at 4.300 s before the station has ever ARP'd the router
/// (`192.168.1.1`) are all delivered on the air and ACKed by the station's MAC (no `Lost` or
/// `Dropped` row). Stock Arduino-ESP32 3.3.12 lwIP compiles `ARP_QUEUEING=1` with
/// `ARP_QUEUE_LEN=3` (`lwip/opt.h:656`, `etharp_query` in `app.bin` at `0x4202_95ac: bgeui a10, 3`),
/// so lwIP queues replies 1 to 3 while waiting for its ARP Reply and frees reply 4 in software;
/// the 3 queued replies go out in one 10 us SIFS TXOP burst once the ARP Reply arrives.
#[test]
fn four_buffered_pings_before_arp_hit_lwips_arp_queue_limit_of_three() {
    let netlist = station(NETWORK, 4_600);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    assert_eq!(
        session.advance_to(Time::from_ps(4_300 * MS)),
        Status::Running
    );
    let sta = Mac(STA);
    assert_eq!(last_power_save(&session), Some(Event::Doze { sta }));

    let router = Ipv4Addr::new(192, 168, 1, 1);
    let sta_ip = Ipv4Addr::new(192, 168, 1, 100);
    let inject_at = Time::from_ps(4_300 * MS);
    for seq in 1..=4u16 {
        let ping = icmp_echo_request(
            router,
            sta_ip,
            0x3004,
            seq,
            format!("cold-{seq}").as_bytes(),
        );
        session.send_to_station(inject_at, sta, &ping).unwrap();
    }

    let status = session.advance_to(Time::from_ps(4_600 * MS));
    runs_to_its_stop(&session, status);
    assert!(
        !session
            .network_rows()
            .iter()
            .any(|r| matches!(r.event, Event::Lost { .. } | Event::Dropped { .. })),
        "MAC must ACK all 4 downlink frames with no loss"
    );

    // All 4 Echo Requests aired from the AP after 4.300 s.
    let sent = sent_to_the_ap(&session);
    let answers = sent_to_the_station(&session);
    let tk = tk(&answers, &sent[3..]);
    let downlink_seqs: Vec<u16> = answers
        .iter()
        .filter(|f| f.start.as_ps() >= 4_300 * MS && f.mpdu[0] == 0x08 && f.mpdu[1] & 0x40 != 0)
        .filter_map(|f| {
            let (_, Some(msdu)) = opened(&tk, &f.mpdu) else {
                return None;
            };
            if msdu.starts_with(&LLC_IPV4) && msdu.len() >= 8 + 20 + 8 && msdu[8 + 9] == 1 {
                let icmp = &msdu[8 + 20..];
                (icmp[0] == 8 && u16::from_be_bytes([icmp[4], icmp[5]]) == 0x3004)
                    .then(|| u16::from_be_bytes([icmp[6], icmp[7]]))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        downlink_seqs,
        vec![1, 2, 3, 4],
        "AP delivered all 4 requests"
    );

    let replies = icmp_echo_replies(&session, 0x3004);
    let seqs: Vec<u16> = replies.iter().map(|(seq, _, _)| *seq).collect();
    assert_eq!(
        seqs,
        vec![1, 2, 3],
        "stock lwIP ARP_QUEUE_LEN=3 queues 3 replies during ARP resolution"
    );
    for pair in replies.windows(2) {
        let after_ack = pair[0].2.end.as_ps() + pair[0].2.rate.acked_after();
        let gap_ps = pair[1].2.start.as_ps() - after_ack;
        assert!(
            (10_000_000..=10_004_168).contains(&gap_ps),
            "expected SIFS (10 us) TXOP gap, got {gap_ps} ps"
        );
    }
}

/// Once the router's MAC is in the station's ARP cache (after one prior ping resolves ARP), both
/// N=4 and N=8 buffered Echo Requests to a dozing station are all delivered and answered with
/// zero loss, and N=9 answers 8 while recording `Event::Dropped` for the 9th exceeding the AP's
/// power-save buffer (`HELD = 8`).
#[test]
fn four_and_eight_buffered_pings_with_warm_arp_are_all_answered() {
    let sta = Mac(STA);
    let router = Ipv4Addr::new(192, 168, 1, 1);
    let sta_ip = Ipv4Addr::new(192, 168, 1, 100);

    for (n, want_replies, want_dropped) in [(4u16, 4usize, 0usize), (8, 8, 0), (9, 8, 1)] {
        let netlist = station(NETWORK, 4_800);
        let mut session = open(&netlist, Vec::new(), BUDGET);
        assert_eq!(
            session.advance_to(Time::from_ps(4_250 * MS)),
            Status::Running
        );
        let warmup = icmp_echo_request(router, sta_ip, 0x4000, 0, b"arp-warm");
        session
            .send_to_station(Time::from_ps(4_250 * MS), sta, &warmup)
            .unwrap();

        assert_eq!(
            session.advance_to(Time::from_ps(4_400 * MS)),
            Status::Running
        );
        assert_eq!(
            last_power_save(&session),
            Some(Event::Doze { sta }),
            "station should be dozing before N={n} burst"
        );
        let inject_at = Time::from_ps(4_400 * MS);
        for seq in 1..=n {
            let ping = icmp_echo_request(
                router,
                sta_ip,
                0x4000 + n,
                seq,
                format!("warm-{n}-{seq}").as_bytes(),
            );
            session.send_to_station(inject_at, sta, &ping).unwrap();
        }

        let status = session.advance_to(Time::from_ps(4_800 * MS));
        runs_to_its_stop(&session, status);

        let lost = session
            .network_rows()
            .iter()
            .filter(|r| matches!(r.event, Event::Lost { .. }))
            .count();
        let dropped = session
            .network_rows()
            .iter()
            .filter(|r| matches!(r.event, Event::Dropped { .. }))
            .count();
        assert_eq!(lost, 0, "N={n}: no unacknowledged frames");
        assert_eq!(dropped, want_dropped, "N={n}: dropped row count");

        let replies = icmp_echo_replies(&session, 0x4000 + n);
        let summary: Vec<(u16, Vec<u8>)> = replies
            .iter()
            .map(|(seq, body, _)| (*seq, body.clone()))
            .collect();
        let expected: Vec<(u16, Vec<u8>)> = (1..=want_replies as u16)
            .map(|seq| (seq, format!("warm-{n}-{seq}").into_bytes()))
            .collect();
        assert_eq!(summary, expected, "N={n}: expected {want_replies} replies");
    }
}
