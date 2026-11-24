//! The compensation formulas, Table 13's measurement times, §4.3.5 `config` write rules,
//! and §3.3.3 Table 6 / Table 11 IIR filter and standby timing.

use orbe_model::{Cx, Device, Dir, Fields, I2cTarget, Levels, Quantity, Readings, Time};

use super::model::{
    Bmp280, adc_for_pressure, adc_for_temperature, compensate_pressure, compensate_temperature,
    measure_us, osr_count,
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

struct Bench {
    dev: Bmp280,
    temp_c: f64,
    pressure_pa: f64,
    t_ps: u64,
}

impl Bench {
    fn new(temp_c: f64, pressure_pa: f64) -> Bench {
        let mut dev = Bmp280::new(&Fields::NONE).unwrap();
        let mut cx = Cx::new(
            Readings::NONE
                .with(Quantity::Temperature, temp_c)
                .with(Quantity::Pressure, pressure_pa),
        );
        // SCK, SDI, CSB high; SDO low (0x76); VDD and VDDIO above Table 2 minimums.
        dev.input(
            &mut cx,
            Time::ZERO,
            Levels::from_slice(&[true, true, true, false, true, true]),
        );
        assert_eq!(cx.broken(), None);
        Bench {
            dev,
            temp_c,
            pressure_pa,
            t_ps: 0,
        }
    }

    fn cx(&self) -> Cx {
        Cx::new(
            Readings::NONE
                .with(Quantity::Temperature, self.temp_c)
                .with(Quantity::Pressure, self.pressure_pa),
        )
    }

    fn write_reg(&mut self, reg: u8, val: u8) -> Option<(&'static str, Time)> {
        let mut cx = self.cx();
        let at = Time::from_ps(self.t_ps);
        self.dev.start(&mut cx, at, Dir::Write);
        self.dev.write(&mut cx, at, reg);
        self.dev.write(&mut cx, at, val);
        self.dev.stop(&mut cx, at);
        cx.broken()
    }

    fn read_reg(&mut self, reg: u8) -> u8 {
        let mut cx = self.cx();
        let at = Time::from_ps(self.t_ps);
        self.dev.start(&mut cx, at, Dir::Write);
        self.dev.write(&mut cx, at, reg);
        self.dev.start(&mut cx, at, Dir::Read);
        let byte = self.dev.read(&mut cx, at);
        self.dev.stop(&mut cx, at);
        byte
    }

    fn read_temp_c100(&mut self) -> i32 {
        let mut cx = self.cx();
        let at = Time::from_ps(self.t_ps);
        self.dev.start(&mut cx, at, Dir::Write);
        self.dev.write(&mut cx, at, 0xFA);
        self.dev.start(&mut cx, at, Dir::Read);
        let msb = i32::from(self.dev.read(&mut cx, at));
        let lsb = i32::from(self.dev.read(&mut cx, at));
        let xlsb = i32::from(self.dev.read(&mut cx, at));
        self.dev.stop(&mut cx, at);
        let adc_t = (msb << 12) | (lsb << 4) | (xlsb >> 4);
        compensate_temperature(adc_t).0
    }

    fn run_to(&mut self, limit_ps: u64) {
        loop {
            let mut cx = self.cx();
            let Some(stop) = self.dev.run_until(&mut cx, Time::from_ps(limit_ps)) else {
                break;
            };
            assert_eq!(cx.broken(), None);
            self.t_ps = stop.at.as_ps();
        }
        self.t_ps = limit_ps;
    }
}

#[test]
fn config_write_in_normal_mode_warns_only_when_changed_and_sleep_or_forced_stays_silent() {
    let mut b = Bench::new(20.0, 101_325.0);

    // Sleep mode: silent and takes effect.
    assert_eq!(b.write_reg(0xF5, 0x10), None);
    assert_eq!(b.read_reg(0xF5), 0x10);

    // Forced mode (0b01): silent and takes effect.
    assert_eq!(b.write_reg(0xF4, 0x55), None);
    assert_eq!(b.write_reg(0xF5, 0x20), None);
    assert_eq!(b.read_reg(0xF5), 0x20);
    b.run_to(40_000_000_000);
    assert_eq!(b.read_reg(0xF4) & 0b11, 0, "forced returns to sleep");

    // Normal mode (0b11): unchanged config write is silent.
    assert_eq!(b.write_reg(0xF4, 0x57), None);
    assert_eq!(b.write_reg(0xF5, 0x20), None);
    assert_eq!(b.read_reg(0xF5), 0x20);

    // Normal mode (0b11): changing config is ignored and warns `config_in_normal`.
    b.t_ps = 41_000_000_000;
    assert_eq!(
        b.write_reg(0xF5, 0x90),
        Some(("config_in_normal", Time::from_ps(41_000_000_000)))
    );
    assert_eq!(b.read_reg(0xF5), 0x20, "config stays unchanged");
}

#[test]
fn filter_x16_and_500ms_standby_step_response_matches_table_6_and_11() {
    const US: u64 = 1_000_000;
    const MEASURE_PS: u64 = 37_500 * US; // Table 13: 1.5 + 2*2 + 2*16 = 37.5 ms
    const STANDBY_PS: u64 = 500_000 * US; // Table 11: t_sb = 100 -> 500 ms
    const CYCLE_PS: u64 = MEASURE_PS + STANDBY_PS;

    let mut b = Bench::new(20.0, 101_325.0);
    assert_eq!(b.write_reg(0xF4, 0x00), None); // sleep
    assert_eq!(b.write_reg(0xF5, 0x90), None); // t_sb = 500 ms, filter = x16
    assert_eq!(b.write_reg(0xF4, 0x57), None); // normal, osrs_t = x2, osrs_p = x16

    // First conversion completes at t_measure = 37.5 ms.
    b.run_to(MEASURE_PS - 1);
    assert_eq!(b.read_reg(0xF3) & 0x08, 0x08, "still measuring");
    b.run_to(MEASURE_PS);
    assert_eq!(b.read_reg(0xF3) & 0x08, 0, "conversion finished");
    assert_eq!(b.read_temp_c100(), 2_000);

    // Step world temperature from 20 C to 85 C during standby.
    b.temp_c = 85.0;
    // Standby lasts the full 500 ms before the next conversion finishes at MEASURE_PS + CYCLE_PS.
    b.run_to(MEASURE_PS + STANDBY_PS);
    assert_eq!(b.read_temp_c100(), 2_000, "unchanged during 500 ms standby");

    let mut samples = Vec::new();
    for n in 1..=22u64 {
        b.run_to(MEASURE_PS + n * CYCLE_PS);
        samples.push(b.read_temp_c100());
    }
    // §3.3.3 IIR (coeff 16): (15 * old + adc(85 C)) / 16.
    assert_eq!(samples[0], 2_409); // sample 1: 24.09 C (ideal 24.06 C)
    assert_eq!(samples[1], 2_792); // sample 2: 27.92 C (ideal 27.87 C)
    assert_eq!(samples[2], 3_151); // sample 3: 31.51 C (ideal 31.44 C)
    assert_eq!(samples[3], 3_487); // sample 4: 34.87 C (ideal 34.79 C)
    // Table 6: 22 samples to reach >= 75% of step response (20 + 0.75 * 65 = 68.75 C).
    assert_eq!(samples[20], 6_833);
    assert!(
        samples[20] < 6_875,
        "sample 21 is under 75%: {}",
        samples[20]
    );
    assert_eq!(samples[21], 6_937);
    assert!(
        samples[21] >= 6_875,
        "sample 22 reaches >= 75%: {}",
        samples[21]
    );
}
