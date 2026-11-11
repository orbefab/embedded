//! The bytes a chip's USART sent in a transient run.
//!
//! ```text
//! traces/serial.csv   when a chip in the netlist has a USART
//!   t_ps,part,usart,byte
//!   <start-bit ps>,<part name>,<USART name>,<decimal byte>
//! ```

use std::fmt::Write;

use orbe_model::Time;

use crate::name::Name;

/// One byte a USART sent: its start-bit time, which USART of the run sent it, and the byte.
/// `usart` indexes the run's USARTs, chips in part order, then each chip's in datasheet order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialByte {
    pub at: Time,
    pub usart: u16,
    pub byte: u8,
}

/// A run's USARTs, each as its part's name and its own, and the bytes they sent in the order
/// stop bits ended, then by part name, USART index and send order.
#[derive(Debug, PartialEq)]
pub struct Serial {
    usarts: Vec<(Name, &'static str)>,
    bytes: Vec<SerialByte>,
}

impl Serial {
    pub fn new(usarts: Vec<(Name, &'static str)>, bytes: Vec<SerialByte>) -> Serial {
        Serial { usarts, bytes }
    }

    /// The bytes of `serial.csv`. The header, then one row per byte in order, each line ended by
    /// `\n`.
    pub fn csv(&self) -> Vec<u8> {
        let mut text = String::from("t_ps,part,usart,byte\n");
        for byte in &self.bytes {
            let (part, usart) = &self.usarts[usize::from(byte.usart)];
            writeln!(
                text,
                "{},{},{usart},{}",
                byte.at.as_ps(),
                part.as_str(),
                byte.byte
            )
            .expect("a String takes every write");
        }
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn byte(at: u64, usart: u16, byte: u8) -> SerialByte {
        SerialByte {
            at: Time::from_ps(at),
            usart,
            byte,
        }
    }

    fn usarts() -> Vec<(Name, &'static str)> {
        vec![
            (Name::new("u1").unwrap(), "USART0"),
            (Name::new("w1").unwrap(), "USART1"),
        ]
    }

    /// `w1`'s byte started first and ended last, so it is the last row.
    #[test]
    fn rows_are_decimal_in_log_order() {
        let serial = Serial::new(
            usarts(),
            vec![
                byte(1_000, 0, 53),
                byte(2_040, 0, 10),
                byte(3_080, 0, 0),
                byte(4_120, 0, 7),
                byte(500, 1, 255),
            ],
        );
        assert_eq!(
            String::from_utf8(serial.csv()).unwrap(),
            "t_ps,part,usart,byte\n\
             1000,u1,USART0,53\n\
             2040,u1,USART0,10\n\
             3080,u1,USART0,0\n\
             4120,u1,USART0,7\n\
             500,w1,USART1,255\n"
        );
    }

    #[test]
    fn an_empty_log_is_the_header() {
        assert_eq!(
            Serial::new(usarts(), Vec::new()).csv(),
            b"t_ps,part,usart,byte\n"
        );
    }
}
