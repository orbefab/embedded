use std::collections::VecDeque;
use std::num::NonZeroU64;

use orbe_model::{Drive, Time};

use crate::active::{self, Output, Stage};
use crate::netlist::{Average, Basis, Model, NetId, Netlist, Rating, RatingKind};

use crate::chip::regulator::{
    LIMIT_HOLD_PS, LIMIT_KNEE_FRACTION, LIMIT_SAG_FRACTION, MIN_CHARGE_RATE, STALL_PS, limit_drop,
};
use crate::diagnostic::{
    CurrentLimitExcursion, Diagnostic, Excursion, VoltageExcursion, exceeds_rounded, significant,
};
use crate::finite::Finite;
use crate::name::{Name, Quantity, Ref};
use crate::passive::DRIVE_REST_PS;

use crate::observe::Columns;

/// Every rated quantity of a netlist, checked against each accepted row.
///
/// It reads the observed row, so a part's rating is checked against the number the run
/// writes. A chip's pin ratings read the row's net voltages and the drive its core had in
/// force for that row, and work out each pin's current the way `active::stamp` stamps it.
///
/// Each current or power rating carries its datasheet's basis. A peak rating is checked on
/// each sample. A sustained rating bounds the heat a held value makes, so it is checked on a
/// first-order filter of the samples with the rating's time constant: of their squares for
/// [`Average::Rms`], read back as the root, and of their magnitudes for [`Average::Mean`]. A
/// pulse much shorter than the time constant counts by its share of it, a PWM load counts by
/// its period's average once the filter has settled, and a load held over the rating crosses
/// it within a few time constants. A rating with no basis takes its kind's: a power rating and
/// a supply source's current are sustained, with [`DEFAULT_TAU_PS`], RMS for the current and
/// mean for the power, and every other current rating is a peak. A chip pin's and a chip
/// supply's current are sustained RMS with that time constant unless the part says otherwise.
///
/// A sustained rating may also carry its sheet's peak limit, such as a diode's surge current or
/// an LED's pulsed forward current. That limit is a second watch on the same quantity, a peak
/// rating, so a sample over it warns at once while the filter still bounds the heat.
///
/// Each filter starts at zero, as a cold part does. A solve that holds each capacitor at a
/// voltage is never a sample: the t = 0 solve, and the restart solve at an edge. A capacitor
/// held there draws whatever its hold takes, which is no current the circuit carries. The first
/// accepted step is the first sample. Each sample holds until the next, except after an edge,
/// where the next sample holds back to the edge, [`Self::restarted`]. A sustained rating with no
/// peak limit whose last sample is over is over now, [`Self::over_now`], before its filter
/// crosses. One with a peak limit is not: its sheet bounds a pulse by that limit and the filter,
/// so a pulse the sheet allows stays quiet throughout. A diode or zener power rating is never
/// over now: its current peak warns at once past the surge, and the power filter warns when the
/// heat crosses. An LED's power stays over now.
#[derive(Clone)]
pub(crate) struct Ratings {
    timing: Timing,
    watches: Vec<Watch>,
    windows: Vec<Window>,
    limits: Vec<RegulatorLimit>,
    /// A resistor's operating voltage, checked on the peak of a drive that leaves zero.
    drives: Vec<CoilDrive>,
    chips: Vec<Gauge>,
    last: Option<Time>,
    /// The last sample was a step that restarted, so the next sample holds back to it.
    restarted: bool,
    /// The filters' decays over the span to the sample being taken.
    decays: Decays,
}

/// Each filter's decay over a span, `exp(-span / tau)`, worked out once per span and time
/// constant. Sustained ratings mostly share `DEFAULT_TAU_PS`, and every filter takes each
/// sample over the same span, so one sample's filters share a few.
#[derive(Clone, Default)]
struct Decays {
    /// The span in ps, the time constant's bits, and the decay.
    known: Vec<(i64, u64, f64)>,
}

impl Decays {
    fn get(&mut self, span: i64, tau_ps: f64) -> f64 {
        let tau = tau_ps.to_bits();
        if let Some(&(_, _, decay)) = self.known.iter().find(|k| (k.0, k.1) == (span, tau)) {
            return decay;
        }
        let decay = libm::exp(-(span as f64) / tau_ps);
        self.known.push((span, tau, decay));
        decay
    }
}

/// The time constant of a sustained rating whose netlist names no basis, 10.2 ms: five periods
/// of the slowest PWM an Arduino core makes by default, 490.2 Hz (16 MHz / (2 * 64 * 255),
/// 2.04 ms). A filter that slow reads a steady 490 Hz PWM within a few percent of its period's
/// RMS, and a capacitor's charging pulse through a pin, `I0^2 * tau / 2` of `i^2 * t`, stays
/// well under it.
pub(crate) const DEFAULT_TAU_PS: u64 = 10_200_000_000;

/// The span a sustained rating also averages over to say whether it is over now, 2.04 ms: one
/// period of the 490.2 Hz PWM. A PWM load whose RMS is under the rating stays under it, though
/// each pulse is over, and so does a switching spike of a few hundred nanoseconds. A load held
/// over the rating crosses it within the span, or at once while the run is younger than it.
pub(crate) const RECENT_PS: u64 = 2_040_000_000;

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
    pub(crate) pin_current: Option<Rating>,
    /// Ampere, through the supply pin and through the ground pin. Every output draws its high
    /// side, its pull-up and its leakage from the one and sinks its low side and its leakage
    /// into the other, and the chip's own draw passes through both.
    pub(crate) supply_current: Option<Rating>,
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
    /// [`significant`] of `limit`, compared on each sample so the limit is not formatted again.
    limit_rounded: f64,
    basis: Basis,
    characterized: bool,
    /// `Some` for a sustained rating.
    heat: Option<Heat>,
    /// Whether the last reading was over.
    over: bool,
    excursion: Option<Excursion>,
    /// The last sample's own magnitude.
    last: f64,
    /// The first of the samples that have each been over, through the last one.
    since: Option<Time>,
}

/// A sustained rating's readings of the same samples.
#[derive(Clone)]
struct Heat {
    /// The rating's thermal filter. It warns when it crosses.
    filter: Filter,
    /// Over [`RECENT_PS`]. With the last sample over, it says the rating is over now. `None`
    /// when the rating has a peak limit, which bounds a pulse in its place, or on a diode or
    /// zener power rating.
    recent: Option<Recent>,
}

/// What a filter or an average stores of a sample, and how it reads the store back.
fn stored(average: Average, magnitude: f64) -> f64 {
    match average {
        Average::Rms => magnitude * magnitude,
        Average::Mean => magnitude,
    }
}

fn root(average: Average, store: f64) -> f64 {
    let store = store.max(0.0);
    match average {
        Average::Rms => store.sqrt(),
        Average::Mean => store,
    }
}

/// A first-order filter with time constant `tau_ps`, each sample held until the next. It holds
/// 0 before its first sample, so it starts cold. A sample `x` held for `t` moves the level `y`
/// to `x + (y - x) * exp(-t / tau)`, exactly, so the step sizes do not change the reading.
#[derive(Clone)]
struct Filter {
    average: Average,
    tau_ps: f64,
    level: f64,
    /// The last sample's time and stored value.
    held: Option<(i64, f64)>,
}

/// The average over the last `length` ps, each sample held until the next, from the first
/// sample on.
#[derive(Clone)]
struct Recent {
    average: Average,
    length: i64,
    /// Spans `(from_ps, to_ps, stored)`, oldest first, which tile the span from the first
    /// sample on.
    spans: VecDeque<(i64, i64, f64)>,
    /// `stored * t` over the spans.
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
    /// The columns of its supply and ground nets for a device's `Draw`.
    draw: Option<(usize, usize)>,
    /// Each output's current out of its pin, then the current into the supply pin, then the
    /// current out of the ground pin. Set by the last sample.
    currents: Vec<f64>,
}

/// Below this absolute voltage the coil is between pulses. A peak that then sits outside
/// `[min, max]` is a drive outside its operating voltage.
const DRIVE_QUIET_V: f64 = 0.2;

