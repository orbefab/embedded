//! The Bosch BMP280 as an I2C target: temperature and pressure from the world, compensated with
//! the sheet's trimming values and the 32-bit temperature and 64-bit pressure formulas.
//!
//! BST-BMP280-DS001-26. The chip id at `0xD0` is `0x58` (§4.3.1). SDO low selects address
//! `0x76` and SDO high selects `0x77` (§5.2). CSB must be high for I2C; low selects SPI (§5.3),
//! which this model does not run. A `1` in `spi3w_en` asks for 3-wire SPI (§4.3.5), which this
//! model does not run either.
//!
//! The trimming words are the compensation example's: `dig_T1` 27504, `dig_T2` 26435,
//! `dig_T3` -1000, `dig_P1` 36477, `dig_P2` -10685, `dig_P3` 3024, `dig_P4` 2855, `dig_P5` 140,
//! `dig_P6` -7, `dig_P7` 15500, `dig_P8` -14600, `dig_P9` 6000. With those, raw temperature
//! 519888 compensates to 25.08 °C (§3.11.3). Each measurement stores the raw value that
//! compensates back to the world, so a host that runs those formulas reads the world within
//! the sheet's absolute accuracy, ±1 °C and ±1 hPa.
//!
//! Measurement time is the typical of Table 13: `1.5 ms + 2.0 ms * osr_t + 2.0 ms * osr_p`.
//! Standby in normal mode is Table 11. The IIR of §3.3.3 resets when `filter` is written, and
//! the next sample passes through. Writes to `config` while the mode is normal are ignored
//! (§4.3.5). A mode write during a conversion waits until that conversion ends (§3.6.4); a
//! second one before then is ignored.

use orbe_model::{Ack, Cx, Device, Dir, Drives, Error, I2cTarget, Params, Quantity, State, Time};

/// Watch order: SCK, SDI, CSB, SDO.
pub const WATCH_SCK: usize = 0;
pub const WATCH_SDI: usize = 1;
pub const WATCH_CSB: usize = 2;
pub const WATCH_SDO: usize = 3;

/// The device drives no pin of its own. The host's I2C engine drives SDI.
pub const OUTPUTS: usize = 0;

const REG_ID: usize = 0xD0;
const REG_RESET: usize = 0xE0;
const REG_STATUS: usize = 0xF3;
const REG_CTRL: usize = 0xF4;
const REG_CONFIG: usize = 0xF5;
const REG_PRESS: usize = 0xF7;
const REG_TEMP: usize = 0xFA;

const CHIP_ID: u8 = 0x58;
const SOFT_RESET: u8 = 0xB6;
const MEASURING: u8 = 0x08;

/// 7-bit address with SDO low. SDO high selects the next address.
pub const ADDRESS: u8 = 0x76;

const DIG_T1: i32 = 27504;
const DIG_T2: i32 = 26435;
const DIG_T3: i32 = -1000;
const DIG_P1: i64 = 36477;
const DIG_P2: i64 = -10685;
const DIG_P3: i64 = 3024;
const DIG_P4: i64 = 2855;
const DIG_P5: i64 = 140;
const DIG_P6: i64 = -7;
const DIG_P7: i64 = 15500;
const DIG_P8: i64 = -14600;
const DIG_P9: i64 = 6000;

/// Operating range the raw value is chosen inside: -40 to 85 °C, 300 to 1100 hPa.
const TEMP_MIN: f64 = -40.0;
const TEMP_MAX: f64 = 85.0;
const PRESS_MIN: f64 = 30_000.0;
const PRESS_MAX: f64 = 110_000.0;

/// One picosecond count of a microsecond.
const US: u64 = 1_000_000;

/// The register file, the conversion in progress, and the IIR memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bmp280 {
    mem: [u8; 256],
    pointer: u8,
    set_pointer: bool,
    /// Oversampling the conversion in progress was started with.
    started_t: u32,
    started_p: u32,
    measuring_until: Option<Time>,
    standby_until: Option<Time>,
    /// The first `ctrl_meas` write during a conversion. A later one is ignored.
    pending_ctrl: Option<u8>,
    filter_t: Option<i32>,
    filter_p: Option<i32>,
    /// `t_fine` from the last temperature conversion, for a pressure reading that skips it.
    t_fine: i32,
}

