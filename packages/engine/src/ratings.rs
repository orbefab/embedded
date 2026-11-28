use std::collections::VecDeque;
use std::num::NonZeroU64;

use orbe_model::{Drive, Time};

use crate::active::{self, Output, Stage};
use crate::netlist::{Average, Basis, Model, NetId, Netlist, Rating, RatingKind};

use crate::chip::{
    self,
    regulator::{
        LIMIT_HOLD_PS, LIMIT_KNEE_FRACTION, LIMIT_SAG_FRACTION, MIN_CHARGE_RATE, STALL_PS,
        limit_drop,
    },
};
use crate::diagnostic::{
    BodyDiodeExcursion, CurrentLimitExcursion, Diagnostic, Excursion, SupplyExcursion,
    VoltageExcursion, exceeds_rounded, significant,
};
use crate::finite::Finite;
use crate::name::{Name, Quantity, Ref};
use crate::passive::DRIVE_REST_PS;

use crate::observe::Columns;

mod temperature;

/// Every rated quantity of a netlist, checked against each accepted row.
///
/// It reads the observed row, so a part's rating is checked against the number the run
/// writes. A chip's pin ratings read the row's net voltages and the drive its core and its
/// supply gate had in force for that row, and work out each pin's current the way
/// `active::stamp` stamps it.
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
    supplies: Vec<SupplyWatch>,
    limits: Vec<RegulatorLimit>,
    /// A resistor's operating voltage, checked on the peak of a drive that leaves zero.
    drives: Vec<CoilDrive>,
    /// A powered analog part's maximum operating supply voltage.
    supply_ceilings: Vec<SupplyCeiling>,
    /// MOSFETs whose intrinsic body diode is omitted by the Level-1 model.
    body_diodes: Vec<BodyDiode>,
    chips: Vec<Gauge>,
    /// Each part's operating temperature, read against the world's schedule rather than a row.
    temperatures: Vec<temperature::TemperatureWatch>,
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

/// A supply pin at or below this voltage over ground is treated as unpowered (switched off or
/// disconnected) and stays silent. Anything strictly above it and below `min` is a brown-out
/// and warns `rating.supply_low`.
pub(crate) const SUPPLY_UNPOWERED_V: f64 = 0.1;

/// Minimum picoseconds a low supply voltage must hold continuously to set `rating.supply_low`'s
/// reported voltage (1 us), matching [`LIMIT_HOLD_PS`]. Nanosecond single-clock or switching dips
/// inside a longer low-supply episode do not replace the voltage the rail held.
pub(crate) const SUPPLY_HOLD_PS: u64 = 1_000_000;

/// Minimum picoseconds a MOSFET's body diode must stay forward-biased past its datasheet forward
/// voltage before `mosfet.body_diode` warns (1 us), matching [`LIMIT_HOLD_PS`] and
/// [`SUPPLY_HOLD_PS`]. Single solver steps and nanosecond switching edges stay quiet while
/// reverse-polarity faults, H-bridge dead-time, and inductive kickback warn.
pub(crate) const BODY_DIODE_HOLD_PS: u64 = 1_000_000;

/// A probe to sample: its time, the row there, and the cores and chip states the row was solved
/// with.
pub(crate) type Sample<'a> = (Time, &'a [Finite], &'a [active::Core], &'a [chip::State]);

