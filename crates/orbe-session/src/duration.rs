//! Durations as picoseconds: `500ms`, `1.5ms` and `20us` parse with integer arithmetic.

use std::fmt;

/// A span of simulated time, in picoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration(u64);

const UNITS: &[(&str, u32)] = &[
    ("ps", 0),
    ("ns", 3),
    ("us", 6),
    ("µs", 6),
    ("ms", 9),
    ("s", 12),
];

impl Duration {
    pub const fn from_ps(ps: u64) -> Duration {
        Duration(ps)
    }

    pub const fn from_ms(ms: u64) -> Duration {
        Duration(ms * 1_000_000_000)
    }

    pub fn as_ps(self) -> u64 {
        self.0
    }

    /// Parses a number and a unit, such as `1.5ms`. A fraction of a picosecond is refused.
    pub fn parse(text: &str) -> Result<Duration, String> {
        let split = text
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .ok_or_else(|| format!("`{text}` has no unit; write one such as `100ms`"))?;
        let (number, unit) = text.split_at(split);
        let exponent = UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, e)| *e)
            .ok_or_else(|| format!("`{unit}` is not a time unit; use ps, ns, us, ms or s"))?;
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        if whole.is_empty()
            || fraction.contains('.')
            || (number.contains('.') && fraction.is_empty())
        {
            return Err(format!("`{text}` is not a duration such as `1.5ms`"));
        }
        let fraction = fraction.trim_end_matches('0');
        let places = u32::try_from(fraction.len()).unwrap_or(u32::MAX);
        if places > exponent {
            return Err(format!("`{text}` is a fraction of a picosecond"));
        }
        let too_long = || format!("`{text}` is longer than a session can run");
        let scale = 10u64.pow(exponent);
        let whole: u64 = whole.parse().map_err(|_| too_long())?;
        let fraction: u64 = if fraction.is_empty() {
            0
        } else {
            fraction.parse::<u64>().map_err(|_| too_long())? * 10u64.pow(exponent - places)
        };
        whole
            .checked_mul(scale)
            .and_then(|ps| ps.checked_add(fraction))
            .map(Duration)
            .ok_or_else(too_long)
    }
}

/// The largest unit the time fills, exact: `100ms`, `1.5ms`, `100.06ms`, `0s`.
impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (unit, exponent) = UNITS
            .iter()
            .rev()
            .filter(|(name, _)| *name != "µs")
            .find(|(_, e)| self.0 >= 10u64.pow(*e))
            .copied()
            .unwrap_or(("s", 12));
        let scale = 10u64.pow(exponent);
        let (whole, rest) = (self.0 / scale, self.0 % scale);
        if rest == 0 {
            return write!(f, "{whole}{unit}");
        }
        let digits = format!("{rest:0width$}", width = exponent as usize);
        write!(f, "{whole}.{}{unit}", digits.trim_end_matches('0'))
    }
}
