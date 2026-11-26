//! Whole-chip tests for the internal temperature sensor (`SENS_SAR_TSENS_CTRL_REG` and
//! `SENS_SAR_TSENS_CTRL2_REG`): `sample` halt on `DUMP_OUT`, eFuse `TEMP_CALIB`, all five DAC
//! ranges, `SENS_TSENS_RESET`, and refused settings.

use super::*;

const ANALOG: u32 = 0x6000_e000;
const SENS: u32 = RTC_CNTL + 0x800;
const TSENS_CTRL: u32 = SENS + 0x50;
const TSENS_CTRL2: u32 = SENS + 0x54;
const CLK_GATE: u32 = SENS + 0x104;
const RESET_CONF: u32 = SENS + 0x108;
const DATA: u32 = 0x3fcb_0000;

const POWER_ON: u32 = (1 << 23) | (1 << 22) | (6 << 14) | (1 << 12);
const DUMP_OUT: u32 = 1 << 24;
const CTRL2_ON: u32 = (1 << 14) | (3 << 12) | 2;

fn regi2c_dac(reg_val: u32) -> (u32, u32) {
    (ANALOG + 4, 0x69 | (0x6 << 8) | (reg_val << 16) | (1 << 24))
}

/// IDF's `temperature_sensor_ll` read sequence: enables clock, powers sensor, sets DAC range,
/// asserts `DUMP_OUT`, polls `READY`, clears `DUMP_OUT`, reads `OUT`, stores it at `DATA`.
fn dump_program(dac_reg: u32) -> Program {
    let pairs = [
        (CLK_GATE, 1 << 29),
        (RESET_CONF, 1 << 29),
        (RESET_CONF, 0),
        (TSENS_CTRL2, CTRL2_ON),
        (TSENS_CTRL, POWER_ON),
        regi2c_dac(dac_reg),
        (TSENS_CTRL, POWER_ON | DUMP_OUT),
    ];
    let mut literals: Vec<u32> = pairs.iter().flat_map(|&(a, v)| [a, v]).collect();
    let more = literals.len();
    literals.extend([TSENS_CTRL, POWER_ON, DATA]);
    let mut program = Program::new(&literals);
    for k in 0..pairs.len() {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
    }
    program.l32r(2, more);
    let poll = program.pc();
    program.rri8(2, 3, 2, 0);
    // `bbci a3, 8, poll` (op0=7, r=6, s=3, t=8 -> READY is bit 8).
    let imm = poll.wrapping_sub(program.pc() + 4) as u8;
    program.raw(&[0x87, 0x60 | 3, imm]);
    // Clear DUMP_OUT, then read OUT (matching IDF `temperature_sensor_ll_get_raw_value`).
    program
        .l32r(4, more + 1)
        .rri8(6, 4, 2, 0)
        .rri8(2, 3, 2, 0)
        .l32r(5, more + 2)
        .rri8(6, 3, 5, 0)
        .raw(&LOOP);
    program
}

#[test]
fn dump_samples_temperature_keeps_out_across_clear_and_is_split_invariant() {
    let run = |dac: u32, temp_c: f64, step: u64| {
        let mut chip = dump_program(dac).chip();
        let mut seen = Vec::new();
        let mut limit = 0;
        while limit < 120 {
            limit = (limit + step).min(120);
            while let Some(halt) = chip.run_until(c(limit)) {
                if let Halt::Change { sample: true, .. } = halt {
                    chip.sample_temperature(temp_c);
                }
                seen.push(halt);
            }
        }
        let stored = chip.load_word(DATA).expect("SRAM");
        (seen, stored, chip.unmodeled().count())
    };

    // DAC 15 (offset 0, -10..80 degC) at 25.0 degC -> out = 104, READY cleared after DUMP_OUT=0.
    let (whole, stored, unmodeled) = run(15, 25.0, 120);
    assert_eq!(unmodeled, 0);
    assert!(
        matches!(
            whole[..],
            [Halt::Change {
                sample: true,
                image: false,
                record: false,
                ..
            }]
        ),
        "{whole:?}"
    );
    assert_eq!(stored & 0xff, 104);
    assert_eq!(stored & (1 << 8), 0, "READY clears with DUMP_OUT");

    for step in [1, 2, 5, 13] {
        let (split, split_stored, _) = run(15, 25.0, step);
        assert_eq!(split, whole, "step {step}");
        assert_eq!(split_stored, stored, "step {step}");
    }

    // Verify all five TRM DAC ranges recover the requested temperature within 0.25 degC.
    for (dac, offset, temp_c) in [
        (5, -2, 100.0),
        (7, -1, 60.0),
        (15, 0, 25.0),
        (11, 1, -10.0),
        (10, 2, -30.0),
    ] {
        let (_, word, _) = run(dac, temp_c, 120);
        let raw = (word & 0xff) as f64;
        let recovered = 0.4386 * raw - 27.88 * f64::from(offset) - 20.52;
        assert!(
            (recovered - temp_c).abs() <= 0.25,
            "dac {dac} temp {temp_c}: raw={raw} recovered={recovered}"
        );
    }
}

#[test]
fn tsens_refuses_unmodeled_settings_and_interrupt_enable() {
    for (pairs, refused, k) in [
        // Missing clock gate
        (
            vec![
                (TSENS_CTRL2, CTRL2_ON),
                (TSENS_CTRL, POWER_ON),
                regi2c_dac(15),
                (TSENS_CTRL, POWER_ON | DUMP_OUT),
            ],
            Refused::Register("SENS", "tsens_clock_off"),
            3,
        ),
        // Held in reset
        (
            vec![
                (CLK_GATE, 1 << 29),
                (RESET_CONF, 1 << 29),
                (TSENS_CTRL2, CTRL2_ON),
                (TSENS_CTRL, POWER_ON),
                regi2c_dac(15),
                (TSENS_CTRL, POWER_ON | DUMP_OUT),
            ],
            Refused::Register("SENS", "tsens_held_in_reset"),
            5,
        ),
        // Invalid DAC range
        (
            vec![
                (CLK_GATE, 1 << 29),
                (TSENS_CTRL2, CTRL2_ON),
                (TSENS_CTRL, POWER_ON),
                regi2c_dac(3),
                (TSENS_CTRL, POWER_ON | DUMP_OUT),
            ],
            Refused::Register("SENS", "tsens_range"),
            4,
        ),
        // Inverted input
        (
            vec![(TSENS_CTRL, POWER_ON | (1 << 13))],
            Refused::Register("SENS", "tsens_in_inv"),
            0,
        ),
        // RTC_CNTL_TSENS_INT_ENA (bit 12)
        (
            vec![(RTC_CNTL + 0x40, 1 << 12)],
            Refused::Interrupt("SENS"),
            0,
        ),
    ] {
        let program = stores(&pairs);
        let mut chip = program.chip();
        let mut last = None;
        while let Some(halt) = chip.run_until(c(200)) {
            last = Some(halt);
        }
        assert!(
            matches!(last, Some(Halt::Fault(_, Fault::Refused(found, pc)))
                if found == refused && pc == store_pc(&program, k)),
            "{refused:?}: {last:?}"
        );
    }
}
