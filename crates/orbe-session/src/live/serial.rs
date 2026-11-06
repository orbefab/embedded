//! The serial a session keeps: every byte since `open`, decoded as UTF-8.

use sim_circuit::SerialCursor;

use crate::reply::Change;

/// The serial bytes a failed `serial contains` shows, from the end.
const SERIAL_SHOWN: usize = 200;

/// Every serial byte since `open`.
#[derive(Default)]
pub(super) struct Serial {
    pub(super) cursor: SerialCursor,
    pub(super) bytes: Vec<u8>,
    /// The bytes a step's changes have shown. The rest end in a character still arriving.
    pub(super) shown: usize,
    pub(super) unshown_at: Vec<u64>,
}

pub(super) fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    bytes.windows(needle.len()).position(|w| w == needle)
}

impl Serial {
    pub(super) fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    pub(super) fn actual(&self) -> String {
        if self.bytes.is_empty() {
            return "no serial".to_string();
        }
        if self.bytes.len() <= SERIAL_SHOWN {
            return format!("serial {:?}", self.text());
        }
        let mut from = self.bytes.len() - SERIAL_SHOWN;
        while self.bytes[from] & 0xC0 == 0x80 && from + 1 < self.bytes.len() {
            from += 1;
        }
        format!(
            "serial ends {:?} ({} bytes in all)",
            String::from_utf8_lossy(&self.bytes[from..]),
            self.bytes.len()
        )
    }

    /// The text of the bytes not yet shown, at the start bit of its first byte. A character
    /// whose last bytes are still on the wire waits for them until the session has ended.
    pub(super) fn show(&mut self, ended: bool) -> Option<Change> {
        let unshown = &self.bytes[self.shown..];
        let waiting = if ended {
            0
        } else {
            unshown
                .utf8_chunks()
                .last()
                .map(|chunk| chunk.invalid())
                .filter(|tail| std::str::from_utf8(tail).is_err_and(|e| e.error_len().is_none()))
                .map_or(0, <[u8]>::len)
        };
        let count = unshown.len() - waiting;
        if count == 0 {
            return None;
        }
        let change = Change::Serial {
            at_ps: self.unshown_at[0],
            text: String::from_utf8_lossy(&unshown[..count]).into_owned(),
        };
        self.shown += count;
        self.unshown_at.drain(..count);
        Some(change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrive(serial: &mut Serial, bytes: &[u8], at_ps: u64) {
        serial.bytes.extend(bytes);
        serial
            .unshown_at
            .extend((0..bytes.len() as u64).map(|k| at_ps + k));
    }

    #[test]
    fn a_character_split_across_slices_shows_once_whole() {
        let mut serial = Serial::default();
        arrive(&mut serial, b"caf\xc3", 100);
        assert_eq!(
            serial.show(false),
            Some(Change::Serial {
                at_ps: 100,
                text: "caf".into()
            })
        );
        arrive(&mut serial, b"\xa9 ok\xff", 200);
        assert_eq!(
            serial.show(false),
            Some(Change::Serial {
                at_ps: 103,
                text: "é ok\u{fffd}".into()
            }),
            "an invalid byte shows as U+FFFD at once"
        );
        arrive(&mut serial, b"\xe2\x82", 300);
        assert_eq!(serial.show(false), None);
        assert_eq!(
            serial.show(true),
            Some(Change::Serial {
                at_ps: 300,
                text: "\u{fffd}".into()
            })
        );
    }
}
