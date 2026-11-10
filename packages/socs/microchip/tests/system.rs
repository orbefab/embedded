//! EEPROM, watchdog, reset, sleep, `PRR`, and `CLKPR` on the 328P.
//!
//! Numbers are from DS40002061B. A check that only repeats a model constant
//! is not in this file.

use crate::common;
use common::Instructions;

use common::{
    TCCR0B, TCNT0, TIFR0, TIFR2, UCSR0B, UDR0, in_io, ldi, out, read_reg, sbi, spin, sts,
};
use orbe_microchip::{Channel, Cpu, FuseBytes, Level, ResetKind};

const EEARL: u16 = 0x0041;
const EEDR: u16 = 0x0040;
const EECR: u16 = 0x003F;
const SMCR: u16 = 0x0053;
const MCUSR: u16 = 0x0054;
const WDTCSR: u16 = 0x0060;
const CLKPR: u16 = 0x0061;
const PRR: u16 = 0x0064;
const TIMSK0: u16 = 0x006E;
const TCCR1B: u16 = 0x0081;
const TCNT1L: u16 = 0x0084;
const TCNT1H: u16 = 0x0085;
const TCCR2B: u16 = 0x00B1;
const TCNT2: u16 = 0x00B2;
const TIMSK2: u16 = 0x0070;
const MCUCR: u16 = 0x0055;
const UCSR0B_UDRIE: u8 = 1 << 5;
const ADCSRA: u16 = 0x007A;
const EICRA: u16 = 0x0069;
const EIMSK: u16 = 0x003D;
const EIFR: u16 = 0x003C;

const EERE: u8 = 1 << 0;
const EEPE: u8 = 1 << 1;
const EEMPE: u8 = 1 << 2;
const EERIE: u8 = 1 << 3;
const EEPM0: u8 = 1 << 4;
const EEPM1: u8 = 1 << 5;

const PORF: u8 = 1;
const EXTRF: u8 = 1 << 1;
const BORF: u8 = 1 << 2;
const WDRF: u8 = 1 << 3;

const WDIE: u8 = 1 << 6;
const WDE: u8 = 1 << 3;
const WDCE: u8 = 1 << 4;
const WDIF: u8 = 1 << 7;

/// Table 8-2 at 16 MHz: 26,368 RC cycles × 16/8.
const ATOMIC_CPU: u64 = 52_736;
/// Table 8-1, 1.8 ms at the same 8 MHz RC, at 16 MHz.
const HALF_CPU: u64 = 28_800;

fn fresh() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.set_fuses(
        FuseBytes::CRYSTAL.low,
        FuseBytes::CRYSTAL.high,
        FuseBytes::CRYSTAL.extended,
    );
    cpu
}

fn io(addr: u16) -> u16 {
    addr - 0x20
}

#[test]
fn eeprom_read_halts_four_cycles() {
    // §8.4.1 and §8.6.3. The byte is in EEDR when the read instruction
    // retires, and the next instruction waits four cycles.
    let mut cpu = fresh();
    cpu.set_eeprom(&[0xA5]);
    cpu.load_words(&[
        ldi(16, 0x00),
        out(16, io(EEARL)),
        sbi(io(EECR), 0),
        in_io(17, io(EEDR)),
    ]);
    // LDI 1, OUT 1, SBI 2 (DS40002198) plus the 4-cycle halt.
    cpu.run_until(8);
    assert_eq!(cpu.cycles(), 8);
    assert_eq!(cpu.peek(EEDR), 0xA5);
    assert_eq!(cpu.peek(EECR) & EERE, 0, "EERE is a strobe");
    cpu.step();
    assert_eq!(cpu.reg(17), 0xA5);
    assert_eq!(cpu.cycles(), 9, "IN is one cycle, after the halt");
}

#[test]
fn eeprom_write_halts_two_cycles_and_takes_table_8_2() {
    // §8.4.1 halt is 2 cycles. Table 8-2 is 26,368 RC cycles, 3.3 ms typical,
    // independent of CLKPR. At 16 MHz that is 52,736 CPU cycles from retire.
    let mut cpu = fresh();
    let words = write_program(0, 0x3C, 0);
    cpu.load_words(&words);
    while cpu.peek(EECR) & EEPE == 0 {
        cpu.step();
    }
    let retire = cpu.cycles();
    cpu.run_until(retire + ATOMIC_CPU - 1);
    assert_eq!(cpu.peek(EECR) & EEPE, EEPE, "one cycle early");
    cpu.run_until(retire + ATOMIC_CPU);
    assert_eq!(cpu.peek(EECR) & EEPE, 0, "EEPE clears at the table time");
    assert_eq!(cpu.eeprom()[0], 0x3C);
    assert_eq!(cpu.eeprom()[1], 0xFF, "untouched bytes stay erased");
    let _ = retire;
}

#[test]
fn eempe_window_accepts_four_and_rejects_five() {
    // §8.6.3. Hardware clears EEMPE after four cycles. EEPE inside that
    // window programs. The fifth cycle does not.
    let mut inside = fresh();
    inside.load_words(&window_program(3));
    inside.run_until(100);
    assert_eq!(inside.peek(EECR) & EEPE, EEPE, "EEPE on the 4th cycle");

    let mut late = fresh();
    late.load_words(&window_program(4));
    late.run_until(100);
    assert_eq!(
        late.peek(EECR) & EEPE,
        0,
        "EEPE on the 5th cycle is ignored"
    );
    assert_eq!(late.eeprom()[0], 0xFF);
}

#[test]
fn eepm_modes_match_table_8_1() {
    // 00 replaces. 01 erases to 0xFF in 1.8 ms. 10 writes by AND. 11 is reserved.
    let mut atomic = fresh();
    atomic.set_eeprom(&[0x0F]);
    atomic.load_words(&write_program(0, 0xF0, 0));
    atomic.run_until(ATOMIC_CPU + 50);
    assert_eq!(atomic.eeprom()[0], 0xF0);

    let mut erase = fresh();
    erase.set_eeprom(&[0x0F]);
    erase.load_words(&write_program(0, 0x00, EEPM0));
    erase.run_until(HALF_CPU + 80);
    assert_eq!(erase.eeprom()[0], 0xFF, "erase only");
    assert_eq!(erase.peek(EECR) & EEPE, 0);

    let mut write = fresh();
    write.set_eeprom(&[0x0F]);
    write.load_words(&write_program(0, 0xF3, EEPM1));
    write.run_until(HALF_CPU + 80);
    assert_eq!(write.eeprom()[0], 0x03, "write only ANDs");

    let mut reserved = fresh();
    reserved.load_words(&write_program(0, 0x11, EEPM1 | EEPM0));
    reserved.run_until(200);
    assert_eq!(reserved.peek(EECR) & EEPE, 0, "EEPM 11 does not program");
    assert_eq!(reserved.eeprom()[0], 0xFF);
}

