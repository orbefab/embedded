//! I2C0 as a master on two open-drain pads. Edges are the wire after each `Moved` halt, the
//! target's pull included, and a release's input is what sets the high side.

use super::*;

const I2C0: u32 = 0x6001_3000;
const I2C1: u32 = 0x6002_7000;
const SCL_PAD: u8 = 10;
const SDA_PAD: u8 = 9;
const SIG_SCL: u32 = 89;
const SIG_SDA: u32 = 90;

const CTR: u32 = 0x04;
const SR: u32 = 0x08;
const TO: u32 = 0x0c;
const FIFO_CONF: u32 = 0x18;
const DATA: u32 = 0x1c;
const INT_RAW: u32 = 0x20;
const INT_CLR: u32 = 0x24;
const INT_ENA: u32 = 0x28;
const SDA_HOLD: u32 = 0x30;
const SDA_SAMPLE: u32 = 0x34;
const SCL_HIGH: u32 = 0x38;
const SCL_LOW: u32 = 0x00;
const SCL_START_HOLD: u32 = 0x40;
const SCL_RSTART_SETUP: u32 = 0x44;
const SCL_STOP_HOLD: u32 = 0x48;
const SCL_STOP_SETUP: u32 = 0x4c;
const FILTER_CFG: u32 = 0x50;
const CLK_CONF: u32 = 0x54;
const COMD0: u32 = 0x58;
const SCL_SP_CONF: u32 = 0x80;

const SDA_FORCE_OUT: u32 = 1;
const SCL_FORCE_OUT: u32 = 1 << 1;
const MS_MODE: u32 = 1 << 4;
const TRANS_START: u32 = 1 << 5;
const CONF_UPGATE: u32 = 1 << 11;
const MODE: u32 = SDA_FORCE_OUT | SCL_FORCE_OUT | MS_MODE;
const GO: u32 = MODE | CONF_UPGATE | TRANS_START;
const SCLK_ACTIVE: u32 = 1 << 21;
/// Both filters on at 7 cycles.
const FILTER: u32 = 7 | 7 << 4 | 1 << 8 | 1 << 9;
const TRANS_COMPLETE: u32 = 1 << 7;
const NACK: u32 = 1 << 10;
const BUS_BUSY: u32 = 1 << 4;
const CMD_DONE: u32 = 1 << 31;

const FUN_WPU: u32 = 1 << 8;
const FUN_IE: u32 = 1 << 9;
const GPIO_FUNC: u32 = 1 << 12;
const PAD_DRIVER: u32 = 1 << 2;
const SIG_IN_SEL: u32 = 1 << 7;

/// IDF's 400 kHz field values. Lengths that count `reg + 1` are stored one short.
const LOW: u64 = 50;
const SDA_HOLD_LEN: u64 = 12;
const SDA_SAMPLE_LEN: u64 = 25;
const WAIT_HIGH: u64 = 23;
const HIGH: u64 = 27;
const HIGH_SIDE: u64 = WAIT_HIGH + HIGH;
const SETUP: u64 = 50;

/// One cycle. Before a spinning `LOOP`, a jump of three cycles, it puts the jump's boundaries
/// on the cycle of `one_byte`'s `TRANS_COMPLETE`.
const NOP: [u8; 3] = [0xf0, 0x20, 0x00];

fn cycles(n: u64) -> u64 {
    n * c(1).0
}

fn cmd(op: u32, n: u32, ack_en: bool, ack_exp: bool, ack_val: bool) -> u32 {
    n | u32::from(ack_en) << 8 | u32::from(ack_exp) << 9 | u32::from(ack_val) << 10 | op << 11
}

fn rstart() -> u32 {
    cmd(6, 0, false, false, false)
}

fn write_cmd(n: u32) -> u32 {
    cmd(1, n, true, false, false)
}

fn read_cmd(n: u32, nack: bool) -> u32 {
    cmd(3, n, false, false, nack)
}

fn stop_cmd() -> u32 {
    cmd(2, 0, false, false, false)
}

fn poke(chip: &mut Esp32s3, offset: u32, value: u32) {
    let soc = &mut chip.soc;
    soc.i2c
        .write(Tick(0), offset, value, &mut soc.gpio)
        .unwrap();
}

fn word(chip: &mut Esp32s3, offset: u32) -> u32 {
    let soc = &mut chip.soc;
    soc.i2c
        .read(Tick(0), offset, &mut soc.gpio)
        .expect("a kept register")
}

fn clock_on(chip: &mut Esp32s3) {
    // Power-on `PERIP_CLK_EN0` with I2C0's clock bit set. The block starts unclocked.
    chip.soc.restore(SYSTEM + 0x18, 0xf9c1_e06f | 1 << 7);
}

fn route(chip: &mut Esp32s3) {
    for (pad, signal) in [(SCL_PAD, SIG_SCL), (SDA_PAD, SIG_SDA)] {
        chip.soc
            .gpio
            .write_io_mux(0x4 + 4 * u32::from(pad), GPIO_FUNC | FUN_IE | FUN_WPU);
        chip.soc
            .gpio
            .write(0x74 + 4 * u32::from(pad), PAD_DRIVER)
            .unwrap();
        chip.soc
            .gpio
            .write(0x554 + 4 * u32::from(pad), signal)
            .unwrap();
        chip.soc
            .gpio
            .write(0x154 + 4 * signal, SIG_IN_SEL | u32::from(pad))
            .unwrap();
    }
}

/// The 400 kHz timing, copied into the run's timing, with both lines released.
fn arm(chip: &mut Esp32s3) {
    clock_on(chip);
    route(chip);
    chip.input(Tick(0), [(SCL_PAD, true), (SDA_PAD, true)]);
    for (offset, value) in [
        (SCL_LOW, (LOW - 1) as u32),
        (SDA_HOLD, (SDA_HOLD_LEN - 1) as u32),
        (SDA_SAMPLE, (SDA_SAMPLE_LEN - 1) as u32),
        (SCL_HIGH, (WAIT_HIGH << 9 | HIGH) as u32),
        (SCL_START_HOLD, (SETUP - 1) as u32),
        (SCL_RSTART_SETUP, (SETUP - 1) as u32),
        (SCL_STOP_HOLD, (SETUP - 1) as u32),
        (SCL_STOP_SETUP, (SETUP - 1) as u32),
        (TO, 10 | 1 << 5),
        (CLK_CONF, SCLK_ACTIVE),
    ] {
        poke(chip, offset, value);
    }
    poke(chip, CTR, MODE | CONF_UPGATE);
}

