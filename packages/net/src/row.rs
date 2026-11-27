//! What the network did, one row of `net.jsonl` each.

use std::fmt::Write;
use std::net::Ipv4Addr;

use orbe_model::Time;

use crate::Mac;

/// One thing the network did, at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub at: Time,
    pub event: Event,
}

/// Which side ended an authentication or an association.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    Ap,
    Station,
}

/// What happened. `sta` is a station's address, or `ff:ff:ff:ff:ff:ff` for a frame to every
/// station. Status and reason codes are IEEE 802.11-2020's (Tables 9-50 and 9-49).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The access point came up and beacons.
    Up,
    /// The access point went down. It forgets every station and stops beaconing.
    Down,
    /// It answered an Authentication frame with `status`.
    Auth {
        sta: Mac,
        status: u16,
    },
    /// It answered an Association or Reassociation Request with `status`, and `aid` when it
    /// associated the station.
    Assoc {
        sta: Mac,
        status: u16,
        aid: Option<u16>,
    },
    Deauth {
        sta: Mac,
        reason: u16,
        by: By,
    },
    Disassoc {
        sta: Mac,
        reason: u16,
        by: By,
    },
    /// A station finished the 4-way handshake and holds its keys.
    Handshake {
        sta: Mac,
    },
    /// It declined a station's ADDBA Request for traffic identifier `tid`.
    AddbaDeclined {
        sta: Mac,
        tid: u8,
    },
    /// A station went into power save. The access point holds its frames.
    Doze {
        sta: Mac,
    },
    /// A station left power save.
    Wake {
        sta: Mac,
    },
    /// No station acknowledged a frame the access point sent it.
    Lost {
        sta: Mac,
    },
    /// The access point had no room for a frame to the station, and dropped it.
    Dropped {
        sta: Mac,
    },
    /// The router's DHCP server acknowledged the station's lease of `ip`.
    DhcpAck {
        sta: Mac,
        ip: Ipv4Addr,
    },
    /// The router's DHCP server refused the station's request for `ip`.
    DhcpNak {
        sta: Mac,
        ip: Ipv4Addr,
    },
    /// The router's DNS server answered a query for a name it serves, with `answer` when the
    /// query asked for its address. `name` is in the text form of RFC 1035 5.1.
    Dns {
        sta: Mac,
        name: String,
        answer: Option<Ipv4Addr>,
    },
    /// The router's DNS server answered a query for a name it does not serve with NXDOMAIN.
    NxDomain {
        sta: Mac,
        name: String,
    },
    /// The router's SNTP server sent the station the time, `utc_ms` milliseconds from
    /// 1970-01-01T00:00:00Z, rounded down.
    Sntp {
        sta: Mac,
        utc_ms: u64,
    },
}

impl Row {
    /// The row as one line of JSON, without the newline: `t_ps`, `event`, then the event's
    /// fields in a fixed order.
    pub fn json(&self) -> String {
        let mut out = format!("{{\"t_ps\":{},\"event\":", self.at.as_ps());
        let by = |by: By| match by {
            By::Ap => "ap",
            By::Station => "station",
        };
        let _ = match &self.event {
            Event::Up => write!(out, "\"ap_up\""),
            Event::Down => write!(out, "\"ap_down\""),
            Event::Auth { sta, status } => {
                write!(out, "\"auth\",\"sta\":\"{sta}\",\"status\":{status}")
            }
            Event::Assoc { sta, status, aid } => write!(
                out,
                "\"assoc\",\"sta\":\"{sta}\",\"status\":{status},\"aid\":"
            )
            .and_then(|()| match aid {
                Some(aid) => write!(out, "{aid}"),
                None => write!(out, "null"),
            }),
            Event::Deauth {
                sta,
                reason,
                by: who,
            } => write!(
                out,
                "\"deauth\",\"sta\":\"{sta}\",\"reason\":{reason},\"by\":\"{}\"",
                by(*who)
            ),
            Event::Disassoc {
                sta,
                reason,
                by: who,
            } => write!(
                out,
                "\"disassoc\",\"sta\":\"{sta}\",\"reason\":{reason},\"by\":\"{}\"",
                by(*who)
            ),
            Event::AddbaDeclined { sta, tid } => {
                write!(out, "\"addba_declined\",\"sta\":\"{sta}\",\"tid\":{tid}")
            }
            Event::Handshake { sta } => write!(out, "\"handshake\",\"sta\":\"{sta}\""),
            Event::Doze { sta } => write!(out, "\"doze\",\"sta\":\"{sta}\""),
            Event::Wake { sta } => write!(out, "\"wake\",\"sta\":\"{sta}\""),
            Event::Lost { sta } => write!(out, "\"lost\",\"sta\":\"{sta}\""),
            Event::Dropped { sta } => write!(out, "\"dropped\",\"sta\":\"{sta}\""),
            Event::DhcpAck { sta, ip } => {
                write!(out, "\"dhcp_ack\",\"sta\":\"{sta}\",\"ip\":\"{ip}\"")
            }
            Event::DhcpNak { sta, ip } => {
                write!(out, "\"dhcp_nak\",\"sta\":\"{sta}\",\"ip\":\"{ip}\"")
            }
            Event::Dns { sta, name, answer } => write!(
                out,
                "\"dns\",\"sta\":\"{sta}\",\"name\":\"{}\",\"answer\":",
                json_text(name)
            )
            .and_then(|()| match answer {
                Some(ip) => write!(out, "\"{ip}\""),
                None => write!(out, "null"),
            }),
            Event::NxDomain { sta, name } => write!(
                out,
                "\"nxdomain\",\"sta\":\"{sta}\",\"name\":\"{}\"",
                json_text(name)
            ),
            Event::Sntp { sta, utc_ms } => write!(
                out,
                "\"sntp\",\"sta\":\"{sta}\",\"sent\":\"{}\"",
                utc(*utc_ms)
            ),
        };
        out.push('}');
        out
    }
}

