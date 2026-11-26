//! The pulse counter (`PCNT`, `0x6001_7000`): four units of two channels each.
//!
//! Each unit holds a signed 16-bit counter driven by two channels, each with a pulse signal
//! (`sig`) and a control signal (`ctrl`) read through the GPIO matrix (`PCNT_SIG_CH0_IN0_IDX` 33
//! through `PCNT_CTRL_CH1_IN3_IDX` 48). Rising and falling edges on `sig` increment, decrement
//! or hold the counter, modified by the current level of `ctrl` (keep, invert or inhibit).
//!
//! Each unit has a glitch filter clocked by `APB_CLK` (up to 1023 cycles) on all four of its
//! inputs: a pulse shorter than `FILTER_THRES` APB cycles is ignored when `FILTER_EN` is set.
//! Reaching `CNT_H_LIM` (> 0) or `CNT_L_LIM` (< 0) resets the counter to 0. Five watchpoints
//! (`THRES1`, `THRES0`, `L_LIM`, `H_LIM` and zero crossing) latch into `PCNT_Un_STATUS_REG` and
//! raise `PCNT_INT_RAW_REG` bit `n` when enabled; source 41 (`ETS_PCNT_INTR_SOURCE`) is high
//! while `PCNT_INT_RAW_REG & PCNT_INT_ENA_REG` is nonzero.
//!
//! Register addresses, fields and reset values follow ESP-IDF v5.5.5
//! `soc/esp32s3/register/soc/pcnt_reg.h` and the ESP32-S3 TRM v1.8 Chapter 36.

use super::clock::Tick;
use super::gpio::Gpio;

/// The four PCNT units on the ESP32-S3.
pub(crate) const UNITS: usize = 4;
/// `ETS_PCNT_INTR_SOURCE`.
pub(crate) const SOURCE: u32 = 41;
/// `PCNT_INT_ENA_REG` offset and the four unit bits the model runs.
pub(crate) const INT_ENA: u32 = 0x48;
pub(crate) const INT_ENA_ALLOWED: u32 = (1 << UNITS) - 1;

/// `PCNT_SIG_CH0_IN0_IDX` in `soc/esp32s3/include/soc/gpio_sig_map.h`.
/// Unit `u` uses signals `SIG_BASE + 4 * u` (ch0 sig), `+ 1` (ch1 sig), `+ 2` (ch0 ctrl),
/// `+ 3` (ch1 ctrl).
const SIG_BASE: u16 = 33;

/// Register offsets.
const CNT_BASE: u32 = 0x30;
const INT_RAW: u32 = 0x40;
const INT_ST: u32 = 0x44;
const INT_CLR: u32 = 0x4c;
const STATUS_BASE: u32 = 0x50;
const CTRL: u32 = 0x60;
const DATE: u32 = 0xfc;

/// `PCNT_Un_CONF0_REG` reset value: `FILTER_THRES = 16`, `FILTER_EN = 1`, `THR_ZERO_EN = 1`,
/// `THR_H_LIM_EN = 1`, `THR_L_LIM_EN = 1`.
const CONF0_RESET: u32 = 16 | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 13);
/// `PCNT_CTRL_REG` reset value: `PULSE_CNT_RST_U0..U3` set, pause bits and `PCNT_CLK_EN` clear.
const CTRL_RESET: u32 = 0x0000_0055;
/// Documented writable bits in `PCNT_CTRL_REG`: reset and pause for units 0..3, plus `PCNT_CLK_EN`.
const CTRL_ALLOWED: u32 = 0x0001_00ff;
/// `PCNT_DATE_REG` reset value (`0x1907_2601`).
const DATE_RESET: u32 = 419_898_881;

/// `PCNT_Un_CONF0_REG` bits.
const FILTER_THRES_MASK: u32 = 0x3ff;
const FILTER_EN: u32 = 1 << 10;
const THR_ZERO_EN: u32 = 1 << 11;
const THR_H_LIM_EN: u32 = 1 << 12;
const THR_L_LIM_EN: u32 = 1 << 13;
const THR_THRES0_EN: u32 = 1 << 14;
const THR_THRES1_EN: u32 = 1 << 15;

