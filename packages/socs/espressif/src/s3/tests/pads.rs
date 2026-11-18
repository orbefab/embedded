//! A pad's input and its interrupt: the edge `GPIO_PINn` names latches into `GPIO_STATUS`, and
//! source 16 rises while an enabled pad's interrupt is pending.

use super::*;

const INTERRUPT_CORE0: u32 = 0x600c_2000;
/// `INTERRUPT_CORE0_INTR_STATUS_0_REG`, sources 0 to 31.
const SOURCES_0: u32 = INTERRUPT_CORE0 + 0x18c;
const GPIO_SOURCE: u32 = 1 << 16;
const PIN4: u32 = GPIO + 0x74 + 4 * 4;
const FALL: u32 = 2 << 7;
const INT_ENA: u32 = 1 << 13;

fn load(chip: &mut Esp32s3, address: u32) -> u32 {
    chip.soc.bus().load(Cycle(0), address, Width::Word).unwrap()
}

#[test]
fn a_falling_pad_input_sets_status_and_raises_source_16() {
    let mut chip = Program::new(&[]).raw(&LOOP).chip_on(&[4]);
    chip.soc
        .gpio
        .write_io_mux(0x4 + 4 * 4, 1 << 12 | 1 << 9)
        .unwrap();
    chip.input(Tick(0), [(4, true)]);
    chip.soc.gpio.write(PIN4 - GPIO, FALL | INT_ENA).unwrap();
    assert_eq!(load(&mut chip, SOURCES_0) & GPIO_SOURCE, 0);
    chip.input(Tick(0), [(4, true)]);
    assert_eq!(load(&mut chip, GPIO + 0x44), 0, "no edge");
    chip.input(Tick(0), [(4, false)]);
    assert_eq!(load(&mut chip, GPIO + 0x44), GPIO4);
    assert_eq!(load(&mut chip, GPIO + 0x5c), GPIO4);
    assert_eq!(load(&mut chip, SOURCES_0) & GPIO_SOURCE, GPIO_SOURCE);
    chip.soc
        .bus()
        .store(Cycle(0), GPIO + 0x4c, Width::Word, GPIO4)
        .unwrap();
    assert_eq!(load(&mut chip, GPIO + 0x44), 0);
    assert_eq!(load(&mut chip, SOURCES_0) & GPIO_SOURCE, 0);
}

const GPIO4: u32 = 1 << 4;

/// A store that sets any bit of `GPIO_PINn`'s `INT_ENA` above bit 0, the NMI enable among them,
/// ends the run with that setting's name and writes nothing.
#[test]
fn an_nmi_enable_is_refused_by_name() {
    for bit in 1..5 {
        let program = store_after_two_loads(PIN4, 1 << (13 + bit), 0);
        let mut chip = program.chip();
        assert_eq!(
            chip.run_until(c(1000)),
            Some(Halt::Fault(
                c(2),
                Fault::Refused(
                    Refused::Register("GPIO", "pin_int_ena"),
                    pro(program.entry() + 6)
                )
            )),
            "INT_ENA bit {bit}"
        );
        assert_eq!(chip.soc.gpio.read(PIN4 - GPIO), Some(0), "writes nothing");
    }
    let mut chip = store_after_two_loads(PIN4, INT_ENA, 0).chip();
    assert_eq!(chip.run_until(c(1000)), None, "INT_ENA bit 0");
}

#[test]
fn a_reserved_interrupt_type_or_a_wakeup_is_refused_by_name() {
    for (value, setting) in [
        (6 << 7, "pin_int_type"),
        (7 << 7, "pin_int_type"),
        (1 << 10, "pin_wakeup"),
    ] {
        let program = store_after_two_loads(PIN4, value, 0);
        let mut chip = program.chip();
        assert_eq!(
            chip.run_until(c(1000)),
            Some(Halt::Fault(
                c(2),
                Fault::Refused(Refused::Register("GPIO", setting), pro(program.entry() + 6))
            )),
            "{value:#x}"
        );
        assert_eq!(chip.soc.gpio.read(PIN4 - GPIO), Some(0), "writes nothing");
    }
    let mut chip = store_after_two_loads(PIN4, 5 << 7 | 1 << 2, 0).chip();
    assert_eq!(chip.run_until(c(1000)), None, "level high, open drain");
}

/// `FUN_WPD` on a pad with a net, when the part names no pull-down. The store writes nothing.
/// GPIO5 has no net, so the same store is kept and the pad pulls down.
#[test]
fn a_pull_down_on_a_netted_pad_is_refused_when_the_part_names_none() {
    const FUN_WPD: u32 = 1 << 7;
    let program = store_after_two_loads(IO_MUX + 4 + 4 * 21, FUN_WPD, 0);
    let mut chip = program.chip();
    let before = chip.soc.gpio.read_io_mux(4 + 4 * 21);
    chip.omit_pull_down();
    assert_eq!(
        chip.run_until(c(1000)),
        Some(Halt::Fault(
            c(2),
            Fault::Refused(
                Refused::Register("IO_MUX", "pull_down"),
                pro(program.entry() + 6)
            )
        ))
    );
    assert_eq!(chip.soc.gpio.read_io_mux(4 + 4 * 21), before);
    let program = store_after_two_loads(IO_MUX + 4 + 4 * 5, FUN_WPD, 0);
    let mut chip = program.chip_on(&[21]);
    chip.omit_pull_down();
    assert_eq!(chip.run_until(c(1000)), None, "GPIO5 has no net");
    assert_eq!(chip.soc.gpio.read_io_mux(4 + 4 * 5), Some(FUN_WPD));
    assert_eq!(chip.soc.gpio.drive(5), PadDrive::PullDown);
}
