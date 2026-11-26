//! The SAR ADCs' RTC controllers in SENS, which run one-shot conversions of ADC1 and ADC2 as
//! IDF's `adc_oneshot` driver starts them, with Wi-Fi off.
//!
//! IDF v5.5.5's `adc_oneshot_read` on the ESP32-S3 goes through `hal/adc_oneshot_hal.c` and
//! `hal/esp32s3/include/hal/adc_ll.h`: it sets the reader's clock divider to 1, the output
//! uninverted, the channel's attenuation, the channel as the only bit of `SARn_EN_PAD`, and the
//! RTC controller with `MEASn_START_FORCE` and `SARn_EN_PAD_FORCE`. It writes `DREF` 4 and the
//! unit's initial code through the analog I2C, then clears and sets `MEASn_START_SAR` and polls
//! `MEASn_DONE_SAR` before it reads `MEASn_DATA_SAR`. ADC2 also gets the arbiter's default,
//! `ADC_ARBITER_CONFIG_DEFAULT`: no forced grant. Channel `n` of ADC1 is GPIO `n + 1` and of
//! ADC2 GPIO `n + 11`, `soc/esp32s3/include/soc/adc_channel.h`.
//!
//! A rising `MEASn_START_SAR` samples the pad at its tick, through the host, and the code is the
//! one [`Curve`] gives that voltage at the channel's attenuation. `MEASn_DONE_SAR` rises and the
//! code shows in `MEASn_DATA_SAR` 25 cycles of the SAR clock later: RTC_FAST_CLK over
//! `SARn_CLK_DIV`. The TRM (v1.8, 39.3.7.1) states 25 cycles a sample for the digital
//! controller's clock and none for the RTC controllers', so the model takes the same. RTC_FAST_CLK
//! is XTAL_D2, 20 MHz, or RC_FAST over `RTC_CNTL_CK8M_DIV_SEL + 1`, 17.5 MHz undivided, as
//! `RTC_CNTL_FAST_CLK_RTC_SEL` picks (TRM 7.2.4.4 and Register 10.25,
//! `SOC_CLK_RC_FAST_FREQ_APPROX`). The TRM (39.3.2) says the digital controller's divider must be
//! at least 2 and gives no bound for the RTC controllers'; IDF sets 1, and the model runs it.
//!
//! Every setting the firmware can make here that changes what a conversion does and that the
//! model does not run ends the run, at the store that makes it or at the start it would change.

use super::analog::Analog;
use super::clock::Tick;
use super::curve::{self, Curve};
use super::efuse::EfuseSet;

/// SENS's offset in the RTC block.
pub(crate) const SENS: u32 = 0x800;
/// `APB_SARADC_APB_ADC_ARB_CTRL_REG`, in APB_SARADC.
pub(crate) const ARB_CTRL: u32 = 0x38;
/// `RTC_CNTL_RTC_CLK_CONF_REG`, in RTC_CNTL.
pub(crate) const CLK_CONF: u32 = 0x74;
/// `RTC_CNTL_INT_ENA_RTC_REG`'s `SARADC1_INT_ENA`, `TSENS_INT_ENA` and `SARADC2_INT_ENA`.
pub(crate) const INT_ENA: u32 = 0x40;
pub(crate) const INT_BITS: u32 = 1 << 11 | 1 << 12 | 1 << 14;

