//! An SG90 micro servo as a device. The host's pulse engine watches `SIGNAL` and calls
//! [`PulseTarget`] when a level ends. This device maps the high's width to a shaft angle,
//! slews toward it, and draws its supply current.
//!
//! The pulse map is the Luxorparts SG90 sheet (Kjell 87897): 1 ms is 0°, 1.5 ms is 90° and
//! 2 ms is 180°, and a width outside that clamps to the nearer end. Tower Pro's SG90 Analog
//! page gives the dead band as 1 µs, the operating point as 4.8 V and the no-load speed as
//! 0.12 s per 60° at that voltage. SunFounder's SF0180 electrical specification, for the SG90
//! they sell, gives 5 mA stopped, 140 mA running with no load and 520 mA stalled, at 4.8 V.
//! Stall is not a state here: nothing loads the shaft.
//!
//! The line rests low. A low of 22 ms, longer than the sheet's 20 ms frame, is idle: the shaft
//! stays where it was and stops following. Below 1 V the part is unpowered and does nothing.
//! From 1 V up to 4.5 V it still moves, slower in proportion to the voltage over 4.8 V, and the
//! first such sample is rule `supply`. At 4.5 V and above it holds the 4.8 V speed, so a Nano's
//! diode-dropped 5 V pin does not warn. The shaft powers on at 90°, the centre the horn is
//! drawn at.

use orbe_model::{
    Cx, Device, Draw, Drives, Error, Idle, Params, PulseTarget, Shows, State, Stop, Time, View,
};

/// 1 ms, the Luxorparts sheet's 0°.
const MIN_US: u64 = 1_000;
/// 2 ms, the Luxorparts sheet's 180°.
const MAX_US: u64 = 2_000;
/// Tower Pro's dead band.
const DEAD_US: u64 = 1;
/// 0.12 s per 60° at 4.8 V is 500 °/s, in millidegrees per second.
const SPEED_MDEG_S: i64 = 500_000;
/// The speed's voltage, millivolts.
const RATED_MV: i32 = 4_800;
/// Below this the shaft does not follow and the draw is none.
const UNPOWERED_MV: i32 = 1_000;
/// Below this, and at or above [`UNPOWERED_MV`], the shaft slows and rule `supply` fires.
/// At and above it the speed is the 4.8 V speed.
const FULL_SPEED_MV: i32 = 4_500;
/// SunFounder SF0180, stopped, at 4.8 V.
const IDLE_A: f64 = 0.005;
/// SunFounder SF0180, running with no load, at 4.8 V.
const RUN_A: f64 = 0.140;
/// The draw holds at the speed's voltage.
const HOLDS_V: f64 = 4.8;
/// Longer than a 20 ms frame, so the low between two pulses is not an idle.
const IDLE_AFTER_PS: u64 = 22_000_000_000;
/// How often the supply is read while the part runs.
const SAMPLE_PS: u64 = 10_000_000_000;
const PS_PER_S: i128 = 1_000_000_000_000;
const TRAVEL_MDEG: i32 = 180_000;
const CENTRE_MDEG: i32 = 90_000;

/// The shaft and the pulse it is following.
#[derive(Clone, Copy, Debug)]
pub struct Sg90 {
    /// Millidegrees, 0 to 180_000, at [`reached`](Self::reached).
    angle_mdeg: i32,
    reached: Time,
    /// Millivolts of `VCC` over `GND` at the last sample. None before it.
    supply_mv: Option<i32>,
    /// The angle the last accepted pulse commanded. None while pulses are idle.
    command: Option<i32>,
    /// That pulse's width in microseconds, for the dead band.
    command_us: Option<u64>,
    /// The shaft angle and time when `command` was accepted. The angle at a later time is this
    /// plus the slew, so a stop in between does not round the move.
    origin_mdeg: i32,
    origin: Time,
    /// The shaft has not reached `command`.
    moving: bool,
    /// The next supply sample. None once the run has no reason to ask again.
    next_sample: Option<Time>,
}

