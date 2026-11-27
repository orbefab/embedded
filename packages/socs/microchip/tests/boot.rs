//! Power-on and the bootloader's way to the sketch.

use crate::common::{ldi, spin};

use orbe_microchip::{Bootloader, Cpu, PinDrive, Refused, ResetKind, SupplyChannel};

const MCUSR: u16 = 0x54;
const WDTCSR: u16 = 0x60;
const SREG: u16 = 0x5F;
const SPL: u16 = 0x5D;
const SPH: u16 = 0x5E;
const NOP: u16 = 0x0000;
const WDRF: u8 = 1 << 3;
const WDE: u8 = 1 << 3;

/// Table 9-4, `CKSEL0` 1 and `SUT` 11 at 16 MHz: the 16K CK start-up, then 14 CK and 65 ms.
const START: u64 = 16_384 + 14 + 1_040_000;

/// The low and extended bytes Arduino's `boards.txt` burns for the Uno and the Nano, `0xFF` and
/// `0xFD`, with `high`, from power-on.
fn powered(high: u8, bootloader: Bootloader, words: &[u16]) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.set_fuses(0xFF, high, 0xFD);
    cpu.set_bootloader(bootloader);
    cpu.load_words(words);
    cpu.power_on();
    cpu
}

/// The cycle the first instruction at word 0 retires on, a `NOP`.
fn first_fetch(cpu: &mut Cpu) -> u64 {
    while cpu.pc() == 0 {
        cpu.step();
    }
    cpu.cycles() - 1
}

#[test]
fn power_on_fetches_the_image_after_the_start_from_reset() {
    let mut cpu = powered(0xDF, Bootloader::None, &[NOP; 4]);
    cpu.run_until(START);
    assert_eq!(cpu.pc(), 0, "still in the start from reset");
    assert_eq!(first_fetch(&mut cpu), START);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, 1, "PORF");
}

#[test]
fn a_bootloader_with_bootrst_unprogrammed_never_runs() {
    let mut cpu = powered(0xDF, Bootloader::Optiboot, &[NOP; 4]);
    assert_eq!(first_fetch(&mut cpu), START);
    assert_eq!(
        cpu.peek(MCUSR) & 0x0F,
        1,
        "PORF, which Optiboot would clear"
    );
}

/// The Uno's high fuse `0xDE` puts the reset vector on Optiboot's first word, `0x3F00`.
#[test]
fn optiboot_reaches_the_sketch_26_cycles_after_a_power_on() {
    let mut cpu = powered(0xDE, Bootloader::Optiboot, &[NOP; 4]);
    assert_eq!(first_fetch(&mut cpu), START + 26);
    // What `optiboot_atmega328.hex` leaves, run on this model from its first word.
    assert_eq!(cpu.peek(MCUSR), 0, "Optiboot cleared PORF");
    assert_eq!(cpu.peek(WDTCSR), 0);
    assert_eq!((cpu.peek(SPH), cpu.peek(SPL)), (0x08, 0xFD));
    let stack: Vec<u8> = (0x08FC..=0x08FF).map(|addr| cpu.peek(addr)).collect();
    assert_eq!(stack, [0x3F, 0xF7, 0x3F, 0x05]);
    assert_eq!(cpu.peek(25), 0x18);
    assert_eq!(cpu.peek(SREG), 0x02);
}

/// The Nano's high fuse `0xDA` puts the reset vector at `0x3C00`, 768 erased words below Optiboot.
#[test]
fn on_the_nano_the_erased_words_below_optiboot_take_a_cycle_each() {
    let mut cpu = powered(0xDA, Bootloader::Optiboot, &[NOP; 4]);
    assert_eq!(first_fetch(&mut cpu), START + 768 + 26);
}

/// After a watchdog reset `WDRF` forces `WDE` on (Table 11-1), and a sketch that never touches
/// the watchdog would reset again 16 ms later. Optiboot clears `MCUSR` and then the watchdog.
#[test]
fn optiboot_turns_off_the_watchdog_a_watchdog_reset_left_on() {
    for (bootloader, high, wde) in [
        (Bootloader::Optiboot, 0xDE, 0),
        (Bootloader::None, 0xDF, WDE),
    ] {
        let mut cpu = powered(high, bootloader, &[NOP; 4]);
        cpu.run_until(START + 1_000);
        orbe_microchip::test_hooks::reset_cpu(&mut cpu, ResetKind::Watchdog);
        let reset_at = cpu.cycles();
        cpu.run_until(reset_at + START + 1_000);
        assert_eq!(cpu.peek(WDTCSR) & WDE, wde, "{bootloader:?}");
        // `MCUSR` keeps `PORF` until a write clears it (§11.9.1).
        let flags = if wde == 0 { 0 } else { 1 | WDRF };
        assert_eq!(cpu.peek(MCUSR) & 0x0F, flags, "{bootloader:?}");
    }
}

