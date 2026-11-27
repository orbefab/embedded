//! Whole-chip tests for a chip built with its Wi-Fi radio: the radio's words are kept and read
//! back, a chip without it refuses them as before, the PHY's write of `SENS_IOMUX_CLK_EN` is not
//! listed, a frame the MAC sends ends in a record halt, and the run ends where an armed TBTT
//! would raise the power interrupt, even at the store that armed it, or not at all once a
//! store disarms it, however the run is split.

use super::*;

const FE_IQ_EST: u32 = 0x6000_6174;
const MAC: u32 = 0x6003_3000;
const CLK_GATE: u32 = RTC_CNTL + 0x800 + 0x104;
const IOMUX_CLK_EN: u32 = 1 << 31;
const SARADC_CLK_EN: u32 = 1 << 30;

/// Stores `value` at `address`, then loads it back into `a5`.
fn store_and_load(address: u32, value: u32) -> Program {
    let mut program = Program::new(&[address, value]);
    program
        .l32r(2, 0)
        .l32r(3, 1)
        .rri8(6, 3, 2, 0)
        .rri8(2, 5, 2, 0)
        .raw(&LOOP);
    program
}

#[test]
fn a_chip_with_its_radio_keeps_the_radio_words() {
    let mut chip = store_and_load(MAC, 0x1234_5678).chip();
    chip.use_radio();
    assert_eq!(chip.run_until(c(1000)), None);
    assert_eq!(chip.unmodeled().count(), 0);
    let mut bus = chip.soc.bus();
    assert_eq!(bus.load(Cycle(0), MAC, Width::Word), Ok(0x1234_5678));
    assert_eq!(bus.load(Cycle(0), FE_IQ_EST, Width::Word), Ok(1 << 16));
}

#[test]
fn a_chip_without_its_radio_refuses_the_radio_words() {
    let mut program = Program::new(&[MAC, 1]);
    program.l32r(2, 0).l32r(3, 1);
    let store = program.pc();
    program.rri8(6, 3, 2, 0).raw(&LOOP);
    let mut chip = program.chip();
    assert_eq!(
        chip.run_until(c(1000)),
        Some(Halt::Fault(
            c(2),
            Fault::Refused(
                Refused::Unsimulated("RADIO", "the Wi-Fi and Bluetooth radio"),
                pro(store)
            )
        ))
    );
}

#[test]
fn a_word_past_the_radio_words_is_refused_on_a_chip_with_its_radio() {
    let mut program = Program::new(&[MAC + 0xffc]);
    program.l32r(2, 0);
    let load = program.pc();
    program.rri8(2, 3, 2, 0).raw(&LOOP);
    let mut chip = program.chip();
    chip.use_radio();
    assert_eq!(
        chip.run_until(c(1000)),
        Some(Halt::Fault(
            c(1),
            Fault::Refused(
                Refused::Unsimulated("RADIO", "the Wi-Fi and Bluetooth radio"),
                pro(load)
            )
        ))
    );
}

/// Setting `SENS_IOMUX_CLK_EN` is listed on a chip without its radio, as before, and not on one
/// with it. Clearing it again is listed on both.
#[test]
fn only_a_chip_with_its_radio_sets_the_rtc_io_clock_unlisted() {
    for radio in [false, true] {
        let mut program = Program::new(&[CLK_GATE, IOMUX_CLK_EN | SARADC_CLK_EN, SARADC_CLK_EN]);
        program.l32r(2, 0).l32r(3, 1).l32r(4, 2);
        let set = program.pc();
        program.rri8(6, 3, 2, 0);
        let clear = program.pc();
        program.rri8(6, 4, 2, 0).raw(&LOOP);
        let mut chip = program.chip();
        if radio {
            chip.use_radio();
        }
        assert_eq!(chip.run_until(c(1000)), None);
        let listed: Vec<u32> = chip.unmodeled().map(|(_, touch)| touch.pc.pc).collect();
        let want = if radio { vec![clear] } else { vec![set, clear] };
        assert_eq!(listed, want, "radio {radio}");
    }
}

