//! A test-only active chip: a script of drives and samples on one output pin, `out`, and one
//! input pin, `in`. A drive at 0 is its power-on drive.

use std::sync::Arc;

use sim_core::netlist::NetId;
use sim_core::time::Time;

use super::{Chip, Drive, Output, Stage, Stop};
use crate::mna::Node;
use crate::watch::{Signal, Watch};

/// What the script does at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Op {
    Drive(Drive),
    Sample(Sample),
}

/// Sample `in`. `after` ps later drive `out` high if it read above `threshold`, otherwise low.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    pub(crate) after: u64,
    pub(crate) threshold: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Core {
    script: Arc<[(Time, Op)]>,
    /// `out` follows `in`'s level this long after it takes it. `None` ignores `in`.
    echo: Option<u64>,
    /// Drives `out` at a read itself, which breaks the contract, so a test sees the seam catch it.
    eager: bool,
    /// Everything up to here is done.
    now: Time,
    /// The next op.
    cursor: usize,
    /// Drives it set itself, by time, in the order it set them.
    pending: Vec<(Time, Drive)>,
    sampling: Option<Sample>,
    drive: [Drive; 1],
}

pub(crate) fn chip(
    (vcc, gnd, out, input): (NetId, NetId, NetId, NetId),
    script: Vec<(Time, Op)>,
    echo: Option<u64>,
) -> Chip {
    let stage = Stage {
        drop: 0.0,
        resistance: 25.0,
    };
    let v_in = Signal {
        terms: vec![(Node::from(input), 1.0)],
        offset: 0.0,
    };
    Chip {
        part: None,
        outputs: vec![Output {
            pin: out,
            vcc,
            gnd,
            high: stage,
            low: stage,
            pull_up: None,
            leakage: None,
        }],
        watches: vec![Watch::comparator(Signal {
            offset: 2.5,
            ..v_in.clone()
        })],
        samples: vec![v_in],
        core: super::Core::Script(Core::new(script, echo)),
    }
}

impl Core {
    fn new(script: Vec<(Time, Op)>, echo: Option<u64>) -> Core {
        assert!(script.is_sorted_by_key(|&(at, _)| at));
        let mut core = Core {
            script: script.into(),
            echo,
            now: Time::ZERO,
            cursor: 0,
            pending: Vec::new(),
            sampling: None,
            eager: false,
            drive: [Drive::Off],
        };
        while let Some(&(Time::ZERO, op)) = core.script.get(core.cursor) {
            let Op::Drive(drive) = op else {
                panic!("a sample at 0 has no step to land on");
            };
            core.drive = [drive];
            core.cursor += 1;
        }
        core
    }

    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        assert!(limit >= self.now, "a core runs only past where it reached");
        let scripted = self.script.get(self.cursor).map(|&(at, _)| at);
        let pending = self.pending.iter().map(|&(at, _)| at).min();
        let Some(at) = scripted
            .into_iter()
            .chain(pending)
            .min()
            .filter(|&at| at <= limit)
        else {
            self.now = limit;
            return None;
        };
        self.now = at;
        while let Some(&(when, op)) = self.script.get(self.cursor)
            && when == at
        {
            match op {
                Op::Drive(drive) => self.drive = [drive],
                Op::Sample(sample) => self.sampling = Some(sample),
            }
            self.cursor += 1;
        }
        while let Some(k) = self.pending.iter().position(|&(when, _)| when == at) {
            self.drive = [self.pending.remove(k).1];
        }
        Some(Stop {
            at,
            sample: self.sampling.is_some(),
        })
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        &self.drive
    }

    pub(crate) fn sample(&mut self, at: Time, volts: &[f64]) {
        assert_eq!(at, self.now, "a sample arrives where the core is");
        let Sample { after, threshold } = self.sampling.take().expect("a stop named the sample");
        let drive = if volts[0] > threshold {
            Drive::High
        } else {
            Drive::Low
        };
        self.act(at, after, drive);
    }

    pub(crate) fn input(&mut self, at: Time, levels: &[bool]) {
        assert_eq!(at, self.now, "an input arrives where the core is");
        if let Some(echo) = self.echo {
            let drive = if levels[0] { Drive::High } else { Drive::Low };
            self.act(at, echo, drive);
        }
    }

    fn act(&mut self, at: Time, after: u64, drive: Drive) {
        if self.eager {
            self.drive = [drive];
        } else {
            self.pending
                .push((Time::from_ps(at.as_ps() + after), drive));
        }
    }
}

pub(crate) fn eager(mut chip: Chip) -> Chip {
    let super::Core::Script(core) = &mut chip.core else {
        unreachable!("a script chip");
    };
    core.eager = true;
    chip
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> Core {
        let script = vec![
            (Time::from_ps(10), Op::Drive(Drive::High)),
            (
                Time::from_ps(20),
                Op::Sample(Sample {
                    after: 5,
                    threshold: 1.0,
                }),
            ),
            (Time::from_ps(40), Op::Drive(Drive::Off)),
        ];
        Core::new(script, None)
    }

    #[test]
    fn a_run_split_anywhere_is_the_same_run() {
        let whole = {
            let mut core = core();
            let mut stops = Vec::new();
            while let Some(stop) = core.run_until(Time::from_ps(50)) {
                stops.push(stop);
                if stop.sample {
                    core.sample(stop.at, &[2.0]);
                }
            }
            (core, stops)
        };
        for split in 0..50 {
            let mut core = core();
            let mut stops = Vec::new();
            for limit in [split, 50] {
                while let Some(stop) = core.run_until(Time::from_ps(limit)) {
                    stops.push(stop);
                    if stop.sample {
                        core.sample(stop.at, &[2.0]);
                    }
                }
            }
            assert_eq!((core, stops), whole, "split at {split} ps");
        }
    }
}
