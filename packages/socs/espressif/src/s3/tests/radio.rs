//! Whole-chip tests for a chip built with its Wi-Fi radio: the radio's words are kept and read
//! back, a chip without it refuses them as before, the PHY's write of `SENS_IOMUX_CLK_EN` is not
//! listed, a frame the MAC sends ends in a record halt, an armed TBTT raises the power interrupt
//! the core takes, even at the store that armed it, or not at all once a store disarms it, and
//! with its RF powered down for modem sleep the radio sends nothing until it wakes, however the
//! run is split.
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

/// `RTC_CNTL_ANA_CONF_REG`, as `rom_open_i2c_xpd` leaves it with the RF powered, and as
/// `rom_phy_xpd_rf` leaves it for modem sleep, with RFRX_PBUS and TXRF powered down.
const ANA_CONF: u32 = RTC_CNTL + 0x34;
const RF_ON: u32 = 0xf840_0000;
const RF_OFF: u32 = 0x2040_0000;
/// `RTC_CNTL_ANA_CONF_REG` as the boot leaves it, with the RF not yet powered.
const RF_BOOT: u32 = 0x0040_0000;

/// Powers the RF and tunes channel 1, then sends a 20-octet frame to everyone from queue 0 at
/// 1 Mb/s.
fn send() -> Esp32s3 {
    sent_after(&[])
}

