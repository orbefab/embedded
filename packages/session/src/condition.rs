//! The one condition grammar `expect` and `step --until` share.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::target::Ref;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    /// `@led1 lit`, `@nano.D13 high`.
    State(Ref, State),
    /// `@led1.i > 5mA`.
    Compare(Ref, Op, Amount),
    /// `@pixel rgb(255,0,0)`: every LED of an addressable part shows that colour.
    Color(Ref, Rgb),
    /// `serial contains "pressed"`.
    SerialContains(Needle),
    /// `no diagnostics`.
    NoDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Lit,
    Dim,
    Off,
    High,
    Low,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Lit => "lit",
            State::Dim => "dim",
            State::Off => "off",
            State::High => "high",
            State::Low => "low",
        }
    }

    /// The light state, for a part that states `lit` and `dim`.
    pub fn light(self) -> Option<crate::reply::Light> {
        match self {
            State::Lit => Some(crate::reply::Light::Lit),
            State::Dim => Some(crate::reply::Light::Dim),
            State::Off => Some(crate::reply::Light::Off),
            State::High | State::Low => None,
        }
    }

    fn parse(word: &str) -> Option<State> {
        [State::Lit, State::Dim, State::Off, State::High, State::Low]
            .into_iter()
            .find(|s| s.as_str() == word)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
        }
    }

    pub fn holds(self, left: f64, right: f64) -> bool {
        match self {
            Op::Lt => left < right,
            Op::Le => left <= right,
            Op::Gt => left > right,
            Op::Ge => left >= right,
        }
    }

    fn parse(word: &str) -> Option<Op> {
        [Op::Lt, Op::Le, Op::Gt, Op::Ge]
            .into_iter()
            .find(|o| o.as_str() == word)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    Ampere,
    Volt,
    Watt,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Ampere => "A",
            Unit::Volt => "V",
            Unit::Watt => "W",
        }
    }

    pub fn parse(text: &str) -> Option<Unit> {
        [Unit::Ampere, Unit::Volt, Unit::Watt]
            .into_iter()
            .find(|u| u.as_str() == text)
    }
}

/// A colour as `rgb(` red, green and blue `)`, each 0 to 255, such as `rgb(255,128,0)`. A word
/// that starts with `#` is a comment on a command line, so a colour is not written in hex. Its
/// wire form is that text without spaces, read by `parse`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// `rgb(255,0,0)`, with or without spaces after the commas.
    pub fn parse(text: &str) -> Result<Rgb, String> {
        let bad = || {
            format!(
                "`{text}` is not a colour; write red, green and blue from 0 to 255, such as `rgb(255,0,0)`"
            )
        };
        let inner = text
            .strip_prefix("rgb(")
            .and_then(|rest| rest.strip_suffix(')'))
            .ok_or_else(bad)?;
        let channels: Vec<u8> = inner
            .split(',')
            .map(|channel| channel.trim().parse::<u8>().map_err(|_| bad()))
            .collect::<Result<_, _>>()?;
        let [r, g, b] = channels[..] else {
            return Err(bad());
        };
        Ok(Rgb([r, g, b]))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "rgb({r},{g},{b})")
    }
}

impl TryFrom<String> for Rgb {
    type Error = String;

    fn try_from(text: String) -> Result<Rgb, String> {
        Rgb::parse(&text)
    }
}

impl From<Rgb> for String {
    fn from(rgb: Rgb) -> String {
        rgb.to_string()
    }
}

/// The text `serial contains` looks for. It is never empty, because `""` is in any serial.
/// Its wire form is the text, read by `parse`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Needle(String);

