//! A test-only Wi-Fi station for a script core, written apart from `orbe-net`: it probes for
//! its SSID, authenticates, associates, runs the supplicant's side of the 4-way handshake,
//! then leases an address over DHCP, looks up a time server and asks it the time, each frame
//! sent in answer to one it heard (IEEE 802.11-2020 12.7.6, RFC 2131, RFC 1035, RFC 4330).

use std::collections::VecDeque;
use std::sync::Arc;

use orbe_crypto::aes::AesKey;
use orbe_crypto::{ccm, hmac, key_wrap, pbkdf2, prf};
use orbe_model::Time;
use orbe_model::air::{Ack, AirFrame, Channel, Rate};

/// What the station learned, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Log {
    /// Whether the frame it sent that ended then was acknowledged.
    Acked(Time, Ack),
    /// The access point associated it with this AID.
    Associated(u16),
    /// Message 3's MIC checked under its PTK, and its GTK unwrapped under the KEK.
    Keyed,
    /// DHCP acknowledged this lease.
    Leased([u8; 4]),
    /// DNS answered the time server's name with this address.
    Resolved([u8; 4]),
    /// The SNTP server's Transmit Timestamp, in ms from 1970, for the request that ended then.
    Time { asked: Time, utc_ms: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Probing,
    Authenticating,
    Associating,
    Handshaking,
    Leasing,
    Requesting,
    Resolving,
    Asking,
    Done,
}

/// The station. `Clone` is the checkpoint.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Station {
    pub(crate) mac: [u8; 6],
    bssid: [u8; 6],
    ssid: Arc<[u8]>,
    channel: Channel,
    pmk: [u8; 32],
    snonce: [u8; 32],
    phase: Phase,
    /// The KCK, KEK and TK, once message 1 gave the ANonce.
    ptk: Option<[u8; 48]>,
    /// The PN of its last protected frame.
    pn: u64,
    sequence: u16,
    /// Frames it will send, by end, oldest first, none overlapping.
    queue: VecDeque<AirFrame>,
    /// The frame that ended at its last stop, until the landing there hands it on.
    aired: Option<AirFrame>,
    lease: [u8; 4],
    /// When its SNTP request ended.
    asked: Time,
    pub(crate) log: Vec<Log>,
}

/// How long after a frame it heard it starts its answer.
const TURNAROUND_PS: u64 = 200_000_000;
/// Every frame goes at 1 Mb/s.
const RATE: Rate = Rate::Dsss1;
/// Its DHCP transaction ID, and the source port of its DNS query.
const XID: [u8; 4] = [0x0b, 0xe5, 0x00, 0x01];
const DNS_PORT: u16 = 0xc001;
/// The name it looks up.
const SERVER: &str = "pool.ntp.org";
const LLC_EAPOL: [u8; 8] = [0xaa, 0xaa, 0x03, 0, 0, 0, 0x88, 0x8e];
const LLC_IPV4: [u8; 8] = [0xaa, 0xaa, 0x03, 0, 0, 0, 0x08, 0x00];
/// Supported Rates and Extended Supported Rates: every ERP rate.
const RATES: [u8; 8] = [0x82, 0x84, 0x8b, 0x96, 0x24, 0x30, 0x48, 0x6c];
const EXTENDED: [u8; 4] = [0x0c, 0x12, 0x18, 0x60];
/// CCMP-128 pairwise and group, PSK.
const RSNE: [u8; 20] = [
    1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 2, 0, 0,
];

impl Station {
    /// A station at `mac` for the WPA2-PSK network `ssid` at `bssid` on `channel`, which
    /// probes with a frame that starts at `start`.
    pub(crate) fn new(
        mac: [u8; 6],
        bssid: [u8; 6],
        ssid: &[u8],
        passphrase: &str,
        channel: Channel,
        start: Time,
    ) -> Station {
        let mut pmk = [0; 32];
        pbkdf2::pbkdf2_sha1(passphrase.as_bytes(), ssid, 4096, &mut pmk);
        let mut station = Station {
            mac,
            bssid,
            ssid: ssid.into(),
            channel,
            pmk,
            snonce: [mac[5]; 32],
            phase: Phase::Probing,
            ptk: None,
            pn: 0,
            sequence: 0,
            queue: VecDeque::new(),
            aired: None,
            lease: [0; 4],
            asked: Time::ZERO,
            log: Vec::new(),
        };
        let mut body = vec![0, ssid.len() as u8];
        body.extend(ssid);
        body.extend(rates());
        let probe = station.management(4, [0xff; 6], [0xff; 6], &body);
        station.send_from(start, probe);
        station
    }

