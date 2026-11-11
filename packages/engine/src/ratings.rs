use orbe_model::Time;

use crate::netlist::{Netlist, RatingKind};

use crate::diagnostic::{Diagnostic, Excursion};
use crate::finite::Finite;
use crate::name::Ref;

use crate::observe::Columns;

/// Every rated quantity of a netlist, checked against each accepted row.
///
/// It reads the observed row, so a rating is checked against the number the
/// run writes.
pub(crate) struct Ratings {
    timing: Timing,
    watches: Vec<Watch>,
    last: Option<Time>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Timing {
    /// One sample with no duration. `over_ps` is null.
    Point,
    /// Samples over a run. Each holds until the next.
    Run,
}

struct Watch {
    kind: RatingKind,
    quantity: Ref,
    column: usize,
    limit: f64,
    /// Whether the last sample was over.
    over: bool,
    excursion: Option<Excursion>,
}

impl Ratings {
    pub(crate) fn new(netlist: &Netlist, columns: &Columns, timing: Timing) -> Ratings {
        let mut watches = Vec::new();
        for (_, part) in netlist.parts() {
            for (kind, quantity, value) in part.model.ratings() {
                let quantity = Ref::Quantity(part.name.clone(), quantity);
                let column = columns
                    .index(&quantity)
                    .expect("every part quantity is a column");
                watches.push(Watch {
                    kind,
                    quantity,
                    column,
                    limit: value.si.get(),
                    over: false,
                    excursion: None,
                });
            }
        }
        Ratings {
            timing,
            watches,
            last: None,
        }
    }

    /// Over means a magnitude above the limit. The first time over stays; the peak rises.
    /// A sample that was over adds the time until this one to `over_ps`.
    pub(crate) fn sample(&mut self, at: Time, row: &[Finite]) {
        for watch in &mut self.watches {
            if watch.over
                && let (Some(last), Some(excursion)) = (self.last, &mut watch.excursion)
                && let Some(over_ps) = &mut excursion.over_ps
            {
                *over_ps += at.as_ps() - last.as_ps();
            }
            let magnitude = row[watch.column].get().abs();
            watch.over = magnitude > watch.limit;
            if !watch.over {
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
                        over_ps: match self.timing {
                            Timing::Point => None,
                            Timing::Run => Some(0),
                        },
                    });
                }
                Some(excursion) if magnitude > excursion.peak => {
                    excursion.peak = magnitude;
                    excursion.peak_at = at;
                }
                Some(_) => {}
            }
        }
        self.last = Some(at);
    }

    /// One `rating.*` diagnostic per quantity that went over, through the last sample.
    pub(crate) fn findings(&self) -> Vec<Diagnostic> {
        self.watches
            .iter()
            .filter_map(|watch| {
                let excursion = watch.excursion.clone()?;
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
        crate::netlist::parse(
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

    /// A row with `#d1.i` set and every other column 0.
    fn row(columns: &Columns, current: f64) -> Vec<Finite> {
        let mut row = vec![Finite::new(0.0).unwrap(); columns.refs().len()];
        row[columns.index(&"#d1.i".parse().unwrap()).unwrap()] = Finite::new(current).unwrap();
        row
    }

    fn ratings(timing: Timing) -> (Ratings, Columns) {
        let netlist = netlist();
        let columns = Columns::new(&netlist);
        (Ratings::new(&netlist, &columns, timing), columns)
    }

    #[test]
    fn exactly_the_limit_is_not_over() {
        let (mut ratings, columns) = ratings(Timing::Run);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.03));
        ratings.sample(Time::from_ps(1), &values(-0.03));
        assert!(ratings.findings().is_empty());
    }

    #[test]
    fn a_reverse_current_counts_by_magnitude_and_the_peak_follows_later_samples() {
        let (mut ratings, columns) = ratings(Timing::Point);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.01));
        ratings.sample(Time::from_ps(10), &values(-0.04));
        ratings.sample(Time::from_ps(20), &values(0.05));
        ratings.sample(Time::from_ps(30), &values(0.045));
        let found = ratings.findings();
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

    #[test]
    fn each_sample_over_the_limit_holds_until_the_next() {
        let (mut ratings, columns) = ratings(Timing::Run);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.01));
        ratings.sample(Time::from_ps(10), &values(0.04));
        ratings.sample(Time::from_ps(25), &values(0.05));
        ratings.sample(Time::from_ps(30), &values(0.02));
        ratings.sample(Time::from_ps(70), &values(0.035));
        ratings.sample(Time::from_ps(75), &values(0.031));
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding");
        };
        assert_eq!(excursion.first, Time::from_ps(10));
        assert_eq!(excursion.peak_at, Time::from_ps(25));
        assert_eq!(excursion.over_ps, Some(15 + 5 + 5));
    }

    #[test]
    fn a_run_over_only_at_its_last_row_is_over_for_no_time() {
        let (mut ratings, columns) = ratings(Timing::Run);
        ratings.sample(Time::ZERO, &row(&columns, 0.0));
        ratings.sample(Time::from_ps(9), &row(&columns, 0.5));
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding");
        };
        assert_eq!(excursion.over_ps, Some(0));
        assert_eq!(excursion.first, Time::from_ps(9));
    }
}
