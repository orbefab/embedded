//! A station's side of the subnet, written apart from the network's: the packets lwIP sends,
//! from their LLC header on, and readers that check the router's answers.

/// The router of the default subnet, and the first lease.
pub const ROUTER: [u8; 4] = [192, 168, 1, 1];
pub const LEASE: [u8; 4] = [192, 168, 1, 100];
pub const SECOND_LEASE: [u8; 4] = [192, 168, 1, 101];
/// The default network clock at time 0: 2026-01-01T00:00:00Z, in seconds from 1970.
pub const EPOCH: u64 = 1_767_225_600;
/// The Transmit Timestamp of the station's SNTP request, which the reply's Originate
/// Timestamp repeats.
pub const NTP_SENT: [u8; 8] = [0xc0, 0xff, 0xee, 0x00, 0x12, 0x34, 0x56, 0x78];
pub const XID: [u8; 4] = [0xab, 0xcd, 0x00, 0x01];

pub const DISCOVER: u8 = 1;
pub const OFFER: u8 = 2;
pub const REQUEST: u8 = 3;
pub const ACK: u8 = 5;
pub const NAK: u8 = 6;

const LLC_IPV4: [u8; 8] = [0xaa, 0xaa, 0x03, 0, 0, 0, 0x08, 0x00];
const LLC_ARP: [u8; 8] = [0xaa, 0xaa, 0x03, 0, 0, 0, 0x08, 0x06];

/// The Internet checksum (RFC 1071).
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = data
        .chunks(2)
        .map(|w| u32::from(w[0]) << 8 | u32::from(*w.get(1).unwrap_or(&0)))
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn ipv4(src: [u8; 4], dst: [u8; 4], protocol: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = LLC_IPV4.to_vec();
    let mut h = vec![0x45, 0];
    h.extend(((20 + payload.len()) as u16).to_be_bytes());
    h.extend([0x12, 0x34, 0, 0, 255, protocol, 0, 0]);
    h.extend(src);
    h.extend(dst);
    let sum = checksum(&h);
    h[10..12].copy_from_slice(&sum.to_be_bytes());
    p.extend(h);
    p.extend(payload);
    p
}

fn udp_sum(src: [u8; 4], dst: [u8; 4], segment: &[u8]) -> u16 {
    let mut pseudo = src.to_vec();
    pseudo.extend(dst);
    pseudo.extend([0, 17]);
    pseudo.extend((segment.len() as u16).to_be_bytes());
    pseudo.extend(segment);
    checksum(&pseudo)
}

pub fn udp(src: [u8; 4], sport: u16, dst: [u8; 4], dport: u16, data: &[u8]) -> Vec<u8> {
    let mut u = sport.to_be_bytes().to_vec();
    u.extend(dport.to_be_bytes());
    u.extend(((8 + data.len()) as u16).to_be_bytes());
    u.extend([0, 0]);
    u.extend(data);
    let sum = udp_sum(src, dst, &u);
    u[6..8].copy_from_slice(&sum.to_be_bytes());
    ipv4(src, dst, 17, &u)
}

/// A DHCP message of `kind` from the station (RFC 2131), from 0.0.0.0 to the broadcast address,
/// or from `ciaddr` to the router when it has one.
pub fn dhcp(
    kind: u8,
    ciaddr: [u8; 4],
    requested: Option<[u8; 4]>,
    server: Option<[u8; 4]>,
) -> Vec<u8> {
    let mut m = vec![1, 1, 6, 0];
    m.extend(XID);
    m.extend([0; 4]);
    m.extend(ciaddr);
    m.extend([0; 12]);
    m.extend(super::STA.0);
    m.extend([0; 10 + 192]);
    m.extend([99, 130, 83, 99, 53, 1, kind, 57, 2, 0x05, 0xdc]);
    if let Some(ip) = requested {
        m.extend([50, 4]);
        m.extend(ip);
    }
    if let Some(ip) = server {
        m.extend([54, 4]);
        m.extend(ip);
    }
    m.extend([55, 4, 1, 3, 28, 6, 255]);
    if ciaddr == [0; 4] {
        udp([0; 4], 68, [255; 4], 67, &m)
    } else {
        udp(ciaddr, 68, ROUTER, 67, &m)
    }
}

