//! The start of a restart's solve. A chip's or a core's edge that happened before moves the
//! circuit about as it did then, so its restart starts from the landing plus the change the
//! restart made last time. Newton still converges to its own tolerance from there. A guess
//! that does not converge within [`GUESS_BUDGET`] iterations is dropped and the restart solves
//! from the landing, as one with no guess does.
//!
//! What it learns depends only on the accepted landings and their restarts, in order, so a
//! session advanced in any chunks guesses as one advanced at once.

use std::collections::BTreeMap;

use orbe_model::Drive;

use crate::active;
use crate::chip;
use crate::mna::{Layout, Solution};
use crate::newton::Iterate;

/// Newton's budget from a guess. A good guess converges in one iteration and a restart from the
/// landing takes about three, so a guess that needs more is no help.
pub(crate) const GUESS_BUDGET: u32 = 2;

/// The image changes kept. A run that meets more forgets them all and learns again.
const KEPT: usize = 256;

/// A guess is tried only when every row of the landing is within this share of its size, plus
/// [`NEAR_FLOORS`] times its Newton floor, of the landing the change was learned on. Farther
/// off, the circuit is in another state, and the old change says nothing about this one.
const NEAR_SHARE: f64 = 1e-2;
const NEAR_FLOORS: f64 = 1e6;

/// What the restarts of one image change taught: the landing before it and the change from
/// there to the restart's solve.
struct Learned {
    landing: Vec<f64>,
    change: Vec<f64>,
    junctions: Vec<f64>,
}

pub(crate) struct Predictor {
    /// The cores' image after the last restart, or at t = 0 before the first.
    image: Vec<u64>,
    /// The cores' image after this landing.
    next: Vec<u64>,
    /// The chips and the cores' image before this landing and after it.
    key: Vec<u64>,
    learned: BTreeMap<Vec<u64>, Learned>,
    #[cfg(test)]
    pub(crate) mode: Mode,
}

/// How a test sets the guesses: as learned, none, or every row a kilo-unit off and every
/// junction a volt off.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    On,
    Off,
    Spoiled,
}

impl Predictor {
    /// Knows nothing yet. `cores` are the cores at t = 0.
    pub(crate) fn new(cores: &[active::Core]) -> Predictor {
        let mut image = Vec::new();
        for core in cores {
            self::image(core, &mut image);
        }
        Predictor {
            image,
            next: Vec::new(),
            key: Vec::new(),
            learned: BTreeMap::new(),
            #[cfg(test)]
            mode: Mode::On,
        }
    }

    /// Keys the image change of a landing whose chips go from `before` to `after` and whose
    /// cores now hold `cores`. Call it once per restart, before [`Self::guess`].
    pub(crate) fn change(
        &mut self,
        before: &[chip::State],
        after: &[chip::State],
        cores: &[active::Core],
    ) {
        self.next.clear();
        for core in cores {
            image(core, &mut self.next);
        }
        self.key.clear();
        self.key.extend(before.iter().map(|state| state.code()));
        self.key.extend_from_slice(&self.image);
        self.key.extend(after.iter().map(|state| state.code()));
        self.key.extend_from_slice(&self.next);
    }

    /// The landing plus the change this image change made last time, when the landing is near
    /// the one it was learned on.
    pub(crate) fn guess(&self, layout: &Layout, landing: &Iterate) -> Option<Iterate> {
        #[cfg(test)]
        if self.mode == Mode::Off {
            return None;
        }
        let learned = self.learned.get(&self.key)?;
        let x = landing.solution.components();
        let near =
            x.iter()
                .zip(&learned.landing)
                .zip(layout.floors())
                .all(|((&now, &then), floor)| match floor {
                    Some(floor) => {
                        (now - then).abs()
                            <= NEAR_SHARE * now.abs().max(then.abs()) + NEAR_FLOORS * floor
                    }
                    None => true,
                });
        if !near {
            return None;
        }
        #[allow(unused_mut)]
        let mut solution: Vec<f64> = x.iter().zip(&learned.change).map(|(x, d)| x + d).collect();
        #[allow(unused_mut)]
        let mut junctions: Vec<f64> = (landing.junctions.iter())
            .zip(&learned.junctions)
            .map(|(j, d)| j + d)
            .collect();
        #[cfg(test)]
        if self.mode == Mode::Spoiled {
            solution.iter_mut().for_each(|x| *x += 1e3);
            junctions.iter_mut().for_each(|j| *j += 1.0);
        }
        Some(Iterate {
            solution: Solution::from(solution),
            junctions,
        })
    }

    /// Learns the restart from `landing` to `restart` under the image change [`Self::change`]
    /// keyed.
    pub(crate) fn learn(&mut self, landing: &Iterate, restart: &Iterate) {
        if !self.learned.contains_key(&self.key) && self.learned.len() >= KEPT {
            self.learned.clear();
        }
        let learned = self
            .learned
            .entry(self.key.clone())
            .or_insert_with(|| Learned {
                landing: Vec::new(),
                change: Vec::new(),
                junctions: Vec::new(),
            });
        let (x, r) = (landing.solution.components(), restart.solution.components());
        learned.landing.clear();
        learned.landing.extend_from_slice(x);
        learned.change.clear();
        learned.change.extend(x.iter().zip(r).map(|(x, r)| r - x));
        learned.junctions.clear();
        learned.junctions.extend(
            (landing.junctions.iter())
                .zip(&restart.junctions)
                .map(|(j, r)| r - j),
        );
    }

    /// The restart is over: its cores' image is the one before the next.
    pub(crate) fn restarted(&mut self) {
        std::mem::swap(&mut self.image, &mut self.next);
    }
}

/// Everything of `core`'s image the circuit stamps.
fn image(core: &active::Core, out: &mut Vec<u64>) {
    let drive = core.drive();
    out.push(drive.len() as u64);
    out.extend(drive.chunks(32).map(|chunk| {
        chunk.iter().enumerate().fold(0, |word, (k, drive)| {
            let bits = match drive {
                Drive::Off => 0,
                Drive::Low => 1,
                Drive::High => 2,
                Drive::PullUp => 3,
            };
            word | bits << (2 * k)
        })
    }));
    for flags in [core.pulled_down(), core.strong_pull_up()] {
        out.push(flags.len() as u64);
        out.extend(flags.chunks(64).map(|chunk| {
            (chunk.iter().enumerate()).fold(0, |word, (k, &on)| word | u64::from(on) << k)
        }));
    }
    let draw = core.draw();
    out.extend([
        u64::from(core.held()),
        draw.amps.to_bits(),
        draw.holds_above_v.to_bits(),
    ]);
}
