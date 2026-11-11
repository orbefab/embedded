//! A chain of WS2812E-1313 LEDs as a device. The host's pulse engine watches `DIN` and calls
//! [`PulseTarget`]. This device keeps the bits and the colours.
//!
//! The times are the Worldsemi WS2812E-1313 datasheet's, V1.1, page 3, "Data Transfer Time":
//! T0H 220 ns to 380 ns, T1H 580 ns to 1 µs. A bit is a 0 or a 1 by its high alone. A high
//! outside both windows is rule `bit` and is dropped. The host owns the reset, a low longer
//! than 280 µs, and calls [`PulseTarget::idle`] there.

use orbe_model::{Cx, Device, Drives, Error, Params, PulseTarget, State, Time};

/// The most LEDs one part holds.
pub const LED_MAX: u16 = 1024;

const LEDS: usize = LED_MAX as usize;
/// T0H, inclusive, picoseconds.
const T0H: std::ops::RangeInclusive<u64> = 220_000..=380_000;
/// T1H, inclusive.
const T1H: std::ops::RangeInclusive<u64> = 580_000..=1_000_000;

/// The bits still open and the colours latched so far.
///
/// `open` is the bit of a high that ended. It shifts when that low ends, or when the line
/// idles, because a latch can cut the low before a rise.
#[derive(Clone, Copy, Debug)]
pub struct Ws2812 {
    count: u16,
    /// The LED the next bit belongs to. `count` once the chain is full.
    index: u16,
    acc: u32,
    got: u8,
    open: Option<bool>,
    pending: [[u8; 3]; LEDS],
    leds: [[u8; 3]; LEDS],
    /// One bit per LED. Set once that LED has its 24 bits. A latch copies those.
    have: [u64; LEDS / 64],
}

impl State for Ws2812 {}

impl Ws2812 {
    fn power_on(count: u16) -> Ws2812 {
        Ws2812 {
            count,
            index: 0,
            acc: 0,
            got: 0,
            open: None,
            pending: [[0; 3]; LEDS],
            leds: [[0; 3]; LEDS],
            have: [0; LEDS / 64],
        }
    }

    fn shift(&mut self, one: bool) {
        if self.index >= self.count {
            return;
        }
        self.acc = (self.acc << 1) | u32::from(one);
        self.got += 1;
        if self.got == 24 {
            let g = (self.acc >> 16) as u8;
            let r = (self.acc >> 8) as u8;
            let b = self.acc as u8;
            let index = usize::from(self.index);
            self.pending[index] = [g, r, b];
            self.have[index / 64] |= 1 << (index % 64);
            self.index += 1;
            self.acc = 0;
            self.got = 0;
        }
    }
}

impl Device for Ws2812 {
    fn new(params: &Params<'_>) -> Result<Ws2812, Error> {
        let Some(leds) = params.int("leds") else {
            return Err(Error("leds is 1 to 1024"));
        };
        let Some(count) = u16::try_from(leds)
            .ok()
            .filter(|count| (1..=LED_MAX).contains(count))
        else {
            return Err(Error("leds is 1 to 1024"));
        };
        Ok(Ws2812::power_on(count))
    }

    fn run_until(&mut self, _cx: &mut Cx, _limit: Time) -> Option<orbe_model::Stop> {
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }
}

impl PulseTarget for Ws2812 {
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
        let count = usize::from(self.count);
        for index in 0..count {
            if self.have[index / 64] & (1 << (index % 64)) != 0 {
                self.leds[index] = self.pending[index];
            }
        }
        self.have = [0; LEDS / 64];
        self.index = 0;
        self.acc = 0;
        self.got = 0;
        let mut bytes = [0u8; LEDS * 3];
        for (index, px) in self.leds.iter().take(count).enumerate() {
            bytes[index * 3] = px[0];
            bytes[index * 3 + 1] = px[1];
            bytes[index * 3 + 2] = px[2];
        }
        cx.view(&bytes[..count * 3]);
    }
}