/// An ARP Request from the station (RFC 826). A sender address of 0.0.0.0 makes it a probe
/// (RFC 5227).
pub fn arp_request(spa: [u8; 4], tpa: [u8; 4]) -> Vec<u8> {
    let mut p = LLC_ARP.to_vec();
    p.extend([0, 1, 8, 0, 6, 4, 0, 1]);
    p.extend(super::STA.0);
    p.extend(spa);
    p.extend([0; 6]);
    p.extend(tpa);
    p
}

/// A standard query with RD set (RFC 1035) from the first lease to the router.
pub fn dns_query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = id.to_be_bytes().to_vec();
    q.extend([1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.split('.') {
        q.push(label.len() as u8);
        q.extend(label.as_bytes());
    }
    q.extend([0, (qtype >> 8) as u8, qtype as u8, 0, 1]);
    udp(LEASE, 0xc001, ROUTER, 53, &q)
}

/// An SNTP version 4 client request (RFC 4330) from the first lease to the router.
pub fn ntp_request(transmit: [u8; 8]) -> Vec<u8> {
    let mut r = vec![0x23, 0, 6, 0];
    r.extend([0; 36]);
    r.extend(transmit);
    udp(LEASE, 123, ROUTER, 123, &r)
}

/// An ICMP Echo Request (RFC 792) from the first lease to the router.
pub fn ping(id: u16, sequence: u16, data: &[u8]) -> Vec<u8> {
    let mut m = vec![8, 0, 0, 0];
    m.extend(id.to_be_bytes());
    m.extend(sequence.to_be_bytes());
    m.extend(data);
    let sum = checksum(&m);
    m[2..4].copy_from_slice(&sum.to_be_bytes());
    ipv4(LEASE, ROUTER, 1, &m)
}

/// An IPv4 datagram from the router, checked: its protocol, destination and payload.
pub fn read_ipv4(msdu: &[u8]) -> (u8, [u8; 4], Vec<u8>) {
    assert_eq!(msdu[..8], LLC_IPV4, "{msdu:02x?}");
    let h = &msdu[8..28];
    assert_eq!(h[0], 0x45);
    assert_eq!(checksum(h), 0);
    let total = usize::from(u16::from_be_bytes([h[2], h[3]]));
    assert_eq!(total, msdu.len() - 8);
    assert_eq!(h[12..16], ROUTER);
    (h[9], h[16..20].try_into().unwrap(), msdu[28..].to_vec())
}

/// A UDP datagram from the router, checked: its source port, destination, destination port
/// and data.
pub fn read_udp(msdu: &[u8]) -> (u16, [u8; 4], u16, Vec<u8>) {
    let (protocol, dst, u) = read_ipv4(msdu);
    assert_eq!(protocol, 17);
    assert_eq!(usize::from(u16::from_be_bytes([u[4], u[5]])), u.len());
    assert_eq!(udp_sum(ROUTER, dst, &u), 0);
    let port = |at: usize| u16::from_be_bytes([u[at], u[at + 1]]);
    (port(0), dst, port(2), u[8..].to_vec())
}

/// A DHCP reply's message type and its other options, in order.
pub fn dhcp_options(reply: &[u8]) -> (u8, Vec<(u8, Vec<u8>)>) {
    assert_eq!(reply[236..240], [99, 130, 83, 99]);
    let mut o = &reply[240..];
    let mut out = Vec::new();
    while let [code, len, rest @ ..] = o {
        if *code == 255 {
            break;
        }
        out.push((*code, rest[..usize::from(*len)].to_vec()));
        o = &rest[usize::from(*len)..];
    }
    let (code, kind) = out.remove(0);
    assert_eq!((code, kind.len()), (53, 1));
    (kind[0], out)
}
