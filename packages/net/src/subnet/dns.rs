//! The router's DNS server (RFC 1035): authoritative for a fixed list of names, each an A record
//! of the router's address, and NXDOMAIN for every other name.

use std::fmt::Write;

use super::Subnet;
use crate::{Event, Mac};

const HEADER: usize = 12;
/// The QR bit, the Opcode, AA, RD and RA in the header's flags (RFC 1035 4.1.1).
const QR: u16 = 0x8000;
const OPCODE: u16 = 0x7800;
const AA: u16 = 0x0400;
const RD: u16 = 0x0100;
const RA: u16 = 0x0080;
const NXDOMAIN: u16 = 3;

const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;
/// The TTL of an answer, in seconds.
const TTL: u32 = 300;
/// The most octets a name takes on the wire (RFC 1035 2.3.4).
const MAX_NAME: usize = 255;

/// The answer to the DNS message `query` from station `sta`, and its row. A standard query of
/// one question in class IN gets an answer: for a listed name, an A record of the router's
/// address when it asks for type A, and no record for another type. Every other name gets
/// NXDOMAIN. Anything else is dropped.
pub(super) fn answer(subnet: &Subnet, sta: Mac, query: &[u8]) -> Option<(Vec<u8>, Event)> {
    let flags = u16::from_be_bytes(query.get(2..4)?.try_into().ok()?);
    if flags & (QR | OPCODE) != 0 || query.get(4..6)? != [0, 1] {
        return None;
    }
    // The question's name, uncompressed, as a query's is.
    let mut labels = Vec::new();
    let mut at = HEADER;
    loop {
        let len = usize::from(*query.get(at)?);
        if len & 0xc0 != 0 {
            return None;
        }
        if len == 0 {
            break;
        }
        labels.push(query.get(at + 1..at + 1 + len)?);
        at += 1 + len;
    }
    if at + 1 - HEADER > MAX_NAME {
        return None;
    }
    let question = query.get(HEADER..at + 5)?;
    let qtype = u16::from_be_bytes([question[question.len() - 4], question[question.len() - 3]]);
    let qclass = u16::from_be_bytes([question[question.len() - 2], question[question.len() - 1]]);
    if qclass != CLASS_IN {
        return None;
    }
    let listed = subnet.names.iter().any(|name| {
        let mut want = name.split('.');
        labels.iter().all(|label| {
            want.next()
                .is_some_and(|w| w.as_bytes().eq_ignore_ascii_case(label))
        }) && want.next().is_none()
    });
    let answers = u16::from(listed && qtype == TYPE_A);
    let rcode = if listed { 0 } else { NXDOMAIN };
    let mut reply = query[..2].to_vec();
    reply.extend((QR | AA | (flags & RD) | RA | rcode).to_be_bytes());
    reply.extend([0, 1]);
    reply.extend(answers.to_be_bytes());
    reply.extend([0, 0, 0, 0]);
    reply.extend(question);
    if answers == 1 {
        // The name as a pointer to the question's (RFC 1035 4.1.4).
        reply.extend([0xc0, HEADER as u8]);
        reply.extend(TYPE_A.to_be_bytes());
        reply.extend(CLASS_IN.to_be_bytes());
        reply.extend(TTL.to_be_bytes());
        reply.extend(4u16.to_be_bytes());
        reply.extend(subnet.router.octets());
    }
    let name = presentation(&labels);
    let event = if listed {
        Event::Dns {
            sta,
            name,
            answer: (answers == 1).then_some(subnet.router),
        }
    } else {
        Event::NxDomain { sta, name }
    };
    Some((reply, event))
}

