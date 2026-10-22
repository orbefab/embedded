//! A source's value over time: a step list, one step, or a pulse train.
//!
//! A plain value is a step list with no events. A pulse train of any length is stored as
//! its five numbers, and every query on it is arithmetic on integer picoseconds.

use crate::time::Time;

use super::{Event, Schedule, Value};

/// A source's value over time. Only parse builds one, so its rules hold: every jump is
/// after t = 0, and two jumps of one waveform are at least [`super::MIN_GAP_PS`] apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// `initial`, then each event's level from its time on. No events is one level at every time.
    Steps(Schedule<Value>),
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
    /// One level at every time: a `steps` waveform with no events.
    pub(super) fn dc(value: Value) -> Waveform {
        Waveform {
            shape: Shape::Steps(Schedule {
                initial: value,
                events: Vec::new(),
            }),
        }
    }

    pub(super) fn steps(initial: Value, events: Vec<Event<Value>>) -> Waveform {
        Waveform {
            shape: Shape::Steps(Schedule { initial, events }),
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
        let at_ps = at.as_ps();
        match &self.shape {
            Shape::Steps(schedule) => schedule.before(at).si.get(),
            Shape::Step(step) => pick(&step.initial, &step.final_level, at_ps > step.at.as_ps()),
            Shape::Pulse(pulse) => {
                let high = at_ps > pulse.delay.as_ps() && {
                    let within = (at_ps - pulse.delay.as_ps()) % pulse.period;
                    0 < within && within <= pulse.width
                };
                pick(&pulse.initial, &pulse.pulsed, high)
            }
        }
    }

    /// The value on the interval that starts at `at`. Differs from `before` only at a jump.
    pub fn after(&self, at: Time) -> f64 {
        let at_ps = at.as_ps();
        match &self.shape {
            Shape::Steps(schedule) => schedule.after(at).si.get(),
            Shape::Step(step) => pick(&step.initial, &step.final_level, at_ps >= step.at.as_ps()),
            Shape::Pulse(pulse) => {
                let high = at_ps >= pulse.delay.as_ps()
                    && (at_ps - pulse.delay.as_ps()) % pulse.period < pulse.width;
                pick(&pulse.initial, &pulse.pulsed, high)
            }
        }
    }

    /// The next jump strictly after `at`.
    pub fn next_change_after(&self, at: Time) -> Option<Time> {
        match &self.shape {
            Shape::Steps(schedule) => schedule.next_change_after(at),
            Shape::Step(step) => (at < step.at).then_some(step.at),
            Shape::Pulse(pulse) => pulse_edge_after(pulse, at.as_ps()).map(Time::from_ps),
        }
    }
}

fn pick(low: &Value, high: &Value, high_side: bool) -> f64 {
    if high_side { high } else { low }.si.get()
}

fn pulse_edge_after(pulse: &Pulse, at: u64) -> Option<u64> {
    let delay = pulse.delay.as_ps();
    let period = pulse.period;
    let width = pulse.width;
    let rise_after = |periods: u64| offset(delay, periods, period, 0);
    let fall_after = |periods: u64| offset(delay, periods, period, width);
    if at < delay {
        return earliest(Some(delay), fall_after(0));
    }
    let periods = (at - delay) / period;
    let next = periods.checked_add(1)?;
    let fall = match fall_after(periods) {
        Some(fall) if fall > at => Some(fall),
        _ => fall_after(next),
    };
    earliest(rise_after(next), fall)
}

fn offset(delay: u64, periods: u64, period: u64, extra: u64) -> Option<u64> {
    delay
        .checked_add(periods.checked_mul(period)?)?
        .checked_add(extra)
}

