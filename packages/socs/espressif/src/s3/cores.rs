//! The two cores and the order they run in.
//!
//! The next step to run is the one with the lowest start cycle, and the PRO CPU's at a tie. A
//! core's key is its cycle while it may run, or its wake while it is parked. A core that may not
//! run has no key: an APP CPU never reset, one that `SYSTEM_CORE_1_CONTROL_0_REG` or the RTC
//! stall holds, and a halted core. Each slice runs one core up to the other's key, so a run to
//! any limit runs a prefix of that one order, and a run split at any cycles runs the same steps.
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
//! A core in `WAITI` parks until its wake, the first cycle at which a poll can latch what no
//! access changed. Each slice of the other core then runs up to that wake, not one step. A store
//! to a peripheral register other than a quiet SYSTIMER snapshot, a steer of the APP CPU, and
//! each return from a run put a parked core where it would stand had it polled after each step
//! of the other, so it runs every poll that can see a change at the cycle it would have run it. So no core is parked between runs,
//! and an input from the seam never meets one. A core in `WAITI` polls on the first cycle after
//! the input's tick, where a core that polled at each run's limit would read it. A change that
//! keeps a core parked across runs must rouse it at an input, with a test that pins its cycle.
//!
//! `CCOUNT` counts through a hold while the APP CPU's clock runs, and not while
//! `CLKGATE_EN` gates it. A reset sets it to 0.
//!
//! A store that makes an unstarted APP CPU runnable leaves it unstarted and records the cycle.
//! The run ends if a later store to `SYSTEM_CORE_1_CONTROL_0_REG` does not set `RESETING`, or if
//! [`APP_RESET_BOUND`] PRO CPU cycles pass with no reset. The bound is the model's, not a chip
//! figure. A jump to itself while that release is pending ends at the bound, not as a halt.

use orbe_xtensa::{Core, Cycle, Sr, Stop};

use super::Refused;
use super::app::Steer;
use super::soc::{CoreBus, Ending, Soc, Step};

/// Which of the S3's two CPUs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreId {
    Pro,
    App,
}

impl CoreId {
    /// What the CPU's core holds at reset: the mask ROM's reset vector and vector base, and its
    /// `PRID`, `0xCDCD` on the PRO CPU and `0xABAB` on the APP CPU. IDF v5.5.5,
    /// `components/xtensa/include/xtensa_context.h`.
    pub(crate) const fn reset(self) -> orbe_xtensa::Reset {
        orbe_xtensa::Reset {
            pc: 0x4000_0400,
            vecbase: 0x4000_0000,
            prid: match self {
                CoreId::Pro => 0xcdcd,
                CoreId::App => 0xabab,
            },
        }
    }
}

/// PRO CPU cycles an unstarted APP CPU may stay runnable before the run ends.
const APP_RESET_BOUND: u64 = 1_000;

type Xtensa = Core<orbe_xtensa::Esp32s3>;

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
    /// In `WAITI`, with no poll before `wake` that can latch what no access changed.
    Parked { wake: Cycle },
    /// At a jump to itself that only an NMI leaves, with no source routed to its NMI line.
    Halted,
}

impl Lane {
    /// Unparks a parked core at `at`.
    fn rouse(&mut self, at: Cycle) {
        if let Run::Parked { wake } = self.run {
            debug_assert!(at <= wake, "roused at {at:?} past its wake {wake:?}");
            self.core.skip_to(at);
            self.run = Run::Ready;
        }
    }
}

