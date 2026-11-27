use orbe_model::{Cx, Device, Drive, Fields, Levels, Time, Value, View};

use super::model::{
    ISET_DROP_V, ISET_RESISTANCE_OHM, Max7219, T_DO_PS, WATCH_CLK, WATCH_DIN, WATCH_LOAD,
    WATCH_SUPPLY, table11_iseg,
};

const US: u64 = 1_000_000;

fn at(ps: u64) -> Time {
    Time::from_ps(ps)
}

fn levels(din: bool, clk: bool, load: bool, supply: bool) -> Levels {
    let mut bits = [false; 4];
    bits[WATCH_DIN] = din;
    bits[WATCH_CLK] = clk;
    bits[WATCH_LOAD] = load;
    bits[WATCH_SUPPLY] = supply;
    Levels::from_slice(&bits)
}

fn make_chip(vled_mv: i64) -> Max7219 {
    let fields = [
        ("vled_mv", Value::Int(vled_mv)),
        ("shutdown_na", Value::Int(150_000)),
        ("logic_na", Value::Int(8_000_000)),
        ("holds_above_mv", Value::Int(4_000)),
    ];
    Max7219::new(&Fields::new(&fields)).expect("valid params")
}

/// Computes `V(ISET)` for a given `VCC` and `RSET` resistor connected between `VCC` and `ISET`.
fn v_iset_for(vcc: f64, rset_ohm: f64) -> f64 {
    let i = (vcc - ISET_DROP_V) / (rset_ohm + ISET_RESISTANCE_OHM);
    ISET_DROP_V + i * ISET_RESISTANCE_OHM
}

fn power_on(chip: &mut Max7219, vcc: f64, rset_ohm: f64) {
    let mut cx = Cx::default();
    chip.sample(&mut cx, Time::ZERO, &[vcc, v_iset_for(vcc, rset_ohm)]);
    chip.input(&mut cx, Time::ZERO, levels(false, false, true, true));
}

/// Shifts a 16-bit word MSB-first and optionally pulses LOAD high at the end.
/// Returns `(shifted_out, broken_rule)`.
fn shift_word(
    chip: &mut Max7219,
    t: &mut u64,
    word: u16,
    latch: bool,
) -> (u16, Option<&'static str>) {
    let mut shifted_out = 0u16;
    let mut broken = None;
    let mut feed = |chip: &mut Max7219, time: u64, lv: Levels| {
        let mut cx = Cx::default();
        chip.input(&mut cx, at(time), lv);
        if broken.is_none() {
            broken = cx.broken().map(|(r, _)| r);
        }
    };
    *t += US;
    feed(chip, *t, levels(false, false, false, true));
    for bit_idx in (0..16).rev() {
        let din = (word >> bit_idx) & 1 != 0;
        let dout = chip.drive().get(0) == Drive::High;
        shifted_out = (shifted_out << 1) | u16::from(dout);
        *t += US;
        feed(chip, *t, levels(din, false, false, true));
        *t += US;
        feed(chip, *t, levels(din, true, false, true));
        *t += US;
        feed(chip, *t, levels(din, false, false, true));
        let mut cx = Cx::default();
        if let Some(stop) = chip.run_until(&mut cx, at(*t + T_DO_PS)) {
            assert_eq!(stop.at, at(*t + T_DO_PS));
        }
    }
    if latch {
        *t += US;
        feed(chip, *t, levels(false, false, true, true));
    }
    (shifted_out, broken)
}

fn frame_of(chip: &Max7219) -> [u8; 64] {
    let View::Screen { frame, .. } = chip.view() else {
        panic!("max7219 shows a screen");
    };
    frame.try_into().expect("64 bytes")
}

#[test]
fn powers_up_in_shutdown_and_blanked() {
    let mut chip = make_chip(1_850);
    power_on(&mut chip, 5.0, 10_000.0);
    assert_eq!(frame_of(&chip), [0; 64]);
    assert!((chip.draw().amps - 150e-6).abs() < 1e-9);
    let View::Screen { panel, .. } = chip.view() else {
        panic!();
    };
    assert!(!panel.on);
    assert!(panel.asleep);
    assert_eq!(panel.settings.get("scan_limit"), Some(0));
    assert_eq!(panel.settings.get("intensity"), Some(0));
    assert_eq!(panel.settings.get("decode_mode"), Some(0));
    assert_eq!(panel.settings.get("display_test"), Some(0));
}