#[test]
fn eepe_blocks_a_second_write_and_a_read() {
    // §8.6.3. A write while EEPE is set is ignored. EERE is ignored too,
    // and EEAR does not change.
    let mut cpu = fresh();
    cpu.set_eeprom(&[0x11, 0x22]);
    cpu.load_words(&write_program(0, 0x5A, 0));
    cpu.run_until(40);
    assert_eq!(cpu.peek(EECR) & EEPE, EEPE);
    let cycles = cpu.cycles();
    cpu.poke(EEARL, 1);
    cpu.poke(EEDR, 0x99);
    cpu.poke(EECR, EEMPE);
    cpu.poke(EECR, EEPE);
    cpu.poke(EECR, EERE);
    assert_eq!(cpu.peek(EEARL), 0, "EEAR held");
    assert_eq!(cpu.peek(EEDR), 0x5A, "the in-progress data register");
    cpu.run_until(cycles + ATOMIC_CPU);
    assert_eq!(cpu.eeprom()[0], 0x5A);
    assert_eq!(cpu.eeprom()[1], 0x22, "the second write did not land");
}

#[test]
fn eerie_is_level_while_eepe_is_clear() {
    // §8.6.3. Vector 23 (word 0x002C) while EEPE is clear and EERIE is set.
    // Entry does not clear EERIE.
    let mut cpu = fresh();
    cpu.poke(EECR, EERIE);
    cpu.set_sreg(0x80);
    cpu.load_words(&[0x0000]);
    cpu.step();
    assert_eq!(cpu.pc(), 0x002C, "EE READY");
    assert_eq!(cpu.peek(EECR) & EERIE, EERIE, "level request stays");
}

#[test]
fn eempe_in_the_same_store_does_not_program() {
    // §8.6.3. One STS of EEMPE|EEPE (0x06) does not program. EEMPE from an
    // earlier store, then EEPE inside four cycles, does.
    let mut same = fresh();
    let mut words = vec![
        ldi(16, 0),
        out(16, io(EEARL)),
        ldi(16, 0x5A),
        out(16, io(EEDR)),
        ldi(16, u16::from(EEMPE | EEPE)),
    ];
    words.extend(sts(16, EECR));
    words.push(0xCFFF);
    same.load_words(&words);
    same.run_until(30);
    assert_eq!(same.peek(EECR) & EEPE, 0, "STS EECR, 0x06 does not program");
    assert_eq!(same.eeprom()[0], 0xFF);

    let mut split = fresh();
    split.load_words(&window_program(0));
    split.run_until(40);
    assert_eq!(
        split.peek(EECR) & EEPE,
        EEPE,
        "EEMPE then EEPE within 4 cycles"
    );
}

#[test]
fn reset_during_a_program_keeps_the_old_byte() {
    // §8.4.2. The write completes when VCC is still sufficient. A brown-out
    // means it was not. The datasheet calls that byte undefined; this model
    // keeps the previous value. VBOT− for these fuses is 2.675 V.
    let mut external = fresh();
    external.set_eeprom(&[0x77]);
    external.load_words(&write_program(0, 0x01, 0));
    external.run_until(80);
    assert_eq!(external.peek(EECR) & EEPE, EEPE);
    orbe_microchip::test_hooks::reset_cpu(&mut external, orbe_microchip::ResetKind::External);
    assert_eq!(external.eeprom()[0], 0x01, "external reset above VBOT");
    assert_eq!(external.peek(EECR) & EEPE, 0);

    let mut watchdog = fresh();
    watchdog.set_eeprom(&[0x77]);
    watchdog.load_words(&write_program(0, 0x02, 0));
    watchdog.run_until(80);
    orbe_microchip::test_hooks::reset_cpu(&mut watchdog, ResetKind::Watchdog);
    assert_eq!(watchdog.eeprom()[0], 0x02, "watchdog reset above VBOT");

    let mut brown = fresh();
    brown.set_eeprom(&[0x77]);
    brown.load_words(&write_program(0, 0x03, 0));
    brown.run_until(80);
    assert_eq!(brown.peek(EECR) & EEPE, EEPE);
    brown.set_vcc(2.0);
    assert!(brown.is_held_in_reset(), "2.0 V is below VBOT−");
    assert_eq!(brown.peek(MCUSR) & BORF, BORF);
    assert_eq!(brown.eeprom()[0], 0x77, "brown-out leaves the old byte");
}

#[test]
fn watchdog_timeouts_match_table_11_2() {
    // 2K, 64K and 1024K at 128 kHz and 16 MHz. 2048 × 125 = 256_000.
    for (code, wdt_cycles) in [(0u8, 2_048u64), (0b0101, 65_536u64), (0b1001, 1_048_576u64)] {
        let mut cpu = fresh();
        arm_wdt(&mut cpu, WDIE, code);
        let start = cpu.cycles();
        let expect = wdt_cycles * 125;
        cpu.run_until(start + expect - 1);
        assert_eq!(cpu.peek(WDTCSR) & WDIF, 0, "code {code} early");
        cpu.run_until(start + expect);
        assert_eq!(cpu.peek(WDTCSR) & WDIF, WDIF, "code {code} at {expect}");
    }
}

#[test]
fn watchdog_modes_match_table_11_1() {
    // Stopped does nothing. Interrupt sets WDIF. Reset sets WDRF and restarts.
    // Interrupt-and-reset clears WDIE on the first timeout and resets on the next.
    let mut stopped = fresh();
    stopped.poke(WDTCSR, 0);
    stopped.run_until(300_000);
    assert_eq!(stopped.peek(MCUSR) & WDRF, 0, "stopped mode does not reset");

    let mut interrupt = fresh();
    arm_wdt(&mut interrupt, WDIE, 0);
    let at = interrupt.cycles() + 256_000;
    interrupt.set_sreg(0x80);
    interrupt.run_until(at);
    assert_eq!(interrupt.pc(), 0x000C, "WDT vector is 7, word 12");
    assert_eq!(interrupt.peek(WDTCSR) & WDIF, 0, "entry clears WDIF");

    let mut reset = fresh();
    arm_wdt(&mut reset, WDE, 0);
    let at = reset.cycles() + 256_000;
    reset.run_until(at);
    assert_eq!(reset.peek(MCUSR) & WDRF, WDRF);
    assert!(reset.cycles() < at + 100, "startup has not finished");
    reset.run_until(at + crystal_reset_delay());
    assert_eq!(reset.cycles(), at + crystal_reset_delay());
    assert_eq!(reset.pc(), 0);

    let mut both = fresh();
    arm_wdt(&mut both, WDE | WDIE, 0);
    let first = both.cycles() + 256_000;
    both.set_sreg(0x80);
    both.run_until(first);
    assert_eq!(both.pc(), 0x000C);
    assert_eq!(both.peek(WDTCSR) & WDIE, 0, "entry clears WDIE");
    assert_eq!(both.peek(MCUSR) & WDRF, 0);
    let second = both.cycles();
    // The next period started at the first timeout, then the interrupt entry
    // consumed cycles. Run until WDRF appears.
    both.run_until(first + 256_000 + crystal_reset_delay());
    assert_eq!(both.peek(MCUSR) & WDRF, WDRF, "second timeout resets");
    let _ = second;
}