/// A chip's minimum operating supply voltage on one supply pin over its ground.
pub(crate) struct SupplyMin {
    pub(crate) pin: &'static str,
    pub(crate) net: NetId,
    pub(crate) gnd: NetId,
    /// Volt, greater than 0.
    pub(crate) min: f64,
    /// Clock frequency in Hz when `min` depends on the chip's clock.
    pub(crate) clock_hz: Option<u32>,
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
    pub(crate) supply_mins: Vec<SupplyMin>,
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
    /// The index of its supply gate's state in a row's chip states, when it has one.
    gate: Option<usize>,
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
struct SupplyCeiling {
    pin: Ref,
    vs: usize,
    gnd: usize,
    max: f64,
    max_rounded: f64,
    /// Absolute maximum supply ceiling, rounded. When `v` exceeds it, the abs-max `Window` warns
    /// instead so a steady over-voltage does not emit two `rating.voltage` warnings.
    abs_max_rounded: Option<f64>,
    worst: f64,
    over: bool,
    excursion: Option<VoltageExcursion>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SupplyBand {
    /// At or below `SUPPLY_UNPOWERED_V` (0.1 V): switched off or disconnected.
    Off,
    /// Strictly above `SUPPLY_UNPOWERED_V` and below the rounded minimum.
    Low,
    /// At or above the rounded minimum.
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SupplyEntry {
    /// Already in the low band on the first sample of the run (or operating point).
    Start,
    /// Entered the low band from `Off`.
    Off,
    /// Entered the low band from `On`.
    On,
}

#[derive(Clone, Copy)]
struct SupplySample {
    at: Time,
    v: f64,
}

#[derive(Clone, Copy)]
struct SupplySpan {
    from: Time,
    to: Time,
    v_from: f64,
    v_to: f64,
}

#[derive(Clone)]
struct SupplyEpisode {
    entry: SupplyEntry,
    first: Time,
    /// When this continuous low-band episode began, for measuring [`SUPPLY_HOLD_PS`].
    hold_since: Time,
    /// Previous sample in this continuous low-band episode.
    last: SupplySample,
    /// Steps intersecting the trailing [`SUPPLY_HOLD_PS`] window, oldest first. Only the front
    /// span can straddle `at - SUPPLY_HOLD_PS`.
    spans: VecDeque<SupplySpan>,
    /// Monotone increasing queue of `(span_end, min_sample)` for spans in the trailing window.
    min_q: VecDeque<(Time, SupplySample)>,
    /// Monotone decreasing queue of `(span_end, max_sample)` for spans in the trailing window.
    max_q: VecDeque<(Time, SupplySample)>,
    /// Level held continuously for at least [`SUPPLY_HOLD_PS`] most recently in this episode.
    held: Option<SupplySample>,
    over_ps: Option<u64>,
}

#[derive(Clone)]
struct SupplyWatch {
    pin: Ref,
    column: usize,
    gnd: usize,
    min: f64,
    min_rounded: f64,
    clock_hz: Option<u32>,
    last_band: Option<SupplyBand>,
    episode: Option<SupplyEpisode>,
    excursion: Option<SupplyExcursion>,
    /// Whether `excursion`'s reported voltage came from a continuous hold of at least
    /// [`SUPPLY_HOLD_PS`] rather than a shorter episode.
    excursion_held: bool,
    /// Episodes begun, power cycles included.
    began: u32,
    /// Episodes closed into `excursion`, which leaves out power cycles.
    closed: u32,
}

/// One chip supply pin's visits below its minimum through a probe, which tell a step which
/// episode it has not yet listed. The pin's [`SupplyExcursion`] merges every episode, so its
/// numbers do not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupplyMark {
    /// `#<part>.pin.<pin>`.
    pub pin: Ref,
    /// Episodes begun, power cycles included. The open one, if any, is the last.
    pub began: u32,
    /// Episodes closed with a warning. A power cycle closes without one.
    pub closed: u32,
    /// Whether the last episode is still open.
    pub open: bool,
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

#[derive(Clone)]
struct BodyDiode {
    part: Ref,
    channel: &'static str,
    /// Net column of the intrinsic diode's anode (`source` for `nmos`, `drain` for `pmos`).
    anode: usize,
    /// Net column of the intrinsic diode's cathode (`drain` for `nmos`, `source` for `pmos`).
    cathode: usize,
    vf: f64,
    vf_rounded: f64,
    /// Reverse-bias spell not yet held for [`BODY_DIODE_HOLD_PS`].
    pending: Option<PendingBodyDiode>,
    /// True while the current continuous reverse-bias spell has passed [`BODY_DIODE_HOLD_PS`]
    /// and is accumulating into `excursion`.
    active: bool,
    excursion: Option<BodyDiodeExcursion>,
}

#[derive(Clone)]
struct PendingBodyDiode {
    first: Time,
    peak: f64,
    peak_at: Time,
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
    /// `gates` is, by active slot, the index of its supply gate's state in a row's chip states.
    pub(crate) fn new(
        netlist: &Netlist,
        actives: &[active::Chip],
        gates: &[Option<usize>],
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
                    gate: gates.get(active).copied().flatten(),
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
            match &part.model {
                Model::Diode(junction) | Model::Led(junction) => {
                    if let Some(limit) = &junction.reverse_voltage {
                        let ground = columns.net(junction.pins[1]);
                        windows.push(Window {
                            pin: Ref::Quantity(part.name.clone(), Quantity::V),
                            column: columns.net(junction.pins[0]),
                            vcc: ground,
                            gnd: ground,
                            below: Some(limit.si.get()),
                            above: None,
                            worst: 0.0,
                            over: false,
                            excursion: None,
                        });
                    }
                }
                Model::Potentiometer(pot) => {
                    if let Some(max) = &pot.max_voltage {
                        let limit = max.si.get();
                        let ground = columns.net(pot.pins[0]);
                        windows.push(Window {
                            pin: Ref::Quantity(part.name.clone(), Quantity::V),
                            column: columns.net(pot.pins[2]),
                            vcc: ground,
                            gnd: ground,
                            below: Some(limit),
                            above: Some(Ceiling::Fixed(limit)),
                            worst: 0.0,
                            over: false,
                            excursion: None,
                        });
                    }
                }
                Model::Switch(switch) => {
                    if let Some(max) = &switch.max_voltage {
                        let limit = max.si.get();
                        let ground = columns.net(switch.pins[1]);
                        windows.push(Window {
                            pin: Ref::Quantity(part.name.clone(), Quantity::V),
                            column: columns.net(switch.pins[0]),
                            vcc: ground,
                            gnd: ground,
                            below: Some(limit),
                            above: Some(Ceiling::Fixed(limit)),
                            worst: 0.0,
                            over: false,
                            excursion: None,
                        });
                    }
                }
                Model::Resistor(resistor) => {
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
                _ => {}
            }
        }
        let mut supplies = Vec::new();
        let mut supply_ceilings = Vec::new();
        for (_, part) in netlist.parts() {
            let pin =
                |name: &str| Ref::Pin(part.name.clone(), Name::new(name).expect("a pin name"));
            match &part.model {
                Model::Chip(model) => {
                    let Some(rated) = crate::chip::pin_ratings(model) else {
                        continue;
                    };
                    for min in rated.supply_mins {
                        supplies.push(SupplyWatch {
                            pin: pin(min.pin),
                            column: columns.net(min.net),
                            gnd: columns.net(min.gnd),
                            min: min.min,
                            min_rounded: significant(min.min),
                            clock_hz: min.clock_hz,
                            last_band: None,
                            episode: None,
                            excursion: None,
                            excursion_held: false,
                            began: 0,
                            closed: 0,
                        });
                    }
                }
                Model::OpAmp(op) => {
                    if let Some(min) = &op.min_supply {
                        let (_, vcc, gnd) = op.common_mode();
                        let min = min.si.get();
                        supplies.push(SupplyWatch {
                            pin: pin("vcc"),
                            column: columns.net(vcc),
                            gnd: columns.net(gnd),
                            min,
                            min_rounded: significant(min),
                            clock_hz: None,
                            last_band: None,
                            episode: None,
                            excursion: None,
                            excursion_held: false,
                            began: 0,
                            closed: 0,
                        });
                    }
                }
                Model::AnalogTempSensor(sensor) => {
                    let vs = columns.net(sensor.pins[0]);
                    let gnd = columns.net(sensor.pins[2]);
                    if let Some(min) = &sensor.supply_min {
                        let min = min.si.get();
                        supplies.push(SupplyWatch {
                            pin: pin("vs"),
                            column: vs,
                            gnd,
                            min,
                            min_rounded: significant(min),
                            clock_hz: None,
                            last_band: None,
                            episode: None,
                            excursion: None,
                            excursion_held: false,
                            began: 0,
                            closed: 0,
                        });
                    }
                    if let Some(max) = &sensor.max_supply {
                        windows.push(Window {
                            pin: pin("vs"),
                            column: vs,
                            vcc: gnd,
                            gnd,
                            below: None,
                            above: Some(Ceiling::Fixed(max.si.get())),
                            worst: 0.0,
                            over: false,
                            excursion: None,
                        });
                    }
                    if let Some(max) = &sensor.supply_max {
                        let max = max.si.get();
                        supply_ceilings.push(SupplyCeiling {
                            pin: pin("vs"),
                            vs,
                            gnd,
                            max,
                            max_rounded: significant(max),
                            abs_max_rounded: sensor
                                .max_supply
                                .as_ref()
                                .map(|abs| significant(abs.si.get())),
                            worst: 0.0,
                            over: false,
                            excursion: None,
                        });
                    }
                }
                _ => {}
            }
        }
        let mut body_diodes = Vec::new();
        for (_, part) in netlist.parts() {
            let (channel, mosfet, anode, cathode) = match &part.model {
                Model::Nmos(nmos) => ("nmos", &nmos.0, nmos.0.pins[2], nmos.0.pins[0]),
                Model::Pmos(pmos) => ("pmos", &pmos.0, pmos.0.pins[0], pmos.0.pins[2]),
                _ => continue,
            };
            let Some(vf) = &mosfet.body_diode_forward_voltage else {
                continue;
            };
            let vf = vf.si.get();
            body_diodes.push(BodyDiode {
                part: Ref::Part(part.name.clone()),
                channel,
                anode: columns.net(anode),
                cathode: columns.net(cathode),
                vf,
                vf_rounded: significant(vf),
                pending: None,
                active: false,
                excursion: None,
            });
        }
        Ratings {
            timing,
            watches,
            windows,
            supplies,
            limits,
            drives,
            supply_ceilings,
            body_diodes,
            chips,
            temperatures: temperature::watches(netlist, columns),
            last: None,
            restarted: false,
            decays: Decays::default(),
        }
    }

    /// Seeds each supply watch's initial band from the t = 0 operating point when the rail starts
    /// cleanly off (<= 0.1 V) or in range (>= min), without recording a sample. A rail already in
    /// the low band at t = 0 leaves `last_band` unset so the first accepted sample warns as `Start`.
    pub(crate) fn seed_supplies(&mut self, row: &[Finite]) {
        for supply in &mut self.supplies {
            let (_, band) = supply.read(row);
            if matches!(band, SupplyBand::Off | SupplyBand::On) {
                supply.last_band = Some(band);
            }
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
    /// for `row`, and `states` the chip states. An operating point, [`Timing::Point`], has no
    /// duration, so its DC value fills each filter.
    pub(crate) fn sample(
        &mut self,
        at: Time,
        row: &[Finite],
        cores: &[active::Core],
        states: &[chip::State],
    ) {
        for gauge in &mut self.chips {
            gauge.read(row, &cores[gauge.active], states);
        }
        self.decays.known.clear();
        let (families, mut step) = self.families_mut(at);
        for family in families {
            family.sample(&mut step, row);
        }
        self.last = Some(at);
        self.restarted = false;
    }

    /// Each family [`Self::sample`] checks on a row, and the step they share. A new field of
    /// [`Ratings`] does not compile until it is named here and in [`Self::families`].
    fn families_mut(&mut self, at: Time) -> ([&mut dyn Family; FAMILIES], Step<'_>) {
        let Ratings {
            timing,
            watches,
            windows,
            supplies,
            limits,
            drives,
            supply_ceilings,
            body_diodes,
            chips,
            temperatures: _,
            last,
            restarted,
            decays,
        } = self;
        let step = Step {
            at,
            last: *last,
            restarted: *restarted,
            timing: *timing,
            chips,
            decays,
        };
        (
            [
                watches,
                windows,
                supplies,
                limits,
                drives,
                supply_ceilings,
                body_diodes,
            ],
            step,
        )
    }

    /// The families of [`Self::families_mut`], to read.
    fn families(&self) -> [&dyn Family; FAMILIES] {
        let Ratings {
            watches,
            windows,
            supplies,
            limits,
            drives,
            supply_ceilings,
            body_diodes,
            timing: _,
            chips: _,
            temperatures: _,
            last: _,
            restarted: _,
            decays: _,
        } = self;
        [
            watches,
            windows,
            supplies,
            limits,
            drives,
            supply_ceilings,
            body_diodes,
        ]
    }

    /// One `rating.*` diagnostic per quantity that went over, plus one `regulator.current_limit`
    /// per regulator held at its current limit and one `mosfet.body_diode` per reverse-biased
    /// MOSFET, through the last sample.
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
        let supplies = self
            .supplies
            .iter()
            .filter_map(|supply| Some(Diagnostic::RatingSupplyLow(supply.finding()?)));
        let limits = self
            .limits
            .iter()
            .filter_map(|limit| Some(Diagnostic::RegulatorCurrentLimit(limit.excursion.clone()?)));
        let drives = self
            .drives
            .iter()
            .filter_map(|drive| Some(Diagnostic::RatingVoltage(drive.excursion.clone()?)));
        let ceilings = self
            .supply_ceilings
            .iter()
            .filter_map(|ceiling| Some(Diagnostic::RatingVoltage(ceiling.excursion.clone()?)));
        let body_diodes = self
            .body_diodes
            .iter()
            .filter_map(|diode| Some(Diagnostic::MosfetBodyDiode(diode.excursion.clone()?)));
        currents
            .chain(voltages)
            .chain(supplies)
            .chain(limits)
            .chain(drives)
            .chain(ceilings)
            .chain(body_diodes)
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
    pub(crate) fn now(&self, probe: Option<Sample>) -> Vec<Diagnostic> {
        match self.through(probe) {
            Through::Last => self.found(),
            Through::Probed(ratings) => ratings.found(),
            Through::Quiet => Vec::new(),
        }
    }

    /// Each supply pin's [`SupplyMark`], through `probe` as [`Self::now`] reads it.
    pub(crate) fn supply_marks(&self, probe: Option<Sample>) -> Vec<SupplyMark> {
        match self.through(probe) {
            Through::Last | Through::Quiet => self.marks(),
            Through::Probed(ratings) => ratings.marks(),
        }
    }

    fn found(&self) -> Vec<Diagnostic> {
        let mut found = self.findings();
        found.extend(self.over_now());
        found
    }

    fn marks(&self) -> Vec<SupplyMark> {
        self.supplies
            .iter()
            .map(|supply| SupplyMark {
                pin: supply.pin.clone(),
                began: supply.began,
                closed: supply.closed,
                open: supply.episode.is_some(),
            })
            .collect()
    }

    /// The ratings through `probe`. A probe that is not after the last sample reads them as
    /// they are. When [`Self::quiet_after`] says a sample there would find nothing, nothing is
    /// copied. Otherwise a copy samples it.
    fn through(&self, probe: Option<Sample>) -> Through {
        let Some((at, row, cores, states)) =
            probe.filter(|&(at, ..)| self.last.is_none_or(|last| at > last))
        else {
            return Through::Last;
        };
        let quiet = self.quiet_after(at, row, cores, states);
        if quiet && !cfg!(debug_assertions) {
            return Through::Quiet;
        }
        let mut ratings = self.clone();
        ratings.sample(at, row, cores, states);
        if !quiet {
            return Through::Probed(Box::new(ratings));
        }
        // Only a debug build gets here. It samples the copy anyway, to check the quiet answer.
        let found = ratings.found();
        assert!(found.is_empty(), "a quiet probe found {found:?}");
        assert_eq!(
            ratings.marks(),
            self.marks(),
            "a quiet probe moved a supply mark"
        );
        Through::Quiet
    }

    /// Nothing is over now, and a sample of `row` at `at` would leave nothing over and open no
    /// supply episode: each family is [`Family::quiet`] there. It changes nothing.
    fn quiet_after(
        &self,
        at: Time,
        row: &[Finite],
        cores: &[active::Core],
        states: &[chip::State],
    ) -> bool {
        let mut chips = self.chips.clone();
        for gauge in &mut chips {
            gauge.read(row, &cores[gauge.active], states);
        }
        let mut step = Step {
            at,
            last: self.last,
            restarted: self.restarted,
            timing: self.timing,
            chips: &chips,
            decays: &mut Decays::default(),
        };
        self.families()
            .into_iter()
            .all(|family| family.quiet(&mut step, row))
    }
}

/// What [`Ratings::through`] found at a probe.
enum Through {
    /// The probe is not after the last sample, so the ratings read as they are.
    Last,
    /// A copy that sampled the probe.
    Probed(Box<Ratings>),
    /// A sample at the probe finds nothing over and moves no supply mark.
    Quiet,
}

/// How many families [`Ratings::families`] lists.
const FAMILIES: usize = 7;

/// What one sample's families read besides the row.
struct Step<'a> {
    at: Time,
    /// The sample before, if any.
    last: Option<Time>,
    /// The sample before restarted at an edge, so this one holds back to it.
    restarted: bool,
    timing: Timing,
    /// Each rated chip's currents in the row.
    chips: &'a [Gauge],
    decays: &'a mut Decays,
}

impl Step<'_> {
    /// The sample before when it restarted, which this one holds back to.
    fn back(&self) -> Option<Time> {
        self.restarted.then_some(self.last).flatten()
    }

    /// Adds the time since the sample before to `over_ps` when that sample was `over`.
    fn elapsed(&self, over: bool, over_ps: Option<&mut u64>) {
        if over && let (Some(last), Some(over_ps)) = (self.last, over_ps) {
            *over_ps += self.at.as_ps() - last.as_ps();
        }
    }
}

/// One rated thing a sample checks on its row.
trait Rated {
    fn sample(&mut self, step: &mut Step, row: &[Finite]);

    /// It has nothing to report and no supply episode open, and a sample of `row` would leave
    /// it so. It reads `row` through the same method [`Self::sample`] reads it with, and
    /// changes nothing.
    fn quiet(&self, step: &mut Step, row: &[Finite]) -> bool;
}

/// Every rated thing of one kind.
trait Family {
    fn sample(&mut self, step: &mut Step, row: &[Finite]);
    fn quiet(&self, step: &mut Step, row: &[Finite]) -> bool;
    #[cfg(test)]
    fn len(&self) -> usize;
}

impl<T: Rated> Family for Vec<T> {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        for rated in self {
            rated.sample(step, row);
        }
    }

    fn quiet(&self, step: &mut Step, row: &[Finite]) -> bool {
        self.iter().all(|rated| rated.quiet(step, row))
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        Vec::len(self)
    }
}

/// What a watch reads in one row.
struct WatchReading {
    /// The row's magnitude.
    sample: f64,
    /// A sustained rating's filter after taking it, in a run.
    filter: Option<Filter>,
    /// What the rating checks: the filter's reading, or else the sample.
    reading: f64,
    /// `reading` is over the rating.
    over: bool,
    /// `sample` is over the rating.
    sample_over: bool,
}

impl Watch {
    fn read(&self, step: &mut Step, row: &[Finite]) -> WatchReading {
        let sample = magnitude(self, step.chips, row);
        let filter = match (&self.heat, step.timing) {
            (Some(heat), Timing::Run) => {
                let mut filter = heat.filter.clone();
                filter.take_in(step.at, sample, step.back().is_some(), step.decays);
                Some(filter)
            }
            _ => None,
        };
        let reading = filter.as_ref().map_or(sample, Filter::reading);
        WatchReading {
            sample,
            filter,
            reading,
            over: exceeds_rounded(reading, self.limit_rounded),
            sample_over: exceeds_rounded(sample, self.limit_rounded),
        }
    }
}

impl Rated for Watch {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        step.elapsed(
            self.over,
            self.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
        );
        let WatchReading {
            sample,
            filter,
            reading,
            over,
            sample_over,
        } = self.read(step, row);
        if let (Some(heat), Some(filter)) = (&mut self.heat, filter) {
            if let Some(recent) = &mut heat.recent {
                recent.take(step.at, sample, step.back().is_some());
            }
            heat.filter = filter;
        }
        self.last = sample;
        let from = step
            .back()
            .filter(|_| self.heat.is_some())
            .unwrap_or(step.at);
        self.since = sample_over.then(|| self.since.unwrap_or(from));
        self.over = over;
        if !over {
            return;
        }
        match &mut self.excursion {
            None => {
                self.excursion = Some(Excursion {
                    quantity: self.quantity.clone(),
                    limit: self.limit,
                    peak: reading,
                    peak_at: step.at,
                    first: step.at,
                    over_ps: step.timing.over_ps(),
                    basis: self.basis,
                    sample,
                    held_ps: None,
                    characterized: self.characterized,
                });
            }
            Some(excursion) if reading > excursion.peak => {
                excursion.peak = reading;
                excursion.peak_at = step.at;
                excursion.sample = sample;
            }
            Some(_) => {}
        }
    }

