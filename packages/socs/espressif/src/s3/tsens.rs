//! The internal temperature sensor in SENS (`SENS_SAR_TSENS_CTRL_REG` and
//! `SENS_SAR_TSENS_CTRL2_REG`), which `temperatureRead()` and ESP-IDF's `temperature_sensor`
//! driver read.
//!
//! A rising `SENS_TSENS_DUMP_OUT` samples the scene's ambient temperature at that tick through
//! the host (`Quantity::Temperature`, defaulting to 25 degC when the scene omits it; chip
//! self-heating is not simulated). The raw 8-bit code `SENS_TSENS_OUT` is the inverse of the
//! ESP32-S3 TRM v1.8 Equation 39.2 transfer function,
//! `T(degC) = 0.4386 * out - 27.88 * offset - 20.52`, shifted by the chip's eFuse `TEMP_CALIB`
//! offset (sign-magnitude in 0.1 degC when `BLK_VERSION_MAJOR == 1`, 0 otherwise), rounded to
//! the nearest code and clamped to `0..=255`. The five DAC range settings (`I2C_SARADC_TSENS_DAC`
//! in analog I2C block `0x69` register `0x6` bits `[3:0]`) come from TRM Table 39-1 and IDF v5.5.5
//! `temperature_sensor_periph.c`.
//!
//! `SENS_TSENS_READY` reads 1 once the sample lands and clears when `SENS_TSENS_DUMP_OUT` is
//! cleared; `SENS_TSENS_OUT` keeps the last code across the clear, as IDF's
//! `temperature_sensor_ll_get_raw_value` reads it after clearing `SENS_TSENS_DUMP_OUT`.

use super::analog::Analog;
use super::efuse::EfuseSet;

/// Offsets in SENS (`0x6000_8800`), from IDF v5.5.5 `soc/esp32s3/register/soc/sens_reg.h`.
pub(crate) const CTRL: u32 = 0x50;
pub(crate) const CTRL2: u32 = 0x54;

/// `SENS_SAR_TSENS_CTRL_REG` reset: `TSENS_CLK_DIV = 6`, `TSENS_INT_EN = 1`.
const CTRL_RESET: u32 = (6 << 14) | (1 << 12);
/// `SENS_SAR_TSENS_CTRL2_REG` reset: `TSENS_CLK_INV = 1`, `TSENS_XPD_WAIT = 2`.
const CTRL2_RESET: u32 = (1 << 14) | 2;

const OUT_MASK: u32 = 0xff;
const READY: u32 = 1 << 8;
const IN_INV: u32 = 1 << 13;
const CLK_DIV_SHIFT: u32 = 14;
const CLK_DIV_MASK: u32 = 0xff;
const POWER_UP: u32 = 1 << 22;
const POWER_UP_FORCE: u32 = 1 << 23;
const DUMP_OUT: u32 = 1 << 24;
const CTRL_WRITABLE: u32 = 0x01ff_f000;

const XPD_FORCE: u32 = 0b11 << 12;
const CLK_INV: u32 = 1 << 14;
const CTRL2_WRITABLE: u32 = 0x7fff;

/// `RTC_CNTL_DIG_CLK8M_EN` (bit 10), `RTC_CNTL_DIG_CLK8M_D256_EN` (bit 9) and
/// `RTC_CNTL_ENB_CK8M_DIV` (bit 7) in `RTC_CNTL_CLK_CONF_REG` (`RTC_CNTL + 0x74`).
/// IDF's `periph_rtc_dig_clk8m_enable` toggles these bits when enabling and calibrating
/// `RC_FAST` for the temperature sensor.
pub(crate) const DIG_CLK8M_EN: u32 = 1 << 10;
pub(crate) const CLK_CONF_RUN: u32 = DIG_CLK8M_EN | (1 << 9) | (1 << 7);

/// Analog I2C SAR ADC block and `I2C_SARADC_TSENS_DAC` register (`regi2c_saradc.h`).
const I2C_SAR_ADC: u8 = 0x69;
const TSENS_DAC_REG: u8 = 0x6;
const TSENS_DAC_MASK: u8 = 0x0f;

