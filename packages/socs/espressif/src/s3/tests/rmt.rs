//! TX channel 0 as the core drives it: gates, the pad, records and the poll horizon.

use orbe_xtensa::{Bus, Cycle, Width};

use super::super::soc::{GPIO, IO_MUX, RMT, SYSTEM};
use super::*;

const CLK_ON: u32 = 0xf9c1_e06f | 1 << 9;
const RST: u32 = 1 << 9;
const SYSCLK: u32 = SYSTEM + 0x60;
const MAP: u32 = 0x600c_2000 + 4 * 40;

fn gate() -> [(u32, u32); 3] {
    [
        (SYSTEM + 0x18, CLK_ON),
        (SYSTEM + 0x20, RST),
        (SYSTEM + 0x20, 0),
    ]
}

fn route() -> [(u32, u32); 2] {
    [(IO_MUX + 4, 1 << 12), (GPIO + 0x554, 81)]
}

fn symbol(d0: u32, level0: bool, d1: u32, level1: bool) -> u32 {
    d0 | u32::from(level0) << 15 | d1 << 16 | u32::from(level1) << 31
}

/// `TX_START`, wrap, idle out, `div_cnt` and one memory block.
fn conf(div: u32, idle: bool) -> u32 {
    1 | 1 << 4 | 1 << 6 | div << 8 | 1 << 16 | u32::from(idle) << 5
}

fn sys(div_num: u32) -> u32 {
    1 | 1 << 24 | 1 << 26 | div_num << 4
}

fn load_of(addr: u32) -> (Program, u32) {
    let mut program = Program::new(&[addr]);
    program.l32r(2, 0);
    let pc = program.pc();
    program.rri8(2, 4, 2, 0).raw(&LOOP);
    (program, pc)
}

fn refused(setting: &'static str, at: Tick, pc: u32) -> Halt {
    Halt::Fault(
        at,
        Fault::Refused(Refused::Register("RMT", setting), pro(pc)),
    )
}

fn drives(pairs: &[(u32, u32)], through: u64) -> Vec<(Halt, PadDrive)> {
    let mut chip = stores(pairs).chip_on(&[0]);
    let mut seen = Vec::new();
    while let Some(halt) = chip.run_until(c(through)) {
        seen.push((halt, chip.drive(0)));
    }
    seen
}

#[test]
fn a_power_on_load_or_store_of_rmt_ends_clock_off() {
    for addr in [RMT + 0x20, RMT + 0x800] {
        let (program, pc) = load_of(addr);
        assert_eq!(
            program.chip_on(&[]).run_until(c(8)),
            Some(refused("clock_off", c(1), pc)),
            "{addr:#x}"
        );
    }
    let program = stores(&[(RMT + 0x800, 1)]);
    assert_eq!(
        program.chip_on(&[]).run_until(c(8)),
        Some(refused("clock_off", c(2), program.entry() + 6))
    );
}

#[test]
fn a_fifo_load_and_store_are_refused() {
    let mut pairs = gate().to_vec();
    pairs.push((RMT, 1));
    let stored = stores(&pairs);
    let mut chip = stored.chip_on(&[]);
    let halt = chip.run_until(c(40)).expect("the store");
    let Halt::Fault(_, why) = halt else {
        panic!("{halt:?}");
    };
    assert_eq!(
        why,
        Fault::Refused(
            Refused::Register("RMT", "fifo"),
            store_pc(&stored, pairs.len() as u32 - 1)
        )
    );

    let mut program = Program::new(&[
        SYSTEM + 0x18,
        CLK_ON,
        SYSTEM + 0x20,
        RST,
        SYSTEM + 0x20,
        0,
        RMT,
    ]);
    for k in 0..3 {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
    }
    program.l32r(2, 6);
    let pc = program.pc();
    program.rri8(2, 4, 2, 0).raw(&LOOP);
    let mut chip = program.chip_on(&[]);
    assert_eq!(chip.run_until(c(40)), Some(refused("fifo", c(10), pc)));
}

