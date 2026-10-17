use std::collections::BTreeMap;

use sim_core::diagnostic::{Diagnostic, Excursion};
use sim_core::finite::Finite;
use sim_core::name::Ref;
use sim_core::netlist::{Netlist, RatingKind};
use sim_core::time::Time;

/// Every rated quantity of a netlist, checked against each accepted solution.
///
/// It reads the observation map, so a rating is checked against the number
/// `observations.json` writes.
pub(crate) struct Ratings {
    watches: Vec<Watch>,
}

struct Watch {
    kind: RatingKind,
    quantity: Ref,
    limit: f64,
    excursion: Option<Excursion>,
}

impl Ratings {
    pub(crate) fn new(netlist: &Netlist) -> Ratings {
        let mut watches = Vec::new();
        for (_, part) in netlist.parts() {
            for (kind, value) in part.model.ratings() {
                watches.push(Watch {
                    kind,
                    quantity: Ref::Quantity(part.name.clone(), kind.quantity()),
                    limit: value.si.get(),
                    excursion: None,
                });
            }
        }
        Ratings { watches }
    }

    /// Over means a magnitude above the limit. The first time over stays; the peak rises.
    pub(crate) fn sample(&mut self, at: Time, values: &BTreeMap<Ref, Finite>) {
        for watch in &mut self.watches {
            let magnitude = values[&watch.quantity].get().abs();
            if magnitude <= watch.limit {
                continue;
            }
            match &mut watch.excursion {
                None => {
                    watch.excursion = Some(Excursion {
                        quantity: watch.quantity.clone(),
                        limit: watch.limit,
                        peak: magnitude,
                        peak_at: at,
                        first: at,
                        over_ps: None,
                    });
                }
                Some(excursion) if magnitude > excursion.peak => {
                    excursion.peak = magnitude;
                    excursion.peak_at = at;
                }
                Some(_) => {}
            }
        }
    }

    /// One `rating.*` diagnostic per quantity that went over.
    pub(crate) fn finish(self) -> Vec<Diagnostic> {
        self.watches
            .into_iter()
            .filter_map(|watch| {
                let excursion = watch.excursion?;
                Some(match watch.kind {
                    RatingKind::Current => Diagnostic::RatingCurrent(excursion),
                    RatingKind::Power => Diagnostic::RatingPower(excursion),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn netlist() -> Netlist {
        sim_core::netlist::parse(
            br#"{
          "version": 0,
          "seed": 0,
          "analysis": { "op": {} },
          "ground": "gnd",
          "nets": ["gnd", "a"],
          "parts": [
            {
              "name": "d1",
              "kind": "diode",
              "pins": { "anode": "a", "cathode": "gnd" },
              "saturation_current": { "value": 1e-14, "unit": "A", "source": "user" },
              "emission_coefficient": { "value": 1.0, "unit": "1", "source": "user" },
              "series_resistance": { "value": 1.0, "unit": "ohm", "source": "user" },
              "ratings": {
                "current": { "value": 0.03, "unit": "A", "source": "user" },
                "power": null
              }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "a", "b": "gnd" },
              "resistance": { "value": 10.0, "unit": "ohm", "source": "user" },
              "ratings": { "power": null }
            }
          ]
        }"#,
        )
        .unwrap()
    }

    fn values(current: f64) -> BTreeMap<Ref, Finite> {
        let mut values = BTreeMap::new();
        values.insert("#d1.i".parse().unwrap(), Finite::new(current).unwrap());
        values
    }

    #[test]
    fn exactly_the_limit_is_not_over() {
        let mut ratings = Ratings::new(&netlist());
        ratings.sample(Time::ZERO, &values(0.03));
        ratings.sample(Time::from_ps(1), &values(-0.03));
        assert!(ratings.finish().is_empty());
    }

    #[test]
    fn a_reverse_current_counts_by_magnitude_and_the_peak_follows_later_samples() {
        let mut ratings = Ratings::new(&netlist());
        ratings.sample(Time::ZERO, &values(0.01));
        ratings.sample(Time::from_ps(10), &values(-0.04));
        ratings.sample(Time::from_ps(20), &values(0.05));
        ratings.sample(Time::from_ps(30), &values(0.045));
        let found = ratings.finish();
        assert_eq!(
            found,
            [Diagnostic::RatingCurrent(Excursion {
                quantity: "#d1.i".parse().unwrap(),
                limit: 0.03,
                peak: 0.05,
                peak_at: Time::from_ps(20),
                first: Time::from_ps(10),
                over_ps: None,
            })]
        );
    }
}