impl State for Sg90 {}

impl Sg90 {
    fn powered(&self) -> bool {
        self.supply_mv.unwrap_or(0) >= UNPOWERED_MV
    }

    fn target_of(us: u64) -> i32 {
        let us = us.clamp(MIN_US, MAX_US);
        let span = MAX_US - MIN_US;
        ((us - MIN_US) as i64 * i64::from(TRAVEL_MDEG) / span as i64) as i32
    }

    /// The millidegrees the shaft can slew in `dt` picoseconds at the present supply.
    fn slew(&self, dt: u64) -> i128 {
        let mv = self.supply_mv.unwrap_or(0);
        if mv < UNPOWERED_MV || dt == 0 {
            return 0;
        }
        let scale = if mv >= FULL_SPEED_MV { RATED_MV } else { mv };
        (i128::from(SPEED_MDEG_S) * i128::from(scale) * i128::from(dt))
            / (i128::from(RATED_MV) * PS_PER_S)
    }

    /// Moves the shaft from [`reached`](Self::reached) toward `until`. Returns whether it
    /// arrived at its command on the way, and then `reached` is that arrival.
    fn integrate(&mut self, until: Time) -> bool {
        let end = until.as_ps();
        if end <= self.reached.as_ps() {
            return false;
        }
        let Some(target) = self.command else {
            self.moving = false;
            self.angle_mdeg = self.origin_mdeg;
            self.reached = until;
            return false;
        };
        let mv = self.supply_mv.unwrap_or(0);
        if mv < UNPOWERED_MV {
            self.moving = false;
            self.reached = until;
            return false;
        }
        let delta = i128::from(target) - i128::from(self.origin_mdeg);
        if delta == 0 {
            self.angle_mdeg = target;
            self.moving = false;
            self.reached = until;
            return false;
        }
        let sign = if delta > 0 { 1 } else { -1 };
        let need = delta.unsigned_abs();
        let gone = self.slew(end.saturating_sub(self.origin.as_ps()));
        if gone < need as i128 {
            self.angle_mdeg = self.origin_mdeg + sign * (gone as i32);
            self.moving = true;
            self.reached = until;
            return false;
        }
        let scale = if mv >= FULL_SPEED_MV { RATED_MV } else { mv };
        let dt = (need as i128 * i128::from(RATED_MV) * PS_PER_S)
            / (i128::from(SPEED_MDEG_S) * i128::from(scale));
        let at = self.origin.as_ps().saturating_add(dt.max(1) as u64);
        self.angle_mdeg = target;
        self.origin_mdeg = target;
        self.moving = false;
        if at <= self.reached.as_ps() {
            // The arrival fell inside the last step. Stay on the host's clock.
            self.origin = self.reached;
            self.reached = until;
            return false;
        }
        let at = at.min(end);
        self.origin = Time::from_ps(at);
        self.reached = Time::from_ps(at);
        true
    }

    fn catch_up(&mut self, at: Time) {
        if self.reached < at {
            let _ = self.integrate(at);
        }
    }
}

impl Device for Sg90 {
    const SHOWS: Shows = Shows::Angle;

