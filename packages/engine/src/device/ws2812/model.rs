//! A chain of WS2812E-1313 LEDs as a device. The host's pulse engine watches `DIN` and calls
//! [`PulseTarget`]. This device keeps the bits and the colours, shows the colours, and drives
//! `DOUT` when the chain has one.
//!
//! The times are the Worldsemi WS2812E-1313 datasheet's, V1.1, page 3, "Data Transfer Time":
//! T0H 220 ns to 380 ns, T1H 580 ns to 1 µs, and RES a low of more than 280 µs. A bit is a 0 or
//! a 1 by its high alone. A high outside both windows is rule `bit` and is dropped. The line
//! rests low, and a rest past RES is idle: the chain latches there.
//!
//! Once every LED holds its 24 bits, each high on `DIN` goes on to `DOUT` `delay` later, its
//! rise and its fall alike, until the chain latches. `DOUT` is low otherwise. A high a record
//! carried has already ended when the record lands, so it goes no further, and the first is
//! rule `dout_record`.

use orbe_model::{
    Cx, Device, Drive, Drives, Error, Idle, Params, PulseTarget, Shows, State, Stop, Time, View,
};

/// The most LEDs one part holds.
pub const LED_MAX: u16 = 1024;

/// T0H, inclusive, picoseconds.
const T0H: std::ops::RangeInclusive<u64> = 220_000..=380_000;
/// T1H, inclusive.
const T1H: std::ops::RangeInclusive<u64> = 580_000..=1_000_000;

/// It drives `DOUT` when the chain has one.
pub(crate) const OUTPUTS: usize = 1;

/// The most `DOUT` edges due at once. A rise and its fall come at least a bit apart, so a line
/// at the datasheet's times queues two. A line that changes faster than this many edges per
/// `delay` drops the newest high whole.
const EDGES: usize = 8;

/// A chain of up to `N` LEDs: the bits still open and the colours latched so far. `N` is how
/// many LEDs the state holds, so a copy of it costs what the chain needs.
///
/// LEDs `0..index` have their 24 bits in `pending`. `open` is the bit of a high that ended. It
/// shifts when that low ends, or when the line idles, because an idle can cut the low before a
/// rise.
#[derive(Clone, Copy, Debug)]
pub struct Ws2812<const N: usize> {
    count: u16,
    /// The LED the next bit belongs to. `count` once the chain is full.
    index: u16,
    acc: u32,
    got: u8,
    open: Option<bool>,
    pending: [[u8; 3]; N],
    leds: [[u8; 3]; N],
    /// `DIN` to `DOUT`, picoseconds. 0 for a chain with no `DOUT`.
    delay: u64,
    /// Where the last run reached. A fall goes no earlier than 1 ps past it.
    reached: Time,
    /// Whether the high on `DIN` now goes on to `DOUT`.
    passing: bool,
    /// `DOUT` is high.
    out: bool,
    /// The `DOUT` edges still due, oldest first, from `first`: the time and the level.
    edges: [(u64, bool); EDGES],
    first: u8,
    queued: u8,
    /// The sum of the channel bytes the circuit is drawing for. `0` at power-on, every channel
    /// off. Only [`Device::run_until`] changes it.
    stamped: u32,
    /// When `stamped` becomes the latched sum, if they differ.
    apply_at: Option<u64>,
}

impl<const N: usize> State for Ws2812<N> {}

impl<const N: usize> Ws2812<N> {
    fn shift(&mut self, one: bool) {
        if self.index >= self.count {
            return;
        }
        self.acc = (self.acc << 1) | u32::from(one);
        self.got += 1;
        if self.got == 24 {
            let [_, g, r, b] = self.acc.to_be_bytes();
            self.pending[usize::from(self.index)] = [g, r, b];
            self.index += 1;
            self.acc = 0;
            self.got = 0;
        }
    }

    fn next_edge(&self) -> Option<Time> {
        (self.queued > 0).then(|| Time::from_ps(self.edges[usize::from(self.first)].0))
    }

    fn take_edge(&mut self) -> bool {
        let (_, level) = self.edges[usize::from(self.first)];
        self.first = (self.first + 1) % EDGES as u8;
        self.queued -= 1;
        level
    }

    /// `DOUT` rises at `at`, unless the queue is full. Returns whether it queued the rise.
    fn queue_rise(&mut self, at: u64) -> bool {
        let full = usize::from(self.queued) == EDGES;
        if !full {
            self.queue(at, true);
        }
        !full
    }

    /// `DOUT` falls at `at`. A full queue drops the high this fall ends instead: the rise it
    /// queued last is that high's, since a high's rise is still due while its `DIN` is high.
    fn queue_fall(&mut self, at: u64) {
        if usize::from(self.queued) == EDGES {
            self.queued -= 1;
            return;
        }
        self.queue(at, false);
    }

    fn queue(&mut self, at: u64, level: bool) {
        let slot = (usize::from(self.first) + usize::from(self.queued)) % EDGES;
        self.edges[slot] = (at, level);
        self.queued += 1;
    }