fn earliest(rise: Option<u64>, fall: Option<u64>) -> Option<u64> {
    rise.into_iter().chain(fall).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finite::Finite;
    use crate::netlist::{Event, Source};

    fn level(v: f64) -> Value {
        Value {
            si: Finite::new(v).unwrap(),
            source: Source::User,
        }
    }

    fn at(ps: u64) -> Time {
        Time::from_ps(ps)
    }

    fn jumps(wave: &Waveform, stop: Time) -> Vec<u64> {
        std::iter::successors(wave.next_change_after(Time::ZERO), |&at| {
            wave.next_change_after(at)
        })
        .take_while(|&at| at < stop)
        .map(Time::as_ps)
        .collect()
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
        assert_eq!(jumps(&wave, at(8)), [7]);
        assert_eq!(jumps(&wave, at(7)), [] as [u64; 0]);
    }

    #[test]
    fn a_fixed_level_never_jumps() {
        let wave = Waveform::dc(level(3.0));
        assert_eq!(wave.before(at(0)), 3.0);
        assert_eq!(wave.after(at(u64::MAX)), 3.0);
        assert!(jumps(&wave, at(u64::MAX)).is_empty());
    }

    #[test]
    fn steps_hold_the_level_in_force_and_jump_at_each_event() {
        let wave = Waveform::steps(
            level(1.0),
            vec![
                Event {
                    at: at(4_000),
                    to: level(5.0),
                },
                Event {
                    at: at(9_000),
                    to: level(2.0),
                },
            ],
        );
        let sides = |t| (wave.before(at(t)), wave.after(at(t)));
        assert_eq!(sides(0), (1.0, 1.0));
        assert_eq!(sides(4_000), (1.0, 5.0));
        assert_eq!(sides(8_999), (5.0, 5.0));
        assert_eq!(sides(9_000), (5.0, 2.0));
        assert_eq!(sides(u64::MAX), (2.0, 2.0));
        assert_eq!(jumps(&wave, at(9_000)), [4_000]);
        assert_eq!(jumps(&wave, at(9_001)), [4_000, 9_000]);
    }

    #[test]
    fn a_pulse_jumps_twice_a_period_strictly_before_stop() {
        let wave = train();
        assert_eq!(jumps(&wave, at(50_000)), [10_000, 15_000, 30_000, 35_000]);
        assert_eq!(
            jumps(&wave, at(50_001)),
            [10_000, 15_000, 30_000, 35_000, 50_000]
        );
        assert_eq!(jumps(&wave, at(10_000)), [] as [u64; 0]);
    }

    #[test]
    fn a_pulse_near_the_largest_time_stops_without_overflow() {
        let wave = Waveform::pulse(level(0.0), level(1.0), at(u64::MAX - 5_000), 2_000, 4_000);
        assert_eq!(
            jumps(&wave, at(u64::MAX)),
            [u64::MAX - 5_000, u64::MAX - 3_000, u64::MAX - 1_000]
        );
        assert_eq!(wave.before(at(u64::MAX)), 1.0);
    }

    #[test]
    fn next_change_after_is_the_next_jump() {
        let step = Waveform::step(level(0.5), level(-2.0), at(7));
        assert_eq!(step.next_change_after(at(0)), Some(at(7)));
        assert_eq!(step.next_change_after(at(6)), Some(at(7)));
        assert_eq!(step.next_change_after(at(7)), None);
        assert_eq!(step.next_change_after(at(u64::MAX)), None);

        let fixed = Waveform::dc(level(3.0));
        assert_eq!(fixed.next_change_after(at(0)), None);
        assert_eq!(fixed.next_change_after(at(u64::MAX)), None);

        let steps = Waveform::steps(
            level(0.0),
            vec![Event {
                at: at(4_000),
                to: level(1.0),
            }],
        );
        assert_eq!(steps.next_change_after(at(0)), Some(at(4_000)));
        assert_eq!(steps.next_change_after(at(4_000)), None);

        let wave = train();
        assert_eq!(wave.next_change_after(at(0)), Some(at(10_000)));
        assert_eq!(wave.next_change_after(at(10_000)), Some(at(15_000)));
        assert_eq!(wave.next_change_after(at(12_000)), Some(at(15_000)));
        assert_eq!(wave.next_change_after(at(15_000)), Some(at(30_000)));
        assert_eq!(wave.next_change_after(at(29_999)), Some(at(30_000)));
        assert_eq!(wave.next_change_after(at(35_000)), Some(at(50_000)));
        assert_eq!(wave.next_change_after(at(50_000)), Some(at(55_000)));

        let late = Waveform::pulse(level(0.0), level(1.0), at(u64::MAX - 5_000), 2_000, 4_000);
        assert_eq!(late.next_change_after(at(0)), Some(at(u64::MAX - 5_000)));
        assert_eq!(
            late.next_change_after(at(u64::MAX - 5_000)),
            Some(at(u64::MAX - 3_000))
        );
        assert_eq!(
            late.next_change_after(at(u64::MAX - 4_000)),
            Some(at(u64::MAX - 3_000))
        );
        assert_eq!(
            late.next_change_after(at(u64::MAX - 3_000)),
            Some(at(u64::MAX - 1_000))
        );
        assert_eq!(
            late.next_change_after(at(u64::MAX - 2_000)),
            Some(at(u64::MAX - 1_000))
        );
        assert_eq!(late.next_change_after(at(u64::MAX - 1_000)), None);
    }
}