    pub(crate) fn tuned(&self) -> Option<Channel> {
        Some(self.channel)
    }

    /// The end of the next frame it sends.
    pub(crate) fn next(&self) -> Option<Time> {
        self.queue.front().map(|frame| frame.end)
    }

    /// Puts the frame that ends at `at` on the air.
    pub(crate) fn air(&mut self, at: Time) {
        let frame = self.queue.pop_front().expect("a frame ends here");
        assert_eq!(frame.end, at);
        assert!(self.aired.is_none(), "a landing hands on each frame");
        self.aired = Some(frame);
    }

    pub(crate) fn aired(&self) -> Option<&AirFrame> {
        self.aired.as_ref()
    }

    pub(crate) fn drop_aired(&mut self) {
        self.aired = None;
    }

    pub(crate) fn acked(&mut self, at: Time, ack: Ack) {
        self.log.push(Log::Acked(at, ack));
    }

    /// A frame it heard that ended at `at`. It acknowledges every individually addressed
    /// Management or Data frame to it, and answers the one its phase waits for.
    pub(crate) fn hear(&mut self, at: Time, frame: &AirFrame) -> Ack {
        let m = &frame.mpdu;
        let (kind, subtype) = ((m[0] >> 2) & 3, m[0] >> 4);
        if m.len() < 24 || m[4..10] != self.mac || kind == 1 {
            return Ack::None;
        }
        // An Authentication frame's status follows its algorithm and transaction, and an
        // Association Response's its capabilities.
        let status = |at: usize| u16::from_le_bytes([m[at], m[at + 1]]);
        match (self.phase, kind, subtype) {
            (Phase::Probing, 0, 5) => {
                self.phase = Phase::Authenticating;
                let auth = self.management(11, self.bssid, self.bssid, &[0, 0, 1, 0, 0, 0]);
                self.reply(at, auth);
            }
            (Phase::Authenticating, 0, 11) if status(28) == 0 => {
                self.phase = Phase::Associating;
                let mut body = vec![0x31, 0x04, 0x0a, 0x00, 0, self.ssid.len() as u8];
                body.extend(self.ssid.iter());
                body.extend(&rates()[..10]);
                body.extend([48, RSNE.len() as u8]);
                body.extend(RSNE);
                body.extend(&rates()[10..]);
                let assoc = self.management(0, self.bssid, self.bssid, &body);
                self.reply(at, assoc);
            }
            (Phase::Associating, 0, 1) if status(26) == 0 => {
                let aid = u16::from_le_bytes([m[28], m[29]]) & 0x3fff;
                self.log.push(Log::Associated(aid));
                self.phase = Phase::Handshaking;
            }
            (Phase::Handshaking, 2, _) if m.get(24..32) == Some(&LLC_EAPOL) => {
                self.handshake(at, &m[32..]);
            }
            (_, 2, _) if m[1] & 0x40 != 0 => {
                if let Some(msdu) = self.open(m) {
                    self.subnet(at, &msdu);
                }
            }
            _ => {}
        }
        Ack::Acked
    }

