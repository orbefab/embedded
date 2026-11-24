//! An SG90 micro servo as a device. The host's pulse engine watches `SIGNAL` and calls
//! [`PulseTarget`] when a level ends. This device maps the high's width to a shaft angle,
//! slews toward it, and draws its supply current.
//!
//! The mechanical travel is the range a real SG90 turns through, about 180° from 0.5 ms to
//! 2.4 ms. DroneBot Workshop's ESP32 servo page set an SG90's ends at 500 µs and 2400 µs so the
//! horn reached both stops. Nick Gammon measured a micro SG90 through 180° on pulses he read as
//! 0.5 ms to 2.5 ms, and quoted a 500 µs to 2400 µs spec (Arduino forum, "servo pulse widths",
//! topic 286525). SunFounder's SG90 page lists 500 µs to 2500 µs for 180° ± 10°. The Luxorparts
//! sheet (Kjell 87897) says 1 ms is 0° (minimum position), 1.5 ms is 90° and 2 ms is 180°
//! (maximum position). The shaft follows the measured 0.5 ms to 2.4 ms travel instead, so a
//! width outside that clamps at the nearer stop. The middle of that travel is 1.45 ms.
//! Tower Pro's SG90 Analog page gives the dead band as 1 µs, the operating point as 4.8 V and
//! the no-load speed as 0.12 s per 60° at that voltage. The Luxorparts sheet gives the same
//! 0.12 s per 60° at 4.8 V and 0.10 s per 60° at 6 V. Between those two voltages the speed is
//! the straight line through the two figures. Below 4.8 V it is the 4.8 V figure times the
//! supply over 4.8 V. Below 4.5 V, the low end of the range the Nano's 5 V pin can read,
//! the first sample is rule `supply`. At 6 V and above it holds
//! the 6 V speed. SunFounder's SF0180 electrical specification, for the SG90 they sell, gives
//! 5 mA stopped, 140 mA running with no load and 520 mA stalled, at 4.8 V. Stall is not a
//! state here: nothing loads the shaft.
//!
//! The line rests low. A low of 22 ms, longer than the sheet's 20 ms frame, is idle: the shaft
//! stays where it was and stops following. The control board in a taken-apart SG90 is an
//! AA51880. Its sheet gives no operating supply range, no UVLO and no restart level. The 2.3 V
//! figure is VREG minimum, the regulator output at Vcc 4.8 V. Below 3.0 V the shaft does not
//! follow and the draw is none. 3.0 V is the bottom of the operating range 4tronix lists for
//! an SG90, 3.0 V to 7.2 V, the lowest such minimum among the cited sheets. A supply that
//! returns above 3.0 V follows the next pulse. When the moving draw itself pulls the supply
//! under 3.0 V, the shaft holds and the part warns that the supply sags under that draw. A
//! real SG90 browns out and jitters there, because the controller resets and tries again. The
//! model holds the shaft instead of buzzing, and the draw returns to the 5 mA hold as soon as
//! the rail recovers. The host samples the supply at every accepted step, and a read of the
//! angle is the shaft's place at that read. The shaft powers on at 90°, the centre the horn
//! is drawn at.

use orbe_model::{
    Cx, Device, Draw, Drives, Error, Idle, Params, PulseTarget, Shows, State, Stop, Time, View,
};

/// 0.5 ms, one mechanical stop.
const MIN_US: u64 = 500;
/// 2.4 ms, the other mechanical stop.
const MAX_US: u64 = 2_400;
/// Tower Pro's dead band.
const DEAD_US: u64 = 1;
/// 0.12 s per 60° at 4.8 V is 500 °/s, in millidegrees per second.
const SPEED_RATED: i64 = 500_000;
/// 0.10 s per 60° at 6 V is 600 °/s, in millidegrees per second.
const SPEED_FAST: i64 = 600_000;
/// The 4.8 V figure, millivolts. Below it the speed is that figure times the supply over it.
const RATED_MV: i32 = 4_800;
/// Below this, and at or above the floor, rule `supply` fires. The Nano part states that
/// its 5 V pin can read from about 4.5 V to 5.0 V, so a pin in that range does not warn.
const WARN_BELOW_MV: i32 = 4_500;
/// The 6 V figure, millivolts. At and above it the speed holds.
const FAST_MV: i32 = 6_000;
/// Below this the shaft does not follow and the draw is none. 4tronix lists 3.0 V to 7.2 V.
const FLOOR_MV: i32 = 3_000;
/// After a moving draw reads under the floor, look again this much later. The host applies a
/// draw change 1 ps after the sample that made it, on the solution the old draw produced, so
/// the recovered rail is the sample one step after that. Without this stop that step is the
/// solver's, and the collapsed reading lasts as long as `max_step_ps`.
const CONFIRM_PS: u64 = 2;
/// SunFounder SF0180, stopped, at 4.8 V.
const IDLE_A: f64 = 0.005;
/// SunFounder SF0180, running with no load, at 4.8 V.
const RUN_A: f64 = 0.140;
/// The draw holds at the speed's voltage.
const HOLDS_V: f64 = 4.8;
/// Longer than a 20 ms frame, so the low between two pulses is not an idle.
const IDLE_AFTER_PS: u64 = 22_000_000_000;
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
    /// When to read the rail again after the moving draw pulled it under the floor. The sample
    /// there decides whether the supply sagged under that draw or is itself under the floor.
    confirm: Option<Time>,
}

