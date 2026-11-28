//! An ESP32-S3 in a netlist with a network is built with its radio. The stock Arduino station
//! sketch then gets through `esp_wifi_init` and `esp_wifi_start`, which `WiFi.mode(WIFI_STA)`
//! runs, with no diagnostic, and its scan puts a probe request on the air on each channel and
//! finds the network on its own. Its `WiFi.begin()` then authenticates, associates, runs the
//! 4-way handshake and leases an address over DHCP in frames its MAC seals and opens under
//! CCMP-128, and the run ends where its TBTT would first raise the power interrupt.
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
use crate::session::{BUDGET, SerialCursor, Session, Status};

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

/// The run ends where the station's TBTT would first raise the power interrupt, which the model
/// does not run, by name.
fn ends_at_the_first_tbtt(session: &Session, status: Status) -> Time {
    assert_eq!(status, Status::Faulted);
    match session.fault() {
        Some(&Diagnostic::ChipUnsupportedSetting {
            peripheral: "RADIO",
            setting: Setting::Register("tbtt"),
            at,
            ..
        }) => at,
        fault => panic!("{fault:?}"),
    }
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
/// Then the station goes into power save, and the run ends by name at about 3.274 s, where its
/// TBTT would first raise the power interrupt.
#[test]
fn the_stock_station_joins_the_network_leases_an_address_and_ends_at_its_first_tbtt() {
    let netlist = station(NETWORK, 3400);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    let status = session.advance_to(Time::from_ps(3400 * MS));
    let at = ends_at_the_first_tbtt(&session, status);
    assert!((3270..3280).contains(&(at.as_ps() / MS)), "{at:?}");
    assert!(
        session.diagnostics().is_empty(),
        "{:?}",
        session.diagnostics()
    );
    let text = printed(&session);
    assert!(
        text.ends_with("scan 1\nap orbe ch 6\n"),
        "UART0 output:\n{text}"
    );

    let sta = Mac(STA);
    assert_eq!(
        joined(&session),
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
        ]
    );
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
/// answers both, and the join goes on to the same lease and the same end.
#[test]
fn a_lost_ack_has_the_driver_send_the_authentication_again_with_the_retry_bit() {
    let netlist = station(NETWORK, 3400);
    let mut session = open(&netlist, Vec::new(), BUDGET);
    session.actives.lose_acks(|frame| {
        frame.mpdu[0] == 0xb0 && frame.mpdu[1] & RETRY == 0 && frame.mpdu[10..16] == STA
    });
    let status = session.advance_to(Time::from_ps(3400 * MS));
    ends_at_the_first_tbtt(&session, status);

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