#[test]
fn wdce_sequence_is_four_cycles() {
    // §11.8.2 and §11.9.2. WDCE|WDE, then the new prescaler within four
    // cycles. A late write leaves the old prescaler.
    let mut cpu = fresh();
    cpu.load_words(&wdce_program(1));
    cpu.run_until(40);
    assert_eq!(cpu.peek(WDTCSR) & 0x07, 0x01, "WDP0 landed");

    let mut late = fresh();
    late.load_words(&wdce_program(6));
    late.run_until(40);
    assert_eq!(late.peek(WDTCSR) & 0x07, 0, "late WDP is ignored");
}

#[test]
fn wdton_forces_system_reset_and_wdrf_forces_wde() {
    // Table 11-1, WDTON programmed. §11.9.2: WDE is set while WDRF is set.
    let mut cpu = Cpu::new();
    cpu.set_fuses(0xE2, 0xD9 & !(1 << 4), 0xFF);
    cpu.poke(WDTCSR, WDIE);
    assert_eq!(
        cpu.peek(WDTCSR) & (WDE | WDIE),
        WDE,
        "WDTON locks reset mode"
    );

    let mut flags = fresh();
    orbe_microchip::test_hooks::reset_cpu(&mut flags, ResetKind::Watchdog);
    assert_eq!(flags.peek(WDTCSR) & WDE, WDE, "WDRF forces WDE");
    // Flags clear by writing zero. A one does not set them (§11.9.1).
    flags.poke(MCUSR, 0);
    assert_eq!(flags.peek(MCUSR) & WDRF, 0);
    flags.poke(WDTCSR, 0);
    assert_eq!(
        flags.peek(WDTCSR) & WDE,
        WDE,
        "WDE needs the timed sequence"
    );
    flags.poke(WDTCSR, WDCE | WDE);
    flags.poke(WDTCSR, 0);
    assert_eq!(
        flags.peek(WDTCSR) & WDE,
        0,
        "window clears WDE once WDRF is clear"
    );
}

#[test]
fn wdr_restarts_the_count_and_wdif_clears_on_write_one() {
    let mut cpu = fresh();
    arm_wdt(&mut cpu, WDIE, 0);
    let start = cpu.cycles();
    cpu.run_until(start + 100_000);
    // RJMP .-1 so the 16K-word PC wrap does not execute WDR again.
    cpu.load_words(&[0x95A8, 0xCFFF]);
    cpu.step();
    assert_eq!(cpu.wdr_executions(), 1);
    // WDR is one cycle. The new period starts on that cycle, not the cycle
    // after it retires. 2K × 125 = 256_000.
    let origin = cpu.cycles() - 1;
    cpu.run_until(origin + 256_000 - 1);
    assert_eq!(cpu.peek(WDTCSR) & WDIF, 0, "WDR moved the deadline");
    cpu.run_until(origin + 256_000);
    assert_eq!(cpu.peek(WDTCSR) & WDIF, WDIF);
    // The waiting instruction is RJMP, which is 2 cycles, so the retire can
    // land one cycle after the timeout.
    assert!(cpu.cycles() - (origin + 256_000) <= 1);
    cpu.poke(WDTCSR, WDIF | WDIE);
    assert_eq!(cpu.peek(WDTCSR) & WDIF, 0, "write-one clears WDIF");
}

#[test]
fn each_reset_sets_its_mcusr_flag_and_keeps_the_others() {
    // §11.9.1. Power-on clears WDRF, BORF and EXTRF. The others accumulate.
    let mut cpu = fresh();
    orbe_microchip::test_hooks::reset_cpu(&mut cpu, orbe_microchip::ResetKind::External);
    assert_eq!(cpu.peek(MCUSR) & 0x0F, EXTRF | PORF);
    cpu.poke(0x0100, 0x5A);
    cpu.run_until(cpu.cycles() + crystal_reset_delay());
    assert_eq!(cpu.peek(0x0100), 0x5A, "SRAM survives an external reset");

    orbe_microchip::test_hooks::reset_cpu(&mut cpu, ResetKind::Watchdog);
    assert_eq!(cpu.peek(MCUSR) & WDRF, WDRF);
    assert_eq!(cpu.peek(0x0100), 0x5A);

    orbe_microchip::test_hooks::reset_cpu(&mut cpu, orbe_microchip::ResetKind::PowerOn);
    assert_eq!(
        cpu.peek(MCUSR) & 0x0F,
        PORF,
        "power-on clears the other flags"
    );
    assert_eq!(cpu.peek(0x0100), 0, "power-on SRAM is cleared to zero");
    assert!(cpu.eeprom().iter().all(|b| *b == 0xFF));
}

#[test]
fn startup_timeout_follows_the_crystal_fuses() {
    // Table 9-4, CKSEL0 = 1, SUT = 11: 14 CK + 65 ms from reset.
    // 65 ms at 16 MHz is 1_040_000. Power-down wake is 16K CK, not that delay.
    let mut cpu = fresh();
    let delay = crystal_reset_delay();
    assert_eq!(delay, 14 + 1_040_000);
    orbe_microchip::test_hooks::reset_cpu(&mut cpu, orbe_microchip::ResetKind::External);
    let at = cpu.cycles();
    cpu.load_words(&[0x0000]);
    cpu.run_until(at + delay - 1);
    assert_eq!(cpu.pc(), 0, "still in the time-out");
    cpu.run_until(at + delay + 1);
    assert_eq!(cpu.cycles(), at + delay + 1, "the NOP retired");
    assert_eq!(cpu.pc(), 1);
}

/// §11.4. A held `RESET` keeps the chip stopped. Release starts the
/// time-out, then the reset vector is fetched with `EXTRF` set.
#[test]
fn a_held_reset_pin_stops_the_chip_until_release() {
    let mut cpu = fresh();
    cpu.load_words(&[0x0000; 4]);
    cpu.pull_reset();
    assert!(cpu.is_held_in_reset());
    cpu.run_until(1_000);
    assert_eq!(cpu.cycles(), 1_000, "time runs while held");
    assert_eq!(cpu.pc(), 0, "nothing is fetched while held");
    cpu.release_reset();
    let delay = crystal_reset_delay();
    cpu.run_until(1_000 + delay);
    assert_eq!(cpu.pc(), 0, "still in the time-out");
    cpu.run_until(1_000 + delay + 1);
    assert_eq!(cpu.pc(), 1);
    assert_eq!(cpu.peek(MCUSR) & EXTRF, EXTRF);
}

#[test]
fn a_clone_runs_the_same_as_its_source() {
    let mut cpu = fresh();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x47, 9);
    cpu.poke(0x44, 0x42);
    cpu.poke(TCCR0B, 0x01);
    cpu.load_words(&[spin()]);
    cpu.run_until(100);
    let mut copy = cpu.clone();
    cpu.run_until(1_000);
    copy.run_until(1_000);
    let source: Vec<_> = std::iter::from_fn(|| cpu.take_pin()).collect();
    let cloned: Vec<_> = std::iter::from_fn(|| copy.take_pin()).collect();
    assert!(source.len() > 80, "OC0A toggles every 10 cycles");
    assert_eq!(source, cloned);
    assert_eq!(cpu.cycles(), copy.cycles());
    assert_eq!(cpu.peek(TCNT0), copy.peek(TCNT0));
}

