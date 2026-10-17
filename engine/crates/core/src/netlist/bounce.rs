//! Seeded contact bounce. Integer arithmetic only, so the reference script
//! reproduces it exactly. The constants are SplitMix64's and FNV-1a's.

use super::{Bounce, Edge, Schedule};
use crate::name::Name;
use crate::time::Time;

/// Every event with its bounce edges, ascending. `events` and `bounce` passed parse.
/// `window` is at least `2 * max_bounces * MIN_GAP_PS`.
///
/// Event `k` draws from `stream(seed, name, k)`: a count `n` in `1..=max_bounces`,
/// then `2n` values in `0..=slack` with `slack = window - 2n * MIN_GAP_PS`, sorted.
/// The `j`-th value plus `j * MIN_GAP_PS` is the offset of the `j`-th flip, so
/// flips are `MIN_GAP_PS` apart, the first is `MIN_GAP_PS` after the event, and
/// the last lands within `window`.
pub fn expand(seed: u64, name: &Name, events: &[Edge], bounce: Option<&Bounce>) -> Vec<Edge> {
    if let Some(bounce) = bounce {
        debug_assert!(bounce.window.value >= 2 * bounce.max_bounces.value * Schedule::MIN_GAP_PS);
    }
    let mut edges = Vec::new();
    for (index, event) in events.iter().enumerate() {
        edges.push(*event);
        let Some(bounce) = bounce else {
            continue;
        };
        let mut rng = stream(seed, name, index as u64);
        let flips = 2 * (1 + rng.below(bounce.max_bounces.value));
        let slack = bounce.window.value - flips * Schedule::MIN_GAP_PS;
        let mut draws: Vec<u64> = (0..flips).map(|_| rng.below(slack + 1)).collect();
        draws.sort_unstable();
        let mut contact = event.to;
        for (j, draw) in (1..).zip(draws) {
            contact = contact.flipped();
            let offset = draw + j * Schedule::MIN_GAP_PS;
            edges.push(Edge {
                at: Time::from_ps(event.at.as_ps() + offset),
                to: contact,
            });
        }
    }
    edges
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.state)
    }

    /// In `[0, n)`: the high half of the 128-bit product. `n > 0`.
    fn below(&mut self, n: u64) -> u64 {
        ((u128::from(self.next()) * u128::from(n)) >> 64) as u64
    }
}

fn mix(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn stream(seed: u64, name: &Name, index: u64) -> SplitMix64 {
    SplitMix64 {
        state: mix(mix(seed ^ fnv1a64(name.as_str().as_bytes())) ^ index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlist::{Contact, IntValue, Source};

    #[test]
    fn the_generators_match_the_reference_vectors() {
        let mut rng = SplitMix64 { state: 0 };
        assert_eq!(rng.next(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(fnv1a64(b"sw1"), 0x826a_de19_5d0a_12ba);
        let sw1 = Name::new("sw1").unwrap();
        assert_eq!(stream(7, &sw1, 0).state, 0xd65f_52ca_546d_d32e);
    }

    #[test]
    fn the_button_expands_to_the_reference_edges() {
        let events = [
            Edge {
                at: Time::from_ps(2_000_000_000),
                to: Contact::Closed,
            },
            Edge {
                at: Time::from_ps(12_000_000_000),
                to: Contact::Open,
            },
        ];
        let bounce = Bounce {
            window: IntValue {
                value: 1_000_000_000,
                source: Source::User,
            },
            max_bounces: IntValue {
                value: 4,
                source: Source::User,
            },
        };
        let edges = expand(7, &Name::new("sw1").unwrap(), &events, Some(&bounce));
        let got: Vec<(u64, Contact)> = edges.iter().map(|e| (e.at.as_ps(), e.to)).collect();
        use Contact::{Closed, Open};
        assert_eq!(
            got,
            [
                (2_000_000_000, Closed),
                (2_040_015_938, Open),
                (2_114_234_716, Closed),
                (2_665_597_584, Open),
                (2_854_176_182, Closed),
                (12_000_000_000, Open),
                (12_089_763_725, Closed),
                (12_337_840_633, Open),
                (12_576_192_418, Closed),
                (12_607_405_695, Open),
                (12_637_715_418, Closed),
                (12_691_361_990, Open),
                (12_720_009_883, Closed),
                (12_727_823_423, Open),
            ]
        );
    }

    #[test]
    fn a_full_window_packs_flips_exactly_one_gap_apart() {
        let events = [Edge {
            at: Time::from_ps(10_000),
            to: Contact::Closed,
        }];
        let bounce = Bounce {
            window: IntValue {
                value: 2 * Schedule::MIN_GAP_PS,
                source: Source::User,
            },
            max_bounces: IntValue {
                value: 1,
                source: Source::User,
            },
        };
        let edges = expand(0, &Name::new("s").unwrap(), &events, Some(&bounce));
        let times: Vec<u64> = edges.iter().map(|e| e.at.as_ps()).collect();
        assert_eq!(times, [10_000, 12_000, 14_000]);
        assert_eq!(edges[2].to, Contact::Closed);
    }

    #[test]
    fn without_bounce_the_events_are_the_edges() {
        let events = [Edge {
            at: Time::from_ps(5),
            to: Contact::Open,
        }];
        assert_eq!(
            expand(3, &Name::new("s").unwrap(), &events, None),
            events.to_vec()
        );
    }
}