/// The five hardware measurement ranges: `(reg_val, offset, range_min, range_max)`.
/// TRM v1.8 Table 39-1 and IDF v5.5.5 `temperature_sensor_attributes`.
const RANGES: [(u8, i32, f64, f64); 5] = [
    (5, -2, 50.0, 125.0),
    (7, -1, 20.0, 100.0),
    (15, 0, -10.0, 80.0),
    (11, 1, -30.0, 50.0),
    (10, 2, -40.0, 20.0),
];

/// TRM v1.8 Equation 39.2 and `hal/esp32s3/include/hal/temperature_sensor_ll.h`.
const ADC_FACTOR: f64 = 0.4386;
const DAC_FACTOR: f64 = 27.88;
const OFFSET_FACTOR: f64 = 20.52;
const MEASURE_MIN_C: f64 = -40.0;
const MEASURE_MAX_C: f64 = 125.0;

/// What a temperature conversion needs from the rest of the chip.
pub(crate) struct Context<'a> {
    pub(crate) efuse: &'a EfuseSet,
    pub(crate) analog: &'a Analog,
    /// `SENS_TSENS_CLK_EN` in `SENS_SAR_PERI_CLK_GATE_CONF_REG`.
    pub(crate) clocked: bool,
    /// `SENS_TSENS_RESET` in `SENS_SAR_PERI_RESET_CONF_REG`.
    pub(crate) reset: bool,
    /// `RTC_CNTL_CLK_CONF_REG`, if kept (flash boot).
    pub(crate) clk_conf: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    offset: i32,
    delta_t: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tsens {
    ctrl: u32,
    ctrl2: u32,
    out: u8,
    ready: bool,
    pending: Option<Pending>,
}

impl Tsens {
    pub(crate) fn new() -> Tsens {
        Tsens {
            ctrl: CTRL_RESET,
            ctrl2: CTRL2_RESET,
            out: 0,
            ready: false,
            pending: None,
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Tsens::new();
    }

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        match offset {
            CTRL => {
                debug_assert!(
                    self.pending.is_none(),
                    "the host gave the temperature sample before the read"
                );
                let ready = if self.ready { READY } else { 0 };
                Some((self.ctrl & !(READY | OUT_MASK)) | ready | u32::from(self.out))
            }
            CTRL2 => Some(self.ctrl2),
            _ => None,
        }
    }

    /// Writes `SENS_SAR_TSENS_CTRL_REG` or `SENS_SAR_TSENS_CTRL2_REG`. Returns `Ok(true)` when
    /// `SENS_TSENS_DUMP_OUT` rises and needs a temperature sample from the host.
    pub(crate) fn write(
        &mut self,
        offset: u32,
        value: u32,
        context: &Context,
    ) -> Result<bool, &'static str> {
        match offset {
            CTRL => {
                let value = value & CTRL_WRITABLE;
                if value & IN_INV != 0 {
                    return Err("tsens_in_inv");
                }
                let rising_dump = self.ctrl & DUMP_OUT == 0 && value & DUMP_OUT != 0;
                if rising_dump {
                    let pending = self.start(value, context)?;
                    self.ctrl = value;
                    self.ready = false;
                    self.pending = Some(pending);
                    return Ok(true);
                }
                self.ctrl = value;
                if value & DUMP_OUT == 0 {
                    self.ready = false;
                    self.pending = None;
                }
                Ok(false)
            }
            CTRL2 => {
                let value = value & CTRL2_WRITABLE;
                if value & CLK_INV == 0 {
                    return Err("tsens_clk_inv");
                }
                self.ctrl2 = value;
                Ok(false)
            }
            _ => unreachable!("only CTRL and CTRL2 route to Tsens"),
        }
    }

    fn start(&self, ctrl: u32, context: &Context) -> Result<Pending, &'static str> {
        if !context.clocked {
            return Err("tsens_clock_off");
        }
        if context.reset {
            return Err("tsens_held_in_reset");
        }
        if let Some(clk_conf) = context.clk_conf
            && clk_conf & DIG_CLK8M_EN == 0
        {
            return Err("tsens_clock_off");
        }
        if ctrl & POWER_UP_FORCE == 0 || ctrl & POWER_UP == 0 || self.ctrl2 & XPD_FORCE == 0 {
            return Err("tsens_power");
        }
        if (ctrl >> CLK_DIV_SHIFT) & CLK_DIV_MASK == 0 {
            return Err("tsens_clk_div");
        }
        let dac = context.analog.byte(I2C_SAR_ADC, TSENS_DAC_REG) & TSENS_DAC_MASK;
        let offset = RANGES
            .iter()
            .find(|&&(reg_val, ..)| reg_val == dac)
            .map(|&(_, offset, ..)| offset)
            .ok_or("tsens_range")?;
        let delta_t = efuse_delta_t(context.efuse);
        Ok(Pending { offset, delta_t })
    }

    /// Completes a pending temperature dump from the scene's ambient `temp_c` (degC).
    pub(crate) fn sample(&mut self, temp_c: f64) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let clamped = temp_c.clamp(MEASURE_MIN_C, MEASURE_MAX_C);
        self.out = raw_code(clamped + pending.delta_t, pending.offset);
        self.ready = true;
    }
}