    /// A sample over the rating is not quiet on a watch with an over-now average, which may be
    /// over too, [`Ratings::over_now`].
    fn quiet(&self, step: &mut Step, row: &[Finite]) -> bool {
        if self.excursion.is_some() {
            return false;
        }
        let reading = self.read(step, row);
        let early = self.heat.as_ref().is_some_and(|heat| heat.recent.is_some());
        !reading.over && !(early && reading.sample_over)
    }
}

impl Rated for Window {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        step.elapsed(
            self.over,
            self.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
        );
        let reading = self.read(row);
        self.over = reading.over();
        if !self.over {
            return;
        }
        let WindowReading {
            v,
            low,
            high,
            below,
            above,
        } = reading;
        let past = low
            .filter(|_| below)
            .map_or(f64::NEG_INFINITY, |low| low - v)
            .max(
                high.filter(|_| above)
                    .map_or(f64::NEG_INFINITY, |high| v - high),
            );
        let reading = VoltageExcursion {
            pin: self.pin.clone(),
            low,
            high,
            peak: v,
            peak_at: step.at,
            first: step.at,
            over_ps: step.timing.over_ps(),
            operating: false,
        };
        match &mut self.excursion {
            None => {
                self.worst = past;
                self.excursion = Some(reading);
            }
            Some(excursion) if past > self.worst => {
                self.worst = past;
                excursion.low = reading.low;
                excursion.high = reading.high;
                excursion.peak = v;
                excursion.peak_at = step.at;
            }
            Some(_) => {}
        }
    }

