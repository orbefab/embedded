//! Whole-chip tests for a chip built with its Wi-Fi radio: the radio's words are kept and read
//! back, a chip without it refuses them as before, the PHY's write of `SENS_IOMUX_CLK_EN` is not
//! listed, and a frame the MAC sends ends in a record halt.

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