/// The peak voltage of a sounding drive across a resistor that rates an operating window.
#[derive(Clone)]
struct CoilDrive {
    quantity: Ref,
    column: usize,
    min: f64,
    max: f64,
    /// `max` rounded as [`significant`] rounds it, for [`exceeds_rounded`].
    max_rounded: f64,
    /// Highest `|v|` since the coil was last near zero.
    peak: f64,
    /// The sample before this pulse left zero. `None` while the coil is quiet.
    pulse_from: Option<Time>,
    /// The last sample the coil was driven.
    loud_at: Option<Time>,
    /// The last judged pulse sat outside the window and the drive has not rested since.
    over: bool,
    /// How far `over_ps` has counted. A quiet gap shorter than a rest counts once the next
    /// pulse arrives, so a tone counts both its half-cycles and its rest counts nothing.
    counted_to: Time,
    /// How far past the window the excursion's peak was.
    worst: f64,
    excursion: Option<VoltageExcursion>,
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
    strong_pull_up: Option<f64>,
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

#[derive(Clone)]
struct RegulatorLimit {
    pin: Ref,
    pin_in: usize,
    pin_out: usize,
    pin_gnd: usize,
    iout: usize,
    reg: crate::chip::LinearRegulator,
    /// `V(out) - V(gnd)` on the last sample clear of the current-limit knee.
    baseline_vo: Option<f64>,
    /// Start of the current continuous clamped spell.
    clamped_since: Option<Time>,
    /// Previous sample in the current continuous clamped spell.
    last_clamped: Option<ClampedSample>,
    /// Monotone decreasing queue of `(span_end, peak_sample)` over the last [`LIMIT_HOLD_PS`],
    /// where `peak_sample` is the higher-voltage endpoint of each clamped step. Once the spell
    /// reaches [`LIMIT_HOLD_PS`], the front is the lowest voltage bound that held across it.
    hold: VecDeque<(Time, ClampedSample)>,
    /// A clamped rail starting below target (such as capacitor charging) that stays quiet while
    /// its output keeps rising toward target.
    pending: Option<PendingLimit>,
    excursion: Option<CurrentLimitExcursion>,
}

#[derive(Clone, Copy)]
struct ClampedSample {
    at: Time,
    vo: f64,
    current: f64,
}

#[derive(Clone)]
struct PendingLimit {
    first: Time,
    over_ps: Option<u64>,
    pulled_since: Option<Time>,
    progress_vo: f64,
    progress_at: Time,
}

/// The basis a sustained rating with no basis of its own takes.
const fn sustained(average: Average) -> Basis {
    Basis::Sustained {
        tau_ps: DEFAULT_TAU_PS,
        average,
    }
}

/// Records `drive.peak` when it sits outside the operating window. A peak inside clears
/// `over` and leaves an earlier finding in place. A drive that turns `over` counts from the
/// start of this pulse.
fn judge_drive(drive: &mut CoilDrive, at: Time, over_ps: Option<u64>) {
    let peak = drive.peak;
    // Each side is compared as the message prints it, to 3 significant digits.
    let below = exceeds_rounded(drive.min, significant(peak));
    let above = exceeds_rounded(peak, significant(drive.max));
    if !below && !above {
        drive.over = false;
        return;
    }
    if !drive.over {
        drive.over = true;
        drive.counted_to = drive.pulse_from.unwrap_or(at);
    }
    let past = if below {
        drive.min - peak
    } else {
        peak - drive.max
    };
    let reading = VoltageExcursion {
        pin: drive.quantity.clone(),
        low: Some(drive.min),
        high: Some(drive.max),
        peak,
        peak_at: at,
        first: at,
        over_ps,
        operating: true,
    };
    match &mut drive.excursion {
        None => {
            drive.worst = past;
            drive.excursion = Some(reading);
        }
        Some(excursion) if past > drive.worst => {
            drive.worst = past;
            excursion.low = reading.low;
            excursion.high = reading.high;
            excursion.peak = peak;
            excursion.peak_at = at;
        }
        Some(_) => {}
    }
}

impl Ratings {
    pub(crate) fn new(
        netlist: &Netlist,
        actives: &[active::Chip],
        columns: &Columns,
        timing: Timing,
    ) -> Ratings {
        let mut watches = Vec::new();
        let mut drives = Vec::new();
        for (_, part) in netlist.parts() {
            // A supply source's current rating is the most it delivers steadily. Every power
            // rating, on every kind, is the heat the part sheds.
            let supply = matches!(part.model, Model::VoltageSource(_));
            let surge = matches!(part.model, Model::Diode(_) | Model::Zener(_));
            for (kind, quantity, rating) in part.model.ratings() {
                let quantity = Ref::Quantity(part.name.clone(), quantity);
                let column = columns
                    .index(&quantity)
                    .expect("every part quantity is a column");
                let default = match kind {
                    RatingKind::Power => sustained(Average::Mean),
                    RatingKind::Current if supply => sustained(Average::Rms),
                    RatingKind::Current => Basis::Peak,
                };
                // A diode or zener power rating is never over now; an LED's stays over now.
                let early = !(kind == RatingKind::Power && surge);
                watch(
                    &mut watches,
                    kind,
                    quantity,
                    Read::Column(column),
                    rating,
                    default,
                    early,
                );
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
            let pin =
                |name: &str| Ref::Pin(part.name.clone(), Name::new(name).expect("a pin name"));
            let net = |id: NetId| columns.net(id);
            if rated.pin_current.is_some() || rated.supply_current.is_some() {
                debug_assert_eq!(rated.outputs.len(), chip.outputs.len());
                let gauge = chips.len();
                let default = sustained(Average::Rms);
                if let Some(rating) = &rated.pin_current {
                    for (k, &name) in rated.outputs.iter().enumerate() {
                        watch(
                            &mut watches,
                            RatingKind::Current,
                            pin(name),
                            Read::Gauge { gauge, k },
                            rating,
                            default,
                            true,
                        );
                    }
                }
                let outputs = chip.outputs.len();
                if let Some(rating) = &rated.supply_current {
                    for (k, name) in [(outputs, rated.vcc.0), (outputs + 1, rated.gnd.0)] {
                        watch(
                            &mut watches,
                            RatingKind::Current,
                            pin(name),
                            Read::Gauge { gauge, k },
                            rating,
                            default,
                            true,
                        );
                    }
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
                    draw: chip
                        .draw
                        .map(|(vcc, gnd)| (columns.net(vcc), columns.net(gnd))),
                    currents: vec![0.0; outputs + 2],
                });
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
        }
        // An input above `vcc - input_headroom` is the same window as a chip pin above its
        // ceiling, and an input below `gnd` past `input_below_gnd` as a pin below its floor.
        // The differential stage does not read either. A headroom of zero rates no ceiling:
        // the law examples use that, and a step can carry a node a little past the supply.
        // The supply's own rating is a window on `vcc`.
        for (_, part) in netlist.parts() {
            let Model::OpAmp(op) = &part.model else {
                continue;
            };
            let pin =
                |name: &str| Ref::Pin(part.name.clone(), Name::new(name).expect("a pin name"));
            let (inputs, vcc, gnd) = op.common_mode();
            let headroom = op.input_headroom.si.get();
            let above = (headroom > 0.0).then_some(Ceiling::OverSupply(-headroom));
            let below = op.input_below_gnd.as_ref().map(|floor| floor.si.get());
            let mut window = |name, net, below, above| {
                windows.push(Window {
                    pin: pin(name),
                    column: columns.net(net),
                    vcc: columns.net(vcc),
                    gnd: columns.net(gnd),
                    below,
                    above,
                    worst: 0.0,
                    over: false,
                    excursion: None,
                });
            };
            if above.is_some() || below.is_some() {
                for (name, net) in inputs {
                    window(name, net, below, above);
                }
            }
            if let Some(supply) = &op.max_supply {
                window("vcc", vcc, None, Some(Ceiling::Fixed(supply.si.get())));
            }
        }
        // A polarized capacitor and a regulator input use the same window. `vcc` is unread,
        // since `above` is `None` or fixed. Pin `b`, and the regulator's `gnd`, are the window's ground.
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
            let above = match &part.model {
                Model::LinearRegulator(reg) => reg
                    .max_input
                    .as_ref()
                    .map(|max| Ceiling::Fixed(max.si.get())),
                _ => None,
            };
            windows.push(Window {
                pin: Ref::Pin(part.name.clone(), Name::new(pin_name).expect("a pin name")),
                column,
                vcc: ground,
                gnd: ground,
                below: Some(below),
                above,
                worst: 0.0,
                over: false,
                excursion: None,
            });
        }
        let mut limits = Vec::new();
        for (_, part) in netlist.parts() {
            let Model::LinearRegulator(reg) = &part.model else {
                continue;
            };
            let iout_ref = Ref::Quantity(part.name.clone(), Quantity::Iout);
            let iout = columns
                .index(&iout_ref)
                .expect("every linear regulator has an iout column");
            limits.push(RegulatorLimit {
                pin: Ref::Pin(part.name.clone(), Name::new("out").expect("a pin name")),
                pin_in: columns.net(reg.pins[0]),
                pin_out: columns.net(reg.pins[1]),
                pin_gnd: columns.net(reg.pins[2]),
                iout,
                reg: reg.clone(),
                baseline_vo: None,
                clamped_since: None,
                last_clamped: None,
                hold: VecDeque::new(),
                pending: None,
                excursion: None,
            });
        }
        for (_, part) in netlist.parts() {
            let Model::Resistor(resistor) = &part.model else {
                continue;
            };
            if let Some(max) = &resistor.max_voltage {
                let limit = max.si.get();
                let ground = columns.net(resistor.pins[1]);
                windows.push(Window {
                    pin: Ref::Quantity(part.name.clone(), Quantity::V),
                    column: columns.net(resistor.pins[0]),
                    vcc: ground,
                    gnd: ground,
                    below: Some(limit),
                    above: Some(Ceiling::Fixed(limit)),
                    worst: 0.0,
                    over: false,
                    excursion: None,
                });
            }
            let Some((min, max)) = resistor.operating else {
                continue;
            };
            let quantity = Ref::Quantity(part.name.clone(), Quantity::V);
            let column = columns
                .index(&quantity)
                .expect("a resistor's voltage is a column");
            drives.push(CoilDrive {
                quantity,
                column,
                min,
                max,
                max_rounded: significant(max),
                peak: 0.0,
                pulse_from: None,
                loud_at: None,
                over: false,
                counted_to: Time::ZERO,
                worst: 0.0,
                excursion: None,
            });
        }
        Ratings {
            timing,
            watches,
            windows,
            limits,
            drives,
            chips,
            last: None,
            restarted: false,
            decays: Decays::default(),
        }
    }