#[test]
fn a_start_on_an_inverted_pad_ends_the_run() {
    let mut pairs = gate().to_vec();
    pairs.extend([
        (IO_MUX + 4, 1 << 12),
        (GPIO + 0x554, 81 | 1 << 9),
        (RMT + 0xc0, sys(0)),
        (RMT + 0x800, symbol(8, true, 8, false)),
        (RMT + 0x20, conf(1, false)),
    ]);
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[0]);
    let mut why = None;
    while let Some(halt) = chip.run_until(c(80)) {
        match halt {
            Halt::Fault(_, fault) => {
                why = Some(fault);
                break;
            }
            Halt::Change {
                image: true,
                record: false,
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        why,
        Some(Fault::Refused(
            Refused::Register("RMT", "inverted"),
            store_pc(&program, pairs.len() as u32 - 1)
        ))
    );
    assert_eq!(chip.load_word(RMT + 0x20).unwrap(), 0x0071_0200);
}

#[test]
fn a_load_while_the_reset_bit_holds_ends_held_in_reset() {
    let literals = [SYSTEM + 0x18, CLK_ON, SYSTEM + 0x20, RST, RMT + 0x20];
    let mut program = Program::new(&literals);
    program
        .l32r(2, 0)
        .l32r(3, 1)
        .rri8(6, 3, 2, 0)
        .l32r(2, 2)
        .l32r(3, 3)
        .rri8(6, 3, 2, 0)
        .l32r(2, 4);
    let load = program.pc();
    program.rri8(2, 4, 2, 0).raw(&LOOP);
    assert_eq!(
        program.chip_on(&[]).run_until(c(20)),
        Some(refused("held_in_reset", c(7), load))
    );
}

#[test]
fn an_idle_level_store_halts_with_the_new_image() {
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.push((RMT + 0x20, 1 << 6 | 1 << 5));
    let seen = drives(&pairs, 40);
    let high = seen
        .iter()
        .position(|(_, drive)| *drive == PadDrive::High)
        .expect("the pad goes high");
    assert!(
        matches!(
            seen[high].0,
            Halt::Change {
                image: true,
                record: false,
                ..
            }
        ),
        "{seen:?}"
    );
    assert!(seen.iter().any(|(_, drive)| *drive == PadDrive::Low));
    assert!(seen.iter().all(|(halt, _)| !matches!(halt, Halt::Moved(_))));
}

#[test]
fn a_reset_drives_the_pad_low_and_clears_the_block() {
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.extend([
        (RMT + 0x800, 0x1111_1111),
        (RMT + 0x20, 1 << 6 | 1 << 5),
        (SYSTEM + 0x20, RST),
        (SYSTEM + 0x20, 0),
    ]);
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[0]);
    let mut seen = Vec::new();
    while let Some(halt) = chip.run_until(c(80)) {
        assert!(
            matches!(
                halt,
                Halt::Change {
                    image: true,
                    record: false,
                    ..
                }
            ),
            "{halt:?}"
        );
        seen.push(chip.drive(0));
    }
    let high = seen
        .iter()
        .position(|drive| *drive == PadDrive::High)
        .unwrap();
    assert_eq!(seen.get(high + 1), Some(&PadDrive::Low), "{seen:?}");
    assert_eq!(chip.load_word(RMT + 0x20).unwrap(), 0x0071_0200);
    assert_eq!(chip.load_word(RMT + 0x70).unwrap(), 0);
    assert_eq!(chip.load_word(RMT + 0x800).unwrap(), 0);
}