/// Powers the RF and tunes channel 1, makes `between`'s stores with `PASSES` of a count down
/// after each, then sends the frame [`send`] sends.
fn sent_after(between: &[(u32, u32)]) -> Esp32s3 {
    let plcp0 = 0xc000_0000 | DESCRIPTOR & 0xf_ffff;
    let mut words = vec![ANA_CONF, RF_ON, FREQ, (1 << 8) | (2 * 12), PASSES];
    words.extend(between.iter().flat_map(|&(at, value)| [at, value]));
    words.extend([PLCP1, 24, PLCP0, plcp0]);
    let mut program = Program::new(&words);
    let store = |program: &mut Program, k: usize| {
        program.l32r(2, k).l32r(3, k + 1).rri8(6, 3, 2, 0);
    };
    store(&mut program, 0);
    store(&mut program, 2);
    for k in 0..between.len() {
        store(&mut program, 5 + 2 * k);
        program.l32r(4, 4);
        let down = program.pc();
        program.addi(4, 4, -1).bnez(4, down);
    }
    let sends = 5 + 2 * between.len();
    store(&mut program, sends);
    store(&mut program, sends + 2);
    program.raw(&LOOP);
    let mut chip = program.chip();
    chip.soc.restore(ANA_CONF, RF_BOOT);
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

/// Two unicast frames sent as one hardware TXOP burst (`0x6003_4314` count 1, slot at
/// `0x6003_437c`) produce two record halts separated by SIFS + ACK + SIFS, and set bits 28 to 31
/// of `TX_STATUS` to 1, identically however the run splits.
#[test]
fn a_txop_burst_ends_each_frame_in_a_record_halt_however_the_run_splits() {
    const TXOP_CTRL: u32 = 0x6003_4314;
    const TXOP_SLOT: u32 = 0x6003_437c;
    const TX_STATUS: u32 = 0x6003_4320;
    const DESC1: u32 = DESCRIPTOR + 16;
    const BUF1: u32 = BUFFER + 0x100;

    let build = || {
        let plcp0 = 0xc000_0000 | (DESCRIPTOR & 0xf_ffff);
        let words = [
            ANA_CONF,
            RF_ON,
            FREQ,
            (1 << 8) | (2 * 12),
            TXOP_SLOT,
            (28 << 20) | (DESC1 & 0xf_ffff),
            TXOP_CTRL,
            1 << 24,
            PLCP1,
            28,
            PLCP0,
            plcp0,
        ];
        let mut program = Program::new(&words);
        for k in 0..6 {
            program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
        }
        program.raw(&LOOP);
        let mut chip = program.chip();
        chip.soc.restore(ANA_CONF, RF_BOOT);
        chip.use_radio();
        let head: u32 = 1 << 31 | 1 << 30 | 28 << 12 | 28;
        for (d, b, seq) in [(DESCRIPTOR, BUFFER, 0x10u8), (DESC1, BUF1, 0x20u8)] {
            let mut desc = Vec::new();
            for word in [head, b, 0] {
                desc.extend(word.to_le_bytes());
            }
            chip.soc.poke(d, &desc);
            let mut frame = [0u8; 28];
            frame[..4].copy_from_slice(&[0x08, 0x01, 0, 0]);
            // Individual Address 1 so both frames wait for an ACK.
            frame[4..10].copy_from_slice(&[0x02, 0x6f, 0x72, 0x62, 0x65, 0x01]);
            frame[22] = seq;
            chip.soc.poke(b, &frame);
        }
        chip
    };

    let run = |step: u64| {
        let mut chip = build();
        let mut seen = Vec::new();
        let mut limit = 0;
        while limit < 120_000 {
            limit = (limit + step).min(120_000);
            while let Some(halt) = chip.run_until(c(limit)) {
                let frame = chip.take_aired();
                if frame.is_some() {
                    chip.acked(orbe_model::air::Ack::Acked);
                }
                seen.push((halt, frame));
            }
        }
        let status = chip.load_word(TX_STATUS).unwrap();
        assert_eq!(chip.unmodeled().count(), 0);
        (seen, status)
    };

    let (seen, status) = run(120_000);
    assert_eq!(status, 1 << 28, "completed count 1, TX_SUCCESS");
    assert_eq!(seen.len(), 2, "{seen:?}");
    let f0 = seen[0].1.as_ref().unwrap();
    let f1 = seen[1].1.as_ref().unwrap();
    assert_eq!(f0.mpdu[22], 0x10);
    assert_eq!(f1.mpdu[22], 0x20);
    // SIFS (10 us) + 1 Mb/s ACK (304 us) + SIFS (10 us) = 324 us between frames.
    assert_eq!(f1.start.0 - f0.end.0, 324 * 480);
    for step in [7, 333, 1_000, 10_000] {
        assert_eq!(run(step), (seen.clone(), status), "step {step}");
    }
}

/// The station's TBTT, its timing, the TSF control word, TBTT 0's control word and the power
/// interrupt's enable word.
const TBTT: u32 = 0x6003_5020;
const TBTT_TIMING: u32 = 0x6003_5030;
const TSF_CONTROL: u32 = 0x6003_500c;
const TBTT_CONTROL: u32 = 0x6003_5028;
const PWR_ENABLE: u32 = 0x6003_5110;
/// The interrupt matrix's word for the power interrupt's source, and the level-1 vector the
/// core jumps to when it takes the CPU interrupt 1 that word routes it to. Nothing is mapped
/// there, so the fetch ends the run where the core took it.
const PWR_ROUTE: u32 = 0x600c_2000 + 4 * wifi::PWR_SOURCE;
const VECTOR: u32 = 0x4000_0300;
/// The timing of a TBTT every `tu` TU, 10 us ahead.
const fn every(tu: u32) -> u32 {
    10 << 16 | tu
}
/// The passes of the count down between the arming and the late store.
const PASSES: u32 = 3_000;
/// Where the TBTT every TU from TSF 0 first raises the power interrupt: 10 us before 1024 us.
const TBTT_AT: u64 = 1014 * 40;
/// Where the core, counting up, then fetches the vector of the interrupt it takes.
const TAKEN_AT: u64 = TBTT_AT + 4;

/// Routes the power interrupt to CPU interrupt 1 and enables that at level 0, then sets a TBTT
/// at TSF 0 with `timing` and latches it, as `hal_set_sta_tbtt` does, and enables TBTT 0 and its
/// interrupt, as `hal_enable_sta_tbtt` does. It counts `PASSES` down, makes the store `late` if
/// any, and then counts up without end, so no fetch spins.
fn armed_then(timing: u32, late: Option<(u32, u32)>) -> Esp32s3 {
    let mut words = vec![
        PWR_ROUTE,
        1,
        TBTT_TIMING,
        timing,
        TBTT,
        0,
        TSF_CONTROL,
        1 << 8,
        TBTT_CONTROL,
        0x9c00_0000,
        PWR_ENABLE,
        1 << 4,
        PASSES,
    ];
    words.extend(late.into_iter().flat_map(|(at, value)| [at, value]));
    let mut program = Program::new(&words);
    program.movi(7, 2).wsr(7, INTENABLE).movi(7, 0).wsr(7, PS);
    for k in 0..6 {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).s32i(3, 2, 0);
    }
    program.l32r(4, 12);
    let down = program.pc();
    program.addi(4, 4, -1).bnez(4, down);
    if late.is_some() {
        program.l32r(2, 13).l32r(3, 14).s32i(3, 2, 0);
    }
    let up = program.pc();
    program.addi(5, 5, 1).j(up);
    let mut chip = program.chip();
    chip.use_radio();
    chip
}

