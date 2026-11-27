//! The closed-world network a radio chip's frames reach: one access point on the air.
//!
//! A [`Network`] is a station on the air, as a chip with a radio is. It speaks through the
//! same calls as `orbe_mcu::Mcu`'s radio: [`run_until`](Network::run_until),
//! [`tuned`](Network::tuned), [`take_aired`](Network::take_aired), [`hear`](Network::hear) and
//! [`acked`](Network::acked), with the types of [`orbe_model::air`]. Its access point beacons,
//! answers probes, authenticates with Open System, associates with an AID, deauthenticates,
//! declines ADDBA, holds frames for a station in power save, and goes down and up. Its RSNE
//! advertises WPA2-PSK, whose handshake is not here yet. Frames follow IEEE 802.11-2020.
//!
//! It is its state: [`Clone`] is a checkpoint. What a scene sets is a [`Config`] behind an
//! [`Arc`], which every copy shares, and the rest is small. Its inputs are times and frames,
//! so equal networks given equal calls stay equal, and a run split at any limit is the same
//! run. Nothing in the engine builds one yet.
//!
//! It sends every frame at 1 Mb/s, DIFS after the medium goes idle, with no random backoff. A
//! frame no station acknowledges is not sent again, and makes a [`Event::Lost`] row. It holds
//! no group frames for a station in power save, and it is neither HT nor QoS.

mod element;
mod frame;
mod row;
mod timing;

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use orbe_model::air::{Ack, AirFrame, Channel, Signal};
use orbe_model::{Stop, Time};

pub use row::{By, Event, Row};

use frame::Header;
use timing::{DIFS, RATE};

/// `ps` picoseconds after `at`.
fn after(at: Time, ps: u64) -> Time {
    Time::from_ps(at.as_ps().saturating_add(ps))
}

/// A MAC address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    pub const BROADCAST: Mac = Mac([0xff; 6]);

    /// Whether it names a group, by its Individual/Group bit.
    pub const fn is_group(self) -> bool {
        self.0[0] & 1 != 0
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{g:02x}")
    }
}

/// What the access point asks of a station that associates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// No RSNE: any station associates.
    Open,
    /// An RSNE with CCMP-128 and the PSK AKM. A station associates when its own RSNE asks for
    /// them.
    Wpa2Psk,
}

/// The access point a scene sets. Every copy of a network shares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    ssid: Box<[u8]>,
    bssid: Mac,
    channel: Channel,
    security: Security,
}

impl Config {
    /// An access point named `ssid`, 1 to 32 octets, at `bssid`, an individual address, on
    /// `channel`. `None` for any other SSID or BSSID.
    pub fn new(ssid: &[u8], bssid: Mac, channel: Channel, security: Security) -> Option<Config> {
        (matches!(ssid.len(), 1..=32) && !bssid.is_group()).then(|| Config {
            ssid: ssid.into(),
            bssid,
            channel,
            security,
        })
    }
}

/// The most stations it keeps, authenticated or associated. Their AIDs are 1 to this.
pub const MAX_STATIONS: usize = 8;
/// The most frames it queues to send.
const QUEUE: usize = 16;
/// The most frames it holds for one station in power save.
const HELD: usize = 8;

// Status codes (9.4.1.9, Table 9-50).
const SUCCESS: u16 = 0;
const UNSPECIFIED_FAILURE: u16 = 1;
const UNSUPPORTED_AUTH_ALGORITHM: u16 = 13;
const AUTH_OUT_OF_SEQUENCE: u16 = 14;
const AP_FULL: u16 = 17;
const REQUEST_DECLINED: u16 = 37;

// Reason codes (9.4.1.7, Table 9-49).
const LEAVING: u16 = 3;
const CLASS2_FROM_NONAUTH: u16 = 6;
const CLASS3_FROM_NONASSOC: u16 = 7;

/// The Open System Authentication Algorithm Number (9.4.1.1).
const OPEN_SYSTEM: u16 = 0;
/// The Block Ack category (9.4.1.11, Table 9-51) and its ADDBA actions (9.6.4.1).
const BLOCK_ACK: u8 = 3;
const ADDBA_REQUEST: u8 = 0;
const ADDBA_RESPONSE: u8 = 1;