/// The RF PLL's word, the start pulse on channel 1, and queue 0's PLCP1 and PLCP0 words.
const FREQ: u32 = 0x6000_e0c4;
const PLCP1: u32 = 0x6003_42f8;
const PLCP0: u32 = 0x6003_3d08;
const DESCRIPTOR: u32 = 0x3fcb_3a58;
const BUFFER: u32 = 0x3fcb_3a88;

/// Tunes channel 1, then sends a 20-octet frame to everyone from queue 0 at 1 Mb/s.
fn send() -> Esp32s3 {
    let plcp0 = 0xc000_0000 | DESCRIPTOR & 0xf_ffff;
    let mut program = Program::new(&[FREQ, (1 << 8) | (2 * 12), PLCP1, 24, PLCP0, plcp0]);
    for k in 0..3 {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
    }
    program.raw(&LOOP);
    let mut chip = program.chip();
    chip.use_radio();
    let head: u32 = 1 << 31 | 1 << 30 | 24 << 12 | 24;
    let mut descriptor = Vec::new();
    for word in [head, BUFFER, 0] {
        descriptor.extend(word.to_le_bytes());
    }
    chip.soc.poke(DESCRIPTOR, &descriptor);
    let mut frame = [0u8; 24];
    frame[..4].copy_from_slice(&[0x40, 0, 0, 0]);
    frame[4..10].fill(0xff);
    chip.soc.poke(BUFFER, &frame);
    chip
}

/// The frame ends in a record halt at its end however the run splits, and the chip hands it
/// over once there.
#[test]
fn a_frame_ends_in_a_record_halt_however_the_run_splits() {
    let run = |step: u64| {
        let mut chip = send();
        let mut seen = Vec::new();
        let mut limit = 0;
        while limit < 40_000 {
            limit = (limit + step).min(40_000);
            while let Some(halt) = chip.run_until(c(limit)) {
                seen.push((halt, chip.take_aired(), chip.take_aired()));
            }
        }
        assert_eq!(chip.unmodeled().count(), 0);
        seen
    };
    let seen = run(40_000);
    let [(halt, Some(ref frame), None)] = seen[..] else {
        panic!("one frame: {seen:?}");
    };
    // 192 us of preamble and header and 192 us for 24 octets with the FCS at 1 Mb/s.
    assert_eq!(frame.end.0 - frame.start.0, (192 + 192) * 480);
    assert_eq!(
        halt,
        Halt::Change {
            at: frame.end,
            image: false,
            record: true,
            sample: false
        }
    );
    assert_eq!(
        (frame.channel, frame.rate),
        (1, orbe_model::air::Rate::Dsss1)
    );
    assert_eq!(frame.mpdu.len(), 20);
    assert_eq!(
        frame.mpdu[..10],
        [0x40, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
    );
    for step in [1, 7, 333, 1_000] {
        assert_eq!(run(step), seen, "step {step}");
    }
}

/// The station's TBTT, its timing, the TSF control word, TBTT 0's control word and the power
/// interrupt's enable word.
const TBTT: u32 = 0x6003_5020;
const TBTT_TIMING: u32 = 0x6003_5030;
const TSF_CONTROL: u32 = 0x6003_500c;
const TBTT_CONTROL: u32 = 0x6003_5028;
const PWR_ENABLE: u32 = 0x6003_5110;
/// The timing of a TBTT every `tu` TU, 10 us ahead.
const fn every(tu: u32) -> u32 {
    10 << 16 | tu
}
/// The passes of the count down between the arming and the late store.
const PASSES: u32 = 3_000;
/// Where the TBTT every TU from TSF 0 would first raise the power interrupt: 10 us before
/// 1024 us.
const TBTT_AT: u64 = 1014 * 40;

/// Sets a TBTT at TSF 0 with `timing` and latches it, as `hal_set_sta_tbtt` does, then enables
/// TBTT 0 and its interrupt, as `hal_enable_sta_tbtt` does. It counts `PASSES` down, makes the
/// store `late` if any, and then counts up without end, so no fetch spins.
fn armed_then(timing: u32, late: Option<(u32, u32)>) -> Esp32s3 {
    let mut words = vec![
        TBTT_TIMING,
        timing,
        TBTT,
        0,
        TSF_CONTROL,
        1 << 8,
        TBTT_CONTROL,
        1 << 26,
        PWR_ENABLE,
        1 << 4,
        PASSES,
    ];
    words.extend(late.into_iter().flat_map(|(at, value)| [at, value]));
    let mut program = Program::new(&words);
    for k in 0..5 {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).s32i(3, 2, 0);
    }
    program.l32r(4, 10);
    let down = program.pc();
    program.addi(4, 4, -1).bnez(4, down);
    if late.is_some() {
        program.l32r(2, 11).l32r(3, 12).s32i(3, 2, 0);
    }
    let up = program.pc();
    program.addi(5, 5, 1).j(up);
    let mut chip = program.chip();
    chip.use_radio();
    chip
}

