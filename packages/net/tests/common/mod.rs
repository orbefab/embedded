//! A scripted station and the air between it and the network, for the tests.

#![allow(dead_code)]

pub mod lan;

use std::sync::Arc;

use orbe_model::Time;
use orbe_model::air::{Ack, AirFrame, Channel, Rate, Signal};
use orbe_net::{Config, Mac, Network, Security};

/// The access point and the station of Wireshark's `wpa-Induction.pcap`
/// (https://wiki.wireshark.org/SampleCaptures, SHA-256
/// 2b57dca7fa2c3bd0e942060b546028d961bfb698fb12ed8b2947b13f88d170c8).
pub const BSSID: Mac = Mac([0x00, 0x0c, 0x41, 0x82, 0xb2, 0x55]);
pub const STA: Mac = Mac([0x00, 0x0d, 0x93, 0x82, 0x36, 0x3a]);
pub const SSID: &[u8] = b"Coherer";
/// The capture's passphrase.
pub const PASSPHRASE: &str = "Induction";
pub const WPA2: Security<'static> = Security::Wpa2Psk {
    passphrase: PASSPHRASE,
};

pub const SIGNAL: Signal = Signal {
    rssi_dbm: -40,
    noise_dbm: -95,
};

pub fn channel() -> Channel {
    Channel::new(1).unwrap()
}

/// The access point of the capture: SSID "Coherer" on channel 1.
pub fn network(security: Security) -> Network {
    Network::new(Config::new(SSID, BSSID, channel(), security).unwrap())
}

pub fn ms(ms: f64) -> Time {
    Time::from_ps((ms * 1e9) as u64)
}

pub fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Frames of the capture, without their FCS, by frame number.
pub mod capture {
    /// The access point's first beacon.
    pub const BEACON_1: &str = "80000000ffffffffffff000c4182b255000c4182b25550f889f1d41b01000000640011040007436f6865726572010882848b962430486c0301010504000100002a01022f010230180100000fac020200000fac04000fac020100000fac02000032040c121860dd06001018020004dd1c0050f20101000050f20202000050f2040050f20201000050f2020000";
    /// The station's Probe Request for "Coherer".
    pub const PROBE_58: &str = "40000000ffffffffffff000d9382363affffffffffff10000007436f6865726572010802040b162430486c32040c121860";
    /// The access point's Probe Response to it.
    pub const PROBE_RESPONSE_59: &str = "50003a01000d9382363a000c4182b255000c4182b255f0fb61ff231c01000000640011040007436f6865726572010882848b962430486c0301012a01022f010230180100000fac020200000fac04000fac020100000fac02000032040c121860dd06001018020004dd1c0050f20101000050f20202000050f2040050f20201000050f2020000";
    /// The station's Authentication, Open System, transaction 1.
    pub const AUTH_78: &str = "b0003a01000c4182b255000d9382363a000c4182b2557001000001000000";
    /// The access point's Authentication, transaction 2, status 0, with a Broadcom vendor
    /// element at its end.
    pub const AUTH_80: &str =
        "b0003a01000d9382363a000c4182b255000c4182b25590fc000002000000dd06001018020004";
    /// The station's Association Request. Its RSNE asks for a TKIP group key.
    pub const ASSOC_82: &str = "00003a01000c4182b255000d9382363a000c4182b255800131040a000007436f6865726572010882848b962430486c30140100000fac020100000fac040100000fac02000032040c121860";
    /// The access point's Association Response, status 0, AID 1, with the same vendor element.
    pub const ASSOC_RESPONSE_84: &str = "10003a01000d9382363a000c4182b255000c4182b255a0fc1104000001c0010882848b962430486c32040c121860dd06001018020004";
    /// The station's Probe Request for any SSID.
    pub const PROBE_999: &str =
        "40000000ffffffffffff000d9382363affffffffffffe0090000010802040b162430486c32040c121860";
    /// The station's Disassociation, reason 8.
    pub const DISASSOC_1050: &str = "a0003a01000c4182b255000d9382363a000c4182b255500b0800";
    /// The vendor element the access point puts at the end of frames 80 and 84.
    pub const BROADCOM: &str = "dd06001018020004";
}

/// The station's Association Request of frame 82 with its RSNE's group cipher made CCMP-128,
/// which the access point offers.
pub fn assoc_ccmp() -> Vec<u8> {
    let mut frame = hex(capture::ASSOC_82);
    let group = 24 + 4 + 9 + 10 + 2 + 2;
    assert_eq!(frame[group..group + 4], [0x00, 0x0f, 0xac, 2]);
    frame[group + 3] = 4;
    frame
}

