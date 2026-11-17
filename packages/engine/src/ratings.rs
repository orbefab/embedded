use std::collections::VecDeque;

use orbe_model::{Drive, Time};

use crate::active::{self, Output, Stage};
use crate::netlist::{Model, NetId, Netlist, RatingKind};

use crate::diagnostic::{Diagnostic, Excursion, VoltageExcursion};
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
#[derive(Clone)]
pub(crate) struct Ratings {
    timing: Timing,
    watches: Vec<Watch>,
    windows: Vec<Window>,
    chips: Vec<Gauge>,
    last: Option<Time>,
}

/// The window a DC current rating averages over, 10.2 ms: five periods of the slowest PWM an
/// Arduino core makes by default, 490.2 Hz (16 MHz / (2 * 64 * 255), 2.04 ms), and ten of
/// the 976.6 Hz one, within 0.4%. Judging a PWM load by its RMS assumes the pin heats slower
/// than one period, so the window holds whole periods and the RMS it reports for a steady PWM
/// is the period's. A pulse longer than the window fills it and counts at its full value. A
/// pin that holds a capacitor's charging pulse, `I0^2 * tau / 2` of `i^2 * t`, stays well
/// under it, and a steady load over the rating crosses it within a fraction of it.
pub(crate) const DC_WINDOW_PS: u64 = 10_200_000_000;

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
    /// into the other.
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
    /// `Some` for a DC rating, checked on the RMS over its window.
    window: Option<Square>,
    /// Whether the last sample was over.
    over: bool,
    excursion: Option<Excursion>,
}

/// A current's square over the last [`DC_WINDOW_PS`], each sample held until the next. The
/// window starts full of the first sample, the operating point, which is DC.
#[derive(Clone, Default)]
struct Square {
    /// Spans `(from_ps, to_ps, i^2)`, oldest first, which tile the window. A span may start
    /// before t = 0.
    spans: VecDeque<(i64, i64, f64)>,
    /// `i^2 * t` over the spans, in A^2 ps.
    sum: f64,
    /// The last sample's time and square.
    held: Option<(i64, f64)>,
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
            // A supply source's current rating is the most it delivers steadily.
            let dc = matches!(part.model, Model::VoltageSource(_));
            for (kind, quantity, value) in part.model.ratings() {
                let quantity = Ref::Quantity(part.name.clone(), quantity);
                let column = columns
                    .index(&quantity)
                    .expect("every part quantity is a column");
                watches.push(Watch::new(
                    kind,
                    quantity,
                    Read::Column(column),
                    value.si.get(),
                    dc,
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
                        true,
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
                        true,
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
                currents: vec![0.0; outputs + 2],
            });
        }
        Ratings {
            timing,
            watches,
            windows,
            chips,
            last: None,
        }
    }

    /// Over means a magnitude above the limit, the RMS over the window for a DC rating, or a
    /// voltage outside its window. The first time over stays; the peak rises. A sample that was
    /// over adds the time until this one to `over_ps`. `cores` are the active chips' cores
    /// whose drive was in force for `row`.
    pub(crate) fn sample(&mut self, at: Time, row: &[Finite], cores: &[active::Core]) {
        for gauge in &mut self.chips {
            gauge.read(row, cores[gauge.active].drive());
        }
        let elapsed = |over: bool, over_ps: Option<&mut u64>, last: Option<Time>| {
            if over && let (Some(last), Some(over_ps)) = (last, over_ps) {
                *over_ps += at.as_ps() - last.as_ps();
            }
        };
        for watch in &mut self.watches {
            elapsed(
                watch.over,
                watch.excursion.as_mut().and_then(|e| e.over_ps.as_mut()),
                self.last,
            );
            let mut magnitude = match watch.read {
                Read::Column(column) => row[column].get(),
                Read::Gauge { gauge, k } => self.chips[gauge].currents[k],
            }
            .abs();
            if let Some(window) = &mut watch.window {
                magnitude = window.rms(at, magnitude);
            }
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
                        over_ps: self.timing.over_ps(),
                        window_ps: watch.window.as_ref().map(|_| DC_WINDOW_PS),
                    });
                }
                Some(excursion) if magnitude > excursion.peak => {
                    excursion.peak = magnitude;
                    excursion.peak_at = at;
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

    /// The findings of one sample at `at` alone, each with no duration. A DC rating's window
    /// runs through the last sample to `at`. With no `cores` it leaves the chips' pin ratings
    /// out.
    pub(crate) fn point(
        &self,
        at: Time,
        row: &[Finite],
        cores: Option<&[active::Core]>,
    ) -> Vec<Diagnostic> {
        let mut point = self.clone();
        point.timing = Timing::Point;
        point.last = None;
        for watch in &mut point.watches {
            watch.over = false;
            watch.excursion = None;
        }
        for window in &mut point.windows {
            window.worst = 0.0;
            window.over = false;
            window.excursion = None;
        }
        let cores = cores.unwrap_or_else(|| {
            point
                .watches
                .retain(|watch| matches!(watch.read, Read::Column(_)));
            point.windows.clear();
            point.chips.clear();
            &[]
        });
        point.sample(at, row, cores);
        point.findings()
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
    fn new(kind: RatingKind, quantity: Ref, read: Read, limit: f64, dc: bool) -> Watch {
        Watch {
            kind,
            quantity,
            read,
            limit,
            window: dc.then(Square::default),
            over: false,
            excursion: None,
        }
    }
}

impl Square {
    /// Takes `magnitude` at `at`, and returns the RMS over the window that ends there. The
    /// sample held since the last one fills the time up to `at`.
    fn rms(&mut self, at: Time, magnitude: f64) -> f64 {
        let window = DC_WINDOW_PS as i64;
        let at = i64::try_from(at.as_ps()).expect("a run's time fits in i64 ps");
        let square = magnitude * magnitude;
        match self.held {
            None => {
                self.spans.push_back((at - window, at, square));
                self.sum = square * window as f64;
            }
            Some((last, held)) if at > last => {
                match self.spans.back_mut() {
                    Some((_, to, back)) if *to == last && *back == held => *to = at,
                    _ => self.spans.push_back((last, at, held)),
                }
                self.sum += held * (at - last) as f64;
                let start = at - window;
                while let Some((from, to, square)) = self.spans.front_mut() {
                    if *to <= start {
                        self.sum -= *square * (*to - *from) as f64;
                        self.spans.pop_front();
                    } else {
                        if *from < start {
                            self.sum -= *square * (start - *from) as f64;
                            *from = start;
                        }
                        break;
                    }
                }
                // One span left is a steady current, whose sum is exact again.
                if let (1, Some(&(from, to, square))) = (self.spans.len(), self.spans.front()) {
                    self.sum = square * (to - from) as f64;
                }
            }
            Some(_) => {}
        }
        self.held = Some((at, square));
        (self.sum.max(0.0) / window as f64).sqrt()
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
            leakage: output.leakage,
        }
    }
}