/// A reset from `RESET` with `RESET` held from 5,000 cycles into the sketch, released 1,001
/// cycles later. The release's cycle and the pin reports from it on.
fn reset_from_the_pin(cpu: &mut Cpu) -> (u64, Vec<(u32, PinDrive, u64)>) {
    cpu.run_until(START + 5_000);
    cpu.pull_reset();
    cpu.run_until(START + 6_001);
    cpu.release_reset();
    let release = cpu.cycles();
    while cpu.take_pin().is_some() {}
    let mut reports = Vec::new();
    while cpu.peek(MCUSR) & WDRF == 0 {
        cpu.run_until(cpu.cycles() + 100_003);
        while let Some(report) = cpu.take_pin() {
            reports.push((report.pin, report.drive, report.cycle - release));
        }
    }
    (release, reports)
}

/// A reset from `RESET` sets `EXTRF`, and Optiboot waits for an upload. With nothing on RXD it
/// takes `TXD` for USART0 at 14 cycles in, makes `PB5` an output at 37, toggles it six times
/// 976 counts of `clkI/O` / 1024 apart, and polls `RXC0` until the watchdog it set to 1 s
/// resets the chip, 128K watchdog cycles after the last `WDR`. Each cycle is the one
/// `optiboot_atmega328.hex` gives on this model, run from its first word at the release.
#[test]
fn after_a_reset_from_the_pin_optiboot_flashes_pb5_until_its_watchdog_resets_the_chip() {
    const PB5: u32 = 5;
    const TXD: u32 = 17;
    let mut cpu = powered(0xDE, Bootloader::Optiboot, &[ldi(16, 1), spin()]);
    cpu.watch_gaps();
    let (release, reports) = reset_from_the_pin(&mut cpu);
    let toggle = |k: u64, drive| (PB5, drive, START + 999_428 + 999_424 * k);
    let watchdog = START + 999_428 + 999_424 * 5 + 16_384_001;
    assert_eq!(
        reports,
        [
            (TXD, PinDrive::High, START + 14),
            (PB5, PinDrive::Low, START + 37),
            toggle(0, PinDrive::High),
            toggle(1, PinDrive::Low),
            toggle(2, PinDrive::High),
            toggle(3, PinDrive::Low),
            toggle(4, PinDrive::High),
            toggle(5, PinDrive::Low),
            (PB5, PinDrive::Off, watchdog),
            (TXD, PinDrive::Off, watchdog),
        ]
    );
    // The watchdog reset finds `EXTRF` cleared, so Optiboot goes straight to the sketch.
    assert_eq!(first_fetch(&mut cpu), release + watchdog + START + 26);
    assert_eq!(cpu.peek(MCUSR), 0);
    assert!(cpu.refusal().is_none());
}

/// A byte on RXD during the wait is the start of an upload, which the model does not run. The
/// run ends at the `getch` poll that finds `RXC0` set.
#[test]
fn a_byte_on_rxd_during_the_wait_ends_the_run_at_getch() {
    let mut cpu = powered(0xDE, Bootloader::Optiboot, &[spin()]);
    cpu.watch_gaps();
    cpu.run_until(START + 1_000);
    cpu.pull_reset();
    cpu.run_until(START + 2_000);
    cpu.release_reset();
    let sent = cpu.cycles() + START + 8_000_000;
    cpu.run_until(sent);
    cpu.supply(SupplyChannel::Usart0, &[0x30]);
    cpu.run_until(sent + 20_000);
    let refusal = cpu.refusal().expect("a refusal at getch");
    // `UBRR0` 16 with `U2X0` is 136 cycles a bit. `RXC0` sets at the stop bit's samples, 9.5
    // bits in, and `getch` polls every 5 cycles.
    assert!(
        (sent + 1_292..sent + 1_360).contains(&refusal.at),
        "{}",
        refusal.at - sent
    );
    assert_eq!(refusal.pc, 0x3FD3);
    assert!(matches!(
        refusal.what,
        Refused::Unsimulated {
            peripheral: "Optiboot",
            ..
        }
    ));
    assert_eq!(cpu.peek(MCUSR) & WDRF, 0);
}

const PORF: u8 = 1 << 0;
const EXTRF: u8 = 1 << 1;
const BORF: u8 = 1 << 2;