impl State for Bmp280 {}

impl Bmp280 {
    const fn power_on() -> Bmp280 {
        let mut mem = [0u8; 256];
        // Little-endian calibration, Table 17.
        let calib: [(usize, u16); 12] = [
            (0x88, DIG_T1 as u16),
            (0x8A, DIG_T2 as u16),
            (0x8C, DIG_T3 as u16),
            (0x8E, DIG_P1 as u16),
            (0x90, DIG_P2 as u16),
            (0x92, DIG_P3 as u16),
            (0x94, DIG_P4 as u16),
            (0x96, DIG_P5 as u16),
            (0x98, DIG_P6 as u16),
            (0x9A, DIG_P7 as u16),
            (0x9C, DIG_P8 as u16),
            (0x9E, DIG_P9 as u16),
        ];
        let mut k = 0;
        while k < calib.len() {
            let (at, word) = calib[k];
            mem[at] = word as u8;
            mem[at + 1] = (word >> 8) as u8;
            k += 1;
        }
        mem[REG_ID] = CHIP_ID;
        mem[REG_PRESS] = 0x80;
        mem[REG_TEMP] = 0x80;
        Bmp280 {
            mem,
            pointer: 0,
            set_pointer: false,
            started_t: 0,
            started_p: 0,
            measuring_until: None,
            standby_until: None,
            pending_ctrl: None,
            filter_t: None,
            filter_p: None,
            t_fine: 0,
        }
    }

    fn store(&mut self, base: usize, adc: i32) {
        let raw = (adc.clamp(0, 0xF_FFFF) as u32) << 4;
        self.mem[base] = (raw >> 16) as u8;
        self.mem[base + 1] = (raw >> 8) as u8;
        self.mem[base + 2] = raw as u8;
    }

    fn capture(&mut self, cx: &Cx) {
        let (temp, pressure) = world_sample(cx);
        if self.started_t == 0 {
            self.store(REG_TEMP, 0x80000);
        } else {
            let (adc, t_fine) = adc_for_temperature(temp);
            self.t_fine = t_fine;
            let coeff = filter_coeff(self.mem[REG_CONFIG]);
            let adc = iir(&mut self.filter_t, coeff, adc);
            self.store(REG_TEMP, adc);
        }
        if self.started_p == 0 {
            self.store(REG_PRESS, 0x80000);
        } else {
            let adc = adc_for_pressure(self.t_fine, pressure);
            let coeff = filter_coeff(self.mem[REG_CONFIG]);
            let adc = iir(&mut self.filter_p, coeff, adc);
            self.store(REG_PRESS, adc);
        }
    }

    fn kick(&mut self, at: Time) {
        let mode = self.mem[REG_CTRL] & 0b11;
        if mode == 0 {
            return;
        }
        self.started_t = osr_count(self.mem[REG_CTRL] >> 5);
        self.started_p = osr_count(self.mem[REG_CTRL] >> 2);
        let us = measure_us(self.started_t, self.started_p);
        if us == 0 {
            self.store(REG_TEMP, 0x80000);
            self.store(REG_PRESS, 0x80000);
            self.after_conversion(at);
            return;
        }
        self.mem[REG_STATUS] |= MEASURING;
        self.measuring_until = after(at, us);
    }

    fn after_conversion(&mut self, at: Time) {
        if let Some(byte) = self.pending_ctrl.take() {
            self.mem[REG_CTRL] = byte;
            self.kick(at);
            return;
        }
        let mode = self.mem[REG_CTRL] & 0b11;
        if mode == 0b11 {
            self.standby_until = after(at, standby_us(self.mem[REG_CONFIG] >> 5));
        } else if mode != 0 {
            self.mem[REG_CTRL] &= !0b11;
        }
    }

    fn finish(&mut self, cx: &Cx, at: Time) {
        self.measuring_until = None;
        self.mem[REG_STATUS] &= !MEASURING;
        self.capture(cx);
        self.after_conversion(at);
    }