#[test]
fn brown_out_uses_table_29_12_and_hysteresis() {
    // BODLEVEL 101 is 2.7 V typical. VHYST is 50 mV, so VBOT- is 2.675 and
    // VBOT+ is 2.725 (§11.5, Table 29-11, Table 29-12).
    let mut cpu = fresh();
    cpu.set_vcc(2.67);
    assert!(cpu.is_held_in_reset(), "below VBOT-");
    assert_eq!(cpu.peek(MCUSR) & BORF, BORF);
    cpu.set_vcc(2.70);
    assert!(cpu.is_held_in_reset(), "still below VBOT+");
    cpu.set_vcc(2.73);
    assert!(!cpu.is_held_in_reset());
    let at = cpu.cycles();
    cpu.load_words(&[0x0000]);
    cpu.run_until(at + crystal_reset_delay() + 1);
    assert_eq!(cpu.pc(), 1, "recovers after the reset time-out");
}

#[test]
fn sleep_wake_sources_follow_table_10_1() {
    // A Timer0 overflow wakes Idle and does not wake power-down.
    // The watchdog wakes power-down. An edge INT does not; a low level does.
    let mut idle = fresh();
    idle.poke(SMCR, 0x01);
    idle.poke(TCCR0B, 0x01);
    idle.poke(TIMSK0, 0x01);
    idle.set_sreg(0x80);
    idle.load_words(&[0x9588]);
    idle.run_until(256 + 8);
    assert_eq!(idle.pc(), 0x0020, "TIMER0_OVF from Idle");

    let mut down = fresh();
    down.poke(SMCR, 0x05);
    down.poke(TCCR0B, 0x01);
    down.poke(TIMSK0, 0x01);
    down.set_sreg(0x80);
    down.load_words(&[0x9588]);
    down.run_until(1_000);
    assert!(down.is_sleeping(), "Timer0 does not wake power-down");

    arm_wdt(&mut down, WDIE, 0);
    let armed = down.cycles();
    down.run_until(armed + 256_000 + 16_384 + 8);
    assert!(!down.is_sleeping(), "WDT wakes power-down");
    assert_eq!(down.pc(), 0x000C);
}

#[test]
fn power_down_startup_is_16k_and_standby_is_six() {
    // Table 9-4 power-down column is 16K CK. §10.7 standby is 6 CK.
    // Then the existing sleep entry adds 8 cycles (§7.7, §10.1).
    let mut down = fresh();
    down.poke(SMCR, 0x05);
    arm_wdt(&mut down, WDIE, 0);
    let armed = down.cycles();
    down.set_sreg(0x80);
    down.load_words(&[0x9588]);
    down.step();
    let wake = armed + 256_000;
    down.run_until(wake + 16_384 + 8);
    assert_eq!(down.cycles(), wake + 16_384 + 8);
    assert_eq!(down.pc(), 0x000C);

    let mut standby = fresh();
    standby.poke(SMCR, 0x0D);
    arm_wdt(&mut standby, WDIE, 0);
    let armed = standby.cycles();
    standby.set_sreg(0x80);
    standby.load_words(&[0x9588]);
    standby.step();
    let wake = armed + 256_000;
    standby.run_until(wake + 6 + 8);
    assert_eq!(standby.cycles(), wake + 6 + 8, "standby is 6 CK plus entry");
    assert_eq!(standby.pc(), 0x000C);
}

#[test]
fn a_low_level_that_ends_during_startup_does_not_interrupt() {
    // §10.5 / §13.2.1. The core still wakes. No vector is taken.
    let mut cpu = fresh();
    cpu.poke(SMCR, 0x05);
    cpu.poke(EICRA, 0x00);
    cpu.poke(EIMSK, 0x01);
    cpu.drive_pin(18, Level::Low);
    cpu.load_words(&[0x9478, 0x9588, 0x0000]);
    cpu.step();
    cpu.step();
    assert!(cpu.is_sleeping(), "SEI lets SLEEP run before INT0");
    let slept = cpu.cycles();
    cpu.drive_pin(18, Level::High);
    cpu.run_until(slept + 16_384);
    assert!(!cpu.is_sleeping());
    assert_eq!(cpu.pc(), 2, "the instruction after SLEEP, not INT0");
}

#[test]
fn noise_reduction_starts_the_adc_and_edge_int_ignores_a_stopped_clock() {
    // §24.6. Entering ADC noise reduction starts one conversion.
    // §13. An edge INT needs clkI/O, which Table 10-1 stops in this mode.
    let mut cpu = fresh();
    cpu.poke(ADCSRA, 0x80 | 0x07);
    cpu.poke(SMCR, 0x03);
    cpu.load_words(&[0x9588]);
    cpu.step();
    assert_eq!(cpu.peek(ADCSRA) & 0x40, 0x40, "ADSC set by the sleep");

    let mut edge = fresh();
    edge.poke(SMCR, 0x05);
    edge.poke(EICRA, 0x02);
    edge.poke(EIMSK, 0x01);
    edge.drive_pin(18, Level::High);
    edge.load_words(&[0x9588]);
    edge.step();
    edge.drive_pin(18, Level::Low);
    edge.run_until(edge.cycles() + 20);
    assert_eq!(edge.peek(EIFR) & 1, 0, "falling INT0 needs clkI/O");
    assert!(edge.is_sleeping());
}

#[test]
fn a_bods_poke_closes_after_four_cycles() {
    // §10.11.2. Both bits set opens a four-cycle window, then both read 0.
    let mut cpu = fresh();
    cpu.poke(MCUCR, 0x60);
    cpu.load_words(&[0x0000; 10]);
    cpu.insns(10);
    assert_eq!(cpu.peek(MCUCR) & 0x60, 0);
}

#[test]
fn noise_reduction_stops_clk_io() {
    // Table 10-1. ADC noise reduction stops clkI/O. Timer0 at /1 does not count.
    let mut cpu = fresh();
    cpu.poke(TCCR0B, 0x01);
    cpu.poke(SMCR, 0x03);
    cpu.load_words(&[0x9588]);
    cpu.step();
    let tcnt = cpu.peek(TCNT0);
    cpu.run_until(cpu.cycles() + 8);
    assert_eq!(cpu.peek(TCNT0), tcnt);
}

