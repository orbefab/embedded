//! The router's DHCP server (RFC 2131), with the options of RFC 2132 that lwIP's client reads.
//! A station's first DISCOVER takes the next free lease, and the station keeps it for good, so
//! it gets the same address each time it asks.

use std::net::Ipv4Addr;

use super::{Subnet, ipv4_at};
use crate::{Event, Mac};

const BOOTREQUEST: u8 = 1;
const BOOTREPLY: u8 = 2;
/// The magic cookie that starts the options (RFC 2131 3).
const COOKIE: [u8; 4] = [99, 130, 83, 99];
/// Where the options start: the fixed fields and the cookie (RFC 2131 2).
const OPTIONS: usize = 240;
/// The least a BOOTP message holds, its options padded (RFC 1542 2.1).
const MIN_LEN: usize = 300;
/// The BROADCAST bit of `flags` (RFC 2131 2).
const BROADCAST: u16 = 0x8000;

// Options (RFC 2132).
const PAD: u8 = 0;
const SUBNET_MASK: u8 = 1;
const ROUTER: u8 = 3;
const DNS_SERVER: u8 = 6;
const REQUESTED_ADDRESS: u8 = 50;
const LEASE_TIME: u8 = 51;
const MESSAGE_TYPE: u8 = 53;
const SERVER_ID: u8 = 54;
const END: u8 = 255;

// DHCP message types (RFC 2132 9.6).
const DISCOVER: u8 = 1;
const OFFER: u8 = 2;
const REQUEST: u8 = 3;
const ACK: u8 = 5;
const NAK: u8 = 6;

/// The options of a client's message that the server reads.
#[derive(Default)]
struct Options {
    kind: Option<u8>,
    requested: Option<Ipv4Addr>,
    server: Option<Ipv4Addr>,
}

/// The options after the cookie, or `None` when one runs past the message, or one the server
/// reads has the wrong length: 1 octet for the message type, and 4 for the Requested IP
/// Address and the Server Identifier (RFC 2132 9.6, 9.1, 9.7).
fn options(mut o: &[u8]) -> Option<Options> {
    let mut out = Options::default();
    loop {
        match o {
            [] | [END, ..] => return Some(out),
            [PAD, rest @ ..] => o = rest,
            [code, len, rest @ ..] => {
                let data = rest.get(..usize::from(*len))?;
                let address = || Some(Ipv4Addr::from(<[u8; 4]>::try_from(data).ok()?));
                match *code {
                    MESSAGE_TYPE => out.kind = Some(<[u8; 1]>::try_from(data).ok()?[0]),
                    REQUESTED_ADDRESS => out.requested = Some(address()?),
                    SERVER_ID => out.server = Some(address()?),
                    _ => {}
                }
                o = &rest[data.len()..];
            }
            [_] => return None,
        }
    }
}

/// The station's lease, taken now if it has none, or `None` when every lease is taken.
fn lease(subnet: &Subnet, leased: &mut Vec<Mac>, sta: Mac) -> Option<Ipv4Addr> {
    if let Some(ip) = recorded(subnet, leased, sta) {
        return Some(ip);
    }
    if leased.len() < usize::from(subnet.leases) {
        leased.push(sta);
        return recorded(subnet, leased, sta);
    }
    None
}

/// The station's lease, if it has one.
fn recorded(subnet: &Subnet, leased: &[Mac], sta: Mac) -> Option<Ipv4Addr> {
    let index = leased.iter().position(|&m| m == sta)?;
    Some(subnet.lease(index))
}