    fn write_ctrl(&mut self, at: Time, byte: u8) {
        if self.measuring_until.is_some() {
            if self.pending_ctrl.is_none() {
                self.pending_ctrl = Some(byte);
            }
            return;
        }
        self.standby_until = None;
        self.mem[REG_CTRL] = byte;
        self.kick(at);
    }

    /// §4.3.5: a write in normal mode may be ignored. Sleep and forced take it. A change of
    /// `filter` clears the IIR memory. `spi3w_en` names 3-wire SPI, which is not modelled.
    fn write_config(&mut self, cx: &mut Cx, at: Time, byte: u8) {
        if self.mem[REG_CTRL] & 0b11 == 0b11 {
            return;
        }
        let old = (self.mem[REG_CONFIG] >> 2) & 0b111;
        let new = (byte >> 2) & 0b111;
        self.mem[REG_CONFIG] = byte;
        if old != new {
            self.filter_t = None;
            self.filter_p = None;
        }
        if byte & 1 == 1 {
            cx.broke("spi3w", at);
        }
    }
}

impl Device for Bmp280 {
    const WORLD: &'static [Quantity] = &[Quantity::Temperature, Quantity::Pressure];

    fn new(_params: &Params<'_>) -> Result<Bmp280, Error> {
        Ok(Bmp280::power_on())
    }

    fn run_until(&mut self, cx: &mut Cx, limit: Time) -> Option<orbe_model::Stop> {
        loop {
            let next = [self.measuring_until, self.standby_until]
                .into_iter()
                .flatten()
                .min();
            let Some(at) = next else {
                break;
            };
            if at > limit {
                break;
            }
            if self.measuring_until == Some(at) {
                self.finish(cx, at);
            } else {
                self.standby_until = None;
                self.kick(at);
            }
        }
        None
    }

    fn drive(&self) -> Drives {
        Drives::OFF
    }

    /// CSB low is SPI mode. I2C does not answer while the host holds the engine there, and the
    /// part names the mode.
    fn input(&mut self, cx: &mut Cx, at: Time, levels: orbe_model::Levels) {
        if !levels.high(WATCH_CSB) {
            cx.broke("spi", at);
        }
    }
}

impl I2cTarget for Bmp280 {
    fn start(&mut self, _cx: &mut Cx, _at: Time, dir: Dir) {
        if dir == Dir::Write {
            self.set_pointer = true;
        }
    }

    fn write(&mut self, cx: &mut Cx, at: Time, byte: u8) -> Ack {
        if self.set_pointer {
            self.pointer = byte;
            self.set_pointer = false;
            return Ack::Yes;
        }
        let reg = self.pointer;
        self.pointer = self.pointer.wrapping_add(1);
        match usize::from(reg) {
            REG_CTRL => self.write_ctrl(at, byte),
            REG_CONFIG => self.write_config(cx, at, byte),
            REG_RESET if byte == SOFT_RESET => *self = Bmp280::power_on(),
            _ => {}
        }
        Ack::Yes
    }

    fn read(&mut self, _cx: &mut Cx, _at: Time) -> u8 {
        let byte = self.mem[usize::from(self.pointer)];
        self.pointer = self.pointer.wrapping_add(1);
        byte
    }
}

/// Temperature in 0.01 °C and pressure in Pa, clamped to the operating range.
fn world_sample(cx: &Cx) -> (i32, i64) {
    let temp = cx.world(Quantity::Temperature).clamp(TEMP_MIN, TEMP_MAX);
    let pressure = cx.world(Quantity::Pressure).clamp(PRESS_MIN, PRESS_MAX);
    ((temp * 100.0).round() as i32, pressure.round() as i64)
}

fn after(at: Time, us: u64) -> Option<Time> {
    at.as_ps()
        .checked_add(us.checked_mul(US)?)
        .map(Time::from_ps)
}

/// Oversampling count for an `osrs` code. `000` skips. `101` and above are ×16 (Tables 21 and 22).
pub(crate) fn osr_count(code: u8) -> u32 {
    match code & 0b111 {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        _ => 16,
    }
}

/// Typical measurement time in microseconds. Each row of Table 13 is
/// `1500 + 2000 * osr_t + 2000 * osr_p`. Both skipped is no conversion.
pub(crate) fn measure_us(osr_t: u32, osr_p: u32) -> u64 {
    if osr_t == 0 && osr_p == 0 {
        return 0;
    }
    1500 + 2000 * u64::from(osr_t) + 2000 * u64::from(osr_p)
}