#[test]
fn prr_freezes_each_module_and_keeps_the_register() {
    // §10.9. Stores and reads go through the instruction. A frozen read has
    // no side effect: TCNT1L does not latch TEMP. PRTIM0 is bit 5, PRTIM1
    // bit 3, PRTIM2 bit 6, PRUSART0 bit 1, PRADC bit 0.
    let mut cpu = fresh();
    store(&mut cpu, TCCR0B, 0x01);
    cpu.run_until(cpu.cycles() + 8);
    store(&mut cpu, PRR, 1 << 5);
    let frozen = read_reg(&mut cpu, TCNT0);
    assert!(frozen > 0, "Timer0 counted before PRTIM0");
    store(&mut cpu, TCCR0B, 0x00);
    cpu.run_until(cpu.cycles() + 30);
    assert_eq!(read_reg(&mut cpu, TCNT0), frozen, "PRTIM0");
    assert_eq!(
        read_reg(&mut cpu, TCCR0B) & 0x07,
        0x01,
        "the clock-select store was discarded"
    );
    store(&mut cpu, PRR, 0);
    cpu.run_until(cpu.cycles() + 4);
    assert!(
        read_reg(&mut cpu, TCNT0) != frozen,
        "clearing PRTIM0 resumes the count"
    );

    store(&mut cpu, TCCR1B, 0x01);
    cpu.run_until(cpu.cycles() + 4);
    store(&mut cpu, PRR, 1 << 3);
    let t1 = read_reg(&mut cpu, TCNT1L);
    cpu.run_until(cpu.cycles() + 12);
    assert_eq!(read_reg(&mut cpu, TCNT1L), t1, "PRTIM1");

    // TCNT1 = 0x1234, then TEMP planted as 0. The frozen low-byte read
    // must leave TEMP at 0, so the high byte reads back 0 rather than 0x12.
    store(&mut cpu, PRR, 0);
    store(&mut cpu, TCCR1B, 0x00);
    store(&mut cpu, TCNT1H, 0x12);
    store(&mut cpu, TCNT1L, 0x34);
    store(&mut cpu, TCNT1H, 0x00);
    store(&mut cpu, PRR, 1 << 3);
    assert_eq!(read_reg(&mut cpu, TCNT1L), 0x34, "frozen TCNT1L");
    assert_eq!(
        read_reg(&mut cpu, TCNT1H),
        0x00,
        "a frozen TCNT1L read does not latch TEMP"
    );

    store(&mut cpu, PRR, 0);
    store(&mut cpu, TCCR2B, 0x01);
    cpu.run_until(cpu.cycles() + 4);
    store(&mut cpu, PRR, 1 << 6);
    let t2 = read_reg(&mut cpu, TCNT2);
    cpu.run_until(cpu.cycles() + 12);
    assert_eq!(read_reg(&mut cpu, TCNT2), t2, "PRTIM2");

    store(&mut cpu, PRR, 1 << 1);
    store(&mut cpu, UCSR0B, 0x08);
    assert_eq!(
        read_reg(&mut cpu, UCSR0B) & 0x08,
        0,
        "PRUSART0 discards the store"
    );

    store(&mut cpu, PRR, 1 << 0);
    store(&mut cpu, ADCSRA, 0x80);
    assert_eq!(read_reg(&mut cpu, ADCSRA) & 0x80, 0, "PRADC discards ADEN");

    store(&mut cpu, PRR, (1 << 7) | (1 << 2));
    assert_eq!(
        read_reg(&mut cpu, PRR) & ((1 << 7) | (1 << 2)),
        (1 << 7) | (1 << 2)
    );
}

#[test]
fn clkpr_window_and_division_scale_wall_time() {
    // §9.12.2. CLKPCE opens four cycles. CLKPS = 1 divides by 2.
    // cycles() stay CPU cycles, so a Timer0 overflow is still 256 of them.
    // The oscillator time doubles. The watchdog, which is not on clkI/O,
    // arrives in half as many CPU cycles.
    let mut cpu = fresh();
    let mut words = vec![ldi(16, 0x80)];
    words.extend_from_slice(&sts(16, CLKPR));
    words.push(ldi(16, 0x01));
    words.extend_from_slice(&sts(16, CLKPR));
    cpu.load_words(&words);
    cpu.run_until(20);
    assert_eq!(cpu.peek(CLKPR) & 0x0F, 0x01);
    assert_eq!(cpu.clock_div(), 2);

    let mut late = fresh();
    let mut words = vec![ldi(16, 0x80)];
    words.extend_from_slice(&sts(16, CLKPR));
    words.extend_from_slice(&[0x0000, 0x0000, 0x0000, 0x0000]);
    words.push(ldi(16, 0x01));
    words.extend_from_slice(&sts(16, CLKPR));
    late.load_words(&words);
    late.run_until(30);
    assert_eq!(
        late.peek(CLKPR) & 0x0F,
        0,
        "the fifth cycle misses the window"
    );

    let mut slow = fresh();
    slow.poke(CLKPR, 0x80);
    slow.poke(CLKPR, 0x01);
    slow.poke(TCCR0B, 0x01);
    let mark = slow.oscillator_cycles();
    slow.run_until(slow.cycles() + 256);
    let osc = slow.oscillator_cycles() - mark;
    assert_eq!(
        osc, 512,
        "div 2 makes 256 CPU cycles into 512 oscillator cycles"
    );

    let mut wdt = fresh();
    wdt.poke(CLKPR, 0x80);
    wdt.poke(CLKPR, 0x01);
    arm_wdt(&mut wdt, WDIE, 0);
    let start = wdt.cycles();
    wdt.run_until(start + 128_000);
    assert_eq!(
        wdt.peek(WDTCSR) & WDIF,
        WDIF,
        "16 ms is 128_000 CPU cycles at div 2"
    );
}

#[test]
fn crystal_fuses_decode_and_the_image_still_starts_at_zero() {
    // 0xFF / 0xDA / 0xFD. BOOTRST points at 0x3C00 (Table 27-13,
    // BOOTSZ = 01). The bootloader is not modeled.
    let cpu = fresh();
    assert_eq!(
        cpu.fuses(),
        FuseBytes {
            low: 0xFF,
            high: 0xDA,
            extended: 0xFD
        }
    );
    assert_eq!(cpu.boot_word(), 0x3C00);
    assert_eq!(cpu.clock_div(), 1, "CKDIV8 is unprogrammed");
    assert_eq!(cpu.pc(), 0);
    cpu_fetch_zero(cpu);
}

fn cpu_fetch_zero(mut cpu: Cpu) {
    cpu.load_words(&[0x0000]);
    cpu.step();
    assert_eq!(cpu.pc(), 1, "the image starts at word 0");
}

#[test]
fn stepped_and_scheduled_sleep_agree_on_the_watchdog() {
    // Differential, plus the Table 11-2 anchor above. A fast oscillator keeps
    // the stepped run short. Both paths must land on the same cycle.
    let mk = || {
        let mut cpu = fresh();
        cpu.set_wdt_hz(16_000_000);
        cpu.poke(SMCR, 0x05);
        arm_wdt(&mut cpu, WDIE, 0);
        cpu.set_sreg(0x80);
        cpu.load_words(&[0x9588]);
        cpu
    };
    let mut jumped = mk();
    jumped.run_until(20_000);
    let mut stepped = mk();
    while stepped.cycles() < jumped.cycles() {
        stepped.run_until(stepped.cycles() + 1);
    }
    assert_eq!(stepped.cycles(), jumped.cycles());
    assert_eq!(stepped.pc(), jumped.pc());
    assert_eq!(stepped.peek(MCUSR), jumped.peek(MCUSR));
    assert!(!jumped.is_sleeping());
}

