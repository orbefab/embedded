//! The DHT22, also sold as the AM2302, as a device: a temperature and humidity sensor that
//! answers a host's start pulse on one open-drain line with 40 bits.
//!
//! Every time is the typical of Aosong's AM2302 product manual, version 1.1, table 6, "single
//! bus signal characteristics", unless a constant says otherwise. A read goes:
//!
//! 1. The host pulls the line low for at least `Tbe`, 0.8 ms, and releases it.
//! 2. `Tgo`, 30 us, after the line rises, the sensor pulls it low for `Trel`, 80 us, and
//!    releases it for `Treh`, 80 us.
//! 3. Each of 40 bits, most significant first, is low for `TLOW`, 50 us, then released for
//!    `TH0`, 26 us, for a 0, or `TH1`, 70 us, for a 1.
//! 4. The sensor pulls the line low for `Ten`, 50 us, and releases it.
//!
//! The bits are humidity, then temperature, each 16 bits of tenths, then a checksum byte, the
//! low 8 bits of the sum of the other four bytes (table 5). Temperature's top bit is its sign,
//! and the 15 below it are the magnitude (table 5, and the -10.1 C example after it).
//!
//! The manual's note under figure 6 says a read sends the last measurement, and section 7.3
//! says the end of each reply starts the next one. So a read sends what the sensor measured at
//! the end of the read before, and the first read sends what it measured during its warm-up
//! after power-on (section 7.4, step 1). The sensor measures the world at those times. A
//! measurement takes no time here.
//!
//! The sensor answers no start inside its 2 s warm-up (section 7.4, step 1) or within 2 s of
//! the last start it answered (section 7.1, note 3). Such a start breaks rule `interval`, and
//! the line stays released.

use std::sync::Arc;

use orbe_model::{
    Cx, Device, Drives, Error, Idle, Params, PulseTarget, Pulses, Quantity, State, Stop, Time,
};

const US: u64 = 1_000_000;
const S: u64 = 1_000_000 * US;

/// `Tbe` minimum: the shortest host low the sensor takes for a start.
pub const START_MIN_PS: u64 = 800 * US;
/// `Tgo`: from the line's rise after a start to the sensor's first low.
pub const GO_PS: u64 = 30 * US;
/// `Trel` and `Treh`: the response's low and high.
pub const RESPONSE_LOW_PS: u64 = 80 * US;
pub const RESPONSE_HIGH_PS: u64 = 80 * US;
/// `TLOW`: the low that opens each bit.
pub const BIT_LOW_PS: u64 = 50 * US;
/// `TH0` and `TH1`: the high of a 0 and of a 1.
pub const ZERO_HIGH_PS: u64 = 26 * US;
pub const ONE_HIGH_PS: u64 = 70 * US;
/// `Ten`: the last low, before the sensor releases the line.
pub const END_LOW_PS: u64 = 50 * US;
/// Section 7.4, step 1: after power-on the sensor answers nothing for 2 s.
pub const WARM_UP_PS: u64 = 2 * S;
/// Section 7.1, note 3, and the note under figure 6: reads at least 2 s apart.
pub const INTERVAL_PS: u64 = 2 * S;

/// The range the sensor reports, in tenths: 0 to 100 %RH, and -40 to 80 C, the DHT22 sheet's
/// operating range (section 3). A world value outside it reads as its nearer end.
const HUMIDITY_TENTHS: (f64, f64) = (0.0, 1000.0);
const TEMPERATURE_TENTHS: (f64, f64) = (-400.0, 800.0);

/// One measurement, in tenths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measured {
    pub humidity: u16,
    pub temperature: i16,
}

impl Measured {
    /// What the sensor reads of the world in `cx`, rounded to its 0.1 resolution.
    fn of(cx: &Cx) -> Measured {
        let tenths =
            |quantity, (min, max): (f64, f64)| (cx.world(quantity) * 10.0).round().clamp(min, max);
        Measured {
            humidity: tenths(Quantity::Humidity, HUMIDITY_TENTHS) as u16,
            temperature: tenths(Quantity::Temperature, TEMPERATURE_TENTHS) as i16,
        }
    }

    /// The five bytes a reply sends: humidity, temperature in sign and magnitude, and the
    /// checksum.
    pub fn frame(self) -> [u8; 5] {
        let temperature = if self.temperature < 0 {
            0x8000 | self.temperature.unsigned_abs()
        } else {
            self.temperature.unsigned_abs()
        };
        let [h1, h0] = self.humidity.to_be_bytes();
        let [t1, t0] = temperature.to_be_bytes();
        let sum = [h1, h0, t1, t0]
            .iter()
            .fold(0u8, |sum, &byte| sum.wrapping_add(byte));
        [h1, h0, t1, t0, sum]
    }

    /// The reply's levels from its first low: a low pulls the line, a high releases it.
    fn reply(self) -> Vec<(bool, u64)> {
        let mut levels = vec![(false, RESPONSE_LOW_PS), (true, RESPONSE_HIGH_PS)];
        for byte in self.frame() {
            for bit in (0..8).rev() {
                let high = if byte >> bit & 1 == 1 {
                    ONE_HIGH_PS
                } else {
                    ZERO_HIGH_PS
                };
                levels.extend([(false, BIT_LOW_PS), (true, high)]);
            }
        }
        levels.push((false, END_LOW_PS));
        levels
    }
}

/// The sensor: what it measured last, when it measures next, and when it next answers a start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dht22 {
    held: Option<Measured>,
    measure_at: Option<Time>,
    ready_at: Time,
}

impl State for Dht22 {}

impl Device for Dht22 {
    const WORLD: &'static [Quantity] = &[Quantity::Temperature, Quantity::Humidity];

    fn new(_params: &Params<'_>) -> Result<Dht22, Error> {
        Ok(Dht22 {
            held: None,
            measure_at: Some(Time::from_ps(WARM_UP_PS)),
            ready_at: Time::from_ps(WARM_UP_PS),
        })
    }

    /// Measures when the run passes the time it measures at. Its drives never change: the host
    /// plays the reply.
    fn run_until(&mut self, cx: &mut Cx, limit: Time) -> Option<Stop> {
        if self.measure_at.is_some_and(|at| at <= limit) {
            self.held = Some(Measured::of(cx));
            self.measure_at = None;
        }
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }
}

impl PulseTarget for Dht22 {
    /// The line rests high on its pull-up. The sensor reads no idle.
    const IDLE: Idle = Idle {
        level: true,
        after: u64::MAX,
    };
    const OPEN_DRAIN: bool = true;

    /// A low of at least [`START_MIN_PS`] that ends at `at` is a start. The sensor's own lows
    /// are shorter, so it never takes its reply for one.
    fn pulse(&mut self, cx: &mut Cx, at: Time, level: bool, width: u64) {
        if level || width < START_MIN_PS {
            return;
        }
        if at < self.ready_at {
            cx.broke("interval", at);
            return;
        }
        let held = self
            .held
            .expect("the sensor measures by the end of its warm-up");
        let reply = held.reply();
        let start = at.as_ps() + GO_PS;
        let end = start + reply.iter().map(|&(_, ps)| ps).sum::<u64>();
        cx.pulses(
            Time::from_ps(start),
            Pulses {
                idle: true,
                pulses: Arc::from(reply),
            },
        );
        self.ready_at = Time::from_ps(at.as_ps() + INTERVAL_PS);
        self.measure_at = Some(Time::from_ps(end));
    }
}
