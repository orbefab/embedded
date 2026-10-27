//! Names and refs.
//!
//! ```text
//! ref      = "#" ( "net." name | name [ "." ( quantity | "pin." name | "logic." name ) ] )
//! quantity = "ib" | "ic" | "id" | "iin" | "iout" | "i" | "v" | "p"
//! name     = ASCII letter, then ASCII letters, digits or "_"; 1..=64 bytes
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Serialize, Serializer};

/// A validated identifier for a part, net, or pin.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(Box<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NameError {
    Empty,
    TooLong,
    BadChar,
    Reserved,
}

impl Name {
    pub const MAX_LEN: usize = 64;

    pub fn new(s: &str) -> Result<Name, NameError> {
        if s.is_empty() {
            return Err(NameError::Empty);
        }
        if s.len() > Self::MAX_LEN {
            return Err(NameError::TooLong);
        }
        let mut chars = s.chars();
        let Some(first) = chars.next() else {
            return Err(NameError::Empty);
        };
        if !first.is_ascii_alphabetic() {
            return Err(NameError::BadChar);
        }
        if chars.any(|c| !c.is_ascii_alphanumeric() && c != '_') {
            return Err(NameError::BadChar);
        }
        Ok(Name(s.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A per-part quantity. One sign rule for every two-terminal part.
/// `i` enters at the first pin, `v` is V(first) - V(second), and `p = v * i`
/// is absorbed power, so a source that delivers power reads negative.
/// A BJT's `ib` and `ic` enter at its base and collector. A MOSFET's `id` enters at its drain.
/// A linear regulator's `iin` enters at `in` and its `iout` enters at `out`.
/// `p` is the power entering at every pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quantity {
    Ib,
    Ic,
    Id,
    Iin,
    Iout,
    I,
    V,
    P,
}

impl Quantity {
    pub fn as_str(self) -> &'static str {
        match self {
            Quantity::Ib => "ib",
            Quantity::Ic => "ic",
            Quantity::Id => "id",
            Quantity::Iin => "iin",
            Quantity::Iout => "iout",
            Quantity::I => "i",
            Quantity::V => "v",
            Quantity::P => "p",
        }
    }
}

/// A stable name for something in a run.
///
/// Derived order is nets, then parts, then pins, then quantities, then logic signals.
/// Run-folder maps keyed by `Ref` serialize in that order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ref {
    Net(Name),
    Part(Name),
    Pin(Name, Name),
    Quantity(Name, Quantity),
    /// A chip's logic level, such as the level its comparator reads at an input pin.
    /// Not the physical pin, which is `Pin`.
    Logic(Name, Name),
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseRefError {
    pub text: String,
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ref::Net(name) => write!(f, "#net.{}", name.as_str()),
            Ref::Part(name) => write!(f, "#{}", name.as_str()),
            Ref::Pin(part, pin) => write!(f, "#{}.pin.{}", part.as_str(), pin.as_str()),
            Ref::Quantity(part, quantity) => {
                write!(f, "#{}.{}", part.as_str(), quantity.as_str())
            }
            Ref::Logic(part, signal) => write!(f, "#{}.logic.{}", part.as_str(), signal.as_str()),
        }
    }
}

impl FromStr for Ref {
    type Err = ParseRefError;

    fn from_str(s: &str) -> Result<Ref, ParseRefError> {
        let fail = || ParseRefError {
            text: s.to_string(),
        };
        let Some(body) = s.strip_prefix('#') else {
            return Err(fail());
        };
        if body.is_empty() {
            return Err(fail());
        }
        // A part may be named `net`. `net.` is a net only when the rest is a name.
        if let Some(rest) = body.strip_prefix("net.")
            && let Ok(name) = Name::new(rest)
        {
            return Ok(Ref::Net(name));
        }
        if let Some((part, pin)) = body.split_once(".pin.") {
            let part = Name::new(part).map_err(|_| fail())?;
            let pin = Name::new(pin).map_err(|_| fail())?;
            return Ok(Ref::Pin(part, pin));
        }
        if let Some((part, signal)) = body.split_once(".logic.") {
            let part = Name::new(part).map_err(|_| fail())?;
            let signal = Name::new(signal).map_err(|_| fail())?;
            return Ok(Ref::Logic(part, signal));
        }
        if let Some((part, quant)) = body.split_once('.') {
            let part = Name::new(part).map_err(|_| fail())?;
            let quantity = match quant {
                "ib" => Quantity::Ib,
                "ic" => Quantity::Ic,
                "id" => Quantity::Id,
                "iin" => Quantity::Iin,
                "iout" => Quantity::Iout,
                "i" => Quantity::I,
                "v" => Quantity::V,
                "p" => Quantity::P,
                _ => return Err(fail()),
            };
            return Ok(Ref::Quantity(part, quantity));
        }
        Ok(Ref::Part(Name::new(body).map_err(|_| fail())?))
    }
}