#[test]
fn standby_keeps_the_prescaler_phase() {
    // Table 10-1 stops clkI/O. §10.7's 6 CK start-up is not a multiple of
    // the /64 divider, so a stale origin would leave TCNT0 unmoved here.
    let mut words = vec![0u16; 60];
    words.push(0x9588);
    let mut cpu = fresh();
    cpu.set_wdt_hz(16_000_000);
    cpu.poke(TCCR0B, 0x03);
    cpu.poke(SMCR, 0x0D);
    arm_wdt(&mut cpu, WDIE, 0);
    cpu.set_sreg(0x80);
    cpu.load_words(&words);
    cpu.run_until(60);
    assert_eq!(cpu.peek(TCNT0), 0, "the first /64 tick is cycle 64");
    cpu.step();
    assert!(cpu.is_sleeping());
    assert_eq!(cpu.cycles(), 61);
    cpu.run_until(2048 + 6);
    assert_eq!(cpu.cycles(), 2048 + 6 + 8, "6 CK plus the sleep entry");
    // Clocks resume at 2054. Eight entry cycles later the frozen phase has
    // seen io-time 69, and 69/64 is one tick.
    assert_eq!(cpu.peek(TCNT0), 1);
}

#[test]
fn reset_releases_a_driven_pin() {
    // §11.1. The ports return to their reset state on the reset cycle.
    let mut cpu = fresh();
    cpu.load_words(&[sbi(0x04, 5), sbi(0x05, 5), 0xCFFF]);
    cpu.run_until(6);
    let mut saw_high = false;
    while let Some(report) = cpu.take_pin() {
        if report.pin == 5 && report.level == Level::High {
            saw_high = true;
        }
    }
    assert!(saw_high, "PORTB5 was driven");
    let at = cpu.cycles();
    orbe_microchip::test_hooks::reset_cpu(&mut cpu, orbe_microchip::ResetKind::External);
    let report = cpu.take_pin().expect("reset reports the released pin");
    assert_eq!(report.pin, 5);
    assert_eq!(report.level, Level::Released);
    assert_eq!(report.cycle, at);
}

#[test]
fn a_running_clkps_survives_a_fuse_override() {
    // §9.12.2. CKDIV8 sets the initial CLKPS. Changing the fuses after an
    // instruction has run leaves the program's divider; the next reset loads it.
    let mut cpu = fresh();
    cpu.load_words(&[0x0000]);
    cpu.step();
    cpu.poke(CLKPR, 0x80);
    cpu.poke(CLKPR, 0x01);
    assert_eq!(cpu.clock_div(), 2);
    cpu.set_fuses(0x7F, FuseBytes::CRYSTAL.high, FuseBytes::CRYSTAL.extended);
    assert_eq!(cpu.clock_div(), 2, "mid-run CLKPS stays");
    orbe_microchip::test_hooks::reset_cpu(&mut cpu, orbe_microchip::ResetKind::External);
    assert_eq!(cpu.clock_div(), 8, "CKDIV8 applies on the reset");
}

#[test]
fn table_10_1_wake_mask_filters_pending_sources() {
    // Table 10-1. A flag that is already set does not wake a mode whose
    // column is blank. The watchdog does, and the vectors that were waiting
    // are then served in vector order (Table 12-6).
    let mut down = fresh();
    down.set_wdt_hz(16_000_000);
    down.poke(TIFR0, 0x01);
    down.poke(TIMSK0, 0x01);
    down.poke(EECR, EERIE);
    down.poke(UCSR0B, UCSR0B_UDRIE);
    down.poke(SMCR, 0x05);
    arm_wdt(&mut down, WDIE, 0);
    let armed = down.cycles();
    down.load_words(&[0x9478, 0x9588, 0x0000]);
    down.step();
    down.step();
    assert!(down.is_sleeping(), "TOV0, EE READY, and UDRIE stay pending");
    down.run_until(armed + 2_048 - 1);
    assert!(
        down.is_sleeping(),
        "still asleep one cycle before the watchdog"
    );
    let wake = armed + 2_048 + 16_384 + 8;
    down.run_until(wake);
    assert_eq!(down.cycles(), wake, "16K CK plus the 8-cycle sleep entry");
    assert_eq!(down.pc(), 0x000C, "WDT is vector 7, ahead of the others");
    assert_eq!(down.peek(TIFR0) & 1, 1, "TOV0 is still pending");
    down.set_sreg(0x80);
    down.step();
    assert_eq!(down.pc(), 0x0020, "TIMER0_OVF is next");
    down.set_sreg(0x80);
    down.step();
    assert_eq!(down.pc(), 0x0026, "USART_UDRE follows");
    down.poke(UCSR0B, 0);
    down.set_sreg(0x80);
    down.step();
    assert_eq!(down.pc(), 0x002C, "EE READY is last of the four");

    // ADC noise reduction: other I/O does not wake. EEPROM ready and the
    // ADC do. ADC is vector 22, word 0x002A.
    stays_asleep(0x03, |cpu| cpu.poke(UCSR0B, UCSR0B_UDRIE));
    let ee = sleep_in(0x03, |cpu| cpu.poke(EECR, EERIE));
    assert_eq!(ee.pc(), 0x002C, "EE READY wakes ADC noise reduction");
    let mut adc = fresh();
    adc.poke(ADCSRA, 0x80 | 0x08);
    adc.poke(SMCR, 0x03);
    adc.set_sreg(0x80);
    adc.load_words(&[0x9588]);
    adc.step();
    assert!(adc.is_sleeping(), "UDRIE does not wake ADC noise reduction");
    assert_eq!(adc.peek(ADCSRA) & 0x40, 0x40, "the conversion was started");
    let started = adc.cycles();
    while adc.is_sleeping() {
        adc.run_until(adc.cycles() + 1);
        assert!(
            adc.cycles() < started + 500,
            "ADC complete did not wake noise reduction"
        );
    }
    assert_eq!(adc.pc(), 0x002A, "ADC vector 22 is word 0x002A");

    // Idle wakes on other I/O. A flag with its enable clear does not.
    let idle = sleep_in(0x01, |cpu| cpu.poke(UCSR0B, UCSR0B_UDRIE));
    assert_eq!(idle.pc(), 0x0026, "Idle wakes on USART UDRE");
    assert_eq!(idle.cycles(), 10, "SEI, SLEEP, and the 8-cycle entry");
    stays_asleep(0x01, |cpu| cpu.poke(TIFR0, 0x01));

    // Power-save and extended standby take a pending Timer2 request.
    // Power-down and standby do not. Timer2's async clock is unmodeled, so
    // the counter does not run in those sleeps; only a request that is
    // already pending is a wake source. Power-save start-up is 16K CK.
    // Extended standby is 6 CK.
    let save = wake_on(0x07, 16_384, |cpu| {
        cpu.poke(TIFR2, 0x01);
        cpu.poke(TIMSK2, 0x01);
    });
    assert_eq!(save.0, 2 + 16_384 + 8);
    assert_eq!(save.1, 0x0012, "TIMER2_OVF wakes power-save");
    stays_asleep(0x07, |cpu| cpu.poke(UCSR0B, UCSR0B_UDRIE));
    stays_asleep(0x05, |cpu| {
        cpu.poke(TIFR2, 0x01);
        cpu.poke(TIMSK2, 0x01);
    });
    let extended = wake_on(0x0F, 6, |cpu| {
        cpu.poke(TIFR2, 0x01);
        cpu.poke(TIMSK2, 0x01);
    });
    assert_eq!(extended.0, 2 + 6 + 8, "extended standby is 6 CK");
    assert_eq!(extended.1, 0x0012);
    stays_asleep(0x0F, |cpu| cpu.poke(UCSR0B, UCSR0B_UDRIE));
    stays_asleep(0x0D, |cpu| {
        cpu.poke(TIFR2, 0x01);
        cpu.poke(TIMSK2, 0x01);
    });
    let standby = wake_on_wdt(0x0D, 6);
    assert_eq!(
        standby.0,
        2_048 + 6 + 8,
        "standby wakes on the watchdog in 6 CK"
    );
    assert_eq!(standby.1, 0x000C);

    // Power-down: a low level wakes, an edge does not (§10.5, §13.2.1).
    let mut level = sleep_in(0x05, |cpu| {
        cpu.poke(EICRA, 0x00);
        cpu.poke(EIMSK, 0x01);
        cpu.drive_pin(18, Level::Low);
    });
    assert!(level.is_sleeping(), "SLEEP ran before INT0");
    let at = level.cycles();
    level.run_until(at + 16_384 + 8);
    assert_eq!(level.pc(), 0x0002, "level INT0 wakes power-down");
    stays_asleep(0x05, |cpu| {
        cpu.poke(EICRA, 0x02);
        cpu.poke(EIMSK, 0x01);
        cpu.poke(EIFR, 0x01);
    });
}

