//! The compensation formulas and Table 13's measurement times.

use super::model::{
    adc_for_pressure, adc_for_temperature, compensate_pressure, compensate_temperature, measure_us,
    osr_count,
};

#[test]
fn the_example_raw_temperature_compensates_to_25_08_c() {
    let (temp, t_fine) = compensate_temperature(519_888);
    assert_eq!(temp, 2508, "t_fine {t_fine}");
    let pressure = compensate_pressure(t_fine, 415_148);
    assert_eq!(pressure, 25_767_233);
}

#[test]
fn table_13_typicals_are_the_measurement_time() {
    let rows = [
        (1, 1, 5_500),
        (2, 1, 7_500),
        (4, 1, 11_500),
        (8, 1, 19_500),
        (16, 2, 37_500),
    ];
    for (osr_p, osr_t, us) in rows {
        assert_eq!(measure_us(osr_t, osr_p), us, "×{osr_p}/×{osr_t}");
    }
    assert_eq!(osr_count(0), 0);
    assert_eq!(osr_count(5), 16);
    assert_eq!(osr_count(7), 16);
    assert_eq!(measure_us(0, 0), 0);
}

#[test]
fn a_compensated_reading_stays_inside_the_sheet_accuracy() {
    // ±1 °C and ±1 hPa. The block's set points are exact on temperature and a few hundredths
    // of a pascal on pressure, which is what two printed decimals show.
    let points = [
        (20.0, 101_325),
        (25.0, 100_000),
        (0.0, 101_325),
        (-10.0, 90_000),
    ];
    for (celsius, pascals) in points {
        let hundredths = (celsius * 100.0_f64).round() as i32;
        let (adc_t, t_fine) = adc_for_temperature(hundredths);
        let (got_t, got_fine) = compensate_temperature(adc_t);
        assert_eq!(got_fine, t_fine);
        assert!(
            (got_t - hundredths).abs() <= 100,
            "{celsius} °C read as {}",
            got_t as f64 / 100.0
        );
        let adc_p = adc_for_pressure(t_fine, pascals);
        let got_p = compensate_pressure(t_fine, adc_p) as f64 / 256.0;
        assert!(
            (got_p - pascals as f64).abs() <= 100.0,
            "{pascals} Pa read as {got_p}"
        );
    }
    let (adc_t, t_fine) = adc_for_temperature(2_000);
    assert_eq!(compensate_temperature(adc_t).0, 2_000);
    let got = compensate_pressure(t_fine, adc_for_pressure(t_fine, 101_325)) as f64 / 256.0;
    assert!((got - 101_325.0).abs() < 0.05, "{got}");
}