    /// The step sampled last restarted at an edge. Its row holds the state before the edge,
    /// and the restart solve holds each capacitor, so neither is the state after it. The next
    /// sample is, and it holds back to the edge in each filter.
    pub(crate) fn restarted(&mut self) {
        self.restarted = true;
    }

    /// Over means a magnitude above the limit, a sustained rating's filter reading above it, or
    /// a voltage outside its window. Each side is compared after both are rounded to 3 significant
    /// digits. The first time over stays; the peak rises. A sample that was over adds the time
    /// until this one to `over_ps`. `cores` are the active chips' cores whose drive was in force
    /// for `row`. An operating point, [`Timing::Point`], has no duration, so its DC value fills
    /// each filter.
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
        self.decays.known.clear();
        for watch in &mut self.watches {
            elapsed(
                watch.over,
                watch.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
                self.last,
            );
            let sample = magnitude(watch, &self.chips, row);
            let reading = match (&mut watch.heat, self.timing) {
                (Some(heat), Timing::Run) => {
                    if let Some(recent) = &mut heat.recent {
                        recent.take(at, sample, back.is_some());
                    }
                    heat.filter
                        .take_in(at, sample, back.is_some(), &mut self.decays)
                }
                _ => sample,
            };
            watch.last = sample;
            let from = back.filter(|_| watch.heat.is_some()).unwrap_or(at);
            watch.since =
                exceeds_rounded(sample, watch.limit_rounded).then(|| watch.since.unwrap_or(from));
            watch.over = exceeds_rounded(reading, watch.limit_rounded);
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
                        basis: watch.basis,
                        sample,
                        held_ps: None,
                        characterized: watch.characterized,
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
            let WindowReading {
                v,
                low,
                high,
                below,
                above,
            } = window.read(row);
            window.over = below || above;
            if !window.over {
                continue;
            }
            let past = low
                .filter(|_| below)
                .map_or(f64::NEG_INFINITY, |low| low - v)
                .max(
                    high.filter(|_| above)
                        .map_or(f64::NEG_INFINITY, |high| v - high),
                );
            let reading = VoltageExcursion {
                pin: window.pin.clone(),
                low,
                high,
                peak: v,
                peak_at: at,
                first: at,
                over_ps: self.timing.over_ps(),
                operating: false,
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
        for limit in &mut self.limits {
            if limit.clamped_since.is_some()
                && let Some(last) = self.last
            {
                let dt = at.as_ps() - last.as_ps();
                if let Some(excursion) = &mut limit.excursion {
                    if let Some(over_ps) = &mut excursion.over_ps {
                        *over_ps += dt;
                    }
                } else if let Some(pending) = &mut limit.pending
                    && let Some(over_ps) = &mut pending.over_ps
                {
                    *over_ps += dt;
                }
            }
            let Clamp {
                vo,
                current: i,
                vout,
                drop,
                sag,
                clamped,
            } = limit.read(row);
            if !clamped {
                limit.clamped_since = None;
                limit.last_clamped = None;
                limit.hold.clear();
                limit.pending = None;
                if drop < LIMIT_KNEE_FRACTION * vout {
                    limit.baseline_vo = Some(vo);
                }
                continue;
            }
            let sample = ClampedSample { at, vo, current: i };
            let since = *limit.clamped_since.get_or_insert(at);
            if let Some(prev) = limit.last_clamped.replace(sample) {
                let high = if sample.vo >= prev.vo { sample } else { prev };
                while limit
                    .hold
                    .back()
                    .is_some_and(|(_, back)| back.vo <= high.vo)
                {
                    limit.hold.pop_back();
                }
                limit.hold.push_back((at, high));
                let cutoff = at.as_ps().saturating_sub(LIMIT_HOLD_PS);
                while limit
                    .hold
                    .front()
                    .is_some_and(|(to, _)| to.as_ps() <= cutoff)
                {
                    limit.hold.pop_front();
                }
            }
            let held = (at.as_ps() - since.as_ps() >= LIMIT_HOLD_PS)
                .then(|| limit.hold.front().map(|(_, s)| *s))
                .flatten();
            if let Some(excursion) = &mut limit.excursion {
                if let Some(peak) = held
                    && peak.vo < excursion.voltage
                {
                    excursion.voltage = peak.vo;
                    excursion.current = peak.current;
                    excursion.peak_at = peak.at;
                }
                continue;
            }
            let pulled = limit.baseline_vo.is_some_and(|before| before - vo >= sag);
            let warn = if self.timing == Timing::Point {
                true
            } else if let Some(pending) = &mut limit.pending {
                if pulled {
                    let since = *pending.pulled_since.get_or_insert(at);
                    at.as_ps() - since.as_ps() >= LIMIT_HOLD_PS
                } else {
                    pending.pulled_since = None;
                    let dt_ps = at.as_ps() - pending.progress_at.as_ps();
                    if dt_ps >= STALL_PS {
                        if vo - pending.progress_vo
                            >= MIN_CHARGE_RATE * drop * (dt_ps as f64 * 1e-12)
                        {
                            pending.progress_vo = vo;
                            pending.progress_at = at;
                            false
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                }
            } else {
                limit.pending = Some(PendingLimit {
                    first: at,
                    over_ps: self.timing.over_ps(),
                    pulled_since: pulled.then_some(at),
                    progress_vo: vo,
                    progress_at: at,
                });
                false
            };
            if warn {
                let (first, over_ps) = match limit.pending.take() {
                    Some(pending) => (pending.first, pending.over_ps),
                    None => (at, self.timing.over_ps()),
                };
                let peak = held.unwrap_or(sample);
                limit.excursion = Some(CurrentLimitExcursion {
                    pin: limit.pin.clone(),
                    limit: limit.reg.current_limit.si.get(),
                    target: vout,
                    voltage: peak.vo,
                    current: peak.current,
                    peak_at: peak.at,
                    first,
                    over_ps,
                });
            }
        }
        for drive in &mut self.drives {
            let volts = row[drive.column].get().abs();
            let loud = volts >= DRIVE_QUIET_V;
            let ended = !loud && drive.peak >= DRIVE_QUIET_V;
            if loud {
                drive.pulse_from.get_or_insert(self.last.unwrap_or(at));
                drive.loud_at = Some(at);
                if volts > drive.peak {
                    drive.peak = volts;
                    if exceeds_rounded(volts, drive.max_rounded) {
                        judge_drive(drive, at, self.timing.over_ps());
                    }
                }
            } else if ended {
                judge_drive(drive, at, self.timing.over_ps());
            }
            // A pulse that ends counts to its last loud sample. The quiet sample after it can
            // come a whole step later.
            let through = if loud { Some(at) } else { drive.loud_at };
            if drive.over
                && (loud || ended)
                && let Some(through) = through
            {
                if let Some(over_ps) = drive.excursion.as_mut().and_then(|e| e.over_ps.as_mut()) {
                    *over_ps += through.as_ps().saturating_sub(drive.counted_to.as_ps());
                }
                drive.counted_to = drive.counted_to.max(through);
            }
            if !loud {
                drive.peak = 0.0;
                drive.pulse_from = None;
                let rested = drive
                    .loud_at
                    .is_none_or(|loud_at| at.as_ps() - loud_at.as_ps() > DRIVE_REST_PS);
                if rested {
                    drive.over = false;
                }
            }
        }
        self.last = Some(at);
        self.restarted = false;
    }

    /// One `rating.*` diagnostic per quantity that went over, plus one `regulator.current_limit`
    /// per regulator held at its current limit, through the last sample.
    pub(crate) fn findings(&self) -> Vec<Diagnostic> {
        let currents = self.watches.iter().filter_map(|watch| {
            let excursion = watch.excursion.clone()?;
            Some(match watch.kind {
                RatingKind::Current => Diagnostic::RatingCurrent(Box::new(excursion)),
                RatingKind::Power => Diagnostic::RatingPower(Box::new(excursion)),
            })
        });
        let voltages = self
            .windows
            .iter()
            .filter_map(|window| Some(Diagnostic::RatingVoltage(window.excursion.clone()?)));
        let limits = self
            .limits
            .iter()
            .filter_map(|limit| Some(Diagnostic::RegulatorCurrentLimit(limit.excursion.clone()?)));
        let drives = self
            .drives
            .iter()
            .filter_map(|drive| Some(Diagnostic::RatingVoltage(drive.excursion.clone()?)));
        currents
            .chain(voltages)
            .chain(limits)
            .chain(drives)
            .collect()
    }

    /// Each sustained rating with no peak limit over now, through the last sample: that sample
    /// is over the rating, the average over [`RECENT_PS`] is over it too, and the filter has not
    /// crossed, so no finding names the rating yet. While the run is younger than
    /// [`RECENT_PS`] that average runs from the first sample, and at the first sample it is
    /// that sample. A diode or zener power rating is never over now: its current peak warns at
    /// once past the surge, and the power filter warns when the heat crosses.
    fn over_now(&self) -> impl Iterator<Item = Diagnostic> + '_ {
        self.watches.iter().filter_map(|watch| {
            let heat = watch.heat.as_ref()?;
            let recent = heat.recent.as_ref()?;
            let since = watch.since?;
            let at = self.last?;
            if watch.excursion.is_some()
                || !exceeds_rounded(recent.since_origin(), watch.limit_rounded)
            {
                return None;
            }
            let held = heat.filter.held_ps(watch.limit)?;
            let excursion = Excursion {
                quantity: watch.quantity.clone(),
                limit: watch.limit,
                peak: heat.filter.reading(),
                peak_at: at,
                first: since,
                over_ps: Some(at.as_ps() - since.as_ps()),
                basis: watch.basis,
                sample: watch.last,
                held_ps: NonZeroU64::new(held.max(1)),
                characterized: watch.characterized,
            };
            Some(match watch.kind {
                RatingKind::Current => Diagnostic::RatingCurrent(Box::new(excursion)),
                RatingKind::Power => Diagnostic::RatingPower(Box::new(excursion)),
            })
        })
    }

    /// [`Self::findings`] and each rating [`Self::over_now`], through `probe` when it is after
    /// the last sample: the time, the row there, and the cores whose drive the row was solved
    /// with. A probe is not kept, so the next accepted step samples as though it had not run.
    /// When [`Self::quiet_after`] says nothing is over before or after the probe, no copy is
    /// sampled.
    pub(crate) fn now(&self, probe: Option<(Time, &[Finite], &[active::Core])>) -> Vec<Diagnostic> {
        let mut probed = None;
        let mut quiet = false;
        if let Some((at, row, cores)) = probe
            && self.last.is_none_or(|last| at > last)
        {
            quiet = self.quiet_after(at, row, cores);
            // A debug build samples the copy anyway, to check that it finds nothing.
            if quiet && !cfg!(debug_assertions) {
                return Vec::new();
            }
            let mut ratings = self.clone();
            ratings.sample(at, row, cores);
            probed = Some(ratings);
        }
        let ratings = probed.as_ref().unwrap_or(self);
        let mut found = ratings.findings();
        found.extend(ratings.over_now());
        debug_assert!(!quiet || found.is_empty(), "a quiet probe found {found:?}");
        found
    }

    /// Nothing is over now, and a sample of `row` at `at` would leave nothing over: no watch,
    /// window, regulator or drive has an excursion, none would start one, and no sustained
    /// rating would be over now. `false` when a sample might change that, such as a clamped
    /// regulator or a drive pulse that ends. It reads what [`Self::sample`] reads the same way,
    /// and changes nothing.
    fn quiet_after(&self, at: Time, row: &[Finite], cores: &[active::Core]) -> bool {
        if self.watches.iter().any(|w| w.excursion.is_some())
            || self.windows.iter().any(|w| w.excursion.is_some())
            || self.limits.iter().any(|l| l.excursion.is_some())
            || self.drives.iter().any(|d| d.excursion.is_some())
        {
            return false;
        }
        let mut chips = self.chips.clone();
        for gauge in &mut chips {
            gauge.read(row, &cores[gauge.active]);
        }
        let back = self.restarted.then_some(self.last).flatten();
        let mut decays = Decays::default();
        let watches = self.watches.iter().all(|watch| {
            let sample = magnitude(watch, &chips, row);
            let reading = match (&watch.heat, self.timing) {
                (Some(heat), Timing::Run) => {
                    heat.filter
                        .clone()
                        .take_in(at, sample, back.is_some(), &mut decays)
                }
                _ => sample,
            };
            let over_now = watch
                .heat
                .as_ref()
                .is_some_and(|heat| heat.recent.is_some())
                && exceeds_rounded(sample, watch.limit_rounded);
            !exceeds_rounded(reading, watch.limit_rounded) && !over_now
        });
        let windows = self.windows.iter().all(|window| {
            let reading = window.read(row);
            !reading.below && !reading.above
        });
        let limits = self.limits.iter().all(|limit| !limit.read(row).clamped);
        let drives = self.drives.iter().all(|drive| {
            let volts = row[drive.column].get().abs();
            let loud = volts >= DRIVE_QUIET_V;
            let judged = if loud {
                volts > drive.peak && exceeds_rounded(volts, drive.max_rounded)
            } else {
                drive.peak >= DRIVE_QUIET_V
            };
            !judged
        });
        watches && windows && limits && drives
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

/// Pushes `rating`'s watch on `quantity`, with `default` as its basis when it names none, and a
/// peak watch beside it when it carries a peak limit.
fn watch(
    watches: &mut Vec<Watch>,
    kind: RatingKind,
    quantity: Ref,
    read: Read,
    rating: &Rating,
    default: Basis,
    early: bool,
) {
    let peak = rating.peak.map(Finite::get);
    let basis = rating.basis.unwrap_or(default);
    watches.push(Watch::new(
        kind,
        quantity.clone(),
        read,
        rating.si.get(),
        basis,
        early && peak.is_none(),
        rating.characterized,
    ));
    if let Some(peak) = peak {
        watches.push(Watch::new(
            kind,
            quantity,
            read,
            peak,
            Basis::Peak,
            false,
            rating.characterized,
        ));
    }
}

impl Watch {
    /// `early` gives a sustained rating its over-now average.
    fn new(
        kind: RatingKind,
        quantity: Ref,
        read: Read,
        limit: f64,
        basis: Basis,
        early: bool,
        characterized: bool,
    ) -> Watch {
        Watch {
            kind,
            quantity,
            read,
            limit,
            limit_rounded: significant(limit),
            basis,
            characterized,
            heat: match basis {
                Basis::Peak => None,
                Basis::Sustained { tau_ps, average } => Some(Heat {
                    filter: Filter::new(average, tau_ps),
                    recent: early.then(|| Recent::new(average, RECENT_PS)),
                }),
            },
            over: false,
            excursion: None,
            last: 0.0,
            since: None,
        }
    }

    /// Points the watch at another rating. The rounded limit moves with it, so a later
    /// sample still compares against this rating.
    #[cfg(test)]
    fn set_limit(&mut self, limit: f64) {
        self.limit_rounded = significant(limit);
        self.limit = limit;
    }
}

fn magnitude(watch: &Watch, chips: &[Gauge], row: &[Finite]) -> f64 {
    match watch.read {
        Read::Column(column) => row[column].get(),
        Read::Gauge { gauge, k } => chips[gauge].currents[k],
    }
    .abs()
}

/// Picoseconds as an `i64`, which every run's time fits.
fn ps(at: Time) -> i64 {
    i64::try_from(at.as_ps()).expect("a run's time fits in i64 ps")
}

impl Filter {
    fn new(average: Average, tau_ps: u64) -> Filter {
        Filter {
            average,
            tau_ps: tau_ps as f64,
            level: 0.0,
            held: None,
        }
    }

    /// Takes `magnitude` at `at`, and returns the reading there. The sample held since the
    /// last one fills the time up to `at`, or with `back` this one does.
    #[cfg(test)]
    fn take(&mut self, at: Time, magnitude: f64, back: bool) -> f64 {
        self.take_in(at, magnitude, back, &mut Decays::default())
    }

    /// `take`, with the decay from `decays`.
    fn take_in(&mut self, at: Time, magnitude: f64, back: bool, decays: &mut Decays) -> f64 {
        let at = ps(at);
        let sample = stored(self.average, magnitude);
        if let Some((last, held)) = self.held
            && at > last
        {
            let x = if back { sample } else { held };
            let decay = decays.get(at - last, self.tau_ps);
            self.level = x + (self.level - x) * decay;
        }
        self.held = Some((at, sample));
        self.reading()
    }

    fn reading(&self) -> f64 {
        root(self.average, self.level)
    }

    /// How long the last sample would have to hold for the reading to pass `limit`, or `None`
    /// when it never would.
    fn held_ps(&self, limit: f64) -> Option<u64> {
        let (_, x) = self.held?;
        let target = stored(self.average, limit);
        if self.level >= target {
            return Some(0);
        }
        (x > target)
            .then(|| (self.tau_ps * libm::log((x - self.level) / (x - target))).ceil() as u64)
    }
}

impl Recent {
    fn new(average: Average, length: u64) -> Recent {
        Recent {
            average,
            length: i64::try_from(length).expect("a span fits in i64 ps"),
            spans: VecDeque::new(),
            sum: 0.0,
            held: None,
            origin: None,
        }
    }

    /// Takes `magnitude` at `at`. The sample held since the last one fills the time up to
    /// `at`, or with `back` this one does.
    fn take(&mut self, at: Time, magnitude: f64, back: bool) {
        let at = ps(at);
        let sample = stored(self.average, magnitude);
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
    }

    /// The average over the time since the first sample while that is shorter than the
    /// length, and the first sample itself until time has passed.
    fn since_origin(&self) -> f64 {
        let (Some((at, sample)), Some(origin)) = (self.held, self.origin) else {
            return 0.0;
        };
        match (at - origin).min(self.length) {
            0 => root(self.average, sample),
            filled => root(self.average, self.sum / filled as f64),
        }
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
            strong_pull_up: output.strong_pull_up,
            pull_down: output.pull_down,
            leakage: output.leakage,
        }
    }
}

/// A pin's voltage over its chip's ground in one row, the window there, and which side of it
/// the voltage is past, each compared after rounding to 3 significant digits.
struct WindowReading {
    v: f64,
    low: Option<f64>,
    high: Option<f64>,
    below: bool,
    above: bool,
}

impl Window {
    fn read(&self, row: &[Finite]) -> WindowReading {
        let ground = row[self.gnd].get();
        let v = row[self.column].get() - ground;
        let low = self.below.map(|margin| -margin);
        let high = self.above.map(|ceiling| match ceiling {
            Ceiling::OverSupply(margin) => row[self.vcc].get() - ground + margin,
            Ceiling::Fixed(volts) => volts,
        });
        let below = low.is_some_and(|low| v < low && exceeds_rounded(-v, -significant(low)));
        let above = high.is_some_and(|high| v > high && exceeds_rounded(v, significant(high)));
        WindowReading {
            v,
            low,
            high,
            below,
            above,
        }
    }
}

/// A regulator's output over its ground and its load current in one row, its target, how far
/// the output sits under what the input allows, and whether that holds it at its current limit.
struct Clamp {
    vo: f64,
    current: f64,
    vout: f64,
    drop: f64,
    sag: f64,
    clamped: bool,
}

impl RegulatorLimit {
    fn read(&self, row: &[Finite]) -> Clamp {
        let ground = row[self.pin_gnd].get();
        let vi = row[self.pin_in].get() - ground;
        let vo = row[self.pin_out].get() - ground;
        let current = -row[self.iout].get();
        let vout = self.reg.output_voltage.si.get();
        let (vo_min, drop) = limit_drop(&self.reg, current, vi);
        let sag = LIMIT_SAG_FRACTION * vout;
        Clamp {
            vo,
            current,
            vout,
            drop,
            sag,
            clamped: vo_min > 0.0 && drop >= sag,
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
            // The same release as `active::stamp`: below the minimum supply the stage is off.
            let released = active::supply_released(core, vcc, gnd);
            let drive = if released { Drive::Off } else { drive };
            match drive {
                Drive::High => from_vcc += (vcc - pin.high.drop - vp) / pin.high.resistance,
                Drive::Low => into_gnd += (vp - gnd - pin.low.drop) / pin.low.resistance,
                Drive::PullUp => {
                    let ohm = if core.pulls_up_strong(n) {
                        pin.strong_pull_up
                            .expect("a strong pull-up drive on a pin that has one")
                    } else {
                        pin.pull_up.expect("a pull-up drive on a pin that has one")
                    };
                    from_vcc += (vcc - vp) / ohm;
                }
                Drive::Off => {}
            }
            if !released
                && let Some(ohm) = pin.pull_down
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
        if let Some((vcc, gnd)) = self.draw {
            let (amps, _) = core.draw().at(row[vcc].get() - row[gnd].get());
            supply += amps;
            ground += amps;
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

    const TAU: f64 = DEFAULT_TAU_PS as f64;

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
            [Diagnostic::RatingCurrent(Box::new(Excursion {
                quantity: "#d1.i".parse().unwrap(),
                limit: 0.03,
                peak: 0.05,
                peak_at: Time::from_ps(20),
                first: Time::from_ps(10),
                over_ps: None,
                basis: Basis::Peak,
                sample: 0.05,
                held_ps: None,
                characterized: false,
            }))]
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

    /// `ratings` with its one watch, `#d1.i`, sustained with `average`, as a supply's is.
    fn sustained_as(average: Average) -> (Ratings, Columns) {
        let (mut ratings, columns) = ratings(Timing::Run);
        ratings.watches[0] = Watch::new(
            RatingKind::Current,
            ratings.watches[0].quantity.clone(),
            ratings.watches[0].read,
            ratings.watches[0].limit,
            sustained(average),
            true,
            false,
        );
        (ratings, columns)
    }

    fn rms() -> (Ratings, Columns) {
        sustained_as(Average::Rms)
    }

    fn close(got: f64, want: f64) {
        assert!(
            (got - want).abs() <= 1e-9 * want.abs().max(1e-12),
            "{got} against {want}"
        );
    }

    /// The filter holds 0 before its first sample, as a cold part does, and a held sample
    /// moves it by `1 - exp(-t / tau)` of the way.
    #[test]
    fn a_shared_decay_is_the_one_each_filter_would_work_out() {
        let mut decays = Decays::default();
        let taus = [DEFAULT_TAU_PS as f64, 1e6, DEFAULT_TAU_PS as f64];
        for span in [1, 1_000, 1_000, 7_777_777, 1] {
            for tau in taus {
                let own = libm::exp(-(span as f64) / tau);
                assert_eq!(decays.get(span, tau).to_bits(), own.to_bits());
            }
        }
        assert_eq!(decays.known.len(), 6);
    }

    #[test]
    fn a_filter_starts_cold() {
        let mut filter = Filter::new(Average::Rms, DEFAULT_TAU_PS);
        close(filter.take(Time::ZERO, 0.3, false), 0.0);
        let half = DEFAULT_TAU_PS / 2;
        let rise = 0.09 * (1.0 - libm::exp(-0.5f64));
        close(filter.take(Time::from_ps(half), 0.0, false), rise.sqrt());
        close(
            filter.take(Time::from_ps(2 * half), 0.0, false),
            (rise * libm::exp(-0.5f64)).sqrt(),
        );
    }

    /// A pin into 100 pF carries about 0.2 A for 4 ns, which reads 0.2 * sqrt(4 ns / 10.2 ms),
    /// 0.13 mA RMS, and then decays.
    #[test]
    fn a_short_pulse_counts_by_its_share_of_the_time_constant() {
        let mut filter = Filter::new(Average::Rms, DEFAULT_TAU_PS);
        filter.take(Time::ZERO, 0.0, false);
        filter.take(Time::from_ps(1_000), 0.2, false);
        let rms = filter.take(Time::from_ps(5_000), 0.0, false);
        close(rms, (0.04 * -libm::expm1(-4_000.0 / TAU)).sqrt());
        assert!((rms / (0.2 * (4_000.0 / TAU).sqrt()) - 1.0).abs() < 1e-6);
        let later = filter.take(Time::from_ps(5_000 + DEFAULT_TAU_PS), 0.0, false);
        close(later, rms * libm::exp(-0.5f64));
    }

    /// The filter reads the same whichever steps the samples come in.
    #[test]
    fn the_step_sizes_do_not_change_the_reading() {
        let mut one = Filter::new(Average::Mean, DEFAULT_TAU_PS);
        let mut many = Filter::new(Average::Mean, DEFAULT_TAU_PS);
        one.take(Time::ZERO, 0.5, false);
        many.take(Time::ZERO, 0.5, false);
        for k in 1..=1_000 {
            many.take(Time::from_ps(k * DEFAULT_TAU_PS / 1_000), 0.5, false);
        }
        let whole = one.take(Time::from_ps(DEFAULT_TAU_PS), 0.5, false);
        assert!((whole / many.reading() - 1.0).abs() < 1e-12);
        close(whole, 0.5 * (1.0 - libm::exp(-1f64)));
    }

    /// A steady 69 mA from 0 crosses 30 mA RMS at `tau * ln(x^2 / (x^2 - L^2))`. The rating
    /// rounds to 0.03 A, and the reading is over once it rounds above that.
    #[test]
    fn a_steady_overload_crosses_its_rating_after_its_share_of_the_time_constant() {
        let (mut ratings, columns) = rms();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        ratings.sample(Time::from_ps(1), &values(0.069), &[]);
        let step = DEFAULT_TAU_PS / 1_000;
        for k in 1..=5_000 {
            ratings.sample(Time::from_ps(1 + k * step), &values(0.069), &[]);
        }
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        let (x, limit) = (0.069f64 * 0.069, 0.03005f64 * 0.03005);
        let crossing = TAU * libm::log(x / (x - limit));
        assert!(
            excursion.first.as_ps().abs_diff(crossing as u64) <= step,
            "{:?} against {crossing}",
            excursion.first
        );
        assert!((excursion.peak / 0.069 - 1.0).abs() < 1e-2);
        assert_eq!(excursion.basis, sustained(Average::Rms));
        assert_eq!(
            excursion.over_ps,
            Some(1 + 5_000 * step - excursion.first.as_ps())
        );
    }

    /// A load that ran under its rating for a while and then rises over it crosses from the
    /// level it had, not from cold: 20 mA for 50 ms, then 50 mA, against 30 mA mean.
    #[test]
    fn a_mid_run_overload_crosses_from_the_level_it_had() {
        let (mut ratings, columns) = sustained_as(Average::Mean);
        let values = |current| row(&columns, current);
        let step = DEFAULT_TAU_PS / 1_000;
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        let start = 5 * DEFAULT_TAU_PS;
        let mut at = 1;
        while at < start {
            ratings.sample(Time::from_ps(at), &values(0.02), &[]);
            at += step;
        }
        let level = 0.02 * -libm::expm1(-((start - 1) as f64) / TAU);
        // Once the last 2.04 ms average is over too, 1 ms on, the rating is over now.
        for k in 0..=100 {
            ratings.sample(Time::from_ps(start + k * step), &values(0.05), &[]);
        }
        assert!(ratings.findings().is_empty());
        let [excursion] = over_now(&ratings).try_into().unwrap();
        let now = 0.05 + (level - 0.05) * libm::exp(-0.1f64);
        let held = TAU * libm::log((0.05 - now) / (0.05 - 0.03));
        let got = excursion.held_ps.unwrap().get() as f64;
        assert!((got / held - 1.0).abs() < 1e-3, "{got} against {held}");
        for k in 101..=2_000 {
            ratings.sample(Time::from_ps(start + k * step), &values(0.05), &[]);
        }
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        let crossing = start as f64 + TAU * libm::log((0.05 - level) / (0.05 - 0.03005));
        assert!(
            excursion.first.as_ps().abs_diff(crossing as u64) <= step,
            "{:?} against {crossing}",
            excursion.first
        );
    }

    /// Arduino's default PWM on D3, D9, D10 and D11 is phase correct at 16 MHz / (2 * 64 *
    /// 255), 490.2 Hz: 2.04 ms a period, high for `duty / 255` of it.
    const PWM_490_PS: u64 = 2_040_000_000;

    /// `ratings` with its one watch a 40 mA DC rating, a port pin's, sampled at each edge of
    /// a PWM of `period` ps, high at `peak` amperes for `high` ps of each period, until `stop`
    /// ps.
    fn pwm(peak: f64, period: u64, high: u64, stop: u64) -> Vec<Diagnostic> {
        let (mut ratings, columns) = rms();
        ratings.watches[0].set_limit(0.04);
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
    /// each period: 35.4 mA RMS, under 40 mA, though each pulse is over 1 ms long. The
    /// filter's ripple tops out at 36.3 mA.
    #[test]
    fn a_490_hz_pwm_under_its_rating_on_its_period_rms_is_not_over() {
        assert_eq!(
            pwm(0.05, PWM_490_PS, PWM_490_PS * 128 / 255, 100 * PWM_490_PS),
            []
        );
    }

    /// A steady PWM settles to its period's RMS, and the reading at the top of each pulse is
    /// within a few percent above it: 69.4 mA at 128 / 255 is 49.2 mA RMS.
    #[test]
    fn a_490_hz_pwm_over_its_rating_reads_near_its_period_rms() {
        let found = pwm(0.0694, PWM_490_PS, PWM_490_PS * 128 / 255, 100 * PWM_490_PS);
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        let rms = 0.0694 * (128.0f64 / 255.0).sqrt();
        let ratio = excursion.peak / rms;
        assert!((1.0..1.03).contains(&ratio), "{}", excursion.peak);
    }

    /// Timer0's fast PWM on D5 and D6, 16 MHz / (64 * 256), is 976.6 Hz, high for
    /// `(duty + 1) / 256` of each 1.024 ms. Its reading is within 2% of the period's RMS.
    #[test]
    fn a_980_hz_pwm_reads_within_two_percent_of_its_period_rms() {
        let period = 1_024_000_000;
        let high = period * 129 / 256;
        let found = pwm(0.0694, period, high, 200 * period);
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        let rms = 0.0694 * (129.0f64 / 256.0).sqrt();
        let ratio = excursion.peak / rms;
        assert!((1.0..1.02).contains(&ratio), "{}", excursion.peak);
        assert_eq!(pwm(0.05, period, high, 200 * period), []);
    }

    /// A pulse held for many time constants reads its own value.
    #[test]
    fn a_pulse_held_for_many_time_constants_reads_its_value() {
        let long = 10 * DEFAULT_TAU_PS;
        let found = pwm(0.1, 2 * long, long, 4 * long);
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        assert!(
            (excursion.peak / 0.1 - 1.0).abs() < 1e-4,
            "{}",
            excursion.peak
        );
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

    /// 3 W for 136 ns is 0.4 µJ. Against a 10.2 ms time constant that reads about 40 µW,
    /// under a 2.419 W rating.
    #[test]
    fn a_switching_spike_of_a_few_hundred_nanoseconds_stays_under() {
        let (mut ratings, columns) = power_ratings(2.419);
        let values = |power| row_power(&columns, power);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        ratings.sample(Time::from_ps(2_000), &values(3.008), &[]);
        ratings.sample(Time::from_ps(2_000 + 136_000), &values(0.0), &[]);
        ratings.sample(Time::from_ps(DEFAULT_TAU_PS), &values(0.0), &[]);
        assert!(ratings.findings().is_empty());
    }

    /// A steady 3 W from 0 crosses 2.419 W at `tau * ln(3 / (3 - 2.425))`, since the rating
    /// rounds to 2.42 W and the reading is over once it rounds above that.
    #[test]
    fn a_held_power_overload_crosses_by_its_share_of_the_time_constant() {
        let (mut ratings, columns) = power_ratings(2.419);
        let values = |power| row_power(&columns, power);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        ratings.sample(Time::from_ps(1), &values(3.0), &[]);
        let step = DEFAULT_TAU_PS / 1_000;
        for k in 1..=5_000 {
            ratings.sample(Time::from_ps(1 + k * step), &values(3.0), &[]);
        }
        let found = ratings.findings();
        let [Diagnostic::RatingPower(excursion)] = found.as_slice() else {
            panic!("not one power finding: {found:?}");
        };
        let crossing = TAU * libm::log(3.0f64 / (3.0 - 2.425));
        assert!(
            excursion.first.as_ps().abs_diff(crossing as u64) <= step,
            "{:?}",
            excursion.first
        );
        assert!((excursion.peak / 3.0 - 1.0).abs() < 1e-2);
        assert_eq!(excursion.basis, sustained(Average::Mean));
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
        assert!(power_pwm(1.5, 2.5, half, 40).is_empty());
        assert!(power_pwm(1.0, 2.5, PWM_490_PS / 10, 40).is_empty());
    }

    /// 2.5 W at 50% averages 1.25 W, and the reading at the top of each pulse is within 5%
    /// above that.
    #[test]
    fn a_power_pulse_train_over_its_average_reads_near_the_period_mean() {
        let found = power_pwm(1.0, 2.5, PWM_490_PS / 2, 40);
        let [Diagnostic::RatingPower(excursion)] = found.as_slice() else {
            panic!("not one power finding: {found:?}");
        };
        let ratio = excursion.peak / 1.25;
        assert!((1.0..1.06).contains(&ratio), "{}", excursion.peak);
    }

    /// The one rating over now, through the last sample.
    fn over_now(ratings: &Ratings) -> Vec<Excursion> {
        ratings
            .over_now()
            .map(|diagnostic| match diagnostic {
                Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                    *excursion
                }
                other => panic!("not a rating: {other:?}"),
            })
            .collect()
    }

    const MS: u64 = 1_000_000_000;

    /// 0.989 W into a 0.25 W resistor from 50 ms on. 2 ms later the filter reads
    /// `0.989 * (1 - exp(-2 / 10.2))`, 0.176 W, under the rating, so it does not warn, and the
    /// rating is over now. Held, the sample lifts it past 0.25 W after
    /// `tau * ln((0.989 - 0.176) / (0.989 - 0.25))` more.
    #[test]
    fn a_fresh_overload_is_over_now_before_its_filter_crosses() {
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
        let level = 0.989 * -libm::expm1(-2.0 * MS as f64 / TAU);
        assert!((excursion.peak / level - 1.0).abs() < 1e-9);
        let held = excursion.held_ps.unwrap().get() as f64;
        let want = TAU * libm::log((0.989 - level) / (0.989 - 0.25));
        assert!((held / want - 1.0).abs() < 1e-6, "{held} against {want}");
        assert_eq!(excursion.basis, sustained(Average::Mean));
    }

    /// A load on at the first sample is over now there, before any time has passed. It warns
    /// after `tau * ln(2.5 / (2.5 - 0.25))`.
    #[test]
    fn a_steady_overload_is_over_now_at_its_first_sample() {
        let (mut ratings, columns) = power_ratings(0.25);
        ratings.sample(Time::from_ps(1), &row_power(&columns, 2.5), &[]);
        assert!(ratings.findings().is_empty());
        let [excursion] = over_now(&ratings).try_into().unwrap();
        close(excursion.sample, 2.5);
        let want = (TAU * libm::log(2.5f64 / 2.25)).ceil() as u64;
        assert_eq!(excursion.held_ps, NonZeroU64::new(want));
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
        let (mut ratings, columns) = rms();
        ratings.watches[0].set_limit(0.04);
        let values = |current| row(&columns, current);
        let high = PWM_490_PS * 128 / 255;
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        let mut start = DEFAULT_TAU_PS;
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
    /// the filter holds 1 ms of it and the rating is over now from the edge.
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
        let rise = |ms: f64| 0.989 * -libm::expm1(-ms * MS as f64 / TAU);
        assert!((excursion.peak / rise(1.0) - 1.0).abs() < 1e-9);
        ratings.sample(Time::from_ps(52 * MS), &values(0.989), &[]);
        let [excursion] = over_now(&ratings).try_into().unwrap();
        assert!((excursion.peak / rise(2.0) - 1.0).abs() < 1e-9);
    }

    /// A diode rated 30 mA as a mean with a 1 ms time constant and 160 mA peak, as an LED's
    /// sheet gives its DC and pulsed forward currents.
    fn with_peak() -> (Ratings, Columns) {
        let netlist = crate::netlist::parse(
            br#"{
          "version": 1,
          "seed": 0,
          "analysis": { "op": {} },
          "ground": "gnd",
          "nets": ["gnd", "a"],
          "parts": [{
            "name": "d1",
            "kind": "diode",
            "pins": { "anode": "a", "cathode": "gnd" },
            "saturation_current": { "value": 1e-14, "unit": "A" },
            "emission_coefficient": { "value": 1.0, "unit": "1" },
            "series_resistance": { "value": 1.0, "unit": "ohm" },
            "ratings": {
              "current": { "value": 0.03, "unit": "A", "basis": { "sustained": {
                "average": "mean", "tau": { "value": 0.001, "unit": "s" },
                "peak": { "value": 0.16, "unit": "A" } } } },
              "power": null
            }
          }]
        }"#,
        )
        .unwrap();
        let columns = Columns::new(&netlist);
        (Ratings::new(&netlist, &[], &columns, Timing::Run), columns)
    }

    /// The sheet's pulse, 0.1 ms at 1/10 duty, at its 160 mA peak, and a held 2 ms pulse at
    /// 100 mA, each over the 30 mA rating: no sample is over now and nothing warns, since the
    /// peak limit bounds a pulse in place of over now.
    #[test]
    fn a_pulse_under_its_peak_limit_is_never_over_now() {
        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        let tenth = MS / 10;
        for period in 1..=30 {
            let start = period * MS;
            ratings.sample(Time::from_ps(start), &values(0.16), &[]);
            ratings.sample(Time::from_ps(start + tenth / 2), &values(0.16), &[]);
            assert!(over_now(&ratings).is_empty(), "period {period}");
            ratings.sample(Time::from_ps(start + tenth), &values(0.0), &[]);
        }
        assert_eq!(ratings.findings(), []);
        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        for k in 1..=20 {
            ratings.sample(Time::from_ps(MS + k * tenth), &values(0.1), &[]);
            assert!(over_now(&ratings).is_empty(), "at {k}");
        }
    }

    /// A sample over the peak limit warns at once on its peak basis. A held load under it still
    /// warns when the filter crosses, `tau * ln(50 / 20)` after it starts.
    #[test]
    fn a_rating_with_a_peak_limit_warns_on_either() {
        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        ratings.sample(Time::from_ps(MS), &values(0.2), &[]);
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(peak)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        assert_eq!(peak.basis, Basis::Peak);
        assert_eq!(peak.limit, 0.16);
        close(peak.peak, 0.2);

        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[]);
        let step = MS / 100;
        for k in 0..=300 {
            ratings.sample(Time::from_ps(MS + k * step), &values(0.05), &[]);
            assert!(over_now(&ratings).is_empty(), "at {k}");
        }
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(held)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        assert!(matches!(held.basis, Basis::Sustained { .. }));
        let crossing = MS as f64 * (1.0 + libm::log(0.05f64 / (0.05 - 0.03005)));
        assert!(
            held.first.as_ps().abs_diff(crossing as u64) <= step,
            "{:?} against {crossing}",
            held.first
        );
    }

    /// A peak rating has no filter and is never over now: it warns on the sample.
    #[test]
    fn a_peak_rating_warns_on_its_first_sample_over() {
        let (mut ratings, columns) = ratings(Timing::Run);
        ratings.sample(Time::ZERO, &row(&columns, 0.0), &[]);
        ratings.sample(Time::from_ps(MS), &row(&columns, 0.031), &[]);
        assert!(over_now(&ratings).is_empty());
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(excursion)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        assert_eq!(excursion.basis, Basis::Peak);
        close(excursion.peak, 0.031);
    }

    /// A diode whose current rating names a surge peak. Power has no peak of its own.
    fn diode_power() -> (Ratings, Columns) {
        let netlist = crate::netlist::parse(
            br#"{
          "version": 1,
          "seed": 0,
          "analysis": { "op": {} },
          "ground": "gnd",
          "nets": ["gnd", "a"],
          "parts": [{
            "name": "d1",
            "kind": "diode",
            "pins": { "anode": "a", "cathode": "gnd" },
            "saturation_current": { "value": 1e-12, "unit": "A" },
            "emission_coefficient": { "value": 1.0, "unit": "1" },
            "series_resistance": { "value": 1.0, "unit": "ohm" },
            "ratings": {
              "current": { "value": 0.15, "unit": "A", "basis": { "sustained": {
                "average": "mean", "tau": { "value": 0.0102, "unit": "s" },
                "peak": { "value": 4.0, "unit": "A" } } } },
              "power": { "value": 0.4, "unit": "W", "basis": { "sustained": {
                "average": "mean", "tau": { "value": 0.0102, "unit": "s" } } } }
            }
          }]
        }"#,
        )
        .unwrap();
        let columns = Columns::new(&netlist);
        (Ratings::new(&netlist, &[], &columns, Timing::Run), columns)
    }

    fn row_at(columns: &Columns, current: f64, power: f64) -> Vec<Finite> {
        let mut row = vec![Finite::new(0.0).unwrap(); columns.refs().len()];
        row[columns.index(&"#d1.i".parse().unwrap()).unwrap()] = Finite::new(current).unwrap();
        row[columns.index(&"#d1.p".parse().unwrap()).unwrap()] = Finite::new(power).unwrap();
        row
    }

    /// 4 A and 16 W is inside the CD1206's surge peak and far over its 0.4 W power rating.
    /// A diode's power is never over now. 5 A is past the peak, so the current warns at once
    /// and power still is not. 0.5 A stays inside the peak and the power filter still warns
    /// when held.
    #[test]
    fn power_inside_a_current_peak_is_not_over_now_and_a_held_overload_still_warns() {
        let (mut ratings, columns) = diode_power();
        let values = |current, power| row_at(&columns, current, power);
        ratings.sample(Time::ZERO, &values(0.0, 0.0), &[]);
        ratings.sample(Time::from_ps(MS), &values(4.0, 16.1), &[]);
        ratings.sample(Time::from_ps(MS + 50_000_000), &values(4.0, 16.1), &[]);
        assert!(over_now(&ratings).is_empty(), "{:?}", over_now(&ratings));
        assert!(ratings.findings().is_empty());

        let (mut ratings, columns) = diode_power();
        let values = |current, power| row_at(&columns, current, power);
        ratings.sample(Time::ZERO, &values(0.0, 0.0), &[]);
        ratings.sample(Time::from_ps(MS), &values(5.0, 20.0), &[]);
        // 100 us later the recent average is over and the 10.2 ms filter has not crossed.
        // The current peak has already warned, so power is still not over now.
        ratings.sample(Time::from_ps(MS + 100_000_000), &values(5.0, 20.0), &[]);
        assert!(
            over_now(&ratings).is_empty(),
            "past the peak, power is not over now: {:?}",
            over_now(&ratings)
        );
        let found = ratings.findings();
        assert!(
            found.iter().any(|d| {
                matches!(d, Diagnostic::RatingCurrent(excursion) if excursion.basis == Basis::Peak)
            }),
            "the current peak warns at once: {found:?}"
        );

        let (mut ratings, columns) = diode_power();
        let values = |current, power| row_at(&columns, current, power);
        ratings.sample(Time::ZERO, &values(0.0, 0.0), &[]);
        let step = MS / 10;
        for k in 0..=200 {
            ratings.sample(Time::from_ps(MS + k * step), &values(0.5, 0.664), &[]);
            assert!(
                over_now(&ratings)
                    .iter()
                    .all(|e| e.quantity.to_string() != "#d1.p"),
                "at {k}"
            );
        }
        let found = ratings.findings();
        assert!(
            found
                .iter()
                .any(|d| matches!(d, Diagnostic::RatingPower(_))),
            "a held overload warns on the filter: {found:?}"
        );
    }

    #[test]
    fn voltage_window_compares_after_rounding_to_three_significant_digits() {
        let netlist = crate::netlist::parse(
            br#"{
          "version": 1,
          "seed": 0,
          "analysis": { "op": {} },
          "ground": "gnd",
          "nets": ["gnd", "vin", "vout"],
          "parts": [{
            "name": "u1",
            "kind": "linear_regulator",
            "pins": { "in": "vin", "out": "vout", "gnd": "gnd" },
            "output_voltage": { "value": 3.3, "unit": "V" },
            "output_resistance": { "value": 0.1, "unit": "ohm" },
            "dropout_voltage": { "value": 0.2, "unit": "V" },
            "dropout_resistance": { "value": 1.0, "unit": "ohm" },
            "current_limit": { "value": 1.0, "unit": "A" },
            "quiescent_current": { "value": 1e-4, "unit": "A" },
            "ground_current_ratio": { "value": 0.0, "unit": "1" },
            "line_regulation": { "value": 0.0, "unit": "1" },
            "ratings": {
              "current": null,
              "power": null,
              "input_voltage": { "value": 3.600019, "unit": "V" }
            }
          }]
        }"#,
        )
        .unwrap();
        let columns = Columns::new(&netlist);
        let row_vin = |volts: f64| {
            let mut row = vec![Finite::new(0.0).unwrap(); columns.refs().len()];
            row[columns.index(&"#net.vin".parse().unwrap()).unwrap()] = Finite::new(volts).unwrap();
            row
        };

        // 3.604 V rounds to 3.6 V, same as the 3.600019 V ceiling: stays quiet.
        // -0.3004 V rounds to -0.3 V, same as the -0.3 V floor: stays quiet.
        let mut quiet = Ratings::new(&netlist, &[], &columns, Timing::Point);
        quiet.sample(Time::ZERO, &row_vin(3.604), &[]);
        quiet.sample(Time::ZERO, &row_vin(-0.3004), &[]);
        assert!(quiet.findings().is_empty(), "{:?}", quiet.findings());

        // 3.606 V rounds to 3.61 V > 3.6 V: warns, with unrounded data and rounded message.
        let mut over = Ratings::new(&netlist, &[], &columns, Timing::Point);
        over.sample(Time::ZERO, &row_vin(3.606), &[]);
        let found = over.findings();
        assert_eq!(found.len(), 1, "{found:?}");
        let json = serde_json::to_value(&found[0]).unwrap();
        assert_eq!(
            json["message"],
            "#u1.pin.in reaches 3.61 V over the chip's ground, outside its -0.3 V to 3.6 V rating"
        );
        assert_eq!(json["data"]["peak"], 3.606);
        assert_eq!(json["data"]["high"], 3.600019);

        // -0.301 V rounds to -0.301 V < -0.3 V: warns below floor.
        let mut under = Ratings::new(&netlist, &[], &columns, Timing::Point);
        under.sample(Time::ZERO, &row_vin(-0.301), &[]);
        let found = under.findings();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(
            serde_json::to_value(&found[0]).unwrap()["message"],
            "#u1.pin.in reaches -0.301 V over the chip's ground, outside its -0.3 V to 3.6 V rating"
        );
    }
}
