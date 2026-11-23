use std::collections::VecDeque;
use std::num::NonZeroU64;

use orbe_model::{Drive, Time};

use crate::active::{self, Output, Stage};
use crate::netlist::{Model, NetId, Netlist, RatingKind};

use crate::diagnostic::{Diagnostic, Excursion, VoltageExcursion, exceeds};
use crate::finite::Finite;
use crate::name::{Name, Ref};

use crate::observe::Columns;

/// Every rated quantity of a netlist, checked against each accepted row.
///
/// It reads the observed row, so a part's rating is checked against the number the run
/// writes. A chip's pin ratings read the row's net voltages and the drive its core had in
/// force for that row, and work out each pin's current the way `active::stamp` stamps it.
///
/// A DC current rating, a chip pin's, a chip supply's or a supply source's, bounds the heat
/// the current makes, which follows its square over time. So it is checked on the current's
/// RMS over the last [`DC_WINDOW_PS`], not on each sample: the pulse that charges a
/// capacitor through a pin stays under it, a PWM load counts by the RMS of its period, and a
/// load that holds the current over its rating for a fraction of the window goes over.
///
/// A power rating is the heat the part can shed, so it uses that same window on the average
/// of the power's magnitude. A switching edge that spends its rating for a few hundred
/// nanoseconds stays under it, and a load held over the rating crosses it within the window.
///
/// Each window starts at zero, as a cold part does. A solve that holds each capacitor at a
/// voltage is never a sample: the t = 0 solve, and the restart solve at an edge. A capacitor
/// held there draws whatever its hold takes, which is no current the circuit carries. The first
/// accepted step is the first sample. Each sample holds until the next, except after an edge,
/// where the next sample holds back to the edge, [`Self::restarted`]. A windowed rating whose
/// last sample is over is over now, [`Self::over_now`], before its window crosses.
#[derive(Clone)]
pub(crate) struct Ratings {
    timing: Timing,
    watches: Vec<Watch>,
    windows: Vec<Window>,
    chips: Vec<Gauge>,
    last: Option<Time>,
    /// The last sample was a step that restarted, so the next sample holds back to it.
    restarted: bool,
}

/// The window a DC current rating averages over, 10.2 ms: five periods of the slowest PWM an
/// Arduino core makes by default, 490.2 Hz (16 MHz / (2 * 64 * 255), 2.04 ms), and ten of
/// the 976.6 Hz one, within 0.4%. Judging a PWM load by its RMS assumes the pin heats slower
/// than one period, so the window holds whole periods and the RMS it reports for a steady PWM
/// is the period's. A pulse longer than the window fills it and counts at its full value. A
/// pin that holds a capacitor's charging pulse, `I0^2 * tau / 2` of `i^2 * t`, stays well
/// under it, and a steady load over the rating crosses it within a fraction of it.
pub(crate) const DC_WINDOW_PS: u64 = 10_200_000_000;

/// The span a windowed rating also averages over to say whether it is over now, 2.04 ms: one
/// period of the 490.2 Hz PWM. A PWM load whose RMS is under the rating stays under it, though
/// each pulse is over, and so does a switching spike of a few hundred nanoseconds. A load held
/// over the rating crosses it within the span, or at once while the run is younger than it.
pub(crate) const RECENT_PS: u64 = DC_WINDOW_PS / 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Timing {
    /// One sample with no duration. `over_ps` is null.
    Point,
    /// Samples over a run. Each holds until the next.
    Run,
}

