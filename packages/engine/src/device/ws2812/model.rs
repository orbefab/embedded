//! A chain of WS2812E-1313 LEDs as a device. The host's pulse engine watches `DIN` and calls
//! [`PulseTarget`]. This device keeps the bits and the colours, and shows the colours.
//!
//! The times are the Worldsemi WS2812E-1313 datasheet's, V1.1, page 3, "Data Transfer Time":
//! T0H 220 ns to 380 ns, T1H 580 ns to 1 µs, and RES a low of more than 280 µs. A bit is a 0 or
//! a 1 by its high alone. A high outside both windows is rule `bit` and is dropped. The line
//! rests low, and a rest past RES is idle: the chain latches there.

use orbe_model::{Cx, Device, Drives, Error, Idle, Params, PulseTarget, Shows, State, Stop, Time};

/// The most LEDs one part holds.
pub const LED_MAX: u16 = 1024;

/// T0H, inclusive, picoseconds.
const T0H: std::ops::RangeInclusive<u64> = 220_000..=380_000;
/// T1H, inclusive.
const T1H: std::ops::RangeInclusive<u64> = 580_000..=1_000_000;

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
}

impl<const N: usize> Device for Ws2812<N> {
    const SHOWS: Shows = Shows::Leds {
        kind: "ws2812",
        order: "grb",
    };

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
        Ok(Ws2812 {
            count,
            index: 0,
            acc: 0,
            got: 0,
            open: None,
            pending: [[0; 3]; N],
            leds: [[0; 3]; N],
        })
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }

    fn view(&self) -> &[u8] {
        self.leds[..usize::from(self.count)].as_flattened()
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
            return;
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
        if let Some(bit) = self.open.take() {
            self.shift(bit);
        }
        let index = usize::from(self.index);
        self.leds[..index].copy_from_slice(&self.pending[..index]);
        self.index = 0;
        self.acc = 0;
        self.got = 0;
        cx.show();
    }
}