// Frames a station sends to the access point.

fn management(subtype: u8, flags: u8, body: &[u8]) -> Vec<u8> {
    let mut f = vec![subtype << 4, flags, 0x3a, 0x01];
    f.extend(BSSID.0);
    f.extend(STA.0);
    f.extend(BSSID.0);
    f.extend([0, 0]);
    f.extend(body);
    f
}

pub fn auth(algorithm: u16, sequence: u16) -> Vec<u8> {
    let mut body = algorithm.to_le_bytes().to_vec();
    body.extend(sequence.to_le_bytes());
    body.extend([0, 0]);
    management(11, 0, &body)
}

/// The Supported Rates and Extended Supported Rates of the capture's station: every ERP rate,
/// up to 54 Mb/s.
pub const STA_RATES: [u8; 8] = [0x82, 0x84, 0x8b, 0x96, 0x24, 0x30, 0x48, 0x6c];
pub const STA_EXTENDED: [u8; 4] = [0x0c, 0x12, 0x18, 0x60];

/// An Association Request for `ssid`, with `rsne` when given, offering the rates of the
/// capture's station.
pub fn assoc(ssid: &[u8], rsne: Option<&[u8]>) -> Vec<u8> {
    assoc_rates(ssid, rsne, &STA_RATES, &STA_EXTENDED)
}

/// An Association Request for `ssid`, with `rsne` when given, offering `rates` and, when not
/// empty, `extended`, in the order of frame 82 of the capture.
pub fn assoc_rates(ssid: &[u8], rsne: Option<&[u8]>, rates: &[u8], extended: &[u8]) -> Vec<u8> {
    let mut body = vec![0x31, 0x04, 0x0a, 0x00, 0, ssid.len() as u8];
    body.extend(ssid);
    body.extend([1, rates.len() as u8]);
    body.extend(rates);
    if let Some(rsne) = rsne {
        body.extend([48, rsne.len() as u8]);
        body.extend(rsne);
    }
    if !extended.is_empty() {
        body.extend([50, extended.len() as u8]);
        body.extend(extended);
    }
    management(0, 0, &body)
}

pub const RSN_PSK: [u8; 20] = [
    1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 2, 0, 0,
];

pub fn deauth(reason: u16) -> Vec<u8> {
    management(12, 0, &reason.to_le_bytes())
}

/// An ADDBA Request for `tid`: Block Ack category 3, action 0 (IEEE 802.11-2020 9.6.4.2),
/// immediate policy, a 64-frame buffer, no timeout and starting sequence 0.
pub fn addba(token: u8, tid: u8, dozing: bool) -> Vec<u8> {
    let params: u16 = 1 << 1 | u16::from(tid) << 2 | 64 << 6;
    let mut body = vec![3, 0, token];
    body.extend(params.to_le_bytes());
    body.extend([0, 0, 0, 0]);
    management(13, if dozing { 0x10 } else { 0 }, &body)
}

/// A Null frame to the DS with the Power Management bit `dozing`.
pub fn null(dozing: bool) -> Vec<u8> {
    let mut f = vec![0x48, 0x01 | if dozing { 0x10 } else { 0 }, 0x3a, 0x01];
    f.extend(BSSID.0);
    f.extend(STA.0);
    f.extend(BSSID.0);
    f.extend([0, 0]);
    f
}

/// A PS-Poll for `aid` (IEEE 802.11-2020 9.3.1.5), with the Power Management bit set.
pub fn ps_poll(aid: u16) -> Vec<u8> {
    let mut f = vec![0xa4, 0x10];
    f.extend((aid | 0xc000).to_le_bytes());
    f.extend(BSSID.0);
    f.extend(STA.0);
    f
}

/// A frame from the station that ends at `end`, at 1 Mb/s.
pub fn from_station(end: Time, mpdu: &[u8]) -> AirFrame {
    let airtime = Rate::Dsss1.airtime(mpdu.len() + 4);
    AirFrame {
        start: Time::from_ps(end.as_ps().saturating_sub(airtime)),
        end,
        channel: channel(),
        rate: Rate::Dsss1,
        mpdu: mpdu.into(),
    }
}

/// What the station does at a time.
#[derive(Clone, Debug)]
pub enum Step {
    /// Puts a frame on the air that ends then.
    Send(Vec<u8>),
    /// Takes the access point up or down.
    Up(bool),
}

