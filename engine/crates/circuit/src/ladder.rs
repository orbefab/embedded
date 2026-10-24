//! The fallback ladder: plain Newton, then gmin stepping, then source stepping.
//!
//! `next` is the whole policy. It sees one rung and whether Newton converged
//! there, and names the next rung. Every rung that ends the ladder with
//! `Done` ran at the base conditions, so a fallback only finds a starting
//! point and never changes the circuit that is reported.

use crate::mna::GMIN;
use crate::newton::Conditions;

const GMIN_START: f64 = 1e-2;
const GMIN_FACTOR: f64 = 10.0;
const GMIN_FACTOR_FLOOR: f64 = 1.01;
const GMIN_RUNGS: u32 = 50;
const SOURCE_STEP: f64 = 0.1;
const SOURCE_STEP_FLOOR: f64 = 1e-4;
const SOURCE_RUNGS: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Rung {
    /// Nominal conditions from the initial iterate.
    Plain,
    /// `extra` siemens on every node that takes gmin, on top of GMIN. `good` is the last extra
    /// that converged. `tried` counts gmin rungs, this one included.
    Gmin {
        extra: f64,
        factor: f64,
        good: Option<f64>,
        tried: u32,
    },
    /// Every source at `scale`. `good` is the last scale that converged.
    Source {
        scale: f64,
        step: f64,
        good: Option<f64>,
        tried: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Next {
    Try(Rung),
    /// The rung just run converged at nominal conditions.
    Done,
    Exhausted,
}

impl Rung {
    /// `base` with this rung's gmin or source scale.
    pub(crate) fn conditions(self, base: Conditions<'_>) -> Conditions<'_> {
        match self {
            Rung::Plain => base,
            Rung::Gmin { extra, .. } => Conditions {
                gmin: base.gmin + extra,
                ..base
            },
            Rung::Source { scale, .. } => Conditions {
                source_scale: scale,
                ..base
            },
        }
    }

    /// Whether this rung starts from the last converged iterate rather than the initial one.
    pub(crate) fn warm(self) -> bool {
        match self {
            Rung::Plain => false,
            Rung::Gmin { good, .. } | Rung::Source { good, .. } => good.is_some(),
        }
    }
}

const GMIN_FIRST: Rung = Rung::Gmin {
    extra: GMIN_START,
    factor: GMIN_FACTOR,
    good: None,
    tried: 1,
};

const SOURCE_FIRST: Rung = Rung::Source {
    scale: 0.0,
    step: SOURCE_STEP,
    good: None,
    tried: 1,
};

pub(crate) fn next(rung: Rung, converged: bool) -> Next {
    match rung {
        Rung::Plain if converged => Next::Done,
        Rung::Plain => Next::Try(GMIN_FIRST),
        Rung::Gmin { extra, .. } if converged && extra == 0.0 => Next::Done,
        Rung::Gmin {
            extra,
            factor,
            good,
            tried,
        } => {
            // The nominal rung is one attempt. A refinement that lands on extra 0
            // would repeat it, so a failure goes to source stepping.
            if !converged && extra == 0.0 {
                return Next::Try(SOURCE_FIRST);
            }
            let (good, factor) = match (converged, good) {
                (true, _) => (extra, factor),
                (false, None) => return Next::Try(SOURCE_FIRST),
                (false, Some(good)) => (good, factor.sqrt()),
            };
            if factor < GMIN_FACTOR_FLOOR || tried == GMIN_RUNGS {
                return Next::Try(SOURCE_FIRST);
            }
            let lower = good / factor;
            let next_extra = if lower <= GMIN { 0.0 } else { lower };
            if !converged && next_extra == 0.0 {
                return Next::Try(SOURCE_FIRST);
            }
            Next::Try(Rung::Gmin {
                extra: next_extra,
                factor,
                good: Some(good),
                tried: tried + 1,
            })
        }
        Rung::Source { scale, .. } if converged && scale == 1.0 => Next::Done,
        Rung::Source {
            scale,
            step,
            good,
            tried,
        } => {
            let (good, step) = match (converged, good) {
                (true, None) => (scale, step),
                (true, Some(_)) => (scale, step * 2.0),
                (false, None) => return Next::Exhausted,
                (false, Some(good)) => (good, step / 4.0),
            };
            if step < SOURCE_STEP_FLOOR || tried == SOURCE_RUNGS {
                return Next::Exhausted;
            }
            Next::Try(Rung::Source {
                scale: (good + step).min(1.0),
                step,
                good: Some(good),
                tried: tried + 1,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tried(rung: Rung) -> Rung {
        match next(rung, true) {
            Next::Try(rung) => rung,
            other => panic!("{other:?} after {rung:?}"),
        }
    }

    /// The rung leaves every condition at nominal.
    fn nominal(rung: Rung) -> bool {
        let conditions = rung.conditions(Conditions::NOMINAL);
        let base = Conditions::NOMINAL;
        (conditions.gmin, conditions.source_scale, conditions.at)
            == (base.gmin, base.source_scale, base.at)
            && conditions.history == base.history
            && conditions.chips == base.chips
            && conditions.cores.is_empty()
    }

    fn failed(rung: Rung) -> Next {
        next(rung, false)
    }

    #[test]
    fn plain_newton_falls_back_to_gmin_stepping() {
        assert_eq!(next(Rung::Plain, true), Next::Done);
        assert_eq!(failed(Rung::Plain), Next::Try(GMIN_FIRST));
        assert_eq!(GMIN_FIRST.conditions(Conditions::NOMINAL).gmin, GMIN + 1e-2);
        assert!(nominal(Rung::Plain));
        assert!(!GMIN_FIRST.warm());
    }

    #[test]
    fn gmin_steps_down_by_decades_and_ends_at_nominal() {
        let mut rung = GMIN_FIRST;
        let mut extras = Vec::new();
        while let Rung::Gmin { extra, .. } = rung {
            extras.push(extra);
            if extra == 0.0 {
                break;
            }
            rung = tried(rung);
            assert!(rung.warm());
        }
        assert_eq!(extras.len(), 12, "{extras:?}");
        for pair in extras[..11].windows(2) {
            assert!((pair[0] / pair[1] - 10.0).abs() < 1e-9, "{extras:?}");
        }
        assert!(nominal(rung));
        assert_eq!(next(rung, true), Next::Done);
    }

    #[test]
    fn a_failed_gmin_rung_retries_closer_to_the_last_good_one() {
        let rung = tried(GMIN_FIRST);
        let Next::Try(retry) = failed(rung) else {
            panic!("gave up after one good rung");
        };
        let Rung::Gmin {
            extra,
            factor,
            good,
            tried,
        } = retry
        else {
            panic!("{retry:?}");
        };
        assert_eq!(good, Some(1e-2));
        assert_eq!(factor, 10f64.sqrt());
        assert_eq!(extra, 1e-2 / 10f64.sqrt());
        assert_eq!(tried, 3);
        assert!(retry.warm());
    }

    #[test]
    fn a_failed_nominal_gmin_rung_goes_to_source_stepping() {
        let nominal = Rung::Gmin {
            extra: 0.0,
            factor: 10.0,
            good: Some(1e-12),
            tried: 12,
        };
        assert_eq!(failed(nominal), Next::Try(SOURCE_FIRST));

        let snaps = Rung::Gmin {
            extra: 1e-11,
            factor: 10.0,
            good: Some(1e-12),
            tried: 11,
        };
        assert_eq!(failed(snaps), Next::Try(SOURCE_FIRST));
    }

    #[test]
    fn gmin_stepping_gives_up_to_source_stepping() {
        assert_eq!(failed(GMIN_FIRST), Next::Try(SOURCE_FIRST));
        let shrunk = Rung::Gmin {
            extra: 1e-3,
            factor: 1.019,
            good: Some(1.01e-3),
            tried: 9,
        };
        assert_eq!(failed(shrunk), Next::Try(SOURCE_FIRST));
        let spent = Rung::Gmin {
            extra: 1e-3,
            factor: 10.0,
            good: Some(1e-2),
            tried: 50,
        };
        assert_eq!(next(spent, true), Next::Try(SOURCE_FIRST));
        assert!(!SOURCE_FIRST.warm());
        assert_eq!(
            SOURCE_FIRST.conditions(Conditions::NOMINAL).source_scale,
            0.0
        );
        assert_eq!(SOURCE_FIRST.conditions(Conditions::NOMINAL).gmin, GMIN);
    }

    #[test]
    fn source_stepping_doubles_its_step_and_lands_on_one() {
        let mut rung = SOURCE_FIRST;
        let mut scales = Vec::new();
        while let Rung::Source { scale, .. } = rung {
            scales.push(scale);
            if scale == 1.0 {
                break;
            }
            rung = tried(rung);
        }
        assert_eq!(scales, [0.0, 0.1, 0.1 + 0.2, 0.1 + 0.2 + 0.4, 1.0]);
        assert!(nominal(rung));
        assert_eq!(next(rung, true), Next::Done);
    }

    #[test]
    fn a_failed_source_rung_quarters_the_step_from_the_last_good_scale() {
        let rung = Rung::Source {
            scale: 0.7,
            step: 0.4,
            good: Some(0.3),
            tried: 4,
        };
        assert_eq!(
            failed(rung),
            Next::Try(Rung::Source {
                scale: 0.4,
                step: 0.1,
                good: Some(0.3),
                tried: 5,
            })
        );
    }

    #[test]
    fn source_stepping_is_exhausted() {
        assert_eq!(failed(SOURCE_FIRST), Next::Exhausted);
        let tiny = Rung::Source {
            scale: 0.5003,
            step: 3e-4,
            good: Some(0.5),
            tried: 20,
        };
        assert_eq!(failed(tiny), Next::Exhausted);
        let spent = Rung::Source {
            scale: 0.5,
            step: 0.1,
            good: Some(0.4),
            tried: 100,
        };
        assert_eq!(next(spent, true), Next::Exhausted);
    }
}