/// With the gate open and no RMT store, each kept register reads the word `rmt_reg.h` resets it to.
#[test]
fn each_kept_register_reads_its_reset_word_with_the_gate_open() {
    let program = stores(&gate());
    let mut chip = program.chip_on(&[]);
    assert!(chip.run_until(c(40)).is_none());
    let conf = 0x0071_0200;
    let rx0 = 0x317f_ff02;
    let rx1 = 0x0000_01e8;
    let lim = 0x0000_0080;
    let sys = 0x0500_0010;
    let words = [
        (0x20, conf),
        (0x24, conf),
        (0x28, conf),
        (0x2c, conf),
        (0x30, rx0),
        (0x38, rx0),
        (0x40, rx0),
        (0x48, rx0),
        (0x34, rx1),
        (0x3c, rx1),
        (0x44, rx1),
        (0x4c, rx1),
        (0x70, 0),
        (0x74, 0),
        (0x78, 0),
        (0x7c, 0),
        (0x90, 0),
        (0x94, 0),
        (0x98, 0),
        (0x9c, 0),
        (0xa0, lim),
        (0xa4, lim),
        (0xa8, lim),
        (0xac, lim),
        (0xb0, lim),
        (0xb4, lim),
        (0xb8, lim),
        (0xbc, lim),
        (0xc0, sys),
        (0xc4, 0),
        (0xc8, 0),
        (0x800, 0),
    ];
    for (offset, word) in words {
        assert_eq!(
            chip.load_word(RMT + offset).unwrap(),
            word,
            "offset {offset:#x}"
        );
    }
}

#[test]
fn a_reset_or_clock_off_while_sending_writes_nothing() {
    let long = symbol(400, true, 400, false);
    for (last, setting) in [
        ((SYSTEM + 0x20, RST), "reset_while_sending"),
        ((SYSTEM + 0x18, 0xf9c1_e06f), "clock_off_while_sending"),
    ] {
        let mut pairs = gate().to_vec();
        pairs.extend(route());
        pairs.extend([
            (RMT + 0xc0, sys(0)),
            (RMT + 0x800, long),
            (RMT + 0x20, conf(1, false)),
            last,
        ]);
        let program = stores(&pairs);
        let k = pairs.len() as u32 - 1;
        let mut chip = program.chip_on(&[0]);
        let mut fault = None;
        while let Some(halt) = chip.run_until(c(80)) {
            match halt {
                Halt::Fault(_, why) => {
                    fault = Some(why);
                    break;
                }
                Halt::Change {
                    image: true,
                    record: false,
                    ..
                } => assert_eq!(chip.drive(0), PadDrive::Low),
                other => panic!("{setting}: {other:?}"),
            }
        }
        assert_eq!(
            fault,
            Some(Fault::Refused(
                Refused::Register("RMT", setting),
                store_pc(&program, k)
            ))
        );
        assert_eq!(chip.drive(0), PadDrive::Low);
        assert_eq!(chip.load_word(SYSTEM + 0x18).unwrap() & RST, RST);
        assert_eq!(chip.load_word(SYSTEM + 0x20).unwrap() & RST, 0);
    }
}

#[test]
fn a_clock_change_while_sending_writes_nothing() {
    let mut pairs = gate().to_vec();
    pairs.extend([
        (RMT + 0xc0, sys(0)),
        (RMT + 0x800, symbol(80, true, 80, false)),
        (RMT + 0x20, conf(1, false)),
        (SYSCLK, 1 << 10),
    ]);
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[]);
    let halt = chip.run_until(c(40)).expect("a fault");
    let Halt::Fault(at, why) = halt else {
        panic!("{halt:?}");
    };
    assert_eq!(
        why,
        Fault::Refused(
            Refused::Register("RMT", "clock_change_while_sending"),
            store_pc(&program, pairs.len() as u32 - 1)
        )
    );
    assert_eq!(chip.load_word(SYSCLK).unwrap(), 0);
    assert!(at < c(40));
}

