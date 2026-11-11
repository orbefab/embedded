//! A source's value over time: a step list, one step, a pulse train, or 8N1 frames.
//!
//! A plain value is a step list with no events. A pulse train of any length is stored as
//! its five numbers, and every query on it is arithmetic on integer picoseconds. A serial
//! waveform stores each frame the same way. A query walks bit boundaries. It does not
//! store an edge list.

use orbe_model::Time;

use super::{Event, Schedule, Value};

/// A source's value over time. Only parse and [`Netlist::append`](super::Netlist::append) build
/// one, so its rules hold: every jump is after t = 0, and two jumps of one waveform are at least
/// [`super::MIN_GAP_PS`] apart.
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
    /// `mark` while idle and for a 1 bit, `space` for a start bit and a 0 bit.
    Serial(Serial),
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

/// Picoseconds in one second. Bit `i` of an event starts at `t_ps + floor(i * PS_PER_S / baud)`.
const PS_PER_S: u128 = 1_000_000_000_000;

/// A baud rate from 1 to [`Baud::MAX`]. There is no `From<u32>`: a rate outside that range is not a baud.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Baud(u32);

impl Baud {
    /// The fastest rate whose bit is still [`super::MIN_GAP_PS`]. A bit at this rate is 2000 ps.
    pub const MAX: u32 = 500_000_000;

    pub const fn new(baud: u32) -> Option<Baud> {
        if baud >= 1 && baud <= Self::MAX {
            Some(Baud(baud))
        } else {
            None
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The one frame a serial event can send. A later format is a new variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    EightN1,
}

impl Frame {
    pub const fn as_str(self) -> &'static str {
        match self {
            Frame::EightN1 => "8N1",
        }
    }
}

/// One run of bytes, back to back from `at`, least significant bit first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialEvent {
    at: Time,
    baud: Baud,
    frame: Frame,
    bytes: Vec<u8>,
}

impl SerialEvent {
    pub(super) fn new(at: Time, baud: Baud, frame: Frame, bytes: Vec<u8>) -> SerialEvent {
        SerialEvent {
            at,
            baud,
            frame,
            bytes,
        }
    }

    pub fn at(&self) -> Time {
        self.at
    }

    pub fn baud(&self) -> Baud {
        self.baud
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The first idle time after the last stop bit.
    pub fn end(&self) -> Time {
        let end = serial_end(
            self.at.as_ps(),
            u64::from(self.baud.get()),
            self.bytes.len(),
        )
        .expect("a stored serial event ends inside u64");
        Time::from_ps(end)
    }
}

/// `mark` at t = 0 and between events. Each event is 8N1 from its own `t_ps`.
#[derive(Clone, Debug, PartialEq)]
pub struct Serial {
    mark: Value,
    space: Value,
    events: Vec<SerialEvent>,
}

/// The bytes [`Netlist::append`](super::Netlist::append) adds to a serial source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialSend {
    pub baud: Baud,
    pub frame: Frame,
    pub bytes: Vec<u8>,
}

impl SerialSend {
    /// The first idle time after these frames start at `at`.
    /// `None` when that time does not fit in a `u64`.
    pub fn end(&self, at: Time) -> Option<Time> {
        serial_end(at.as_ps(), u64::from(self.baud.get()), self.bytes.len()).map(Time::from_ps)
    }
}

impl Serial {
    pub fn mark(&self) -> &Value {
        &self.mark
    }

    pub fn space(&self) -> &Value {
        &self.space
    }

    pub fn events(&self) -> &[SerialEvent] {
        &self.events
    }

    pub(super) fn push(&mut self, at: Time, send: SerialSend) {
        debug_assert!(!send.bytes.is_empty());
        debug_assert!(at > Time::ZERO);
        if let Some(last) = self.events.last() {
            debug_assert!(at.as_ps() > last.at.as_ps());
            debug_assert!(at.as_ps() >= last.end().as_ps());
        }
        debug_assert!(
            serial_end(at.as_ps(), u64::from(send.baud.get()), send.bytes.len()).is_some()
        );
        self.events
            .push(SerialEvent::new(at, send.baud, send.frame, send.bytes));
    }

