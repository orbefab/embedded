//! The records active chips sent each other in a transient run.
//!
//! ```text
//! traces/link.csv   when a chip in the netlist can send records
//!   t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash
//!   <end ps>,<start ps>,<sender part name>,<sender pin>,<spi, pulses or duty>,<cs>,<phases>,<bytes>,<hash>
//! ```
//!
//! A `spi` row's `cs` is `held` or `released`, its `phases` are its phases before the data,
//! each `<bits>/<lanes>:<value in hex>`, separated by spaces, and its `bytes` and `hash` are the
//! data's length and the first 16 hex digits of the data's SHA-256. A `pulses` row's `phases`
//! is `idle=0` or `idle=1`, and its `bytes` and `hash` cover each pulse as its level byte, then
//! its duration in picoseconds as a little-endian `u64`. A `duty` row's `phases` is
//! `<duty>/<steps>@<hz>`. Every other cell is empty.

use std::fmt::Write;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::name::Name;
use crate::netlist::NetId;
use crate::time::Time;

/// One record a chip's core made. `end` is a stop of its sender, where every chip whose record
/// pins sit on its nets takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// How many records its sender made before it.
    pub seq: u64,
    pub start: Time,
    pub end: Time,
    /// The sender's pin that names the link.
    pub pin: &'static str,
    pub kind: RecordKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecordKind {
    Spi(Spi),
    Pulses(Pulses),
    Duty(Duty),
}

/// One SPI transaction as the host's registers set it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spi {
    /// The phases before the data, such as a command and an address, in the order they ran.
    pub phases: Vec<SpiPhase>,
    /// The data's lanes.
    pub lanes: Lanes,
    pub data: Arc<[u8]>,
    pub clock_hz: u32,
    /// The chip select at the transaction's end.
    pub cs: Cs,
    /// The nets of the host's pins when it sent.
    pub nets: SpiNets,
}

/// One phase before the data: `bits` bits of `value`, most significant first, on `lanes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpiPhase {
    /// From 1 to 32.
    pub bits: u8,
    pub lanes: Lanes,
    /// Below `2^bits`.
    pub value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lanes {
    One,
    Two,
    Four,
}

impl Lanes {
    pub fn count(self) -> u8 {
        match self {
            Lanes::One => 1,
            Lanes::Two => 2,
            Lanes::Four => 4,
        }
    }
}

/// Whether a transaction leaves its chip select asserted. A data-only transaction after one
/// that ends `Held` continues it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cs {
    Held,
    Released,
}

/// The nets of an SPI host's CS, SCLK and data pins. A receiver takes the transaction when its
/// own CS and SCLK pins sit on `cs` and `sclk`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpiNets {
    pub cs: NetId,
    pub sclk: NetId,
    /// D0 to D3. `None` for a lane the host has no pin for.
    pub data: [Option<NetId>; 4],
}

/// A pulse train, as an RMT channel sends it: the idle level, then each pulse's level and
/// duration in picoseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Pulses {
    pub idle: bool,
    pub pulses: Arc<[(bool, u64)]>,
    /// The net of the sender's pin when it sent. A receiver whose pulse pin sits on it takes
    /// the record.
    pub net: NetId,
}

/// A PWM duty as an LEDC channel sets it: `duty` of `2^resolution` steps at `hz`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duty {
    pub duty: u32,
    /// Below 32.
    pub resolution: u8,
    pub hz: u32,
    /// The net of the sender's pin when it sent. A receiver whose duty pin sits on it takes the
    /// record.
    pub net: NetId,
}

/// Why [`Link::csv`] cannot write a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// `duty.resolution` is 32 or more.
    Resolution,
}

/// One delivered record and which sender of the run made it. `sender` indexes the run's
/// senders, chips in part order.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkRow {
    pub sender: u16,
    pub record: Record,
}

/// A run's senders, each as its part's name, and the records they sent, ordered by end, then
/// by sender name, then by sequence.
#[derive(Debug, PartialEq)]
pub struct Link {
    senders: Vec<Name>,
    rows: Vec<LinkRow>,
}

impl Link {
    pub fn new(senders: Vec<Name>, rows: Vec<LinkRow>) -> Link {
        Link { senders, rows }
    }

    /// The bytes of `link.csv`. The header, then one row per record in order, each line ended
    /// by `\n`. A duty whose resolution is 32 or more is [`LinkError::Resolution`].
    pub fn csv(&self) -> Result<Vec<u8>, LinkError> {
        let mut text = String::from("t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash\n");
        for row in &self.rows {
            let record = &row.record;
            let part = self.senders[usize::from(row.sender)].as_str();
            write!(
                text,
                "{},{},{part},{},",
                record.end.as_ps(),
                record.start.as_ps(),
                record.pin
            )
            .expect("a String takes every write");
            match &record.kind {
                RecordKind::Spi(spi) => {
                    let cs = match spi.cs {
                        Cs::Held => "held",
                        Cs::Released => "released",
                    };
                    let phases: Vec<String> = spi.phases.iter().map(phase).collect();
                    writeln!(
                        text,
                        "spi,{cs},{},{},{}",
                        phases.join(" "),
                        spi.data.len(),
                        hash(&spi.data)
                    )
                }
                RecordKind::Pulses(pulses) => {
                    let payload: Vec<u8> = pulses
                        .pulses
                        .iter()
                        .flat_map(|&(level, ps)| {
                            std::iter::once(u8::from(level)).chain(ps.to_le_bytes())
                        })
                        .collect();
                    writeln!(
                        text,
                        "pulses,,idle={},{},{}",
                        u8::from(pulses.idle),
                        payload.len(),
                        hash(&payload)
                    )
                }
                RecordKind::Duty(duty) => writeln!(
                    text,
                    "duty,,{}/{}@{},,",
                    duty.duty,
                    duty_steps(duty.resolution)?,
                    duty.hz
                ),
            }
            .expect("a String takes every write");
        }
        Ok(text.into_bytes())
    }
}

