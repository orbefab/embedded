//! The subnet behind the access point. A station's MSDUs come in after IEEE 802.2 LLC/SNAP
//! (RFC 1042), and the router answers ARP for its address (RFC 826), leases addresses over
//! DHCP (RFC 2131), answers pings (RFC 792), and serves DNS (RFC 1035) and SNTP (RFC 4330).
//! Every answer is made at the time its request lands. Anything else is dropped.

mod dhcp;
mod dns;
mod sntp;

use std::net::Ipv4Addr;

use orbe_model::Time;

use crate::rsn::LLC_SNAP;
use crate::{Event, Mac};

const IPV4: u16 = 0x0800;
const ARP: u16 = 0x0806;

const ICMP: u8 = 1;
const UDP: u8 = 17;

const DHCP_SERVER: u16 = 67;
const DHCP_CLIENT: u16 = 68;
const DNS: u16 = 53;
const NTP: u16 = 123;

/// The subnet behind the access point, which every copy of a network shares. Its router is the
/// gateway, the DHCP server, the DNS server and the SNTP server, at the access point's BSSID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subnet {
    pub router: Ipv4Addr,
    pub mask: Ipv4Addr,
    /// The first address DHCP leases. Each station that asks takes the next one, and keeps it.
    pub first_lease: Ipv4Addr,
    /// How many addresses DHCP leases.
    pub leases: u16,
    /// The lease time DHCP gives, in seconds.
    pub lease_secs: u32,
    /// The names DNS answers with the router's address. Every other name is NXDOMAIN. Case
    /// does not matter.
    pub names: Vec<String>,
    /// The seconds from 1970-01-01T00:00:00Z the network's clock reads at time 0.
    pub epoch: u64,
}

impl Default for Subnet {
    /// `192.168.1.0/24` with the router at `.1`, 100 leases of a day from `.100`, the public
    /// time server names, and a clock that reads 2026-01-01T00:00:00Z at time 0.
    fn default() -> Subnet {
        let mut names = vec!["pool.ntp.org".to_string()];
        for zone in [
            "",
            "africa.",
            "asia.",
            "europe.",
            "north-america.",
            "oceania.",
            "south-america.",
        ] {
            if !zone.is_empty() {
                names.push(format!("{zone}pool.ntp.org"));
            }
            names.extend((0..4).map(|n| format!("{n}.{zone}pool.ntp.org")));
        }
        for name in [
            "time.nist.gov",
            "time.google.com",
            "time.cloudflare.com",
            "time.windows.com",
            "time.apple.com",
        ] {
            names.push(name.into());
        }
        Subnet {
            router: Ipv4Addr::new(192, 168, 1, 1),
            mask: Ipv4Addr::new(255, 255, 255, 0),
            first_lease: Ipv4Addr::new(192, 168, 1, 100),
            leases: 100,
            lease_secs: 86_400,
            names,
            epoch: 1_767_225_600,
        }
    }
}

impl Subnet {
    /// Whether its mask is contiguous, and its leases are host addresses of its subnet apart
    /// from the router's.
    pub(crate) fn is_valid(&self) -> bool {
        let mask = u32::from(self.mask);
        let net = u32::from(self.router) & mask;
        let host = |a: u32| a & mask == net && a != net && a != net | !mask;
        let first = u32::from(self.first_lease);
        let last = first.checked_add(u32::from(self.leases)).map(|end| end - 1);
        mask.leading_ones() + mask.trailing_zeros() == 32
            && self.leases > 0
            && host(u32::from(self.router))
            && last.is_some_and(|last| {
                host(first) && host(last) && !(first..=last).contains(&u32::from(self.router))
            })
    }

    /// The address of lease `index`.
    fn lease(&self, index: usize) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.first_lease) + index as u32)
    }

    fn broadcast(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.router) | !u32::from(self.mask))
    }
}

/// What the router answers an MSDU with: the MSDU it sends the station, from its LLC header
/// on, and the row it makes.
pub(crate) struct Answer {
    pub(crate) msdu: Vec<u8>,
    pub(crate) event: Option<Event>,
}