impl Needle {
    pub fn parse(text: &str) -> Result<Needle, String> {
        if text.is_empty() {
            return Err(
                "`serial contains` needs text, because `\"\"` is in any serial".to_string(),
            );
        }
        Ok(Needle(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Needle {
    type Error = String;

    fn try_from(text: String) -> Result<Needle, String> {
        Needle::parse(&text)
    }
}

impl From<Needle> for String {
    fn from(needle: Needle) -> String {
        needle.0
    }
}

/// A value in its SI unit, as written: `5mA` is 0.005 A, and `3.2e-7A` is 320 nA.
/// Its wire form is the text as written, read by `parse`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Amount {
    pub value: f64,
    pub unit: Unit,
    pub written: String,
}

impl TryFrom<String> for Amount {
    type Error = String;

    fn try_from(text: String) -> Result<Amount, String> {
        Amount::parse(&text)
    }
}

impl From<Amount> for String {
    fn from(amount: Amount) -> String {
        amount.written
    }
}

const PREFIXES: &[(&str, f64)] = &[
    ("p", 1e-12),
    ("n", 1e-9),
    ("u", 1e-6),
    ("µ", 1e-6),
    ("m", 1e-3),
    ("k", 1e3),
    ("M", 1e6),
];

impl Amount {
    pub fn parse(text: &str) -> Result<Amount, String> {
        let bad = || format!("`{text}` is not a value with a unit, such as `5mA` or `2.5V`");
        let split = text
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
            .ok_or_else(bad)?;
        let (number, suffix) = text.split_at(split);
        let number: f64 = number
            .parse()
            .ok()
            .filter(|n: &f64| n.is_finite())
            .ok_or_else(bad)?;
        let (scale, unit) = match Unit::parse(suffix) {
            Some(unit) => (1.0, unit),
            None => PREFIXES
                .iter()
                .find_map(|(p, scale)| Some((*scale, Unit::parse(suffix.strip_prefix(p)?)?)))
                .ok_or_else(bad)?,
        };
        Ok(Amount {
            value: number * scale,
            unit,
            written: text.to_string(),
        })
    }
}

impl Condition {
    /// Parses the words after `expect` or `step --until`.
    pub fn parse(words: &[String]) -> Result<Condition, String> {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["no", "diagnostics"] => Ok(Condition::NoDiagnostics),
            ["serial", "contains", text] => Ok(Condition::SerialContains(Needle::parse(text)?)),
            [r, first, ..] if r.starts_with('@') && first.starts_with("rgb(") => {
                let colour = words[1..].concat();
                Ok(Condition::Color(Ref::parse(r)?, Rgb::parse(&colour)?))
            }
            [r, word] if r.starts_with('@') => {
                let state = State::parse(word).ok_or_else(|| {
                    format!("`{word}` is not a state; use lit, dim, off, high, low or a colour such as rgb(255,0,0)")
                })?;
                Ok(Condition::State(Ref::parse(r)?, state))
            }
            [r, op, value] if r.starts_with('@') => {
                let op = Op::parse(op)
                    .ok_or_else(|| format!("`{op}` is not a comparison; use <, <=, > or >="))?;
                Ok(Condition::Compare(
                    Ref::parse(r)?,
                    op,
                    Amount::parse(value)?,
                ))
            }
            [r] if r.starts_with('@') => Err(format!(
                "`{r}` needs what to expect of it, such as `{r} lit` or `{r} rgb(255,0,0)`; a word that starts with # is a comment"
            )),
            [] => Err("a condition is missing".to_string()),
            _ => Err(format!(
                "`{}` is not a condition; write `@led1 lit`, `@led1.brightness > 2mA`, `@pixel rgb(255,0,0)`, `serial contains \"text\"` or `no diagnostics`",
                words.join(" ")
            )),
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Condition::State(r, state) => write!(f, "{r} {}", state.as_str()),
            Condition::Compare(r, op, amount) => {
                write!(f, "{r} {} {}", op.as_str(), amount.written)
            }
            Condition::Color(r, rgb) => write!(f, "{r} {rgb}"),
            Condition::SerialContains(text) => write!(f, "serial contains {:?}", text.as_str()),
            Condition::NoDiagnostics => f.write_str("no diagnostics"),
        }
    }
}