/// A chip's ratings on its package pins, which its adapter builds from its part.
pub(crate) struct PinRatings {
    /// Each output's package pin, in the order of the chip's outputs.
    pub(crate) outputs: Vec<&'static str>,
    /// Ampere, through any one output.
    pub(crate) pin_current: Option<f64>,
    /// Ampere, through the supply pin and through the ground pin. Every output draws its high
    /// side, its pull-up and its leakage from the one and sinks its low side and its leakage
    /// into the other, and the chip's own draw passes through both.
    pub(crate) supply_current: Option<f64>,
    /// The supply pin and its net.
    pub(crate) vcc: (&'static str, NetId),
    /// The ground pin and its net. Every window is over it.
    pub(crate) gnd: (&'static str, NetId),
    pub(crate) windows: Vec<PinWindow>,
}

/// The voltage a pin may take over the chip's ground.
pub(crate) struct PinWindow {
    pub(crate) pin: &'static str,
    pub(crate) net: NetId,
    /// Volt below ground, or `None` for no floor.
    pub(crate) below: Option<f64>,
    pub(crate) above: Option<Ceiling>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Ceiling {
    /// Volt above the chip's supply pin.
    OverSupply(f64),
    /// Volt above the chip's ground.
    Fixed(f64),
}

#[derive(Clone)]
struct Watch {
    kind: RatingKind,
    quantity: Ref,
    read: Read,
    limit: f64,
    /// `Some` when the rating is checked over [`DC_WINDOW_PS`]. A DC current stores squares
    /// and reads the RMS. A power rating stores the magnitude and reads the mean.
    heat: Option<Heat>,
    /// Whether the last reading was over.
    over: bool,
    excursion: Option<Excursion>,
    /// The last sample's own magnitude.
    last: f64,
    /// The first of the samples that have each been over, through the last one.
    since: Option<Time>,
}

/// A windowed rating's two averages of the same samples.
#[derive(Clone)]
struct Heat {
    /// Over [`DC_WINDOW_PS`]. It warns when it crosses.
    window: Square,
    /// Over [`RECENT_PS`]. With the last sample over, it says the rating is over now.
    recent: Square,
}

/// What a window folds each sample into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reduce {
    /// Store the square and report the root of the mean. Heat of a current.
    Rms,
    /// Store the sample and report the mean. Heat of a power.
    Mean,
}

/// A quantity over the last `length` ps, each sample held until the next. Before the first
/// sample the window holds 0, so it starts cold. The stored sample is the square for
/// [`Reduce::Rms`] and the magnitude for [`Reduce::Mean`].
#[derive(Clone)]
struct Square {
    reduce: Reduce,
    length: i64,
    /// Spans `(from_ps, to_ps, sample)`, oldest first, which tile the window from the first
    /// sample on.
    spans: VecDeque<(i64, i64, f64)>,
    /// `sample * t` over the spans.
    sum: f64,
    /// The last sample's time and stored value.
    held: Option<(i64, f64)>,
    /// The first sample's time.
    origin: Option<i64>,
}

/// Where a watch reads its value.
#[derive(Clone, Copy)]
enum Read {
    /// A column of the row.
    Column(usize),
    /// Entry `k` of a gauge's currents.
    Gauge { gauge: usize, k: usize },
}

/// One rated chip's currents, worked out from each row.
#[derive(Clone)]
struct Gauge {
    /// Its index among the circuit's active chips, which is its core's index.
    active: usize,
    outputs: Vec<Pin>,
    /// The columns of its supply and ground nets and its own draw.
    supply: Option<(usize, usize, active::Supply)>,
    /// Each output's current out of its pin, then the current into the supply pin, then the
    /// current out of the ground pin. Set by the last sample.
    currents: Vec<f64>,
}

/// An output's stage and the columns of its nets.
#[derive(Clone)]
struct Pin {
    pin: usize,
    vcc: usize,
    gnd: usize,
    high: Stage,
    low: Stage,
    pull_up: Option<f64>,
    pull_down: Option<f64>,
    leakage: Option<f64>,
}

#[derive(Clone)]
struct Window {
    pin: Ref,
    column: usize,
    vcc: usize,
    gnd: usize,
    below: Option<f64>,
    above: Option<Ceiling>,
    /// How far past the window the excursion's peak was.
    worst: f64,
    over: bool,
    excursion: Option<VoltageExcursion>,
}

impl Ratings {
    pub(crate) fn new(
        netlist: &Netlist,
        actives: &[active::Chip],
        columns: &Columns,
        timing: Timing,
    ) -> Ratings {
        let mut watches = Vec::new();
        for (_, part) in netlist.parts() {
            // A supply source's current rating is the most it delivers steadily. Every power
            // rating, on every kind, is the heat the part sheds, so it takes the same window.
            let supply = matches!(part.model, Model::VoltageSource(_));
            for (kind, quantity, value) in part.model.ratings() {
                let quantity = Ref::Quantity(part.name.clone(), quantity);
                let column = columns
                    .index(&quantity)
                    .expect("every part quantity is a column");
                let reduce = match kind {
                    RatingKind::Power => Some(Reduce::Mean),
                    RatingKind::Current if supply => Some(Reduce::Rms),
                    RatingKind::Current => None,
                };
                watches.push(Watch::new(
                    kind,
                    quantity,
                    Read::Column(column),
                    value.si.get(),
                    reduce,
                ));
            }
        }
        let mut windows = Vec::new();
        let mut chips = Vec::new();
        for (active, chip) in actives.iter().enumerate() {
            let Some(id) = chip.part else { continue };
            let part = netlist.part(id);
            let Model::Chip(model) = &part.model else {
                continue;
            };
            let Some(rated) = crate::chip::pin_ratings(model) else {
                continue;
            };
            debug_assert_eq!(rated.outputs.len(), chip.outputs.len());
            let pin =
                |name: &str| Ref::Pin(part.name.clone(), Name::new(name).expect("a pin name"));
            let net = |id: NetId| columns.net(id);
            let gauge = chips.len();
            if let Some(limit) = rated.pin_current {
                for (k, &name) in rated.outputs.iter().enumerate() {
                    watches.push(Watch::new(
                        RatingKind::Current,
                        pin(name),
                        Read::Gauge { gauge, k },
                        limit,
                        Some(Reduce::Rms),
                    ));
                }
            }
            let outputs = chip.outputs.len();
            if let Some(limit) = rated.supply_current {
                for (k, name) in [(outputs, rated.vcc.0), (outputs + 1, rated.gnd.0)] {
                    watches.push(Watch::new(
                        RatingKind::Current,
                        pin(name),
                        Read::Gauge { gauge, k },
                        limit,
                        Some(Reduce::Rms),
                    ));
                }
            }
            for window in &rated.windows {
                windows.push(Window {
                    pin: pin(window.pin),
                    column: net(window.net),
                    vcc: net(rated.vcc.1),
                    gnd: net(rated.gnd.1),
                    below: window.below,
                    above: window.above,
                    worst: 0.0,
                    over: false,
                    excursion: None,
                });
            }
            chips.push(Gauge {
                active,
                outputs: chip
                    .outputs
                    .iter()
                    .map(|output| Pin::new(output, columns))
                    .collect(),
                supply: chip
                    .supply
                    .map(|supply| (columns.net(supply.vcc), columns.net(supply.gnd), supply)),
                currents: vec![0.0; outputs + 2],
            });
        }
        // An input above `vcc - input_headroom` is the same window as a chip pin above its
        // ceiling. The differential stage does not read the field. Zero rates no ceiling:
        // the law examples use that, and a step can carry a node a little past the supply.
        for (_, part) in netlist.parts() {
            let Model::OpAmp(op) = &part.model else {
                continue;
            };
            if op.input_headroom.si.get() == 0.0 {
                continue;
            }
            let pin =
                |name: &str| Ref::Pin(part.name.clone(), Name::new(name).expect("a pin name"));
            let (inputs, vcc, gnd) = op.common_mode();
            for (name, net) in inputs {
                windows.push(Window {
                    pin: pin(name),
                    column: columns.net(net),
                    vcc: columns.net(vcc),
                    gnd: columns.net(gnd),
                    below: None,
                    above: Some(Ceiling::OverSupply(-op.input_headroom.si.get())),
                    worst: 0.0,
                    over: false,
                    excursion: None,
                });
            }
        }
        // A polarized capacitor and a regulator input use the same window. `vcc` is unread
        // when `above` is `None`. Pin `b`, and the regulator's `gnd`, are the window's ground.
        for (_, part) in netlist.parts() {
            let (pin_name, column, ground, below) = match &part.model {
                Model::Capacitor(cap) => {
                    let Some(limit) = &cap.reverse_voltage else {
                        continue;
                    };
                    (
                        "a",
                        columns.net(cap.pins[0]),
                        columns.net(cap.pins[1]),
                        limit.si.get(),
                    )
                }
                Model::LinearRegulator(reg) => (
                    "in",
                    columns.net(reg.pins[0]),
                    columns.net(reg.pins[2]),
                    crate::chip::regulator::INPUT_FLOOR_V,
                ),
                _ => continue,
            };
            windows.push(Window {
                pin: Ref::Pin(part.name.clone(), Name::new(pin_name).expect("a pin name")),
                column,
                vcc: ground,
                gnd: ground,
                below: Some(below),
                above: None,
                worst: 0.0,
                over: false,
                excursion: None,
            });
        }
        Ratings {
            timing,
            watches,
            windows,
            chips,
            last: None,
            restarted: false,
        }
    }

    /// The step sampled last restarted at an edge. Its row holds the state before the edge,
    /// and the restart solve holds each capacitor, so neither is the state after it. The next
    /// sample is, and it holds back to the edge in each window.
    pub(crate) fn restarted(&mut self) {
        self.restarted = true;
    }

    /// Over means a magnitude above the limit, the RMS over the window for a DC current, the
    /// average over the window for a power rating, or a voltage outside its window. A magnitude
    /// is over when it is over after both are rounded to 3 significant digits. The first time
    /// over stays; the peak rises. A sample that was over adds the time until this one to
    /// `over_ps`. `cores` are the active chips' cores whose drive was in force for `row`. An
    /// operating point, [`Timing::Point`], has no duration, so its DC value fills each window.
    pub(crate) fn sample(&mut self, at: Time, row: &[Finite], cores: &[active::Core]) {
        for gauge in &mut self.chips {
            gauge.read(row, &cores[gauge.active]);
        }
        let elapsed = |over: bool, over_ps: Option<&mut u64>, last: Option<Time>| {
            if over && let (Some(last), Some(over_ps)) = (last, over_ps) {
                *over_ps += at.as_ps() - last.as_ps();
            }
        };
        let back = self.restarted.then_some(self.last).flatten();
        for watch in &mut self.watches {
            elapsed(
                watch.over,
                watch.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
                self.last,
            );
            let sample = magnitude(watch, &self.chips, row);
            let reading = match (&mut watch.heat, self.timing) {
                (Some(heat), Timing::Run) => {
                    heat.recent.take(at, sample, back.is_some());
                    heat.window.take(at, sample, back.is_some())
                }
                _ => sample,
            };
            watch.last = sample;
            let from = back.filter(|_| watch.heat.is_some()).unwrap_or(at);
            watch.since = exceeds(sample, watch.limit).then(|| watch.since.unwrap_or(from));
            watch.over = exceeds(reading, watch.limit);
            if !watch.over {
                continue;
            }
            match &mut watch.excursion {
                None => {
                    watch.excursion = Some(Excursion {
                        quantity: watch.quantity.clone(),
                        limit: watch.limit,
                        peak: reading,
                        peak_at: at,
                        first: at,
                        over_ps: self.timing.over_ps(),
                        window_ps: watch.heat.as_ref().map(|_| DC_WINDOW_PS),
                        sample,
                        held_ps: None,
                    });
                }
                Some(excursion) if reading > excursion.peak => {
                    excursion.peak = reading;
                    excursion.peak_at = at;
                    excursion.sample = sample;
                }
                Some(_) => {}
            }
        }
        for window in &mut self.windows {
            elapsed(
                window.over,
                window.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
                self.last,
            );
            let ground = row[window.gnd].get();
            let v = row[window.column].get() - ground;
            let low = window.below.map(|margin| -margin);
            let high = window.above.map(|ceiling| match ceiling {
                Ceiling::OverSupply(margin) => row[window.vcc].get() - ground + margin,
                Ceiling::Fixed(volts) => volts,
            });
            let past = low
                .map_or(f64::NEG_INFINITY, |low| low - v)
                .max(high.map_or(f64::NEG_INFINITY, |high| v - high));
            window.over = past > 0.0;
            if !window.over {
                continue;
            }
            let reading = VoltageExcursion {
                pin: window.pin.clone(),
                low,
                high,
                peak: v,
                peak_at: at,
                first: at,
                over_ps: self.timing.over_ps(),
            };
            match &mut window.excursion {
                None => {
                    window.worst = past;
                    window.excursion = Some(reading);
                }
                Some(excursion) if past > window.worst => {
                    window.worst = past;
                    excursion.low = reading.low;
                    excursion.high = reading.high;
                    excursion.peak = v;
                    excursion.peak_at = at;
                }
                Some(_) => {}
            }
        }
        self.last = Some(at);
        self.restarted = false;
    }

    /// One `rating.*` diagnostic per quantity that went over, through the last sample.
    pub(crate) fn findings(&self) -> Vec<Diagnostic> {
        let currents = self.watches.iter().filter_map(|watch| {
            let excursion = watch.excursion.clone()?;
            Some(match watch.kind {
                RatingKind::Current => Diagnostic::RatingCurrent(excursion),
                RatingKind::Power => Diagnostic::RatingPower(excursion),
            })
        });
        let voltages = self
            .windows
            .iter()
            .filter_map(|window| Some(Diagnostic::RatingVoltage(window.excursion.clone()?)));
        currents.chain(voltages).collect()
    }

    /// Each windowed rating over now, through the last sample: that sample is over the rating,
    /// the average over [`RECENT_PS`] is over it too, and the window has not crossed, so no
    /// finding names the rating yet. While the run is younger than [`RECENT_PS`] that average
    /// runs from the first sample, and at the first sample it is that sample.
    fn over_now(&self) -> impl Iterator<Item = Diagnostic> + '_ {
        self.watches.iter().filter_map(|watch| {
            let heat = watch.heat.as_ref()?;
            let since = watch.since?;
            let at = self.last?;
            if watch.excursion.is_some() || !exceeds(heat.recent.since_origin(), watch.limit) {
                return None;
            }
            let excursion = Excursion {
                quantity: watch.quantity.clone(),
                limit: watch.limit,
                peak: heat.window.reading(),
                peak_at: at,
                first: since,
                over_ps: Some(at.as_ps() - since.as_ps()),
                window_ps: Some(DC_WINDOW_PS),
                sample: watch.last,
                held_ps: NonZeroU64::new(heat.window.held_ps(watch.limit).max(1)),
            };
            Some(match watch.kind {
                RatingKind::Current => Diagnostic::RatingCurrent(excursion),
                RatingKind::Power => Diagnostic::RatingPower(excursion),
            })
        })
    }

    /// [`Self::findings`] and each rating [`Self::over_now`], through `probe` when it is after
    /// the last sample: the time, the row there, and the cores whose drive the row was solved
    /// with. A probe is not kept, so the next accepted step samples as though it had not run.
    pub(crate) fn now(&self, probe: Option<(Time, &[Finite], &[active::Core])>) -> Vec<Diagnostic> {
        let mut probed = None;
        if let Some((at, row, cores)) = probe
            && self.last.is_none_or(|last| at > last)
        {
            let mut ratings = self.clone();
            ratings.sample(at, row, cores);
            probed = Some(ratings);
        }
        let ratings = probed.as_ref().unwrap_or(self);
        let mut found = ratings.findings();
        found.extend(ratings.over_now());
        found
    }
}

impl Timing {
    fn over_ps(self) -> Option<u64> {
        match self {
            Timing::Point => None,
            Timing::Run => Some(0),
        }
    }
}

impl Watch {
    fn new(
        kind: RatingKind,
        quantity: Ref,
        read: Read,
        limit: f64,
        reduce: Option<Reduce>,
    ) -> Watch {
        Watch {
            kind,
            quantity,
            read,
            limit,
            heat: reduce.map(|reduce| Heat {
                window: Square::new(reduce, DC_WINDOW_PS),
                recent: Square::new(reduce, RECENT_PS),
            }),
            over: false,
            excursion: None,
            last: 0.0,
            since: None,
        }
    }
}

fn magnitude(watch: &Watch, chips: &[Gauge], row: &[Finite]) -> f64 {
    match watch.read {
        Read::Column(column) => row[column].get(),
        Read::Gauge { gauge, k } => chips[gauge].currents[k],
    }
    .abs()
}

impl Square {
    fn new(reduce: Reduce, length: u64) -> Square {
        Square {
            reduce,
            length: i64::try_from(length).expect("a window fits in i64 ps"),
            spans: VecDeque::new(),
            sum: 0.0,
            held: None,
            origin: None,
        }
    }

