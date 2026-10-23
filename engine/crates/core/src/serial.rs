//! The bytes a chip's USART sent in a transient run.

use crate::time::Time;

/// One byte a USART sent: its start-bit time, which USART of the run sent it, and the byte.
/// `usart` indexes the run's USARTs, chips in part order, then each chip's in datasheet order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialByte {
    pub at: Time,
    pub usart: u16,
    pub byte: u8,
}