/// A name as text (RFC 1035 5.1): its labels joined by dots, with a dot or a backslash in a
/// label escaped by a backslash, and any octet that is not a printable character as `\DDD`.
fn presentation(labels: &[&[u8]]) -> String {
    if labels.is_empty() {
        return ".".into();
    }
    let mut out = String::new();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        for &b in *label {
            match b {
                b'.' | b'\\' => {
                    out.push('\\');
                    out.push(char::from(b));
                }
                0x21..=0x7e => out.push(char::from(b)),
                _ => {
                    let _ = write!(out, "\\{b:03}");
                }
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use std::net::Ipv4Addr;

    use super::super::tests::STA;
    use super::*;

    /// A standard query with RD set for `name` of type `qtype`, class IN.
    pub(crate) fn query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
        let mut q = id.to_be_bytes().to_vec();
        q.extend([0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
        for label in name.split('.') {
            q.push(label.len() as u8);
            q.extend(label.as_bytes());
        }
        q.push(0);
        q.extend(qtype.to_be_bytes());
        q.extend(CLASS_IN.to_be_bytes());
        q
    }

    // RFC 1035 3.1 and 4.1.4: "F.ISI.ARPA" on the wire is 1 F 3 ISI 4 ARPA 0.
    #[test]
    fn names_are_read_as_rfc_1035_writes_them() {
        let subnet = Subnet {
            names: vec!["f.isi.arpa".into()],
            ..Subnet::default()
        };
        let q = query(0x1234, "F.ISI.ARPA", TYPE_A);
        assert_eq!(q[12..24], *b"\x01F\x03ISI\x04ARPA\x00");
        let (reply, event) = answer(&subnet, STA, &q).unwrap();
        assert_eq!(
            event,
            Event::Dns {
                sta: STA,
                name: "F.ISI.ARPA".into(),
                answer: Some(Ipv4Addr::new(192, 168, 1, 1)),
            }
        );
        // The header: ID, QR AA RD RA and NOERROR, one question and one answer.
        assert_eq!(
            reply[..12],
            [0x12, 0x34, 0x85, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]
        );
        assert_eq!(reply[12..q.len()], q[12..]);
        assert_eq!(
            reply[q.len()..],
            [0xc0, 12, 0, 1, 0, 1, 0, 0, 0x01, 0x2c, 0, 4, 192, 168, 1, 1]
        );
    }

    #[test]
    fn an_unlisted_name_is_nxdomain_and_another_type_has_no_record() {
        let subnet = Subnet::default();
        let (reply, event) = answer(&subnet, STA, &query(7, "pool.ntp.orq", TYPE_A)).unwrap();
        assert_eq!(reply[2..8], [0x85, 0x83, 0, 1, 0, 0]);
        assert_eq!(
            event,
            Event::NxDomain {
                sta: STA,
                name: "pool.ntp.orq".into()
            }
        );
        // A name that is a suffix or a prefix of a listed one is not listed.
        for name in ["ntp.org", "pool.ntp.org.uk", "x.pool.ntp.org"] {
            let (reply, _) = answer(&subnet, STA, &query(7, name, TYPE_A)).unwrap();
            assert_eq!(reply[3] & 0xf, 3, "{name}");
        }
        // AAAA of a listed name: NOERROR with no answer.
        let (reply, event) = answer(&subnet, STA, &query(7, "Time.Google.com", 28)).unwrap();
        assert_eq!(reply[2..8], [0x85, 0x80, 0, 1, 0, 0]);
        assert_eq!(
            event,
            Event::Dns {
                sta: STA,
                name: "Time.Google.com".into(),
                answer: None
            }
        );
    }

    #[test]
    fn a_label_with_a_dot_is_not_two_labels() {
        let subnet = Subnet::default();
        let mut q = query(7, "x.org", TYPE_A);
        // "pool.ntp" as one label, then "org".
        q.splice(12..14, *b"\x08pool.ntp");
        assert_eq!(q[12..26], *b"\x08pool.ntp\x03org\x00");
        let (_, event) = answer(&subnet, STA, &q).unwrap();
        assert_eq!(
            event,
            Event::NxDomain {
                sta: STA,
                name: r"pool\.ntp.org".into()
            }
        );
        assert_eq!(presentation(&[b"a\x00\"\\", b"b c"]), r#"a\000"\\.b\032c"#);
        assert_eq!(presentation(&[]), ".");
    }

    #[test]
    fn a_response_a_compressed_name_or_another_opcode_is_dropped() {
        let subnet = Subnet::default();
        let good = query(7, "pool.ntp.org", TYPE_A);
        for (at, value) in [(2, 0x81), (2, 0x09), (5, 2), (12, 0xc0)] {
            let mut bad = good.clone();
            bad[at] = value;
            assert!(answer(&subnet, STA, &bad).is_none(), "{at} {value}");
        }
        let mut chaos = good.clone();
        *chaos.last_mut().unwrap() = 3;
        assert!(answer(&subnet, STA, &chaos).is_none());
        // A name longer than 255 octets.
        let long = vec!["a".repeat(63); 4].join(".");
        assert!(answer(&subnet, STA, &query(7, &long, TYPE_A)).is_none());
        let fits = [
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(61),
        ]
        .join(".");
        assert!(answer(&subnet, STA, &query(7, &fits, TYPE_A)).is_some());
    }
}