/// The router's answer to the MSDU `msdu`, from its LLC header on, that station `sta` sent at
/// `at` to `da`, under `subnet`, its router at `router`. `leased` holds the station each lease
/// went to, in order.
pub(crate) fn answer(
    subnet: &Subnet,
    leased: &mut Vec<Mac>,
    router: Mac,
    at: Time,
    sta: Mac,
    da: Mac,
    msdu: &[u8],
) -> Option<Answer> {
    if da != router && da != Mac::BROADCAST {
        return None;
    }
    let (ethertype, body) = msdu.strip_prefix(&LLC_SNAP)?.split_first_chunk::<2>()?;
    let (ethertype, packet, event) = match u16::from_be_bytes(*ethertype) {
        ARP => (ARP, arp(subnet, router, body)?, None),
        IPV4 => {
            let (packet, event) = ipv4(subnet, leased, at, sta, body)?;
            (IPV4, packet, event)
        }
        _ => return None,
    };
    let mut msdu = LLC_SNAP.to_vec();
    msdu.extend(ethertype.to_be_bytes());
    msdu.extend(packet);
    Some(Answer { msdu, event })
}

/// An ARP Request for the router's address gets a Reply from the router. A probe, whose sender
/// address is 0.0.0.0 (RFC 5227 2.1.1), and an announcement of the router's own address get
/// none, so a station keeps the address it probes.
fn arp(subnet: &Subnet, router: Mac, packet: &[u8]) -> Option<Vec<u8>> {
    // Ethernet and IPv4 addresses, and the Request opcode.
    let head = packet.get(..8)?;
    let sha = packet.get(8..14)?;
    let spa = ipv4_at(packet, 14)?;
    let tpa = ipv4_at(packet, 24)?;
    if head != [0, 1, 8, 0, 6, 4, 0, 1]
        || tpa != subnet.router
        || spa.is_unspecified()
        || spa == subnet.router
    {
        return None;
    }
    let mut reply = vec![0, 1, 8, 0, 6, 4, 0, 2];
    reply.extend(router.0);
    reply.extend(subnet.router.octets());
    reply.extend(sha);
    reply.extend(spa.octets());
    Some(reply)
}

/// An IPv4 datagram of one fragment, with a good header checksum (RFC 791).
struct Datagram<'a> {
    src: Ipv4Addr,
    dst: Ipv4Addr,
    protocol: u8,
    payload: &'a [u8],
}

impl Datagram<'_> {
    fn parse(packet: &[u8]) -> Option<Datagram<'_>> {
        let first = *packet.first()?;
        let ihl = usize::from(first & 0xf) * 4;
        let total = usize::from(u16_at(packet, 2)?);
        let fragment = u16_at(packet, 6)? & 0x3fff;
        if first >> 4 != 4 || ihl < 20 || total < ihl || total > packet.len() || fragment != 0 {
            return None;
        }
        if checksum(&[&packet[..ihl]]) != 0 {
            return None;
        }
        Some(Datagram {
            src: ipv4_at(packet, 12)?,
            dst: ipv4_at(packet, 16)?,
            protocol: packet[9],
            payload: &packet[ihl..total],
        })
    }
}

/// An IPv4 datagram from the router: no options, Don't Fragment with Identification 0 (RFC
/// 6864 4.1), and a TTL of 64.
fn datagram(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x45, 0];
    out.extend(((20 + payload.len()) as u16).to_be_bytes());
    out.extend([0, 0, 0x40, 0, 64, protocol, 0, 0]);
    out.extend(src.octets());
    out.extend(dst.octets());
    let sum = checksum(&[&out]);
    out[10..12].copy_from_slice(&sum.to_be_bytes());
    out.extend(payload);
    out
}

/// A datagram from the station to the router's address, or to a broadcast address for DHCP.
fn ipv4(
    subnet: &Subnet,
    leased: &mut Vec<Mac>,
    at: Time,
    sta: Mac,
    packet: &[u8],
) -> Option<(Vec<u8>, Option<Event>)> {
    let ip = Datagram::parse(packet)?;
    let to_router = ip.dst == subnet.router;
    let broadcast = ip.dst == Ipv4Addr::BROADCAST || ip.dst == subnet.broadcast();
    match ip.protocol {
        ICMP if to_router => {
            let reply = echo(ip.payload)?;
            Some((datagram(subnet.router, ip.src, ICMP, &reply), None))
        }
        UDP if to_router || broadcast => {
            let (sport, dport, data) = udp(&ip)?;
            let (dst, port, reply, event) = match dport {
                DHCP_SERVER if sport == DHCP_CLIENT => {
                    let (dst, reply, event) = dhcp::answer(subnet, leased, sta, data)?;
                    (dst, DHCP_CLIENT, reply, event)
                }
                DNS | NTP if to_router && !ip.src.is_unspecified() && !ip.src.is_broadcast() => {
                    let (reply, event) = if dport == DNS {
                        dns::answer(subnet, sta, data)?
                    } else {
                        sntp::answer(subnet.epoch, at, sta, data)?
                    };
                    (ip.src, sport, reply, Some(event))
                }
                _ => return None,
            };
            let segment = udp_segment(subnet.router, dport, dst, port, &reply);
            Some((datagram(subnet.router, dst, UDP, &segment), event))
        }
        _ => None,
    }
}