    /// The latched green, red and blue bytes, added.
    fn shown_sum(&self) -> u32 {
        self.leds[..usize::from(self.count)]
            .iter()
            .flatten()
            .map(|&byte| u32::from(byte))
            .sum()
    }

    /// The latched colours draw from at least 1 ps after this call and after `reached`. The
    /// same sum again draws nothing new.
    fn follow(&mut self, at: Time) {
        let sum = self.shown_sum();
        if sum == self.stamped {
            self.apply_at = None;
            return;
        }
        let when = self
            .reached
            .as_ps()
            .saturating_add(1)
            .max(at.as_ps().saturating_add(1));
        self.apply_at = Some(when);
    }
}

impl<const N: usize> Device for Ws2812<N> {
    const SHOWS: Shows = Shows::Leds {
        kind: "ws2812",
        order: "grb",
    };

    /// `leds`, the chain's length, and `delay`, `DIN` to `DOUT` in picoseconds, 0 or absent for
    /// a chain with no `DOUT`.
    fn new(params: &Params<'_>) -> Result<Ws2812<N>, Error> {
        let Some(count) = params
            .int("leds")
            .and_then(|leds| u16::try_from(leds).ok())
            .filter(|count| (1..=LED_MAX).contains(count))
        else {
            return Err(Error("leds is 1 to 1024"));
        };
        if usize::from(count) > N {
            return Err(Error("leds is more than this state holds"));
        }
        let Ok(delay) = u64::try_from(params.int("delay").unwrap_or(0)) else {
            return Err(Error("delay is 0 or more"));
        };
        Ok(Ws2812 {
            count,
            index: 0,
            acc: 0,
            got: 0,
            open: None,
            pending: [[0; 3]; N],
            leds: [[0; 3]; N],
            delay,
            reached: Time::ZERO,
            passing: false,
            out: false,
            edges: [(0, false); EDGES],
            first: 0,
            queued: 0,
            stamped: 0,
            apply_at: None,
        })
    }

    /// Stops at each `DOUT` edge and when the latched colours start to draw.
    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        let edge = self.next_edge().filter(|&at| at <= limit);
        let draw = self.apply_at.map(Time::from_ps).filter(|&at| at <= limit);
        let Some(at) = edge.into_iter().chain(draw).min() else {
            self.reached = limit;
            return None;
        };
        if self.apply_at == Some(at.as_ps()) {
            self.stamped = self.shown_sum();
            self.apply_at = None;
        }
        while self.next_edge() == Some(at) {
            self.out = self.take_edge();
        }
        self.reached = at;
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        if self.delay == 0 {
            return Drives::OFF;
        }
        Drives::new([if self.out { Drive::High } else { Drive::Low }])
    }

    /// The sum of the channel bytes the circuit is drawing for.
    fn load(&self) -> u64 {
        u64::from(self.stamped)
    }

    /// The undo of a latch puts the chain back, colours and all, which would rewind the load.
    /// The load stays `kept`. Colours that draw a different sum take it after the landing.
    fn after_record(&mut self, at: Time, kept: u64) {
        self.stamped = u32::try_from(kept).expect("a chain's load is its channel-byte sum");
        if self.shown_sum() == self.stamped {
            self.apply_at = None;
            return;
        }
        let when = at.as_ps().saturating_add(1);
        self.apply_at = Some(self.apply_at.map_or(when, |pending| pending.max(when)));
    }

    fn view(&self) -> View<'_> {
        View::Leds(self.leds[..usize::from(self.count)].as_flattened())
    }
}

impl<const N: usize> PulseTarget for Ws2812<N> {
    const IDLE: Idle = Idle {
        level: false,
        after: 280_000_001,
    };

    fn pulse(&mut self, cx: &mut Cx, at: Time, level: bool, width: u64) {
        if !level {
            if let Some(bit) = self.open.take() {
                self.shift(bit);
            }
            if self.delay > 0 && self.index >= self.count {
                if cx.replayed() {
                    cx.broke("dout_record", at);
                } else {
                    self.passing = self.queue_rise(at.as_ps().saturating_add(self.delay));
                }
            }
            return;
        }
        if self.passing {
            self.passing = false;
            let fall = at
                .as_ps()
                .saturating_add(self.delay)
                .max(self.reached.as_ps() + 1);
            self.queue_fall(fall);
        }
        self.open = if T0H.contains(&width) {
            Some(false)
        } else if T1H.contains(&width) {
            Some(true)
        } else {
            cx.broke("bit", at);
            None
        };
    }

    fn idle(&mut self, cx: &mut Cx, at: Time, _level: bool) {
        if let Some(bit) = self.open.take() {
            self.shift(bit);
        }
        let index = usize::from(self.index);
        self.leds[..index].copy_from_slice(&self.pending[..index]);
        self.index = 0;
        self.acc = 0;
        self.got = 0;
        self.follow(at);
        cx.show();
    }
}