/// A run of [`armed_then`]: its name, its chip, and where it ends as `tbtt`, if it does.
struct TbttRun {
    name: &'static str,
    chip: fn() -> Esp32s3,
    ends: Option<Tick>,
}

/// The three runs: armed to the end, disarmed by the late store, and re-timed by it from 2 TU
/// to 1 TU.
fn tbtt_runs() -> [TbttRun; 3] {
    [
        TbttRun {
            name: "armed",
            chip: || armed_then(every(1), None),
            ends: Some(c(TBTT_AT)),
        },
        TbttRun {
            name: "disarmed",
            chip: || armed_then(every(1), Some((PWR_ENABLE, 0))),
            ends: None,
        },
        TbttRun {
            name: "retimed",
            chip: || armed_then(every(2), Some((TBTT_TIMING, every(1)))),
            ends: Some(c(TBTT_AT)),
        },
    ]
}

/// The run's limit, two TUs.
const TBTT_LIMIT: u64 = 80_000;

/// Every halt of a run to `TBTT_LIMIT` cycles, stepped through each limit of `splits` first.
/// Each halt comes from the first call whose limit reaches it.
fn halts(mut chip: Esp32s3, splits: &[u64]) -> Vec<Halt> {
    let mut seen = Vec::new();
    let mut served = None;
    for &limit in splits.iter().chain([&TBTT_LIMIT]) {
        while let Some(halt) = chip.run_until(c(limit)) {
            assert!(
                served.is_none_or(|served| halt.at() > c(served)),
                "{halt:?} after a call to {served:?} returned"
            );
            seen.push(halt);
        }
        served = Some(limit);
    }
    assert_eq!(chip.unmodeled().count(), 0);
    seen
}

/// The cycles at which the radio's TBTT deadline changes, stepping one cycle at a time to
/// `until`.
fn tbtt_moves(mut chip: Esp32s3, until: u64) -> Vec<u64> {
    let mut moves = Vec::new();
    let mut last = None;
    for cycle in 1..=until {
        chip.run_until(c(cycle));
        let now = chip.soc.radio().and_then(wifi::Radio::tbtt);
        if now != last {
            moves.push(cycle);
            last = now;
        }
    }
    moves
}

/// The one-shot run ends as `tbtt` where the TBTT would first raise the power interrupt, or not
/// at all when the late store disarmed it, although the program never spins, so nothing but the
/// deadline stops the core there.
#[test]
fn a_straight_run_ends_at_the_tbtt_it_armed_and_not_at_one_it_disarmed() {
    for TbttRun { name, chip, ends } in tbtt_runs() {
        let seen = halts(chip(), &[]);
        match (ends, &seen[..]) {
            (None, []) => {}
            (Some(at), [Halt::Fault(fault_at, Fault::Refused(refused, _))]) => {
                assert_eq!(
                    (*fault_at, *refused),
                    (at, Refused::Register("RADIO", "tbtt"))
                );
            }
            _ => panic!("{name}: {seen:?}"),
        }
    }
}