/// An ICMP Echo Request with a good checksum gets its Echo Reply, with the same identifier,
/// sequence number and data (RFC 792).
fn echo(message: &[u8]) -> Option<Vec<u8>> {
    if message.get(..2)? != [8, 0] || message.len() < 8 || checksum(&[message]) != 0 {
        return None;
    }
    let mut reply = message.to_vec();
    reply[0] = 0;
    reply[2..4].fill(0);
    let sum = checksum(&[&reply]);
    reply[2..4].copy_from_slice(&sum.to_be_bytes());
    Some(reply)
}

/// The ports and the data of a UDP datagram, whose checksum is 0 or good (RFC 768).
fn udp<'a>(ip: &Datagram<'a>) -> Option<(u16, u16, &'a [u8])> {
    let segment = ip.payload;
    let len = usize::from(u16_at(segment, 4)?);
    let sum = u16_at(segment, 6)?;
    if len < 8 || len > segment.len() {
        return None;
    }
    let segment = &segment[..len];
    if sum != 0 && checksum(&[&pseudo_header(ip.src, ip.dst, len), segment]) != 0 {
        return None;
    }
    Some((u16_at(segment, 0)?, u16_at(segment, 2)?, &segment[8..]))
}

fn udp_segment(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, data: &[u8]) -> Vec<u8> {
    let len = 8 + data.len();
    let mut out = Vec::with_capacity(len);
    out.extend(sport.to_be_bytes());
    out.extend(dport.to_be_bytes());
    out.extend((len as u16).to_be_bytes());
    out.extend([0, 0]);
    out.extend(data);
    // A computed 0 goes as all ones, since 0 means no checksum.
    let sum = match checksum(&[&pseudo_header(src, dst, len), &out]) {
        0 => 0xffff,
        sum => sum,
    };
    out[6..8].copy_from_slice(&sum.to_be_bytes());
    out
}

/// The pseudo-header a UDP checksum covers.
fn pseudo_header(src: Ipv4Addr, dst: Ipv4Addr, len: usize) -> [u8; 12] {
    let mut out = [0; 12];
    out[..4].copy_from_slice(&src.octets());
    out[4..8].copy_from_slice(&dst.octets());
    out[9] = UDP;
    out[10..].copy_from_slice(&(len as u16).to_be_bytes());
    out
}