    fn quiet(&self, _: &mut Step, row: &[Finite]) -> bool {
        self.excursion.is_none() && !self.read(row).over()
    }
}

impl Rated for SupplyWatch {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        let (v, band) = self.read(row);
        match band {
            SupplyBand::Low => {
                if let Some(ep) = &mut self.episode {
                    if let (Some(last), Some(over_ps)) = (step.last, &mut ep.over_ps) {
                        *over_ps += step.at.as_ps() - last.as_ps();
                    }
                    ep.push(step.at, v, step.restarted);
                } else {
                    let entry = match self.last_band {
                        None => SupplyEntry::Start,
                        Some(SupplyBand::Off) => SupplyEntry::Off,
                        Some(SupplyBand::On) => SupplyEntry::On,
                        Some(SupplyBand::Low) => unreachable!("episode is open while Low"),
                    };
                    let start = match (step.timing, entry, step.back()) {
                        (Timing::Run, SupplyEntry::Start, _) if step.last.is_none() => Time::ZERO,
                        (Timing::Run, _, Some(last)) => last,
                        _ => step.at,
                    };
                    self.episode =
                        Some(SupplyEpisode::start(entry, start, step.at, v, step.timing));
                    self.began += 1;
                }
            }
            SupplyBand::On | SupplyBand::Off => {
                if let Some(mut ep) = self.episode.take() {
                    if !step.restarted
                        && let (Some(last), Some(over_ps)) = (step.last, &mut ep.over_ps)
                    {
                        *over_ps += step.at.as_ps() - last.as_ps();
                    }
                    let power_cycle = matches!(
                        (ep.entry, band),
                        (SupplyEntry::Off, SupplyBand::On) | (SupplyEntry::On, SupplyBand::Off)
                    );
                    if !power_cycle {
                        self.merge(ep);
                        self.closed += 1;
                    }
                }
            }
        }
        self.last_band = Some(band);
    }

    fn quiet(&self, _: &mut Step, row: &[Finite]) -> bool {
        self.excursion.is_none() && self.episode.is_none() && self.read(row).1 != SupplyBand::Low
    }
}

impl Rated for RegulatorLimit {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        let at = step.at;
        if self.clamped_since.is_some()
            && let Some(last) = step.last
        {
            let dt = at.as_ps() - last.as_ps();
            if let Some(excursion) = &mut self.excursion {
                if let Some(over_ps) = &mut excursion.over_ps {
                    *over_ps += dt;
                }
            } else if let Some(pending) = &mut self.pending
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
        } = self.read(row);
        if !clamped {
            self.clamped_since = None;
            self.last_clamped = None;
            self.hold.clear();
            self.pending = None;
            if drop < LIMIT_KNEE_FRACTION * vout {
                self.baseline_vo = Some(vo);
            }
            return;
        }
        let sample = ClampedSample { at, vo, current: i };
        let since = *self.clamped_since.get_or_insert(at);
        if let Some(prev) = self.last_clamped.replace(sample) {
            let high = if sample.vo >= prev.vo { sample } else { prev };
            while self.hold.back().is_some_and(|(_, back)| back.vo <= high.vo) {
                self.hold.pop_back();
            }
            self.hold.push_back((at, high));
            let cutoff = at.as_ps().saturating_sub(LIMIT_HOLD_PS);
            while self
                .hold
                .front()
                .is_some_and(|(to, _)| to.as_ps() <= cutoff)
            {
                self.hold.pop_front();
            }
        }
        let held = (at.as_ps() - since.as_ps() >= LIMIT_HOLD_PS)
            .then(|| self.hold.front().map(|(_, s)| *s))
            .flatten();
        if let Some(excursion) = &mut self.excursion {
            if let Some(peak) = held
                && peak.vo < excursion.voltage
            {
                excursion.voltage = peak.vo;
                excursion.current = peak.current;
                excursion.peak_at = peak.at;
            }
            return;
        }
        let pulled = self.baseline_vo.is_some_and(|before| before - vo >= sag);
        let warn = if step.timing == Timing::Point {
            true
        } else if let Some(pending) = &mut self.pending {
            if pulled {
                let since = *pending.pulled_since.get_or_insert(at);
                at.as_ps() - since.as_ps() >= LIMIT_HOLD_PS
            } else {
                pending.pulled_since = None;
                let dt_ps = at.as_ps() - pending.progress_at.as_ps();
                if dt_ps >= STALL_PS {
                    if vo - pending.progress_vo >= MIN_CHARGE_RATE * drop * (dt_ps as f64 * 1e-12) {
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
            self.pending = Some(PendingLimit {
                first: at,
                over_ps: step.timing.over_ps(),
                pulled_since: pulled.then_some(at),
                progress_vo: vo,
                progress_at: at,
            });
            false
        };
        if warn {
            let (first, over_ps) = match self.pending.take() {
                Some(pending) => (pending.first, pending.over_ps),
                None => (at, step.timing.over_ps()),
            };
            let peak = held.unwrap_or(sample);
            self.excursion = Some(CurrentLimitExcursion {
                pin: self.pin.clone(),
                limit: self.reg.current_limit.si.get(),
                target: vout,
                voltage: peak.vo,
                current: peak.current,
                peak_at: peak.at,
                first,
                over_ps,
            });
        }
    }

    /// A clamped regulator is not quiet even before it warns: whether it ever does depends on
    /// the samples after.
    fn quiet(&self, _: &mut Step, row: &[Finite]) -> bool {
        self.excursion.is_none() && !self.read(row).clamped
    }
}

impl CoilDrive {
    /// Whether a sample at `volts` judges the pulse: a peak that rises past the window's top, or
    /// a quiet sample that ends the pulse.
    fn judges(&self, volts: f64) -> bool {
        if volts >= DRIVE_QUIET_V {
            volts > self.peak && exceeds_rounded(volts, self.max_rounded)
        } else {
            self.peak >= DRIVE_QUIET_V
        }
    }
}

impl Rated for CoilDrive {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        let at = step.at;
        let volts = row[self.column].get().abs();
        let loud = volts >= DRIVE_QUIET_V;
        let judges = self.judges(volts);
        // A quiet sample judges exactly when it ends a pulse.
        let ended = !loud && judges;
        if loud {
            self.pulse_from.get_or_insert(step.last.unwrap_or(at));
            self.loud_at = Some(at);
            self.peak = self.peak.max(volts);
        }
        if judges {
            judge_drive(self, at, step.timing.over_ps());
        }
        // A pulse that ends counts to its last loud sample. The quiet sample after it can
        // come a whole step later.
        let through = if loud { Some(at) } else { self.loud_at };
        if self.over
            && (loud || ended)
            && let Some(through) = through
        {
            if let Some(over_ps) = self.excursion.as_mut().and_then(|e| e.over_ps.as_mut()) {
                *over_ps += through.as_ps().saturating_sub(self.counted_to.as_ps());
            }
            self.counted_to = self.counted_to.max(through);
        }
        if !loud {
            self.peak = 0.0;
            self.pulse_from = None;
            let rested = self
                .loud_at
                .is_none_or(|loud_at| at.as_ps() - loud_at.as_ps() > DRIVE_REST_PS);
            if rested {
                self.over = false;
            }
        }
    }

    fn quiet(&self, _: &mut Step, row: &[Finite]) -> bool {
        self.excursion.is_none() && !self.judges(row[self.column].get().abs())
    }
}

impl SupplyCeiling {
    /// The supply over ground in `row`, and whether it is over the operating ceiling but not
    /// past the absolute maximum, where the window warns instead.
    fn read(&self, row: &[Finite]) -> (f64, bool) {
        let v = row[self.vs].get() - row[self.gnd].get();
        let over_operating = exceeds_rounded(v, self.max_rounded);
        let over_abs = self
            .abs_max_rounded
            .is_some_and(|abs| exceeds_rounded(v, abs));
        (v, over_operating && !over_abs)
    }
}

impl Rated for SupplyCeiling {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        step.elapsed(
            self.over,
            self.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
        );
        let (v, over) = self.read(row);
        self.over = over;
        if !over {
            return;
        }
        let past = v - self.max;
        match &mut self.excursion {
            None => {
                self.worst = past;
                self.excursion = Some(VoltageExcursion {
                    pin: self.pin.clone(),
                    low: None,
                    high: Some(self.max),
                    peak: v,
                    peak_at: step.at,
                    first: step.at,
                    over_ps: step.timing.over_ps(),
                    operating: true,
                });
            }
            Some(excursion) if past > self.worst => {
                self.worst = past;
                excursion.peak = v;
                excursion.peak_at = step.at;
            }
            Some(_) => {}
        }
    }

    fn quiet(&self, _: &mut Step, row: &[Finite]) -> bool {
        self.excursion.is_none() && !self.read(row).1
    }
}