    fn level_of(&self, mark: bool) -> f64 {
        if mark {
            self.mark.si.get()
        } else {
            self.space.si.get()
        }
    }

    /// The event that owns `at`, when `at` is at or after its start and before its end.
    fn covering(&self, at: u64) -> Option<&SerialEvent> {
        let index = self.events.partition_point(|event| event.at.as_ps() <= at);
        let event = self.events.get(index.checked_sub(1)?)?;
        (at < event.end().as_ps()).then_some(event)
    }

    fn sample(&self, at: Time, before: bool) -> f64 {
        let mark = self.mark.si.get();
        let Some(event) = self.covering(at.as_ps()) else {
            return mark;
        };
        let start = event.at.as_ps();
        let baud = u64::from(event.baud.get());
        let nbits = bit_count(&event.bytes);
        let index = bit_index(start, baud, nbits, at.as_ps());
        let on_boundary = boundary(start, baud, index) == Some(at.as_ps());
        let place = if before && on_boundary {
            if index == 0 {
                return mark;
            }
            index - 1
        } else {
            index
        };
        self.level_of(bit_is_mark(&event.bytes, place))
    }

    fn change_after(&self, at: Time) -> Option<Time> {
        let current = self.sample(at, false);
        let at_ps = at.as_ps();
        // Ends increase because a stored event starts at or after the previous end.
        let from = self
            .events
            .partition_point(|event| event.end().as_ps() <= at_ps);
        // A start bit is space and the idle line is mark, so the next jump is in this
        // event or the one after it.
        for event in self.events.iter().skip(from).take(2) {
            let start = event.at.as_ps();
            let baud = u64::from(event.baud.get());
            let nbits = bit_count(&event.bytes);
            // Boundary `nbits` is the frame end. The stop bit and the idle line are both mark,
            // so that boundary is not a jump and this scan does not visit it.
            let first = if at_ps < start {
                0
            } else {
                bit_index(start, baud, nbits, at_ps) + 1
            };
            for index in first..nbits {
                let time = boundary(start, baud, index).expect("a stored serial bit fits in u64");
                if self.level_of(bit_is_mark(&event.bytes, index)) != current {
                    return Some(Time::from_ps(time));
                }
            }
        }
        None
    }
}

/// The end of `nbytes` frames, or `None` when `baud` is 0 or the end does not fit in `u64`.
pub(super) fn serial_end(start: u64, baud: u64, nbytes: usize) -> Option<u64> {
    let bits = u64::try_from(nbytes).ok()?.checked_mul(10)?;
    boundary(start, baud, bits)
}

/// Boundary `index` of a bit train from `start` at `baud` bits a second, or `None` when `baud`
/// is 0 or the boundary does not fit in `u64`.
pub(crate) fn boundary(start: u64, baud: u64, index: u64) -> Option<u64> {
    if baud == 0 {
        return None;
    }
    let span = u128::from(index).checked_mul(PS_PER_S)? / u128::from(baud);
    start.checked_add(u64::try_from(span).ok()?)
}

fn bit_count(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len())
        .expect("a stored serial event's length fits in u64")
        .checked_mul(10)
        .expect("a stored serial event's bit count fits in u64")
}

/// Largest `i` with `boundary(i) <= at`. `nbits > 0` and `boundary(0) <= at < boundary(nbits)`.
fn bit_index(start: u64, baud: u64, nbits: u64, at: u64) -> u64 {
    let mut lo = 0;
    let mut hi = nbits;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        match boundary(start, baud, mid) {
            Some(time) if time <= at => lo = mid,
            _ => hi = mid,
        }
    }
    lo
}