fn go(offset: u8) -> Program {
    let mut program = Program::new(&[I2C0, GO]);
    program
        .l32r(2, 0)
        .l32r(3, 1)
        .rri8(6, 3, 2, offset)
        .raw(&LOOP);
    program
}

/// `hold_sda` sees the 1-based count of SCL releases so far, 0 before the first.
fn play(
    chip: &mut Esp32s3,
    through: u64,
    step: u64,
    mut hold_sda: impl FnMut(u32) -> bool,
) -> Result<Vec<(u64, u8, bool)>, Halt> {
    let mut wire = [true, true];
    let mut rises = 0u32;
    let mut edges = Vec::new();
    let mut limit = 0u64;
    while limit < through {
        limit = (limit + step).min(through);
        loop {
            match chip.run_until(Tick(limit)) {
                Some(Halt::Moved(at)) => {
                    let drives = [chip.drive(SCL_PAD), chip.drive(SDA_PAD)];
                    if !matches!(drives[0], PadDrive::Low) && !wire[0] {
                        rises += 1;
                    }
                    let hold = [false, hold_sda(rises)];
                    let mut changed = Vec::new();
                    for (k, pad) in [SCL_PAD, SDA_PAD].into_iter().enumerate() {
                        let level = !hold[k] && !matches!(drives[k], PadDrive::Low);
                        if level != wire[k] {
                            wire[k] = level;
                            edges.push((at.0, pad, level));
                            changed.push((pad, level));
                        }
                    }
                    chip.input(at, changed);
                }
                Some(Halt::Change { .. }) => {}
                Some(halt) => return Err(halt),
                None => break,
            }
        }
    }
    Ok(edges)
}

fn one_byte(chip: &mut Esp32s3) {
    arm(chip);
    poke(chip, DATA, 0xa5);
    poke(chip, COMD0, rstart());
    poke(chip, COMD0 + 4, write_cmd(1));
    poke(chip, COMD0 + 8, stop_cmd());
}

/// SCL releases so far, then whether the target holds SDA. Write-address ACK, the repeated
/// start's address ACK, and the read byte `0xA5`.
fn read_target(rises: u32) -> bool {
    if rises == 9 || rises == 19 {
        return true;
    }
    if (20..=27).contains(&rises) {
        let bit = rises - 20;
        return 0xa5u8 & (1 << (7 - bit)) == 0;
    }
    false
}

fn write_read(chip: &mut Esp32s3) {
    arm(chip);
    poke(chip, DATA, 0x50);
    poke(chip, DATA, 0x51);
    poke(chip, INT_ENA, TRANS_COMPLETE | NACK);
    let commands = [
        rstart(),
        write_cmd(1),
        rstart(),
        write_cmd(1),
        read_cmd(1, true),
        stop_cmd(),
    ];
    for (k, command) in commands.into_iter().enumerate() {
        poke(chip, COMD0 + 4 * k as u32, command);
    }
}

fn deltas(edges: &[(u64, u8, bool)]) -> Vec<(u64, u8, bool)> {
    let t0 = edges[0].0;
    edges
        .iter()
        .map(|(t, pad, level)| (t - t0, *pad, *level))
        .collect()
}

fn scl_edges(edges: &[(u64, u8, bool)]) -> Vec<(u64, bool)> {
    deltas(edges)
        .into_iter()
        .filter(|&(_, pad, _)| pad == SCL_PAD)
        .map(|(t, _, level)| (t, level))
        .collect()
}

/// One written byte `0xA5`, acknowledged, then a stop. Ticks are from the first SDA fall.
fn write_byte_edges() -> Vec<(u64, u8, bool)> {
    let mut edges = vec![(0, SDA_PAD, false), (cycles(SETUP), SCL_PAD, false)];
    let mut t = SETUP;
    let mut sda = false;
    let want = [true, false, true, false, false, true, false, true, true];
    for (i, bit) in want.into_iter().enumerate() {
        if bit != sda {
            sda = bit;
            edges.push((cycles(t + SDA_HOLD_LEN), SDA_PAD, sda));
        }
        let rise = t + LOW;
        edges.push((cycles(rise), SCL_PAD, true));
        if i + 1 == want.len() {
            edges.push((cycles(rise), SDA_PAD, false));
            edges.push((cycles(rise + HIGH_SIDE), SCL_PAD, false));
            edges.push((cycles(rise + HIGH_SIDE + LOW), SCL_PAD, true));
            edges.push((cycles(rise + HIGH_SIDE + LOW + SETUP), SDA_PAD, true));
            break;
        }
        t = rise + HIGH_SIDE;
        edges.push((cycles(t), SCL_PAD, false));
    }
    edges
}

/// The wire a master's moves make with `target` on SDA, in cycles of the I2C clock.
struct Wire {
    /// Whether the target holds SDA after a count of SCL releases, as `play` takes it.
    target: fn(u32) -> bool,
    edges: Vec<(u64, u8, bool)>,
    scl: bool,
    sda: bool,
    master_sda: bool,
    rises: u32,
}

impl Wire {
    /// The master puts `level` on `pad` at cycle `t`, and SDA reads what the target leaves it.
    fn set(&mut self, t: u64, pad: u8, level: bool) {
        if pad == SCL_PAD && level != self.scl {
            self.rises += u32::from(level);
            self.scl = level;
            self.edges.push((cycles(t), SCL_PAD, level));
        }
        if pad == SDA_PAD {
            self.master_sda = level;
        }
        let sda = self.master_sda && !(self.target)(self.rises);
        if sda != self.sda {
            self.sda = sda;
            self.edges.push((cycles(t), SDA_PAD, sda));
        }
    }

    /// A clock carrying `bit` from SCL's fall at `fall`, to the fall after its high side.
    fn clock(&mut self, fall: u64, bit: bool) -> u64 {
        let rise = self.release(fall, bit);
        self.set(rise + HIGH_SIDE, SCL_PAD, false);
        rise + HIGH_SIDE
    }