#[test]
fn table11_matches_all_twenty_datasheet_points_and_interpolation() {
    // Maxim MAX7219/MAX7221 Table 11 (p. 11): all 20 (RSET, VLED) -> ISEG points.
    let vleds = [1.5, 2.0, 2.5, 3.0, 3.5];
    let rows = [
        (0.040, [12_200.0, 11_800.0, 11_000.0, 10_600.0, 9_690.0]),
        (0.030, [17_800.0, 17_100.0, 15_800.0, 15_000.0, 14_000.0]),
        (0.020, [29_800.0, 28_000.0, 25_900.0, 24_500.0, 22_600.0]),
        (0.010, [66_700.0, 63_700.0, 59_300.0, 55_400.0, 51_200.0]),
    ];
    for (iseg, rsets) in rows {
        for (vled, rset) in vleds.into_iter().zip(rsets) {
            let got = table11_iseg(rset, vled);
            assert!(
                (got - iseg).abs() < 1e-6,
                "VLED={vled} RSET={rset}: got {got}, expected {iseg}"
            );
        }
    }
    // 9.53 kohm at VLED = 3.5 V (~VOUT = V+ - 1 V) sits near the 40 mA typ electrical spec;
    // at VLED = 1.85 V it extrapolates cleanly above 40 mA per Table 11's conductance slope.
    assert!((table11_iseg(9_530.0, 3.5) - 0.04055).abs() < 1e-4);
    assert!((table11_iseg(10_000.0, 3.5) - 0.03899).abs() < 1e-4);
    assert!((table11_iseg(9_530.0, 1.85) - 0.04805).abs() < 1e-4);
    assert!((table11_iseg(10_000.0, 1.85) - 0.04617).abs() < 1e-4);
    // Larger RSET lowers current monotonically across the range.
    assert!(table11_iseg(20_000.0, 1.85) < table11_iseg(10_000.0, 1.85));
    assert!(table11_iseg(100_000.0, 2.0) < table11_iseg(63_700.0, 2.0));
}

#[test]
fn registers_intensity_scan_limit_shutdown_and_display_test() {
    let mut chip = make_chip(2_000);
    power_on(&mut chip, 5.0, 11_800.0); // exactly 40 mA at VLED = 2.0 V
    assert!((chip.iseg_a() - 0.040).abs() < 1e-4);
    let mut t = 0;

    // Wake up, scan all 8 digits, intensity 15 (31/32), write row 0 = 0xAA and row 7 = 0x55
    shift_word(&mut chip, &mut t, 0x0C01, true); // normal operation
    shift_word(&mut chip, &mut t, 0x0B07, true); // scan limit = 7 (8 digits)
    shift_word(&mut chip, &mut t, 0x0A0F, true); // intensity = 15 (31/32)
    shift_word(&mut chip, &mut t, 0x01AA, true); // digit 0 = 1010_1010
    shift_word(&mut chip, &mut t, 0x0855, true); // digit 7 = 0101_0101

    let expected_dot = 0.040 * (31.0 / 32.0) / 8.0;
    assert!((chip.dot_a() - expected_dot).abs() < 1e-5);
    let f = frame_of(&chip);
    assert!(f[0] > 0 && f[1] == 0 && f[2] > 0 && f[7] == 0);
    assert!(f[56] == 0 && f[57] > 0 && f[63] > 0);
    // Supply draw = 8 mA + 8 lit dots * expected_dot
    assert!((chip.draw().amps - (0.008 + 8.0 * expected_dot)).abs() < 1e-4);

    // Lower intensity to 0 (1/32): brightness drops by 31x
    shift_word(&mut chip, &mut t, 0x0A00, true);
    assert!((chip.dot_a() - (0.040 * (1.0 / 32.0) / 8.0)).abs() < 1e-5);
    assert!(frame_of(&chip)[0] > 0 && frame_of(&chip)[0] < f[0]);

    // Reduce scan limit to 3 (4 digits): row 7 blanks, row 0 doubles in brightness
    shift_word(&mut chip, &mut t, 0x0B03, true);
    assert!((chip.dot_a() - (0.040 * (1.0 / 32.0) / 4.0)).abs() < 1e-5);
    assert_eq!(frame_of(&chip)[57], 0);
    assert!(frame_of(&chip)[0] > 0);

    // Shutdown blanks display and drops draw to 150 uA, retaining digits
    shift_word(&mut chip, &mut t, 0x0C00, true);
    assert_eq!(frame_of(&chip), [0; 64]);
    assert!((chip.draw().amps - 150e-6).abs() < 1e-9);
    assert_eq!(chip.digits()[0], 0xAA);

    // Display test overrides shutdown, scan limit, and intensity: all 64 LEDs at 31/32, N = 8
    shift_word(&mut chip, &mut t, 0x0F01, true);
    assert!(frame_of(&chip).iter().all(|&px| px > 0));
    assert!((chip.dot_a() - expected_dot).abs() < 1e-5);
    // Leaving display test returns to shutdown (blank)
    shift_word(&mut chip, &mut t, 0x0F00, true);
    assert_eq!(frame_of(&chip), [0; 64]);
}