/// eFuse `TEMP_CALIB` offset in degC, matching IDF v5.5.5 `esp_efuse_rtc_calib_get_tsens_val`
/// divided by 10.
fn efuse_delta_t(efuse: &EfuseSet) -> f64 {
    if efuse.field("BLK_VERSION_MAJOR") != 1 {
        return 0.0;
    }
    let raw = efuse.field("TEMP_CALIB") as u32;
    let mag = (raw & 0xff) as i32;
    let signed = if raw & 0x100 != 0 { -mag } else { mag };
    f64::from(signed) / 10.0
}

/// Raw 8-bit `SENS_TSENS_OUT` code for uncalibrated sensor temperature `temp_c` and DAC `offset`.
pub(crate) fn raw_code(temp_c: f64, offset: i32) -> u8 {
    let raw = (temp_c + OFFSET_FACTOR + DAC_FACTOR * f64::from(offset)) / ADC_FACTOR;
    raw.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analog_with_dac(reg_val: u8) -> Analog {
        let mut analog = Analog::default();
        let word = u32::from(I2C_SAR_ADC)
            | u32::from(TSENS_DAC_REG) << 8
            | u32::from(reg_val) << 16
            | 1 << 24;
        analog.write(4, word);
        analog
    }

    fn enabled_tsens(context: &Context) -> Tsens {
        let mut tsens = Tsens::new();
        assert_eq!(
            tsens.write(CTRL2, CTRL2_RESET | (1 << 12), context),
            Ok(false)
        );
        assert_eq!(
            tsens.write(CTRL, CTRL_RESET | POWER_UP_FORCE | POWER_UP, context),
            Ok(false)
        );
        tsens
    }

    #[test]
    fn every_range_converts_temperatures_within_trm_and_idf_bounds() {
        let efuse = EfuseSet::default();
        for &(reg_val, offset, min_c, max_c) in &RANGES {
            let analog = analog_with_dac(reg_val);
            let context = Context {
                efuse: &efuse,
                analog: &analog,
                clocked: true,
                reset: false,
                clk_conf: Some(DIG_CLK8M_EN),
            };
            let mut tsens = enabled_tsens(&context);
            for temp in [min_c, (min_c + max_c) / 2.0, max_c] {
                assert_eq!(
                    tsens.write(
                        CTRL,
                        CTRL_RESET | POWER_UP_FORCE | POWER_UP | DUMP_OUT,
                        &context
                    ),
                    Ok(true)
                );
                tsens.sample(temp);
                let word = tsens.read(CTRL).unwrap();
                assert_ne!(word & READY, 0);
                // IDF clears DUMP_OUT before reading OUT.
                assert_eq!(
                    tsens.write(CTRL, CTRL_RESET | POWER_UP_FORCE | POWER_UP, &context),
                    Ok(false)
                );
                let cleared = tsens.read(CTRL).unwrap();
                assert_eq!(cleared & READY, 0);
                let raw = (cleared & OUT_MASK) as u8;
                let back =
                    ADC_FACTOR * f64::from(raw) - DAC_FACTOR * f64::from(offset) - OFFSET_FACTOR;
                assert!(
                    (back - temp).abs() <= ADC_FACTOR / 2.0 + 1e-9,
                    "dac={reg_val} temp={temp} raw={raw} back={back}"
                );
                // IDF truncates `back` to i32 and requires it in [-40, 125].
                let idf_c = (back as i32) as f64;
                assert!(
                    (MEASURE_MIN_C..=MEASURE_MAX_C).contains(&idf_c),
                    "dac={reg_val} temp={temp} idf_c={idf_c}"
                );
            }
        }
    }

    #[test]
    fn efuse_temp_calib_shifts_the_raw_code_so_idf_recovers_ambient() {
        // Raw 25 => +2.5 degC calibration offset; 0x100 | 30 => -3.0 degC.
        for (field, delta_t) in [(25u128, 2.5), (0x100 | 30, -3.0)] {
            let efuse = EfuseSet::default().with("TEMP_CALIB", field);
            let analog = analog_with_dac(15); // offset 0
            let context = Context {
                efuse: &efuse,
                analog: &analog,
                clocked: true,
                reset: false,
                clk_conf: None,
            };
            let mut tsens = enabled_tsens(&context);
            assert_eq!(
                tsens.write(
                    CTRL,
                    CTRL_RESET | POWER_UP_FORCE | POWER_UP | DUMP_OUT,
                    &context
                ),
                Ok(true)
            );
            tsens.sample(25.0);
            let raw = (tsens.read(CTRL).unwrap() & OUT_MASK) as u8;
            let uncal = ADC_FACTOR * f64::from(raw) - OFFSET_FACTOR;
            let calibrated = uncal - delta_t;
            assert!(
                (calibrated - 25.0).abs() <= ADC_FACTOR / 2.0 + 1e-9,
                "field={field:#x} calibrated={calibrated}"
            );
        }
    }

    #[test]
    fn unpowered_unclocked_or_invalid_settings_are_refused() {
        let efuse = EfuseSet::default();
        let good_analog = analog_with_dac(15);
        let bad_analog = analog_with_dac(3);
        let dump = CTRL_RESET | POWER_UP_FORCE | POWER_UP | DUMP_OUT;

        // Clock off
        let ctx = Context {
            efuse: &efuse,
            analog: &good_analog,
            clocked: false,
            reset: false,
            clk_conf: None,
        };
        assert_eq!(
            enabled_tsens(&ctx).write(CTRL, dump, &ctx),
            Err("tsens_clock_off")
        );

        // DIG_CLK8M_EN clear
        let ctx = Context {
            efuse: &efuse,
            analog: &good_analog,
            clocked: true,
            reset: false,
            clk_conf: Some(0),
        };
        assert_eq!(
            enabled_tsens(&ctx).write(CTRL, dump, &ctx),
            Err("tsens_clock_off")
        );

        // Held in reset
        let ctx = Context {
            efuse: &efuse,
            analog: &good_analog,
            clocked: true,
            reset: true,
            clk_conf: None,
        };
        assert_eq!(
            enabled_tsens(&ctx).write(CTRL, dump, &ctx),
            Err("tsens_held_in_reset")
        );

        // Invalid DAC range
        let ctx = Context {
            efuse: &efuse,
            analog: &bad_analog,
            clocked: true,
            reset: false,
            clk_conf: None,
        };
        assert_eq!(
            enabled_tsens(&ctx).write(CTRL, dump, &ctx),
            Err("tsens_range")
        );

        // Powered down
        let ctx = Context {
            efuse: &efuse,
            analog: &good_analog,
            clocked: true,
            reset: false,
            clk_conf: None,
        };
        assert_eq!(
            Tsens::new().write(CTRL, CTRL_RESET | DUMP_OUT, &ctx),
            Err("tsens_power")
        );

        // Inverted input or clock
        assert_eq!(
            Tsens::new().write(CTRL, CTRL_RESET | IN_INV, &ctx),
            Err("tsens_in_inv")
        );
        assert_eq!(Tsens::new().write(CTRL2, 2, &ctx), Err("tsens_clk_inv"));

        // Zero clock divider
        assert_eq!(
            enabled_tsens(&ctx).write(CTRL, (1 << 12) | POWER_UP_FORCE | POWER_UP | DUMP_OUT, &ctx),
            Err("tsens_clk_div")
        );
    }
}
