//! A number param's unit and the value grammar: `"220"`, `"10k"`, `"10kohm"`, `"4.7 kΩ"`, `"1ms"`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    Ohm,
    Farad,
    Volt,
    Ampere,
    Second,
    Hertz,
    Watt,
    One,
}

impl Unit {
    pub(crate) fn parse(text: &str) -> Option<Unit> {
        Some(match text {
            "ohm" => Unit::Ohm,
            "F" => Unit::Farad,
            "V" => Unit::Volt,
            "A" => Unit::Ampere,
            "s" => Unit::Second,
            "Hz" => Unit::Hertz,
            "W" => Unit::Watt,
            "1" => Unit::One,
            _ => return None,
        })
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Unit::Ohm => "ohm",
            Unit::Farad => "F",
            Unit::Volt => "V",
            Unit::Ampere => "A",
            Unit::Second => "s",
            Unit::Hertz => "Hz",
            Unit::Watt => "W",
            Unit::One => "1",
        }
    }

    fn written(self) -> &'static [&'static str] {
        match self {
            Unit::Ohm => &["ohm", "ohms", "Ω"],
            Unit::Farad => &["F"],
            Unit::Volt => &["V"],
            Unit::Ampere => &["A"],
            Unit::Second => &["s"],
            Unit::Hertz => &["Hz"],
            Unit::Watt => &["W"],
            Unit::One => &[],
        }
    }

    /// An example for an error message.
    pub(crate) fn example(self) -> &'static str {
        match self {
            Unit::Ohm => "\"220\" or \"10k\"",
            Unit::Farad => "\"100n\" or \"10u\"",
            Unit::Volt => "\"5\" or \"3.3V\"",
            Unit::Ampere => "\"20m\" or \"1mA\"",
            Unit::Second => "\"1ms\"",
            Unit::Hertz => "\"16M\"",
            Unit::Watt => "\"0.25\"",
            Unit::One => "3",
        }
    }
}

const PREFIXES: &[(&str, i32)] = &[
    ("p", -12),
    ("n", -9),
    ("u", -6),
    ("µ", -6),
    ("μ", -6),
    ("m", -3),
    ("k", 3),
    ("M", 6),
    ("G", 9),
];

/// The value of `text` in `unit`, with `shift` added to its decimal exponent, so a duration can
/// come back in picoseconds with no rounding from a multiplication.
pub(crate) fn parse(text: &str, unit: Unit, shift: i32) -> Result<f64, String> {
    let text = text.trim();
    let digits = number_prefix(text);
    if digits.is_empty() {
        return Err(format!("`{text}` does not start with a number"));
    }
    let rest = text[digits.len()..].trim_start();
    let exponent = if rest.is_empty() || unit.written().contains(&rest) {
        0
    } else {
        PREFIXES
            .iter()
            .find(|(p, _)| {
                rest.strip_prefix(p)
                    .is_some_and(|tail| tail.is_empty() || unit.written().contains(&tail))
            })
            .map(|(_, e)| *e)
            .ok_or_else(|| format!("`{rest}` is not a prefix or unit of {}", unit.as_str()))?
    };
    let (mantissa, written_exponent) = match digits.find(['e', 'E']) {
        Some(i) => (
            &digits[..i],
            digits[i + 1..].parse::<i32>().map_err(|e| e.to_string())?,
        ),
        None => (digits, 0),
    };
    format!("{mantissa}e{}", written_exponent + exponent + shift)
        .parse()
        .map_err(|_| format!("`{text}` is not a number"))
}

fn number_prefix(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    let digits = |i: &mut usize| {
        let from = *i;
        while bytes.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i > from
    };
    if !digits(&mut i) {
        return "";
    }
    if bytes.get(i) == Some(&b'.') {
        let mut j = i + 1;
        if digits(&mut j) {
            i = j;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if digits(&mut j) {
            i = j;
        }
    }
    &text[..i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar() {
        assert_eq!(parse("220", Unit::Ohm, 0), Ok(220.0));
        assert_eq!(parse("10k", Unit::Ohm, 0), Ok(10_000.0));
        assert_eq!(parse("10kohm", Unit::Ohm, 0), Ok(10_000.0));
        assert_eq!(parse("4.7 kΩ", Unit::Ohm, 0), Ok(4_700.0));
        assert_eq!(parse("100n", Unit::Farad, 0), Ok(1e-7));
        assert_eq!(parse("1ms", Unit::Second, 12), Ok(1e9));
        assert_eq!(parse("2.5e3", Unit::Ohm, 0), Ok(2_500.0));
        assert!(parse("10kV", Unit::Ohm, 0).is_err());
        assert!(parse("k10", Unit::Ohm, 0).is_err());
    }
}