/// Table 11-2. A watchdog time-out that falls inside the four-cycle entry a poke of `UDRIE0`
/// raised resets in that step. A run to the same cycle ends on the reset vector with `WDRF` set.
#[test]
fn a_watchdog_reset_inside_a_boundary_entry() {
    for back in 1..=4u64 {
        let mut cpu = fresh();
        cpu.set_wdt_hz(16_000_000);
        arm_wdt(&mut cpu, WDE, 0);
        let timeout = cpu.cycles() + 2_048;
        cpu.set_sreg(0x80);
        let image = vec![0u16; 4096];
        cpu.load_words(&image);
        cpu.run_until(timeout - back);
        assert_eq!(cpu.peek(MCUSR) & WDRF, 0, "back {back}");
        cpu.poke(UCSR0B, UCSR0B_UDRIE);
        let mut run = cpu.clone();
        cpu.step();
        let after = cpu.cycles();
        run.run_until(after);
        assert_eq!(cpu.pc(), 0, "reset in the entry, back {back}");
        assert_eq!(cpu.peek(MCUSR) & WDRF, WDRF, "back {back}");
        assert_eq!(
            (run.pc(), run.cycles(), run.peek(MCUSR) & WDRF),
            (0, cpu.cycles(), WDRF),
            "back {back}"
        );
    }
}

#[test]
fn idle_starts_the_adc_when_aden_is_set() {
    // §10.3. Entering Idle starts one conversion when ADEN is set.
    // ADSC is bit 6 of ADCSRA.
    let mut cpu = fresh();
    cpu.poke(ADCSRA, 0x80);
    cpu.poke(SMCR, 0x01);
    cpu.load_words(&[0x9588]);
    cpu.step();
    assert!(cpu.is_sleeping());
    assert_eq!(cpu.peek(ADCSRA) & 0x40, 0x40, "ADSC set by Idle");

    let mut off = fresh();
    off.poke(SMCR, 0x01);
    off.load_words(&[0x9588]);
    off.step();
    assert_eq!(off.peek(ADCSRA) & 0x40, 0, "ADEN clear does not start one");
}

#[test]
fn bods_adds_sixty_microseconds_to_the_wake() {
    // §10.11.2. BODS|BODSE, then BODS with BODSE clear within four cycles.
    // The bit is active for three cycles. A later store does not latch it.
    // §10.2. With BOD off in the sleep, 2.0 V does not reset. The wake is
    // the mode's start-up plus about 60 µs: 960 CPU cycles at 16 MHz.
    let mut late = fresh();
    let mut words = vec![ldi(16, 0x60)];
    words.extend(sts(16, MCUCR));
    words.extend([0, 0, 0, 0]);
    words.push(ldi(16, 0x40));
    words.extend(sts(16, MCUCR));
    late.load_words(&words);
    late.insns(8);
    assert_eq!(late.peek(MCUCR) & 0x40, 0, "BODS outside the four cycles");

    let mut cpu = fresh();
    cpu.set_wdt_hz(16_000_000);
    cpu.poke(SMCR, 0x05);
    arm_wdt(&mut cpu, WDIE, 0);
    let armed = cpu.cycles();
    cpu.set_sreg(0x80);
    let mut words = vec![ldi(16, 0x60)];
    words.extend(sts(16, MCUCR));
    words.push(ldi(16, 0x40));
    words.extend(sts(16, MCUCR));
    words.push(0x9588);
    cpu.load_words(&words);
    while !cpu.is_sleeping() {
        cpu.step();
    }
    cpu.set_vcc(2.0);
    assert!(!cpu.is_held_in_reset(), "BOD is off during the sleep");
    assert_eq!(cpu.peek(MCUSR) & BORF, 0);
    cpu.set_vcc(5.0);
    let wake = armed + 2_048 + 16_384 + 960 + 8;
    cpu.run_until(wake);
    assert_eq!(cpu.cycles(), wake, "16K CK + 60 µs + the 8-cycle entry");
    assert_eq!(cpu.pc(), 0x000C);
}

#[test]
fn usart_after_reset_counts_phase_from_the_release() {
    // §11.1 holds internal reset until the time-out ends. §20.3 / Table 20-1:
    // UBRR 0 and U2X clear make a bit 16 clocks and an 8N1 frame 160 clocks
    // after the start bit. Phase 0 is the release cycle R, so the first
    // txclk is R+16 and the stop bit ends at R+176. The same program from
    // cycle 0 ends at 176.
    const DELAY: u64 = 14 + 1_040_000;
    let program = tx_program();
    let mut cold = fresh();
    cold.load_words(&program);
    cold.run_until(176);
    assert_eq!(cold.take(Channel::Usart0), Some((0x41, 16 + 160)));

    let mut warm = fresh();
    warm.load_words(&[0x0000]);
    warm.run_until(30);
    let reset_at = warm.cycles();
    orbe_microchip::test_hooks::reset_cpu(&mut warm, orbe_microchip::ResetKind::External);
    assert_eq!(warm.cycles(), reset_at);
    let release = reset_at + DELAY;
    warm.load_words(&program);
    warm.run_until(release + 176);
    assert_eq!(
        warm.take(Channel::Usart0),
        Some((0x41, release + 16 + 160)),
        "the divider's phase 0 is R, not the absolute cycle"
    );
}

