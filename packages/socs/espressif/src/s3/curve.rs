//! The SAR ADCs' transfer function, from a chip's eFuse calibration.
//!
//! IDF reads a voltage from a raw code in two steps, IDF v5.5.5
//! `components/esp_adc/adc_cali_curve_fitting.c`. The first is linear: `v = raw * coeff_a / 65536`
//! with `coeff_a = 65536 * 850 / digi`, where `digi` is the code the chip read at 850 mV in the
//! factory. The second subtracts a polynomial in `v`, the error
//! `K0 + K1 v + K2 v^2` (and `+ K3 v^3 + K4 v^4` at 11 dB), each term signed, with the coefficients
//! of `components/esp_adc/esp32s3/curve_fitting_coefficients.c`. `digi` and the initial code the
//! SAR starts from come from the eFuse fields as `components/efuse/esp32s3/esp_efuse_rtc_calib.c`
//! adds them up, which IDF uses only when `BLK_VERSION_MAJOR` is 1.
//!
//! The chip's analog error is what that calibration corrects, so the model's code for a voltage
//! is the code the calibration maps back to it: the real-valued two steps, `G(x)`, inverted, the
//! code rounded to the nearest and clipped to 0 and 4095. [`Curve::millivolts`] is IDF's
//! integer arithmetic, wrap and truncation included, which the firmware runs on that code.

use super::efuse::EfuseSet;

/// `coeff_a_scaling`.
const SCALING: u64 = 65_536;
/// The voltage every `digi` was read at, `out_vol_mv`.
const CAL_MV: u32 = 850;
const E16: u64 = 10_000_000_000_000_000;
const E15: u64 = 1_000_000_000_000_000;
/// The highest code, 12 bits.
pub(crate) const MAX: u16 = 4095;

/// The error's terms by unit and attenuation, each a coefficient, its divisor and its sign.
/// `adc1_error_coef_atten`, `adc2_error_coef_atten`, `adc1_error_sign` and `adc2_error_sign`. A
/// 3-term row has `(0, 1, 0)` in its last two places, which IDF never reads.
const TERMS: [[[(u64, u64, i32); 5]; 4]; 2] = {
    const NONE: (u64, u64, i32) = (0, 1, 0);
    [
        [
            [
                (27_856_531_419_538_344, E16, -1),
                (50_871_540_569_528, E16, -1),
                (9_798_249_589, E15, 1),
                NONE,
                NONE,
            ],
            [
                (29_831_022_915_028_695, E16, -1),
                (49_393_185_868_806, E16, -1),
                (101_379_430_548, E16, 1),
                NONE,
                NONE,
            ],
            [
                (23_285_545_746_296_417, E16, -1),
                (147_640_181_047_414, E16, -1),
                (208_385_525_314, E16, 1),
                NONE,
                NONE,
            ],
            [
                (644_403_418_269_478, E15, -1),
                (644_334_888_647_536, E16, -1),
                (1_297_891_447_611, E16, 1),
                (70_769_718, E15, -1),
                (13_515, E15, 1),
            ],
        ],
        [
            [
                (25_668_651_654_328_927, E16, -1),
                (1_353_548_869_615, E16, 1),
                (36_615_265_189, E16, 1),
                NONE,
                NONE,
            ],
            [
                (23_690_184_690_298_404, E16, -1),
                (66_319_894_226_185, E16, -1),
                (118_964_995_959, E16, 1),
                NONE,
                NONE,
            ],
            [
                (9_452_499_397_020_617, E16, -1),
                (200_996_773_954_387, E16, -1),
                (259_011_467_956, E16, 1),
                NONE,
                NONE,
            ],
            [
                (12_247_719_764_336_924, E16, 1),
                (755_717_904_943_462, E16, -1),
                (1_478_791_187_119, E16, 1),
                (79_672_528, E15, -1),
                (15_038, E15, 1),
            ],
        ],
    ]
};