    fn new(_params: &Params<'_>) -> Result<Sg90, Error> {
        Ok(Sg90 {
            angle_mdeg: CENTRE_MDEG,
            reached: Time::ZERO,
            supply_mv: None,
            command: None,
            command_us: None,
            origin_mdeg: CENTRE_MDEG,
            origin: Time::ZERO,
            moving: false,
            next_sample: Some(Time::from_ps(1)),
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        if self.reached >= limit {
            return None;
        }
        let sample_at = self
            .next_sample
            .filter(|at| *at > self.reached && *at <= limit);
        let bound = sample_at.unwrap_or(limit);
        let arrived = self.integrate(bound);
        let sampled = sample_at.is_some_and(|at| self.reached == at);
        if sampled {
            self.next_sample = Some(Time::from_ps(
                self.reached.as_ps().saturating_add(SAMPLE_PS),
            ));
        }
        (arrived || sampled).then_some(Stop {
            at: self.reached,
            sample: sampled,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }

    fn draw(&self) -> Draw {
        let amps = if self.powered() {
            if self.moving { RUN_A } else { IDLE_A }
        } else {
            0.0
        };
        Draw {
            amps,
            holds_above_v: HOLDS_V,
        }
    }

    fn sample(&mut self, cx: &mut Cx, at: Time, volts: &[f64]) {
        self.catch_up(at);
        let mv = volts.first().map_or(0, |v| (v * 1_000.0).round() as i32);
        let mv = mv.max(0);
        let scale = |millivolts: i32| {
            if millivolts >= FULL_SPEED_MV {
                RATED_MV
            } else {
                millivolts
            }
        };
        if self.command.is_some() && scale(self.supply_mv.unwrap_or(0)) != scale(mv) {
            self.origin_mdeg = self.angle_mdeg;
            self.origin = at;
        }
        self.supply_mv = Some(mv);
        if (UNPOWERED_MV..FULL_SPEED_MV).contains(&mv) {
            cx.broke("supply", at);
        }
        if mv < UNPOWERED_MV {
            self.command = None;
            self.command_us = None;
            self.moving = false;
        }
        if self.next_sample.is_some_and(|due| due <= at) {
            self.next_sample = Some(Time::from_ps(at.as_ps().saturating_add(SAMPLE_PS)));
        }
    }

    fn view(&self) -> View<'_> {
        View::Degrees(self.angle_mdeg)
    }
}

impl PulseTarget for Sg90 {
    const IDLE: Idle = Idle {
        level: false,
        after: IDLE_AFTER_PS,
    };

    fn pulse(&mut self, _cx: &mut Cx, at: Time, level: bool, width: u64) {
        self.catch_up(at);
        if !level || !self.powered() {
            return;
        }
        let us = width.saturating_add(500_000) / 1_000_000;
        if self
            .command_us
            .is_some_and(|prev| us.abs_diff(prev) <= DEAD_US)
        {
            return;
        }
        let target = Self::target_of(us);
        self.origin_mdeg = self.angle_mdeg;
        self.origin = at;
        self.command_us = Some(us);
        self.command = Some(target);
        self.moving = target != self.angle_mdeg;
    }

    fn idle(&mut self, _cx: &mut Cx, at: Time, _level: bool) {
        self.catch_up(at);
        self.origin_mdeg = self.angle_mdeg;
        self.origin = at;
        self.command = None;
        self.command_us = None;
        self.moving = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const US: u64 = 1_000_000;
    const MS: u64 = 1_000 * US;

    fn servo() -> Sg90 {
        Sg90::new(&Params::NONE).expect("an sg90 takes no params")
    }

    fn at(ps: u64) -> Time {
        Time::from_ps(ps)
    }

    /// Runs to the first sample, feeds `volts`, and leaves the shaft ready to follow.
    fn power(dev: &mut Sg90, volts: f64) {
        let mut cx = Cx::default();
        let stop = dev.run_until(&mut cx, at(MS)).expect("the first sample");
        assert!(stop.sample);
        dev.sample(&mut cx, stop.at, &[volts]);
    }

    fn degrees(dev: &Sg90) -> f64 {
        let View::Degrees(mdeg) = dev.view() else {
            panic!("an sg90 shows degrees");
        };
        mdeg as f64 / 1_000.0
    }

    fn pulse(dev: &mut Sg90, at_ps: u64, us: u64) {
        dev.pulse(&mut Cx::default(), at(at_ps), true, us * US);
    }

    /// Runs to `limit`, answering each supply sample with the voltage already read.
    fn run(dev: &mut Sg90, limit: u64) -> Option<Stop> {
        let volts = dev.supply_mv.unwrap_or(0) as f64 / 1_000.0;
        loop {
            let stop = dev.run_until(&mut Cx::default(), at(limit))?;
            if !stop.sample {
                return Some(stop);
            }
            dev.sample(&mut Cx::default(), stop.at, &[volts]);
        }
    }

    #[test]
    fn it_powers_on_at_centre_and_draws_nothing_until_the_supply_is_known() {
        let dev = servo();
        assert_eq!(degrees(&dev), 90.0);
        assert_eq!(dev.draw().amps, 0.0);
    }

    #[test]
    fn a_centred_pulse_holds_and_draws_the_idle_current() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 1_500);
        assert!(!dev.moving);
        assert_eq!(degrees(&dev), 90.0);
        assert_eq!(dev.draw().amps, IDLE_A);
    }

    #[test]
    fn it_slews_sixty_degrees_in_the_sheet_time_and_holds() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 1_000);
        assert!(dev.moving);
        assert_eq!(dev.draw().amps, RUN_A);
        assert!(run(&mut dev, MS + 120 * MS).is_none());
        assert_eq!(degrees(&dev), 30.0);
        let stop = run(&mut dev, MS + 200 * MS).expect("it arrives");
        assert_eq!(stop.at.as_ps() - MS, 180 * MS);
        assert_eq!(degrees(&dev), 0.0);
        assert!(!dev.moving);
        assert_eq!(dev.draw().amps, IDLE_A);
        assert!(run(&mut dev, MS + 250 * MS).is_none());
        assert_eq!(degrees(&dev), 0.0);
    }