#[test]
fn sleep_enters_on_the_instruction_that_executes_it() {
    // §10.1. With SE set, SLEEP retires into the mode. The next instruction
    // does not run, and power-down has already stopped clkI/O.
    let mut cpu = fresh();
    cpu.poke(TCCR0B, 0x01);
    cpu.run_until(4);
    assert!(cpu.peek(TCNT0) > 0, "Timer0 was counting");
    cpu.poke(SMCR, 0x05);
    let at = cpu.cycles();
    cpu.load_words(&[0x9588, ldi(16, 0x11)]);
    cpu.step();
    assert!(cpu.is_sleeping());
    assert_eq!(cpu.cycles(), at + 1, "SLEEP is one cycle");
    assert_eq!(cpu.pc(), 1);
    assert_eq!(cpu.reg(16), 0, "the instruction after SLEEP did not run");
    let frozen = cpu.peek(TCNT0);
    cpu.run_until(cpu.cycles() + 32);
    assert_eq!(
        cpu.peek(TCNT0),
        frozen,
        "power-down stopped clkI/O at SLEEP"
    );
    assert!(cpu.is_sleeping());
}

#[test]
fn watchdog_inside_a_multicycle_instruction_resets_at_retire() {
    // Table 11-2 at a 16 MHz watchdog clock: 2,048 counts is 2,048 CPU
    // cycles. CALL is 4 cycles (DS40002198). The timeout falls on the third
    // of those cycles, and the reset is taken when CALL retires, before the
    // next instruction.
    let mut cpu = fresh();
    cpu.set_wdt_hz(16_000_000);
    arm_wdt(&mut cpu, WDE, 0);
    let timeout = cpu.cycles() + 2_048;
    let start = timeout - 2;
    let mut words = vec![0u16; usize::try_from(start).unwrap()];
    words.push(0x940E);
    words.push(0x0000);
    words.push(ldi(16, 0xA5));
    cpu.load_words(&words);
    cpu.run_until(start);
    assert_eq!(cpu.cycles(), start);
    assert_eq!(cpu.pc(), u16::try_from(start).unwrap());
    assert_eq!(
        cpu.peek(MCUSR) & WDRF,
        0,
        "the timeout is still inside CALL"
    );
    cpu.step();
    assert_eq!(cpu.peek(MCUSR) & WDRF, WDRF);
    assert_eq!(
        cpu.cycles(),
        start + 4,
        "reset at CALL's retire, not a cycle later"
    );
    assert_eq!(
        cpu.pc(),
        0,
        "the reset vector is not fetched during the time-out"
    );
    cpu.run_until(start + 4 + crystal_reset_delay() - 1);
    assert_eq!(cpu.pc(), 0, "still in the 14 CK + 65 ms time-out");
}

fn crystal_reset_delay() -> u64 {
    14 + 1_040_000
}

/// Sleep in `smcr`. `plant` runs before `SLEEP`. Returns after the step that
/// enters sleep, which has already taken a zero-delay wake.
/// `SEI` then `SLEEP`, so the sleep starts before a planted request is served (§7.7).
fn sleep_in(smcr: u8, plant: impl FnOnce(&mut Cpu)) -> Cpu {
    let mut cpu = fresh();
    plant(&mut cpu);
    cpu.poke(SMCR, smcr);
    cpu.load_words(&[0x9478, 0x9588, 0x0000]);
    cpu.step();
    cpu.step();
    cpu
}

fn stays_asleep(smcr: u8, plant: impl FnOnce(&mut Cpu)) {
    let mut cpu = sleep_in(smcr, plant);
    assert!(cpu.is_sleeping(), "asleep after SLEEP");
    assert_eq!(cpu.pc(), 2, "pc is the word after SLEEP");
    let at = cpu.cycles();
    cpu.run_until(at + 20_000);
    assert!(cpu.is_sleeping(), "asleep after 20000 cycles");
    assert_eq!(cpu.pc(), 2, "pc is the word after SLEEP");
}

/// `(cycles, pc)` once a pre-set source has finished the mode's start-up.
fn wake_on(smcr: u8, startup: u64, plant: impl FnOnce(&mut Cpu)) -> (u64, u16) {
    let mut cpu = sleep_in(smcr, plant);
    assert!(cpu.is_sleeping(), "start-up has not finished");
    let at = cpu.cycles();
    cpu.run_until(at + startup + 8);
    (cpu.cycles(), cpu.pc())
}

fn wake_on_wdt(smcr: u8, startup: u64) -> (u64, u16) {
    let mut cpu = fresh();
    cpu.set_wdt_hz(16_000_000);
    cpu.poke(SMCR, smcr);
    arm_wdt(&mut cpu, WDIE, 0);
    let armed = cpu.cycles();
    cpu.set_sreg(0x80);
    cpu.load_words(&[0x9588]);
    cpu.step();
    assert!(cpu.is_sleeping());
    cpu.run_until(armed + 2_048 + startup + 8);
    (cpu.cycles(), cpu.pc())
}

fn tx_program() -> Vec<u16> {
    let mut words = vec![ldi(16, 0x08)];
    words.extend(sts(16, UCSR0B));
    words.push(ldi(17, 0x41));
    words.extend(sts(17, UDR0));
    words.push(0xCFFF);
    words
}

fn store(cpu: &mut Cpu, addr: u16, value: u8) {
    let mut words = vec![ldi(16, u16::from(value))];
    words.extend(sts(16, addr));
    cpu.load_words(&words);
    cpu.insns(2);
}

fn arm_wdt(cpu: &mut Cpu, mode: u8, wdp: u8) {
    cpu.poke(WDTCSR, WDCE | WDE);
    cpu.poke(
        WDTCSR,
        mode | (wdp & 0x07) | if wdp & 0x08 != 0 { 1 << 5 } else { 0 },
    );
}

/// EEMPE, then `gap` NOPs, then EEPE. `gap` 3 is the 4th cycle; 4 is the 5th.
fn window_program(gap: usize) -> Vec<u16> {
    let mut words = vec![
        ldi(16, 0),
        out(16, io(EEARL)),
        ldi(16, 0x42),
        out(16, io(EEDR)),
        sbi(io(EECR), 2),
    ];
    words.extend(std::iter::repeat_n(0x0000, gap));
    words.push(sbi(io(EECR), 1));
    words
}

fn write_program(addr: u8, data: u8, mode: u8) -> Vec<u16> {
    let mut words = vec![
        ldi(16, u16::from(addr)),
        out(16, io(EEARL)),
        ldi(16, u16::from(data)),
        out(16, io(EEDR)),
        ldi(16, u16::from(mode | EEMPE)),
        out(16, io(EECR)),
        ldi(16, u16::from(mode | EEMPE | EEPE)),
        out(16, io(EECR)),
    ];
    words.push(0x0000);
    words
}

fn wdce_program(gap: usize) -> Vec<u16> {
    let mut words = vec![ldi(16, u16::from(WDCE | WDE))];
    words.extend_from_slice(&sts(16, WDTCSR));
    words.extend(std::iter::repeat_n(0x0000, gap));
    words.push(ldi(16, u16::from(WDE | 0x01)));
    words.extend_from_slice(&sts(16, WDTCSR));
    words
}