/// A station's script: its steps in time order, and the spans it dozes, its RF off.
#[derive(Clone, Debug, Default)]
pub struct Script {
    pub steps: Vec<(Time, Step)>,
    pub dozes: Vec<(Time, Time)>,
}

impl Script {
    pub fn send(mut self, at: Time, mpdu: Vec<u8>) -> Script {
        self.steps.push((at, Step::Send(mpdu)));
        self
    }

    pub fn up(mut self, at: Time, up: bool) -> Script {
        self.steps.push((at, Step::Up(up)));
        self
    }

    pub fn doze(mut self, from: Time, to: Time) -> Script {
        self.dozes.push((from, to));
        self
    }

    fn awake(&self, at: Time) -> bool {
        !self.dozes.iter().any(|&(from, to)| from <= at && at < to)
    }
}

/// What the air saw, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// A frame the network sent, and whether the station acknowledged it.
    Aired(AirFrame, Ack),
    /// Whether the network acknowledged the station's frame that ended then.
    Heard(Time, Ack),
    Row(String),
}

/// The air between one scripted station and the network. `Clone` is a checkpoint, as the
/// network's is.
#[derive(Clone, Debug)]
pub struct Air {
    script: Arc<Script>,
    next: usize,
    at: Time,
    pub seen: Vec<Seen>,
}

impl Air {
    pub fn new(script: Script) -> Air {
        Air {
            script: Arc::new(script),
            next: 0,
            at: Time::ZERO,
            seen: Vec::new(),
        }
    }

    /// Runs the network and the script to `until`. `split` may put a limit in `(from, to)`
    /// before each `run_until`.
    pub fn run(
        &mut self,
        net: &mut Network,
        until: Time,
        split: &mut dyn FnMut(Time, Time) -> Option<Time>,
    ) {
        loop {
            let step = self
                .script
                .steps
                .get(self.next)
                .filter(|(at, _)| *at <= until)
                .cloned();
            let target = step.as_ref().map_or(until, |(at, _)| *at);
            self.run_to(net, target, split);
            let Some((at, step)) = step else {
                break;
            };
            match step {
                Step::Send(mpdu) => {
                    let heard = match net.tuned() {
                        Some(channel) if channel == self::channel() => {
                            net.hear(at, &from_station(at, &mpdu), SIGNAL)
                        }
                        _ => Ack::None,
                    };
                    self.seen.push(Seen::Heard(at, heard));
                }
                Step::Up(up) => net.set_up(at, up),
            }
            self.next += 1;
            self.rows(net);
        }
    }

    pub fn run_through(&mut self, net: &mut Network, until: Time) {
        self.run(net, until, &mut |_, _| None);
    }

    fn run_to(
        &mut self,
        net: &mut Network,
        target: Time,
        split: &mut dyn FnMut(Time, Time) -> Option<Time>,
    ) {
        loop {
            let limit = split(self.at, target)
                .filter(|&limit| self.at < limit && limit < target)
                .unwrap_or(target);
            match net.run_until(limit) {
                Some(stop) => {
                    assert!(stop.record && !stop.sample);
                    assert!(
                        self.at < stop.at && stop.at <= limit,
                        "{stop:?} out of ({:?}, {limit:?}]",
                        self.at
                    );
                    self.at = stop.at;
                    let mut aired = Vec::new();
                    net.take_aired(|frame| aired.push(frame));
                    let [frame] = <[AirFrame; 1]>::try_from(aired).expect("one frame at a stop");
                    assert_eq!(frame.end, stop.at);
                    // Every station the script names acknowledges a frame to it while awake.
                    let ack = if frame.mpdu[4] & 1 == 0 && self.script.awake(stop.at) {
                        Ack::Acked
                    } else {
                        Ack::None
                    };
                    net.acked(stop.at, ack);
                    self.seen.push(Seen::Aired(frame, ack));
                    self.rows(net);
                }
                None => {
                    self.at = limit;
                    if limit == target {
                        return;
                    }
                }
            }
        }
    }

    fn rows(&mut self, net: &mut Network) {
        net.take_rows(|row| self.seen.push(Seen::Row(row.json())));
    }

    /// The frames the network sent, in order.
    pub fn aired(&self) -> Vec<&AirFrame> {
        (self.seen.iter())
            .filter_map(|seen| match seen {
                Seen::Aired(frame, _) => Some(frame),
                _ => None,
            })
            .collect()
    }

