//! A source's value over time: DC, a step, or a pulse train.
//!
//! Every query is arithmetic on integer picoseconds, so a pulse train of any length is
//! stored as its five numbers.

use crate::time::Time;

use super::Value;

/// A source's value over time. Only parse builds one, so its rules hold: every jump is
/// after t = 0, and two jumps of one waveform are at least `Schedule::MIN_GAP_PS` apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Dc(Value),
    Step(Step),
    Pulse(Pulse),
}

/// `initial` until `at`, then `final_level` to the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    initial: Value,
    final_level: Value,
    at: Time,
}

/// `initial` until `delay`, then `pulsed` for `width`, then `initial` until the next
/// period, every `period`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pulse {
    initial: Value,
    pulsed: Value,
    delay: Time,
    width: u64,
    period: u64,
}

impl Step {
    pub fn initial(&self) -> &Value {
        &self.initial
    }

    pub fn final_level(&self) -> &Value {
        &self.final_level
    }

    pub fn at(&self) -> Time {
        self.at
    }
}

impl Pulse {
    pub fn initial(&self) -> &Value {
        &self.initial
    }

    pub fn pulsed(&self) -> &Value {
        &self.pulsed
    }

    pub fn delay(&self) -> Time {
        self.delay
    }

    /// Picoseconds.
    pub fn width(&self) -> u64 {
        self.width
    }

    /// Picoseconds.
    pub fn period(&self) -> u64 {
        self.period
    }
}

impl Waveform {
    pub(super) fn dc(value: Value) -> Waveform {
        Waveform {
            shape: Shape::Dc(value),
        }
    }

    pub(super) fn step(initial: Value, final_level: Value, at: Time) -> Waveform {
        Waveform {
            shape: Shape::Step(Step {
                initial,
                final_level,
                at,
            }),
        }
    }

    pub(super) fn pulse(
        initial: Value,
        pulsed: Value,
        delay: Time,
        width: u64,
        period: u64,
    ) -> Waveform {
        Waveform {
            shape: Shape::Pulse(Pulse {
                initial,
                pulsed,
                delay,
                width,
                period,
            }),
        }
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    /// The value on the interval that ends at `at`. `initial` at t = 0.
    pub fn before(&self, at: Time) -> f64 {
        let at = at.as_ps();
        let high = match &self.shape {
            Shape::Dc(value) => return value.si.get(),
            Shape::Step(step) => at > step.at.as_ps(),
            Shape::Pulse(pulse) => {
                at > pulse.delay.as_ps() && {
                    let within = (at - pulse.delay.as_ps()) % pulse.period;
                    0 < within && within <= pulse.width
                }
            }
        };
        self.level(high)
    }

    /// The value on the interval that starts at `at`. Differs from `before` only at a jump.
    pub fn after(&self, at: Time) -> f64 {
        let at = at.as_ps();
        let high = match &self.shape {
            Shape::Dc(value) => return value.si.get(),
            Shape::Step(step) => at >= step.at.as_ps(),
            Shape::Pulse(pulse) => {
                at >= pulse.delay.as_ps() && (at - pulse.delay.as_ps()) % pulse.period < pulse.width
            }
        };
        self.level(high)
    }

    /// Jump times in `(0, stop)`, ascending, at most `limit` of them.
    pub fn jumps(&self, stop: Time, limit: usize) -> Vec<Time> {
        self.jumps_before(stop).take(limit).collect()
    }

    /// Jump times in `(0, stop)`, ascending, one at a time. A long pulse train is not stored.
    pub fn jumps_before(&self, stop: Time) -> impl Iterator<Item = Time> + '_ {
        Jumps {
            shape: &self.shape,
            stop: stop.as_ps(),
            rise: match &self.shape {
                Shape::Pulse(pulse) => Some(pulse.delay.as_ps()),
                Shape::Dc(_) | Shape::Step(_) => None,
            },
            fall: false,
            yielded_step: false,
        }
    }

    /// The pulsed or final level when `high`, else `initial`.
    fn level(&self, high: bool) -> f64 {
        let (low, high_level) = match &self.shape {
            Shape::Dc(value) => (value, value),
            Shape::Step(step) => (&step.initial, &step.final_level),
            Shape::Pulse(pulse) => (&pulse.initial, &pulse.pulsed),
        };
        if high { high_level } else { low }.si.get()
    }
}

/// One jump of `jumps_before` at a time.
struct Jumps<'a> {
    shape: &'a Shape,
    stop: u64,
    /// The rise under consideration. `None` once a pulse cannot advance.
    rise: Option<u64>,
    /// The next yield of a pulse is its fall.
    fall: bool,
    yielded_step: bool,
}