    /// Message 1 or 3 of the 4-way handshake, as its EAPOL frame.
    fn handshake(&mut self, at: Time, eapol: &[u8]) {
        let info = u16::from_be_bytes([eapol[5], eapol[6]]);
        let replay = u64::from_be_bytes(eapol[9..17].try_into().expect("8 octets"));
        if info & 0x0100 == 0 {
            // Message 1: Pairwise, ACK, no MIC.
            let anonce: [u8; 32] = eapol[17..49].try_into().expect("32 octets");
            self.ptk = Some(self.derive(&anonce));
            let data = [&[48, RSNE.len() as u8][..], &RSNE].concat();
            let message_2 = self.key(0x010a, replay, self.snonce, &data);
            self.reply(at, message_2);
            return;
        }
        // Message 3: Install, ACK, MIC, Secure and Encrypted Key Data.
        let ptk = self.ptk.expect("message 1 came first");
        let mut zeroed = eapol.to_vec();
        zeroed[81..97].fill(0);
        let mic = hmac::hmac_sha1(&ptk[..16], &zeroed);
        assert_eq!(eapol[81..97], mic[..16], "message 3's MIC under the PTK");
        let len = usize::from(u16::from_be_bytes([eapol[97], eapol[98]]));
        let kek = AesKey::new(&ptk[16..32]).expect("a 128-bit key");
        let data = key_wrap::unwrap(&kek, &eapol[99..99 + len]).expect("key data under the KEK");
        assert!(
            data.windows(4).any(|w| w == [0x00, 0x0f, 0xac, 0x01]),
            "a GTK KDE in {data:02x?}"
        );
        self.log.push(Log::Keyed);
        let message_4 = self.key(0x030a, replay, [0; 32], &[]);
        self.reply(at, message_4);
        self.phase = Phase::Leasing;
        let discover = self.sealed([0xff; 6], &dhcp(1, self.mac, None, None));
        self.reply(at, discover);
    }

    /// An MSDU the access point sent it under the TK, from its LLC header on.
    fn subnet(&mut self, at: Time, msdu: &[u8]) {
        if msdu.len() < 36 || msdu[..8] != LLC_IPV4 || msdu[17] != 17 {
            return;
        }
        let src: [u8; 4] = msdu[20..24].try_into().expect("an address");
        let sport = u16::from_be_bytes([msdu[28], msdu[29]]);
        let data = &msdu[36..];
        match (self.phase, sport) {
            (Phase::Leasing | Phase::Requesting, 67) => {
                let yiaddr: [u8; 4] = data[16..20].try_into().expect("an address");
                let server = option(data, 54).map(|server| server.try_into().expect("4 octets"));
                match (self.phase, option(data, 53)) {
                    (Phase::Leasing, Some([2])) => {
                        self.phase = Phase::Requesting;
                        let request = dhcp(3, self.mac, Some(yiaddr), server);
                        let request = self.sealed([0xff; 6], &request);
                        self.reply(at, request);
                    }
                    (Phase::Requesting, Some([5])) => {
                        self.lease = yiaddr;
                        self.log.push(Log::Leased(yiaddr));
                        self.phase = Phase::Resolving;
                        let query = dns_query(self.lease, server.unwrap_or(src));
                        let query = self.sealed(self.bssid, &query);
                        self.reply(at, query);
                    }
                    _ => {}
                }
            }
            (Phase::Resolving, 53) if data[3] & 0x0f == 0 && data[7] > 0 => {
                let answer: [u8; 4] = data[data.len() - 4..].try_into().expect("an address");
                self.log.push(Log::Resolved(answer));
                self.phase = Phase::Asking;
                let mut request = vec![0x23, 0, 6, 0];
                request.extend([0; 44]);
                let ask = udp(self.lease, 123, answer, 123, &request);
                let frame = self.sealed(self.bssid, &ask);
                self.asked = self.reply(at, frame);
            }
            (Phase::Asking, 123) => {
                let seconds = u64::from(u32::from_be_bytes(data[40..44].try_into().expect("4")));
                let fraction = u64::from(u32::from_be_bytes(data[44..48].try_into().expect("4")));
                let utc_ms = (seconds - 2_208_988_800) * 1000 + ((fraction * 1000) >> 32);
                self.log.push(Log::Time {
                    asked: self.asked,
                    utc_ms,
                });
                self.phase = Phase::Done;
            }
            _ => {}
        }
    }

    /// Queues `mpdu` to start a turnaround after `at`, or after the frame it queued last.
    /// Returns its end.
    fn reply(&mut self, at: Time, mpdu: Vec<u8>) -> Time {
        let start = Time::from_ps(at.as_ps() + TURNAROUND_PS);
        let start = self.queue.back().map_or(start, |last| start.max(last.end));
        self.send_from(start, mpdu)
    }

