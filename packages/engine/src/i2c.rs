//! The bus phases a chip's TWI master saw in a transient run.
//!
//! ```text
//! traces/i2c.csv   when a chip in the netlist has a TWI
//!   t_ps,part,phase,address,read,byte,ack
//!   <first bus change ps>,<part name>,<phase>,<7-bit address>,<0 or 1>,<decimal byte>,<0 or 1>
//! ```
//!
//! `phase` is `start`, `repeated_start`, `address`, `data`, `stop`, `arbitration_lost` or
//! `bus_error`. `address` is set on address rows, `byte` on data rows, and `read` and `ack` on
//! both. Every other cell is empty.

use std::fmt::Write;

use orbe_mcu::I2cPhase;
use orbe_model::Time;

use crate::name::Name;

/// One phase: its first bus change, which TWI of the run saw it, and what it was.
/// `twi` indexes the run's TWIs, chips in part order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct I2cRow {
    pub at: Time,
    pub twi: u16,
    pub phase: I2cPhase,
}

/// A run's TWIs, each as its part's name, and the phases they saw in the order the phases
/// ended, then in TWI order.
#[derive(Debug, PartialEq)]
pub struct I2c {
    twis: Vec<Name>,
    rows: Vec<I2cRow>,
}

impl I2c {
    pub fn new(twis: Vec<Name>, rows: Vec<I2cRow>) -> I2c {
        I2c { twis, rows }
    }

    /// The bytes of `i2c.csv`. The header, then one row per phase in order, each line ended by
    /// `\n`.
    pub fn csv(&self) -> Vec<u8> {
        let mut text = String::from("t_ps,part,phase,address,read,byte,ack\n");
        let bit = |b: bool| u8::from(b);
        for row in &self.rows {
            let part = self.twis[usize::from(row.twi)].as_str();
            let at = row.at.as_ps();
            match row.phase {
                I2cPhase::Address { address, read, ack } => writeln!(
                    text,
                    "{at},{part},address,{address},{},,{}",
                    bit(read),
                    bit(ack)
                ),
                I2cPhase::Data { byte, read, ack } => {
                    writeln!(text, "{at},{part},data,,{},{byte},{}", bit(read), bit(ack))
                }
                other => {
                    let phase = match other {
                        I2cPhase::Start => "start",
                        I2cPhase::RepeatedStart => "repeated_start",
                        I2cPhase::Stop => "stop",
                        I2cPhase::ArbitrationLost => "arbitration_lost",
                        _ => "bus_error",
                    };
                    writeln!(text, "{at},{part},{phase},,,,")
                }
            }
            .expect("a String takes every write");
        }
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(at: u64, twi: u16, phase: I2cPhase) -> I2cRow {
        I2cRow {
            at: Time::from_ps(at),
            twi,
            phase,
        }
    }

    #[test]
    fn each_phase_fills_only_its_cells() {
        let twis = vec![Name::new("u1").unwrap(), Name::new("u2").unwrap()];
        let i2c = I2c::new(
            twis,
            vec![
                row(10, 0, I2cPhase::Start),
                row(
                    20,
                    0,
                    I2cPhase::Address {
                        address: 0x50,
                        read: false,
                        ack: true,
                    },
                ),
                row(
                    30,
                    0,
                    I2cPhase::Data {
                        byte: 255,
                        read: true,
                        ack: false,
                    },
                ),
                row(40, 0, I2cPhase::RepeatedStart),
                row(50, 1, I2cPhase::ArbitrationLost),
                row(60, 1, I2cPhase::BusError),
                row(70, 0, I2cPhase::Stop),
            ],
        );
        assert_eq!(
            String::from_utf8(i2c.csv()).unwrap(),
            "t_ps,part,phase,address,read,byte,ack\n\
             10,u1,start,,,,\n\
             20,u1,address,80,0,,1\n\
             30,u1,data,,1,255,0\n\
             40,u1,repeated_start,,,,\n\
             50,u2,arbitration_lost,,,,\n\
             60,u2,bus_error,,,,\n\
             70,u1,stop,,,,\n"
        );
    }

    #[test]
    fn an_empty_log_is_the_header() {
        assert_eq!(
            I2c::new(vec![Name::new("u1").unwrap()], Vec::new()).csv(),
            b"t_ps,part,phase,address,read,byte,ack\n"
        );
    }
}