impl Serialize for Ref {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_identifiers_and_rejects_the_rest() {
        assert_eq!(Name::new("gnd").unwrap().as_str(), "gnd");
        assert_eq!(Name::new("r1").unwrap().as_str(), "r1");
        assert_eq!(Name::new("net_1").unwrap().as_str(), "net_1");
        assert!(Name::new("a".repeat(64).as_str()).is_ok());
        assert_eq!(Name::new("").unwrap_err(), NameError::Empty);
        assert_eq!(Name::new(&"a".repeat(65)).unwrap_err(), NameError::TooLong);
        assert_eq!(Name::new("1r").unwrap_err(), NameError::BadChar);
        assert_eq!(Name::new("_r").unwrap_err(), NameError::BadChar);
        assert_eq!(Name::new("r-1").unwrap_err(), NameError::BadChar);
        assert_eq!(Name::new("r.1").unwrap_err(), NameError::BadChar);
        assert!(Name::new("net").is_ok());
    }

    #[test]
    fn refs_round_trip() {
        for text in [
            "#net.vout",
            "#net.i",
            "#net.pin.c",
            "#r1",
            "#r1.pin.a",
            "#r1.pin.p",
            "#r1.i",
            "#r1.v",
            "#r1.p",
            "#led1.p",
            "#q1.ib",
            "#q1.ic",
            "#m1.id",
            "#u1.logic.out",
            "#logic.logic.pin",
            "#net.logic.out",
        ] {
            let parsed: Ref = text.parse().unwrap();
            assert_eq!(parsed.to_string(), text, "{text}");
            let again: Ref = parsed.to_string().parse().unwrap();
            assert_eq!(again, parsed);
        }
    }

    #[test]
    fn pin_p_is_not_power() {
        let pin: Ref = "#v1.pin.p".parse().unwrap();
        let power: Ref = "#v1.p".parse().unwrap();
        assert_ne!(pin, power);
        assert!(matches!(pin, Ref::Pin(_, _)));
        assert!(matches!(power, Ref::Quantity(_, Quantity::P)));
    }

    #[test]
    fn net_prefix_wins_over_a_part_named_net() {
        let net: Ref = "#net.vout".parse().unwrap();
        assert!(matches!(net, Ref::Net(_)));
        let part: Ref = "#net".parse().unwrap();
        assert!(matches!(part, Ref::Part(_)));
        let net_i: Ref = "#net.i".parse().unwrap();
        assert!(matches!(net_i, Ref::Net(_)));
        let pin: Ref = "#net.pin.c".parse().unwrap();
        assert!(matches!(pin, Ref::Pin(_, _)));
    }

    #[test]
    fn rejects_malformed_refs() {
        for text in [
            "",
            "r1",
            "#",
            "#net.",
            "#net.1vout",
            "#r1.pin",
            "#r1.pin.",
            "#r1.x",
            "#1r",
            "#r1.i.v",
            "#q1.ie",
            " #r1",
            "#r1.",
            "#u1.logic",
            "#u1.logic.",
            "#u1.logic.1out",
            "#u1.logic.pin.out",
        ] {
            assert!(text.parse::<Ref>().is_err(), "{text} should be rejected");
        }
    }

    #[test]
    fn a_logic_signal_is_not_the_pin_it_reads() {
        let logic: Ref = "#u1.logic.out".parse().unwrap();
        let pin: Ref = "#u1.pin.out".parse().unwrap();
        assert_ne!(logic, pin);
        assert!(matches!(logic, Ref::Logic(_, _)));
        let part: Ref = "#net.logic.out".parse().unwrap();
        assert!(
            matches!(part, Ref::Logic(_, _)),
            "a part may be named `net`"
        );
    }

    #[test]
    fn order_is_nets_then_parts_then_quantities() {
        let net: Ref = "#net.vout".parse().unwrap();
        let part: Ref = "#r1".parse().unwrap();
        let pin: Ref = "#r1.pin.a".parse().unwrap();
        let i: Ref = "#r1.i".parse().unwrap();
        let v: Ref = "#r1.v".parse().unwrap();
        let p: Ref = "#r1.p".parse().unwrap();
        assert!(net < part);
        assert!(part < pin);
        assert!(pin < i);
        assert!(i < v && v < p);
        let ib: Ref = "#r1.ib".parse().unwrap();
        let ic: Ref = "#r1.ic".parse().unwrap();
        assert!(
            pin < ib && ib < ic && ic < p,
            "a transistor's columns sort ib, ic, p"
        );
        let logic: Ref = "#a1.logic.out".parse().unwrap();
        assert!(p < logic, "logic signals sort after every quantity");
    }
}