impl State for Sg90 {}

impl Sg90 {
    fn powered(&self) -> bool {
        self.supply_mv.unwrap_or(0) >= FLOOR_MV
    }

    /// Millidegrees per second at `mv`. Zero below the floor. The 4.8 V figure scaled by the
    /// supply over 4.8 V up to that voltage, then the line from that figure to the 6 V figure,
    /// and the 6 V figure from there on.
    fn speed_mdeg_s(mv: i32) -> i64 {
        if mv < FLOOR_MV {
            return 0;
        }
        if mv <= RATED_MV {
            return SPEED_RATED * i64::from(mv) / i64::from(RATED_MV);
        }
        if mv >= FAST_MV {
            return SPEED_FAST;
        }
        let span = i64::from(FAST_MV - RATED_MV);
        SPEED_RATED + (SPEED_FAST - SPEED_RATED) * i64::from(mv - RATED_MV) / span
    }

    fn target_of(us: u64) -> i32 {
        let us = us.clamp(MIN_US, MAX_US);
        let span = MAX_US - MIN_US;
        ((us - MIN_US) as i64 * i64::from(TRAVEL_MDEG) / span as i64) as i32
    }

    /// The shaft angle at `at`. Between stops this is ahead of [`angle_mdeg`](Self::angle_mdeg),
    /// which is the angle at [`reached`](Self::reached).
    fn position_at(&self, at: Time) -> i32 {
        let Some(target) = self.command else {
            return self.angle_mdeg;
        };
        let speed = Self::speed_mdeg_s(self.supply_mv.unwrap_or(0));
        if speed <= 0 {
            return self.angle_mdeg;
        }
        let end = at.as_ps();
        let start = self.origin.as_ps();
        if end <= start {
            return self.origin_mdeg;
        }
        let delta = i128::from(target) - i128::from(self.origin_mdeg);
        if delta == 0 {
            return target;
        }
        let sign = if delta > 0 { 1 } else { -1 };
        let gone = (i128::from(speed) * i128::from(end - start)) / PS_PER_S;
        if gone >= delta.unsigned_abs() as i128 {
            target
        } else {
            self.origin_mdeg + sign * (gone as i32)
        }
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
        let speed = Self::speed_mdeg_s(self.supply_mv.unwrap_or(0));
        if speed <= 0 {
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
        let here = self.position_at(until);
        if here != target {
            self.angle_mdeg = here;
            self.moving = true;
            self.reached = until;
            return false;
        }
        let need = (i128::from(target) - i128::from(self.origin_mdeg)).unsigned_abs();
        let dt = (need as i128 * PS_PER_S) / i128::from(speed);
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
    const TRACKS_SUPPLY: bool = true;

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
            confirm: None,
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        if self.reached >= limit {
            return None;
        }
        if let Some(confirm) = self.confirm
            && confirm > self.reached
            && confirm <= limit
        {
            let _ = self.integrate(confirm);
            return Some(Stop {
                at: confirm,
                sample: true,
                record: false,
            });
        }
        let arrived = self.integrate(limit);
        arrived.then_some(Stop {
            at: self.reached,
            sample: false,
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
        let trying = self.moving || self.command.is_some();
        let was = Self::speed_mdeg_s(self.supply_mv.unwrap_or(0));
        if self.command.is_some() && was != Self::speed_mdeg_s(mv) {
            self.origin_mdeg = self.angle_mdeg;
            self.origin = at;
        }
        self.supply_mv = Some(mv);
        let sag = self.confirm.is_some_and(|due| due <= at);
        if mv < FLOOR_MV {
            // Hold the angle `catch_up` just reached. Clearing `command` alone would send the
            // next step back to `origin_mdeg`.
            self.origin_mdeg = self.angle_mdeg;
            self.origin = at;
            self.command = None;
            self.command_us = None;
            self.moving = false;
            if trying && self.confirm.is_none() {
                self.confirm = Some(Time::from_ps(at.as_ps().saturating_add(CONFIRM_PS)));
            } else if sag {
                // The rail is still under the floor with the moving draw gone, so the supply
                // itself is low. That is not a sag.
                self.confirm = None;
            }
        } else if sag {
            self.confirm = None;
            // The rail came back once the moving draw dropped, so that draw pulled it under
            // the floor. The shaft stays. The draw is the hold, because the supply is up.
            // One call reports one rule, so the slow-range warning waits for the next sample.
            cx.broke("supply_sag", at);
        } else if (FLOOR_MV..WARN_BELOW_MV).contains(&mv) {
            cx.broke("supply", at);
        }
    }

    fn view(&self) -> View<'_> {
        View::Degrees(self.angle_mdeg)
    }

    fn angle_at(&self, at: Time) -> Option<i32> {
        Some(self.position_at(at))
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

    /// Feeds `volts` at t = 0 and leaves the shaft ready to follow.
    fn power(dev: &mut Sg90, volts: f64) {
        dev.sample(&mut Cx::default(), Time::ZERO, &[volts]);
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
        // 1.45 ms is the middle of the 0.5 ms to 2.4 ms travel. The sheet's 1.5 ms sits a few degrees off it.
        pulse(&mut dev, MS, 1_450);
        assert!(!dev.moving);
        assert_eq!(degrees(&dev), 90.0);
        assert_eq!(dev.draw().amps, IDLE_A);
    }

    #[test]
    fn it_slews_sixty_degrees_in_the_sheet_time_and_holds() {
        let mut dev = servo();
        power(&mut dev, 4.8);
        pulse(&mut dev, MS, 500);
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
    fn a_pulse_past_the_mechanical_stops_clamps_and_the_sheet_band_does_not() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 400);
        assert_eq!(dev.command, Some(0));
        pulse(&mut dev, 2 * MS, 1_000);
        // Luxorparts calls 1 ms the minimum position. On the measured 0.5 ms to 2.4 ms travel it is about 47°.
        assert_eq!(dev.command, Some(47_368));
        pulse(&mut dev, 3 * MS, 2_000);
        assert_eq!(dev.command, Some(142_105));
        pulse(&mut dev, 4 * MS, 3_000);
        assert_eq!(dev.command, Some(TRAVEL_MDEG));
    }

    /// Servo 1.3.0 `write(angle)` is `angle * (2400 - 544) / 180 + 544`, integer division.
    fn library_us(angle: u64) -> u64 {
        angle * (2_400 - 544) / 180 + 544
    }

    #[test]
    fn a_stock_servo_write_moves_the_horn_the_whole_way() {
        let mut prev = -1;
        for angle in 0..=180 {
            let target = Sg90::target_of(library_us(angle));
            assert!(
                target > prev,
                "write({angle}) at {} us is {target}",
                library_us(angle)
            );
            prev = target;
        }
        let start = Sg90::target_of(library_us(0));
        let end = Sg90::target_of(library_us(180));
        assert_eq!(library_us(0), 544);
        assert_eq!(library_us(45), 1_008);
        assert_eq!(library_us(180), 2_400);
        assert!(start < 5_000, "{start}");
        assert_eq!(end, TRAVEL_MDEG);
        assert!(end - start > 170_000);
        // 1.45 ms is the middle of 0.5 ms and 2.4 ms.
        assert_eq!(Sg90::target_of(1_450), CENTRE_MDEG);
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
        power(&mut dev, 4.8);
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
    fn at_three_volts_it_slows_and_warns() {
        let mut dev = servo();
        let mut cx = Cx::default();
        dev.sample(&mut cx, Time::ZERO, &[3.0]);
        assert_eq!(cx.broken().map(|broken| broken.0), Some("supply"));
        pulse(&mut dev, MS, 500);
        assert!(run(&mut dev, MS + 192 * MS).is_none());
        assert_eq!(degrees(&dev), 30.0);
        let stop = run(&mut dev, MS + 400 * MS).expect("it arrives later than at 4.8 V");
        assert_eq!(stop.at.as_ps() - MS, 288 * MS);
        assert_eq!(degrees(&dev), 0.0);
    }

    #[test]
    fn a_collapsed_rail_does_not_turn_the_shaft() {
        let mut dev = servo();
        power(&mut dev, 1.44);
        pulse(&mut dev, MS, 500);
        assert!(dev.command.is_none());
        assert_eq!(degrees(&dev), 90.0);
        assert_eq!(dev.draw().amps, 0.0);
        assert!(run(&mut dev, MS + 500 * MS).is_none());
        assert_eq!(degrees(&dev), 90.0);
    }

    #[test]
    fn below_three_volts_it_does_not_follow_and_at_three_it_does() {
        let mut dead = servo();
        power(&mut dead, 2.9);
        pulse(&mut dead, MS, 500);
        assert!(dead.command.is_none());
        assert_eq!(degrees(&dead), 90.0);
        let mut dev = servo();
        let mut cx = Cx::default();
        dev.sample(&mut cx, Time::ZERO, &[3.0]);
        assert_eq!(cx.broken().map(|broken| broken.0), Some("supply"));
        pulse(&mut dev, MS, 500);
        assert!(dev.moving);
    }

    #[test]
    fn a_supply_back_at_four_volts_follows_again() {
        let mut dev = servo();
        power(&mut dev, 0.0);
        let mut cx = Cx::default();
        dev.sample(&mut cx, at(2 * MS), &[4.0]);
        pulse(&mut dev, 2 * MS, 500);
        assert_eq!(
            dev.command,
            Some(0),
            "4 V is above the floor, so the shaft follows"
        );
        assert!(run(&mut dev, 2 * MS + 200 * MS).is_none());
        assert!(
            degrees(&dev) < 20.0,
            "the shaft left centre, at {}",
            degrees(&dev)
        );
    }

    #[test]
    fn six_volts_covers_sixty_degrees_in_a_tenth_of_a_second() {
        let mut dev = servo();
        power(&mut dev, 6.0);
        pulse(&mut dev, MS, 500);
        assert!(run(&mut dev, MS + 100 * MS).is_none());
        assert_eq!(degrees(&dev), 30.0, "0.10 s per 60° at 6 V");
        let stop = run(&mut dev, MS + 200 * MS).expect("it arrives");
        assert_eq!(stop.at.as_ps() - MS, 150 * MS);
        assert_eq!(degrees(&dev), 0.0);
    }

    #[test]
    fn losing_power_mid_move_holds_the_angle() {
        let mut dev = servo();
        power(&mut dev, 4.8);
        pulse(&mut dev, MS, 500);
        assert!(run(&mut dev, MS + 60 * MS).is_none());
        let held = degrees(&dev);
        assert!((held - 60.0).abs() < 0.01, "{held}");
        let mut cx = Cx::default();
        dev.sample(&mut cx, at(MS + 60 * MS), &[1.44]);
        assert!(run(&mut dev, MS + 500 * MS).is_none());
        assert_eq!(degrees(&dev), held);
        assert_eq!(dev.draw().amps, 0.0);
    }

    #[test]
    fn a_moving_draw_that_collapses_the_rail_warns_and_holds() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 2_400);
        assert!(dev.moving);
        let mut cx = Cx::default();
        dev.sample(&mut cx, at(MS), &[2.8]);
        assert!(
            cx.broken().is_none(),
            "one low sample is not yet known to be the moving draw"
        );
        assert_eq!(dev.draw().amps, 0.0);
        assert_eq!(degrees(&dev), 90.0);
        let mut cx = Cx::default();
        dev.sample(&mut cx, at(MS + 2), &[5.0]);
        assert_eq!(cx.broken().map(|broken| broken.0), Some("supply_sag"));
        assert_eq!(dev.draw().amps, IDLE_A);
        assert_eq!(degrees(&dev), 90.0);
        assert!(run(&mut dev, MS + 200 * MS).is_none());
        assert_eq!(degrees(&dev), 90.0);
    }

    #[test]
    fn a_supply_that_stays_under_the_floor_is_not_a_sag() {
        let mut dev = servo();
        power(&mut dev, 5.0);
        pulse(&mut dev, MS, 2_400);
        let mut cx = Cx::default();
        dev.sample(&mut cx, at(MS), &[2.0]);
        dev.sample(&mut cx, at(MS + 2), &[2.0]);
        assert!(cx.broken().is_none());
        assert_eq!(dev.draw().amps, 0.0);
        assert_eq!(degrees(&dev), 90.0);
    }
}