/// The network: one access point, and where its run has reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    config: Arc<Config>,
    state: State,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    reached: Time,
    power: Power,
    /// The earliest its next frame may start: DIFS after the medium last went idle, which is
    /// after its own last frame and the ACK it waited for, or after a frame it heard and the
    /// ACK it answered.
    idle: Time,
    /// The next target beacon transmission time, a whole number of beacon intervals from 0.
    tbtt: Time,
    /// The last sequence number it sent.
    sequence: u16,
    stations: Vec<Station>,
    queue: VecDeque<Pending>,
    /// The frame on the air now, and the station that acknowledges it.
    airing: Option<(AirFrame, Option<Mac>)>,
    /// The frame that ended at the last stop, until [`Network::take_aired`].
    aired: Option<AirFrame>,
    /// The end of its last individually addressed frame, and its station, until
    /// [`Network::acked`].
    awaiting: Option<(Time, Mac)>,
    rows: Vec<Row>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Power {
    Up,
    /// Down, once the Deauthentication it sends every station has gone.
    Leaving,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Station {
    mac: Mac,
    /// Its AID once associated, or `None` while only authenticated.
    aid: Option<u16>,
    /// Whether it is in power save, from the Power Management bit of its last frame.
    dozing: bool,
    /// The frames the access point holds for it while it dozes, oldest first.
    held: VecDeque<Arc<[u8]>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    /// The earliest it may start.
    ready: Time,
    /// The station it goes to, or `None` for every station.
    to: Option<Mac>,
    out: Out,
    /// It answers a PS-Poll, so it goes while the station dozes, with More Data when the
    /// access point holds more.
    polled: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Out {
    /// A Probe Response, built when it starts, since its Timestamp is then.
    Probe,
    Mpdu(Arc<[u8]>),
}

impl Network {
    pub fn new(config: Config) -> Network {
        Network {
            config: Arc::new(config),
            state: State {
                reached: Time::ZERO,
                power: Power::Up,
                idle: Time::ZERO,
                tbtt: Time::ZERO,
                sequence: 0xfff,
                stations: Vec::new(),
                queue: VecDeque::new(),
                airing: None,
                aired: None,
                awaiting: None,
                rows: Vec::new(),
            },
        }
    }

    /// Runs from the last time it reached. Returns the first stop in `(reached, limit]`, where
    /// a frame it sent ends, with `record` set, or `None` once it has done everything up to
    /// `limit`. A run split at any limit is the same run.
    pub fn run_until(&mut self, limit: Time) -> Option<Stop> {
        let limit = limit.max(self.state.reached);
        loop {
            if let Some((frame, _)) = &self.state.airing {
                if frame.end > limit {
                    break;
                }
                return Some(self.end_airing());
            }
            if !self.start_next(limit) {
                break;
            }
        }
        self.state.reached = limit;
        None
    }

    /// The channel it listens on: its own while the access point is up, `None` otherwise.
    pub fn tuned(&self) -> Option<Channel> {
        (self.state.power == Power::Up).then_some(self.config.channel)
    }

    /// Hands over the frame it put on the air that ended at the stop `run_until` last returned.
    pub fn take_aired(&mut self, mut aired: impl FnMut(AirFrame)) {
        if let Some(frame) = self.state.aired.take() {
            aired(frame);
        }
    }

    /// Another station's frame on its channel, which ended at `at`, where the network has run
    /// to. It returns whether the access point acknowledges it: every individually addressed
    /// Management or Data frame to its BSSID, and a PS-Poll. What it causes comes at least
    /// DIFS later.
    pub fn hear(&mut self, at: Time, frame: &AirFrame, _signal: Signal) -> Ack {
        if self.tuned() != Some(frame.channel) {
            return Ack::None;
        }
        let header = Header::parse(&frame.mpdu);
        let bssid = self.config.bssid;
        let acks = header.is_some_and(|h| {
            h.addr1 == bssid && (h.kind != frame::CONTROL || h.subtype == frame::PS_POLL)
        });
        let busy = if acks {
            timing::acked_after(frame.rate)
        } else {
            0
        };
        self.state.idle = self.state.idle.max(after(at, busy + DIFS));
        if let Some(h) = header
            && (h.addr1 == bssid || h.addr1 == Mac::BROADCAST)
            && !h.addr2.is_group()
        {
            self.receive(at, &h, &frame.mpdu[h.body..]);
        }
        if acks { Ack::Acked } else { Ack::None }
    }

    /// Whether the station acknowledged the frame it put on the air that ended at `at`.
    pub fn acked(&mut self, at: Time, ack: Ack) {
        if let Some((end, sta)) = self.state.awaiting.take()
            && end == at
            && ack == Ack::None
        {
            self.row(at, Event::Lost { sta });
        }
    }

    /// Takes the access point up or down at `at`, where the network has run to. Going down, it
    /// sends every station a Deauthentication, reason 3, and forgets them. Coming up, it
    /// beacons from the next target beacon time.
    pub fn set_up(&mut self, at: Time, up: bool) {
        let s = &mut self.state;
        match (up, s.power) {
            (false, Power::Up) => {
                s.queue.clear();
                let leaving = !s.stations.is_empty();
                s.stations.clear();
                s.power = if leaving { Power::Leaving } else { Power::Down };
                self.row(at, Event::Down);
                if leaving {
                    self.send_deauth(at, Mac::BROADCAST, LEAVING, true);
                }
            }
            (true, Power::Leaving | Power::Down) => {
                s.power = Power::Up;
                let interval = timing::BEACON_INTERVAL;
                s.tbtt = Time::from_ps((at.as_ps() / interval + 1) * interval);
                self.row(at, Event::Up);
            }
            _ => {}
        }
    }

    /// Hands over each row it made, in order.
    pub fn take_rows(&mut self, mut row: impl FnMut(Row)) {
        self.state.rows.drain(..).for_each(&mut row);
    }

    fn row(&mut self, at: Time, event: Event) {
        self.state.rows.push(Row { at, event });
    }

    /// Ends the frame on the air at its end, which is a stop.
    fn end_airing(&mut self) -> Stop {
        let s = &mut self.state;
        let (frame, to) = s.airing.take().expect("a frame on the air");
        let end = frame.end;
        let acked = if to.is_some() {
            timing::acked_after(frame.rate)
        } else {
            0
        };
        s.reached = end;
        s.idle = s.idle.max(after(end, acked + DIFS));
        s.awaiting = to.map(|sta| (end, sta));
        s.aired = Some(frame);
        if s.power == Power::Leaving && s.queue.is_empty() {
            s.power = Power::Down;
        }
        Stop {
            at: end,
            sample: false,
            record: true,
        }
    }

    /// Puts its next frame on the air, if it starts by `limit`: a beacon at its target time,
    /// first on a tie, or the frame at the head of the queue. A frame to a station that dozes
    /// goes to that station's held frames instead. Returns whether it did either.
    fn start_next(&mut self, limit: Time) -> bool {
        let s = &self.state;
        let beacon = (s.power == Power::Up).then(|| s.tbtt.max(s.idle));
        let queued = s.queue.front().map(|p| p.ready.max(s.idle));
        let start = match (beacon, queued) {
            (Some(b), Some(q)) if q < b => q,
            (Some(b), _) => b,
            (None, Some(q)) => q,
            (None, None) => return false,
        };
        if start > limit {
            return false;
        }
        if beacon == Some(start) {
            let mpdu = self.beacon(start);
            let interval = timing::BEACON_INTERVAL;
            self.state.tbtt =
                Time::from_ps((start.as_ps() / interval + 1).saturating_mul(interval));
            self.air(start, mpdu, None);
            return true;
        }
        let pending = self.state.queue.pop_front().expect("a queued frame");
        if let (Out::Mpdu(mpdu), Some(to), None) = (&pending.out, pending.to, pending.polled)
            && frame::bufferable(mpdu)
            && let Some(sta) = self.station(to)
            && sta.dozing
        {
            if sta.held.len() < HELD {
                sta.held.push_back(mpdu.clone());
            } else {
                self.row(start, Event::Dropped { sta: to });
            }
            return true;
        }
        let mut mpdu = match pending.out {
            Out::Probe => self.probe_response(start, pending.to.unwrap_or(Mac::BROADCAST)),
            Out::Mpdu(mpdu) => mpdu.to_vec(),
        };
        if pending.polled == Some(true) {
            mpdu[1] |= frame::MORE_DATA;
        }
        self.air(start, mpdu, pending.to);
        true
    }

    fn air(&mut self, start: Time, mut mpdu: Vec<u8>, to: Option<Mac>) {
        let s = &mut self.state;
        s.sequence = (s.sequence + 1) & 0xfff;
        frame::set_sequence(&mut mpdu, s.sequence);
        let frame = AirFrame {
            start,
            end: after(start, RATE.airtime(mpdu.len() + 4)),
            channel: self.config.channel,
            rate: RATE,
            mpdu: mpdu.into(),
        };
        s.airing = Some((frame, to));
    }

    fn station(&mut self, mac: Mac) -> Option<&mut Station> {
        self.state.stations.iter_mut().find(|sta| sta.mac == mac)
    }

    fn queue(&mut self, at: Time, to: Mac, out: Out, polled: Option<bool>) {
        if self.state.queue.len() < QUEUE {
            let to = (!to.is_group()).then_some(to);
            self.state.queue.push_back(Pending {
                ready: at,
                to,
                out,
                polled,
            });
        } else {
            self.row(at, Event::Dropped { sta: to });
        }
    }

    fn send(&mut self, at: Time, subtype: u8, to: Mac, body: &[u8]) {
        let duration = if to.is_group() {
            0
        } else {
            timing::duration_us()
        };
        let mut mpdu = frame::header(
            frame::MANAGEMENT,
            subtype,
            0,
            duration,
            to,
            self.config.bssid,
        );
        mpdu.extend(body);
        self.queue(at, to, Out::Mpdu(mpdu.into()), None);
    }

    fn send_deauth(&mut self, at: Time, sta: Mac, reason: u16, deauth: bool) {
        let (subtype, event) = if deauth {
            (
                frame::DEAUTH,
                Event::Deauth {
                    sta,
                    reason,
                    by: By::Ap,
                },
            )
        } else {
            (
                frame::DISASSOC,
                Event::Disassoc {
                    sta,
                    reason,
                    by: By::Ap,
                },
            )
        };
        self.send(at, subtype, sta, &reason.to_le_bytes());
        self.row(at, event);
    }

    /// Timestamp, Beacon Interval and Capability Information, then the elements a Beacon
    /// (9.3.3.2, Table 9-27) and a Probe Response (9.3.3.10) share up to the TIM: SSID,
    /// Supported Rates and DSSS Parameter Set.
    fn bss_head(&self, start: Time, out: &mut Vec<u8>) {
        // The Timestamp is the TSF, microseconds from 0, when its first bit is on the air
        // (9.4.1.10): after the 192 us PLCP and the 24-octet header at 1 Mb/s.
        let tsf = start.as_ps() / timing::US + 192 + 24 * 8;
        out.extend(tsf.to_le_bytes());
        out.extend(timing::BEACON_INTERVAL_TU.to_le_bytes());
        out.extend(self.capability().to_le_bytes());
        element::push(out, element::SSID, &self.config.ssid);
        element::push(out, element::SUPPORTED_RATES, &element::RATES);
        element::push(out, element::DSSS, &[self.config.channel.number()]);
    }

    /// The elements after the TIM: ERP, with no non-ERP station and no protection (9.4.2.11),
    /// Extended Supported Rates, and the RSNE under WPA2-PSK.
    fn bss_tail(&self, out: &mut Vec<u8>) {
        element::push(out, element::ERP, &[0]);
        element::push(out, element::EXTENDED_RATES, &element::EXTENDED);
        if self.config.security == Security::Wpa2Psk {
            element::push(out, element::RSN, &element::RSN_PSK);
        }
    }

    /// Capability Information (9.4.1.4): ESS, Short Slot Time, and Privacy under WPA2-PSK.
    fn capability(&self) -> u16 {
        let privacy = if self.config.security == Security::Wpa2Psk {
            1 << 4
        } else {
            0
        };
        1 | privacy | 1 << 10
    }

    fn beacon(&self, start: Time) -> Vec<u8> {
        let mut mpdu = frame::header(
            frame::MANAGEMENT,
            frame::BEACON,
            0,
            0,
            Mac::BROADCAST,
            self.config.bssid,
        );
        self.bss_head(start, &mut mpdu);
        let beacons = start.as_ps() / timing::BEACON_INTERVAL;
        let period = u64::from(timing::DTIM_PERIOD);
        let count = ((period - beacons % period) % period) as u8;
        let mut aids: Vec<u16> = (self.state.stations.iter())
            .filter(|sta| !sta.held.is_empty())
            .filter_map(|sta| sta.aid)
            .collect();
        aids.sort_unstable();
        element::tim(&mut mpdu, count, timing::DTIM_PERIOD, aids.into_iter());
        self.bss_tail(&mut mpdu);
        mpdu
    }

    fn probe_response(&self, start: Time, to: Mac) -> Vec<u8> {
        let mut mpdu = frame::header(
            frame::MANAGEMENT,
            frame::PROBE_RESPONSE,
            0,
            timing::duration_us(),
            to,
            self.config.bssid,
        );
        self.bss_head(start, &mut mpdu);
        self.bss_tail(&mut mpdu);
        mpdu
    }

    /// A frame to its BSSID, or a group frame, from station `h.addr2`, whose body is `body`.
    fn receive(&mut self, at: Time, h: &Header, body: &[u8]) {
        let sta = h.addr2;
        let bssid = self.config.bssid;
        let to_us = h.addr1 == bssid;
        match (h.kind, h.subtype) {
            (frame::MANAGEMENT, frame::PROBE_REQUEST) => {
                if h.addr3.is_some_and(|a| a == bssid || a == Mac::BROADCAST) {
                    self.probe(at, sta, body);
                }
            }
            (frame::MANAGEMENT, _) if !to_us || h.addr3 != Some(bssid) => {}
            // A protected management frame needs keys the access point does not have yet.
            (frame::MANAGEMENT, _) if h.flags & frame::PROTECTED != 0 => {}
            (frame::MANAGEMENT, frame::AUTH) => self.authenticate(at, sta, body),
            (frame::MANAGEMENT, frame::ASSOC_REQUEST) => self.associate(at, sta, body, false),
            (frame::MANAGEMENT, frame::REASSOC_REQUEST) => self.associate(at, sta, body, true),
            (frame::MANAGEMENT, frame::DISASSOC) => {
                let reason = frame::u16_at(body, 0).unwrap_or(0);
                if let Some(station) = self.station(sta)
                    && station.aid.is_some()
                {
                    station.aid = None;
                    station.dozing = false;
                    station.held.clear();
                    self.row(
                        at,
                        Event::Disassoc {
                            sta,
                            reason,
                            by: By::Station,
                        },
                    );
                }
            }
            (frame::MANAGEMENT, frame::DEAUTH) => {
                let reason = frame::u16_at(body, 0).unwrap_or(0);
                let before = self.state.stations.len();
                self.state.stations.retain(|station| station.mac != sta);
                if self.state.stations.len() < before {
                    self.row(
                        at,
                        Event::Deauth {
                            sta,
                            reason,
                            by: By::Station,
                        },
                    );
                }
            }
            (frame::MANAGEMENT, frame::ACTION) if to_us && self.class3(at, sta) => {
                self.action(at, sta, body);
            }
            (frame::DATA, _)
                if to_us && h.flags & (frame::TO_DS | frame::FROM_DS) == frame::TO_DS =>
            {
                self.class3(at, sta);
            }
            (frame::CONTROL, frame::PS_POLL) if to_us && self.class3(at, sta) => {
                self.poll(at, sta, h.duration & 0x3fff);
            }
            _ => {}
        }
        if h.kind != frame::CONTROL && to_us {
            self.power_save(at, sta, h.flags & frame::PWR_MGT != 0);
        }
    }

    /// Answers a Probe Request whose SSID is the wildcard or its own (11.1.4.3).
    fn probe(&mut self, at: Time, sta: Mac, body: &[u8]) {
        let Some(ssid) = element::find(body, element::SSID) else {
            return;
        };
        if ssid.is_empty() || *ssid == *self.config.ssid {
            self.queue(at, sta, Out::Probe, None);
        }
    }

    /// Open System authentication (12.3.3.2): the station's frame 1 gets frame 2, with the
    /// status. Another algorithm is refused with status 13, and a frame out of sequence with
    /// 14. A station that authenticates again loses its association.
    fn authenticate(&mut self, at: Time, sta: Mac, body: &[u8]) {
        let (Some(algorithm), Some(sequence)) = (frame::u16_at(body, 0), frame::u16_at(body, 2))
        else {
            return;
        };
        let mut status = if algorithm != OPEN_SYSTEM {
            UNSUPPORTED_AUTH_ALGORITHM
        } else if sequence != 1 {
            AUTH_OUT_OF_SEQUENCE
        } else {
            SUCCESS
        };
        if status == SUCCESS {
            if let Some(station) = self.station(sta) {
                station.aid = None;
                station.dozing = false;
                station.held.clear();
            } else if self.state.stations.len() < MAX_STATIONS {
                self.state.stations.push(Station {
                    mac: sta,
                    aid: None,
                    dozing: false,
                    held: VecDeque::new(),
                });
            } else {
                status = AP_FULL;
            }
        }
        let mut reply = Vec::with_capacity(6);
        reply.extend(algorithm.to_le_bytes());
        reply.extend(sequence.wrapping_add(1).to_le_bytes());
        reply.extend(status.to_le_bytes());
        self.send(at, frame::AUTH, sta, &reply);
        self.row(at, Event::Auth { sta, status });
    }

    /// An Association or Reassociation Request (9.3.3.5, 9.3.3.7) from an authenticated
    /// station (11.3.5.3). Its SSID must be the access point's, and under WPA2-PSK its RSNE
    /// must ask for CCMP-128 and PSK. The station keeps its AID, or takes the lowest free one.
    /// A station that has not authenticated gets a Deauthentication, reason 6 (11.3.3).
    fn associate(&mut self, at: Time, sta: Mac, body: &[u8], reassoc: bool) {
        // Capability Information and Listen Interval, then the current AP's address in a
        // Reassociation Request.
        let Some(elements) = body.get(if reassoc { 10 } else { 4 }..) else {
            return;
        };
        if self.station(sta).is_none() {
            self.send_deauth(at, sta, CLASS2_FROM_NONAUTH, true);
            return;
        }
        let ssid = element::find(elements, element::SSID);
        let status = if ssid != Some(&self.config.ssid) {
            UNSPECIFIED_FAILURE
        } else if self.config.security == Security::Wpa2Psk {
            match element::find(elements, element::RSN) {
                Some(rsne) => element::check_rsne(rsne).err().unwrap_or(SUCCESS),
                None => element::INVALID_ELEMENT,
            }
        } else {
            SUCCESS
        };
        let used: Vec<u16> = self.state.stations.iter().filter_map(|s| s.aid).collect();
        let station = self.station(sta).expect("an authenticated station");
        let aid = if status == SUCCESS {
            let aid = station
                .aid
                .unwrap_or_else(|| (1..).find(|aid| !used.contains(aid)).expect("a free AID"));
            station.aid = Some(aid);
            Some(aid)
        } else {
            station.aid = None;
            station.held.clear();
            None
        };
        station.dozing = false;
        // Capability Information, Status Code, and the AID with its two top bits set (9.4.1.8),
        // then the rates (9.3.3.6).
        let mut reply = Vec::with_capacity(24);
        reply.extend(self.capability().to_le_bytes());
        reply.extend(status.to_le_bytes());
        reply.extend(aid.map_or(0, |aid| aid | 0xc000).to_le_bytes());
        element::push(&mut reply, element::SUPPORTED_RATES, &element::RATES);
        element::push(&mut reply, element::EXTENDED_RATES, &element::EXTENDED);
        let subtype = if reassoc {
            frame::REASSOC_RESPONSE
        } else {
            frame::ASSOC_RESPONSE
        };
        self.send(at, subtype, sta, &reply);
        self.row(at, Event::Assoc { sta, status, aid });
    }

    /// Whether `sta` is associated, so it may send a Class 3 frame. A station that is not gets
    /// a Disassociation, reason 7, when it has authenticated, and a Deauthentication otherwise
    /// (11.3.3).
    fn class3(&mut self, at: Time, sta: Mac) -> bool {
        match self.station(sta).map(|station| station.aid.is_some()) {
            Some(true) => true,
            Some(false) => {
                self.send_deauth(at, sta, CLASS3_FROM_NONASSOC, false);
                false
            }
            None => {
                self.send_deauth(at, sta, CLASS3_FROM_NONASSOC, true);
                false
            }
        }
    }

    /// Declines an ADDBA Request (9.6.4.2) with an ADDBA Response (9.6.4.3) of status 37 that
    /// echoes its Dialog Token, Block Ack Parameter Set and Block Ack Timeout. Other Action
    /// frames are dropped.
    fn action(&mut self, at: Time, sta: Mac, body: &[u8]) {
        let Some(&[BLOCK_ACK, ADDBA_REQUEST, token, p0, p1, t0, t1, ..]) = body.get(..9) else {
            return;
        };
        let mut reply = vec![BLOCK_ACK, ADDBA_RESPONSE, token];
        reply.extend(REQUEST_DECLINED.to_le_bytes());
        reply.extend([p0, p1, t0, t1]);
        self.send(at, frame::ACTION, sta, &reply);
        // The TID is bits 2 to 5 of the Block Ack Parameter Set (9.4.1.13).
        self.row(
            at,
            Event::AddbaDeclined {
                sta,
                tid: (p0 >> 2) & 0xf,
            },
        );
    }

    /// The Power Management bit of a frame from an associated station (11.2.3.2). Going into
    /// power save, the access point holds its bufferable frames. Leaving it, it sends them.
    fn power_save(&mut self, at: Time, sta: Mac, dozing: bool) {
        let Some(station) = self.station(sta) else {
            return;
        };
        if station.aid.is_none() || station.dozing == dozing {
            return;
        }
        station.dozing = dozing;
        let held = std::mem::take(&mut station.held);
        if dozing {
            self.row(at, Event::Doze { sta });
        } else {
            self.row(at, Event::Wake { sta });
            for mpdu in held {
                self.queue(at, sta, Out::Mpdu(mpdu), None);
            }
        }
    }

    /// A PS-Poll (9.3.1.5) from an associated station in power save: the access point sends
    /// its oldest held frame, with More Data when it holds more, or a Null frame when it holds
    /// none (11.2.3.6).
    fn poll(&mut self, at: Time, sta: Mac, aid: u16) {
        let bssid = self.config.bssid;
        let Some(station) = self.station(sta) else {
            return;
        };
        if station.aid != Some(aid) {
            return;
        }
        let (mpdu, more) = match station.held.pop_front() {
            Some(mpdu) => (mpdu, !station.held.is_empty()),
            None => {
                let null = frame::header(
                    frame::DATA,
                    frame::NULL,
                    frame::FROM_DS,
                    timing::duration_us(),
                    sta,
                    bssid,
                );
                (null.into(), false)
            }
        };
        self.queue(at, sta, Out::Mpdu(mpdu), Some(more));
    }
}