    /// Takes `magnitude` at `at`, and returns the window's reading there. The sample held
    /// since the last one fills the time up to `at`, or with `back` this one does.
    fn take(&mut self, at: Time, magnitude: f64, back: bool) -> f64 {
        let at = i64::try_from(at.as_ps()).expect("a run's time fits in i64 ps");
        let sample = match self.reduce {
            Reduce::Rms => magnitude * magnitude,
            Reduce::Mean => magnitude,
        };
        if let Some((last, held)) = self.held
            && at > last
        {
            let held = if back { sample } else { held };
            match self.spans.back_mut() {
                Some((_, to, back)) if *to == last && *back == held => *to = at,
                _ => self.spans.push_back((last, at, held)),
            }
            self.sum += held * (at - last) as f64;
            let start = at - self.length;
            while let Some((from, to, stored)) = self.spans.front_mut() {
                if *to <= start {
                    self.sum -= *stored * (*to - *from) as f64;
                    self.spans.pop_front();
                } else {
                    if *from < start {
                        self.sum -= *stored * (start - *from) as f64;
                        *from = start;
                    }
                    break;
                }
            }
            // One span left is a steady sample, whose sum is exact again.
            if let (1, Some(&(from, to, stored))) = (self.spans.len(), self.spans.front()) {
                self.sum = stored * (to - from) as f64;
            }
        }
        self.origin.get_or_insert(at);
        self.held = Some((at, sample));
        self.reading()
    }