/// `text` inside a JSON string: a quote, a backslash and a control character escaped.
fn json_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\u{0}'..='\u{1f}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            _ => out.push(c),
        }
    }
    out
}

/// `ms` milliseconds from 1970-01-01T00:00:00Z as an RFC 3339 UTC time with milliseconds, by
/// the proleptic Gregorian calendar.
fn utc(ms: u64) -> String {
    let (days, ms) = (ms / 86_400_000, ms % 86_400_000);
    // Days from 0000-03-01, in 400-year eras of 146097 days, with March first in each year so
    // that the leap day ends it.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era * 400 + yoe + u64::from(month <= 2);
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{ms:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STA: Mac = Mac([0x00, 0x0d, 0x93, 0x82, 0x36, 0x3a]);

    fn json(event: Event) -> String {
        Row {
            at: Time::from_ps(12),
            event,
        }
        .json()
    }

    #[test]
    fn each_row_is_one_json_object_with_its_fields_in_order() {
        assert_eq!(json(Event::Up), r#"{"t_ps":12,"event":"ap_up"}"#);
        assert_eq!(json(Event::Down), r#"{"t_ps":12,"event":"ap_down"}"#);
        assert_eq!(
            json(Event::Auth {
                sta: STA,
                status: 0
            }),
            r#"{"t_ps":12,"event":"auth","sta":"00:0d:93:82:36:3a","status":0}"#
        );
        assert_eq!(
            json(Event::Assoc {
                sta: STA,
                status: 0,
                aid: Some(1)
            }),
            r#"{"t_ps":12,"event":"assoc","sta":"00:0d:93:82:36:3a","status":0,"aid":1}"#
        );
        assert_eq!(
            json(Event::Assoc {
                sta: STA,
                status: 41,
                aid: None
            }),
            r#"{"t_ps":12,"event":"assoc","sta":"00:0d:93:82:36:3a","status":41,"aid":null}"#
        );
        assert_eq!(
            json(Event::Deauth {
                sta: Mac::BROADCAST,
                reason: 3,
                by: By::Ap
            }),
            r#"{"t_ps":12,"event":"deauth","sta":"ff:ff:ff:ff:ff:ff","reason":3,"by":"ap"}"#
        );
        assert_eq!(
            json(Event::Disassoc {
                sta: STA,
                reason: 8,
                by: By::Station
            }),
            r#"{"t_ps":12,"event":"disassoc","sta":"00:0d:93:82:36:3a","reason":8,"by":"station"}"#
        );
        assert_eq!(
            json(Event::AddbaDeclined { sta: STA, tid: 6 }),
            r#"{"t_ps":12,"event":"addba_declined","sta":"00:0d:93:82:36:3a","tid":6}"#
        );
        for (event, name) in [
            (Event::Handshake { sta: STA }, "handshake"),
            (Event::Doze { sta: STA }, "doze"),
            (Event::Wake { sta: STA }, "wake"),
            (Event::Lost { sta: STA }, "lost"),
            (Event::Dropped { sta: STA }, "dropped"),
        ] {
            assert_eq!(
                json(event),
                format!(r#"{{"t_ps":12,"event":"{name}","sta":"00:0d:93:82:36:3a"}}"#)
            );
        }
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        assert_eq!(
            json(Event::DhcpAck { sta: STA, ip }),
            r#"{"t_ps":12,"event":"dhcp_ack","sta":"00:0d:93:82:36:3a","ip":"192.168.1.100"}"#
        );
        assert_eq!(
            json(Event::DhcpNak { sta: STA, ip }),
            r#"{"t_ps":12,"event":"dhcp_nak","sta":"00:0d:93:82:36:3a","ip":"192.168.1.100"}"#
        );
        assert_eq!(
            json(Event::Dns {
                sta: STA,
                name: "pool.ntp.org".into(),
                answer: Some(Ipv4Addr::new(192, 168, 1, 1))
            }),
            r#"{"t_ps":12,"event":"dns","sta":"00:0d:93:82:36:3a","name":"pool.ntp.org","answer":"192.168.1.1"}"#
        );
        assert_eq!(
            json(Event::Dns {
                sta: STA,
                name: "pool.ntp.org".into(),
                answer: None
            }),
            r#"{"t_ps":12,"event":"dns","sta":"00:0d:93:82:36:3a","name":"pool.ntp.org","answer":null}"#
        );
        assert_eq!(
            json(Event::NxDomain {
                sta: STA,
                name: r#"a"b\000.c"#.into()
            }),
            r#"{"t_ps":12,"event":"nxdomain","sta":"00:0d:93:82:36:3a","name":"a\"b\\000.c"}"#
        );
        assert_eq!(
            json(Event::Sntp {
                sta: STA,
                utc_ms: 1_773_480_362_090
            }),
            r#"{"t_ps":12,"event":"sntp","sta":"00:0d:93:82:36:3a","sent":"2026-03-14T09:26:02.090Z"}"#
        );
    }

    #[test]
    fn utc_times_follow_the_gregorian_calendar() {
        for (ms, want) in [
            (0, "1970-01-01T00:00:00.000Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (1_767_225_599_999, "2025-12-31T23:59:59.999Z"),
            (1_767_225_600_000, "2026-01-01T00:00:00.000Z"),
            (4_107_542_400_000, "2100-03-01T00:00:00.000Z"),
            (2_085_978_496_000, "2036-02-07T06:28:16.000Z"),
        ] {
            assert_eq!(utc(ms), want);
        }
        assert_eq!(json_text("\u{1}x"), "\\u0001x");
    }
}
