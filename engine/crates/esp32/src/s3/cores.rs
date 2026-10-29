//! The two cores and the order they run in.
//!
//! The next step to run is the one with the lowest start cycle, and the PRO CPU's at a tie. A
//! core's key is its cycle while it may run. A core that may not run has no key: an APP CPU
//! never reset, one that `SYSTEM_CORE_1_CONTROL_0_REG` or the RTC stall holds, and a halted
//! core. Each slice runs one core up to the other's key, so a run to any limit runs a prefix of
//! that one order, and a run split at any cycles runs the same steps.
//!
//! A store that changes what a core may do sets a [`Steer`]. The storing core returns before
//! its next step, and the steer moves the other core before either runs on.
//!
//! A core that the bus stopped in a step to list an access runs its next step before the other
//! core runs any, whatever their cycles. That step is the stopped one again, unless an interrupt
//! falls due on its cycle, which the core then takes first. A refetch can have charged the
//! stopped step a cycle past the other core's key, and the other core's step would otherwise
//! land between the stop and its rerun. A core that the stopped step's own store holds keeps no
//! such turn. A stop at a fetch charges no cycle, so the order runs that core next already, and
//! a watchdog stage that falls due first runs out first. A stopped step charged onto the run's
//! limit keeps its turn into the next run, so a watchdog stage due on that cycle runs out
//! before it, as on one core.
//!
//! `CCOUNT` counts through a hold while the APP CPU's clock runs, and not while
//! `CLKGATE_EN` gates it. A reset sets it to 0.

use sim_xtensa::{Core, CoreId, Cycle, Sr, Stop};

use super::app::Steer;
use super::soc::{CoreBus, Soc, Step};

type Xtensa = Core<sim_xtensa::Esp32s3>;

#[derive(Clone, Debug)]
struct Lane {
    core: Xtensa,
    run: Run,
}

/// Where a core stands in the order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    /// Runs from its cycle.
    Ready,
    /// At a jump to itself that only an NMI leaves, with no source routed to its NMI line.
    Halted,
}

/// The APP CPU. The model knows no state of it before its first reset.
#[derive(Clone, Debug)]
enum App {
    Unstarted,
    Started(Box<Lane>),
}

#[derive(Clone, Debug)]
pub(crate) struct Cores {
    pro: Lane,
    app: App,
    /// The cycle through which every core has run everything, apart from the other core's steps
    /// that a stopped core's turn holds back.
    reached: Cycle,
    /// The core the bus stopped to list an access, which runs its next step before the other
    /// core runs, while it may run.
    rerun: Option<CoreId>,
}

impl Cores {
    /// The PRO CPU as `pro`, and the APP CPU not started.
    pub(crate) fn new(pro: Xtensa) -> Cores {
        Cores {
            reached: pro.cycle(),
            pro: Lane {
                core: pro,
                run: Run::Ready,
            },
            app: App::Unstarted,
            rerun: None,
        }
    }

    pub(crate) fn reached(&self) -> Cycle {
        self.reached
    }

    /// The core `id`, which has started.
    pub(crate) fn core(&self, id: CoreId) -> &Xtensa {
        &self.lane(id).expect("a started core").core
    }

    fn lane(&self, id: CoreId) -> Option<&Lane> {
        match (id, &self.app) {
            (CoreId::Pro, _) => Some(&self.pro),
            (CoreId::App, App::Started(lane)) => Some(lane),
            (CoreId::App, App::Unstarted) => None,
        }
    }

    fn lane_mut(&mut self, id: CoreId) -> Option<&mut Lane> {
        match (id, &mut self.app) {
            (CoreId::Pro, _) => Some(&mut self.pro),
            (CoreId::App, App::Started(lane)) => Some(lane),
            (CoreId::App, App::Unstarted) => None,
        }
    }

    /// The start of `id`'s next step, while it may run.
    fn key(&self, id: CoreId, soc: &Soc) -> Option<Cycle> {
        let lane = self.lane(id)?;
        let held = id == CoreId::App && !soc.app_runs();
        (lane.run == Run::Ready && !held).then(|| lane.core.cycle())
    }