/// A chip fused as a Uno's but without `BOOTRST`, so the image's first `NOP` is the first fetch,
/// with brown-out at `BODLEVEL` 101 when `brown_out`, started at power-on with `VCC` at 0 V. At
/// 0 V `RESET` reads low, since its pull-up has nothing to pull to, and brown-out reads low.
fn from_0_v(brown_out: bool) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.set_fuses(0xFF, 0xDF, if brown_out { 0xFD } else { 0xFF });
    cpu.load_words(&[NOP; 4]);
    cpu.power_on();
    cpu.set_power_on_reset(true);
    cpu.pull_reset();
    cpu.set_brown_out(true);
    cpu
}

/// The power-on reset clears `EXTRF` while it lasts (§11.9.1), and `RESET` reads high once its
/// pull-up has a supply, so a start from 0 V is no external reset. With brown-out on, `VCC`
/// between `VPOT` and `VBOT+` is a brown-out reset (§11.2), which sets `BORF` and holds the
/// chip until `VBOT+` (§11.5). The start from reset runs from there.
#[test]
fn a_start_from_0_v_sets_porf_and_borf_and_runs_from_vbot() {
    let mut cpu = from_0_v(true);
    cpu.run_until(1_000);
    cpu.release_reset();
    cpu.run_until(2_000);
    cpu.set_power_on_reset(false);
    cpu.run_until(3_000);
    assert!(cpu.in_reset(), "brown-out holds the chip past VPOT");
    cpu.set_brown_out(false);
    assert_eq!(first_fetch(&mut cpu), 3_000 + START);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, PORF | BORF);
}

/// With brown-out off, only the power-on reset holds the chip on the way up, and the start from
/// reset runs from `VPOT` rising (§11.3, Figure 11-2).
#[test]
fn a_start_from_0_v_without_brown_out_sets_porf_and_runs_from_vpot() {
    let mut cpu = from_0_v(false);
    cpu.run_until(1_000);
    cpu.release_reset();
    cpu.run_until(2_000);
    cpu.set_power_on_reset(false);
    assert_eq!(first_fetch(&mut cpu), 2_000 + START);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, PORF);
}

/// `RESET` held low by the circuit past `VPOT` extends the start externally (Figure 11-3): that
/// is an external reset, so `EXTRF` is set, and the start from reset runs from the release.
#[test]
fn reset_held_low_past_vpot_sets_extrf_and_runs_from_its_release() {
    let mut cpu = from_0_v(true);
    cpu.run_until(1_000);
    cpu.set_brown_out(false);
    cpu.set_power_on_reset(false);
    cpu.run_until(2_000);
    assert!(cpu.in_reset(), "RESET holds the chip past VPOT");
    cpu.release_reset();
    assert_eq!(first_fetch(&mut cpu), 2_000 + START);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, PORF | EXTRF);
}

/// From a running chip: a dip under `VBOT-` that stays over `VPOT` sets `BORF` and keeps
/// `PORF`. A dip under `VPOT` is a power-on reset, which leaves `PORF` alone, even with
/// `RESET` read low at 0 V and brown-out tripped on the way down. A press of `RESET` adds
/// `EXTRF`.
#[test]
fn each_reset_on_a_running_chip_sets_its_own_flag() {
    let mut cpu = from_0_v(true);
    cpu.release_reset();
    cpu.set_brown_out(false);
    cpu.set_power_on_reset(false);
    cpu.run_until(START + 1_000);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, PORF);

    cpu.set_brown_out(true);
    cpu.run_until(START + 2_000);
    cpu.set_brown_out(false);
    let recovery = cpu.cycles();
    assert_eq!(first_fetch(&mut cpu), recovery + START);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, PORF | BORF, "brown-out");

    let at = cpu.cycles() + 1_000;
    cpu.run_until(at);
    cpu.pull_reset();
    cpu.set_brown_out(true);
    cpu.set_power_on_reset(true);
    cpu.run_until(at + 1_000);
    cpu.release_reset();
    cpu.set_power_on_reset(false);
    cpu.run_until(at + 2_000);
    cpu.set_brown_out(false);
    let recovery = cpu.cycles();
    assert_eq!(first_fetch(&mut cpu), recovery + START);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, PORF | BORF, "power cycle to 0 V");

    let at = cpu.cycles() + 1_000;
    cpu.run_until(at);
    cpu.pull_reset();
    cpu.run_until(at + 1_000);
    cpu.release_reset();
    assert_eq!(first_fetch(&mut cpu), at + 1_000 + START);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, PORF | BORF | EXTRF, "RESET press");
}