/// SENS's registers the model keeps, as offsets in SENS, with their reset values from
/// `soc/esp32s3/register/soc/sens_reg.h`.
const RESET: [(u32, u32); 17] = [
    (READER1, 0x2004_0002),
    (MEAS1_CTRL1, 0),
    (MEAS1_CTRL2, 0),
    (MEAS1_MUX, 0),
    (ATTEN1, 0xffff_ffff),
    (AMP_CTRL1, 0x000a_000a),
    (AMP_CTRL2, 0x000a_0000),
    (AMP_CTRL3, 0x0073_38f3),
    (READER2, 0x4005_0002),
    (MEAS2_CTRL1, 0x0702_0200),
    (MEAS2_CTRL2, 0),
    (MEAS2_MUX, 0),
    (ATTEN2, 0xffff_ffff),
    (POWER, 0),
    (SLAVE_ADDR1, 0),
    (CLK_GATE, 0),
    (RESET_CONF, 0),
];
const READER1: u32 = 0x00;
const MEAS1_CTRL1: u32 = 0x08;
const MEAS1_CTRL2: u32 = 0x0c;
const MEAS1_MUX: u32 = 0x10;
const ATTEN1: u32 = 0x14;
const AMP_CTRL1: u32 = 0x18;
const AMP_CTRL2: u32 = 0x1c;
const AMP_CTRL3: u32 = 0x20;
const READER2: u32 = 0x24;
const MEAS2_CTRL1: u32 = 0x2c;
const MEAS2_CTRL2: u32 = 0x30;
const MEAS2_MUX: u32 = 0x34;
const ATTEN2: u32 = 0x38;
/// `SENS_SAR_POWER_XPD_SAR_REG`.
const POWER: u32 = 0x3c;
const SLAVE_ADDR1: u32 = 0x40;
/// `SENS_SAR_PERI_CLK_GATE_CONF_REG`.
pub(crate) const CLK_GATE: u32 = 0x104;
/// `SENS_SAR_PERI_RESET_CONF_REG`.
pub(crate) const RESET_CONF: u32 = 0x108;

/// `SARn_DATA_INV`, `SARn_SAMPLE_NUM` and `SARn_CLK_GATED` of each reader, and
/// `SAR2_WAIT_ARB_CYCLE`.
const DATA_INV: [u32; 2] = [1 << 28, 1 << 29];
const SAMPLE_NUM: u32 = 0xff << 19;
const CLK_GATED: u32 = 1 << 18;
const WAIT_ARB_CYCLE: u32 = 0b11 << 16;
const CLK_DIV: u32 = 0xff;
/// `MEASn_CTRL2`'s fields.
const EN_PAD_FORCE: u32 = 1 << 31;
const EN_PAD_SHIFT: u32 = 19;
const EN_PAD: u32 = 0xfff << EN_PAD_SHIFT;
const START_FORCE: u32 = 1 << 18;
const START_SAR: u32 = 1 << 17;
const DONE: u32 = 1 << 16;
const DATA: u32 = 0xffff;
/// `SAR1_DIG_FORCE` and `SAR2_RTC_FORCE`.
const MUX_FORCE: u32 = 1 << 31;
/// `SARCLK_EN` and `FORCE_XPD_SAR`, which IDF's `sar_ctrl_ll_set_power_mode` sets to 3 for a
/// one-shot read, `hal/esp32s3/include/hal/sar_ctrl_ll.h`.
const SARCLK_EN: u32 = 1 << 31;
const FORCE_XPD_SAR: u32 = 0b11 << 29;
const XPD_ON: u32 = 0b11 << 29;
/// `SARADC_CLK_EN` and `SARADC_RESET`.
pub(crate) const SARADC_BIT: u32 = 1 << 30;
/// `TSENS_CLK_EN` and `TSENS_RESET`.
pub(crate) const TSENS_BIT: u32 = 1 << 29;
/// `SARADC_MEAS_STATUS`, which reads 0, and the I2C slave addresses, which have no effect here.
pub(crate) const SLAVE_ADDRS: u32 = 0x003f_ffff;
/// `ADC_ARB_GRANT_FORCE` and `ADC_ARB_RTC_FORCE`, `soc/esp32s3/register/soc/apb_saradc_reg.h`.
const GRANT_FORCE: u32 = 1 << 5;
const RTC_FORCE: u32 = 1 << 3;
/// `APB_SARADC_APB_ADC_ARB_CTRL_REG`'s reset value.
const ARB_RESET: u32 = 0x900;

