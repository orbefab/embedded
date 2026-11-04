//! Wall-clock time for the run manifest.

/// Wall-clock time at whole-second precision, written as RFC 3339 UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    unix_seconds: u64,
}

impl Timestamp {
    #[allow(clippy::disallowed_methods)]
    pub fn now() -> Timestamp {
        let unix_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        Timestamp { unix_seconds }
    }

    pub fn from_unix_seconds(unix_seconds: u64) -> Timestamp {
        Timestamp { unix_seconds }
    }

    pub(crate) fn rfc3339(self) -> String {
        let days = (self.unix_seconds / 86_400) as i64;
        let tod = self.unix_seconds % 86_400;
        let hour = tod / 3_600;
        let minute = (tod % 3_600) / 60;
        let second = tod % 60;
        let (year, month, day) = civil_from_days(days);
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    }

    pub(crate) fn parse(text: &str) -> Option<Timestamp> {
        let bytes = text.as_bytes();
        let t = bytes.iter().position(|byte| *byte == b'T')?;
        if t < 10 || bytes.len() != t + 10 || bytes[t + 9] != b'Z' {
            return None;
        }
        let date = &text[..t];
        let time = &text[t + 1..t + 9];
        let (year_text, rest) = date.split_once('-')?;
        let (month_text, day_text) = rest.split_once('-')?;
        if year_text.len() < 4
            || month_text.len() != 2
            || day_text.len() != 2
            || time.as_bytes()[2] != b':'
            || time.as_bytes()[5] != b':'
        {
            return None;
        }
        let year = ascii_i64(year_text)?;
        let month = ascii_i64(month_text)?;
        let day = ascii_i64(day_text)?;
        let hour = ascii_i64(&time[0..2])?;
        let minute = ascii_i64(&time[3..5])?;
        let second = ascii_i64(&time[6..8])?;
        if !(1..=12).contains(&month)
            || !(0..=23).contains(&hour)
            || !(0..=59).contains(&minute)
            || !(0..=59).contains(&second)
        {
            return None;
        }
        let month = u32::try_from(month).ok()?;
        let day = u32::try_from(day).ok()?;
        let max_day = days_in_month(year, month)?;
        if day < 1 || day > max_day {
            return None;
        }
        let days = days_from_civil(year, month, day)?;
        if days < 0 {
            return None;
        }
        let unix_seconds = u64::try_from(days)
            .ok()?
            .checked_mul(86_400)?
            .checked_add(u64::try_from(hour).ok()? * 3_600)?
            .checked_add(u64::try_from(minute).ok()? * 60)?
            .checked_add(u64::try_from(second).ok()?)?;
        let stamp = Timestamp { unix_seconds };
        (stamp.rfc3339() == text).then_some(stamp)
    }
}

/// Days since the Unix epoch to a Gregorian year, month, and day.
/// 719468 shifts the epoch to a 400-year boundary. Unix time has no leap seconds.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year, month as u32, day as u32)
}

fn ascii_i64(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn days_in_month(year: i64, month: u32) -> Option<u32> {
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => return None,
    })
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Inverse of `civil_from_days`.
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let year = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = if year >= 0 {
        year / 400
    } else {
        year.checked_sub(399)? / 400
    };
    let yoe = year.checked_sub(era.checked_mul(400)?)?;
    if !(0..400).contains(&yoe) {
        return None;
    }
    let yoe = u64::try_from(yoe).ok()?;
    let shifted = if month > 2 {
        u64::from(month) - 3
    } else {
        u64::from(month) + 9
    };
    let doy = (153 * shifted + 2) / 5 + u64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era.checked_mul(146_097)?
        .checked_add(i64::try_from(doe).ok()?)?
        .checked_sub(719_468)
}