/// The Internet checksum of the parts, taken in order as one run of octets (RFC 1071): the
/// one's complement of the one's complement sum of its 16-bit words. A run with its checksum
/// in place sums to 0.
fn checksum(parts: &[&[u8]]) -> u16 {
    let mut sum: u32 = 0;
    let mut odd: Option<u8> = None;
    for &b in parts.iter().flat_map(|part| part.iter()) {
        match odd.take() {
            Some(high) => sum += u32::from(u16::from_be_bytes([high, b])),
            None => odd = Some(b),
        }
    }
    if let Some(high) = odd {
        sum += u32::from(high) << 8;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn ipv4_at(bytes: &[u8], at: usize) -> Option<Ipv4Addr> {
    Some(Ipv4Addr::from(
        <[u8; 4]>::try_from(bytes.get(at..at + 4)?).ok()?,
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const STA: Mac = Mac([0x02, 0, 0, 0, 0, 1]);
    pub(crate) const ROUTER: Mac = Mac([0x02, 0, 0, 0, 0, 0xfe]);

    // RFC 1071 3: the words 0001 f203 f4f5 f6f7 sum to ddf2, so their checksum is 220d.
    #[test]
    fn the_checksum_is_rfc_1071_s() {
        let words = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(checksum(&[&words]), 0x220d);
        assert_eq!(checksum(&[&words[..3], &words[3..]]), 0x220d);
        let mut with = words.to_vec();
        with.extend([0x22, 0x0d]);
        assert_eq!(checksum(&[&with]), 0);
        // An odd octet is padded with a zero.
        assert_eq!(checksum(&[&[0x01]]), !0x0100);
    }

    #[test]
    fn the_default_subnet_is_valid_and_a_bad_one_is_not() {
        let subnet = Subnet::default();
        assert!(subnet.is_valid());
        assert_eq!(subnet.lease(99), Ipv4Addr::new(192, 168, 1, 199));
        assert_eq!(subnet.broadcast(), Ipv4Addr::new(192, 168, 1, 255));
        assert!(subnet.names.iter().any(|n| n == "2.europe.pool.ntp.org"));
        for bad in [
            Subnet {
                leases: 156,
                ..Subnet::default()
            },
            Subnet {
                first_lease: Ipv4Addr::new(192, 168, 1, 1),
                ..Subnet::default()
            },
            Subnet {
                mask: Ipv4Addr::new(255, 0, 255, 0),
                ..Subnet::default()
            },
            Subnet {
                leases: 0,
                ..Subnet::default()
            },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
    }

    /// An MSDU of `ethertype` and `packet`.
    pub(crate) fn msdu(ethertype: u16, packet: &[u8]) -> Vec<u8> {
        let mut out = LLC_SNAP.to_vec();
        out.extend(ethertype.to_be_bytes());
        out.extend(packet);
        out
    }

    pub(crate) fn udp_msdu(
        src: Ipv4Addr,
        sport: u16,
        dst: Ipv4Addr,
        dport: u16,
        data: &[u8],
    ) -> Vec<u8> {
        let segment = udp_segment(src, sport, dst, dport, data);
        msdu(IPV4, &datagram(src, dst, UDP, &segment))
    }

    /// The UDP data of an answer, after checking its headers and checksums.
    pub(crate) fn udp_data(answer: &Answer) -> Vec<u8> {
        let packet = answer.msdu.strip_prefix(&msdu(IPV4, &[])[..]).unwrap();
        let ip = Datagram::parse(packet).unwrap();
        assert_eq!(ip.protocol, UDP);
        udp(&ip).unwrap().2.to_vec()
    }

    pub(crate) fn ask(subnet: &Subnet, leased: &mut Vec<Mac>, msdu: &[u8]) -> Option<Answer> {
        answer(subnet, leased, ROUTER, Time::ZERO, STA, ROUTER, msdu)
    }

    fn arp_request(spa: [u8; 4], tpa: [u8; 4]) -> Vec<u8> {
        let mut p = vec![0, 1, 8, 0, 6, 4, 0, 1];
        p.extend(STA.0);
        p.extend(spa);
        p.extend([0; 6]);
        p.extend(tpa);
        msdu(ARP, &p)
    }

    // RFC 826: the Reply swaps the sender and target, with the router's hardware address.
    // RFC 5227 2.1.1: a probe has sender address 0.0.0.0 and gets no answer.
    #[test]
    fn arp_answers_for_the_router_only() {
        let subnet = Subnet::default();
        let reply = ask(
            &subnet,
            &mut vec![],
            &arp_request([192, 168, 1, 100], [192, 168, 1, 1]),
        )
        .unwrap();
        let mut want = vec![0, 1, 8, 0, 6, 4, 0, 2];
        want.extend(ROUTER.0);
        want.extend([192, 168, 1, 1]);
        want.extend(STA.0);
        want.extend([192, 168, 1, 100]);
        assert_eq!(reply.msdu, msdu(ARP, &want));
        for (spa, tpa) in [
            ([0, 0, 0, 0], [192, 168, 1, 100]),
            ([0, 0, 0, 0], [192, 168, 1, 1]),
            ([192, 168, 1, 100], [192, 168, 1, 100]),
            ([192, 168, 1, 100], [192, 168, 1, 7]),
            ([192, 168, 1, 1], [192, 168, 1, 1]),
        ] {
            assert!(ask(&subnet, &mut vec![], &arp_request(spa, tpa)).is_none());
        }
    }

    #[test]
    fn a_ping_to_the_router_gets_its_echo() {
        let subnet = Subnet::default();
        let sta = Ipv4Addr::new(192, 168, 1, 100);
        let mut request = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1];
        request.extend(b"abcdefgh");
        let sum = checksum(&[&request]);
        request[2..4].copy_from_slice(&sum.to_be_bytes());
        let ping = msdu(IPV4, &datagram(sta, subnet.router, ICMP, &request));
        let reply = ask(&subnet, &mut vec![], &ping).unwrap();
        let ip = Datagram::parse(reply.msdu.strip_prefix(&msdu(IPV4, &[])[..]).unwrap()).unwrap();
        assert_eq!((ip.src, ip.dst, ip.protocol), (subnet.router, sta, ICMP));
        assert_eq!(checksum(&[ip.payload]), 0);
        assert_eq!(ip.payload[..2], [0, 0]);
        assert_eq!(ip.payload[4..], request[4..]);

        // A bad checksum, or a ping to another address, is dropped.
        let mut bad = request.clone();
        bad[9] ^= 1;
        assert!(
            ask(
                &subnet,
                &mut vec![],
                &msdu(IPV4, &datagram(sta, subnet.router, ICMP, &bad))
            )
            .is_none()
        );
        let other = Ipv4Addr::new(192, 168, 1, 7);
        assert!(
            ask(
                &subnet,
                &mut vec![],
                &msdu(IPV4, &datagram(sta, other, ICMP, &request))
            )
            .is_none()
        );
    }

    #[test]
    fn a_frame_to_another_address_or_a_fragment_is_dropped() {
        let subnet = Subnet::default();
        let sta = Ipv4Addr::new(192, 168, 1, 100);
        let query = dns::tests::query(7, "pool.ntp.org", 1);
        let good = udp_msdu(sta, 5000, subnet.router, DNS, &query);
        assert!(ask(&subnet, &mut vec![], &good).is_some());
        let other = Mac([0x02, 0, 0, 0, 0, 9]);
        assert!(answer(&subnet, &mut vec![], ROUTER, Time::ZERO, STA, other, &good).is_none());
        // More Fragments.
        let mut fragment = good.clone();
        fragment[8 + 6] = 0x20;
        fragment[8 + 10..8 + 12].fill(0);
        let sum = checksum(&[&fragment[8..8 + 20]]);
        fragment[8 + 10..8 + 12].copy_from_slice(&sum.to_be_bytes());
        assert!(ask(&subnet, &mut vec![], &fragment).is_none());
        // A bad UDP checksum.
        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(ask(&subnet, &mut vec![], &bad).is_none());
        // No UDP checksum at all is fine.
        let mut none = good;
        none[8 + 20 + 6..8 + 20 + 8].fill(0);
        assert!(ask(&subnet, &mut vec![], &none).is_some());
    }

    /// A seeded xorshift.
    pub(crate) struct Rng(pub(crate) u64);

    impl Rng {
        pub(crate) fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        pub(crate) fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    // Every cut of every request, and requests corrupted at random, without a panic.
    #[test]
    fn no_msdu_panics_the_router() {
        let subnet = Subnet::default();
        let sta = Ipv4Addr::new(192, 168, 1, 100);
        let mut ping = vec![8, 0, 0xf7, 0xfe, 0, 0, 0, 1];
        ping.extend([0; 4]);
        let mut requests = vec![
            arp_request([192, 168, 1, 100], [192, 168, 1, 1]),
            msdu(IPV4, &datagram(sta, subnet.router, ICMP, &ping)),
            udp_msdu(
                sta,
                5000,
                subnet.router,
                DNS,
                &dns::tests::query(7, "pool.ntp.org", 1),
            ),
            udp_msdu(sta, 123, subnet.router, NTP, &sntp::tests::request()),
        ];
        for kind in [1, 3] {
            let request = dhcp::tests::request(kind, Some(subnet.first_lease), Some(subnet.router));
            requests.push(udp_msdu(
                Ipv4Addr::UNSPECIFIED,
                68,
                Ipv4Addr::BROADCAST,
                67,
                &request,
            ));
        }
        let mut leased = Vec::new();
        for request in &requests {
            assert!(
                ask(&subnet, &mut leased, request).is_some(),
                "{request:02x?}"
            );
            for len in 0..request.len() {
                let _ = ask(&subnet, &mut leased, &request[..len]);
            }
        }
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..50_000 {
            let mut msdu = requests[rng.below(requests.len() as u64) as usize].clone();
            for _ in 0..1 + rng.below(4) {
                let k = rng.below(msdu.len() as u64) as usize;
                msdu[k] ^= 1 << rng.below(8);
            }
            if rng.below(3) == 0 {
                msdu.truncate(rng.below(msdu.len() as u64 + 1) as usize);
            }
            let _ = ask(&subnet, &mut leased, &msdu);
            // The same past the checksums, so the parsers see the corruption.
            if msdu.len() > 8 + 20 && msdu[6..8] == [8, 0] {
                let ihl = usize::from(msdu[8] & 0xf) * 4;
                if ihl >= 20 && msdu.len() >= 8 + ihl {
                    msdu[8 + 10..8 + 12].fill(0);
                    let sum = checksum(&[&msdu[8..8 + ihl]]);
                    msdu[8 + 10..8 + 12].copy_from_slice(&sum.to_be_bytes());
                    if msdu[8 + 9] == UDP && msdu.len() >= 8 + ihl + 8 {
                        msdu[8 + ihl + 6..8 + ihl + 8].fill(0);
                    }
                }
                let _ = ask(&subnet, &mut leased, &msdu);
            }
        }
    }
}