/// `RTC_CNTL_FAST_CLK_RTC_SEL` and `RTC_CNTL_CK8M_DIV_SEL`. The first resets to 0, XTAL_D2.
const FAST_RC: u32 = 1 << 29;
const CK8M_DIV_SEL_SHIFT: u32 = 12;
const RC_FAST_HZ: u64 = 17_500_000;
const XTAL_D2_HZ: u64 = 20_000_000;
const TICK_HZ: u64 = 480_000_000;
/// SAR clock cycles a conversion takes.
const CYCLES: u64 = 25;

/// The SAR ADC block in the analog I2C, its host 1, and its registers,
/// `soc/esp32s3/include/soc/regi2c_saradc.h`.
const I2C_SAR_ADC: u8 = 0x69;
/// The low and high byte of each unit's initial code, the high byte's bits 3 to 0.
const INITIAL_CODE: [(u8, u8); 2] = [(0x0, 0x1), (0x3, 0x4)];
/// Each unit's `DREF`, bits 6 to 4, which IDF's `adc_ll_calibration_init` sets to 4.
const DREF: [u8; 2] = [0x2, 0x5];
/// `ADC_SAR1_SAMPLE_CYCLE`, bits 2 to 0 of register 2.
const SAMPLE_CYCLE: u8 = 0x2;
/// Register 7: `ADC_SARn_ENCAL_GND` at bits 5 and 7, and `DTEST_RTC`, `ENT_TSENS`, `ENT_RTC`
/// and `ENCAL_REF` at bits 0 to 4.
const TEST: u8 = 0x7;
const ENCAL_GND: [u8; 2] = [1 << 5, 1 << 7];
const TEST_MUX: u8 = 0x1f;

/// What a conversion needs from the rest of the chip.
pub(crate) struct Context<'a> {
    pub(crate) efuse: &'a EfuseSet,
    pub(crate) analog: &'a Analog,
    /// `RTC_CNTL_RTC_CLK_CONF_REG`, if the model keeps it.
    pub(crate) clk_conf: Option<u32>,
    pub(crate) netted: &'a [u8],
}

