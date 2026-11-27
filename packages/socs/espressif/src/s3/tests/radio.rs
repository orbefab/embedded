//! Whole-chip tests for a chip built with its Wi-Fi radio: the radio's words are kept and read
//! back, a chip without it refuses them as before, and the PHY's write of `SENS_IOMUX_CLK_EN` is
//! not listed.

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