    /// The reading over the whole length, through the last sample.
    fn reading(&self) -> f64 {
        self.root(self.sum / self.length as f64)
    }

    /// The reading over the time since the first sample while that is shorter than the
    /// length, and the first sample itself until time has passed.
    fn since_origin(&self) -> f64 {
        let (Some((at, sample)), Some(origin)) = (self.held, self.origin) else {
            return 0.0;
        };
        match (at - origin).min(self.length) {
            0 => self.root(sample),
            filled => self.root(self.sum / filled as f64),
        }
    }

    fn root(&self, mean: f64) -> f64 {
        let mean = mean.max(0.0);
        match self.reduce {
            Reduce::Rms => mean.sqrt(),
            Reduce::Mean => mean,
        }
    }

    /// How long the last sample would have to hold for the reading to pass `limit`. As it
    /// holds, it enters the window and the oldest of the window leaves, the cold time before
    /// the first sample included.
    fn held_ps(&self, limit: f64) -> u64 {
        let Some((at, held)) = self.held else {
            return self.length.unsigned_abs();
        };
        let target = match self.reduce {
            Reduce::Rms => limit * limit,
            Reduce::Mean => limit,
        } * self.length as f64;
        let start = at - self.length;
        let cold = self
            .spans
            .front()
            .map_or(at, |&(from, _, _)| from)
            .max(start);
        let leaving = std::iter::once((start, cold, 0.0)).chain(self.spans.iter().copied());
        let (mut sum, mut elapsed) = (self.sum, 0.0);
        for (from, to, stored) in leaving {
            let length = (to - from) as f64;
            let slope = held - stored;
            if slope > 0.0 && sum + slope * length > target {
                return (elapsed + (target - sum) / slope).ceil() as u64;
            }
            sum += slope * length;
            elapsed += length;
        }
        self.length.unsigned_abs()
    }
}

impl Pin {
    fn new(output: &Output, columns: &Columns) -> Pin {
        Pin {
            pin: columns.net(output.pin),
            vcc: columns.net(output.vcc),
            gnd: columns.net(output.gnd),
            high: output.high,
            low: output.low,
            pull_up: output.pull_up,
            pull_down: output.pull_down,
            leakage: output.leakage,
        }
    }
}

impl Gauge {
    /// Each output's current under `drive`, as `active::stamp` stamps its stage at nominal
    /// sources, and the sums through the supply and ground pins.
    fn read(&mut self, row: &[Finite], core: &active::Core) {
        let drive = core.drive();
        debug_assert_eq!(drive.len(), self.outputs.len());
        let (mut supply, mut ground) = (0.0, 0.0);
        for (n, ((pin, &drive), current)) in self
            .outputs
            .iter()
            .zip(drive)
            .zip(&mut self.currents)
            .enumerate()
        {
            let v = |column: usize| row[column].get();
            let (vp, vcc, gnd) = (v(pin.pin), v(pin.vcc), v(pin.gnd));
            // From VCC into the pin's node, and from the node into GND.
            let (mut from_vcc, mut into_gnd) = (0.0, 0.0);
            if let Some(ohm) = pin.leakage {
                from_vcc += (vcc - vp) / ohm;
                into_gnd += (vp - gnd) / ohm;
            }
            match drive {
                Drive::High => from_vcc += (vcc - pin.high.drop - vp) / pin.high.resistance,
                Drive::Low => into_gnd += (vp - gnd - pin.low.drop) / pin.low.resistance,
                Drive::PullUp => {
                    from_vcc +=
                        (vcc - vp) / pin.pull_up.expect("a pull-up drive on a pin that has one");
                }
                Drive::Off => {}
            }
            if let Some(ohm) = pin.pull_down
                && core.pulls_down(n)
            {
                into_gnd += (vp - gnd) / ohm;
            }
            *current = from_vcc - into_gnd;
            supply += from_vcc;
            ground += into_gnd;
        }
        if let Some((vcc, gnd, own)) = self.supply {
            let own = (row[vcc].get() - row[gnd].get()) * own.conductance(core);
            supply += own;
            ground += own;
        }
        let outputs = self.outputs.len();
        self.currents[outputs] = supply;
        self.currents[outputs + 1] = ground;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn netlist() -> Netlist {
        crate::netlist::parse(
            br#"{
          "version": 1,
          "seed": 0,
          "analysis": { "op": {} },
          "ground": "gnd",
          "nets": ["gnd", "a"],
          "parts": [
            {
              "name": "d1",
              "kind": "diode",
              "pins": { "anode": "a", "cathode": "gnd" },
              "saturation_current": { "value": 1e-14, "unit": "A" },
              "emission_coefficient": { "value": 1.0, "unit": "1" },
              "series_resistance": { "value": 1.0, "unit": "ohm" },
              "ratings": {
                "current": { "value": 0.03, "unit": "A" },
                "power": null
              }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "a", "b": "gnd" },
              "resistance": { "value": 10.0, "unit": "ohm" },
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
        (Ratings::new(&netlist, &[], &columns, timing), columns)
    }

    #[test]
    fn exactly_the_limit_is_not_over() {
        let (mut ratings, columns) = ratings(Timing::Run);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.03), &[]);
        ratings.sample(Time::from_ps(1), &values(-0.03), &[]);
        assert!(ratings.findings().is_empty());
    }

    #[test]
    fn a_reverse_current_counts_by_magnitude_and_the_peak_follows_later_samples() {
        let (mut ratings, columns) = ratings(Timing::Point);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.01), &[]);
        ratings.sample(Time::from_ps(10), &values(-0.04), &[]);
        ratings.sample(Time::from_ps(20), &values(0.05), &[]);
        ratings.sample(Time::from_ps(30), &values(0.045), &[]);
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
                window_ps: None,
                sample: 0.05,
                held_ps: None,
            })]
        );
    }

    #[test]
    fn each_sample_over_the_limit_holds_until_the_next() {
        let (mut ratings, columns) = ratings(Timing::Run);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.01), &[]);
        ratings.sample(Time::from_ps(10), &values(0.04), &[]);
        ratings.sample(Time::from_ps(25), &values(0.05), &[]);
        ratings.sample(Time::from_ps(30), &values(0.02), &[]);
        ratings.sample(Time::from_ps(70), &values(0.035), &[]);
        ratings.sample(Time::from_ps(75), &values(0.031), &[]);
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding");
        };
        assert_eq!(excursion.first, Time::from_ps(10));
        assert_eq!(excursion.peak_at, Time::from_ps(25));
        assert_eq!(excursion.over_ps, Some(15 + 5 + 5));
    }

    /// `ratings` with its one watch, `#d1.i`, a DC rating over the window, as a supply's is.
    fn windowed() -> (Ratings, Columns) {
        let (mut ratings, columns) = ratings(Timing::Run);
        ratings.watches[0] = Watch::new(
            RatingKind::Current,
            ratings.watches[0].quantity.clone(),
            ratings.watches[0].read,
            ratings.watches[0].limit,
            Some(Reduce::Rms),
        );
        (ratings, columns)
    }

    fn close(got: f64, want: f64) {
        assert!(
            (got - want).abs() <= 1e-12 * want.max(1.0),
            "{got} against {want}"
        );
    }

    /// The window holds 0 before its first sample, as a cold part does.
    #[test]
    fn a_window_starts_cold() {
        let mut square = Square::new(Reduce::Rms, DC_WINDOW_PS);
        close(square.take(Time::ZERO, 0.3, false), 0.0);
        close(
            square.take(Time::from_ps(DC_WINDOW_PS / 2), 0.0, false),
            0.3 / 2f64.sqrt(),
        );
        close(
            square.take(Time::from_ps(DC_WINDOW_PS), 0.0, false),
            0.3 / 2f64.sqrt(),
        );
        close(
            square.take(Time::from_ps(3 * DC_WINDOW_PS / 2), 0.0, false),
            0.0,
        );
    }

    /// A pin into 100 pF carries about 0.2 A for 4 ns, which is 0.2 * sqrt(4 ns / 10.2 ms),
    /// 0.13 mA RMS over the window.
    #[test]
    fn a_short_pulse_counts_by_its_share_of_the_window() {
        let mut square = Square::new(Reduce::Rms, DC_WINDOW_PS);
        square.take(Time::ZERO, 0.0, false);
        square.take(Time::from_ps(1_000), 0.2, false);
        let rms = square.take(Time::from_ps(5_000), 0.0, false);
        close(rms, 0.2 * (4_000.0 / DC_WINDOW_PS as f64).sqrt());
        assert!(rms < 0.001);
        square.take(Time::from_ps(DC_WINDOW_PS + 2_000), 0.0, false);
        close(
            square.take(Time::from_ps(DC_WINDOW_PS + 3_000), 0.0, false),
            0.2 * (2_000.0 / DC_WINDOW_PS as f64).sqrt(),
        );
        close(
            square.take(Time::from_ps(DC_WINDOW_PS + 5_000), 0.0, false),
            0.0,
        );
    }

    /// A steady 69 mA from 0 crosses 40 mA once (40 / 69)^2 of the window has filled.
    #[test]
    fn a_steady_overload_crosses_its_rating_inside_the_window() {
        let (mut ratings, columns) = windowed();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        ratings.sample(Time::from_ps(1), &values(0.069), &[]);
        let step = DC_WINDOW_PS / 1_000;
        for k in 1..=2_000 {
            ratings.sample(Time::from_ps(1 + k * step), &values(0.069), &[]);
        }
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        let crossing = (0.03f64 / 0.069) * (0.03 / 0.069) * DC_WINDOW_PS as f64;
        assert!(
            excursion.first.as_ps().abs_diff(crossing as u64) <= step,
            "{:?}",
            excursion.first
        );
        close(excursion.peak, 0.069);
        assert_eq!(excursion.window_ps, Some(DC_WINDOW_PS));
        assert_eq!(
            excursion.over_ps,
            Some(1 + 2_000 * step - excursion.first.as_ps())
        );
    }

    /// Arduino's default PWM on D3, D9, D10 and D11 is phase correct at 16 MHz / (2 * 64 *
    /// 255), 490.2 Hz: 2.04 ms a period, high for `duty / 255` of it.
    const PWM_490_PS: u64 = 2_040_000_000;

    // The window holds a whole number of those periods, at least four.
    const _: () =
        assert!(DC_WINDOW_PS.is_multiple_of(PWM_490_PS) && DC_WINDOW_PS >= 4 * PWM_490_PS);

    /// `ratings` with its one watch a 40 mA DC rating, a port pin's, sampled at each edge of
    /// a PWM of `period` ps, high at `peak` amperes for `high` ps of each period, until `stop`
    /// ps.
    fn pwm(peak: f64, period: u64, high: u64, stop: u64) -> Vec<Diagnostic> {
        let (mut ratings, columns) = windowed();
        ratings.watches[0].limit = 0.04;
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        let mut start = 1;
        while start + period <= stop {
            ratings.sample(Time::from_ps(start), &values(peak), &[]);
            ratings.sample(Time::from_ps(start + high), &values(0.0), &[]);
            start += period;
        }
        ratings.sample(Time::from_ps(start), &values(0.0), &[]);
        ratings.findings()
    }

    /// `analogWrite(9, 128)` into 75 ohm behind the 25 ohm stage is 50 mA for 128 / 255 of
    /// each period: 35.4 mA RMS, under 40 mA, though each pulse is over 1 ms long.
    #[test]
    fn a_490_hz_pwm_under_its_rating_on_its_period_rms_is_not_over() {
        assert_eq!(
            pwm(0.05, PWM_490_PS, PWM_490_PS * 128 / 255, 100 * PWM_490_PS),
            []
        );
    }

    /// The window holds a whole number of 490 Hz periods, so the RMS it reports for a steady
    /// 490 Hz PWM is the period's: 69.4 mA into 47 ohm at 128 / 255 is 49.2 mA RMS.
    #[test]
    fn a_490_hz_pwm_over_its_rating_reads_its_period_rms() {
        let found = pwm(0.0694, PWM_490_PS, PWM_490_PS * 128 / 255, 100 * PWM_490_PS);
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        let rms = 0.0694 * (128.0f64 / 255.0).sqrt();
        assert!(
            (excursion.peak / rms - 1.0).abs() < 1e-6,
            "{}",
            excursion.peak
        );
    }

    /// Timer0's fast PWM on D5 and D6, 16 MHz / (64 * 256), is 976.6 Hz, high for
    /// `(duty + 1) / 256` of each 1.024 ms. Its RMS over the window is within 1% of the
    /// period's.
    #[test]
    fn a_980_hz_pwm_reads_within_a_percent_of_its_period_rms() {
        let period = 1_024_000_000;
        let high = period * 129 / 256;
        let found = pwm(0.0694, period, high, 200 * period);
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        let rms = 0.0694 * (129.0f64 / 256.0).sqrt();
        assert!(
            (excursion.peak / rms - 1.0).abs() < 0.01,
            "{}",
            excursion.peak
        );
        assert_eq!(pwm(0.05, period, high, 200 * period), []);
    }

    /// A pulse longer than the window fills it, so it counts at its full value.
    #[test]
    fn a_pulse_longer_than_the_window_counts_at_its_value() {
        let found = pwm(0.1, 4 * DC_WINDOW_PS, 2 * DC_WINDOW_PS, 8 * DC_WINDOW_PS);
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        close(excursion.peak, 0.1);
    }

    #[test]
    fn a_run_over_only_at_its_last_row_is_over_for_no_time() {
        let (mut ratings, columns) = ratings(Timing::Run);
        ratings.sample(Time::ZERO, &row(&columns, 0.0), &[]);
        ratings.sample(Time::from_ps(9), &row(&columns, 0.5), &[]);
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding");
        };
        assert_eq!(excursion.over_ps, Some(0));
        assert_eq!(excursion.first, Time::from_ps(9));
    }

    /// A resistor rated `limit` watts. The watch reads `#r1.p`, which the tests set directly.
    fn power_ratings(limit: f64) -> (Ratings, Columns) {
        let netlist = crate::netlist::parse(
            format!(
                r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": ["gnd", "a"],
              "parts": [{{
                "name": "r1",
                "kind": "resistor",
                "pins": {{ "a": "a", "b": "gnd" }},
                "resistance": {{ "value": 10.0, "unit": "ohm" }},
                "ratings": {{ "power": {{ "value": {limit}, "unit": "W" }} }}
              }}]
            }}"#
            )
            .as_bytes(),
        )
        .unwrap();
        let columns = Columns::new(&netlist);
        (Ratings::new(&netlist, &[], &columns, Timing::Run), columns)
    }

    fn row_power(columns: &Columns, power: f64) -> Vec<Finite> {
        let mut row = vec![Finite::new(0.0).unwrap(); columns.refs().len()];
        row[columns.index(&"#r1.p".parse().unwrap()).unwrap()] = Finite::new(power).unwrap();
        row
    }

    /// 3 W for 136 ns is 0.4 µJ. Over 10.2 ms that averages about 40 µW, under a 2.419 W rating.
    #[test]
    fn a_switching_spike_of_a_few_hundred_nanoseconds_stays_under() {
        let (mut ratings, columns) = power_ratings(2.419);
        let values = |power| row_power(&columns, power);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        ratings.sample(Time::from_ps(2_000), &values(3.008), &[]);
        ratings.sample(Time::from_ps(2_000 + 136_000), &values(0.0), &[]);
        ratings.sample(Time::from_ps(DC_WINDOW_PS), &values(0.0), &[]);
        assert!(ratings.findings().is_empty());
    }

    /// A steady 3 W from 0 crosses 2.419 W once about 2.419 / 3 of the window has filled.
    #[test]
    fn a_held_power_overload_crosses_by_its_share_of_the_window() {
        let (mut ratings, columns) = power_ratings(2.419);
        let values = |power| row_power(&columns, power);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        ratings.sample(Time::from_ps(1), &values(3.0), &[]);
        let step = DC_WINDOW_PS / 1_000;
        for k in 1..=2_000 {
            ratings.sample(Time::from_ps(1 + k * step), &values(3.0), &[]);
        }
        let found = ratings.findings();
        let [Diagnostic::RatingPower(excursion)] = found.as_slice() else {
            panic!("not one power finding: {found:?}");
        };
        // The rating rounds to 2.42 W, and the window is over once it rounds above that.
        let crossing = (2.425 / 3.0) * DC_WINDOW_PS as f64;
        assert!(
            excursion.first.as_ps().abs_diff(crossing as u64) <= step,
            "{:?}",
            excursion.first
        );
        close(excursion.peak, 3.0);
        assert_eq!(excursion.window_ps, Some(DC_WINDOW_PS));
    }

    /// `peak` watts for `high` ps of each 490 Hz period, `periods` of them, on a power rating.
    fn power_pwm(limit: f64, peak: f64, high: u64, periods: u64) -> Vec<Diagnostic> {
        let (mut ratings, columns) = power_ratings(limit);
        let values = |power| row_power(&columns, power);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        let mut start = 1;
        for _ in 0..periods {
            ratings.sample(Time::from_ps(start), &values(peak), &[]);
            ratings.sample(Time::from_ps(start + high), &values(0.0), &[]);
            start += PWM_490_PS;
        }
        ratings.sample(Time::from_ps(start), &values(0.0), &[]);
        ratings.findings()
    }

    /// 2.5 W at 50% is 1.25 W average, under 1.5 W. At 10% it is 0.25 W, under 1 W, though
    /// every pulse is over the rating.
    #[test]
    fn a_power_pulse_train_under_its_average_is_not_over() {
        let half = PWM_490_PS / 2;
        assert!(power_pwm(1.5, 2.5, half, 20).is_empty());
        assert!(power_pwm(1.0, 2.5, PWM_490_PS / 10, 20).is_empty());
    }

    /// 2.5 W at 50% averages 1.25 W. The window holds whole 490 Hz periods, so the average it
    /// reports is the period's.
    #[test]
    fn a_power_pulse_train_over_its_average_reads_the_period_mean() {
        let found = power_pwm(1.0, 2.5, PWM_490_PS / 2, 20);
        let [Diagnostic::RatingPower(excursion)] = found.as_slice() else {
            panic!("not one power finding: {found:?}");
        };
        assert!(
            (excursion.peak / 1.25 - 1.0).abs() < 1e-6,
            "{}",
            excursion.peak
        );
        assert_eq!(excursion.window_ps, Some(DC_WINDOW_PS));
    }

    /// The one rating over now, through the last sample.
    fn over_now(ratings: &Ratings) -> Vec<Excursion> {
        ratings
            .over_now()
            .map(|diagnostic| match diagnostic {
                Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                    excursion
                }
                other => panic!("not a rating: {other:?}"),
            })
            .collect()
    }

    const MS: u64 = 1_000_000_000;

    /// 0.989 W into a 0.25 W resistor from 50 ms on. 2 ms later the window averages 0.194 W,
    /// under the rating, so it does not warn, and the rating is over now. Held, the sample
    /// lifts the window past 0.25 W in (0.25 * 10.2 - 0.989 * 2) / 0.989 ms, 0.578 ms.
    #[test]
    fn a_fresh_overload_is_over_now_before_its_window_crosses() {
        let (mut ratings, columns) = power_ratings(0.25);
        let values = |power| row_power(&columns, power);
        for k in 0..50 {
            ratings.sample(Time::from_ps(k * MS), &values(0.0), &[]);
        }
        let start = 50 * MS;
        for k in 0..=20 {
            ratings.sample(Time::from_ps(start + k * MS / 10), &values(0.989), &[]);
        }
        assert!(ratings.findings().is_empty());
        let [excursion] = over_now(&ratings).try_into().unwrap();
        assert_eq!(excursion.first, Time::from_ps(start));
        assert_eq!(excursion.over_ps, Some(2 * MS));
        close(excursion.sample, 0.989);
        close(excursion.peak, 0.989 * 2.0 / 10.2);
        let held = excursion.held_ps.unwrap().get() as f64;
        assert!((held / (0.578 * MS as f64) - 1.0).abs() < 0.01, "{held}");
        assert_eq!(excursion.window_ps, Some(DC_WINDOW_PS));
    }

    /// A load on at the first sample is over now there, before any time has passed.
    #[test]
    fn a_steady_overload_is_over_now_at_its_first_sample() {
        let (mut ratings, columns) = power_ratings(0.25);
        ratings.sample(Time::from_ps(1), &row_power(&columns, 2.5), &[]);
        assert!(ratings.findings().is_empty());
        let [excursion] = over_now(&ratings).try_into().unwrap();
        close(excursion.sample, 2.5);
        assert_eq!(excursion.held_ps, NonZeroU64::new(DC_WINDOW_PS / 10));
    }

    /// 3 W for 136 ns, over a 2.419 W rating, stays under the average over the last 2.04 ms,
    /// so a probe inside the spike is not over now.
    #[test]
    fn a_switching_spike_is_not_over_now() {
        let (mut ratings, columns) = power_ratings(2.419);
        let values = |power| row_power(&columns, power);
        for k in 0..=10 {
            ratings.sample(Time::from_ps(k * MS), &values(0.0), &[]);
        }
        ratings.sample(Time::from_ps(10 * MS + 2_000), &values(3.008), &[]);
        let inside = values(3.0);
        let probe = Some((Time::from_ps(10 * MS + 70_000), inside.as_slice(), &[][..]));
        assert_eq!(ratings.now(probe), []);
        assert_eq!(ratings.now(None), []);
    }

    /// `analogWrite` at 128 / 255 into a pin rated 40 mA, 50 mA a pulse, from 10.2 ms on. Each
    /// pulse is over and the RMS of each period, 35.4 mA, is under, so no sample is over now.
    #[test]
    fn a_pwm_whose_rms_is_under_is_never_over_now() {
        let (mut ratings, columns) = windowed();
        ratings.watches[0].limit = 0.04;
        let values = |current| row(&columns, current);
        let high = PWM_490_PS * 128 / 255;
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        let mut start = DC_WINDOW_PS;
        for _ in 0..20 {
            ratings.sample(Time::from_ps(start), &values(0.05), &[]);
            for k in 1..=10 {
                ratings.sample(Time::from_ps(start + k * high / 10), &values(0.05), &[]);
                assert!(over_now(&ratings).is_empty(), "at {start} + {k}");
            }
            ratings.sample(Time::from_ps(start + high), &values(0.0), &[]);
            start += PWM_490_PS;
        }
        assert_eq!(ratings.findings(), []);
    }

    /// A load switched on at an edge, 50 ms in. The step's row holds the state before the edge,
    /// 0 W, and the next step, 1 ms later, reads 0.989 W. That sample holds back to the edge, so
    /// the window holds 1 ms of it and the rating is over now from the edge.
    #[test]
    fn after_an_edge_the_next_sample_holds_back_to_it() {
        let (mut ratings, columns) = power_ratings(0.25);
        let values = |power| row_power(&columns, power);
        for k in 0..=50 {
            ratings.sample(Time::from_ps(k * MS), &values(0.0), &[]);
        }
        ratings.restarted();
        ratings.sample(Time::from_ps(51 * MS), &values(0.989), &[]);
        let [excursion] = over_now(&ratings).try_into().unwrap();
        assert_eq!(excursion.first, Time::from_ps(50 * MS));
        close(excursion.peak, 0.989 / 10.2);
        ratings.sample(Time::from_ps(52 * MS), &values(0.989), &[]);
        let [excursion] = over_now(&ratings).try_into().unwrap();
        close(excursion.peak, 2.0 * 0.989 / 10.2);
    }
}