    fn send_from(&mut self, start: Time, mut mpdu: Vec<u8>) -> Time {
        mpdu[22..24].copy_from_slice(&(self.sequence << 4).to_le_bytes());
        self.sequence = (self.sequence + 1) & 0xfff;
        let end = Time::from_ps(start.as_ps() + RATE.airtime(mpdu.len() + 4));
        self.queue.push_back(AirFrame {
            start,
            end,
            channel: self.channel,
            rate: RATE,
            mpdu: mpdu.into(),
        });
        end
    }

    fn management(&self, subtype: u8, addr1: [u8; 6], addr3: [u8; 6], body: &[u8]) -> Vec<u8> {
        let mut f = vec![subtype << 4, 0, 0, 0];
        f.extend(addr1);
        f.extend(self.mac);
        f.extend(addr3);
        f.extend([0, 0]);
        f.extend(body);
        f
    }

    /// A Data frame to the DS for `da`, with `flags` set in its second octet.
    fn data(&self, flags: u8, da: [u8; 6]) -> Vec<u8> {
        let mut f = vec![0x08, 0x01 | flags, 0, 0];
        f.extend(self.bssid);
        f.extend(self.mac);
        f.extend(da);
        f.extend([0, 0]);
        f
    }

    /// The KCK, the KEK and the TK under `anonce` (12.7.1.3).
    fn derive(&self, anonce: &[u8; 32]) -> [u8; 48] {
        let mut data = Vec::new();
        data.extend(self.bssid.min(self.mac));
        data.extend(self.bssid.max(self.mac));
        data.extend((*anonce).min(self.snonce));
        data.extend((*anonce).max(self.snonce));
        let mut ptk = [0; 48];
        prf::prf(&self.pmk, b"Pairwise key expansion", &data, &mut ptk);
        ptk
    }

    /// An EAPOL-Key frame with Key Information `info`, its MIC under the KCK.
    fn key(&self, info: u16, replay: u64, nonce: [u8; 32], data: &[u8]) -> Vec<u8> {
        let mut eapol = vec![1, 3];
        eapol.extend(((95 + data.len()) as u16).to_be_bytes());
        eapol.push(2);
        eapol.extend(info.to_be_bytes());
        eapol.extend([0, 0]);
        eapol.extend(replay.to_be_bytes());
        eapol.extend(nonce);
        eapol.extend([0; 16 + 8 + 8 + 16]);
        eapol.extend((data.len() as u16).to_be_bytes());
        eapol.extend(data);
        let ptk = self.ptk.expect("a PTK");
        let mic = hmac::hmac_sha1(&ptk[..16], &eapol);
        eapol[81..97].copy_from_slice(&mic[..16]);
        let mut f = self.data(0, self.bssid);
        f.extend(LLC_EAPOL);
        f.extend(eapol);
        f
    }

    /// A Data frame for `da` of `msdu`, sealed under CCMP-128 with its next PN (12.5.3).
    fn sealed(&mut self, da: [u8; 6], msdu: &[u8]) -> Vec<u8> {
        self.pn += 1;
        let mut f = self.data(0x40, da);
        let p = self.pn.to_be_bytes();
        f.extend([p[7], p[6], 0, 0x20, p[5], p[4], p[3], p[2]]);
        let (aad, nonce) = (aad(&f), nonce(self.mac, self.pn));
        let tk = AesKey::new(&self.ptk.expect("a PTK")[32..]).expect("a 128-bit key");
        let body = ccm::seal(&tk, &nonce, &aad, msdu, 8);
        f.extend(body);
        f
    }

    /// The MSDU of a protected Data frame from the DS, or `None` when its MIC fails.
    fn open(&self, mpdu: &[u8]) -> Option<Vec<u8>> {
        let ptk = self.ptk?;
        let c = mpdu.get(24..32)?;
        let pn = u64::from_le_bytes([c[0], c[1], c[4], c[5], c[6], c[7], 0, 0]);
        let a2: [u8; 6] = mpdu[10..16].try_into().expect("an address");
        let tk = AesKey::new(&ptk[32..]).expect("a 128-bit key");
        ccm::open(&tk, &nonce(a2, pn), &aad(mpdu), &mpdu[32..], 8)
    }
}