    /// The frames the network sent that were not beacons.
    pub fn sent(&self) -> Vec<&AirFrame> {
        self.aired()
            .into_iter()
            .filter(|f| f.mpdu[0] != 0x80)
            .collect()
    }

    pub fn rows_seen(&self) -> Vec<&str> {
        (self.seen.iter())
            .filter_map(|seen| match seen {
                Seen::Row(row) => Some(row.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// The elements of a Beacon or Probe Response body after its 12 octets of fixed fields.
pub fn elements(mpdu: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut body = &mpdu[24 + 12..];
    let mut out = Vec::new();
    while let [id, len, rest @ ..] = body {
        let len = usize::from(*len);
        out.push((*id, rest[..len].to_vec()));
        body = &rest[len..];
    }
    out
}

/// A seeded xorshift, so a failing run replays.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// `frame` sent by `sta` instead of [`STA`].
pub fn by(sta: Mac, mut frame: Vec<u8>) -> Vec<u8> {
    frame[10..16].copy_from_slice(&sta.0);
    frame
}

/// A station's session with the [`keyed_network`]: it probes, joins, runs the 4-way handshake
/// and sends Data frames under CCMP, one of them again, has an ADDBA Request declined, dozes
/// with a frame held for it and polls for it, wakes, leaves and joins again, which starts
/// another handshake. Then the access point goes down and comes up, and the station's next
/// frame is refused.
pub fn session() -> (Script, Time) {
    let sta = Supplicant::new(PASSPHRASE);
    let script = Script::default()
        .send(ms(10.0), hex(capture::PROBE_58))
        .send(ms(20.0), hex(capture::AUTH_78))
        .send(ms(30.0), assoc_ccmp())
        .send(ms(33.0), sta.message_2(&ANONCE, 0, &RSN_PSK))
        .send(ms(37.0), sta.message_4(&ANONCE, 1))
        .send(ms(38.0), sta.data(&ANONCE, 1, b"one"))
        .send(ms(39.0), sta.data(&ANONCE, 1, b"one"))
        .send(ms(40.0), addba(1, 0, false))
        .send(ms(50.0), null(true))
        .doze(ms(50.5), ms(59.5))
        .send(ms(60.0), addba(2, 6, true))
        .doze(ms(60.5), ms(101.5))
        .send(ms(104.0), ps_poll(1))
        .doze(ms(106.0), ms(204.0))
        .send(ms(205.0), null(false))
        .send(ms(210.0), hex(capture::DISASSOC_1050))
        .send(ms(220.0), null(false))
        .send(ms(230.0), assoc_ccmp())
        .up(ms(300.0), false)
        .send(ms(400.0), hex(capture::PROBE_58))
        .up(ms(500.0), true)
        .send(ms(520.0), null(false));
    (script, ms(700.0))
}

/// The key counter the tests' networks start at, so a station knows the ANonce of message 1.
pub const ANONCE: [u8; 32] = [0xa5; 32];

/// The access point of the capture under WPA2-PSK, its key counter at [`ANONCE`].
pub fn keyed_network() -> Network {
    Network::new(
        Config::new(SSID, BSSID, channel(), WPA2)
            .unwrap()
            .with_counter(ANONCE),
    )
}

/// The ANonce plus one, the Key IV of the first message 3.
pub fn anonce_plus(n: u8) -> [u8; 32] {
    let mut nonce = ANONCE;
    nonce[31] += n;
    nonce
}

/// A station's side of WPA2-PSK, written apart from the network's: it answers messages 1 and 3
/// from its passphrase, and seals its Data frames under CCMP-128 (IEEE 802.11-2020 12.7.6,
/// 12.5.3).
pub struct Supplicant {
    pmk: [u8; 32],
    pub snonce: [u8; 32],
}

/// The Key Information of message 2 and of message 4.
const KEY_2: u16 = 0x010a;
const KEY_4: u16 = 0x030a;

impl Supplicant {
    pub fn new(passphrase: &str) -> Supplicant {
        let mut pmk = [0; 32];
        orbe_crypto::pbkdf2::pbkdf2_sha1(passphrase.as_bytes(), SSID, 4096, &mut pmk);
        Supplicant {
            pmk,
            snonce: [0x5a; 32],
        }
    }

    /// The KCK, the KEK and the TK under `anonce`.
    pub fn ptk(&self, anonce: &[u8; 32]) -> [u8; 48] {
        let mut data = Vec::new();
        data.extend(BSSID.0.min(STA.0));
        data.extend(BSSID.0.max(STA.0));
        data.extend((*anonce).min(self.snonce));
        data.extend((*anonce).max(self.snonce));
        let mut ptk = [0; 48];
        orbe_crypto::prf::prf(&self.pmk, b"Pairwise key expansion", &data, &mut ptk);
        ptk
    }

    fn key(
        &self,
        anonce: &[u8; 32],
        info: u16,
        replay: u64,
        nonce: [u8; 32],
        data: &[u8],
    ) -> Vec<u8> {
        let mut eapol = vec![1, 3];
        eapol.extend(((95 + data.len()) as u16).to_be_bytes());
        eapol.push(2);
        eapol.extend(info.to_be_bytes());
        eapol.extend(0u16.to_be_bytes());
        eapol.extend(replay.to_be_bytes());
        eapol.extend(nonce);
        eapol.extend([0; 16 + 8 + 8 + 16]);
        eapol.extend((data.len() as u16).to_be_bytes());
        eapol.extend(data);
        let mic = orbe_crypto::hmac::hmac_sha1(&self.ptk(anonce)[..16], &eapol);
        eapol[81..97].copy_from_slice(&mic[..16]);
        let mut f = vec![0x08, 0x01, 0x3a, 0x01];
        f.extend(BSSID.0);
        f.extend(STA.0);
        f.extend(BSSID.0);
        f.extend([0, 0]);
        f.extend(hex("aaaa03000000888e"));
        f.extend(eapol);
        f
    }

    /// Message 2 answering message 1 with Key Replay Counter `replay`, with the RSNE body
    /// `rsne`.
    pub fn message_2(&self, anonce: &[u8; 32], replay: u64, rsne: &[u8]) -> Vec<u8> {
        let mut data = vec![48, rsne.len() as u8];
        data.extend(rsne);
        self.key(anonce, KEY_2, replay, self.snonce, &data)
    }

    pub fn message_4(&self, anonce: &[u8; 32], replay: u64) -> Vec<u8> {
        self.key(anonce, KEY_4, replay, [0; 32], &[])
    }

    /// A Data frame to the DS of `payload` after LLC/SNAP and EtherType IPv4, sealed with PN
    /// `pn`.
    pub fn data(&self, anonce: &[u8; 32], pn: u64, payload: &[u8]) -> Vec<u8> {
        let mut msdu = hex("aaaa030000000800");
        msdu.extend(payload);
        self.sealed(anonce, pn, Mac::BROADCAST, &msdu)
    }

    /// A Data frame to the DS for `da` of `msdu`, from its LLC header on, sealed with PN `pn`.
    pub fn sealed(&self, anonce: &[u8; 32], pn: u64, da: Mac, msdu: &[u8]) -> Vec<u8> {
        let mut f = vec![0x08, 0x41, 0x3a, 0x01];
        f.extend(BSSID.0);
        f.extend(STA.0);
        f.extend(da.0);
        f.extend([0x10, 0]);
        let mut aad = vec![0x08, 0x41];
        aad.extend(&f[4..22]);
        aad.extend([0, 0]);
        let p = pn.to_be_bytes();
        let mut nonce = vec![0];
        nonce.extend(STA.0);
        nonce.extend(&p[2..]);
        f.extend([p[7], p[6], 0, 0x20, p[5], p[4], p[3], p[2]]);
        let tk = orbe_crypto::aes::AesKey::new(&self.ptk(anonce)[32..]).unwrap();
        f.extend(orbe_crypto::ccm::seal(&tk, &nonce, &aad, msdu, 8));
        f
    }

    /// The PN and the MSDU of a protected Data frame from the DS, under the TK of `anonce`, or
    /// `None` when its MIC fails.
    pub fn open(&self, anonce: &[u8; 32], mpdu: &[u8]) -> Option<(u64, Vec<u8>)> {
        assert_eq!(mpdu[..2], [0x08, 0x42]);
        let c = &mpdu[24..32];
        let pn = u64::from_le_bytes([c[0], c[1], c[4], c[5], c[6], c[7], 0, 0]);
        let mut aad = vec![0x08, 0x42 & !0x38];
        aad.extend(&mpdu[4..22]);
        aad.extend([mpdu[22] & 0x0f, 0]);
        let mut nonce = vec![0];
        nonce.extend(&mpdu[10..16]);
        nonce.extend(&pn.to_be_bytes()[2..]);
        let tk = orbe_crypto::aes::AesKey::new(&self.ptk(anonce)[32..]).unwrap();
        let msdu = orbe_crypto::ccm::open(&tk, &nonce, &aad, &mpdu[32..], 8)?;
        Some((pn, msdu))
    }
}

/// A Data frame to the DS for `da` of `msdu`, from its LLC header on, in the clear, from
/// station `sta`.
pub fn clear(sta: Mac, da: Mac, msdu: &[u8]) -> Vec<u8> {
    let mut f = vec![0x08, 0x01, 0x3a, 0x01];
    f.extend(BSSID.0);
    f.extend(sta.0);
    f.extend(da.0);
    f.extend([0x10, 0]);
    f.extend(msdu);
    f
}

/// The Data frames from the DS that carry a body, past the EAPOL ones, and when each started.
pub fn data_sent(air: &Air) -> Vec<&AirFrame> {
    (air.sent().into_iter())
        .filter(|f| f.mpdu[0] == 0x08 && f.mpdu[1] & 0x02 != 0)
        .filter(|f| f.mpdu.get(24..32) != Some(&hex("aaaa03000000888e")))
        .collect()
}

/// When the station of [`subnet_session`] sends each request.
pub mod at {
    pub const DISCOVER: f64 = 50.0;
    pub const REQUEST: f64 = 60.0;
    pub const PROBE: f64 = 70.0;
    pub const ARP: f64 = 80.0;
    pub const DNS: f64 = 90.0;
    pub const SNTP: f64 = 100.0;
    pub const PING: f64 = 110.0;
    pub const NXDOMAIN: f64 = 115.0;
}

/// The station's requests after it joins, as lwIP sends them, each an MSDU and the address it
/// goes to: it asks for a lease, probes the address, finds the router, looks up a time server,
/// asks it the time, pings the router and looks up a name the network does not serve.
pub fn requests() -> Vec<(f64, Mac, Vec<u8>)> {
    use lan::*;
    vec![
        (
            at::DISCOVER,
            Mac::BROADCAST,
            dhcp(DISCOVER, [0; 4], None, None),
        ),
        (
            at::REQUEST,
            Mac::BROADCAST,
            dhcp(REQUEST, [0; 4], Some(LEASE), Some(ROUTER)),
        ),
        (at::PROBE, Mac::BROADCAST, arp_request([0; 4], LEASE)),
        (at::ARP, Mac::BROADCAST, arp_request(LEASE, ROUTER)),
        (at::DNS, BSSID, dns_query(0x4e21, "pool.ntp.org", 1)),
        (at::SNTP, BSSID, ntp_request(NTP_SENT)),
        (at::PING, BSSID, ping(0x0101, 7, b"orbe ping")),
        (at::NXDOMAIN, BSSID, dns_query(0x4e22, "pool.ntp.orq", 1)),
    ]
}

/// The station's session on the [`keyed_network`]: it joins and runs the 4-way handshake,
/// then sends the [`requests`] under CCMP.
pub fn subnet_session() -> (Script, Time) {
    let sta = Supplicant::new(PASSPHRASE);
    let mut script = Script::default()
        .send(ms(10.0), auth(0, 1))
        .send(ms(20.0), assoc_ccmp())
        .send(ms(30.0), sta.message_2(&ANONCE, 0, &RSN_PSK))
        .send(ms(40.0), sta.message_4(&ANONCE, 1));
    for (pn, (at, da, msdu)) in (1..).zip(requests()) {
        script = script.send(ms(at), sta.sealed(&ANONCE, pn, da, &msdu));
    }
    (script, ms(130.0))
}

/// The same on an open network, in the clear.
pub fn open_subnet_session() -> (Script, Time) {
    let mut script = Script::default()
        .send(ms(10.0), auth(0, 1))
        .send(ms(20.0), assoc(SSID, None));
    for (at, da, msdu) in requests() {
        script = script.send(ms(at), clear(STA, da, &msdu));
    }
    (script, ms(130.0))
}

/// The EAPOL-Key frames the network sent, as their EAPOL frames.
pub fn eapol_sent(air: &Air) -> Vec<Vec<u8>> {
    (air.sent().into_iter())
        .filter(|f| f.mpdu[0] == 0x08 && f.mpdu.get(24..32) == Some(&hex("aaaa03000000888e")))
        .map(|f| f.mpdu[32..].to_vec())
        .collect()
}