#[test]
fn an_rmt_interrupt_enable_the_model_never_raises_ends_the_run() {
    for bit in [1 << 1, 1 << 4, 1 << 16] {
        let mut pairs = gate().to_vec();
        pairs.push((RMT + 0x78, bit));
        let program = stores(&pairs);
        let mut chip = program.chip_on(&[]);
        let halt = chip.run_until(c(40)).expect("a fault");
        let Halt::Fault(_, why) = halt else {
            panic!("{bit:#x}: {halt:?}");
        };
        assert_eq!(
            why,
            Fault::Refused(
                Refused::Interrupt("RMT"),
                store_pc(&program, pairs.len() as u32 - 1)
            ),
            "{bit:#x}"
        );
        assert_eq!(chip.load_word(RMT + 0x78).unwrap(), 0, "{bit:#x}");
    }
}

/// Channel 0's end, threshold and loop enables are the ones a transmit keeps.
#[test]
fn tx_end_threshold_and_loop_enables_are_kept() {
    let kept = 1 | 1 << 8 | 1 << 12;
    let mut pairs = gate().to_vec();
    pairs.push((RMT + 0x78, kept));
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[]);
    assert!(chip.run_until(c(40)).is_none());
    assert_eq!(chip.load_word(RMT + 0x78).unwrap(), kept);
}

#[test]
fn a_start_with_sclk_active_clear_ends_the_run() {
    let mut pairs = gate().to_vec();
    pairs.extend([
        (RMT + 0xc0, 1 | 1 << 24),
        (RMT + 0x800, symbol(8, true, 8, false)),
        (RMT + 0x20, conf(1, false)),
    ]);
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[]);
    let halt = chip.run_until(c(40)).expect("a fault");
    let Halt::Fault(_, why) = halt else {
        panic!("{halt:?}");
    };
    assert_eq!(
        why,
        Fault::Refused(
            Refused::Register("RMT", "sclk_active"),
            store_pc(&program, pairs.len() as u32 - 1)
        )
    );
    assert_eq!(chip.load_word(RMT + 0x20).unwrap(), 0x0071_0200);
}

#[test]
fn a_second_start_or_a_stop_while_sending_ends_the_run() {
    let long = symbol(80, true, 80, false);
    for (bits, setting) in [(1, "start_while_sending"), (1 << 7, "stop_while_sending")] {
        let mut pairs = gate().to_vec();
        pairs.extend([
            (RMT + 0xc0, sys(0)),
            (RMT + 0x800, long),
            (RMT + 0x20, conf(1, false)),
            (RMT + 0x20, bits),
        ]);
        let program = stores(&pairs);
        let mut chip = program.chip_on(&[]);
        let halt = chip.run_until(c(40)).expect("a fault");
        let Halt::Fault(_, why) = halt else {
            panic!("{halt:?}");
        };
        assert_eq!(
            why,
            Fault::Refused(
                Refused::Register("RMT", setting),
                store_pc(&program, pairs.len() as u32 - 1)
            )
        );
    }
}