/// Whether IDF reads this set's ADC calibration, `esp_efuse_rtc_calib_get_ver` returning 1.
pub(crate) fn calibrated(set: &EfuseSet) -> bool {
    set.field("BLK_VERSION_MAJOR") == 1
}

/// The four raw `u32`s of `fields`, as `esp_efuse_read_field_blob` reads each.
fn raw(set: &EfuseSet, fields: [&str; 4]) -> [u32; 4] {
    fields.map(|field| set.field(field) as u32)
}

/// The initial code IDF writes before a conversion of `unit` (0 for ADC1) at `atten`, from
/// `esp_efuse_rtc_calib_get_init_code`, with its unsigned 32-bit sums. `None` when IDF does not
/// read the calibration, and then calibrates the code on the chip itself.
pub(crate) fn initial_code(set: &EfuseSet, unit: usize, atten: usize) -> Option<u32> {
    if !calibrated(set) {
        return None;
    }
    let d = if unit == 0 {
        raw(
            set,
            [
                "ADC1_INIT_CODE_ATTEN0",
                "ADC1_INIT_CODE_ATTEN1",
                "ADC1_INIT_CODE_ATTEN2",
                "ADC1_INIT_CODE_ATTEN3",
            ],
        )
    } else {
        raw(
            set,
            [
                "ADC2_INIT_CODE_ATTEN0",
                "ADC2_INIT_CODE_ATTEN1",
                "ADC2_INIT_CODE_ATTEN2",
                "ADC2_INIT_CODE_ATTEN3",
            ],
        )
    };
    let (base, steps) = if unit == 0 {
        (1850, [90, 0, 70])
    } else {
        (2020, [0, 0, 0])
    };
    let mut code = [0u32; 4];
    code[0] = d[0].wrapping_add(base);
    for k in 1..4 {
        code[k] = d[k].wrapping_add(code[k - 1]).wrapping_add(steps[k - 1]);
    }
    Some(code[atten])
}

/// The code `unit` read at 850 mV at `atten` in the factory, `esp_efuse_rtc_calib_get_cal_voltage`'s
/// `out_digi`, with its unsigned 32-bit sums.
fn digi(set: &EfuseSet, unit: usize, atten: usize) -> u32 {
    let adc1 = raw(
        set,
        [
            "ADC1_CAL_VOL_ATTEN0",
            "ADC1_CAL_VOL_ATTEN1",
            "ADC1_CAL_VOL_ATTEN2",
            "ADC1_CAL_VOL_ATTEN3",
        ],
    );
    let adc2 = raw(
        set,
        [
            "ADC2_CAL_VOL_ATTEN0",
            "ADC2_CAL_VOL_ATTEN1",
            "ADC2_CAL_VOL_ATTEN2",
            "ADC2_CAL_VOL_ATTEN3",
        ],
    );
    let mut vol1 = [0u32; 4];
    vol1[3] = adc1[3].wrapping_add(900);
    vol1[2] = adc1[2].wrapping_add(vol1[3]).wrapping_add(800);
    vol1[1] = adc1[1].wrapping_add(vol1[2]).wrapping_add(700);
    vol1[0] = adc1[0].wrapping_add(vol1[1]).wrapping_add(800);
    if unit == 0 {
        return vol1[atten];
    }
    let offset = [40, 10, 20, 15];
    vol1[atten]
        .wrapping_sub(adc2[atten])
        .wrapping_add(offset[atten])
}

/// One unit's calibration at one attenuation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Curve {
    unit: usize,
    atten: usize,
    /// `coeff_a`, as IDF's 32-bit division leaves it.
    coeff_a: u32,
}