/// Start is space, stop is mark, and the eight data bits are least significant first.
fn bit_is_mark(bytes: &[u8], index: u64) -> bool {
    let place = (index % 10) as u8;
    match place {
        0 => false,
        9 => true,
        _ => {
            let byte = bytes[(index / 10) as usize];
            (byte >> (place - 1)) & 1 == 1
        }
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

    pub(super) fn serial(mark: Value, space: Value, events: Vec<SerialEvent>) -> Waveform {
        Waveform {
            shape: Shape::Serial(Serial {
                mark,
                space,
                events,
            }),
        }
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    pub(crate) fn as_steps(&self) -> Option<&Schedule<Value>> {
        match &self.shape {
            Shape::Steps(schedule) => Some(schedule),
            Shape::Step(_) | Shape::Pulse(_) | Shape::Serial(_) => None,
        }
    }

    pub(crate) fn steps_mut(&mut self) -> Option<&mut Schedule<Value>> {
        match &mut self.shape {
            Shape::Steps(schedule) => Some(schedule),
            Shape::Step(_) | Shape::Pulse(_) | Shape::Serial(_) => None,
        }
    }

    pub(crate) fn as_serial(&self) -> Option<&Serial> {
        match &self.shape {
            Shape::Serial(serial) => Some(serial),
            Shape::Steps(_) | Shape::Step(_) | Shape::Pulse(_) => None,
        }
    }

    pub(crate) fn serial_mut(&mut self) -> Option<&mut Serial> {
        match &mut self.shape {
            Shape::Serial(serial) => Some(serial),
            Shape::Steps(_) | Shape::Step(_) | Shape::Pulse(_) => None,
        }
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
            Shape::Serial(serial) => serial.sample(at, true),
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
            Shape::Serial(serial) => serial.sample(at, false),
        }
    }

    /// The next jump strictly after `at`.
    pub fn next_change_after(&self, at: Time) -> Option<Time> {
        match &self.shape {
            Shape::Steps(schedule) => schedule.next_change_after(at),
            Shape::Step(step) => (at < step.at).then_some(step.at),
            Shape::Pulse(pulse) => pulse_edge_after(pulse, at.as_ps()).map(Time::from_ps),
            Shape::Serial(serial) => serial.change_after(at),
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
    use crate::netlist::{Event, Source};

    use super::*;
    use crate::finite::Finite;

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

    fn frames(events: &[(u64, u32, &[u8])]) -> Waveform {
        Waveform::serial(
            level(5.0),
            level(0.0),
            events
                .iter()
                .map(|(t, baud, bytes)| {
                    SerialEvent::new(
                        at(*t),
                        Baud::new(*baud).unwrap(),
                        Frame::EightN1,
                        bytes.to_vec(),
                    )
                })
                .collect(),
        )
    }

    fn changes(wave: &Waveform) -> Vec<(u64, f64, f64)> {
        std::iter::successors(wave.next_change_after(Time::ZERO), |&t| {
            wave.next_change_after(t)
        })
        .map(|t| (t.as_ps(), wave.before(t), wave.after(t)))
        .collect()
    }

    #[test]
    fn a_byte_at_9600_jumps_at_the_floor_boundaries() {
        let wave = frames(&[(1_000_000_000, 9_600, &[0x31])]);
        assert_eq!(
            changes(&wave),
            [
                (1_000_000_000, 5.0, 0.0),
                (1_104_166_666, 0.0, 5.0),
                (1_208_333_333, 5.0, 0.0),
                (1_520_833_333, 0.0, 5.0),
                (1_729_166_666, 5.0, 0.0),
                (1_937_500_000, 0.0, 5.0),
            ]
        );
        assert_eq!(wave.before(at(0)), 5.0);
        assert_eq!(wave.after(at(1_208_333_334)), 0.0);
        assert_eq!(wave.next_change_after(at(1_937_500_000)), None);
    }

    #[test]
    fn frame_two_of_a_run_starts_at_the_cumulative_boundary() {
        let wave = frames(&[(5_000_000_000, 9_600, b"0hi")]);
        let start = 7_083_333_333;
        assert_eq!(wave.before(at(start)), 5.0);
        assert_eq!(wave.after(at(start)), 0.0);
        assert_eq!(wave.next_change_after(at(start - 1)), Some(at(start)));
        let times: Vec<_> = changes(&wave).into_iter().map(|(t, _, _)| t).collect();
        assert!(times.contains(&start));
    }

    #[test]
    fn equal_bits_are_one_level_and_the_stop_returns_to_mark() {
        let at_fast = |byte: u8| frames(&[(10_000, 500_000_000, &[byte])]);
        let times = |byte: u8| -> Vec<u64> {
            changes(&at_fast(byte))
                .into_iter()
                .map(|(t, _, _)| t)
                .collect()
        };
        assert_eq!(times(0x00), [10_000, 28_000]);
        assert_eq!(times(0xff), [10_000, 12_000]);
        assert_eq!(
            times(0x55),
            [
                10_000, 12_000, 14_000, 16_000, 18_000, 20_000, 22_000, 24_000, 26_000, 28_000
            ]
        );
        let zero = at_fast(0x00);
        assert_eq!(
            (zero.before(at(28_000)), zero.after(at(28_000))),
            (0.0, 5.0)
        );
        assert_eq!(zero.after(at(30_000)), 5.0);
        assert_eq!(zero.next_change_after(at(28_000)), None);
        assert_eq!(
            times(0x31),
            [10_000, 12_000, 14_000, 20_000, 24_000, 28_000]
        );
    }

    #[test]
    fn a_second_event_counts_bits_from_its_own_start() {
        let wave = frames(&[(1_000, 9_600, &[0x00]), (1_041_667_666, 9_600, &[0xff])]);
        let times: Vec<_> = changes(&wave).into_iter().map(|(t, _, _)| t).collect();
        assert_eq!(times, [1_000, 937_501_000, 1_041_667_666, 1_145_834_332]);
        assert_eq!(wave.before(at(1_041_667_666)), 5.0);
        assert_eq!(wave.after(at(1_041_667_666)), 0.0);
    }

    #[test]
    fn a_boundary_past_u64_uses_the_wide_product() {
        assert_eq!(
            boundary(0, 500_000_000, 20_000_000),
            Some(40_000_000_000),
            "20_000_000 * 10^12 does not fit in u64"
        );
        assert_eq!(
            boundary(0, 2, 20_000_000),
            Some(10_000_000_000_000_000_000),
            "20_000_000 * 10^12 does not fit in u64"
        );
    }

    #[test]
    fn a_gap_between_events_stays_at_mark() {
        let wave = frames(&[
            (10_000, 500_000_000, &[0x00]),
            (50_000, 500_000_000, &[0xff]),
        ]);
        let end = 30_000;
        let start = 50_000;
        for t in [end, 40_000] {
            assert_eq!((wave.before(at(t)), wave.after(at(t))), (5.0, 5.0), "{t}");
            assert_eq!(wave.next_change_after(at(t)), Some(at(start)), "{t}");
        }
        assert_eq!((wave.before(at(start)), wave.after(at(start))), (5.0, 0.0));
        assert_eq!(wave.next_change_after(at(start)), Some(at(52_000)));
    }

    #[test]
    fn the_next_jump_skips_events_that_have_ended() {
        let mut specs = Vec::new();
        let mut t = 10_000u64;
        for _ in 0..8 {
            specs.push((t, vec![0x55u8]));
            t += 20_000;
        }
        let idle = t;
        let last = t + 40_000;
        specs.push((last, vec![0x00]));
        let wave = Waveform::serial(
            level(5.0),
            level(0.0),
            specs
                .into_iter()
                .map(|(start, bytes)| {
                    SerialEvent::new(
                        at(start),
                        Baud::new(500_000_000).unwrap(),
                        Frame::EightN1,
                        bytes,
                    )
                })
                .collect(),
        );
        assert_eq!(wave.before(at(idle)), 5.0);
        assert_eq!(wave.after(at(idle)), 5.0);
        assert_eq!(wave.next_change_after(at(idle)), Some(at(last)));
        assert_eq!(wave.before(at(last)), 5.0);
        assert_eq!(wave.after(at(last)), 0.0);
        assert_eq!(wave.next_change_after(at(last)), Some(at(last + 18_000)));
    }
}