/// The run ends the same way in 1 000-cycle chunks, in two chunks split before the late store
/// and in three split around it.
#[test]
fn a_chunked_run_ends_as_the_straight_run_does() {
    for TbttRun { name, chip, .. } in tbtt_runs() {
        let straight = halts(chip(), &[]);
        let chunks: Vec<u64> = (1..TBTT_LIMIT / 1_000).map(|k| k * 1_000).collect();
        assert_eq!(
            halts(chip(), &chunks),
            straight,
            "{name}: 1 000-cycle chunks"
        );
        assert_eq!(halts(chip(), &[5_000]), straight, "{name}: split at 5 000");
        assert_eq!(
            halts(chip(), &[5_000, 20_000]),
            straight,
            "{name}: split at 5 000 and 20 000"
        );
    }
}

/// Every split gives the straight run's halts: each cycle around the arming, the late store
/// and the deadline, and every 997th cycle, alone and in pairs.
#[test]
fn every_split_of_a_tbtt_run_ends_as_the_straight_run_does() {
    for TbttRun { name, chip, .. } in tbtt_runs() {
        let straight = halts(chip(), &[]);
        let moves = tbtt_moves(chip(), 20_000);
        // The arming, then the late store between the chunked test's splits at 5 000 and
        // 20 000.
        assert!(moves[0] < 5_000, "{name}: {moves:?}");
        assert!(
            moves[1..].iter().all(|at| (5_000..20_000).contains(at)) && moves.len() <= 2,
            "{name}: {moves:?}"
        );
        let mut splits: Vec<u64> = (1..TBTT_LIMIT / 997).map(|k| k * 997).collect();
        for at in moves.iter().copied().chain([TBTT_AT]) {
            splits.extend(at.saturating_sub(12)..at + 12);
        }
        splits.sort_unstable();
        splits.dedup();
        for &split in &splits {
            assert_eq!(
                halts(chip(), &[split]),
                straight,
                "{name}: split at {split}"
            );
        }
        for &first in &moves {
            for &second in &moves {
                for (a, b) in [
                    (first - 1, second + 1),
                    (first, second),
                    (first + 1, TBTT_AT),
                ] {
                    if a < b {
                        assert_eq!(
                            halts(chip(), &[a, b]),
                            straight,
                            "{name}: splits at {a} and {b}"
                        );
                    }
                }
            }
        }
    }
}

/// With no ahead time, a TBTT every TU from TSF 0 armed at TSF 0 falls due at the arming store
/// itself. The call whose limit reaches the store ends the run there, at every split around it.
#[test]
fn a_tbtt_due_at_its_arming_store_ends_the_call_that_ran_it() {
    let chip = || armed_then(1, None);
    let arming = tbtt_moves(chip(), 100)[0];
    let straight = halts(chip(), &[]);
    let [Halt::Fault(at, Fault::Refused(refused, _))] = straight[..] else {
        panic!("{straight:?}");
    };
    assert_eq!(
        (at, refused),
        (c(arming), Refused::Register("RADIO", "tbtt"))
    );
    for split in 1..arming + 12 {
        assert_eq!(halts(chip(), &[split]), straight, "split at {split}");
    }
}

/// The TBTTs fall at the TSF's whole TUs, so the first one the arming lets through is at
/// 1024 us, and its interrupt would come 10 us before. The run ends there by name, however it
/// steps. Disarmed, it runs on.
#[test]
fn the_run_ends_where_an_armed_tbtt_would_raise_the_power_interrupt() {
    let run = |step: u64| {
        let mut chip = armed_then(every(1), None);
        let mut limit = 0;
        loop {
            limit += step;
            if let Some(halt) = chip.run_until(c(limit)) {
                return halt;
            }
        }
    };
    let halt = run(100_000);
    let Halt::Fault(at, Fault::Refused(refused, _)) = halt else {
        panic!("{halt:?}");
    };
    assert_eq!(at, c(TBTT_AT));
    assert_eq!(refused, Refused::Register("RADIO", "tbtt"));
    for step in [1_000, 7_777, 40_559, 40_560] {
        assert_eq!(run(step), halt, "step {step}");
    }

    let mut chip = armed_then(every(1), Some((PWR_ENABLE, 0)));
    assert_eq!(chip.run_until(c(100_000)), None);
}