/// `PCNT_Un_STATUS_REG` latched watch-event bits.
const STATUS_THRES1_LAT: u32 = 1 << 2;
const STATUS_THRES0_LAT: u32 = 1 << 3;
const STATUS_L_LIM_LAT: u32 = 1 << 4;
const STATUS_H_LIM_LAT: u32 = 1 << 5;
const STATUS_ZERO_LAT: u32 = 1 << 6;

/// One input line (`sig0`, `sig1`, `ctrl0` or `ctrl1`) through the unit's glitch filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Input {
    /// Latest raw level sampled from the GPIO matrix.
    raw: bool,
    /// Filtered level visible to the counter logic.
    stable: bool,
    /// Active glitch-filter timer while `raw != stable`: `(edge_tick, commit_tick)`.
    pending: Option<(Tick, Tick)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Unit {
    conf0: u32,
    conf1: u32,
    conf2: u32,
    count: i16,
    /// Bits `[1:0]` of `PCNT_Un_STATUS_REG`.
    zero_mode: u32,
    /// Bits `[6:2]` of `PCNT_Un_STATUS_REG`, latched until `INT_CLR` clears this unit.
    latched: u32,
    /// `[sig0, sig1, ctrl0, ctrl1]`.
    inputs: [Input; 4],
}

impl Unit {
    fn new() -> Unit {
        Unit {
            conf0: CONF0_RESET,
            conf1: 0,
            conf2: 0,
            count: 0,
            zero_mode: 0,
            latched: 0,
            inputs: [Input::default(); 4],
        }
    }

    fn filter_ticks(&self, apb: u64) -> u64 {
        if self.conf0 & FILTER_EN == 0 {
            0
        } else {
            u64::from(self.conf0 & FILTER_THRES_MASK) * apb
        }
    }

    fn thres0(&self) -> i16 {
        (self.conf1 as u16) as i16
    }

    fn thres1(&self) -> i16 {
        ((self.conf1 >> 16) as u16) as i16
    }

    fn h_lim(&self) -> i16 {
        (self.conf2 as u16) as i16
    }

    fn l_lim(&self) -> i16 {
        ((self.conf2 >> 16) as u16) as i16
    }

    /// Counter delta (`-1`, `0` or `+1`) for a rising (`rising = true`) or falling edge on
    /// `channel` (`0` or `1`), using that channel's filtered `ctrl` level.
    fn edge_delta(&self, channel: usize, rising: bool) -> i16 {
        let base = if channel == 0 { 16 } else { 24 };
        let neg_mode = (self.conf0 >> base) & 0x3;
        let pos_mode = (self.conf0 >> (base + 2)) & 0x3;
        let hctrl_mode = (self.conf0 >> (base + 4)) & 0x3;
        let lctrl_mode = (self.conf0 >> (base + 6)) & 0x3;

        let edge_mode = if rising { pos_mode } else { neg_mode };
        let mut delta = match edge_mode {
            1 => 1i16,
            2 => -1i16,
            _ => 0i16,
        };
        if delta == 0 {
            return 0;
        }
        let ctrl_high = self.inputs[2 + channel].stable;
        let ctrl_mode = if ctrl_high { hctrl_mode } else { lctrl_mode };
        match ctrl_mode {
            0 => {}
            1 => delta = -delta,
            _ => delta = 0,
        }
        delta
    }