impl BodyDiode {
    /// Forward bias across the missing intrinsic body diode (`V(source) - V(drain)` for `nmos`,
    /// `V(drain) - V(source)` for `pmos`) and whether it exceeds `vf` after rounding.
    fn read(&self, row: &[Finite]) -> (f64, bool) {
        let v = row[self.anode].get() - row[self.cathode].get();
        (v, exceeds_rounded(v, self.vf_rounded))
    }
}

impl Rated for BodyDiode {
    fn sample(&mut self, step: &mut Step, row: &[Finite]) {
        let (v, over) = self.read(row);
        if step.timing == Timing::Point {
            if over {
                self.excursion = Some(BodyDiodeExcursion {
                    part: self.part.clone(),
                    channel: self.channel,
                    forward_voltage: self.vf,
                    peak: v,
                    peak_at: step.at,
                    first: step.at,
                    over_ps: None,
                });
            }
            return;
        }
        if !over {
            if self.active {
                if !step.restarted
                    && let (Some(last), Some(over_ps)) = (
                        step.last,
                        self.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
                    )
                {
                    *over_ps += step.at.as_ps() - last.as_ps();
                }
                self.active = false;
            }
            self.pending = None;
            return;
        }
        if self.active {
            if let Some(excursion) = &mut self.excursion {
                if let (Some(last), Some(over_ps)) = (step.last, &mut excursion.over_ps) {
                    *over_ps += step.at.as_ps() - last.as_ps();
                }
                if v > excursion.peak {
                    excursion.peak = v;
                    excursion.peak_at = step.at;
                }
            }
            return;
        }
        let pending = self.pending.get_or_insert_with(|| PendingBodyDiode {
            first: step.back().unwrap_or(step.at),
            peak: v,
            peak_at: step.at,
        });
        if v > pending.peak {
            pending.peak = v;
            pending.peak_at = step.at;
        }
        let held_ps = step.at.as_ps().saturating_sub(pending.first.as_ps());
        if held_ps >= BODY_DIODE_HOLD_PS {
            let pending = self.pending.take().expect("pending was just inserted");
            self.active = true;
            match &mut self.excursion {
                None => {
                    self.excursion = Some(BodyDiodeExcursion {
                        part: self.part.clone(),
                        channel: self.channel,
                        forward_voltage: self.vf,
                        peak: pending.peak,
                        peak_at: pending.peak_at,
                        first: pending.first,
                        over_ps: Some(held_ps),
                    });
                }
                Some(excursion) => {
                    if let Some(over_ps) = &mut excursion.over_ps {
                        *over_ps += held_ps;
                    }
                    if pending.peak > excursion.peak {
                        excursion.peak = pending.peak;
                        excursion.peak_at = pending.peak_at;
                    }
                }
            }
        }
    }

