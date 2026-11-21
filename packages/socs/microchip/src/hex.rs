//! Intel HEX flash images.

use crate::chip::FLASH_WORDS;

/// The flash, 32 KB, DS40002061B Table 2-1.
pub const FLASH_BYTES: usize = FLASH_WORDS as usize * 2;

/// Why a HEX text is no flash image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HexRule {
    /// The line is not an Intel HEX record: no `:`, a digit that is not hexadecimal, or a
    /// length that does not match its byte count or its type.
    Record,
    /// The record's bytes do not sum to 0.
    Checksum,
    /// A record type other than 00 to 05.
    RecordType,
    /// A record after the end-of-file record.
    AfterEnd,
    /// No end-of-file record.
    NoEnd,
    /// No data record, so the flash is erased: an `.eep` of a sketch with no EEPROM data, for
    /// one, which holds only the end-of-file record.
    Empty,
    /// A data byte at or past the end of the flash.
    PastFlash,
    /// A data byte an earlier record already placed.
    Overlap,
}

/// The rule a HEX text breaks, and the 1-based line that breaks it, `None` when the text as a
/// whole does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HexFault {
    pub line: Option<usize>,
    pub rule: HexRule,
}

/// Flash from Intel HEX text, one record a line. Records 00 to 05 are read: 02 and 04 move the
/// base address, and 01, 03 and 05 may name a start address, which a part at reset does not
/// use. A gap is erased flash, `0xFF`, and no byte is written twice. Text with no data record
/// would leave the whole flash erased, which no build makes, so it is refused.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, HexFault> {
    let mut flash = Vec::new();
    let mut placed = Vec::new();
    let mut base = 0;
    let mut ended = false;
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let fault = |rule| HexFault {
            line: Some(index + 1),
            rule,
        };
        if ended {
            return Err(fault(HexRule::AfterEnd));
        }
        let bytes = record(line).ok_or(fault(HexRule::Record))?;
        if bytes.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte)) != 0 {
            return Err(fault(HexRule::Checksum));
        }
        let address = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
        let data = &bytes[4..bytes.len() - 1];
        let upper = |shift: u32| match *data {
            [high, low] => Ok(usize::from(u16::from_be_bytes([high, low])) << shift),
            _ => Err(fault(HexRule::Record)),
        };
        match bytes[3] {
            0x00 => {
                let start = base + address;
                let end = start + data.len();
                if end > FLASH_BYTES {
                    return Err(fault(HexRule::PastFlash));
                }
                if flash.len() < end {
                    flash.resize(end, 0xFF);
                    placed.resize(end, false);
                }
                if placed[start..end].contains(&true) {
                    return Err(fault(HexRule::Overlap));
                }
                flash[start..end].copy_from_slice(data);
                placed[start..end].fill(true);
            }
            0x01 if data.is_empty() => ended = true,
            0x01 => return Err(fault(HexRule::Record)),
            0x02 => base = upper(4)?,
            0x04 => base = upper(16)?,
            0x03 | 0x05 if data.len() == 4 => {}
            0x03 | 0x05 => return Err(fault(HexRule::Record)),
            _ => return Err(fault(HexRule::RecordType)),
        }
    }
    let whole = |rule| HexFault { line: None, rule };
    if !ended {
        return Err(whole(HexRule::NoEnd));
    }
    if flash.is_empty() {
        return Err(whole(HexRule::Empty));
    }
    Ok(flash)
}

/// The record's bytes, its count through its checksum, when its length matches its count.
fn record(line: &str) -> Option<Vec<u8>> {
    let digits = line.strip_prefix(':')?.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Vec<u8> = digits
        .chunks(2)
        .map(|pair| {
            let nibble = |digit: u8| char::from(digit).to_digit(16);
            Some((nibble(pair[0])? << 4 | nibble(pair[1])?) as u8)
        })
        .collect::<Option<_>>()?;
    (bytes.len() >= 5 && bytes.len() == usize::from(bytes[0]) + 5).then_some(bytes)
}