/// Where the core took the power interrupt, if the run's halts are that one fetch of the
/// vector.
fn taken(seen: &[Halt]) -> Option<Tick> {
    match seen {
        [] => None,
        [Halt::Fault(at, Fault::UnmappedFetch(pc))] if *pc == pro(VECTOR) => Some(*at),
        _ => panic!("{seen:?}"),
    }
}

/// A run of [`armed_then`]: its name, its chip, and where the core takes the interrupt, if it
/// does.
struct TbttRun {
    name: &'static str,
    chip: fn() -> Esp32s3,
    taken: Option<Tick>,
}

/// The three runs: armed to the end, disarmed by the late store, and re-timed by it from 2 TU
/// to 1 TU.
fn tbtt_runs() -> [TbttRun; 3] {
    [
        TbttRun {
            name: "armed",
            chip: || armed_then(every(1), None),
            taken: Some(c(TAKEN_AT)),
        },
        TbttRun {
            name: "disarmed",
            chip: || armed_then(every(1), Some((PWR_ENABLE, 0))),
            taken: None,
        },
        TbttRun {
            name: "retimed",
            chip: || armed_then(every(2), Some((TBTT_TIMING, every(1)))),
            taken: Some(c(TAKEN_AT)),
        },
    ]
}

/// The run's limit, two TUs.
const TBTT_LIMIT: u64 = 80_000;

/// Every halt of a run to `limit` cycles, stepped through each limit of `splits` first. Each
/// halt comes from the first call whose limit reaches it.
fn halts_to(mut chip: Esp32s3, splits: &[u64], limit: u64) -> Vec<Halt> {
    let mut seen = Vec::new();
    let mut served = None;
    for &limit in splits.iter().chain([&limit]) {
        while let Some(halt) = chip.run_until(c(limit)) {
            assert!(
                served.is_none_or(|served| halt.at() > c(served)),
                "{halt:?} after a call to {served:?} returned"
            );
            seen.push(halt);
            if matches!(halt, Halt::Fault(..)) {
                return seen;
            }
        }
        served = Some(limit);
    }
    assert_eq!(chip.unmodeled().count(), 0);
    seen
}