/// The CCMP AAD of a Data frame (12.5.3.3.3): its Frame Control with Retry, Power Management
/// and More Data masked, its addresses, and its Sequence Control's fragment number.
pub(crate) fn aad(mpdu: &[u8]) -> Vec<u8> {
    let mut aad = vec![mpdu[0] & 0x8f, mpdu[1] & !0x38];
    aad.extend(&mpdu[4..22]);
    aad.extend([mpdu[22] & 0x0f, 0]);
    aad
}

/// The CCMP nonce of a Data frame with no QoS Control (12.5.3.3.4): priority 0, Address 2 and the
/// PN.
pub(crate) fn nonce(a2: [u8; 6], pn: u64) -> Vec<u8> {
    let mut nonce = vec![0];
    nonce.extend(a2);
    nonce.extend(&pn.to_be_bytes()[2..]);
    nonce
}

/// The Supported Rates element, then the Extended Supported Rates element.
fn rates() -> Vec<u8> {
    let mut out = vec![1, RATES.len() as u8];
    out.extend(RATES);
    out.extend([50, EXTENDED.len() as u8]);
    out.extend(EXTENDED);
    out
}

/// The Internet checksum (RFC 1071).
fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = (data.chunks(2))
        .map(|w| u32::from(w[0]) << 8 | u32::from(*w.get(1).unwrap_or(&0)))
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// A UDP datagram in IPv4, from its LLC header on.
fn udp(src: [u8; 4], sport: u16, dst: [u8; 4], dport: u16, data: &[u8]) -> Vec<u8> {
    let mut u = sport.to_be_bytes().to_vec();
    u.extend(dport.to_be_bytes());
    u.extend(((8 + data.len()) as u16).to_be_bytes());
    u.extend([0, 0]);
    u.extend(data);
    let mut pseudo = [&src[..], &dst, &[0, 17], &(u.len() as u16).to_be_bytes()].concat();
    pseudo.extend(&u);
    u[6..8].copy_from_slice(&checksum(&pseudo).to_be_bytes());
    let mut h = vec![0x45, 0];
    h.extend(((20 + u.len()) as u16).to_be_bytes());
    h.extend([0, 1, 0, 0, 64, 17, 0, 0]);
    h.extend(src);
    h.extend(dst);
    let sum = checksum(&h);
    h[10..12].copy_from_slice(&sum.to_be_bytes());
    [&LLC_IPV4[..], &h, &u].concat()
}

/// A DHCP message of `kind` from 0.0.0.0 to the broadcast address.
fn dhcp(kind: u8, mac: [u8; 6], requested: Option<[u8; 4]>, server: Option<[u8; 4]>) -> Vec<u8> {
    let mut m = vec![1, 1, 6, 0];
    m.extend(XID);
    m.extend([0; 20]);
    m.extend(mac);
    m.extend([0; 10 + 192]);
    m.extend([99, 130, 83, 99, 53, 1, kind]);
    for (code, ip) in [(50, requested), (54, server)] {
        if let Some(ip) = ip {
            m.extend([code, 4]);
            m.extend(ip);
        }
    }
    m.extend([55, 3, 1, 3, 6, 255]);
    udp([0; 4], 68, [255; 4], 67, &m)
}

/// The option `code` of a DHCP message.
fn option(message: &[u8], code: u8) -> Option<&[u8]> {
    let mut o = message.get(240..)?;
    while let [c, len, rest @ ..] = o {
        let len = usize::from(*len);
        if *c == code {
            return rest.get(..len);
        }
        o = rest.get(len..)?;
    }
    None
}

/// A standard query for the address of [`SERVER`], with RD set.
fn dns_query(src: [u8; 4], dst: [u8; 4]) -> Vec<u8> {
    let mut q = vec![0x4e, 0x21, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in SERVER.split('.') {
        q.push(label.len() as u8);
        q.extend(label.as_bytes());
    }
    q.extend([0, 0, 1, 0, 1]);
    udp(src, DNS_PORT, dst, 53, &q)
}