/// The answer to the DHCP message `message` from station `sta`: where it goes, the message,
/// and its row. A DISCOVER gets an OFFER of the station's lease. A REQUEST from a station with
/// a lease gets an ACK when it asks for that address, in its Requested IP Address option or
/// its `ciaddr`, and a NAK when it asks for another. A REQUEST from a station with no lease
/// on record gets no answer and takes no lease (RFC 2131 4.3.2). A REQUEST that names another
/// server, a message relayed by an agent, and every other message type are dropped.
pub(super) fn answer(
    subnet: &Subnet,
    leased: &mut Vec<Mac>,
    sta: Mac,
    message: &[u8],
) -> Option<(Ipv4Addr, Vec<u8>, Option<Event>)> {
    if message.len() < OPTIONS
        || message[0] != BOOTREQUEST
        || message[1..3] != [1, 6]
        || message[236..OPTIONS] != COOKIE
    {
        return None;
    }
    let ciaddr = ipv4_at(message, 12)?;
    let giaddr = ipv4_at(message, 24)?;
    let options = options(&message[OPTIONS..])?;
    if !giaddr.is_unspecified() {
        return None;
    }
    let (kind, yiaddr, event) = match options.kind? {
        DISCOVER => (OFFER, lease(subnet, leased, sta)?, None),
        REQUEST => {
            if options.server.is_some_and(|server| server != subnet.router) {
                return None;
            }
            let asked = options
                .requested
                .or((!ciaddr.is_unspecified()).then_some(ciaddr))?;
            match recorded(subnet, leased, sta)? {
                ip if ip == asked => (ACK, ip, Some(Event::DhcpAck { sta, ip })),
                _ => (
                    NAK,
                    Ipv4Addr::UNSPECIFIED,
                    Some(Event::DhcpNak { sta, ip: asked }),
                ),
            }
        }
        _ => return None,
    };
    // Table 3 of RFC 2131: `xid`, `flags` and `chaddr` are the client's, `ciaddr` is the
    // client's in an ACK, and a NAK carries only the message type and the server.
    let mut reply = vec![BOOTREPLY, 1, 6, 0];
    reply.extend(&message[4..8]);
    reply.extend([0, 0]);
    reply.extend(&message[10..12]);
    let ciaddr_out = if kind == ACK {
        ciaddr
    } else {
        Ipv4Addr::UNSPECIFIED
    };
    reply.extend(ciaddr_out.octets());
    reply.extend(yiaddr.octets());
    reply.extend([0; 8]);
    reply.extend(&message[28..44]);
    reply.extend([0; 64 + 128]);
    reply.extend(COOKIE);
    reply.extend([MESSAGE_TYPE, 1, kind]);
    reply.extend([SERVER_ID, 4]);
    reply.extend(subnet.router.octets());
    if kind != NAK {
        reply.extend([LEASE_TIME, 4]);
        reply.extend(subnet.lease_secs.to_be_bytes());
        for (code, address) in [
            (SUBNET_MASK, subnet.mask),
            (ROUTER, subnet.router),
            (DNS_SERVER, subnet.router),
        ] {
            reply.extend([code, 4]);
            reply.extend(address.octets());
        }
    }
    reply.push(END);
    reply.resize(reply.len().max(MIN_LEN), PAD);
    // Where it goes, with no relay agent (RFC 2131 4.1): a NAK is broadcast, an answer to a
    // client that has an address goes to it, and the rest go to the new address unless the
    // client asked for a broadcast. The frame itself goes to the station either way.
    let flags = u16::from_be_bytes([message[10], message[11]]);
    let dst = if kind == NAK || (ciaddr.is_unspecified() && flags & BROADCAST != 0) {
        Ipv4Addr::BROADCAST
    } else if !ciaddr.is_unspecified() {
        ciaddr
    } else {
        yiaddr
    };
    Some((dst, reply, event))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::tests::{STA, ask, udp_data, udp_msdu};
    use super::*;

    const XID: [u8; 4] = [0x3d, 0x1d, 0x00, 0x2a];

    /// A client's message of `kind` with the Requested IP Address and Server Identifier
    /// options when given, and `ciaddr` 0.
    pub(crate) fn request(
        kind: u8,
        requested: Option<Ipv4Addr>,
        server: Option<Ipv4Addr>,
    ) -> Vec<u8> {
        message(kind, Ipv4Addr::UNSPECIFIED, requested, server, 0)
    }

    fn message(
        kind: u8,
        ciaddr: Ipv4Addr,
        requested: Option<Ipv4Addr>,
        server: Option<Ipv4Addr>,
        flags: u16,
    ) -> Vec<u8> {
        let mut m = vec![1, 1, 6, 0];
        m.extend(XID);
        m.extend([0, 0]);
        m.extend(flags.to_be_bytes());
        m.extend(ciaddr.octets());
        m.extend([0; 12]);
        m.extend(STA.0);
        m.extend([0; 10 + 64 + 128]);
        m.extend(COOKIE);
        m.extend([MESSAGE_TYPE, 1, kind]);
        // A Parameter Request List and a Host Name, as lwIP and Arduino send, which the
        // server reads past.
        m.extend([55, 4, 1, 3, 28, 6, 12, 3, b'e', b's', b'p']);
        if let Some(ip) = requested {
            m.extend([REQUESTED_ADDRESS, 4]);
            m.extend(ip.octets());
        }
        if let Some(ip) = server {
            m.extend([SERVER_ID, 4]);
            m.extend(ip.octets());
        }
        m.push(END);
        m
    }

    fn send(
        subnet: &Subnet,
        leased: &mut Vec<Mac>,
        src: Ipv4Addr,
        dst: Ipv4Addr,
        message: &[u8],
    ) -> Option<(Ipv4Addr, Vec<u8>, Option<Event>)> {
        let answer = ask(subnet, leased, &udp_msdu(src, 68, dst, 67, message))?;
        let packet = &answer.msdu[8..];
        let dst = ipv4_at(packet, 16).unwrap();
        Some((dst, udp_data(&answer), answer.event))
    }

    /// The options of a reply as (code, data), in order, up to END.
    fn reply_options(reply: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut o = &reply[OPTIONS..];
        let mut out = Vec::new();
        while let [code, len, rest @ ..] = o {
            if *code == END {
                break;
            }
            out.push((*code, rest[..usize::from(*len)].to_vec()));
            o = &rest[usize::from(*len)..];
        }
        out
    }

    // RFC 2131 3.1: DISCOVER, OFFER, REQUEST naming the server and the offered address, ACK.
    #[test]
    fn a_station_gets_its_lease_in_four_messages() {
        let subnet = Subnet::default();
        let mut leased = Vec::new();
        let any = Ipv4Addr::UNSPECIFIED;
        let all = Ipv4Addr::BROADCAST;
        let lease = Ipv4Addr::new(192, 168, 1, 100);

        let (dst, offer, event) = send(
            &subnet,
            &mut leased,
            any,
            all,
            &request(DISCOVER, None, None),
        )
        .unwrap();
        assert_eq!((dst, event), (lease, None));
        assert_eq!(offer.len(), MIN_LEN);
        assert_eq!(offer[..4], [BOOTREPLY, 1, 6, 0]);
        assert_eq!(offer[4..8], XID);
        assert_eq!(ipv4_at(&offer, 12), Some(any));
        assert_eq!(ipv4_at(&offer, 16), Some(lease));
        assert_eq!(offer[28..34], STA.0);
        assert_eq!(offer[236..240], COOKIE);
        let options = vec![
            (MESSAGE_TYPE, vec![OFFER]),
            (SERVER_ID, vec![192, 168, 1, 1]),
            (LEASE_TIME, 86_400u32.to_be_bytes().to_vec()),
            (SUBNET_MASK, vec![255, 255, 255, 0]),
            (ROUTER, vec![192, 168, 1, 1]),
            (DNS_SERVER, vec![192, 168, 1, 1]),
        ];
        assert_eq!(reply_options(&offer), options);

        let selecting = request(REQUEST, Some(lease), Some(subnet.router));
        let (dst, ack, event) = send(&subnet, &mut leased, any, all, &selecting).unwrap();
        assert_eq!(
            (dst, event),
            (
                lease,
                Some(Event::DhcpAck {
                    sta: STA,
                    ip: lease
                })
            )
        );
        assert_eq!(ipv4_at(&ack, 16), Some(lease));
        let mut ack_options = options.clone();
        ack_options[0].1 = vec![ACK];
        assert_eq!(reply_options(&ack), ack_options);

        // Renewing: unicast from the lease, with ciaddr, which the ACK repeats and goes to.
        let renewing = message(REQUEST, lease, None, None, 0);
        let (dst, ack, _) = send(&subnet, &mut leased, lease, subnet.router, &renewing).unwrap();
        assert_eq!((dst, ipv4_at(&ack, 12)), (lease, Some(lease)));
        assert_eq!(leased, vec![STA]);

        // A client that asks for a broadcast gets one.
        let flagged = message(DISCOVER, any, None, None, BROADCAST);
        assert_eq!(
            send(&subnet, &mut leased, any, all, &flagged).unwrap().0,
            all
        );
    }

    // RFC 2131 4.3.2: a REQUEST for an address that is not the client's gets a NAK, broadcast,
    // with only the message type and the server. One for another server is not answered.
    #[test]
    fn a_wrong_address_gets_a_nak() {
        let subnet = Subnet::default();
        let mut leased = Vec::new();
        let (any, all) = (Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST);
        let wrong = Ipv4Addr::new(192, 168, 1, 150);
        send(
            &subnet,
            &mut leased,
            any,
            all,
            &request(DISCOVER, None, None),
        )
        .unwrap();
        let init_reboot = request(REQUEST, Some(wrong), None);
        let (dst, nak, event) = send(&subnet, &mut leased, any, all, &init_reboot).unwrap();
        assert_eq!(
            (dst, event),
            (
                all,
                Some(Event::DhcpNak {
                    sta: STA,
                    ip: wrong
                })
            )
        );
        assert_eq!(ipv4_at(&nak, 16), Some(any));
        assert_eq!(
            reply_options(&nak),
            vec![(MESSAGE_TYPE, vec![NAK]), (SERVER_ID, vec![192, 168, 1, 1])]
        );
        let elsewhere = request(REQUEST, Some(wrong), Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(send(&subnet, &mut leased, any, all, &elsewhere).is_none());
        // A REQUEST with neither an address option nor ciaddr asks for nothing.
        assert!(
            send(
                &subnet,
                &mut leased,
                any,
                all,
                &request(REQUEST, None, None)
            )
            .is_none()
        );
        // RELEASE, DECLINE and INFORM are not answered.
        for kind in [4, 7, 8] {
            assert!(send(&subnet, &mut leased, any, all, &request(kind, None, None)).is_none());
        }
    }

    #[test]
    fn each_station_keeps_its_own_address_until_the_pool_runs_out() {
        let subnet = Subnet {
            leases: 2,
            ..Subnet::default()
        };
        let mut leased = vec![Mac([2, 0, 0, 0, 0, 7])];
        assert_eq!(
            lease(&subnet, &mut leased, STA),
            Some(Ipv4Addr::new(192, 168, 1, 101))
        );
        assert_eq!(
            lease(&subnet, &mut leased, STA),
            Some(Ipv4Addr::new(192, 168, 1, 101))
        );
        assert_eq!(lease(&subnet, &mut leased, Mac([2, 0, 0, 0, 0, 8])), None);
        let discover = request(DISCOVER, None, None);
        let other = Mac([2, 0, 0, 0, 0, 9]);
        assert!(answer(&subnet, &mut leased, other, &discover).is_none());
        // A REQUEST from a station with no lease is not answered, full pool or not.
        let asked = request(REQUEST, Some(Ipv4Addr::new(192, 168, 1, 101)), None);
        assert!(answer(&subnet, &mut leased, other, &asked).is_none());
    }

    // RFC 2131 4.3.2: a server with no record of the client stays silent on its INIT-REBOOT
    // REQUEST, right address or wrong, and it takes no lease for it. Nor does a REQUEST that
    // names it with no OFFER before, or one from a client that says it has an address.
    #[test]
    fn a_station_with_no_lease_gets_no_answer_to_a_request() {
        let subnet = Subnet {
            leases: 1,
            ..Subnet::default()
        };
        let mut leased = Vec::new();
        for asked in [Ipv4Addr::new(192, 168, 1, 150), subnet.first_lease] {
            for server in [None, Some(subnet.router)] {
                let m = request(REQUEST, Some(asked), server);
                assert!(
                    answer(&subnet, &mut leased, STA, &m).is_none(),
                    "{asked} {server:?}"
                );
            }
            let renewing = message(REQUEST, asked, None, None, 0);
            assert!(answer(&subnet, &mut leased, STA, &renewing).is_none());
        }
        assert!(leased.is_empty());
        // So the one lease is still free for the next station.
        let other = Mac([2, 0, 0, 0, 0, 9]);
        let (_, offer, _) =
            answer(&subnet, &mut leased, other, &request(DISCOVER, None, None)).unwrap();
        assert_eq!(ipv4_at(&offer, 16), Some(subnet.first_lease));
    }

    // RFC 2132 9.6, 9.1 and 9.7: the message type is 1 octet, and the Requested IP Address and
    // the Server Identifier are 4. A message with one of another length is malformed, and is
    // dropped rather than read as if the option were absent.
    #[test]
    fn an_option_of_the_wrong_length_drops_the_message() {
        let subnet = Subnet::default();
        let mut leased = vec![STA];
        let lease = subnet.first_lease.octets();
        let with = |kind: u8, code: u8, data: &[u8]| {
            let mut m = request(kind, None, None);
            assert_eq!(m.pop(), Some(END));
            m.extend([code, data.len() as u8]);
            m.extend(data);
            m.push(END);
            m
        };
        let good = with(REQUEST, SERVER_ID, &subnet.router.octets());
        let mut good = good[..good.len() - 1].to_vec();
        good.extend([REQUESTED_ADDRESS, 4]);
        good.extend(lease);
        good.push(END);
        assert!(answer(&subnet, &mut leased, STA, &good).is_some());
        for data in [&[][..], &[192, 168, 1], &[192, 168, 1, 1, 0]] {
            for code in [SERVER_ID, REQUESTED_ADDRESS] {
                let mut m = with(REQUEST, code, data);
                m.pop();
                let other = if code == SERVER_ID {
                    REQUESTED_ADDRESS
                } else {
                    SERVER_ID
                };
                m.extend([other, 4]);
                m.extend(if other == SERVER_ID {
                    subnet.router.octets()
                } else {
                    lease
                });
                m.push(END);
                assert!(
                    answer(&subnet, &mut leased, STA, &m).is_none(),
                    "{code} {data:?}"
                );
            }
        }
        let mut long_type = request(DISCOVER, None, None);
        long_type[OPTIONS + 1] = 2;
        long_type.insert(OPTIONS + 3, 0);
        assert!(answer(&subnet, &mut leased, STA, &long_type).is_none());
    }

    #[test]
    fn options_that_run_past_the_message_drop_it() {
        assert!(options(&[MESSAGE_TYPE, 2, 1]).is_none());
        assert!(options(&[MESSAGE_TYPE]).is_none());
        let o = options(&[PAD, MESSAGE_TYPE, 1, 3, END, 0xff, 0xff]).unwrap();
        assert_eq!(o.kind, Some(3));
        // An option the server reads of the wrong length drops the message.
        assert!(options(&[SERVER_ID, 3, 1, 2, 3]).is_none());
        assert!(options(&[REQUESTED_ADDRESS, 5, 1, 2, 3, 4, 5]).is_none());
        assert!(options(&[MESSAGE_TYPE, 0]).is_none());
    }
}