    #[test]
    fn a_pulse_outside_the_sheet_clamps() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 400);
        assert_eq!(dev.command, Some(0));
        pulse(&mut dev, 2 * MS, 3_000);
        assert_eq!(dev.command, Some(TRAVEL_MDEG));
    }

    #[test]
    fn a_pulse_inside_the_dead_band_does_not_move_the_command() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 1_000);
        pulse(&mut dev, 2 * MS, 1_001);
        assert_eq!(dev.command_us, Some(1_000));
    }

    #[test]
    fn pulses_stopping_leaves_the_shaft_where_it_was() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 1_000);
        assert!(run(&mut dev, MS + 60 * MS).is_none());
        let held = degrees(&dev);
        assert!((held - 60.0).abs() < 0.01, "{held}");
        dev.idle(&mut Cx::default(), at(MS + 60 * MS), false);
        assert!(!dev.moving);
        assert!(dev.command.is_none());
        assert!(run(&mut dev, MS + 500 * MS).is_none());
        assert_eq!(degrees(&dev), held);
        assert_eq!(dev.draw().amps, IDLE_A);
    }

    #[test]
    fn unpowered_it_does_not_follow() {
        let mut dev = servo();
        power(&mut dev, 0.0);
        pulse(&mut dev, MS, 1_000);
        assert!(dev.command.is_none());
        assert_eq!(degrees(&dev), 90.0);
        assert_eq!(dev.draw().amps, 0.0);
        assert!(run(&mut dev, MS + 500 * MS).is_none());
        assert_eq!(degrees(&dev), 90.0);
    }

    #[test]
    fn below_four_and_a_half_volts_it_slows_and_warns() {
        let mut dev = servo();
        let mut cx = Cx::default();
        let stop = dev.run_until(&mut cx, at(MS)).expect("the first sample");
        dev.sample(&mut cx, stop.at, &[3.0]);
        assert_eq!(cx.broken().map(|broken| broken.0), Some("supply"));
        pulse(&mut dev, MS, 1_000);
        assert!(run(&mut dev, MS + 192 * MS).is_none());
        assert_eq!(degrees(&dev), 30.0);
        let stop = run(&mut dev, MS + 400 * MS).expect("it arrives later than at 4.8 V");
        assert_eq!(stop.at.as_ps() - MS, 288 * MS);
        assert_eq!(degrees(&dev), 0.0);
    }
}
