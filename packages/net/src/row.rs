//! What the network did, one row of `net.jsonl` each.

use std::fmt::Write;

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
        };
        out.push('}');
        out
    }
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
    }
}