impl Iterator for Jumps<'_> {
    type Item = Time;

    fn next(&mut self) -> Option<Time> {
        match self.shape {
            Shape::Dc(_) => None,
            Shape::Step(step) => {
                if self.yielded_step || step.at.as_ps() >= self.stop {
                    return None;
                }
                self.yielded_step = true;
                Some(step.at)
            }
            Shape::Pulse(pulse) => {
                let at = self.rise?;
                let jump = if self.fall {
                    self.fall = false;
                    self.rise = at.checked_add(pulse.period);
                    at.checked_add(pulse.width)
                } else {
                    self.fall = true;
                    Some(at)
                };
                match jump {
                    Some(jump) if jump < self.stop => Some(Time::from_ps(jump)),
                    _ => {
                        self.rise = None;
                        None
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finite::Finite;
    use crate::netlist::Source;

    fn level(v: f64) -> Value {
        Value {
            si: Finite::new(v).unwrap(),
            source: Source::User,
        }
    }

    fn at(ps: u64) -> Time {
        Time::from_ps(ps)
    }

    fn ps(times: &[Time]) -> Vec<u64> {
        times.iter().map(|t| t.as_ps()).collect()
    }

    /// 1 to 4, delay 10 ns, width 5 ns, period 20 ns.
    fn train() -> Waveform {
        Waveform::pulse(level(1.0), level(4.0), at(10_000), 5_000, 20_000)
    }

    #[test]
    fn a_pulse_holds_the_old_level_at_each_jump_and_the_new_one_after_it() {
        let wave = train();
        let sides = |t| (wave.before(at(t)), wave.after(at(t)));
        assert_eq!(sides(0), (1.0, 1.0));
        assert_eq!(sides(10_000), (1.0, 4.0));
        assert_eq!(sides(12_000), (4.0, 4.0));
        assert_eq!(sides(15_000), (4.0, 1.0));
        assert_eq!(sides(29_999), (1.0, 1.0));
        assert_eq!(sides(30_000), (1.0, 4.0));
        assert_eq!(sides(35_000), (4.0, 1.0));
        assert_eq!(sides(35_001), (1.0, 1.0));
    }

    #[test]
    fn a_step_jumps_once_and_holds() {
        let wave = Waveform::step(level(0.5), level(-2.0), at(7));
        let sides = |t| (wave.before(at(t)), wave.after(at(t)));
        assert_eq!(sides(0), (0.5, 0.5));
        assert_eq!(sides(6), (0.5, 0.5));
        assert_eq!(sides(7), (0.5, -2.0));
        assert_eq!(sides(u64::MAX), (-2.0, -2.0));
        assert_eq!(ps(&wave.jumps(at(8), 10)), [7]);
        assert_eq!(ps(&wave.jumps(at(7), 10)), [] as [u64; 0]);
        assert_eq!(ps(&wave.jumps(at(8), 0)), [] as [u64; 0]);
    }

    #[test]
    fn dc_never_jumps() {
        let wave = Waveform::dc(level(3.0));
        assert_eq!(wave.before(at(0)), 3.0);
        assert_eq!(wave.after(at(u64::MAX)), 3.0);
        assert!(wave.jumps(at(u64::MAX), usize::MAX).is_empty());
    }

    #[test]
    fn a_pulse_jumps_twice_a_period_strictly_before_stop() {
        let wave = train();
        assert_eq!(
            ps(&wave.jumps(at(50_000), usize::MAX)),
            [10_000, 15_000, 30_000, 35_000]
        );
        assert_eq!(
            ps(&wave.jumps(at(50_001), usize::MAX)),
            [10_000, 15_000, 30_000, 35_000, 50_000]
        );
        assert_eq!(ps(&wave.jumps(at(10_000), usize::MAX)), [] as [u64; 0]);
    }

    #[test]
    fn jumps_stop_at_the_limit() {
        assert_eq!(
            ps(&train().jumps(at(u64::MAX), 3)),
            [10_000, 15_000, 30_000]
        );
    }

    #[test]
    fn a_pulse_near_the_largest_time_stops_without_overflow() {
        let wave = Waveform::pulse(level(0.0), level(1.0), at(u64::MAX - 5_000), 2_000, 4_000);
        assert_eq!(
            ps(&wave.jumps(at(u64::MAX), usize::MAX)),
            [u64::MAX - 5_000, u64::MAX - 3_000, u64::MAX - 1_000]
        );
        assert_eq!(wave.before(at(u64::MAX)), 1.0);
    }
}