    fn status(&self) -> u32 {
        (self.zero_mode & 0x3) | self.latched
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pcnt {
    units: [Unit; UNITS],
    ctrl: u32,
    int_raw: u32,
    int_ena: u32,
    date: u32,
    /// Whether SYSTEM's `PCNT_CLK_EN` is set and `PCNT_RST` is clear.
    clocked: bool,
}

impl Pcnt {
    pub(crate) fn new() -> Pcnt {
        Pcnt {
            units: std::array::from_fn(|_| Unit::new()),
            ctrl: CTRL_RESET,
            int_raw: 0,
            int_ena: 0,
            date: DATE_RESET,
            clocked: false,
        }
    }

    fn unit_reset(&self, unit: usize) -> bool {
        self.ctrl & (1 << (2 * unit)) != 0
    }

    fn unit_paused(&self, unit: usize) -> bool {
        self.ctrl & (1 << (2 * unit + 1)) != 0
    }

    /// Synchronizes PCNT's module clock state from `SYSTEM_PERIP_CLK_EN0_REG` / `RST_EN0_REG`.
    /// When the clock turns on, every input snaps to its current GPIO matrix level without
    /// counting a spurious edge.
    pub(crate) fn set_clocked(&mut self, now: Tick, clocked: bool, gpio: &Gpio) {
        if self.clocked == clocked {
            return;
        }
        if self.clocked {
            self.settle(now);
        }
        self.clocked = clocked;
        if clocked {
            for (u, unit) in self.units.iter_mut().enumerate() {
                for s in 0..4 {
                    let level = Self::signal_level(gpio, u, s);
                    unit.inputs[s] = Input {
                        raw: level,
                        stable: level,
                        pending: None,
                    };
                }
            }
        }
    }

    /// Whether any unit currently has an active glitch-filter timer waiting to commit.
    pub(crate) fn has_pending(&self) -> bool {
        self.units
            .iter()
            .any(|unit| unit.inputs.iter().any(|input| input.pending.is_some()))
    }

    fn signal_level(gpio: &Gpio, unit: usize, slot: usize) -> bool {
        let signal = SIG_BASE + (4 * unit + slot) as u16;
        gpio.input(signal).unwrap_or(false)
    }

    /// Commits every pending glitch-filter transition due at or before `through`, in time order.
    pub(crate) fn settle(&mut self, through: Tick) {
        if !self.clocked {
            return;
        }
        for u in 0..UNITS {
            loop {
                let earliest = self.units[u]
                    .inputs
                    .iter()
                    .filter_map(|input| input.pending.map(|(_, at)| at))
                    .filter(|&at| at <= through)
                    .min();
                let Some(at) = earliest else {
                    break;
                };
                // Apply control transitions at `at` before signal edges at `at`.
                for slot in 2..4 {
                    let input = &mut self.units[u].inputs[slot];
                    if input.pending.is_some_and(|(_, commit)| commit == at) {
                        input.stable = input.raw;
                        input.pending = None;
                    }
                }
                let mut delta = 0i16;
                for channel in 0..2 {
                    let input = &mut self.units[u].inputs[channel];
                    if input.pending.is_some_and(|(_, commit)| commit == at) {
                        let rising = input.raw;
                        input.stable = rising;
                        input.pending = None;
                        delta += self.units[u].edge_delta(channel, rising);
                    }
                }
                if delta != 0 {
                    self.apply_delta(u, delta);
                }
            }
        }
    }

    /// Samples all 16 GPIO matrix inputs at `at`, cancelling glitches shorter than the filter
    /// threshold and starting timers (or counting immediately when unfiltered) for new edges.
    pub(crate) fn note_pins(&mut self, at: Tick, gpio: &Gpio, apb: u64) {
        if !self.clocked {
            return;
        }
        self.settle(at);
        for u in 0..UNITS {
            let filter_ticks = self.units[u].filter_ticks(apb);
            if filter_ticks == 0 {
                // Update control levels before evaluating signal edges on the same tick.
                for slot in 2..4 {
                    let level = Self::signal_level(gpio, u, slot);
                    let input = &mut self.units[u].inputs[slot];
                    input.raw = level;
                    input.stable = level;
                    input.pending = None;
                }
                let mut delta = 0i16;
                for channel in 0..2 {
                    let level = Self::signal_level(gpio, u, channel);
                    let input = &mut self.units[u].inputs[channel];
                    let changed = input.stable != level;
                    input.raw = level;
                    input.stable = level;
                    input.pending = None;
                    if changed {
                        delta += self.units[u].edge_delta(channel, level);
                    }
                }
                if delta != 0 {
                    self.apply_delta(u, delta);
                }
            } else {
                for slot in 0..4 {
                    let level = Self::signal_level(gpio, u, slot);
                    let input = &mut self.units[u].inputs[slot];
                    if level != input.raw {
                        input.raw = level;
                        if level == input.stable {
                            input.pending = None;
                        } else {
                            input.pending = Some((at, Tick(at.0 + filter_ticks)));
                        }
                    }
                }
            }
        }
    }

    fn apply_delta(&mut self, u: usize, delta: i16) {
        if delta == 0 || self.unit_reset(u) || self.unit_paused(u) {
            return;
        }
        let unit = &mut self.units[u];
        let prev = unit.count;
        let next_i32 = i32::from(prev) + i32::from(delta);
        let h_lim = unit.h_lim();
        let l_lim = unit.l_lim();
        let thres0 = unit.thres0();
        let thres1 = unit.thres1();

        let hit_h_lim = delta > 0 && h_lim > 0 && prev < h_lim && next_i32 >= i32::from(h_lim);
        let hit_l_lim = delta < 0 && l_lim < 0 && prev > l_lim && next_i32 <= i32::from(l_lim);
        let next = prev.wrapping_add(delta);

        let mut events = 0u32;
        if unit.conf0 & THR_THRES0_EN != 0 && next == thres0 {
            events |= STATUS_THRES0_LAT;
        }
        if unit.conf0 & THR_THRES1_EN != 0 && next == thres1 {
            events |= STATUS_THRES1_LAT;
        }

        if hit_h_lim {
            unit.count = 0;
            unit.zero_mode = 0;
            if unit.conf0 & THR_H_LIM_EN != 0 {
                events |= STATUS_H_LIM_LAT;
            }
        } else if hit_l_lim {
            unit.count = 0;
            unit.zero_mode = 1;
            if unit.conf0 & THR_L_LIM_EN != 0 {
                events |= STATUS_L_LIM_LAT;
            }
        } else {
            unit.count = next;
            let zero_cross = if prev > 0 && next == 0 {
                unit.zero_mode = 0;
                true
            } else if prev < 0 && next == 0 {
                unit.zero_mode = 1;
                true
            } else if prev < 0 && next > 0 {
                unit.zero_mode = 2;
                true
            } else if prev > 0 && next < 0 {
                unit.zero_mode = 3;
                true
            } else {
                unit.zero_mode = if next < 0 {
                    2
                } else if next > 0 {
                    3
                } else {
                    0
                };
                false
            };
            if zero_cross && unit.conf0 & THR_ZERO_EN != 0 {
                events |= STATUS_ZERO_LAT;
            }
        }

        if events != 0 {
            unit.latched |= events;
            self.int_raw |= 1 << u;
        }
    }

    /// Whether source 41 (`ETS_PCNT_INTR_SOURCE`) is asserted.
    pub(crate) fn line(&self) -> bool {
        self.int_raw & self.int_ena != 0
    }

    /// Earliest tick after `now` at which a pending glitch-filter commit could raise an enabled
    /// PCNT interrupt line.
    pub(crate) fn next_event(&self, now: Tick) -> Option<Tick> {
        if !self.clocked || self.int_ena == 0 {
            return None;
        }
        (0..UNITS)
            .filter(|&u| self.int_ena & (1 << u) != 0 && self.int_raw & (1 << u) == 0)
            .flat_map(|u| {
                self.units[u]
                    .inputs
                    .iter()
                    .filter_map(|input| input.pending.map(|(_, at)| at))
            })
            .filter(|&at| at > now)
            .min()
    }

    pub(crate) fn read(&mut self, now: Tick, offset: u32) -> Option<u32> {
        self.settle(now);
        if offset < (UNITS as u32) * 12 {
            let unit = (offset / 12) as usize;
            return Some(match offset % 12 {
                0 => self.units[unit].conf0,
                4 => self.units[unit].conf1,
                8 => self.units[unit].conf2,
                _ => return None,
            });
        }
        if (CNT_BASE..CNT_BASE + 4 * UNITS as u32).contains(&offset) {
            let unit = ((offset - CNT_BASE) / 4) as usize;
            return Some(u32::from(self.units[unit].count as u16));
        }
        if (STATUS_BASE..STATUS_BASE + 4 * UNITS as u32).contains(&offset) {
            let unit = ((offset - STATUS_BASE) / 4) as usize;
            return Some(self.units[unit].status());
        }
        Some(match offset {
            INT_RAW => self.int_raw,
            INT_ST => self.int_raw & self.int_ena,
            INT_ENA => self.int_ena,
            INT_CLR => 0,
            CTRL => self.ctrl,
            DATE => self.date,
            _ => return None,
        })
    }

    /// Writes `value` at `offset` at `now`. `Ok(true)` if the register is kept, `Ok(false)` if
    /// unkept, and `Err(setting)` when a setting is refused, writing nothing.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
        apb: u64,
    ) -> Result<bool, &'static str> {
        self.settle(now);
        if offset < (UNITS as u32) * 12 {
            let u = (offset / 12) as usize;
            match offset % 12 {
                0 => {
                    self.units[u].conf0 = value;
                    self.retime_filter(u, now, apb);
                }
                4 => self.units[u].conf1 = value,
                8 => {
                    let h_lim = (value as u16) as i16;
                    let l_lim = ((value >> 16) as u16) as i16;
                    if h_lim < 0 {
                        return Err("h_lim");
                    }
                    if l_lim > 0 {
                        return Err("l_lim");
                    }
                    self.units[u].conf2 = value;
                }
                _ => return Ok(false),
            }
            return Ok(true);
        }
        if (CNT_BASE..CNT_BASE + 4 * UNITS as u32).contains(&offset)
            || (STATUS_BASE..STATUS_BASE + 4 * UNITS as u32).contains(&offset)
        {
            return Ok(true);
        }
        match offset {
            INT_RAW | INT_ST => {}
            INT_ENA => {
                self.int_ena = value & INT_ENA_ALLOWED;
            }
            INT_CLR => {
                let clear = value & INT_ENA_ALLOWED;
                self.int_raw &= !clear;
                for (u, unit) in self.units.iter_mut().enumerate() {
                    if clear & (1 << u) != 0 {
                        unit.latched = 0;
                    }
                }
            }
            CTRL => {
                if value & !CTRL_ALLOWED != 0 {
                    return Err("ctrl");
                }
                self.ctrl = value;
                for (u, unit) in self.units.iter_mut().enumerate() {
                    if value & (1 << (2 * u)) != 0 {
                        unit.count = 0;
                        unit.zero_mode = 0;
                    }
                }
            }
            DATE => self.date = value,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Updates any pending glitch-filter timers on unit `u` after `CONF0` changes.
    fn retime_filter(&mut self, u: usize, now: Tick, apb: u64) {
        let filter_ticks = self.units[u].filter_ticks(apb);
        for slot in 0..4 {
            let input = &mut self.units[u].inputs[slot];
            if let Some((since, _)) = input.pending {
                input.pending = Some((since, Tick(since.0 + filter_ticks)));
            }
        }
        self.settle(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APB: u64 = 6; // 80 MHz APB on 480 MHz tick timeline
    const GPIO_FUNC: u32 = 1 << 12;
    const FUN_IE: u32 = 1 << 9;
    const SIG_IN_SEL: u32 = 1 << 7;

    fn wired_gpio(pads: &[u8]) -> Gpio {
        let mut gpio = Gpio::new(pads);
        for &pad in pads {
            gpio.write_io_mux(4 + 4 * u32::from(pad), GPIO_FUNC | FUN_IE)
                .unwrap();
        }
        gpio
    }

    fn route(gpio: &mut Gpio, signal: u16, pad: u8) {
        gpio.write(0x154 + 4 * u32::from(signal), SIG_IN_SEL | u32::from(pad))
            .unwrap();
    }

    /// Configures unit `u` for full x4 quadrature decoding on ch0 (A=sig, B=ctrl) and ch1
    /// (B=sig, A=ctrl), with glitch filter set to `filter_cycles` APB cycles (0 disables it).
    fn setup_quadrature(
        pcnt: &mut Pcnt,
        gpio: &mut Gpio,
        u: usize,
        pad_a: u8,
        pad_b: u8,
        filter_cycles: u32,
    ) {
        route(gpio, SIG_BASE + (4 * u) as u16, pad_a);
        route(gpio, SIG_BASE + (4 * u + 2) as u16, pad_b);
        route(gpio, SIG_BASE + (4 * u + 1) as u16, pad_b);
        route(gpio, SIG_BASE + (4 * u + 3) as u16, pad_a);

        // Ch0: A rising -> dec, A falling -> inc; B high -> invert, B low -> keep.
        // Ch1: B rising -> inc, B falling -> dec; A high -> invert, A low -> keep.
        let ch0 = (1 << 16) | (2 << 18) | (1 << 20);
        let ch1 = (2 << 24) | (1 << 26) | (1 << 28);
        let filt = if filter_cycles > 0 {
            FILTER_EN | (filter_cycles & FILTER_THRES_MASK)
        } else {
            0
        };
        let conf0 = ch0 | ch1 | filt | THR_H_LIM_EN | THR_L_LIM_EN | THR_ZERO_EN;
        let base = 12 * u as u32;
        pcnt.write(Tick(0), base, conf0, APB).unwrap();
        pcnt.write(
            Tick(0),
            base + 8,
            (100u32) | ((-100i16 as u16 as u32) << 16),
            APB,
        )
        .unwrap();
        // Release reset and unpause unit `u`.
        let ctrl = CTRL_RESET & !(1 << (2 * u));
        pcnt.write(Tick(0), CTRL, ctrl, APB).unwrap();
    }

    #[test]
    fn quadrature_counts_up_clockwise_and_down_counterclockwise() {
        let mut gpio = wired_gpio(&[4, 5]);
        let mut pcnt = Pcnt::new();
        pcnt.set_clocked(Tick(0), true, &gpio);
        setup_quadrature(&mut pcnt, &mut gpio, 0, 4, 5, 0);

        // Quadrature sequence CW: (0,0) -> (1,0) -> (1,1) -> (0,1) -> (0,0) => -4 steps
        // Wait: with ch0 pos=dec when B=0, (0,0)->(1,0) gives -1. Let's drive CCW first:
        // (0,0) -> (0,1) -> (1,1) -> (1,0) -> (0,0) => +4 steps!
        let mut t = 100u64;
        for (a, b) in [(false, true), (true, true), (true, false), (false, false)] {
            gpio.set_pad(4, a);
            gpio.set_pad(5, b);
            pcnt.note_pins(Tick(t), &gpio, APB);
            t += 1000;
        }
        assert_eq!(pcnt.read(Tick(t), CNT_BASE), Some(4));

        // Reverse direction: (0,0) -> (1,0) -> (1,1) -> (0,1) -> (0,0) => -4 steps back to 0
        for (a, b) in [(true, false), (true, true), (false, true), (false, false)] {
            gpio.set_pad(4, a);
            gpio.set_pad(5, b);
            pcnt.note_pins(Tick(t), &gpio, APB);
            t += 1000;
        }
        assert_eq!(pcnt.read(Tick(t), CNT_BASE), Some(0));
        // Zero crossing from positive to 0 sets zero_mode = 0 and ZERO_LAT.
        let status = pcnt.read(Tick(t), STATUS_BASE).unwrap();
        assert_eq!(status & 0x3, 0);
        assert_ne!(status & STATUS_ZERO_LAT, 0);

        // Another reverse cycle goes negative (-4).
        for (a, b) in [(true, false), (true, true), (false, true), (false, false)] {
            gpio.set_pad(4, a);
            gpio.set_pad(5, b);
            pcnt.note_pins(Tick(t), &gpio, APB);
            t += 1000;
        }
        assert_eq!(pcnt.read(Tick(t), CNT_BASE).unwrap() as u16 as i16, -4);
    }

    #[test]
    fn glitch_filter_ignores_short_bounce_and_counts_long_pulse() {
        let mut gpio = wired_gpio(&[4, 5]);
        let mut pcnt = Pcnt::new();
        pcnt.set_clocked(Tick(0), true, &gpio);
        route(&mut gpio, SIG_BASE, 4);
        // Unit 0 ch0: rising edge increments; filter = 80 APB cycles (480 ticks = 1 us).
        let conf0 = (1 << 18) | FILTER_EN | 80;
        pcnt.write(Tick(0), 0, conf0, APB).unwrap();
        pcnt.write(Tick(0), CTRL, CTRL_RESET & !1, APB).unwrap();

        // Short bounce: high for 479 ticks (< 480 ticks) -> ignored!
        gpio.set_pad(4, true);
        pcnt.note_pins(Tick(1000), &gpio, APB);
        gpio.set_pad(4, false);
        pcnt.note_pins(Tick(1479), &gpio, APB);
        assert_eq!(pcnt.read(Tick(2000), CNT_BASE), Some(0));

        // Valid pulse: high for 480 ticks (== 80 APB cycles) -> counted at 2000 + 480 = 2480!
        gpio.set_pad(4, true);
        pcnt.note_pins(Tick(2000), &gpio, APB);
        assert_eq!(pcnt.read(Tick(2479), CNT_BASE), Some(0));
        assert_eq!(pcnt.read(Tick(2480), CNT_BASE), Some(1));
        gpio.set_pad(4, false);
        pcnt.note_pins(Tick(3000), &gpio, APB);
        assert_eq!(pcnt.read(Tick(4000), CNT_BASE), Some(1));
    }

    #[test]
    fn limits_thresholds_pause_clear_and_interrupts_work() {
        let mut gpio = wired_gpio(&[4]);
        let mut pcnt = Pcnt::new();
        pcnt.set_clocked(Tick(0), true, &gpio);
        route(&mut gpio, SIG_BASE, 4);
        // Unit 0 ch0: rising edge increments; filter off; thres0 = 3, h_lim = 5, l_lim = -5.
        let conf0 = (1 << 18) | THR_THRES0_EN | THR_H_LIM_EN | THR_L_LIM_EN;
        pcnt.write(Tick(0), 0, conf0, APB).unwrap();
        pcnt.write(Tick(0), 4, 3, APB).unwrap();
        pcnt.write(Tick(0), 8, 5 | ((-5i16 as u16 as u32) << 16), APB)
            .unwrap();
        pcnt.write(Tick(0), INT_ENA, 1, APB).unwrap();
        pcnt.write(Tick(0), CTRL, CTRL_RESET & !1, APB).unwrap();

        let pulse = |pcnt: &mut Pcnt, gpio: &mut Gpio, t: u64| {
            gpio.set_pad(4, true);
            pcnt.note_pins(Tick(t), gpio, APB);
            gpio.set_pad(4, false);
            pcnt.note_pins(Tick(t + 100), gpio, APB);
        };

        // 3 pulses -> hits thres0 = 3, raises interrupt without resetting counter.
        for k in 0..3 {
            pulse(&mut pcnt, &mut gpio, 1000 + k * 500);
        }
        assert_eq!(pcnt.read(Tick(3000), CNT_BASE), Some(3));
        assert!(pcnt.line());
        assert_eq!(pcnt.read(Tick(3000), INT_ST), Some(1));
        assert_ne!(
            pcnt.read(Tick(3000), STATUS_BASE).unwrap() & STATUS_THRES0_LAT,
            0
        );
        pcnt.write(Tick(3100), INT_CLR, 1, APB).unwrap();
        assert!(!pcnt.line());

        // Pause freezes counter.
        pcnt.write(Tick(3200), CTRL, (CTRL_RESET & !1) | (1 << 1), APB)
            .unwrap();
        pulse(&mut pcnt, &mut gpio, 3300);
        assert_eq!(pcnt.read(Tick(3500), CNT_BASE), Some(3));
        // Unpause and send 2 more pulses -> reaches h_lim = 5 -> resets to 0 and raises H_LIM_LAT.
        pcnt.write(Tick(3600), CTRL, CTRL_RESET & !1, APB).unwrap();
        pulse(&mut pcnt, &mut gpio, 3700);
        assert_eq!(pcnt.read(Tick(3900), CNT_BASE), Some(4));
        pulse(&mut pcnt, &mut gpio, 4000);
        assert_eq!(pcnt.read(Tick(4200), CNT_BASE), Some(0));
        assert!(pcnt.line());
        let status = pcnt.read(Tick(4200), STATUS_BASE).unwrap();
        assert_ne!(status & STATUS_H_LIM_LAT, 0);
        assert_eq!(
            status & STATUS_ZERO_LAT,
            0,
            "limit wrap does not fire zero-cross"
        );

        // Clear count resets to 0.
        pcnt.write(Tick(4300), INT_CLR, 1, APB).unwrap();
        pulse(&mut pcnt, &mut gpio, 4400);
        assert_eq!(pcnt.read(Tick(4600), CNT_BASE), Some(1));
        pcnt.write(Tick(4700), CTRL, CTRL_RESET, APB).unwrap();
        pcnt.write(Tick(4800), CTRL, CTRL_RESET & !1, APB).unwrap();
        assert_eq!(pcnt.read(Tick(4900), CNT_BASE), Some(0));
    }

    #[test]
    fn invalid_limits_and_ctrl_bits_are_refused() {
        let mut pcnt = Pcnt::new();
        assert_eq!(pcnt.write(Tick(0), 8, 0xffff, APB), Err("h_lim"));
        assert_eq!(pcnt.write(Tick(0), 8, 5 << 16, APB), Err("l_lim"));
        assert_eq!(pcnt.write(Tick(0), CTRL, 1 << 8, APB), Err("ctrl"));
    }
}
