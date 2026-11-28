//! The 74HC595 as a device: an 8-bit shift register, a storage register, and
//! push-pull outputs. TI SNx4HC595, SCLS041J.
//!
//! `SRCLK` rising shifts `SER` into the first stage. `RCLK` rising copies the
//! shift register into storage, which drives `QA` to `QH`. When both clocks
//! rise together the storage register takes the shift register as it was, so
//! the shift register stays one clock ahead (section 8.1). `SRCLR` low clears
//! the shift register and leaves storage alone (table 8-1). `OE` high turns
//! `QA` to `QH` off. `QHp` is the shift register's last stage and stays driven.
//!
//! Each delay is the typical at VCC = 4.5 V, CL = 50 pF and 25 C, section 6.7.
//! A drive changes in `run_until`, after the edge that caused it.
//!
//! Once the supply rises to [`MIN_VCC_V`] the host drives the outputs and the device reads its
//! inputs. Once it falls below [`RELEASE_VCC_V`] the host releases every output and the device
//! ignores its inputs, so a supply that returns has not shifted.

use orbe_model::{Cx, Device, Drive, Drives, Error, Levels, Params, State, Stop, Time};

/// Watch order.
pub const WATCH_SER: usize = 0;
pub const WATCH_SRCLK: usize = 1;
pub const WATCH_RCLK: usize = 2;
pub const WATCH_SRCLR: usize = 3;
pub const WATCH_OE: usize = 4;
pub const WATCH_SUPPLY: usize = 5;
/// The logic levels in `prev`. The supply watch is not one of them.
const LOGIC: usize = 5;

/// `QA` through `QH`, then `QHp`.
pub const OUTPUTS: usize = 9;
pub const QA: usize = 0;
pub const QH: usize = 7;
pub const QHP: usize = 8;

/// `SRCLK` to `QHp`, tpd typical.
pub const SHIFT_PS: u64 = 17_000;
/// `RCLK` to `QA`–`QH`, tpd typical.
pub const LATCH_PS: u64 = 17_000;
/// `SRCLR` low to `QHp`, tPHL typical.
pub const CLEAR_PS: u64 = 18_000;
/// `OE` low to the outputs on, ten typical.
pub const ENABLE_PS: u64 = 15_000;
/// `OE` high to the outputs off, tdis typical.
pub const DISABLE_PS: u64 = 23_000;
/// Recommended minimum supply, section 6.3. The chip powers up once its supply rises to it.
pub const MIN_VCC_V: f64 = 2.0;
/// The chip powers down once its supply falls below this. The sheet gives no power-on reset or
/// hysteresis, so the 0.2 V band is a guess.
pub const RELEASE_VCC_V: f64 = 1.8;

/// Bit 0 is the first stage (`QA`). Bit 7 is the last (`QH` and `QHp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hc595 {
    shift: u8,
    storage: u8,
    /// `QA`–`QH` follow `storage`. `OE` high turns them off.
    enabled: bool,
    prev: Option<[bool; LOGIC]>,
    /// The bit `SER` held at the rising edge, shifted in at this time.
    shift_due: Option<(Time, bool)>,
    /// The shift register captured at the rising edge, stored at this time.
    latch_due: Option<(Time, u8)>,
    clear_due: Option<Time>,
    /// Whether the outputs are enabled at this time.
    oe_due: Option<(Time, bool)>,
}

impl State for Hc595 {}

impl Hc595 {
    const fn power_on() -> Hc595 {
        Hc595 {
            shift: 0,
            storage: 0,
            enabled: true,
            prev: None,
            shift_due: None,
            latch_due: None,
            clear_due: None,
            oe_due: None,
        }
    }

    fn due(&self) -> Option<Time> {
        let mut due = None;
        let mut keep = |time: Time| due = Some(due.map_or(time, |old: Time| old.min(time)));
        if let Some((time, _)) = self.shift_due {
            keep(time);
        }
        if let Some((time, _)) = self.latch_due {
            keep(time);
        }
        if let Some(time) = self.clear_due {
            keep(time);
        }
        if let Some((time, _)) = self.oe_due {
            keep(time);
        }
        due
    }

    fn later(at: Time, delay: u64) -> Time {
        Time::from_ps(at.as_ps().saturating_add(delay))
    }
}

impl Device for Hc595 {
    const MIN_SUPPLY_V: Option<f64> = Some(MIN_VCC_V);
    const RELEASE_SUPPLY_V: Option<f64> = Some(RELEASE_VCC_V);

    fn new(_params: &Params<'_>) -> Result<Hc595, Error> {
        Ok(Hc595::power_on())
    }

    fn run_until(&mut self, _cx: &mut Cx, limit: Time) -> Option<Stop> {
        let at = self.due().filter(|&at| at <= limit)?;
        if self.clear_due == Some(at) {
            self.shift = 0;
            self.clear_due = None;
            if self.shift_due.is_some_and(|(time, _)| time == at) {
                self.shift_due = None;
            }
        }
        if let Some((time, bit)) = self.shift_due
            && time == at
        {
            self.shift = (self.shift << 1) | u8::from(bit);
            self.shift_due = None;
        }
        if let Some((time, bits)) = self.latch_due
            && time == at
        {
            self.storage = bits;
            self.latch_due = None;
        }
        if let Some((time, enabled)) = self.oe_due
            && time == at
        {
            self.enabled = enabled;
            self.oe_due = None;
        }
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    fn drive(&self) -> Drives {
        let mut drives = [Drive::Off; OUTPUTS];
        if self.enabled {
            for (stage, drive) in drives[QA..QHP].iter_mut().enumerate() {
                *drive = if self.storage & (1 << stage) == 0 {
                    Drive::Low
                } else {
                    Drive::High
                };
            }
        }
        drives[QHP] = if self.shift & (1 << QH) == 0 {
            Drive::Low
        } else {
            Drive::High
        };
        Drives::new(drives)
    }

    fn input(&mut self, _cx: &mut Cx, at: Time, levels: Levels) {
        // Below the recommended minimum the part does nothing. Dropping a pending edge
        // here does not change the drive image; the host is what releases the pins.
        if !levels.high(WATCH_SUPPLY) {
            self.prev = None;
            self.shift_due = None;
            self.latch_due = None;
            self.clear_due = None;
            self.oe_due = None;
            return;
        }
        let now = [0, 1, 2, 3, 4].map(|watch| levels.high(watch));
        let Some(prev) = self.prev else {
            self.prev = Some(now);
            if now[WATCH_OE] {
                self.oe_due = Some((Self::later(at, DISABLE_PS), false));
            }
            if !now[WATCH_SRCLR] {
                self.clear_due = Some(Self::later(at, CLEAR_PS));
            }
            return;
        };
        self.prev = Some(now);
        let rise = |watch: usize| !prev[watch] && now[watch];
        let fall = |watch: usize| prev[watch] && !now[watch];
        if fall(WATCH_SRCLR) {
            self.clear_due = Some(Self::later(at, CLEAR_PS));
        }
        if rise(WATCH_SRCLK) && now[WATCH_SRCLR] {
            self.shift_due = Some((Self::later(at, SHIFT_PS), now[WATCH_SER]));
        }
        if rise(WATCH_RCLK) {
            self.latch_due = Some((Self::later(at, LATCH_PS), self.shift));
        }
        if prev[WATCH_OE] != now[WATCH_OE] {
            let enabled = !now[WATCH_OE];
            let delay = if enabled { ENABLE_PS } else { DISABLE_PS };
            self.oe_due = Some((Self::later(at, delay), enabled));
        }
    }
}