impl Gauge {
    /// Each output's current under `drive`, as `active::stamp` stamps its stage at nominal
    /// sources, and the sums through the supply and ground pins.
    fn read(&mut self, row: &[Finite], drive: &[Drive]) {
        debug_assert_eq!(drive.len(), self.outputs.len());
        let (mut supply, mut ground) = (0.0, 0.0);
        for ((pin, &drive), current) in self.outputs.iter().zip(drive).zip(&mut self.currents) {
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
            *current = from_vcc - into_gnd;
            supply += from_vcc;
            ground += into_gnd;
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

    fn close(got: f64, want: f64) {
        assert!(
            (got - want).abs() <= 1e-12 * want.max(1.0),
            "{got} against {want}"
        );
    }

    /// The window starts full of the operating point, which is DC.
    #[test]
    fn a_window_starts_full_of_its_first_sample() {
        let mut square = Square::default();
        close(square.rms(Time::ZERO, 0.3), 0.3);
        close(square.rms(Time::from_ps(DC_WINDOW_PS / 2), 0.0), 0.3);
        close(
            square.rms(Time::from_ps(DC_WINDOW_PS), 0.0),
            0.3 / 2f64.sqrt(),
        );
        close(square.rms(Time::from_ps(3 * DC_WINDOW_PS / 2), 0.0), 0.0);
    }

    /// A pin into 100 pF carries about 0.2 A for 4 ns, which is 0.2 * sqrt(4 ns / 10.2 ms),
    /// 0.13 mA RMS over the window.
    #[test]
    fn a_short_pulse_counts_by_its_share_of_the_window() {
        let mut square = Square::default();
        square.rms(Time::ZERO, 0.0);
        square.rms(Time::from_ps(1_000), 0.2);
        let rms = square.rms(Time::from_ps(5_000), 0.0);
        close(rms, 0.2 * (4_000.0 / DC_WINDOW_PS as f64).sqrt());
        assert!(rms < 0.001);
        square.rms(Time::from_ps(DC_WINDOW_PS + 2_000), 0.0);
        close(
            square.rms(Time::from_ps(DC_WINDOW_PS + 3_000), 0.0),
            0.2 * (2_000.0 / DC_WINDOW_PS as f64).sqrt(),
        );
        close(square.rms(Time::from_ps(DC_WINDOW_PS + 5_000), 0.0), 0.0);
    }

    /// A steady 69 mA from 0 crosses 40 mA once (40 / 69)^2 of the window has filled.
    #[test]
    fn a_steady_overload_crosses_its_rating_inside_the_window() {
        let (mut ratings, columns) = ratings(Timing::Run);
        ratings.watches[0].window = Some(Square::default());
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
        let (mut ratings, columns) = ratings(Timing::Run);
        ratings.watches[0].window = Some(Square::default());
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
}