/// [`halts_to`] `TBTT_LIMIT`.
fn halts(chip: Esp32s3, splits: &[u64]) -> Vec<Halt> {
    halts_to(chip, splits, TBTT_LIMIT)
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

/// The one-shot run ends where the core takes the power interrupt the TBTT raised, or not at
/// all when the late store disarmed it, although the program never spins, so nothing but the
/// interrupt stops the core there.
#[test]
fn a_straight_run_takes_the_interrupt_of_the_tbtt_it_armed_and_not_of_one_it_disarmed() {
    for TbttRun {
        name,
        chip,
        taken: at,
    } in tbtt_runs()
    {
        assert_eq!(taken(&halts(chip(), &[])), at, "{name}");
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
/// and the interrupt, and every 997th cycle, alone and in pairs.
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
/// itself, and the core fetches its interrupt's vector five cycles later, after the store and
/// the load that follows it, at every split around it.
#[test]
fn a_tbtt_due_at_its_arming_store_raises_the_interrupt_there() {
    let chip = || armed_then(1, None);
    let arming = tbtt_moves(chip(), 100)[0];
    let straight = halts(chip(), &[]);
    assert_eq!(taken(&straight), Some(c(arming + 5)));
    for split in 1..arming + 12 {
        assert_eq!(halts(chip(), &[split]), straight, "split at {split}");
    }
}

/// The TBTTs fall at the TSF's whole TUs, so the first one the arming lets through is at
/// 1024 us, and its interrupt comes 10 us before, however the run steps. Disarmed, it runs on.
#[test]
fn the_core_takes_the_power_interrupt_where_an_armed_tbtt_raises_it() {
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
    assert_eq!(taken(&[halt]), Some(c(TAKEN_AT)));
    for step in [1_000, 7_777, 40_559, 40_560, 40_561] {
        assert_eq!(run(step), halt, "step {step}");
    }

    let mut chip = armed_then(every(1), Some((PWR_ENABLE, 0)));
    assert_eq!(chip.run_until(c(100_000)), None);
}

/// Modem sleep as `ram_phy_close_rf` and `phy_wakeup_init` make it: `rom_phy_xpd_rf` powers
/// RFRX_PBUS and TXRF down, and `rom_open_i2c_xpd` powers them up again. Dozing, the radio is
/// tuned to no channel, and a frame it sends ends the run by name at its PLCP0 store. Awake
/// again, it is back on channel 1 and the frame ends in a record halt, however the run splits.
#[test]
fn a_dozing_radio_refuses_to_send_and_an_awake_one_sends_however_the_run_splits() {
    let limit = 60_000;
    for (name, between) in [
        ("dozing", &[(ANA_CONF, RF_OFF)][..]),
        ("woken", &[(ANA_CONF, RF_OFF), (ANA_CONF, RF_ON)][..]),
    ] {
        // Tuned once the PLL starts, then on no channel from the power-down, and on channel 1
        // again from the wake after the count.
        let mut chip = sent_after(between);
        let mut tuned = vec![(0, None)];
        for cycle in 1..limit {
            chip.run_until(c(cycle));
            let now = chip.soc.radio().and_then(wifi::Radio::tuned);
            if tuned.last().is_some_and(|&(_, last)| last != now) {
                tuned.push((cycle, now));
            }
        }
        let channels: Vec<Option<u8>> = tuned.iter().map(|&(_, channel)| channel).collect();
        let want = if between.len() == 1 {
            vec![None, Some(1), None]
        } else {
            vec![None, Some(1), None, Some(1)]
        };
        assert_eq!(channels, want, "{name}: {tuned:?}");
        assert!(tuned[2].0 < 100 && tuned.get(3).is_none_or(|&(at, _)| at > 9_000));
        let straight = halts_to(sent_after(between), &[], limit);
        match (name, &straight[..]) {
            ("dozing", [Halt::Fault(_, Fault::Refused(refused, _))]) => {
                assert_eq!(*refused, Refused::Register("RADIO", "tx_rf_off"));
            }
            ("woken", [Halt::Change { record: true, .. }]) => {}
            _ => panic!("{name}: {straight:?}"),
        }
        let at = straight[0].at().0 / XTAL;
        let mut splits: Vec<u64> = (1..limit / 997).map(|k| k * 997).collect();
        splits.extend(at.saturating_sub(12)..at + 12);
        for &(at, _) in &tuned[1..] {
            splits.extend(at.saturating_sub(12)..at + 12);
        }
        for split in splits {
            assert_eq!(
                halts_to(sent_after(between), &[split], limit),
                straight,
                "{name}: split at {split}"
            );
        }
    }
}
