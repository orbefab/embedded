//! What the LED chains of a transient run showed.
//!
//! ```text
//! traces/leds.csv                  when the netlist has a ws2812 part
//!   t_ps,part,count,grb
//!   <latch ps>,<part name>,<led count>,<g r b of each led, lowercase hex, no separators>
//! ```
//!
//! A row is one latch: the chain after a low longer than the reset. `grb` is green, red, then
//! blue for LED 0 onward, six hex digits each. A chain that never latched leaves the header.

use std::fmt::Write;

use serde::Serialize;

use orbe_model::Time;

use crate::name::Name;

/// The colours of one chain a reader sees. `Session::leds` returns it, and each entry of
/// `observations.json` `leds` is one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chain {
    pub kind: &'static str,
    pub count: u16,
    /// Byte order in a latch. `grb` is green, then red, then blue.
    pub order: &'static str,
    /// One `[green, red, blue]` per LED, LED 0 first.
    pub pixels: Vec<[u8; 3]>,
}

/// One latch. `part` indexes the run's chains, in part order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Latch {
    pub at: Time,
    pub part: u16,
    pub pixels: Vec<[u8; 3]>,
}

/// A run's chains in part order, and the latches they wrote, ordered by time, then by part name.
#[derive(Debug, PartialEq)]
pub struct Leds {
    parts: Vec<(Name, Chain)>,
    rows: Vec<Latch>,
}

impl Leds {
    pub fn new(parts: Vec<(Name, Chain)>, rows: Vec<Latch>) -> Leds {
        Leds { parts, rows }
    }

    /// The bytes of `leds.csv`. The header, then one row per latch in order, each line ended
    /// by `\n`.
    pub fn csv(&self) -> Vec<u8> {
        let mut text = String::from("t_ps,part,count,grb\n");
        for row in &self.rows {
            let (name, chain) = &self.parts[usize::from(row.part)];
            writeln!(
                text,
                "{},{},{},{}",
                row.at.as_ps(),
                name.as_str(),
                chain.count,
                hex(&row.pixels)
            )
            .expect("a String takes every write");
        }
        text.into_bytes()
    }

    /// The `observations.json` `leds` object, by part name.
    pub fn observed(&self) -> std::collections::BTreeMap<String, Chain> {
        self.parts
            .iter()
            .map(|(name, chain)| (name.as_str().to_string(), chain.clone()))
            .collect()
    }
}

fn hex(pixels: &[[u8; 3]]) -> String {
    let mut out = String::with_capacity(pixels.len() * 6);
    for px in pixels {
        write!(out, "{:02x}{:02x}{:02x}", px[0], px[1], px[2]).expect("a String takes every write");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_latch_is_one_row_of_grb_hex() {
        let chain = Chain {
            kind: "ws2812",
            count: 2,
            order: "grb",
            pixels: vec![[1, 2, 3], [4, 5, 6]],
        };
        let leds = Leds::new(
            vec![(Name::new("ring").unwrap(), chain.clone())],
            vec![Latch {
                at: Time::from_ps(9),
                part: 0,
                pixels: chain.pixels.clone(),
            }],
        );
        assert_eq!(
            std::str::from_utf8(&leds.csv()).unwrap(),
            "t_ps,part,count,grb\n9,ring,2,010203040506\n"
        );
        assert_eq!(
            leds.observed().into_iter().next().unwrap().1.pixels,
            vec![[1, 2, 3], [4, 5, 6]]
        );
    }
}