#[test]
fn code_b_decode_mode_and_noop_and_undefined_register() {
    let mut chip = make_chip(2_000);
    power_on(&mut chip, 5.0, 11_800.0);
    let mut t = 0;

    shift_word(&mut chip, &mut t, 0x0C01, true);
    shift_word(&mut chip, &mut t, 0x0B01, true); // 2 digits
    shift_word(&mut chip, &mut t, 0x0901, true); // Code B on digit 0 only
    shift_word(&mut chip, &mut t, 0x0180, true); // '0' + DP -> 0xFE
    shift_word(&mut chip, &mut t, 0x0201, true); // raw 0x01 -> only col 7 (G)

    let f = frame_of(&chip);
    // Row 0 = 0xFE = 1111_1110 (DP..F lit, G dark)
    assert!(f[0..7].iter().all(|&px| px > 0));
    assert_eq!(f[7], 0);
    // Row 1 = 0x01 = 0000_0001
    assert!(f[8..15].iter().all(|&px| px == 0));
    assert!(f[15] > 0);

    // No-op does nothing
    shift_word(&mut chip, &mut t, 0x00FF, true);
    assert_eq!(frame_of(&chip), f);

    // Undefined address 0xD warns protocol rule `command` and leaves display untouched
    let (_, broken) = shift_word(&mut chip, &mut t, 0x0D42, true);
    assert_eq!(broken, Some("command"));
    assert_eq!(frame_of(&chip), f);
}

#[test]
fn dout_shifts_out_16_clocks_later_and_missing_load_does_not_latch() {
    let mut chip = make_chip(2_000);
    power_on(&mut chip, 5.0, 11_800.0);
    let mut t = 0;

    // Shift 0x0C01 WITHOUT LOAD: display stays blank
    let _ = shift_word(&mut chip, &mut t, 0x0C01, false);
    assert_eq!(frame_of(&chip), [0; 64]);

    // Shift 0xA53C next: DOUT shifts out 0x0C01 over the 16 clocks!
    let (out, _) = shift_word(&mut chip, &mut t, 0xA53C, false);
    assert_eq!(out, 0x0C01);
    let (out2, _) = shift_word(&mut chip, &mut t, 0x0000, false);
    assert_eq!(out2, 0xA53C);
}

#[test]
fn open_iset_and_unpowered_blank_the_display() {
    let mut chip = make_chip(2_000);
    let mut cx = Cx::default();
    // Open ISET -> V_ISET == ISET_DROP_V -> i_iset == 0 -> blank, logic draw only (8 mA)
    chip.sample(&mut cx, Time::ZERO, &[5.0, ISET_DROP_V]);
    chip.input(&mut cx, Time::ZERO, levels(false, false, true, true));
    let mut t = 0;
    shift_word(&mut chip, &mut t, 0x0C01, true);
    shift_word(&mut chip, &mut t, 0x01FF, true);
    assert_eq!(frame_of(&chip), [0; 64]);
    assert!((chip.draw().amps - 0.008).abs() < 1e-6);

    // Dip to 3.3 V / 2.5 V / 2.0 V (above 2.0 V SRAM retention floor) keeps registers intact.
    for vcc in [3.999, 3.3, 2.5, 2.0] {
        chip.sample(&mut cx, at(t + US), &[vcc, v_iset_for(vcc, 10_000.0)]);
        assert_eq!(chip.digits()[0], 0xFF, "vcc={vcc}");
        assert!(frame_of(&chip)[0] > 0, "vcc={vcc}");
    }

    // Below 2.0 V retention floor: resets, blanks, draws nothing.
    chip.sample(&mut cx, at(t + 2 * US), &[1.9, 0.0]);
    chip.input(&mut cx, at(t + 2 * US), levels(false, false, false, false));
    assert_eq!(chip.digits()[0], 0x00);
    assert_eq!(frame_of(&chip), [0; 64]);
    assert_eq!(chip.draw().amps, 0.0);
}

#[test]
fn dot_ua_setting_does_not_clip_past_u16_max() {
    let mut chip = make_chip(1_850);
    power_on(&mut chip, 6.5, 1_000.0);
    let mut t = 0;
    shift_word(&mut chip, &mut t, 0x0F01, true); // display test: all 64 dots at 31/32
    let View::Screen { panel, .. } = chip.view() else {
        panic!("max7219 shows a screen");
    };
    let dot_ua = panel.settings.get("dot_ua").unwrap();
    assert!(dot_ua > u32::from(u16::MAX), "dot_ua clipped: {dot_ua}");
    assert!((f64::from(dot_ua) - 65_920.0).abs() < 200.0, "{dot_ua}");
}