    fn quiet(&self, _: &mut Step, row: &[Finite]) -> bool {
        self.excursion.is_none() && !self.read(row).1
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

impl WindowReading {
    fn over(&self) -> bool {
        self.below || self.above
    }
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

impl SupplySpan {
    /// Min and max samples of this span over `[from.max(cutoff), to]`, linearly interpolating at
    /// `cutoff` when the span straddles it.
    fn bounds_since(&self, cutoff: u64) -> (SupplySample, SupplySample) {
        let (left_at, v_left) = if self.from.as_ps() < cutoff {
            let dt = (self.to.as_ps() - self.from.as_ps()) as f64;
            let rem = (self.to.as_ps() - cutoff) as f64;
            let v = self.v_to - (self.v_to - self.v_from) * (rem / dt);
            (Time::from_ps(cutoff), v)
        } else {
            (self.from, self.v_from)
        };
        let left = SupplySample {
            at: left_at,
            v: v_left,
        };
        let right = SupplySample {
            at: self.to,
            v: self.v_to,
        };
        if v_left < self.v_to {
            (left, right)
        } else if v_left > self.v_to {
            (right, left)
        } else {
            (right, right)
        }
    }
}

impl SupplyEpisode {
    fn start(entry: SupplyEntry, start: Time, at: Time, v: f64, timing: Timing) -> SupplyEpisode {
        let mut ep = SupplyEpisode {
            entry,
            first: start,
            hold_since: start,
            last: SupplySample { at, v },
            spans: VecDeque::new(),
            min_q: VecDeque::new(),
            max_q: VecDeque::new(),
            held: None,
            over_ps: timing.over_ps().map(|_| at.as_ps() - start.as_ps()),
        };
        if at > start {
            ep.record_span(SupplySpan {
                from: start,
                to: at,
                v_from: v,
                v_to: v,
            });
        }
        ep
    }

    fn push(&mut self, at: Time, v: f64, restarted: bool) {
        let prev = self.last;
        self.last = SupplySample { at, v };
        if at <= prev.at {
            return;
        }
        let v_from = if restarted { v } else { prev.v };
        self.record_span(SupplySpan {
            from: prev.at,
            to: at,
            v_from,
            v_to: v,
        });
    }

    fn record_span(&mut self, span: SupplySpan) {
        let (s_lo, s_hi) = span.bounds_since(span.from.as_ps());
        self.spans.push_back(span);
        while self.min_q.back().is_some_and(|(_, back)| back.v >= s_lo.v) {
            self.min_q.pop_back();
        }
        self.min_q.push_back((span.to, s_lo));
        while self.max_q.back().is_some_and(|(_, back)| back.v <= s_hi.v) {
            self.max_q.pop_back();
        }
        self.max_q.push_back((span.to, s_hi));

        let at = span.to;
        let cutoff = at.as_ps().saturating_sub(SUPPLY_HOLD_PS);
        while self.spans.front().is_some_and(|s| s.to.as_ps() <= cutoff) {
            self.spans.pop_front();
        }
        let drop_to = match self.spans.front() {
            Some(front) if front.from.as_ps() < cutoff => front.to.as_ps(),
            _ => cutoff,
        };
        while self
            .min_q
            .front()
            .is_some_and(|(to, _)| to.as_ps() <= drop_to)
        {
            self.min_q.pop_front();
        }
        while self
            .max_q
            .front()
            .is_some_and(|(to, _)| to.as_ps() <= drop_to)
        {
            self.max_q.pop_front();
        }

        if at.as_ps() - self.hold_since.as_ps() >= SUPPLY_HOLD_PS
            && let Some((lo, hi)) = self.window_bounds(cutoff)
        {
            self.held = Some(match self.held {
                None => hi,
                Some(cur) if hi.v < cur.v => hi,
                Some(cur) if lo.v > cur.v => lo,
                Some(cur) => cur,
            });
        }
    }

    /// Min and max samples across `[at - SUPPLY_HOLD_PS, at]` (`cutoff = at - SUPPLY_HOLD_PS`).
    fn window_bounds(&self, cutoff: u64) -> Option<(SupplySample, SupplySample)> {
        let front = self.spans.front()?;
        if front.from.as_ps() < cutoff {
            let (mut lo, mut hi) = front.bounds_since(cutoff);
            if let Some(&(_, q_lo)) = self.min_q.front()
                && q_lo.v <= lo.v
            {
                lo = q_lo;
            }
            if let Some(&(_, q_hi)) = self.max_q.front()
                && q_hi.v >= hi.v
            {
                hi = q_hi;
            }
            Some((lo, hi))
        } else {
            Some((self.min_q.front()?.1, self.max_q.front()?.1))
        }
    }

    fn reading(&self) -> (SupplySample, bool) {
        match self.held {
            Some(held) => (held, true),
            None => (self.max_q.front().map_or(self.last, |&(_, s)| s), false),
        }
    }
}

/// Whether a newer `rating.supply_low` reading replaces the existing one: any reading held for at
/// least [`SUPPLY_HOLD_PS`] beats an unheld shorter-episode reading, and ties take the newer
/// reading.
fn replaces_supply_voltage(cur_held: bool, new_held: bool) -> bool {
    !cur_held || new_held
}

impl SupplyWatch {
    fn read(&self, row: &[Finite]) -> (f64, SupplyBand) {
        let v = row[self.column].get() - row[self.gnd].get();
        let band = if v <= SUPPLY_UNPOWERED_V {
            SupplyBand::Off
        } else if v < self.min && (v < self.min * 0.99 || significant(v) < self.min_rounded) {
            SupplyBand::Low
        } else {
            SupplyBand::On
        };
        (v, band)
    }

    fn excursion_from(&self, ep: &SupplyEpisode) -> (SupplyExcursion, bool) {
        let (reading, held) = ep.reading();
        (
            SupplyExcursion {
                pin: self.pin.clone(),
                min: self.min,
                clock_hz: self.clock_hz,
                voltage: reading.v,
                peak_at: reading.at,
                first: ep.first,
                over_ps: ep.over_ps,
            },
            held,
        )
    }

    fn merge(&mut self, ep: SupplyEpisode) {
        let (reading, held) = ep.reading();
        match &mut self.excursion {
            None => {
                let (excursion, held) = self.excursion_from(&ep);
                self.excursion = Some(excursion);
                self.excursion_held = held;
            }
            Some(existing) => {
                if replaces_supply_voltage(self.excursion_held, held) {
                    existing.voltage = reading.v;
                    existing.peak_at = reading.at;
                    self.excursion_held = held;
                }
                if let (Some(total), Some(add)) = (&mut existing.over_ps, ep.over_ps) {
                    *total += add;
                }
            }
        }
    }

    fn finding(&self) -> Option<SupplyExcursion> {
        match (&self.excursion, &self.episode) {
            (None, None) => None,
            (Some(ex), None) => Some(ex.clone()),
            (None, Some(ep)) => Some(self.excursion_from(ep).0),
            (Some(ex), Some(ep)) => {
                let mut merged = ex.clone();
                let (reading, held) = ep.reading();
                if replaces_supply_voltage(self.excursion_held, held) {
                    merged.voltage = reading.v;
                    merged.peak_at = reading.at;
                }
                if let (Some(total), Some(add)) = (&mut merged.over_ps, ep.over_ps) {
                    *total += add;
                }
                Some(merged)
            }
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
    /// sources with its gate's state in `states`, and the sums through the supply and ground
    /// pins.
    fn read(&mut self, row: &[Finite], core: &active::Core, states: &[chip::State]) {
        let released = self
            .gate
            .is_some_and(|slot| states[slot] == chip::State::Supply(active::Powered::Off));
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
            // The same release as `active::stamp`: with its gate off the stage is off.
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
            let draw = if released {
                core.released_draw()
            } else {
                core.draw()
            };
            let (amps, _) = draw.at(row[vcc].get() - row[gnd].get());
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
        (Ratings::new(&netlist, &[], &[], &columns, timing), columns)
    }

    const TAU: f64 = DEFAULT_TAU_PS as f64;

    #[test]
    fn exactly_the_limit_is_not_over() {
        let (mut ratings, columns) = ratings(Timing::Run);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.03), &[], &[]);
        ratings.sample(Time::from_ps(1), &values(-0.03), &[], &[]);
        assert!(ratings.findings().is_empty());
    }

    #[test]
    fn a_reverse_current_counts_by_magnitude_and_the_peak_follows_later_samples() {
        let (mut ratings, columns) = ratings(Timing::Point);
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.01), &[], &[]);
        ratings.sample(Time::from_ps(10), &values(-0.04), &[], &[]);
        ratings.sample(Time::from_ps(20), &values(0.05), &[], &[]);
        ratings.sample(Time::from_ps(30), &values(0.045), &[], &[]);
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
        ratings.sample(Time::ZERO, &values(0.01), &[], &[]);
        ratings.sample(Time::from_ps(10), &values(0.04), &[], &[]);
        ratings.sample(Time::from_ps(25), &values(0.05), &[], &[]);
        ratings.sample(Time::from_ps(30), &values(0.02), &[], &[]);
        ratings.sample(Time::from_ps(70), &values(0.035), &[], &[]);
        ratings.sample(Time::from_ps(75), &values(0.031), &[], &[]);
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
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        ratings.sample(Time::from_ps(1), &values(0.069), &[], &[]);
        let step = DEFAULT_TAU_PS / 1_000;
        for k in 1..=5_000 {
            ratings.sample(Time::from_ps(1 + k * step), &values(0.069), &[], &[]);
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
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        let start = 5 * DEFAULT_TAU_PS;
        let mut at = 1;
        while at < start {
            ratings.sample(Time::from_ps(at), &values(0.02), &[], &[]);
            at += step;
        }
        let level = 0.02 * -libm::expm1(-((start - 1) as f64) / TAU);
        // Once the last 2.04 ms average is over too, 1 ms on, the rating is over now.
        for k in 0..=100 {
            ratings.sample(Time::from_ps(start + k * step), &values(0.05), &[], &[]);
        }
        assert!(ratings.findings().is_empty());
        let [excursion] = over_now(&ratings).try_into().unwrap();
        let now = 0.05 + (level - 0.05) * libm::exp(-0.1f64);
        let held = TAU * libm::log((0.05 - now) / (0.05 - 0.03));
        let got = excursion.held_ps.unwrap().get() as f64;
        assert!((got / held - 1.0).abs() < 1e-3, "{got} against {held}");
        for k in 101..=2_000 {
            ratings.sample(Time::from_ps(start + k * step), &values(0.05), &[], &[]);
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
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        let mut start = 1;
        while start + period <= stop {
            ratings.sample(Time::from_ps(start), &values(peak), &[], &[]);
            ratings.sample(Time::from_ps(start + high), &values(0.0), &[], &[]);
            start += period;
        }
        ratings.sample(Time::from_ps(start), &values(0.0), &[], &[]);
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
        ratings.sample(Time::ZERO, &row(&columns, 0.0), &[], &[]);
        ratings.sample(Time::from_ps(9), &row(&columns, 0.5), &[], &[]);
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
        (
            Ratings::new(&netlist, &[], &[], &columns, Timing::Run),
            columns,
        )
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
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        ratings.sample(Time::from_ps(2_000), &values(3.008), &[], &[]);
        ratings.sample(Time::from_ps(2_000 + 136_000), &values(0.0), &[], &[]);
        ratings.sample(Time::from_ps(DEFAULT_TAU_PS), &values(0.0), &[], &[]);
        assert!(ratings.findings().is_empty());
    }

    /// A steady 3 W from 0 crosses 2.419 W at `tau * ln(3 / (3 - 2.425))`, since the rating
    /// rounds to 2.42 W and the reading is over once it rounds above that.
    #[test]
    fn a_held_power_overload_crosses_by_its_share_of_the_time_constant() {
        let (mut ratings, columns) = power_ratings(2.419);
        let values = |power| row_power(&columns, power);
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        ratings.sample(Time::from_ps(1), &values(3.0), &[], &[]);
        let step = DEFAULT_TAU_PS / 1_000;
        for k in 1..=5_000 {
            ratings.sample(Time::from_ps(1 + k * step), &values(3.0), &[], &[]);
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
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        let mut start = 1;
        for _ in 0..periods {
            ratings.sample(Time::from_ps(start), &values(peak), &[], &[]);
            ratings.sample(Time::from_ps(start + high), &values(0.0), &[], &[]);
            start += PWM_490_PS;
        }
        ratings.sample(Time::from_ps(start), &values(0.0), &[], &[]);
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
            ratings.sample(Time::from_ps(k * MS), &values(0.0), &[], &[]);
        }
        let start = 50 * MS;
        for k in 0..=20 {
            ratings.sample(Time::from_ps(start + k * MS / 10), &values(0.989), &[], &[]);
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
        ratings.sample(Time::from_ps(1), &row_power(&columns, 2.5), &[], &[]);
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
            ratings.sample(Time::from_ps(k * MS), &values(0.0), &[], &[]);
        }
        ratings.sample(Time::from_ps(10 * MS + 2_000), &values(3.008), &[], &[]);
        let inside = values(3.0);
        let probe = Some((
            Time::from_ps(10 * MS + 70_000),
            inside.as_slice(),
            &[][..],
            &[][..],
        ));
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
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        let mut start = DEFAULT_TAU_PS;
        for _ in 0..20 {
            ratings.sample(Time::from_ps(start), &values(0.05), &[], &[]);
            for k in 1..=10 {
                ratings.sample(
                    Time::from_ps(start + k * high / 10),
                    &values(0.05),
                    &[],
                    &[],
                );
                assert!(over_now(&ratings).is_empty(), "at {start} + {k}");
            }
            ratings.sample(Time::from_ps(start + high), &values(0.0), &[], &[]);
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
            ratings.sample(Time::from_ps(k * MS), &values(0.0), &[], &[]);
        }
        ratings.restarted();
        ratings.sample(Time::from_ps(51 * MS), &values(0.989), &[], &[]);
        let [excursion] = over_now(&ratings).try_into().unwrap();
        assert_eq!(excursion.first, Time::from_ps(50 * MS));
        let rise = |ms: f64| 0.989 * -libm::expm1(-ms * MS as f64 / TAU);
        assert!((excursion.peak / rise(1.0) - 1.0).abs() < 1e-9);
        ratings.sample(Time::from_ps(52 * MS), &values(0.989), &[], &[]);
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
        (
            Ratings::new(&netlist, &[], &[], &columns, Timing::Run),
            columns,
        )
    }

    /// The sheet's pulse, 0.1 ms at 1/10 duty, at its 160 mA peak, and a held 2 ms pulse at
    /// 100 mA, each over the 30 mA rating: no sample is over now and nothing warns, since the
    /// peak limit bounds a pulse in place of over now.
    #[test]
    fn a_pulse_under_its_peak_limit_is_never_over_now() {
        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        let tenth = MS / 10;
        for period in 1..=30 {
            let start = period * MS;
            ratings.sample(Time::from_ps(start), &values(0.16), &[], &[]);
            ratings.sample(Time::from_ps(start + tenth / 2), &values(0.16), &[], &[]);
            assert!(over_now(&ratings).is_empty(), "period {period}");
            ratings.sample(Time::from_ps(start + tenth), &values(0.0), &[], &[]);
        }
        assert_eq!(ratings.findings(), []);
        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        for k in 1..=20 {
            ratings.sample(Time::from_ps(MS + k * tenth), &values(0.1), &[], &[]);
            assert!(over_now(&ratings).is_empty(), "at {k}");
        }
    }

    /// A sample over the peak limit warns at once on its peak basis. A held load under it still
    /// warns when the filter crosses, `tau * ln(50 / 20)` after it starts.
    #[test]
    fn a_rating_with_a_peak_limit_warns_on_either() {
        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        ratings.sample(Time::from_ps(MS), &values(0.2), &[], &[]);
        let found = ratings.findings();
        let [Diagnostic::RatingCurrent(peak)] = found.as_slice() else {
            panic!("not one current finding: {found:?}");
        };
        assert_eq!(peak.basis, Basis::Peak);
        assert_eq!(peak.limit, 0.16);
        close(peak.peak, 0.2);

        let (mut ratings, columns) = with_peak();
        let values = |current| row(&columns, current);
        ratings.sample(Time::ZERO, &values(0.0), &[], &[]);
        let step = MS / 100;
        for k in 0..=300 {
            ratings.sample(Time::from_ps(MS + k * step), &values(0.05), &[], &[]);
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
        ratings.sample(Time::ZERO, &row(&columns, 0.0), &[], &[]);
        ratings.sample(Time::from_ps(MS), &row(&columns, 0.031), &[], &[]);
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
        (
            Ratings::new(&netlist, &[], &[], &columns, Timing::Run),
            columns,
        )
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
        ratings.sample(Time::ZERO, &values(0.0, 0.0), &[], &[]);
        ratings.sample(Time::from_ps(MS), &values(4.0, 16.1), &[], &[]);
        ratings.sample(Time::from_ps(MS + 50_000_000), &values(4.0, 16.1), &[], &[]);
        assert!(over_now(&ratings).is_empty(), "{:?}", over_now(&ratings));
        assert!(ratings.findings().is_empty());

        let (mut ratings, columns) = diode_power();
        let values = |current, power| row_at(&columns, current, power);
        ratings.sample(Time::ZERO, &values(0.0, 0.0), &[], &[]);
        ratings.sample(Time::from_ps(MS), &values(5.0, 20.0), &[], &[]);
        // 100 us later the recent average is over and the 10.2 ms filter has not crossed.
        // The current peak has already warned, so power is still not over now.
        ratings.sample(
            Time::from_ps(MS + 100_000_000),
            &values(5.0, 20.0),
            &[],
            &[],
        );
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
        ratings.sample(Time::ZERO, &values(0.0, 0.0), &[], &[]);
        let step = MS / 10;
        for k in 0..=200 {
            ratings.sample(Time::from_ps(MS + k * step), &values(0.5, 0.664), &[], &[]);
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
        let mut quiet = Ratings::new(&netlist, &[], &[], &columns, Timing::Point);
        quiet.sample(Time::ZERO, &row_vin(3.604), &[], &[]);
        quiet.sample(Time::ZERO, &row_vin(-0.3004), &[], &[]);
        assert!(quiet.findings().is_empty(), "{:?}", quiet.findings());

        // 3.606 V rounds to 3.61 V > 3.6 V: warns, with unrounded data and rounded message.
        let mut over = Ratings::new(&netlist, &[], &[], &columns, Timing::Point);
        over.sample(Time::ZERO, &row_vin(3.606), &[], &[]);
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
        let mut under = Ratings::new(&netlist, &[], &[], &columns, Timing::Point);
        under.sample(Time::ZERO, &row_vin(-0.301), &[], &[]);
        let found = under.findings();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(
            serde_json::to_value(&found[0]).unwrap()["message"],
            "#u1.pin.in reaches -0.301 V over the chip's ground, outside its -0.3 V to 3.6 V rating"
        );
    }

    /// A part in every family: a diode's peak current and a resistor's sustained power are
    /// watches, that resistor's maximum voltage is a window, then a sensor's supply minimum, a
    /// regulator's current limit, a coil's operating drive, the sensor's operating ceiling, and
    /// a MOSFET's body diode.
    fn every_family() -> (Ratings, Columns) {
        let netlist = crate::netlist::parse(
            br#"{
          "version": 1,
          "seed": 0,
          "analysis": { "op": {} },
          "ground": "gnd",
          "nets": ["gnd", "a", "c", "vin", "vout", "vs", "sense", "md", "ms"],
          "world": { "temperature": { "value": 25.0, "unit": "degC" } },
          "parts": [
            {
              "name": "d1",
              "kind": "diode",
              "pins": { "anode": "a", "cathode": "gnd" },
              "saturation_current": { "value": 1e-14, "unit": "A" },
              "emission_coefficient": { "value": 1.0, "unit": "1" },
              "series_resistance": { "value": 1.0, "unit": "ohm" },
              "ratings": { "current": { "value": 0.03, "unit": "A" }, "power": null }
            },
            {
              "name": "r1",
              "kind": "resistor",
              "pins": { "a": "a", "b": "gnd" },
              "resistance": { "value": 10.0, "unit": "ohm" },
              "ratings": {
                "power": { "value": 0.25, "unit": "W" },
                "voltage": { "value": 10.0, "unit": "V" }
              }
            },
            {
              "name": "r2",
              "kind": "resistor",
              "pins": { "a": "c", "b": "gnd" },
              "resistance": { "value": 42.0, "unit": "ohm" },
              "ratings": { "power": null, "voltage": {
                "min": { "value": 3.0, "unit": "V" },
                "max": { "value": 5.0, "unit": "V" } } }
            },
            {
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
              "ratings": { "current": null, "power": null }
            },
            {
              "name": "s1",
              "kind": "analog_temp_sensor",
              "pins": { "vs": "vs", "vout": "sense", "gnd": "gnd" },
              "output_voltage": { "value": 0.75, "unit": "V" },
              "output_resistance": { "value": 60.0, "unit": "ohm" },
              "quiescent_current": { "value": 5e-5, "unit": "A" },
              "ratings": {
                "supply_min": { "value": 2.7, "unit": "V" },
                "supply_max": { "value": 5.5, "unit": "V" },
                "supply_voltage": { "value": 7.0, "unit": "V" }
              }
            },
            {
              "name": "m1",
              "kind": "nmos",
              "pins": { "drain": "md", "gate": "gnd", "source": "ms" },
              "transconductance_parameter": { "value": 0.3236, "unit": "A/V^2" },
              "threshold_voltage": { "value": 1.3, "unit": "V" },
              "channel_length_modulation": { "value": 0.0, "unit": "1/V" },
              "gate_source_capacitance": { "value": 2.1e-11, "unit": "F" },
              "gate_drain_capacitance": { "value": 6e-12, "unit": "F" },
              "drain_source_capacitance": { "value": 7e-12, "unit": "F" },
              "body_diode_forward_voltage": { "value": 0.8, "unit": "V" },
              "ratings": { "current": null, "power": null }
            }
          ]
        }"#,
        )
        .unwrap();
        let columns = Columns::new(&netlist);
        (
            Ratings::new(&netlist, &[], &[], &columns, Timing::Run),
            columns,
        )
    }

    /// A row with each named column set and every other column 0, which is quiet in every
    /// family.
    fn row_of(columns: &Columns, set: &[(&str, f64)]) -> Vec<Finite> {
        let mut row = vec![Finite::new(0.0).unwrap(); columns.refs().len()];
        for &(name, value) in set {
            row[columns.index(&name.parse().unwrap()).unwrap()] = Finite::new(value).unwrap();
        }
        row
    }

    /// Takes `rows` 1 us apart. Before each, a probe of it must read as a copy that samples it:
    /// the same warnings from `now` and the same supply marks. Returns each probe's warnings.
    fn probe_each(ratings: &mut Ratings, rows: &[Vec<Finite>]) -> Vec<Vec<Diagnostic>> {
        let mut found = Vec::new();
        for (k, row) in rows.iter().enumerate() {
            let at = Time::from_ps((k as u64 + 1) * 1_000_000);
            let mut copy = ratings.clone();
            copy.sample(at, row, &[], &[]);
            let probe = Some((at, row.as_slice(), &[][..], &[][..]));
            let now = ratings.now(probe);
            assert_eq!(now, copy.found(), "warnings at row {k}");
            assert_eq!(
                ratings.supply_marks(probe),
                copy.marks(),
                "marks at row {k}"
            );
            ratings.sample(at, row, &[], &[]);
            found.push(now);
        }
        found
    }

    /// In every family, a live probe reads as a sample: while all is quiet, when the family goes
    /// over, and as it recovers. A release build answers a quiet probe without sampling, so a
    /// family the quiet check skipped would lose its warning here, and a debug build fails its
    /// own check of the quiet answer.
    #[test]
    fn a_probe_reads_as_a_sample_in_every_family() {
        let (ratings, columns) = every_family();
        for (k, family) in ratings.families().iter().enumerate() {
            assert!(family.len() > 0, "family {k} has no part here");
        }
        let row = |set: &[(&str, f64)]| row_of(&columns, set);
        let idle = row(&[]);
        let held = [("#net.vin", 5.0), ("#net.vout", 3.3), ("#u1.iout", -0.01)];
        let clamped = [("#net.vin", 5.0), ("#net.vout", 1.0), ("#u1.iout", -2.0)];
        // Each family's index in `families`, its rows, and the ref its warning names.
        let scenarios: [(usize, Vec<Vec<Finite>>, &str); 8] = [
            (0, vec![row(&[("#d1.i", 0.05)]), idle.clone()], "#d1.i"),
            (
                0,
                vec![row(&[("#r1.p", 1.0)]), row(&[("#r1.p", 1.0)]), idle.clone()],
                "#r1.p",
            ),
            (1, vec![row(&[("#net.a", 20.0)]), idle.clone()], "#r1.v"),
            (
                2,
                vec![
                    row(&[("#net.vs", 5.0)]),
                    row(&[("#net.vs", 2.0)]),
                    row(&[("#net.vs", 2.0)]),
                    row(&[("#net.vs", 5.0)]),
                ],
                "#s1.pin.vs",
            ),
            (
                3,
                vec![
                    row(&held),
                    row(&clamped),
                    row(&clamped),
                    row(&clamped),
                    row(&held),
                ],
                "#u1.pin.out",
            ),
            (
                4,
                vec![row(&[("#r2.v", 6.0)]), idle.clone(), idle.clone()],
                "#r2.v",
            ),
            (
                5,
                vec![row(&[("#net.vs", 6.0)]), idle.clone()],
                "#s1.pin.vs",
            ),
            (
                6,
                vec![
                    row(&[("#net.ms", 2.0)]),
                    row(&[("#net.ms", 2.0)]),
                    idle.clone(),
                ],
                "#m1",
            ),
        ];
        let covered: std::collections::BTreeSet<usize> =
            scenarios.iter().map(|(family, ..)| *family).collect();
        assert_eq!(covered, (0..FAMILIES).collect(), "a family has no scenario");
        for (family, rows, named) in scenarios {
            let (mut ratings, _) = every_family();
            let mut rows = rows;
            rows.insert(0, idle.clone());
            rows.push(idle.clone());
            let found = probe_each(&mut ratings, &rows);
            assert!(
                found[0].is_empty(),
                "family {family} starts quiet: {:?}",
                found[0]
            );
            assert!(
                found
                    .iter()
                    .flatten()
                    .any(|d| d.refs().iter().any(|r| r.to_string() == named)),
                "family {family} names {named}: {found:?}"
            );
        }
    }

    /// A MOSFET body diode warning waits for `BODY_DIODE_HOLD_PS` (1 us) in a transient so brief
    /// switching spikes and single solver steps stay quiet, while unpowered or below-knee reverse
    /// bias never warns and sustained reverse bias warns with the right time and peak.
    #[test]
    fn mosfet_body_diode_hold_and_threshold_rules() {
        let netlist = crate::netlist::parse(
            br#"{
          "version": 1,
          "seed": 0,
          "analysis": { "op": {} },
          "ground": "gnd",
          "nets": ["gnd", "nd", "ns", "pd", "ps"],
          "parts": [
            {
              "name": "mn",
              "kind": "nmos",
              "pins": { "drain": "nd", "gate": "gnd", "source": "ns" },
              "transconductance_parameter": { "value": 0.3236, "unit": "A/V^2" },
              "threshold_voltage": { "value": 1.3, "unit": "V" },
              "channel_length_modulation": { "value": 0.0, "unit": "1/V" },
              "gate_source_capacitance": { "value": 2.1e-11, "unit": "F" },
              "gate_drain_capacitance": { "value": 6e-12, "unit": "F" },
              "drain_source_capacitance": { "value": 7e-12, "unit": "F" },
              "body_diode_forward_voltage": { "value": 0.8, "unit": "V" },
              "ratings": { "current": null, "power": null }
            },
            {
              "name": "mp",
              "kind": "pmos",
              "pins": { "drain": "pd", "gate": "gnd", "source": "ps" },
              "transconductance_parameter": { "value": 4.579, "unit": "A/V^2" },
              "threshold_voltage": { "value": 0.8, "unit": "V" },
              "channel_length_modulation": { "value": 0.0, "unit": "1/V" },
              "gate_source_capacitance": { "value": 7.23e-10, "unit": "F" },
              "gate_drain_capacitance": { "value": 5.6e-11, "unit": "F" },
              "drain_source_capacitance": { "value": 6.5e-11, "unit": "F" },
              "body_diode_forward_voltage": { "value": 0.7, "unit": "V" },
              "ratings": { "current": null, "power": null }
            }
          ]
        }"#,
        )
        .unwrap();
        let columns = Columns::new(&netlist);
        let row = |set: &[(&str, f64)]| row_of(&columns, set);

        // Operating point warns at once on either polarity, and stays quiet at or below Vf.
        let mut op = Ratings::new(&netlist, &[], &[], &columns, Timing::Point);
        op.sample(
            Time::ZERO,
            &row(&[("#net.ns", 0.8), ("#net.pd", 0.7)]),
            &[],
            &[],
        );
        assert_eq!(op.findings(), []);
        op.sample(
            Time::ZERO,
            &row(&[("#net.ns", 2.5), ("#net.pd", 1.9)]),
            &[],
            &[],
        );
        let found = op.findings();
        assert_eq!(found.len(), 2, "{found:?}");

        // Transient: brief spike under BODY_DIODE_HOLD_PS stays quiet, then held reverse bias
        // warns with first at the start of the held spell.
        let mut run = Ratings::new(&netlist, &[], &[], &columns, Timing::Run);
        run.sample(Time::from_ps(1_000), &row(&[]), &[], &[]);
        // Sub-1us spike on both devices (500 ns).
        run.sample(
            Time::from_ps(1_000_000),
            &row(&[("#net.ns", 3.3), ("#net.pd", 3.3)]),
            &[],
            &[],
        );
        run.sample(
            Time::from_ps(1_500_000),
            &row(&[("#net.ns", 3.3), ("#net.pd", 3.3)]),
            &[],
            &[],
        );
        run.sample(Time::from_ps(1_600_000), &row(&[]), &[], &[]);
        assert_eq!(run.findings(), [], "brief spike must stay quiet");

        // Below-knee reverse voltage held for 10 ms stays quiet.
        run.sample(
            Time::from_ps(2_000_000),
            &row(&[("#net.ns", 0.5), ("#net.pd", 0.4)]),
            &[],
            &[],
        );
        run.sample(
            Time::from_ps(12_000_000_000),
            &row(&[("#net.ns", 0.5), ("#net.pd", 0.4)]),
            &[],
            &[],
        );
        assert_eq!(
            run.findings(),
            [],
            "below-knee reverse bias must stay quiet"
        );

        // Edge restart at 20 ms followed by sustained reverse bias on mn (2.5 V) and mp (1.8 V).
        run.sample(Time::from_ps(20_000_000_000), &row(&[]), &[], &[]);
        run.restarted();
        run.sample(
            Time::from_ps(20_000_001_000),
            &row(&[("#net.ns", 2.5), ("#net.pd", 1.8)]),
            &[],
            &[],
        );
        assert_eq!(run.findings(), [], "1 ns after edge is still under 1 us");
        run.sample(
            Time::from_ps(20_001_000_000),
            &row(&[("#net.ns", 2.5), ("#net.pd", 1.8)]),
            &[],
            &[],
        );
        let found = run.findings();
        assert_eq!(found.len(), 2, "{found:?}");
        let Diagnostic::MosfetBodyDiode(nmos) = &found[0] else {
            panic!("expected MosfetBodyDiode: {found:?}");
        };
        assert_eq!(nmos.part.to_string(), "#mn");
        assert_eq!(nmos.channel, "nmos");
        assert_eq!(nmos.first, Time::from_ps(20_000_000_000));
        assert_eq!(nmos.over_ps, Some(1_000_000));
        close(nmos.peak, 2.5);
        let Diagnostic::MosfetBodyDiode(pmos) = &found[1] else {
            panic!("expected MosfetBodyDiode: {found:?}");
        };
        assert_eq!(pmos.part.to_string(), "#mp");
        assert_eq!(pmos.channel, "pmos");
        assert_eq!(pmos.first, Time::from_ps(20_000_000_000));
        assert_eq!(pmos.over_ps, Some(1_000_000));
        close(pmos.peak, 1.8);
    }
}