    /// Runs the cores in order until every key reaches `until`, or a core stops for another
    /// reason, and returns that core and its stop. With no core that may run, the PRO CPU
    /// stops with [`Stop::Spin`] at the latest cycle any core ran to, and `reached` is `until`.
    pub(crate) fn run_until(&mut self, soc: &mut Soc, until: Cycle) -> (CoreId, Stop) {
        loop {
            let keys = [self.key(CoreId::Pro, soc), self.key(CoreId::App, soc)];
            let rerun = self
                .rerun
                .take()
                .and_then(|id| keys[id as usize].map(|key| (id, key)));
            let pick = match (rerun, keys) {
                (Some((stopped, _)), _) => stopped,
                (None, [Some(pro), Some(app)]) if app < pro => CoreId::App,
                (None, [None, Some(_)]) => CoreId::App,
                (None, [Some(_), _]) => CoreId::Pro,
                (None, [None, None]) => {
                    if let Some(app) = self.lane(CoreId::App).map(|lane| lane.core.cycle()) {
                        self.pro.core.skip_to(app);
                    }
                    self.reached = until;
                    return (CoreId::Pro, Stop::Spin);
                }
            };
            let other = keys[1 - pick as usize];
            let limit = if let Some((stopped, key)) = rerun {
                if key >= until {
                    self.rerun = Some(stopped);
                    self.reached = until;
                    return (stopped, Stop::Limit);
                }
                Cycle(key.0 + 1)
            } else {
                let reached = keys.into_iter().flatten().min().expect("a key");
                if reached >= until {
                    self.reached = reached;
                    return (pick, Stop::Limit);
                }
                other.map_or(until, |key| {
                    until.min(Cycle(key.0 + u64::from(pick == CoreId::Pro)))
                })
            };
            let step = Step {
                core: pick,
                start: Cycle(0),
                paired: matches!(self.app, App::Started(_)),
                other_sleeps: self
                    .lane(other_core(pick))
                    .is_some_and(|lane| lane.run == Run::Halted),
            };
            let lane = self.lane_mut(pick).expect("a key");
            let stop = lane.core.run_until(&mut CoreBus { soc, step }, limit);
            if let Some(steer) = soc.steer.take() {
                self.apply(pick, steer, soc);
            }
            match stop {
                Stop::Limit | Stop::Yield => {}
                Stop::Spin if self.key(other_core(pick), soc).is_some() => {
                    if !soc.routes_to_nmi(pick) {
                        self.lane_mut(pick).expect("a key").run = Run::Halted;
                    }
                }
                stop => {
                    if stop == Stop::Bus && (soc.first_touch.is_some() || soc.change_stop) {
                        self.rerun = Some(pick);
                    }
                    let keys = [self.key(CoreId::Pro, soc), self.key(CoreId::App, soc)];
                    self.reached = keys.into_iter().flatten().min().unwrap_or(until);
                    return (pick, stop);
                }
            }
        }
    }

    /// Applies the steer a store by `storer` set.
    fn apply(&mut self, storer: CoreId, steer: Steer, soc: &Soc) {
        match steer {
            Steer::Reset(at) => {
                self.app = App::Started(Box::new(Lane {
                    core: Core::reset(CoreId::App, at),
                    run: Run::Ready,
                }));
            }
            Steer::Run { at, clocked } => {
                if let App::Started(lane) = &mut self.app {
                    let ccount = lane.core.special(Sr::Ccount);
                    lane.core.skip_to(at);
                    if !clocked {
                        lane.core.set_special(Sr::Ccount, ccount);
                    }
                }
            }
            Steer::Wake(at) => {
                let other = other_core(storer);
                if soc.routes_to_nmi(other)
                    && let Some(lane) = self.lane_mut(other)
                    && lane.run == Run::Halted
                {
                    lane.core.skip_to(at);
                    lane.run = Run::Ready;
                }
            }
        }
    }
}

fn other_core(id: CoreId) -> CoreId {
    match id {
        CoreId::Pro => CoreId::App,
        CoreId::App => CoreId::Pro,
    }
}
