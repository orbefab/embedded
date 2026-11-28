//! A chain of WS2812E-1313 LEDs as a device. The host's pulse engine watches `DIN` and calls
//! [`PulseTarget`]. This device keeps the bits and the colours, shows the colours, and drives
//! `DOUT` when the chain has one.
//!
//! T1H and RES are the Worldsemi WS2812E-1313 datasheet's, V1.1, page 3. T0H runs to 400 ns,
//! that sheet's 380 ns extended to the SK68xx T0H maximum: 220 ns to 400 ns, 580 ns to 1 µs,
//! and a low of more than 280 µs. A bit is a 0 or a 1 by its high alone. A high outside both
//! windows is rule `bit` and is dropped. The line
//! rests low, and a rest past RES is idle: the chain latches there.
//!
//! Once every LED holds its 24 bits, each high on `DIN` goes on to `DOUT` `delay` later, its
//! rise and its fall alike, until the chain latches. `DOUT` is low otherwise. A high a record
//! carried has already ended when the record lands, so the device passes its rise and its fall,
//! each `delay` later, and the host sends them on `DOUT`'s net as a record of its own. The next
//! chain takes that record as this one took the first, so a chain of parts each keeps its own
//! bits whether they came edge by edge or in a record.

use orbe_model::{
    Cx, Device, Draw, Drive, Drives, Error, Idle, Levels, Params, PulseTarget, Shows, State, Stop,
    Time, View,
};

/// Watch order: `DIN`, then the supply watch when the part sets `min_supply`: high once the supply
/// rises to `min_supply`, low once it falls below [`RELEASE_FRACTION`] of it.
pub const WATCH_DIN: usize = 0;
pub const WATCH_SUPPLY: usize = 1;

/// The share of `min_supply` below which the chain powers down, until the supply rises to
/// `min_supply` again. Neither sheet gives a power-on reset or hysteresis, so the band is a guess.
pub const RELEASE_FRACTION: f64 = 0.9;

/// The most LEDs one part holds.
pub const LED_MAX: u16 = 1024;

/// T0H, inclusive, picoseconds. 400 ns is the SK68xx sheet's T0H maximum. Arduino
/// `rgbLedWrite` sends that high, four ticks of a 10 MHz RMT, and it is still below T1H.
const T0H: std::ops::RangeInclusive<u64> = 220_000..=400_000;
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
    /// The latched green, red and blue bytes of every LED, added.
    lit: u32,
    /// Ampere at a channel value of 255.
    channel: f64,
    /// Ampere per LED, at every colour.
    idle: f64,
    /// The supply at and above which the draw holds, volt.
    holds_above_v: f64,
    /// Minimum supply in volts, or `None` when the part sets none.
    min_supply_v: Option<f64>,
    /// Whether the supply is at or above `min_supply_v`. Always true when `min_supply_v` is `None`.
    powered: bool,
}

impl<const N: usize> State for Ws2812<N> {}

impl<const N: usize> Ws2812<N> {
    /// Clears latched colours, partial bits and queued `DOUT` edges on power loss. Leaves
    /// `self.out` alone so `drive()` stays unchanged during `input()`; if `DOUT` was high,
    /// queues a fall 1 ps later so it starts low once power returns.
    fn power_off(&mut self, at: Time) {
        self.index = 0;
        self.acc = 0;
        self.got = 0;
        self.open = None;
        self.pending = [[0; 3]; N];
        self.leds = [[0; 3]; N];
        self.passing = false;
        self.first = 0;
        self.queued = 0;
        self.lit = 0;
        if self.delay > 0 && self.out {
            self.queue(at.as_ps().saturating_add(1), false);
        }
    }

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
}

impl<const N: usize> Device for Ws2812<N> {
    const SHOWS: Shows = Shows::Leds {
        kind: "ws2812",
        order: "grb",
    };