/// `2^resolution` when `resolution` is below 32.
fn duty_steps(resolution: u8) -> Result<u64, LinkError> {
    1u64.checked_shl(u32::from(resolution))
        .filter(|_| resolution < 32)
        .ok_or(LinkError::Resolution)
}

/// `<bits>/<lanes>:<value>`, the value in lowercase hex with one digit per four bits or part.
fn phase(phase: &SpiPhase) -> String {
    let digits = usize::from(phase.bits.div_ceil(4));
    format!(
        "{}/{}:{:0digits$x}",
        phase.bits,
        phase.lanes.count(),
        phase.value
    )
}

/// The first 16 hex digits of the SHA-256 of `bytes`.
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spi(seq: u64, end: u64, cs: Cs, data: &[u8]) -> Record {
        Record {
            seq,
            start: Time::from_ps(end - 100),
            end: Time::from_ps(end),
            pin: "CS",
            kind: RecordKind::Spi(Spi {
                phases: vec![
                    SpiPhase {
                        bits: 8,
                        lanes: Lanes::One,
                        value: 0x02,
                    },
                    SpiPhase {
                        bits: 24,
                        lanes: Lanes::One,
                        value: 0x2c00,
                    },
                ],
                lanes: Lanes::Four,
                data: data.into(),
                clock_hz: 40_000_000,
                cs,
                nets: SpiNets {
                    cs: NetId::GROUND,
                    sclk: NetId::GROUND,
                    data: [Some(NetId::GROUND), None, None, None],
                },
            }),
        }
    }

    #[test]
    fn each_kind_writes_its_row() {
        let names = vec![Name::new("rp1").unwrap(), Name::new("u2").unwrap()];
        let rows = vec![
            LinkRow {
                sender: 0,
                record: spi(0, 500, Cs::Held, b"abc"),
            },
            LinkRow {
                sender: 1,
                record: Record {
                    seq: 0,
                    start: Time::from_ps(600),
                    end: Time::from_ps(900),
                    pin: "GPIO0",
                    kind: RecordKind::Pulses(Pulses {
                        idle: false,
                        pulses: vec![(true, 400), (false, 850)].into(),
                        net: NetId::GROUND,
                    }),
                },
            },
            LinkRow {
                sender: 1,
                record: Record {
                    seq: 1,
                    start: Time::from_ps(1000),
                    end: Time::from_ps(1000),
                    pin: "GPIO21",
                    kind: RecordKind::Duty(Duty {
                        duty: 512,
                        resolution: 10,
                        hz: 5000,
                        net: NetId::GROUND,
                    }),
                },
            },
        ];
        let pulses = [1, 144, 1, 0, 0, 0, 0, 0, 0, 0, 82, 3, 0, 0, 0, 0, 0, 0];
        let csv = String::from_utf8(Link::new(names, rows).csv().unwrap()).unwrap();
        assert_eq!(
            csv,
            format!(
                "t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash\n\
                 500,400,rp1,CS,spi,held,8/1:02 24/1:002c00,3,{}\n\
                 900,600,u2,GPIO0,pulses,,idle=0,18,{}\n\
                 1000,1000,u2,GPIO21,duty,,512/1024@5000,,\n",
                hash(b"abc"),
                hash(&pulses),
            )
        );
    }

    #[test]
    fn the_hash_is_the_first_eight_bytes_of_the_sha256() {
        assert_eq!(hash(b""), "e3b0c44298fc1c14");
        assert_eq!(hash(b"abc"), "ba7816bf8f01cfea");
    }

    #[test]
    fn no_record_is_the_header_alone() {
        let link = Link::new(vec![Name::new("rp1").unwrap()], Vec::new());
        assert_eq!(
            link.csv().unwrap(),
            b"t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash\n"
        );
    }

    fn duty(resolution: u8) -> Link {
        Link::new(
            vec![Name::new("u1").unwrap()],
            vec![LinkRow {
                sender: 0,
                record: Record {
                    seq: 0,
                    start: Time::from_ps(0),
                    end: Time::from_ps(1),
                    pin: "GPIO21",
                    kind: RecordKind::Duty(Duty {
                        duty: 1,
                        resolution,
                        hz: 1,
                        net: NetId::GROUND,
                    }),
                },
            }],
        )
    }

    #[test]
    fn a_duty_resolution_of_32_or_more_is_an_error() {
        assert_eq!(duty(32).csv(), Err(LinkError::Resolution));
        assert_eq!(duty(64).csv(), Err(LinkError::Resolution));
        let csv = String::from_utf8(duty(31).csv().unwrap()).unwrap();
        assert!(csv.contains("1/2147483648@1"), "{csv}");
    }
}