/// The APP CPU. The model knows no state of it before its first reset.
#[derive(Clone, Debug)]
enum App {
    /// Never started. `released` is the cycle a store made it runnable with no reset yet.
    Unstarted {
        released: Option<Cycle>,
    },
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
    /// How many more slices a test lets the cores run before it fails, so that a test whose run
    /// stops moving fails rather than hangs.
    #[cfg(test)]
    pub(crate) slices_left: u64,
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
            app: App::Unstarted { released: None },
            rerun: None,
            #[cfg(test)]
            slices_left: 1_000_000,
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
            (CoreId::App, App::Unstarted { .. }) => None,
        }
    }

    fn lane_mut(&mut self, id: CoreId) -> Option<&mut Lane> {
        match (id, &mut self.app) {
            (CoreId::Pro, _) => Some(&mut self.pro),
            (CoreId::App, App::Started(lane)) => Some(lane),
            (CoreId::App, App::Unstarted { .. }) => None,
        }
    }

    /// The cycle a store made the unstarted APP CPU runnable, when one has.
    fn app_released(&self) -> Option<Cycle> {
        match self.app {
            App::Unstarted { released } => released,
            App::Started(_) => None,
        }
    }

    /// Ends the run at the reset bound. The PRO CPU stands at that cycle, on the instruction it
    /// had reached.
    fn refuse_unreset(&mut self, soc: &mut Soc) {
        let at = self.app_released().expect("a release");
        self.pro.core.skip_to(Cycle(at.0 + APP_RESET_BOUND));
        let refused = Refused::Register("SYSTEM", "app_cpu_without_reset");
        soc.ending = Some(Ending::Refused(refused));
    }

    /// The start of `id`'s next step, or its wake while parked, while it may run.
    fn key(&self, id: CoreId, soc: &Soc) -> Option<Cycle> {
        let lane = self.lane(id)?;
        if id == CoreId::App && !soc.app_runs() {
            return None;
        }
        match lane.run {
            Run::Ready => Some(lane.core.cycle()),
            Run::Parked { wake } => Some(wake),
            Run::Halted => None,
        }
    }

    fn rouse(&mut self, at: Cycle) {
        for id in [CoreId::Pro, CoreId::App] {
            if let Some(lane) = self.lane_mut(id) {
                lane.rouse(at);
            }
        }
    }

    /// Whether a core is parked.
    pub(crate) fn parked(&self) -> bool {
        [CoreId::Pro, CoreId::App]
            .into_iter()
            .filter_map(|id| self.lane(id))
            .any(|lane| matches!(lane.run, Run::Parked { .. }))
    }

    /// Sets `reached` to the lowest key, or to `until` when no core may run.
    fn reach(&mut self, soc: &Soc, until: Cycle) {
        let keys = [self.key(CoreId::Pro, soc), self.key(CoreId::App, soc)];
        self.reached = keys.into_iter().flatten().min().unwrap_or(until);
    }

    /// Runs the cores in order until every key reaches `until`, or a core stops for another
    /// reason, and returns that core and its stop. With no core that may run, the PRO CPU
    /// stops with [`Stop::Spin`] at the latest cycle any core ran to, and `reached` is `until`.
    pub(crate) fn run_until(&mut self, soc: &mut Soc, until: Cycle) -> (CoreId, Stop) {
        soc.forget_events();
        loop {
            if self
                .app_released()
                .is_some_and(|at| self.pro.core.cycle() >= Cycle(at.0 + APP_RESET_BOUND))
            {
                self.refuse_unreset(soc);
                self.reach(soc, until);
                return (CoreId::Pro, Stop::Bus);
            }
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
            let mut limit = if let Some((stopped, key)) = rerun {
                if key >= until {
                    self.rerun = Some(stopped);
                    self.reached = until;
                    return (stopped, Stop::Limit);
                }
                Cycle(key.0 + 1)
            } else {
                let reached = keys.into_iter().flatten().min().expect("a key");
                if reached >= until {
                    self.rouse(until);
                    self.reach(soc, until);
                    return (pick, Stop::Limit);
                }
                other.map_or(until, |key| {
                    until.min(Cycle(key.0 + u64::from(pick == CoreId::Pro)))
                })
            };
            if pick == CoreId::Pro
                && let Some(at) = self.app_released()
            {
                limit = limit.min(Cycle(at.0 + APP_RESET_BOUND));
            }
            let step = Step {
                core: pick,
                start: Cycle(0),
                paired: matches!(self.app, App::Started(_)),
                app_released: self.app_released(),
                other_sleeps: self
                    .lane(other_core(pick))
                    .is_some_and(|lane| lane.run != Run::Ready),
            };
            #[cfg(test)]
            {
                self.slices_left = self.slices_left.checked_sub(1).expect("a slice left");
            }
            let key = keys[pick as usize].expect("a key");
            let lane = self.lane_mut(pick).expect("a key");
            lane.rouse(key);
            let mut bus = CoreBus::new(soc, step);
            let stop = lane.core.run_until(&mut bus, limit);
            let start = bus.last_start();
            if let Some(steer) = soc.steer.take() {
                self.apply(pick, steer, soc);
            }
            match stop {
                Stop::Limit => {
                    let lane = self.lane_mut(pick).expect("a key");
                    if let Some(wake) = lane.core.wake() {
                        lane.run = Run::Parked { wake };
                    }
                }
                Stop::Yield => {}
                Stop::Spin if self.key(other_core(pick), soc).is_some() => {
                    if !soc.routes_to_nmi(pick) {
                        self.lane_mut(pick).expect("a key").run = Run::Halted;
                    }
                }
                Stop::Spin if self.app_released().is_some() => {
                    self.refuse_unreset(soc);
                    self.reach(soc, until);
                    return (CoreId::Pro, Stop::Bus);
                }
                stop => {
                    if stop == Stop::Bus && (soc.first_touch.is_some() || soc.change_stop) {
                        self.rerun = Some(pick);
                    }
                    let other = other_core(pick);
                    if let Some(lane) = self.lane_mut(other) {
                        lane.rouse(Cycle(start.0 + bias(other)));
                    }
                    self.reach(soc, until);
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
                    core: Core::reset(CoreId::App.reset(), at),
                    run: Run::Ready,
                }));
            }
            Steer::Run { at, clocked } => match &mut self.app {
                App::Started(lane) => {
                    lane.rouse(at);
                    let ccount = lane.core.special(Sr::Ccount);
                    lane.core.skip_to(at);
                    if !clocked {
                        lane.core.set_special(Sr::Ccount, ccount);
                    }
                }
                App::Unstarted { released } if released.is_none() && soc.app_runs() => {
                    *released = Some(at);
                }
                App::Unstarted { .. } => {}
            },
            Steer::Wake(at) => {
                let other = other_core(storer);
                let nmi = soc.routes_to_nmi(other);
                let Some(lane) = self.lane_mut(other) else {
                    return;
                };
                match lane.run {
                    Run::Parked { .. } => lane.rouse(Cycle(at.0 + bias(other))),
                    Run::Halted if nmi => {
                        lane.core.skip_to(at);
                        lane.run = Run::Ready;
                    }
                    Run::Ready | Run::Halted => {}
                }
            }
        }
    }
}

/// How far past the start of the other core's step `id` would stand had it polled after that
/// step. The PRO CPU runs first on a tie, so it stands a cycle on.
fn bias(id: CoreId) -> u64 {
    u64::from(id == CoreId::Pro)
}

fn other_core(id: CoreId) -> CoreId {
    match id {
        CoreId::Pro => CoreId::App,
        CoreId::App => CoreId::Pro,
    }
}