    /// The low side of a clock carrying `bit` from SCL's fall at `fall`, to its release.
    fn release(&mut self, fall: u64, bit: bool) -> u64 {
        if bit != self.master_sda {
            self.set(fall + SDA_HOLD_LEN, SDA_PAD, bit);
        }
        self.set(fall + LOW, SCL_PAD, true);
        fall + LOW
    }

    /// A byte and its ACK clock, from SCL's fall at `fall`. A read's byte is `0xFF`, released.
    fn byte(&mut self, fall: u64, byte: u8) -> u64 {
        let fall = (0..8).fold(fall, |fall, bit| {
            self.clock(fall, byte >> (7 - bit) & 1 != 0)
        });
        self.clock(fall, true)
    }

    /// A start on an idle bus, to SCL's fall.
    fn start(target: fn(u32) -> bool) -> (Wire, u64) {
        let mut wire = Wire {
            target,
            edges: Vec::new(),
            scl: true,
            sda: true,
            master_sda: true,
            rises: 0,
        };
        wire.set(0, SDA_PAD, false);
        wire.set(SETUP, SCL_PAD, false);
        (wire, SETUP)
    }

    /// The stop from SCL's fall at `fall`, and the wire it ends.
    fn stop(mut self, fall: u64) -> Vec<(u64, u8, bool)> {
        let rise = self.release(fall, false);
        self.set(rise + SETUP, SDA_PAD, true);
        self.edges
    }
}

/// The wire of a write of `byte` that no target acknowledges, from the first SDA fall.
fn nack_edges(byte: u8) -> Vec<(u64, u8, bool)> {
    let (mut wire, fall) = Wire::start(|_| false);
    let fall = wire.byte(fall, byte);
    wire.stop(fall)
}

/// The wire of `write_read` against `read_target`, from the first SDA fall.
fn write_read_edges() -> Vec<(u64, u8, bool)> {
    let (mut wire, fall) = Wire::start(read_target);
    let fall = wire.byte(fall, 0x50);
    let rise = wire.release(fall, true);
    wire.set(rise + SETUP, SDA_PAD, false);
    wire.set(rise + SETUP + SETUP, SCL_PAD, false);
    let fall = wire.byte(rise + SETUP + SETUP, 0x51);
    let fall = wire.byte(fall, 0xff);
    wire.stop(fall)
}

fn assert_low_and_high(edges: &[(u64, u8, bool)]) {
    let scl = scl_edges(edges);
    let falls = scl.iter().filter(|(_, level)| !*level).map(|(t, _)| *t);
    let mut rises = scl.iter().filter(|(_, level)| *level).map(|(t, _)| *t);
    let mut bit_highs = 0;
    let mut restarts = 0;
    let mut stops = 0;
    for fall in falls {
        let rise = rises.next().expect("a fall without a release");
        assert_eq!(rise - fall, cycles(LOW), "SCL low at {fall}");
        let next_fall = scl
            .iter()
            .find(|(t, level)| *t > rise && !*level)
            .map(|(t, _)| *t);
        match next_fall.map(|t| t - rise) {
            Some(delta) if delta == cycles(HIGH_SIDE) => bit_highs += 1,
            Some(delta) if delta == cycles(SETUP + SETUP) => restarts += 1,
            Some(delta) => panic!("high side {delta} at {rise}"),
            None => {
                let sda = deltas(edges)
                    .into_iter()
                    .find(|(t, pad, level)| *pad == SDA_PAD && *level && *t > rise)
                    .map(|(t, ..)| t);
                assert_eq!(sda, Some(rise + cycles(SETUP)), "stop setup");
                stops += 1;
            }
        }
    }
    assert_eq!(rises.next(), None, "a release with no low before it");
    assert!(
        bit_highs > 0 && stops == 1,
        "highs {bit_highs}, restarts {restarts}, stops {stops}"
    );
}