    /// `leds`, the chain's length, and `delay`, `DIN` to `DOUT` in picoseconds, 0 or absent for
    /// a chain with no `DOUT`. `channel_na` and `idle_na` are its draw in nanoamperes, at a
    /// channel value of 255 and per LED, 0 or absent for none, and `holds_above_mv` the supply
    /// in millivolts at and above which it holds.
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
        let amps = |field| match params.int(field).unwrap_or(0) {
            nanoamps @ 0.. => Ok(nanoamps as f64 * 1e-9),
            _ => Err(Error("a draw is 0 or more nanoamperes")),
        };
        let (channel, idle) = (amps("channel_na")?, amps("idle_na")?);
        let holds_above_v = match params.int("holds_above_mv").unwrap_or(1_000) {
            millivolts @ 1.. => millivolts as f64 * 1e-3,
            _ => return Err(Error("holds_above_mv is more than 0")),
        };
        let min_supply_v = match params.int("min_supply_uv") {
            None => None,
            Some(microvolts @ 1..) => Some(microvolts as f64 * 1e-6),
            Some(_) => return Err(Error("min_supply_uv is more than 0")),
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
            lit: 0,
            channel,
            idle,
            holds_above_v,
            min_supply_v,
            powered: true,
        })
    }

    fn min_supply_v(&self) -> Option<f64> {
        self.min_supply_v
    }

    fn release_supply_v(&self) -> Option<f64> {
        self.min_supply_v.map(|min| RELEASE_FRACTION * min)
    }

    /// Stops at each `DOUT` edge.
    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        let Some(at) = self.next_edge().filter(|&at| at <= limit) else {
            self.reached = limit;
            return None;
        };
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

    fn input(&mut self, _cx: &mut Cx, at: Time, levels: Levels) {
        if self.min_supply_v.is_none() {
            return;
        }
        let powered = levels.high(WATCH_SUPPLY);
        if powered == self.powered {
            return;
        }
        self.powered = powered;
        if !powered {
            self.power_off(at);
        }
    }

    /// Each LED's idle current, and each channel's current at its latched value. While unpowered
    /// every LED is off (`lit == 0`), so only the faded idle curve stays, keeping a rail behind an
    /// open switch from bouncing back above `min_supply_v`.
    fn draw(&self) -> Draw {
        Draw {
            amps: f64::from(self.count) * self.idle + self.channel * f64::from(self.lit) / 255.0,
            holds_above_v: self.holds_above_v,
        }
    }

    fn released_draw(&self) -> Draw {
        Draw {
            amps: f64::from(self.count) * self.idle,
            holds_above_v: self.holds_above_v,
        }
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
        if !self.powered {
            return;
        }
        if !level {
            if let Some(bit) = self.open.take() {
                self.shift(bit);
            }
            if self.delay > 0 && self.index >= self.count {
                let rise = at.as_ps().saturating_add(self.delay);
                if cx.replayed() {
                    cx.pass(Time::from_ps(rise), true);
                    self.passing = true;
                } else {
                    self.passing = self.queue_rise(rise);
                }
            }
            return;
        }
        if self.passing {
            self.passing = false;
            let fall = at.as_ps().saturating_add(self.delay);
            if cx.replayed() {
                cx.pass(Time::from_ps(fall), false);
            } else {
                self.queue_fall(fall.max(self.reached.as_ps() + 1));
            }
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

    fn idle(&mut self, cx: &mut Cx, _at: Time, _level: bool) {
        if !self.powered {
            return;
        }
        if let Some(bit) = self.open.take() {
            self.shift(bit);
        }
        let index = usize::from(self.index);
        self.leds[..index].copy_from_slice(&self.pending[..index]);
        self.lit = self.leds[..usize::from(self.count)]
            .as_flattened()
            .iter()
            .map(|&byte| u32::from(byte))
            .sum();
        self.index = 0;
        self.acc = 0;
        self.got = 0;
        cx.show();
    }
}
