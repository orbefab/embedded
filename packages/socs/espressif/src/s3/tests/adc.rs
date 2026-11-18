//! A one-shot conversion as IDF's `adc_oneshot_read` runs it, through the whole chip: the
//! sample halt at the start's store, the code in `MEAS1_DATA_SAR` 25 SAR cycles later, and the
//! same halts however the run splits.

use super::super::curve::Curve;
use super::*;

const ANALOG: u32 = 0x6000_e000;
const SENS: u32 = RTC_CNTL + 0x800;
const MEAS1_CTRL2: u32 = SENS + 0x0c;
/// `SAR1_EN_PAD_FORCE`, channel 5's `SAR1_EN_PAD` bit and `MEAS1_START_FORCE`.
const CTRL2: u32 = 1 << 31 | 1 << (19 + 5) | 1 << 18;
const START: u32 = 1 << 17;
const DATA: u32 = 0x3fcb_0000;
const GPIO6_VOLTS: f64 = 1.65;

/// A write of `byte` to register `reg` of the SAR ADC's analog I2C block.
fn regi2c(reg: u32, byte: u32) -> (u32, u32) {
    (ANALOG + 4, 0x69 | reg << 8 | byte << 16 | 1 << 24)
}

/// IDF's writes for a read of ADC1 channel 5 at 11 dB on the synthetic set, ending in `start`.
fn setup(start: u32) -> Vec<(u32, u32)> {
    vec![
        (SENS + 0x104, 1 << 30),
        (SENS + 0x3c, 3 << 29),
        (SENS, 0x2004_0001),
        (MEAS1_CTRL2, CTRL2),
        regi2c(2, 4 << 4),
        regi2c(1, 2010 >> 8),
        regi2c(0, 2010 & 0xff),
        (MEAS1_CTRL2, start),
    ]
}

/// [`setup`], then the poll of `MEAS1_DONE_SAR`, the word stored at `DATA`, and GPIO21 raised.
fn conversion() -> Program {
    let pairs = setup(CTRL2 | START);
    let mut literals: Vec<u32> = pairs.iter().flat_map(|&(a, v)| [a, v]).collect();
    let more = literals.len();
    literals.extend([MEAS1_CTRL2, DATA, GPIO, 1 << 21]);
    let mut program = Program::new(&literals);
    for k in 0..pairs.len() {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
    }
    program.l32r(2, more);
    let poll = program.pc();
    program.rri8(2, 3, 2, 0);
    // `bbci a3, 16, poll`: op0 7, r 7 for a bit at 16 or above, s a3, t bit 16's low four bits.
    let imm = poll.wrapping_sub(program.pc() + 4) as u8;
    program.raw(&[0x07, 0x70 | 3, imm]);
    program
        .l32r(4, more + 1)
        .rri8(6, 3, 4, 0)
        .l32r(5, more + 2)
        .l32r(6, more + 3)
        .rri8(6, 6, 5, 0x24)
        .rri8(6, 6, 5, 0x08)
        .raw(&LOOP);
    program
}

/// Each halt with GPIO21's drive after it, through crystal cycle `through` in limits `step`
/// apart, with GPIO6 at 1.65 V at every sample.
fn run(step: u64) -> (Vec<(Halt, PadDrive)>, Esp32s3) {
    let mut chip = conversion().chip_on(&[6, 21]);
    let mut seen = Vec::new();
    let mut limit = 0;
    while limit < 600 {
        limit = (limit + step).min(600);
        while let Some(halt) = chip.run_until(c(limit)) {
            if let Halt::Change { sample: true, .. } = halt {
                chip.sample(|gpio| if gpio == 6 { GPIO6_VOLTS } else { 0.0 });
            }
            seen.push((halt, chip.drive(21)));
        }
    }
    (seen, chip)
}

/// The start's store halts with `sample` at its own tick, and the core reads the code 25 SAR
/// cycles later, at the same ticks however the run splits.
#[test]
fn a_conversion_samples_at_its_start_and_lands_the_same_however_the_run_splits() {
    let (seen, mut chip) = run(600);
    let sample = seen[0].0;
    let start = store_pc(&conversion(), 7);
    assert!(
        matches!(
            sample,
            Halt::Change {
                sample: true,
                image: false,
                record: false,
                ..
            }
        ),
        "{sample:?}"
    );
    // Store `k` of three instructions each reaches the bus on cycle `3k + 2`.
    assert_eq!(sample.at(), c(23), "the start at {start:?}");
    let code = Curve::new(&EfuseSet::default(), 0, 3)
        .expect("calibrated")
        .code(GPIO6_VOLTS * 1000.0);
    let word = chip.load_word(DATA).expect("SRAM");
    assert_eq!(word & 0x1_ffff, 1 << 16 | u32::from(code));
    let raised = seen.last().expect("GPIO21 rises").0.at();
    // A bare chip keeps no `RTC_CNTL_RTC_CLK_CONF_REG`, so RTC_FAST_CLK is XTAL_D2 at 20 MHz,
    // and 25 cycles of it are 600 ticks.
    assert!(raised > Tick(sample.at().0 + 600), "{raised:?}");
    for step in [1, 2, 3, 7] {
        let (split, _) = run(step);
        assert_eq!(split, seen, "limits {step} apart");
    }
}

/// The chip's eFuse set feeds the start's check: an initial code the set does not give ends the
/// run at the start's store.
#[test]
fn a_start_with_another_sets_initial_code_ends_the_run_at_its_store() {
    let program = stores(&setup(CTRL2 | START));
    let mut chip = program.chip_on(&[6]);
    let other = EfuseSet::default().with("ADC1_INIT_CODE_ATTEN0", 1);
    chip.use_efuse(other);
    let mut seen = Vec::new();
    while let Some(halt) = chip.run_until(c(200)) {
        seen.push(halt);
    }
    let refused = Refused::Register("SENS", "initial_code");
    assert_eq!(
        seen.last().copied().map(|halt| match halt {
            Halt::Fault(_, fault) => Some(fault),
            _ => None,
        }),
        Some(Some(Fault::Refused(refused, store_pc(&program, 7))))
    );
    assert!(
        !seen
            .iter()
            .any(|halt| matches!(halt, Halt::Change { sample: true, .. })),
        "a refused start samples nothing"
    );
}

/// A pad with no net, an interrupt enable and the digital controller end the run at their
/// stores. A SENS bit the model keeps without running is listed.
#[test]
fn sens_settings_the_model_does_not_run_end_the_run_at_their_store() {
    for (pairs, refused, k) in [
        (
            setup(CTRL2 | START),
            Refused::Register("SENS", "pad_no_net"),
            7,
        ),
        (
            vec![(RTC_CNTL + 0x40, 1 << 11)],
            Refused::Interrupt("SENS"),
            0,
        ),
        (
            vec![(SENS + 0x10, 1 << 31)],
            Refused::Register("SENS", "dig_controller"),
            0,
        ),
        (
            vec![(SENS, 0x2004_0001 | 1 << 28)],
            Refused::Register("SENS", "data_inv"),
            0,
        ),
    ] {
        let program = stores(&pairs);
        let mut chip = program.chip_on(&[21]);
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
    let program = stores(&[(SENS + 0x104, 1 << 31 | 1 << 30)]);
    let mut chip = program.chip();
    while chip.run_until(c(200)).is_some() {}
    let listed: Vec<u32> = chip.unmodeled().map(|(address, _)| address).collect();
    assert_eq!(listed, [SENS + 0x104], "IOMUX_CLK_EN is kept, not run");
}