#[test]
fn ten_entries_at_the_pll_record_50_ns_ticks() {
    let symbol = symbol(8, true, 4, false);
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.push((SYSCLK, 1 << 10));
    pairs.push((RMT + 0xc0, sys(1)));
    for index in 0..10 {
        pairs.push((RMT + 0x800 + 4 * index, symbol));
    }
    pairs.push((RMT + 0x800 + 40, 0));
    pairs.push((RMT + 0x20, conf(2, false)));
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[0]);
    let mut trains = Vec::new();
    while let Some(halt) = chip.run_until(c(20_000)) {
        match halt {
            Halt::Change { record: true, .. } => {
                assert_eq!(chip.drive(0), PadDrive::Low);
                trains.extend(chip.take_trains());
            }
            Halt::Change {
                image: true,
                record: false,
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }
    let train = trains.pop().expect("a record");
    assert!(trains.is_empty());
    let mut expect = Vec::new();
    for _ in 0..10 {
        expect.push((true, 400_000));
        expect.push((false, 200_000));
    }
    assert_eq!(train.pulses, expect);
    assert_eq!(train.pad, 0);
    assert!(!train.idle);
    assert_eq!(train.end.0 - train.start.0, 10 * 12 * 24);
}

/// Dividers stay at reset: `SCLK_DIV_NUM` 1 and `DIV_CNT` 2, so each tick is 50 ns on the PLL.
#[test]
fn ten_entries_at_the_reset_dividers_record_50_ns_ticks() {
    let symbol = symbol(1, true, 1, false);
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.push((SYSCLK, 1 << 10));
    // `APB_FIFO_MASK` only. The reset divider fields stay.
    pairs.push((RMT + 0xc0, 0x0500_0010 | 1));
    for index in 0..10 {
        pairs.push((RMT + 0x800 + 4 * index, symbol));
    }
    pairs.push((RMT + 0x800 + 40, 0));
    // Carrier off, wrap and idle out on. `DIV_CNT` stays 2.
    pairs.push((RMT + 0x20, 0x0051_0200 | 1 | 1 << 4 | 1 << 6));
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[0]);
    let mut trains = Vec::new();
    while let Some(halt) = chip.run_until(c(20_000)) {
        match halt {
            Halt::Change { record: true, .. } => {
                assert_eq!(chip.drive(0), PadDrive::Low);
                trains.extend(chip.take_trains());
            }
            Halt::Change {
                image: true,
                record: false,
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }
    let train = trains.pop().expect("a record");
    assert!(trains.is_empty());
    let expect = [(true, 50_000), (false, 50_000)];
    assert_eq!(train.pulses, expect.repeat(10));
    assert_eq!(train.pad, 0);
    assert!(!train.idle);
    assert_eq!(train.end.0 - train.start.0, 10 * 2 * 24);
}

#[test]
fn an_idle_high_pad_stays_high_through_a_low_pulse() {
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.extend([
        (RMT + 0xc0, sys(0)),
        (RMT + 0x800, symbol(4, true, 4, false)),
        (RMT + 0x804, 0),
        (RMT + 0x20, conf(1, true)),
    ]);
    let program = stores(&pairs);
    let mut chip = program.chip_on(&[0]);
    let mut train = None;
    while let Some(halt) = chip.run_until(c(400)) {
        match halt {
            Halt::Change { record: true, .. } => {
                assert_eq!(chip.drive(0), PadDrive::High);
                train = chip.take_trains().next();
            }
            Halt::Change {
                image: true,
                record: false,
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }
    let train = train.expect("a record");
    assert!(train.idle);
    assert_eq!(train.pulses.last().map(|pulse| pulse.0), Some(false));
    assert_eq!(chip.drive(0), PadDrive::High);
}

#[test]
fn a_skipped_poll_still_takes_the_threshold() {
    let (program, start) = threshold_program();
    let mut early = program.chip_on(&[]);
    assert_eq!(early.run_until(c(start + 5)), None);
    let now = early.cores.reached();
    let threshold = Cycle(start + 401);
    assert!(now < threshold);
    assert_eq!(early.soc.bus().next_event(now), Some(threshold));

    let mut chip = program.chip_on(&[]);
    let halt = chip.run_until(c(start + 500)).expect("the threshold");
    assert_eq!(
        halt,
        Halt::Fault(c(threshold.0 + 6), Fault::UnmappedFetch(pro(0x4000_0340))),
        "a jump that starts the cycle before the line rises pays two redirect cycles"
    );
}

fn threshold_program() -> (Program, u64) {
    let pairs = [
        gate().as_slice(),
        &[
            (RMT + 0xc0, sys(0)),
            (RMT + 0xa0, 1),
            (RMT + 0x800, symbol(400, true, 1, false)),
            (RMT + 0x78, 1 << 8),
            (MAP, 1),
        ],
    ]
    .concat();
    let mut literals: Vec<u32> = pairs.iter().flat_map(|&(a, v)| [a, v]).collect();
    literals.extend([RMT + 0x20, conf(1, false)]);
    let mut program = Program::new(&literals);
    for k in 0..pairs.len() {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
    }
    program.movi(7, 2).wsr(7, INTENABLE).movi(7, 32).wsr(7, PS);
    let start = program.code.len() as u64 / 3 + 2;
    let n = pairs.len();
    program
        .l32r(2, 2 * n)
        .l32r(3, 2 * n + 1)
        .rri8(6, 3, 2, 0)
        .raw(&LOOP);
    (program, start)
}

#[test]
fn a_cut_runs_the_refill_the_lagging_copy_has_not() {
    let valid = symbol(4, true, 4, false);
    let poison = symbol(80, true, 1, false);
    let mut pairs = gate().to_vec();
    pairs.push((RMT + 0xc0, sys(0)));
    pairs.push((RMT + 0xa0, 128));
    for index in 0..24 {
        pairs.push((RMT + 0x800 + 4 * index, valid));
    }
    for index in 24..48 {
        pairs.push((RMT + 0x800 + 4 * index, poison));
    }
    pairs.push((RMT + 0x20, conf(1, false)));
    let start_at = pairs.len();
    for index in 24..48 {
        pairs.push((RMT + 0x800 + 4 * index, valid));
    }
    let program = stores(&pairs);
    let start = 3 * start_at as u64 - 1;
    let mut chip = program.chip_on(&[]);
    assert_eq!(chip.run_until(c(start)), None);
    let at = c(start + 30 * 8 + 2);
    let flight = chip.in_flight(at).expect("in flight");
    assert!(
        flight.pulses.iter().all(|(_, ps)| *ps != 2_000_000),
        "{:?}",
        flight.pulses
    );
    assert!(flight.pulses.len() > 24 * 2, "{}", flight.pulses.len());
    assert_eq!(flight.pulses[0], (true, 100_000));
}

/// The last entry is a long low whose second half is 0. A GPIO store inside that low, and a
/// cut there, see one train whose halts stay in time order however the run is split.
fn zero_second_half() -> (Program, u64) {
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.push((RMT + 0xc0, sys(0)));
    pairs.push((RMT + 0x800, symbol(8, true, 8, false)));
    pairs.push((RMT + 0x804, symbol(800, false, 0, false)));
    pairs.push((RMT + 0x20, conf(1, false)));
    let start = 3 * (pairs.len() as u64 - 1) + 2;
    // Five stores land in the first entry. The next three land in the long low.
    for _ in 0..5 {
        pairs.push((RMT + 0xc8, 0));
    }
    pairs.push((GPIO + 0x24, 1 << 2));
    pairs.push((GPIO + 0x08, 1 << 2));
    pairs.push((GPIO + 0x0c, 1 << 2));
    (stores(&pairs), start)
}

#[test]
fn a_zero_second_half_split_around_a_gpio_store_stays_in_order() {
    let (program, start) = zero_second_half();
    let end = start + 16 + 800;
    let run = |step: u64| {
        let mut chip = program.chip_on(&[0, 2]);
        let mut seen = Vec::new();
        let mut limit = 0u64;
        while limit < end + 40 {
            limit = (limit + step).min(end + 40);
            while let Some(halt) = chip.run_until(c(limit)) {
                seen.push(halt);
            }
        }
        let trains: Vec<_> = chip.take_trains().collect();
        (seen, trains)
    };
    let whole = run(end + 40);
    let times: Vec<u64> = whole.0.iter().map(|halt| halt.at().0).collect();
    assert!(
        times.windows(2).all(|pair| pair[0] <= pair[1]),
        "halts go backwards: {times:?}"
    );
    let split = run(1);
    assert_eq!(split.0, whole.0);
    assert_eq!(split.1, whole.1);
    assert_eq!(whole.1.len(), 1);
    assert_eq!(whole.1[0].end, c(end));
    assert_eq!(whole.1[0].pulses.len(), 3);
    let inside = whole.0.iter().filter(|halt| {
        matches!(
            halt,
            Halt::Change {
                image: true,
                record: false,
                ..
            }
        ) && halt.at() > c(start + 16)
            && halt.at() < c(end)
    });
    assert!(inside.count() >= 2, "{:?}", whole.0);
}

#[test]
fn a_cut_inside_a_zero_second_half_keeps_the_pairs() {
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.push((RMT + 0xc0, sys(0)));
    pairs.push((RMT + 0x800, symbol(8, true, 8, false)));
    pairs.push((RMT + 0x804, symbol(800, false, 0, false)));
    pairs.push((RMT + 0x20, conf(1, false)));
    let program = stores(&pairs);
    let start = 3 * (pairs.len() as u64 - 1) + 2;
    let at = c(start + 40);
    let mut chip = program.chip_on(&[0]);
    while let Some(halt) = chip.run_until(at) {
        assert!(
            matches!(halt, Halt::Change { record: false, .. }),
            "{halt:?}"
        );
    }
    let flight = chip.in_flight(at).expect("the train is still running");
    assert_eq!(flight.pulses.len(), 3, "{:?}", flight.pulses);
    assert!(!flight.pulses[2].0);
    assert!(flight.pulses[2].1 > flight.pulses[0].1);
}

/// A train ends with `INT_ENA` clear, so the raw end bit is set and the line is low. The
/// core then enables that bit and waits. One run and a run split at every cycle take the
/// handler at the same tick.
fn int_ena_over_raw() -> (Program, Program) {
    const VECTORS: u32 = BASE + 0x2000;
    const MAP: u32 = 0x600c_2000 + 4 * 40;
    let mut handler = Program::at(VECTORS + 0x300 - 8, &[GPIO, GPIO21]);
    handler
        .l32r(2, 0)
        .l32r(3, 1)
        .rri8(6, 3, 2, 0x24)
        .rri8(6, 3, 2, 0x08)
        .raw(&LOOP);
    let pairs = [
        gate().as_slice(),
        &[
            (RMT + 0xc0, sys(0)),
            (RMT + 0x800, symbol(1, true, 1, false)),
            (RMT + 0x804, 0),
            (MAP, 1),
        ],
    ]
    .concat();
    let n = pairs.len();
    let mut literals: Vec<u32> = pairs
        .iter()
        .flat_map(|&(addr, value)| [addr, value])
        .collect();
    literals.extend([RMT + 0x20, conf(1, false), RMT + 0x78, 1, VECTORS]);
    let mut program = Program::new(&literals);
    for k in 0..n {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
    }
    program
        .l32r(5, 2 * n + 4)
        .wsr(5, 231)
        .movi(6, 2)
        .wsr(6, 228)
        .movi(6, 0)
        .wsr(6, 230)
        .raw(&[0x10, 0x20, 0x00])
        .l32r(2, 2 * n)
        .l32r(3, 2 * n + 1)
        .rri8(6, 3, 2, 0);
    for _ in 0..40 {
        program.raw(&[0xf0, 0x20, 0x00]);
    }
    program
        .l32r(2, 2 * n + 2)
        .l32r(3, 2 * n + 3)
        .rri8(6, 3, 2, 0)
        .raw(&WAITI)
        .raw(&LOOP);
    (program, handler)
}

fn int_ena_chip(program: &Program, handler: &Program) -> Esp32s3 {
    let pro = program.image();
    let isr = handler.image();
    Esp32s3::new(
        &[21],
        program.entry(),
        [(BASE, pro.as_slice()), (handler.base, isr.as_slice())],
        None,
    )
}

#[test]
fn an_int_ena_store_over_a_set_raw_bit_matches_a_run_split_at_every_cycle() {
    let (program, handler) = int_ena_over_raw();
    let run = |step: u64| {
        let mut chip = int_ena_chip(&program, &handler);
        let mut seen = Vec::new();
        let mut limit = 0u64;
        while limit < 400 {
            limit = (limit + step).min(400);
            while let Some(halt) = chip.run_until(c(limit)) {
                seen.push((halt, chip.drive(21)));
            }
        }
        seen
    };
    let whole = run(400);
    let split = run(1);
    assert!(
        split.iter().any(|(_, drive)| *drive == PadDrive::High),
        "a run split at every cycle raised GPIO21: {split:?}"
    );
    assert_eq!(whole, split);
}

#[test]
fn a_train_split_at_every_boundary_sends_the_same_record() {
    let mut pairs = gate().to_vec();
    pairs.extend(route());
    pairs.push((RMT + 0xc0, sys(0)));
    for index in 0..10 {
        pairs.push((RMT + 0x800 + 4 * index, symbol(2, true, 2, false)));
    }
    pairs.push((RMT + 0x800 + 40, 0));
    pairs.push((RMT + 0x20, conf(1, false)));
    let program = stores(&pairs);
    let run = |step: u64| {
        let mut chip = program.chip_on(&[0]);
        let mut seen = Vec::new();
        let mut limit = 0u64;
        while limit < 400 {
            limit = (limit + step).min(400);
            while let Some(halt) = chip.run_until(c(limit)) {
                seen.push(halt);
            }
        }
        let trains: Vec<_> = chip.take_trains().collect();
        (seen, trains)
    };
    let whole = run(400);
    assert_eq!(run(4).0, whole.0);
    assert_eq!(run(4).1, whole.1);
    assert_eq!(whole.1.len(), 1);
    assert_eq!(whole.1[0].pulses.len(), 20);
}

#[test]
fn a_train_with_no_end_word_ends_after_65536_entries() {
    let mut pairs = gate().to_vec();
    pairs.push((RMT + 0xc0, sys(0)));
    let word = symbol(1, true, 1, false);
    for index in 0..48 {
        pairs.push((RMT + 0x800 + 4 * index, word));
    }
    pairs.push((RMT + 0x20, conf(1, false)));
    let program = stores(&pairs);
    let after = program.entry() + 9 * (pairs.len() as u32 - 1) + 9;
    let mut chip = program.chip_on(&[]);
    let halt = chip.run_until(c(200_000)).expect("no end");
    assert_eq!(
        halt,
        Halt::Fault(
            c(3 * pairs.len() as u64 - 1 + 65_536 * 2),
            Fault::Refused(Refused::Register("RMT", "no_end"), pro(after))
        )
    );
}

#[test]
fn a_refill_that_keeps_the_threshold_does_not_rearm() {
    let (mut chip, cycle) = running();
    let before = chip.soc.rmt.next_event();
    chip.soc
        .bus()
        .store(
            cycle,
            RMT + 0x800 + 30 * 4,
            Width::Word,
            symbol(3, false, 3, true),
        )
        .unwrap();
    assert!(!chip.soc.rearmed);
    assert_eq!(chip.soc.rmt.next_event(), before);
}

#[test]
fn an_eof_that_moves_the_end_rearms() {
    let (mut chip, cycle) = running();
    let before = chip.soc.rmt.next_event().expect("a threshold");
    chip.soc
        .bus()
        .store(cycle, RMT + 0x800 + 10 * 4, Width::Word, 0)
        .unwrap();
    assert!(chip.soc.rearmed);
    let after = chip.soc.rmt.next_event().expect("the end");
    assert!(after < before);
}

fn running() -> (Esp32s3, Cycle) {
    let mut pairs = gate().to_vec();
    pairs.push((RMT + 0xc0, sys(0)));
    pairs.push((RMT + 0xa0, 24));
    pairs.push((RMT + 0x78, 1 << 8 | 1));
    let word = symbol(4, true, 4, false);
    for index in 0..48 {
        pairs.push((RMT + 0x800 + 4 * index, word));
    }
    pairs.push((RMT + 0x20, conf(1, false)));
    let program = stores(&pairs);
    let start = 3 * pairs.len() as u64 - 1;
    let mut chip = program.chip_on(&[]);
    assert_eq!(chip.run_until(c(start + 2)), None);
    let cycle = chip.cores.reached();
    (chip, cycle)
}