impl Curve {
    /// `unit` (0 for ADC1) at `atten` on a chip with `set`. `None` when IDF does not read the
    /// calibration, or would divide by a `digi` of 0.
    pub(crate) fn new(set: &EfuseSet, unit: usize, atten: usize) -> Option<Curve> {
        if !calibrated(set) {
            return None;
        }
        let digi = digi(set, unit, atten);
        let coeff_a = (SCALING as u32).wrapping_mul(CAL_MV).checked_div(digi)?;
        Some(Curve {
            unit,
            atten,
            coeff_a,
        })
    }

    fn terms(&self) -> &'static [(u64, u64, i32)] {
        let count = if self.atten == 3 { 5 } else { 3 };
        &TERMS[self.unit][self.atten][..count]
    }

    /// The millivolts `adc_cali_raw_to_voltage` gives for `raw`, in IDF's integer arithmetic.
    #[cfg(test)]
    pub(crate) fn millivolts(&self, raw: u16) -> i32 {
        let v = u64::from(raw) * u64::from(self.coeff_a) / SCALING;
        if v == 0 {
            return 0;
        }
        let terms = self.terms();
        let (k0, d0, s0) = terms[0];
        let mut error = (k0 / d0) as i32 * s0;
        let mut variable: u64 = 1;
        for &(k, d, sign) in &terms[1..] {
            variable = variable.wrapping_mul(v);
            let term = variable.wrapping_mul(k) / d;
            error = error.wrapping_add((term as i32).wrapping_mul(sign));
        }
        (v as i32).wrapping_sub(error)
    }

    /// The two steps in real numbers at code `x`, with the first term as IDF's integer division
    /// leaves it.
    fn real(&self, x: f64) -> f64 {
        let v = x * f64::from(self.coeff_a) / SCALING as f64;
        let terms = self.terms();
        let (k0, d0, s0) = terms[0];
        let mut error = ((k0 / d0) as i32 * s0) as f64;
        let mut power = 1.0;
        for &(k, d, sign) in &terms[1..] {
            power *= v;
            error += f64::from(sign) * (k as f64 / d as f64) * power;
        }
        v - error
    }

    /// The code the SAR gives at `millivolts`: the real-valued calibration inverted, rounded to
    /// the nearest code and clipped to 0 and 4095.
    pub(crate) fn code(&self, millivolts: f64) -> u16 {
        let top = f64::from(MAX);
        if millivolts.is_nan() || millivolts <= self.real(0.0) {
            return 0;
        }
        if millivolts >= self.real(top) {
            return MAX;
        }
        let (mut low, mut high) = (0.0, top);
        for _ in 0..64 {
            let mid = (low + high) / 2.0;
            if self.real(mid) < millivolts {
                low = mid;
            } else {
                high = mid;
            }
        }
        ((low + high) / 2.0 + 0.5).floor().clamp(0.0, top) as u16
    }

    /// The highest voltage below which the code does not clip, `G(4095)`.
    #[cfg(test)]
    pub(crate) fn full_scale(&self) -> f64 {
        self.real(f64::from(MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic(unit: usize, atten: usize) -> Curve {
        Curve::new(&EfuseSet::default(), unit, atten).expect("calibrated")
    }

    /// With every calibration field 0, `digi` and the initial codes are the nominal values the
    /// sums start from.
    #[test]
    fn the_synthetic_set_gives_the_nominal_calibration() {
        let set = EfuseSet::default();
        let digis: Vec<[u32; 4]> = (0..2)
            .map(|unit| [0, 1, 2, 3].map(|atten| digi(&set, unit, atten)))
            .collect();
        assert_eq!(digis, [[3200, 2400, 1700, 900], [3240, 2410, 1720, 915]]);
        let codes: Vec<[Option<u32>; 4]> = (0..2)
            .map(|unit| [0, 1, 2, 3].map(|atten| initial_code(&set, unit, atten)))
            .collect();
        assert_eq!(
            codes,
            [[1850, 1940, 1940, 2010], [2020, 2020, 2020, 2020]].map(|row| row.map(Some))
        );
    }

    /// IDF's integer arithmetic at a few codes, each worked by hand from the formulas: at 2048,
    /// ADC1 at 11 dB has `coeff_a` 61895 and `v` 1934, and its error is
    /// `0 - 124 + 485 - 511 + 189`, so 1934 - 39.
    #[test]
    fn millivolts_is_idfs_integer_arithmetic() {
        let curve = synthetic(0, 3);
        assert_eq!(curve.coeff_a, 61_895);
        assert_eq!(curve.millivolts(0), 0);
        assert_eq!(curve.millivolts(2048), 1934 - 39);
        assert_eq!(synthetic(0, 0).millivolts(4095), 1083);
        assert_eq!(synthetic(0, 1).millivolts(4095), 1438);
        assert_eq!(synthetic(0, 2).millivolts(4095), 1992);
        assert_eq!(synthetic(1, 0).millivolts(4095), 1072);
        assert_eq!(synthetic(1, 2).millivolts(4095), 1957);
    }

    /// The real-valued calibration rises across every code, so it has one inverse.
    #[test]
    fn the_synthetic_curves_rise_across_every_code() {
        for unit in 0..2 {
            for atten in 0..4 {
                let curve = synthetic(unit, atten);
                let mut last = curve.real(0.0);
                for k in 1..=4 * 4095 {
                    let next = curve.real(f64::from(k) / 4.0);
                    assert!(next > last, "ADC{} atten {atten} at {k}/4", unit + 1);
                    last = next;
                }
            }
        }
    }

    /// At every attenuation the code reads back within a code's step and IDF's rounding of the
    /// voltage, and it clips at 0 and at the curve's top. At 11 dB the check stops at 2900 mV,
    /// the top of the range the datasheet states its error for: above 3770 mV of `v` on ADC1 and
    /// 3532 mV on ADC2, IDF's `v^2` term passes 64 bits and its reading wraps, as on the chip.
    #[test]
    fn the_code_reads_back_its_voltage_at_every_attenuation() {
        let tops = [
            [1083.7, 1438.1, 1992.4, 3245.6],
            [1071.9, 1431.0, 1958.2, 3187.4],
        ];
        for (unit, row) in tops.iter().enumerate() {
            for (atten, want) in row.iter().enumerate() {
                let curve = synthetic(unit, atten);
                let top = curve.full_scale();
                assert!((top - want).abs() < 0.1, "{top}");
                assert_eq!(curve.code(-5.0), 0);
                assert_eq!(curve.code(top + 1.0), MAX);
                assert_eq!(curve.code(5000.0), MAX);
                let end = if atten == 3 { 2900.0 } else { top - 20.0 };
                let mut mv = 20.0;
                while mv < end {
                    let code = curve.code(mv);
                    let read = f64::from(curve.millivolts(code));
                    assert!(
                        (read - mv).abs() <= 3.0,
                        "{unit} {atten} {mv}: {code} {read}"
                    );
                    mv += 7.3;
                }
            }
        }
    }

    /// 1.65 V clips at 0 and 2.5 dB and reads the same at 6 and 11 dB.
    #[test]
    fn half_of_three_point_three_volts() {
        assert_eq!(synthetic(0, 0).code(1650.0), MAX);
        assert_eq!(synthetic(0, 1).code(1650.0), MAX);
        for atten in [2, 3] {
            let curve = synthetic(0, atten);
            let read = curve.millivolts(curve.code(1650.0));
            assert!((read - 1650).abs() <= 2, "{atten}: {read}");
        }
    }

    /// A set IDF does not read the calibration of has no curve and no initial code.
    #[test]
    fn a_set_without_calibration_has_no_curve() {
        let set = EfuseSet::default().with("BLK_VERSION_MAJOR", 0);
        assert!(!calibrated(&set));
        assert_eq!(Curve::new(&set, 0, 3), None);
        assert_eq!(initial_code(&set, 0, 3), None);
    }
}
