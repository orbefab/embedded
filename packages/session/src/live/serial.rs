//! The serial a session keeps: every byte since `open`, decoded as UTF-8 as it arrives. The
//! replies show this text and `serial contains` searches it.

use orbe_circuit::SerialCursor;

use crate::reply::{Change, held};

/// The serial text a failed `serial contains` shows, from the end.
const SERIAL_SHOWN: usize = 200;

#[derive(Default)]
pub(super) struct Serial {
    pub(super) cursor: SerialCursor,
    /// Every whole character so far. A byte that starts no character is U+FFFD.
    text: String,
    /// The bytes since `open`.
    count: usize,
    /// The bytes of a character whose last bytes have not reached the session, with their
    /// start bits.
    held: Vec<(u8, u64)>,
}

/// The text one batch of bytes completed: where it starts in the serial's text, and the end
/// of each character in it with the start bit of that character's last byte.
#[derive(Debug, Default)]
pub(super) struct Arrived {
    from: usize,
    ends: Vec<(usize, u64)>,
}

impl Serial {
    /// Decodes bytes as they arrive. A character whose last bytes have not arrived waits for
    /// them, unless the session has ended, when it is U+FFFD.
    pub(super) fn arrive(
        &mut self,
        bytes: impl Iterator<Item = (u8, u64)>,
        ended: bool,
    ) -> Arrived {
        let mut raw = std::mem::take(&mut self.held);
        let before = raw.len();
        raw.extend(bytes);
        self.count += raw.len() - before;
        let data: Vec<u8> = raw.iter().map(|&(byte, _)| byte).collect();
        let mut arrived = Arrived {
            from: self.text.len(),
            ends: Vec::new(),
        };
        let mut read = 0;
        for chunk in data.utf8_chunks() {
            for c in chunk.valid().chars() {
                read += c.len_utf8();
                self.text.push(c);
                arrived.ends.push((self.text.len(), raw[read - 1].1));
            }
            let invalid = chunk.invalid();
            if invalid.is_empty() {
                continue;
            }
            let unfinished = read + invalid.len() == data.len()
                && std::str::from_utf8(invalid).is_err_and(|e| e.error_len().is_none());
            if unfinished && !ended {
                self.held = raw[read..].to_vec();
                break;
            }
            read += invalid.len();
            self.text.push(char::REPLACEMENT_CHARACTER);
            arrived.ends.push((self.text.len(), raw[read - 1].1));
        }
        arrived
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn held(&self) -> usize {
        self.held.len()
    }

    pub(super) fn contains(&self, needle: &str) -> bool {
        self.text.contains(needle)
    }

    /// The start bit of the byte that completed the first match of `needle` that `arrived`
    /// completed.
    pub(super) fn met(&self, arrived: &Arrived, needle: &str) -> Option<u64> {
        let mut start = arrived.from.saturating_sub(needle.len().saturating_sub(1));
        while !self.text.is_char_boundary(start) {
            start -= 1;
        }
        let end = start + self.text[start..].find(needle)? + needle.len();
        arrived
            .ends
            .iter()
            .find(|&&(at, _)| at >= end)
            .map(|&(_, at_ps)| at_ps)
    }

    /// The text `arrived` completed, by `by_ps`, the time the session read it.
    pub(super) fn change(&self, arrived: &Arrived, by_ps: u64) -> Option<Change> {
        let &(_, sent_at_ps) = arrived.ends.last()?;
        Some(Change::Serial {
            by_ps,
            sent_at_ps,
            text: self.text[arrived.from..].to_string(),
        })
    }

    pub(super) fn actual(&self) -> String {
        if self.count == 0 {
            return "no serial".to_string();
        }
        let held = held(self.held.len());
        if self.text.len() <= SERIAL_SHOWN {
            let note = held.as_ref().map(|a| format!(" ({a})")).unwrap_or_default();
            return format!("serial {:?}{note}", self.text);
        }
        let mut from = self.text.len() - SERIAL_SHOWN;
        while !self.text.is_char_boundary(from) {
            from += 1;
        }
        let note = held.map(|a| format!("; {a}")).unwrap_or_default();
        format!(
            "serial ends {:?} ({} bytes in all{note})",
            &self.text[from..],
            self.count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrive(serial: &mut Serial, bytes: &[u8], at_ps: u64, ended: bool) -> Option<Change> {
        let timed = bytes.iter().zip(at_ps..).map(|(&b, at)| (b, at));
        let arrived = serial.arrive(timed, ended);
        serial.change(&arrived, at_ps + 50)
    }

    fn change(by_ps: u64, sent_at_ps: u64, text: &str) -> Option<Change> {
        Some(Change::Serial {
            by_ps,
            sent_at_ps,
            text: text.into(),
        })
    }

    #[test]
    fn a_character_split_across_batches_shows_once_whole() {
        let mut serial = Serial::default();
        assert_eq!(
            arrive(&mut serial, b"caf\xc3", 100, false),
            change(150, 102, "caf")
        );
        assert_eq!(
            serial.actual(),
            "serial \"caf\" (1 byte of a character held until the rest arrives)"
        );
        assert_eq!(
            arrive(&mut serial, b"\xa9 ok\xff", 200, false),
            change(250, 204, "é ok\u{fffd}"),
            "an invalid byte shows as U+FFFD at once"
        );
        assert!(
            serial.contains("é ok\u{fffd}"),
            "a match reads the text shown"
        );
        assert_eq!(arrive(&mut serial, b"\xe2\x82", 300, false), None);
        assert_eq!(serial.held(), 2);
        assert_eq!(
            arrive(&mut serial, b"", 400, true),
            change(450, 301, "\u{fffd}"),
            "once the session has ended no more bytes come"
        );
        assert_eq!(serial.actual(), "serial \"café ok\u{fffd}\u{fffd}\"");
    }

    #[test]
    fn a_match_meets_at_the_byte_that_completed_it() {
        let mut serial = Serial::default();
        let first = serial.arrive([(b'a', 10), (0xc3, 20)].into_iter(), false);
        assert_eq!(serial.met(&first, "aé"), None);
        let second = serial.arrive([(0xa9, 30), (b'b', 40)].into_iter(), false);
        assert_eq!(serial.met(&second, "aé"), Some(30));
        assert_eq!(serial.met(&second, "b"), Some(40));
    }
}