/// A conversion that started.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Conversion {
    pad: u8,
    curve: Curve,
    end: Tick,
    /// The code, once the host gave the pad's voltage.
    code: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Unit {
    /// `MEASn_DATA_SAR` before the last conversion ended.
    data: u16,
    last: Option<Conversion>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sar {
    words: [u32; RESET.len()],
    arb_ctrl: u32,
    units: [Unit; 2],
}

impl Sar {
    pub(crate) fn new() -> Sar {
        Sar {
            words: RESET.map(|(_, value)| value),
            arb_ctrl: ARB_RESET,
            units: [Unit::default(); 2],
        }
    }

    fn index(offset: u32) -> Option<usize> {
        RESET.iter().position(|&(at, _)| at == offset)
    }

    fn word(&self, offset: u32) -> u32 {
        self.words[Self::index(offset).expect("a kept register")]
    }

    /// The unit whose conversion `MEASn_CTRL2` at `offset` starts.
    fn unit_of(offset: u32) -> Option<usize> {
        match offset {
            MEAS1_CTRL2 => Some(0),
            MEAS2_CTRL2 => Some(1),
            _ => None,
        }
    }

    /// Whether `unit` converts at `now`.
    fn converting(&self, unit: usize, now: Tick) -> bool {
        self.units[unit].last.is_some_and(|last| now < last.end)
    }

    /// SENS's register at `offset` at `now`, `None` for one the model does not keep.
    pub(crate) fn read(&self, now: Tick, offset: u32) -> Option<u32> {
        let word = self.words[Self::index(offset)?];
        let Some(unit) = Self::unit_of(offset) else {
            return Some(word);
        };
        let state = &self.units[unit];
        let (done, data) = match state.last {
            Some(last) if now >= last.end => (
                DONE,
                last.code
                    .expect("the host gave the sample before the conversion ended"),
            ),
            _ => (0, state.data),
        };
        Some(word & !(DONE | DATA) | done | u32::from(data))
    }

    pub(crate) fn arb_ctrl(&self) -> u32 {
        self.arb_ctrl
    }

    pub(crate) fn write_arb_ctrl(&mut self, value: u32) {
        self.arb_ctrl = value;
    }

    pub(crate) fn tsens_clocked(&self) -> bool {
        self.word(CLK_GATE) & TSENS_BIT != 0
    }

    pub(crate) fn tsens_reset(&self) -> bool {
        self.word(RESET_CONF) & TSENS_BIT != 0
    }

    /// Writes SENS's register at `offset` at `now`. `Ok(Some(pad))` when the write starts a
    /// conversion that samples GPIO `pad` now, `Ok(None)` when it starts none, and `Err` with
    /// the setting the model does not run, writing nothing. `Err(None)` for a register the model
    /// does not keep.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        offset: u32,
        value: u32,
        context: &Context,
    ) -> Result<Option<u8>, Option<&'static str>> {
        let index = Self::index(offset).ok_or(None)?;
        let before = self.words[index];
        let value = match offset {
            MEAS1_CTRL2 | MEAS2_CTRL2 => value & !(DONE | DATA),
            MEAS2_CTRL1 => value & !0b111,
            SLAVE_ADDR1 => value & SLAVE_ADDRS,
            _ => value,
        };
        if let Some(setting) = self.refused(offset, before, value, now) {
            return Err(Some(setting));
        }
        let started = match Self::unit_of(offset) {
            Some(unit) if value & START_SAR != 0 && before & START_SAR == 0 => {
                let pad = self.start(unit, value, now, context).map_err(Some)?;
                Some(pad)
            }
            _ => None,
        };
        self.words[index] = value;
        Ok(started)
    }

    /// The setting a write of `value` over `before` at `offset` makes that the model does not
    /// run, if any.
    fn refused(&self, offset: u32, before: u32, value: u32, now: Tick) -> Option<&'static str> {
        let reader = |unit: usize| {
            if value & DATA_INV[unit] != 0 {
                Some("data_inv")
            } else if value & SAMPLE_NUM != 0 {
                Some("sample_num")
            } else if value & CLK_GATED == 0 {
                Some("clk_gated")
            } else if unit == 1 && value & WAIT_ARB_CYCLE != 1 << 16 {
                Some("wait_arb_cycle")
            } else {
                None
            }
        };
        let reset = Self::index(offset).map(|index| RESET[index].1);
        let setting = match offset {
            READER1 => reader(0),
            READER2 => reader(1),
            MEAS1_CTRL1 | AMP_CTRL1 | AMP_CTRL2 | AMP_CTRL3 => {
                (Some(value) != reset).then_some("amplifier")
            }
            MEAS2_CTRL1 => (Some(value) != reset).then_some("pwdet"),
            MEAS1_MUX => (value & MUX_FORCE != 0).then_some("dig_controller"),
            MEAS2_MUX => (value & MUX_FORCE != 0).then_some("rtc_force"),
            POWER => (value & SARCLK_EN != 0).then_some("sarclk_en"),
            _ => None,
        };
        if setting.is_some() {
            return setting;
        }
        // A conversion keeps its unit's settings, the power and the clock until it ends.
        let unit_registers = [
            [READER1, MEAS1_CTRL2, MEAS1_MUX, ATTEN1],
            [READER2, MEAS2_CTRL2, MEAS2_MUX, ATTEN2],
        ];
        let shared = match offset {
            POWER => value & FORCE_XPD_SAR != before & FORCE_XPD_SAR,
            CLK_GATE | RESET_CONF => value & SARADC_BIT != before & SARADC_BIT,
            _ => false,
        };
        let busy = (0..2).any(|unit| {
            self.converting(unit, now)
                && (shared || unit_registers[unit].contains(&offset) && value != before)
        });
        busy.then_some("change_while_converting")
    }

    /// Starts a conversion of `unit` with `MEASn_CTRL2` at `ctrl2`, or names the setting that
    /// the model does not run.
    fn start(
        &mut self,
        unit: usize,
        ctrl2: u32,
        now: Tick,
        context: &Context,
    ) -> Result<u8, &'static str> {
        // A start while the unit converts changes its `MEASn_CTRL2`, which `refused` ends first.
        debug_assert!(!self.converting(unit, now));
        if ctrl2 & START_FORCE == 0 || ctrl2 & EN_PAD_FORCE == 0 {
            return Err("ulp_controller");
        }
        let pads = (ctrl2 & EN_PAD) >> EN_PAD_SHIFT;
        if pads.count_ones() != 1 || pads.trailing_zeros() > 9 {
            return Err("channels");
        }
        let channel = pads.trailing_zeros();
        let pad = (channel + 1 + 10 * unit as u32) as u8;
        if !context.netted.contains(&pad) {
            return Err("pad_no_net");
        }
        if self.word(POWER) & FORCE_XPD_SAR != XPD_ON {
            return Err("power");
        }
        if self.word(CLK_GATE) & SARADC_BIT == 0 {
            return Err("clock_off");
        }
        if self.word(RESET_CONF) & SARADC_BIT != 0 {
            return Err("held_in_reset");
        }
        let reader = [READER1, READER2][unit];
        let divider = u64::from(self.word(reader) & CLK_DIV);
        if divider == 0 {
            return Err("clk_div");
        }
        if unit == 1 && self.arb_ctrl & (GRANT_FORCE | RTC_FORCE) == GRANT_FORCE {
            return Err("arbiter");
        }
        let atten = (self.word([ATTEN1, ATTEN2][unit]) >> (2 * channel) & 0b11) as usize;
        let curve = Curve::new(context.efuse, unit, atten).ok_or("calibration")?;
        let byte = |reg| context.analog.byte(I2C_SAR_ADC, reg);
        let (low, high) = INITIAL_CODE[unit];
        let code = u32::from(byte(high) & 0xf) << 8 | u32::from(byte(low));
        if Some(code) != curve::initial_code(context.efuse, unit, atten) {
            return Err("initial_code");
        }
        if byte(DREF[unit]) >> 4 & 0b111 != 4 {
            return Err("dref");
        }
        if unit == 0 && byte(SAMPLE_CYCLE) & 0b111 != 0 {
            return Err("sample_cycle");
        }
        let test = byte(TEST);
        if test & ENCAL_GND[unit] != 0 {
            return Err("internal_ground");
        }
        if test & TEST_MUX != 0 {
            return Err("test_mux");
        }
        let clk_conf = context.clk_conf.unwrap_or(0);
        let (hz, divided) = if clk_conf & FAST_RC != 0 {
            (
                RC_FAST_HZ,
                u64::from(clk_conf >> CK8M_DIV_SEL_SHIFT & 0b111) + 1,
            )
        } else {
            (XTAL_D2_HZ, 1)
        };
        let ticks = (CYCLES * divider * divided * TICK_HZ).div_ceil(hz);
        let state = &mut self.units[unit];
        if let Some(last) = state.last {
            state.data = last.code.expect("the host gave the last sample");
        }
        state.last = Some(Conversion {
            pad,
            curve,
            end: Tick(now.0 + ticks),
            code: None,
        });
        Ok(pad)
    }

    /// Gives each conversion waiting for its sample the code at its pad's `volts`.
    pub(crate) fn sample(&mut self, volts: impl Fn(u8) -> f64) {
        for unit in &mut self.units {
            if let Some(last) = unit.last.as_mut().filter(|last| last.code.is_none()) {
                last.code = Some(last.curve.code(volts(last.pad) * 1000.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IDF's analog I2C writes for ADC1 at `atten` on the synthetic set.
    fn analog(unit: usize, atten: usize) -> Analog {
        let set = EfuseSet::default();
        let code = curve::initial_code(&set, unit, atten).expect("calibrated");
        let mut analog = Analog::default();
        let (low, high) = INITIAL_CODE[unit];
        for (reg, byte) in [
            (DREF[unit], 4 << 4),
            (high, (code >> 8) as u8),
            (low, code as u8),
        ] {
            let word = u32::from(I2C_SAR_ADC) | u32::from(reg) << 8 | u32::from(byte) << 16;
            analog.write(4, word | 1 << 24);
        }
        analog
    }

    /// The SENS writes of IDF's one-shot read of ADC1 channel 5 at `atten`, up to the start.
    fn setup(sar: &mut Sar, atten: u32, context: &Context) {
        let at = Tick(0);
        let writes = [
            (CLK_GATE, SARADC_BIT),
            (POWER, XPD_ON),
            (READER1, 0x2004_0001),
            (ATTEN1, !(0b11 << 10) | atten << 10),
            (MEAS1_CTRL2, 1 << (EN_PAD_SHIFT + 5)),
            (MEAS1_MUX, 0),
            (
                MEAS1_CTRL2,
                EN_PAD_FORCE | 1 << (EN_PAD_SHIFT + 5) | START_FORCE,
            ),
        ];
        for (offset, value) in writes {
            assert_eq!(
                sar.write(at, offset, value, context),
                Ok(None),
                "{offset:#x}"
            );
        }
    }

    const CTRL2: u32 = EN_PAD_FORCE | 1 << (EN_PAD_SHIFT + 5) | START_FORCE;

    #[test]
    fn a_conversion_samples_its_pad_and_shows_its_code_25_cycles_later() {
        let set = EfuseSet::default();
        let analog = analog(0, 3);
        let context = Context {
            efuse: &set,
            analog: &analog,
            clk_conf: Some(0x30c8_069c),
            netted: &[6],
        };
        let mut sar = Sar::new();
        setup(&mut sar, 3, &context);
        let start = Tick(1000);
        assert_eq!(
            sar.write(start, MEAS1_CTRL2, CTRL2 | START_SAR, &context),
            Ok(Some(6))
        );
        sar.sample(|pad| if pad == 6 { 1.65 } else { 0.0 });
        // 25 cycles of 17.5 MHz is 685.7 ticks of 480 MHz.
        let end = Tick(start.0 + 686);
        let ctrl2 = |at| sar.read(at, MEAS1_CTRL2).expect("kept");
        assert_eq!(ctrl2(Tick(end.0 - 1)) & (DONE | DATA), 0);
        let code = Curve::new(&set, 0, 3).expect("calibrated").code(1650.0);
        assert_eq!(ctrl2(end) & (DONE | DATA), DONE | u32::from(code));
        assert_eq!(sar.read(end, SLAVE_ADDR1), Some(0), "meas_status reads 0");
    }

    /// The divider and RTC_FAST_CLK's source and divider set the conversion's length.
    #[test]
    fn the_sar_clock_sets_the_length() {
        let set = EfuseSet::default();
        let analog = analog(0, 3);
        for (clk_conf, divider, ticks) in [
            (Some(0x30c8_069c), 1, 686),
            (Some(0x30c8_069c), 2, 1372),
            (Some(0x30c8_069c | 1 << CK8M_DIV_SEL_SHIFT), 1, 1372),
            (None, 1, 600),
        ] {
            let context = Context {
                efuse: &set,
                analog: &analog,
                clk_conf,
                netted: &[6],
            };
            let mut sar = Sar::new();
            setup(&mut sar, 3, &context);
            sar.write(Tick(0), READER1, 0x2004_0000 | divider, &context)
                .expect("a divider");
            let start = Tick(10);
            sar.write(start, MEAS1_CTRL2, CTRL2 | START_SAR, &context)
                .expect("a start");
            sar.sample(|_| 0.5);
            let done = |at: u64| sar.read(Tick(at), MEAS1_CTRL2).expect("kept") & DONE != 0;
            assert!(
                !done(start.0 + ticks - 1) && done(start.0 + ticks),
                "{ticks}"
            );
        }
    }

    /// Each start-time setting the model does not run ends the run, and the write starts nothing.
    #[test]
    fn a_start_with_a_setting_the_model_does_not_run_is_refused() {
        let set = EfuseSet::default();
        let good = analog(0, 3);
        let mut ground = good.clone();
        ground.write(4, 0x0120_0769);
        let mut tsens = good.clone();
        tsens.write(4, 0x0104_0769);
        let mut dref = good.clone();
        dref.write(4, 0x0130_0269);
        let mut cycle = good.clone();
        cycle.write(4, 0x0141_0269);
        let wrong_code = analog(0, 0);
        let uncalibrated = set.with("BLK_VERSION_MAJOR", 0);
        let context = |analog, efuse, netted| Context {
            efuse,
            analog,
            clk_conf: None,
            netted,
        };
        type Change = fn(&mut Sar);
        let none: Change = |_| {};
        type Case<'a> = (&'a str, Change, &'a Analog, &'a EfuseSet, &'a [u8], u32);
        let cases: [Case; 13] = [
            (
                "ulp_controller",
                none,
                &good,
                &set,
                &[6],
                CTRL2 & !START_FORCE,
            ),
            (
                "ulp_controller",
                none,
                &good,
                &set,
                &[6],
                CTRL2 & !EN_PAD_FORCE,
            ),
            (
                "channels",
                none,
                &good,
                &set,
                &[6],
                CTRL2 | 1 << (EN_PAD_SHIFT + 6),
            ),
            (
                "channels",
                none,
                &good,
                &set,
                &[6],
                START_FORCE | EN_PAD_FORCE | 1 << (EN_PAD_SHIFT + 10),
            ),
            ("pad_no_net", none, &good, &set, &[7], CTRL2),
            (
                "power",
                |sar| sar.words[Sar::index(POWER).unwrap()] = 0,
                &good,
                &set,
                &[6],
                CTRL2,
            ),
            (
                "clock_off",
                |sar| sar.words[Sar::index(CLK_GATE).unwrap()] = 0,
                &good,
                &set,
                &[6],
                CTRL2,
            ),
            (
                "held_in_reset",
                |sar| sar.words[Sar::index(RESET_CONF).unwrap()] = SARADC_BIT,
                &good,
                &set,
                &[6],
                CTRL2,
            ),
            ("calibration", none, &good, &uncalibrated, &[6], CTRL2),
            ("initial_code", none, &wrong_code, &set, &[6], CTRL2),
            ("internal_ground", none, &ground, &set, &[6], CTRL2),
            ("test_mux", none, &tsens, &set, &[6], CTRL2),
            ("dref", none, &dref, &set, &[6], CTRL2),
        ];
        for (setting, change, analog, efuse, netted, ctrl2) in cases {
            let context = context(analog, efuse, netted);
            let mut sar = Sar::new();
            setup(&mut sar, 3, &context);
            change(&mut sar);
            let before = sar.clone();
            assert_eq!(
                sar.write(Tick(5), MEAS1_CTRL2, ctrl2 | START_SAR, &context),
                Err(Some(setting))
            );
            assert_eq!(sar, before, "{setting} writes nothing");
        }
        let context = context(&cycle, &set, &[6]);
        let mut sar = Sar::new();
        setup(&mut sar, 3, &context);
        assert_eq!(
            sar.write(Tick(5), MEAS1_CTRL2, CTRL2 | START_SAR, &context),
            Err(Some("sample_cycle"))
        );
    }

    /// ADC2 starts on GPIO11 up when the arbiter grants the RTC controller, and not when a
    /// forced grant names another.
    #[test]
    fn adc2_runs_unless_the_arbiter_is_forced_away() {
        let set = EfuseSet::default();
        let analog = analog(1, 3);
        let context = Context {
            efuse: &set,
            analog: &analog,
            clk_conf: None,
            netted: &[13],
        };
        let ctrl2 = EN_PAD_FORCE | 1 << (EN_PAD_SHIFT + 2) | START_FORCE;
        let start = |arb: u32| {
            let mut sar = Sar::new();
            for (offset, value) in [
                (CLK_GATE, SARADC_BIT),
                (POWER, XPD_ON),
                (MEAS2_CTRL2, ctrl2),
            ] {
                sar.write(Tick(0), offset, value, &context).expect("setup");
            }
            sar.write_arb_ctrl(arb);
            sar.write(Tick(1), MEAS2_CTRL2, ctrl2 | START_SAR, &context)
        };
        assert_eq!(start(ARB_RESET), Ok(Some(13)));
        assert_eq!(
            start(0x900 | 1 << 12 | 1 << 4),
            Ok(Some(13)),
            "IDF's default"
        );
        assert_eq!(start(GRANT_FORCE | 1 << 4), Err(Some("arbiter")));
        assert_eq!(start(GRANT_FORCE | RTC_FORCE), Ok(Some(13)));
    }

    /// Each store-time setting the model does not run is refused before it is written.
    #[test]
    fn a_store_with_a_setting_the_model_does_not_run_is_refused() {
        let set = EfuseSet::default();
        let analog = Analog::default();
        let context = Context {
            efuse: &set,
            analog: &analog,
            clk_conf: None,
            netted: &[],
        };
        for (offset, value, setting) in [
            (READER1, 0x2004_0002 | 1 << 28, "data_inv"),
            (READER1, 0x2004_0002 | 1 << 19, "sample_num"),
            (READER1, 0x2000_0002, "clk_gated"),
            (READER2, 0x4005_0002 | 1 << 29, "data_inv"),
            (READER2, 0x4006_0002, "wait_arb_cycle"),
            (MEAS1_CTRL1, 1 << 24, "amplifier"),
            (AMP_CTRL3, 0, "amplifier"),
            (MEAS2_CTRL1, 0x0702_0208, "pwdet"),
            (MEAS1_MUX, MUX_FORCE, "dig_controller"),
            (MEAS2_MUX, MUX_FORCE, "rtc_force"),
            (POWER, SARCLK_EN, "sarclk_en"),
        ] {
            let mut sar = Sar::new();
            assert_eq!(
                sar.write(Tick(0), offset, value, &context),
                Err(Some(setting))
            );
            assert_eq!(sar, Sar::new(), "{setting} writes nothing");
        }
        let mut sar = Sar::new();
        assert_eq!(sar.write(Tick(0), 0x44, 1, &context), Err(None), "not kept");
        assert_eq!(
            sar.write(Tick(0), MEAS2_MUX, 4 << 28, &context),
            Ok(None),
            "PWDET_CCT"
        );
        assert_eq!(
            sar.write(Tick(0), READER1, 0x0004_0001, &context),
            Ok(None),
            "INT_EN"
        );
    }

    /// A conversion keeps its settings, the power and the clock until it ends, and a second
    /// start waits for it.
    #[test]
    fn a_change_while_converting_is_refused() {
        let set = EfuseSet::default();
        let analog = analog(0, 3);
        let context = Context {
            efuse: &set,
            analog: &analog,
            clk_conf: None,
            netted: &[6],
        };
        let mut sar = Sar::new();
        setup(&mut sar, 3, &context);
        sar.write(Tick(0), MEAS1_CTRL2, CTRL2 | START_SAR, &context)
            .expect("a start");
        sar.sample(|_| 1.0);
        for (offset, value) in [
            (ATTEN1, 0),
            (MEAS1_CTRL2, CTRL2),
            (POWER, 0),
            (CLK_GATE, 0),
            (RESET_CONF, SARADC_BIT),
        ] {
            assert_eq!(
                sar.write(Tick(10), offset, value, &context),
                Err(Some("change_while_converting")),
                "{offset:#x}"
            );
        }
        assert_eq!(
            sar.write(Tick(10), ATTEN2, 0, &context),
            Ok(None),
            "the other unit"
        );
        assert_eq!(
            sar.write(Tick(600), MEAS1_CTRL2, CTRL2, &context),
            Ok(None),
            "ended"
        );
        assert_eq!(
            sar.write(Tick(601), MEAS1_CTRL2, CTRL2 | START_SAR, &context),
            Ok(Some(6))
        );
        let ctrl2 = sar.read(Tick(602), MEAS1_CTRL2).expect("kept");
        let first = Curve::new(&set, 0, 3).expect("calibrated").code(1000.0);
        assert_eq!(
            ctrl2 & (DONE | DATA),
            u32::from(first),
            "the last code until this one ends"
        );
    }
}