/// `t_sb` standby in microseconds, Table 11.
fn standby_us(code: u8) -> u64 {
    match code & 0b111 {
        0 => 500,
        1 => 62_500,
        2 => 125_000,
        3 => 250_000,
        4 => 500_000,
        5 => 1_000_000,
        6 => 2_000_000,
        _ => 4_000_000,
    }
}

/// IIR coefficient, Table 6. Off is a pass-through. Codes past 16 stay at 16.
fn filter_coeff(config: u8) -> i32 {
    match (config >> 2) & 0b111 {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        _ => 16,
    }
}

/// §3.3.3. A cleared memory, and a coefficient of 1, pass `adc` through and that value becomes
/// the memory. A write that changes `filter` clears it before the next sample.
fn iir(memory: &mut Option<i32>, coeff: i32, adc: i32) -> i32 {
    let out = match (*memory, coeff) {
        (Some(old), coeff) if coeff > 1 => (old * (coeff - 1) + adc) / coeff,
        _ => adc,
    };
    *memory = Some(out);
    out
}

/// §3.11.3 `bmp280_compensate_T_int32`. Returns hundredths of a degree and `t_fine`.
pub(crate) fn compensate_temperature(adc_t: i32) -> (i32, i32) {
    let var1 = (((adc_t >> 3) - (DIG_T1 << 1)) * DIG_T2) >> 11;
    let var2 = (((((adc_t >> 4) - DIG_T1) * ((adc_t >> 4) - DIG_T1)) >> 12) * DIG_T3) >> 14;
    let t_fine = var1 + var2;
    let temp = (t_fine * 5 + 128) >> 8;
    (temp, t_fine)
}

/// §3.11.3 `bmp280_compensate_P_int64`. Pressure in Q24.8 pascals.
pub(crate) fn compensate_pressure(t_fine: i32, adc_p: i32) -> i64 {
    let var1 = i64::from(t_fine) - 128_000;
    let mut var2 = var1 * var1 * DIG_P6;
    var2 += (var1 * DIG_P5) << 17;
    var2 += DIG_P4 << 35;
    let mut var1 = ((var1 * var1 * DIG_P3) >> 8) + ((var1 * DIG_P2) << 12);
    var1 = (((1_i64 << 47) + var1) * DIG_P1) >> 33;
    if var1 == 0 {
        return 0;
    }
    let mut p = 1_048_576 - i64::from(adc_p);
    p = (((p << 31) - var2) * 3125) / var1;
    let var1 = (DIG_P9 * (p >> 13) * (p >> 13)) >> 25;
    let var2 = (DIG_P8 * p) >> 19;
    ((p + var1 + var2) >> 8) + (DIG_P7 << 4)
}

/// The raw temperature whose compensated hundredths are `hundredths`, and its `t_fine`.
pub(crate) fn adc_for_temperature(hundredths: i32) -> (i32, i32) {
    let mut lo = 0i32;
    let mut hi = 0xF_FFFF;
    let mut best = (i32::MAX, 0i32, 0i32);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let (got, t_fine) = compensate_temperature(mid);
        let err = (got - hundredths).abs();
        if err < best.0 || (err == best.0 && mid < best.1) {
            best = (err, mid, t_fine);
        }
        if got < hundredths {
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    (best.1, best.2)
}

/// The raw pressure whose compensated Q24.8 value is nearest `pascals` * 256.
pub(crate) fn adc_for_pressure(t_fine: i32, pascals: i64) -> i32 {
    let target = pascals.saturating_mul(256);
    let mut lo = 0i32;
    let mut hi = 0xF_FFFF;
    let increasing = compensate_pressure(t_fine, hi) > compensate_pressure(t_fine, lo);
    let mut best = (i64::MAX, 0i32);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let got = compensate_pressure(t_fine, mid);
        let err = (got - target).abs();
        if err < best.0 {
            best = (err, mid);
        }
        if got == target {
            return mid;
        }
        if (got < target) == increasing {
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    best.1
}