fn refused(halt: Option<Halt>) -> (&'static str, &'static str) {
    match halt {
        Some(Halt::Fault(_, Fault::Refused(Refused::Register(block, setting), _))) => {
            (block, setting)
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn run_refusal(chip: &mut Esp32s3) -> (&'static str, &'static str) {
    let mut seen = None;
    for _ in 0..8 {
        match chip.run_until(c(4000)) {
            Some(Halt::Moved(_)) => {}
            halt => {
                seen = Some(halt);
                break;
            }
        }
    }
    refused(seen.expect("a halt"))
}

#[test]
fn a_write_read_list_makes_the_waveform_of_its_registers() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    write_read(&mut chip);
    let edges = play(&mut chip, c(4000).0, c(4000).0, read_target).unwrap();
    assert_low_and_high(&edges);
    let scl = scl_edges(&edges);
    let restart = scl
        .iter()
        .filter(|(_, level)| *level)
        .find(|(rise, _)| {
            scl.iter()
                .any(|(t, level)| *t > *rise && !*level && *t - *rise == cycles(SETUP + SETUP))
        })
        .map(|(t, _)| *t)
        .expect("a repeated start");
    let sda_fall = deltas(&edges)
        .into_iter()
        .find(|(t, pad, level)| *pad == SDA_PAD && !*level && *t > restart)
        .map(|(t, ..)| t)
        .expect("SDA falls for the repeated start");
    assert_eq!(sda_fall - restart, cycles(SETUP));
    assert_eq!(word(&mut chip, DATA), 0xa5, "the byte the target drove");
    assert_eq!(
        word(&mut chip, INT_RAW) & (TRANS_COMPLETE | NACK),
        TRANS_COMPLETE
    );
    assert_eq!(word(&mut chip, SR) & BUS_BUSY, 0);
    assert!(word(&mut chip, COMD0) & CMD_DONE != 0);
}

#[test]
fn the_masters_own_release_starts_the_high_side_at_its_landing() {
    let run = |step: u64, ack: bool| {
        let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
        one_byte(&mut chip);
        play(&mut chip, c(4000).0, step, |rises| ack && rises == 9)
    };
    let edges = run(c(4000).0, true).unwrap();
    assert_eq!(deltas(&edges), write_byte_edges());
    for step in [1, 2, 3, 7] {
        assert_eq!(run(step, true).unwrap(), edges, "steps of {step} ticks");
    }
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    one_byte(&mut chip);
    // The fall's landing leaves SCL low. Withholding the release's landing is what stretches.
    let mut fell = false;
    let release = loop {
        match chip.run_until(c(4000)) {
            Some(Halt::Moved(at)) => {
                if matches!(chip.drive(SCL_PAD), PadDrive::Low) {
                    fell = true;
                    chip.input(at, [(SCL_PAD, false)]);
                } else if fell {
                    break at;
                }
            }
            other => panic!("before the release: {other:?}"),
        }
    };
    let halt = chip.run_until(Tick(release.0 + cycles(1 << 10) + cycles(LOW)));
    assert_eq!(
        refused(halt),
        ("I2C0", "timeout"),
        "withheld input at {release:?}"
    );
    match halt {
        Some(Halt::Fault(at, _)) => assert_eq!(at.0, release.0 + cycles(1 << 10)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_sample_reads_an_input_landed_before_its_tick_and_not_one_at_it() {
    let sample = |early: bool| {
        let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
        one_byte(&mut chip);
        let mut rises = 0u32;
        let mut wire_scl = true;
        let ack = loop {
            match chip.run_until(c(4000)) {
                Some(Halt::Moved(at)) => {
                    let scl = chip.drive(SCL_PAD);
                    let sda = chip.drive(SDA_PAD);
                    if !matches!(scl, PadDrive::Low) && !wire_scl {
                        rises += 1;
                    }
                    wire_scl = !matches!(scl, PadDrive::Low);
                    let mut pads = Vec::new();
                    if wire_scl {
                        pads.push((SCL_PAD, true));
                    } else {
                        pads.push((SCL_PAD, false));
                    }
                    let sda_level = !matches!(sda, PadDrive::Low);
                    pads.push((SDA_PAD, sda_level));
                    chip.input(at, pads);
                    if rises == 9 {
                        break at;
                    }
                }
                other => panic!("{other:?}"),
            }
        };
        let sample = Tick(ack.0 + cycles(SDA_SAMPLE_LEN));
        // An input at the sample tick arrives after the sample event. One a tick earlier is in it.
        if early {
            chip.input(Tick(sample.0 - 1), [(SDA_PAD, false)]);
        }
        let during = chip.run_until(sample);
        assert!(during.is_none(), "the sample is not a halt: {during:?}");
        if !early {
            chip.input(sample, [(SDA_PAD, false)]);
        }
        let rest = play(&mut chip, c(4000).0, c(4000).0, |_| false);
        (rest, word(&mut chip, INT_RAW))
    };
    let (early, raw) = sample(true);
    assert!(early.is_ok(), "{early:?}");
    assert_eq!(raw & (TRANS_COMPLETE | NACK), TRANS_COMPLETE);
    let (late, raw) = sample(false);
    assert!(late.is_ok(), "{late:?}");
    assert_eq!(raw & NACK, NACK);
    assert_eq!(raw & TRANS_COMPLETE, 0);
}

#[test]
fn a_nack_sets_the_bit_and_stops_the_bus() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    one_byte(&mut chip);
    let edges = play(&mut chip, c(4000).0, c(4000).0, |_| false).unwrap();
    assert_eq!(word(&mut chip, INT_RAW) & NACK, NACK);
    assert_eq!(word(&mut chip, INT_RAW) & TRANS_COMPLETE, 0);
    assert_eq!(word(&mut chip, SR) & BUS_BUSY, 0);
    assert_eq!(
        word(&mut chip, COMD0 + 8) & CMD_DONE,
        0,
        "the STOP command never runs"
    );
    let scl = scl_edges(&edges);
    let last = scl
        .iter()
        .rfind(|(_, level)| *level)
        .map(|(t, _)| *t)
        .unwrap();
    let sda = deltas(&edges)
        .into_iter()
        .find(|(t, pad, level)| *pad == SDA_PAD && *level && *t >= last)
        .map(|(t, ..)| t);
    assert_eq!(
        sda,
        Some(last + cycles(SETUP)),
        "the hardware stop still releases SDA"
    );
}

#[test]
fn a_held_scl_stretches_the_high_side_from_its_release() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    one_byte(&mut chip);
    let mut wire_scl = true;
    let mut rises = 0u32;
    let release = loop {
        match chip.run_until(c(4000)) {
            Some(Halt::Moved(at)) => {
                let scl = chip.drive(SCL_PAD);
                let sda = chip.drive(SDA_PAD);
                let releasing = !matches!(scl, PadDrive::Low) && !wire_scl;
                if releasing {
                    rises += 1;
                }
                wire_scl = !matches!(scl, PadDrive::Low);
                if rises == 1 && releasing {
                    chip.input(at, [(SDA_PAD, !matches!(sda, PadDrive::Low))]);
                    break at;
                }
                let level = !matches!(scl, PadDrive::Low);
                chip.input(
                    at,
                    [(SCL_PAD, level), (SDA_PAD, !matches!(sda, PadDrive::Low))],
                );
            }
            other => panic!("{other:?}"),
        }
    };
    let h = Tick(release.0 + cycles(80));
    chip.input(h, [(SCL_PAD, true)]);
    let edges = play(&mut chip, c(4000).0, c(4000).0, |n| n == 9).unwrap();
    let fall = edges
        .iter()
        .find(|(t, pad, level)| *pad == SCL_PAD && !*level && *t > h.0)
        .map(|(t, ..)| *t)
        .expect("SCL falls after the stretch");
    assert_eq!(fall, h.0 + cycles(HIGH));
}

#[test]
fn a_stretch_past_the_deadline_is_refused_as_timeout() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    one_byte(&mut chip);
    let mut fell = false;
    let release = loop {
        match chip.run_until(c(4000)) {
            Some(Halt::Moved(at)) => {
                if matches!(chip.drive(SCL_PAD), PadDrive::Low) {
                    fell = true;
                    chip.input(at, [(SCL_PAD, false)]);
                } else if fell {
                    break at;
                }
            }
            other => panic!("{other:?}"),
        }
    };
    chip.input(Tick(release.0 + cycles(10)), [(SCL_PAD, false)]);
    let halt = chip.run_until(Tick(release.0 + cycles(1 << 10) + c(1).0));
    assert_eq!(refused(halt), ("I2C0", "timeout"));
    match halt {
        Some(Halt::Fault(at, _)) => assert_eq!(at.0, release.0 + cycles(1 << 10)),
        other => panic!("{other:?}"),
    }
}

/// A deadline that a load of `I2C_INT_RAW_REG` runs past ends the run as `timeout` at the
/// deadline, as one the chip settles on its own does.
#[test]
fn a_timeout_met_by_a_status_load_ends_the_run() {
    let mut program = Program::new(&[I2C0, GO]);
    program.l32r(2, 0).l32r(3, 1).rri8(6, 3, 2, CTR as u8);
    wait_bit(&mut program, INT_RAW, 7, true);
    program.raw(&LOOP);
    let mut chip = program.chip_on(&[SCL_PAD, SDA_PAD]);
    one_byte(&mut chip);
    let mut fell = false;
    let release = loop {
        match chip.run_until(c(4000)) {
            Some(Halt::Moved(at)) => {
                if matches!(chip.drive(SCL_PAD), PadDrive::Low) {
                    fell = true;
                    chip.input(at, [(SCL_PAD, false)]);
                } else if fell {
                    break at;
                }
            }
            other => panic!("{other:?}"),
        }
    };
    chip.input(Tick(release.0 + cycles(10)), [(SCL_PAD, false)]);
    let halt = chip.run_until(Tick(release.0 + cycles(1 << 11)));
    assert_eq!(refused(halt), ("I2C0", "timeout"));
    match halt {
        Some(Halt::Fault(at, _)) => assert_eq!(at.0, release.0 + cycles(1 << 10)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn trans_complete_raises_source_42_on_the_mapped_core() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    one_byte(&mut chip);
    poke(&mut chip, INT_ENA, TRANS_COMPLETE);
    play(&mut chip, c(4000).0, c(4000).0, |n| n == 9).unwrap();
    let status = chip
        .soc
        .bus()
        .load(
            orbe_xtensa::Cycle(0),
            0x600c_2000 + 0x190,
            orbe_xtensa::Width::Word,
        )
        .unwrap();
    assert_eq!(status & (1 << 10), 1 << 10, "source 42");
}

/// A chip that starts `one_byte`, takes source 42 on interrupt 1 and runs `wait`, whose
/// handler enables GPIO21 and raises it.
fn interrupt_chip(wait: &[u8]) -> Esp32s3 {
    const VECTORS: u32 = BASE + 0x2000;
    let mut handler = Program::at(VECTORS + 0x300 - 8, &[GPIO, GPIO21]);
    handler
        .l32r(2, 0)
        .l32r(3, 1)
        .rri8(6, 3, 2, 0x24)
        .rri8(6, 3, 2, 0x08)
        .raw(&LOOP);
    let mut program = Program::new(&[I2C0, GO, VECTORS]);
    program
        .l32r(2, 0)
        .l32r(3, 1)
        .rri8(6, 3, 2, CTR as u8)
        .l32r(5, 2)
        .wsr(5, 231)
        .movi(6, 2)
        .wsr(6, 228)
        .movi(6, 0)
        .wsr(6, 230)
        .raw(&[0x10, 0x20, 0x00])
        .raw(wait)
        .raw(&LOOP);
    let handler_image = handler.image();
    let pro_image = program.image();
    let mut chip = Esp32s3::new(
        &[SCL_PAD, SDA_PAD, 21],
        program.entry(),
        [
            (BASE, pro_image.as_slice()),
            (handler.base, handler_image.as_slice()),
        ],
        None,
    );
    one_byte(&mut chip);
    poke(&mut chip, INT_ENA, TRANS_COMPLETE);
    chip.soc.restore(0x600c_2000 + 4 * 42, 1);
    chip
}

/// The handler's two changes with GPIO21's drive after each, and the tick of the last wire
/// change, taking limits from `step` through tick `through`.
fn handler_drives(chip: &mut Esp32s3, through: u64, step: u64) -> (Vec<(Halt, PadDrive)>, u64) {
    let mut wire = [true, true];
    let mut rises = 0u32;
    let mut last = 0u64;
    let mut drives = Vec::new();
    let mut limit = 0u64;
    while drives.len() < 2 {
        assert!(limit < through, "the handler ran by tick {through}");
        limit = (limit + step).min(through);
        while let Some(halt) = chip.run_until(Tick(limit)) {
            match halt {
                Halt::Moved(at) => {
                    let drives_now = [chip.drive(SCL_PAD), chip.drive(SDA_PAD)];
                    if !matches!(drives_now[0], PadDrive::Low) && !wire[0] {
                        rises += 1;
                    }
                    let hold_sda = rises == 9;
                    let mut changed = Vec::new();
                    for (k, pad) in [SCL_PAD, SDA_PAD].into_iter().enumerate() {
                        let level =
                            !(k == 1 && hold_sda) && !matches!(drives_now[k], PadDrive::Low);
                        if level != wire[k] {
                            wire[k] = level;
                            last = at.0;
                            changed.push((pad, level));
                        }
                    }
                    chip.input(at, changed);
                }
                Halt::Change { .. } => drives.push((halt, chip.drive(21))),
                other => panic!("{other:?}"),
            }
        }
    }
    (drives, last)
}

#[test]
fn trans_complete_wakes_a_parked_core_at_its_tick() {
    let mut chip = interrupt_chip(&WAITI);
    let (drives, last) = handler_drives(&mut chip, c(8000).0, c(8000).0);
    let done = last + cycles(SETUP);
    let woke = chip
        .soc
        .clock
        .tick(chip.soc.clock.cycle_after(Tick(done + 1)));
    assert_eq!(
        drives,
        [
            (image_at(Tick(woke.0 + 6 * c(1).0)), PadDrive::Low),
            (image_at(Tick(woke.0 + 7 * c(1).0)), PadDrive::High),
        ]
    );
}

#[test]
fn trans_complete_interrupts_a_spinning_core_at_its_tick() {
    let whole = handler_drives(&mut interrupt_chip(&NOP), c(8000).0, c(8000).0);
    let (drives, last) = &whole;
    // The jump that starts on the event's cycle runs before the event, so the core takes the
    // interrupt where that jump ends, three cycles on.
    let taken = last + cycles(SETUP) + cycles(3);
    assert_eq!(
        drives,
        &[
            (image_at(Tick(taken + cycles(6))), PadDrive::Low),
            (image_at(Tick(taken + cycles(7))), PadDrive::High),
        ]
    );
    for step in [1, 2, 3, 7, 11] {
        let split = handler_drives(&mut interrupt_chip(&NOP), c(8000).0, step);
        assert_eq!(split, whole, "steps of {step} ticks");
    }
}

#[test]
fn timing_words_take_effect_at_conf_upgate() {
    let low_ticks = |reg: u32, upgate: bool| {
        let mut program = Program::new(&[I2C0, MODE | TRANS_START]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, CTR as u8)
            .raw(&LOOP);
        let mut chip = program.chip_on(&[SCL_PAD, SDA_PAD]);
        arm(&mut chip);
        poke(&mut chip, DATA, 0xa5);
        poke(&mut chip, COMD0, rstart());
        poke(&mut chip, COMD0 + 4, write_cmd(1));
        poke(&mut chip, COMD0 + 8, stop_cmd());
        poke(&mut chip, SCL_LOW, reg);
        if upgate {
            poke(&mut chip, CTR, MODE | CONF_UPGATE);
        }
        let edges = play(&mut chip, c(8000).0, c(8000).0, |n| n == 9).unwrap();
        let scl = scl_edges(&edges);
        let fall = scl.iter().find(|(_, level)| !*level).unwrap().0;
        let rise = scl.iter().find(|(t, level)| *level && *t > fall).unwrap().0;
        rise - fall
    };
    assert_eq!(low_ticks(99, false), cycles(LOW));
    assert_eq!(low_ticks(99, true), cycles(100));
}

#[test]
fn a_stop_command_on_a_released_bus_raises_trans_complete_with_no_edge() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    arm(&mut chip);
    poke(&mut chip, COMD0, stop_cmd());
    poke(&mut chip, INT_ENA, TRANS_COMPLETE);
    let edges = play(&mut chip, c(2000).0, c(2000).0, |_| false).unwrap();
    assert!(edges.is_empty(), "{edges:?}");
    assert_eq!(word(&mut chip, INT_RAW) & TRANS_COMPLETE, TRANS_COMPLETE);
    assert_eq!(word(&mut chip, SR) & BUS_BUSY, 0);
    assert!(word(&mut chip, COMD0) & CMD_DONE != 0);
}

#[test]
fn a_reserved_clk_en_stays_clear_and_the_transfer_runs() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    one_byte(&mut chip);
    assert_eq!(word(&mut chip, CTR) & (1 << 8), 0);
    let edges = play(&mut chip, c(4000).0, c(4000).0, |n| n == 9).unwrap();
    assert_eq!(deltas(&edges), write_byte_edges());
}

#[test]
fn a_split_run_makes_the_same_edges() {
    let once = |step: u64| {
        let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
        write_read(&mut chip);
        play(&mut chip, c(8000).0, step, read_target).unwrap()
    };
    let whole = once(c(8000).0);
    assert_eq!(deltas(&whole), write_read_edges());
    for step in [1, 2, 3, 7] {
        assert_eq!(once(step), whole, "steps of {step} ticks");
    }
}

/// A `TRANS_START` store that a setting refuses leaves I2C0 as it was, `I2C_CTR_REG` and the
/// timing that `CONF_UPGATE` in the same word would copy included.
#[test]
fn a_refused_trans_start_writes_nothing() {
    let refuse = |value: u32, setting: &str, prepare: fn(&mut Esp32s3)| {
        let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
        one_byte(&mut chip);
        poke(&mut chip, SCL_LOW, 99);
        prepare(&mut chip);
        let before = chip.soc.i2c.clone();
        let soc = &mut chip.soc;
        let refused = soc.i2c.write(Tick(0), CTR, value, &mut soc.gpio);
        assert_eq!(refused, Err(setting));
        assert!(chip.soc.i2c == before, "{setting} wrote to I2C0");
    };
    refuse(GO | 1 << 9, "arbitration", |_| {});
    refuse(GO, "timing", |chip| poke(chip, SDA_HOLD, 200));
    refuse(GO, "opcode", |chip| poke(chip, COMD0, write_cmd(1)));
    refuse(GO, "not_routed", |chip| {
        chip.soc.gpio.write(0x154 + 4 * SIG_SDA, 0).unwrap();
    });
}

/// A read command with its `ack_val` set sends a NACK after each byte, not only the last.
#[test]
fn a_read_with_its_nack_bit_nacks_every_byte() {
    let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
    arm(&mut chip);
    poke(&mut chip, DATA, 0x51);
    for (k, command) in [rstart(), write_cmd(1), read_cmd(2, true), stop_cmd()]
        .into_iter()
        .enumerate()
    {
        poke(&mut chip, COMD0 + 4 * k as u32, command);
    }
    let target = |rises| rises == 9;
    let edges = play(&mut chip, c(8000).0, c(8000).0, target).unwrap();
    let (mut wire, fall) = Wire::start(target);
    let fall = wire.byte(fall, 0x51);
    let fall = wire.byte(fall, 0xff);
    let fall = wire.byte(fall, 0xff);
    assert_eq!(deltas(&edges), wire.stop(fall));
}

/// Loads the word at `offset` of a2 into a4 until its bit `bit`, below 16, reads `level`.
fn wait_bit(program: &mut Program, offset: u32, bit: u8, level: bool) {
    let top = program.pc();
    program.rri8(2, 4, 2, offset as u8);
    let imm = top.wrapping_sub(program.pc() + 4) as u8;
    let op = if level { 0x6 } else { 0xe };
    program.raw(&[bit << 4 | 7, op << 4 | 4, imm]);
}

/// The stores IDF makes after a NACK on the S3, which has no `SOC_I2C_SUPPORT_HW_FSM_RST`, in
/// its order: the interrupt disable that ends the failed `s_i2c_transaction_start`, then
/// `s_i2c_hw_fsm_reset`, then the next `s_i2c_transaction_start` and its commands for
/// `write_read`. Each bitfield write is a store of the word it leaves. The reset puts every word
/// at its power-on value.
fn reset_and_rerun() -> Vec<(u32, u32)> {
    const FIFO_RESET: u32 = 0x408b;
    let high = (WAIT_HIGH << 9 | HIGH) as u32;
    let setup = (SETUP - 1) as u32;
    let sda = [
        (SDA_HOLD, (SDA_HOLD_LEN - 1) as u32),
        (SDA_SAMPLE, (SDA_SAMPLE_LEN - 1) as u32),
    ];
    let fifo_resets = [
        (FIFO_CONF, FIFO_RESET | 1 << 13),
        (FIFO_CONF, FIFO_RESET),
        (FIFO_CONF, FIFO_RESET | 1 << 12),
        (FIFO_CONF, FIFO_RESET),
    ];
    let mut i2c = Vec::new();
    i2c.extend([
        (CTR, 0x21b),
        (CTR, 0x21b),
        (CTR, 0x01b),
        (CTR, MODE),
        (CTR, MODE),
    ]);
    i2c.extend(fifo_resets);
    i2c.extend([
        (CLK_CONF, SCLK_ACTIVE),
        (FIFO_CONF, FIFO_RESET),
        (INT_ENA, 0),
        (INT_CLR, 0x3fff),
    ]);
    i2c.extend([
        (SCL_LOW, (LOW - 1) as u32),
        (SCL_HIGH, HIGH as u32),
        (SCL_HIGH, high),
        (SCL_RSTART_SETUP, setup),
        (SCL_START_HOLD, setup),
        (SCL_STOP_SETUP, setup),
        (SCL_STOP_HOLD, setup),
    ]);
    i2c.extend(sda);
    i2c.push((TO, 10));
    i2c.extend([(CLK_CONF, SCLK_ACTIVE); 5]);
    i2c.push((FILTER_CFG, 0x300 | 7));
    i2c.extend([(FILTER_CFG, FILTER); 3]);
    i2c.extend([
        (CLK_CONF, SCLK_ACTIVE),
        (SCL_LOW, (LOW - 1) as u32),
        (SCL_HIGH, high),
        (SCL_HIGH, high),
    ]);
    i2c.extend(sda);
    i2c.extend([
        (SCL_RSTART_SETUP, setup),
        (SCL_STOP_SETUP, setup),
        (SCL_START_HOLD, setup),
        (SCL_STOP_HOLD, setup),
        (TO, 10),
        (TO, 10 | 1 << 5),
    ]);
    // `I2C_LL_SCL_WAIT_US_VAL_DEFAULT`, 2000 us of the 40 MHz clock, is under 2^17 cycles.
    i2c.push((TO, 17 | 1 << 5));
    i2c.extend([(CLK_CONF, SCLK_ACTIVE); 2]);
    i2c.push((CTR, MODE | CONF_UPGATE));
    i2c.extend(fifo_resets);
    i2c.push((INT_ENA, 1 << 3 | 1 << 5 | TRANS_COMPLETE | 1 << 8 | NACK));
    i2c.extend([(DATA, 0x50), (DATA, 0x51)]);
    let commands = [
        rstart(),
        write_cmd(1),
        rstart(),
        write_cmd(1),
        read_cmd(1, true),
        stop_cmd(),
    ];
    i2c.extend(
        (0..)
            .zip(commands)
            .map(|(k, command)| (COMD0 + 4 * k, command)),
    );
    i2c.extend([(CTR, MODE | CONF_UPGATE), (CTR, MODE | TRANS_START)]);
    let mut stores = vec![
        (I2C0 + INT_ENA, 0),
        (SYSTEM + 0x20, 1 << 7),
        (SYSTEM + 0x20, 0),
    ];
    stores.extend(
        i2c.into_iter()
            .map(|(offset, value)| (I2C0 + offset, value)),
    );
    stores
}

/// After a NACK, IDF's driver resets I2C0 through `SYSTEM_PERIP_RST_EN0`, sets it up again,
/// and the next transfer runs to its `TRANS_COMPLETE`.
#[test]
fn a_nack_then_idfs_peripheral_reset_and_setup_runs_the_next_transfer() {
    let stores = reset_and_rerun();
    let mut literals = vec![I2C0, GO];
    literals.extend(stores.iter().flat_map(|&(addr, value)| [addr, value]));
    let mut program = Program::new(&literals);
    // The filter counts the idle lines from power-on, so the start waits out its 7 cycles.
    for _ in 0..8 {
        program.raw(&NOP);
    }
    program.l32r(2, 0).l32r(3, 1).rri8(6, 3, 2, CTR as u8);
    wait_bit(&mut program, INT_RAW, 10, true);
    wait_bit(&mut program, SR, 4, false);
    for k in 0..stores.len() {
        program
            .l32r(5, 2 + 2 * k)
            .l32r(6, 3 + 2 * k)
            .rri8(6, 6, 5, 0);
    }
    program.raw(&LOOP);
    let mut chip = program.chip_on(&[SCL_PAD, SDA_PAD]);
    arm(&mut chip);
    poke(&mut chip, FILTER_CFG, FILTER);
    poke(&mut chip, DATA, 0xa0);
    poke(&mut chip, COMD0, rstart());
    poke(&mut chip, COMD0 + 4, write_cmd(1));
    poke(&mut chip, COMD0 + 8, stop_cmd());
    let through = c(20_000).0;
    let edges = play(&mut chip, through, through, |n| {
        n > 10 && read_target(n - 10)
    })
    .unwrap();
    let nack = nack_edges(0xa0);
    assert!(edges.len() > nack.len(), "{edges:?}");
    let (first, second) = edges.split_at(nack.len());
    assert_eq!(deltas(first), nack);
    assert_eq!(deltas(second), write_read_edges());
    assert_eq!(
        word(&mut chip, INT_RAW) & (TRANS_COMPLETE | NACK),
        TRANS_COMPLETE
    );
    assert_eq!(word(&mut chip, DATA), 0xa5, "the byte the target drove");
    assert_eq!(word(&mut chip, SR) & BUS_BUSY, 0);
}

#[test]
fn each_refusal_names_its_setting() {
    fn ready(chip: &mut Esp32s3) {
        arm(chip);
        poke(chip, DATA, 0xa5);
        poke(chip, COMD0, rstart());
        poke(chip, COMD0 + 4, write_cmd(1));
        poke(chip, COMD0 + 8, stop_cmd());
    }
    let case = |program: Program, setting: &str, prepare: fn(&mut Esp32s3)| {
        let mut chip = program.chip_on(&[SCL_PAD, SDA_PAD]);
        prepare(&mut chip);
        assert_eq!(run_refusal(&mut chip), ("I2C0", setting), "{setting}");
    };
    let plain = |value: u32, offset: u8, setting: &str, prepare: fn(&mut Esp32s3)| {
        let mut program = Program::new(&[I2C0, value]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, offset)
            .raw(&LOOP);
        case(program, setting, prepare);
    };

    {
        let mut program = Program::new(&[I2C1, 0]);
        program.l32r(2, 0).l32r(3, 1).rri8(6, 3, 2, 0).raw(&LOOP);
        let mut chip = program.chip_on(&[SCL_PAD, SDA_PAD]);
        assert_eq!(run_refusal(&mut chip), ("I2C1", "port"));
    }

    plain(
        SDA_FORCE_OUT | SCL_FORCE_OUT | CONF_UPGATE | TRANS_START,
        CTR as u8,
        "slave_mode",
        ready,
    );
    plain(
        SCL_FORCE_OUT | MS_MODE | CONF_UPGATE | TRANS_START,
        CTR as u8,
        "push_pull",
        ready,
    );
    plain(GO, CTR as u8, "not_routed", |chip| {
        clock_on(chip);
        chip.input(Tick(0), [(SCL_PAD, true), (SDA_PAD, true)]);
    });
    plain(GO | 1 << 6, CTR as u8, "lsb_first", ready);
    plain(GO | 1 << 9, CTR as u8, "arbitration", ready);
    plain(GO | 1 << 2, CTR as u8, "sample_scl_level", ready);
    plain(GO, CTR as u8, "clock_off", |chip| {
        ready(chip);
        poke(chip, CLK_CONF, 0);
    });
    plain(
        SCLK_ACTIVE | 1 << 20,
        CLK_CONF as u8,
        "rc_fast_clock",
        ready,
    );
    plain(
        SCLK_ACTIVE | 1 << 8,
        CLK_CONF as u8,
        "fractional_divider",
        ready,
    );
    plain(0x408b | 1 << 10, FIFO_CONF as u8, "nonfifo", ready);
    plain(GO | 1 << 10, CTR as u8, "fsm_reset", ready);
    plain(1, SCL_SP_CONF as u8, "clear_bus", ready);
    plain(1 << 6, SCL_SP_CONF as u8, "power_down", ready);
    plain(1, INT_ENA as u8, "int_ena", ready);

    plain(GO, CTR as u8, "end_command", |chip| {
        ready(chip);
        poke(chip, COMD0, 4 << 11);
    });
    plain(GO, CTR as u8, "opcode", |chip| {
        ready(chip);
        poke(chip, COMD0, 0);
    });
    plain(GO, CTR as u8, "opcode", |chip| {
        ready(chip);
        poke(chip, COMD0 + 4, write_cmd(0));
    });
    plain(GO, CTR as u8, "tx_underflow", |chip| {
        arm(chip);
        poke(chip, COMD0, rstart());
        poke(chip, COMD0 + 4, write_cmd(1));
        poke(chip, COMD0 + 8, stop_cmd());
    });
    plain(0xff, DATA as u8, "tx_overflow", |chip| {
        arm(chip);
        for _ in 0..32 {
            poke(chip, DATA, 0);
        }
    });

    {
        let mut program = Program::new(&[I2C0]);
        program.l32r(2, 0).rri8(2, 3, 2, DATA as u8).raw(&LOOP);
        case(program, "rx_underflow", ready);
    }

    plain(MODE | CONF_UPGATE, CTR as u8, "busy_write", |chip| {
        ready(chip);
        poke(chip, CTR, GO);
    });
    plain(GO, CTR as u8, "timing", |chip| {
        ready(chip);
        poke(chip, SDA_HOLD, LOW as u32);
        poke(chip, CTR, MODE | CONF_UPGATE);
    });

    // A gate store while the command list runs.
    for (value, offset, setting) in [
        (0u32, 0x18u8, "reset_while_sending"),
        (1 << 7, 0x20, "reset_while_sending"),
    ] {
        let mut program = Program::new(&[I2C0, GO, SYSTEM, value]);
        program
            .l32r(2, 0)
            .l32r(3, 1)
            .rri8(6, 3, 2, CTR as u8)
            .l32r(4, 2)
            .l32r(5, 3)
            .rri8(6, 5, 4, offset)
            .raw(&LOOP);
        case(program, setting, ready);
    }

    {
        let mut program = Program::new(&[I2C0, 0]);
        program.l32r(2, 0).l32r(3, 1).rri8(6, 3, 2, 0).raw(&LOOP);
        let mut chip = program.chip_on(&[SCL_PAD, SDA_PAD]);
        assert_eq!(run_refusal(&mut chip), ("I2C0", "clock_off"));
    }
    {
        let mut program = Program::new(&[I2C0, 0]);
        program.l32r(2, 0).l32r(3, 1).rri8(6, 3, 2, 0).raw(&LOOP);
        let mut chip = program.chip_on(&[SCL_PAD, SDA_PAD]);
        clock_on(&mut chip);
        chip.soc.restore(SYSTEM + 0x20, 1 << 7);
        assert_eq!(run_refusal(&mut chip), ("I2C0", "held_in_reset"));
    }

    // A pulse shorter than the filter threshold, while the master holds the bus.
    {
        let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
        ready(&mut chip);
        poke(&mut chip, FILTER_CFG, FILTER);
        let mut fell = false;
        let release = loop {
            match chip.run_until(c(4000)) {
                Some(Halt::Moved(at)) => {
                    if matches!(chip.drive(SCL_PAD), PadDrive::Low) {
                        fell = true;
                    } else if fell {
                        break at;
                    }
                }
                other => panic!("{other:?}"),
            }
        };
        let first = Tick(release.0 + c(1).0);
        let second = Tick(first.0 + cycles(6));
        chip.input(first, [(SDA_PAD, false)]);
        chip.input(second, [(SDA_PAD, true)]);
        assert_eq!(refused(chip.run_until(c(4000))), ("I2C0", "glitch"));
    }

    // A read of 33 bytes fills the FIFO on the 33rd byte.
    {
        let mut chip = go(CTR as u8).chip_on(&[SCL_PAD, SDA_PAD]);
        arm(&mut chip);
        poke(&mut chip, DATA, 0x51);
        poke(&mut chip, COMD0, rstart());
        poke(&mut chip, COMD0 + 4, write_cmd(1));
        poke(&mut chip, COMD0 + 8, read_cmd(33, true));
        poke(&mut chip, COMD0 + 12, stop_cmd());
        let halt = play(&mut chip, c(200_000).0, c(200_000).0, |rises| rises == 9);
        assert_eq!(refused(halt.err()), ("I2C0", "rx_overflow"));
    }
}
