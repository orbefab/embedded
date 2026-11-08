//! GPIO inputs, external interrupts, pin change, and the timer input pins.
//!
//! Cycle numbers are the datasheet delays, not a trace of this model:
//! `PINx` at `T + 2` (DS40002061B §14.2.4), `T0`/`T1`/`ICP1` at `T + 4`
//! (§17.3, §16.6.1), and the noise canceler four clocks on top of that (§16.6.2).

use crate::common;
use common::Instructions;
use common::{PINB, TCCR0B, TCNT0, TIFR1, imm, in_io, ldi, lds, out, sbi, sts};

use orbe_avr::{Cpu, FuseBytes, Level};

const DDRD: u16 = 0x002A;
const PORTD: u16 = 0x002B;
const EIFR: u16 = 0x003C;
const EIMSK: u16 = 0x003D;
const PCIFR: u16 = 0x003B;
const MCUCR: u16 = 0x0055;
const PCICR: u16 = 0x0068;
const EICRA: u16 = 0x0069;
const PCMSK0: u16 = 0x006B;
const PCMSK1: u16 = 0x006C;
const PCMSK2: u16 = 0x006D;
const TCCR1A: u16 = 0x0080;
const TCCR1B: u16 = 0x0081;
const TCNT1L: u16 = 0x0084;
const TCNT1H: u16 = 0x0085;
const ICR1L: u16 = 0x0086;
const ICR1H: u16 = 0x0087;

fn nops(n: usize) -> Vec<u16> {
    vec![0; n]
}

/// §14.2.4 Figure 14-3. A change at cycle T is not visible to an `IN` in that
/// cycle or the next one. It is visible to the instruction that starts at T+2.
#[test]
fn an_input_change_is_not_visible_in_the_same_cycle() {
    let mut cpu = Cpu::new();
    cpu.drive_pin(0, Level::High);
    cpu.load_words(&[in_io(16, 0x03), in_io(16, 0x03), in_io(16, 0x03)]);
    cpu.step();
    assert_eq!(cpu.cycles(), 1);
    assert_eq!(
        cpu.reg(16) & 1,
        0,
        "IN in the change cycle reads the old pin"
    );
    cpu.step();
    assert_eq!(
        cpu.reg(16) & 1,
        0,
        "the next cycle is still inside the 1.5-clock window"
    );
    cpu.step();
    assert_eq!(cpu.cycles(), 3);
    assert_eq!(cpu.reg(16) & 1, 1, "visible at T+2");
}

/// §14.2.4 Figure 14-4. The OUT sets the latch; the following instruction still
/// reads the old `PINx`. The instruction after the nop reads the new value.
#[test]
fn a_software_pin_write_needs_a_nop_before_pin_reads_it() {
    let early = [ldi(16, 0x01), out(16, 0x04), out(16, 0x05), in_io(17, 0x03)];
    let mut cpu = Cpu::new();
    cpu.load_words(&early);
    cpu.insns(4);
    assert_eq!(
        cpu.reg(17) & 1,
        0,
        "IN immediately after OUT reads the old pin"
    );

    let waited = [
        ldi(16, 0x01),
        out(16, 0x04),
        out(16, 0x05),
        0x0000,
        in_io(17, 0x03),
    ];
    let mut cpu = Cpu::new();
    cpu.load_words(&waited);
    cpu.insns(5);
    assert_eq!(cpu.reg(17) & 1, 1, "IN after the nop reads the driven pin");
}

/// §14.2.1 and Table 14-1. Pull-up when PORT = 1, DDR = 0, PUD = 0.
/// A released pin with no pull-up reads 0. The die leaves that level undefined.
#[test]
fn a_released_pin_with_pullup_reads_high_and_without_reads_low() {
    let mut cpu = Cpu::new();
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(cpu.peek(PINB) & 1, 0, "floating input reads 0");

    cpu.load_words(&[
        ldi(16, 0x01),
        out(16, 0x05),
        0x0000,
        0x0000,
        in_io(17, 0x03),
    ]);
    cpu.insns(5);
    assert_eq!(cpu.reg(17) & 1, 1, "pull-up reads high");
    let report = cpu.take_pin().expect("pull-up edge");
    assert_eq!((report.pin, report.level), (0, Level::High));
}

/// §14.2.1. `MCUCR.PUD` disables every pull-up. The pin then reports released.
#[test]
fn pud_disables_pullups() {
    let mut cpu = Cpu::new();
    cpu.load_words(&[
        ldi(16, 0x01),
        out(16, 0x05),
        ldi(16, 1 << 4),
        out(16, 0x35),
        0x0000,
        0x0000,
        in_io(17, 0x03),
    ]);
    cpu.insns(7);
    assert_eq!(cpu.peek(MCUCR) & (1 << 4), 1 << 4);
    assert_eq!(cpu.reg(17) & 1, 0, "PUD wins over PORT");
    let levels: Vec<_> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.pin, report.level))
        .collect();
    assert!(
        levels.contains(&(0, Level::Released)),
        "clearing the pull-up releases the pin, got {levels:?}"
    );
}

/// Clearing `DDR` on a pin driven low, with `PORT` clear, reports released.
#[test]
fn clearing_ddr_reports_released() {
    let mut cpu = Cpu::new();
    cpu.load_words(&[
        ldi(16, 0x01),
        out(16, 0x04),
        ldi(17, 0x00),
        out(17, 0x04),
        out(17, 0x05),
    ]);
    cpu.insns(5);
    let levels: Vec<_> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.pin, report.level))
        .collect();
    assert_eq!(levels.last().copied(), Some((0, Level::Released)));
}

/// A host driving an output does not change `PINx`, which stays the MCU level.
#[test]
fn a_host_drive_on_an_output_still_reads_as_driven() {
    let mut cpu = Cpu::new();
    cpu.load_words(&[ldi(16, 0x01), out(16, 0x04), out(16, 0x05)]);
    cpu.insns(3);
    cpu.drive_pin(0, Level::Low);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(cpu.peek(PINB) & 1, 1, "the output wins the read");
}

/// §15.6. `PINx` reads the compare level, not `PORTx`, once the
/// synchronizer has caught the override. `OC0A` is `PD6`.
#[test]
fn pin_reads_the_compare_override() {
    let mut cpu = Cpu::new();
    cpu.poke(DDRD, 1 << 6);
    // COM0A = 3, then FOC0A. PORTD6 stays 0. Two nops cover the synchronizer.
    let program = [
        ldi(16, 0xC0),
        out(16, 0x24),
        ldi(17, 0x80),
        out(17, 0x25),
        0x0000,
        0x0000,
        in_io(18, 0x09),
    ];
    cpu.load_words(&program);
    cpu.insns(7);
    assert_eq!(cpu.peek(PORTD) & (1 << 6), 0, "PORT is still low");
    assert_eq!(cpu.reg(18) & (1 << 6), 1 << 6, "PIN reads OC0A");
}

fn arm_int0(cpu: &mut Cpu, isc: u8) {
    cpu.poke(EICRA, isc);
    cpu.poke(EIMSK, 0x01);
}

/// §13.2.1 Table 13-2. Each ISC mode, and low level leaves `INTF0` clear.
#[test]
fn each_int0_sense_mode_follows_the_table() {
    // Rising. The edge at T is sampled at T+2.
    let mut cpu = Cpu::new();
    arm_int0(&mut cpu, 0x03);
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(1));
    cpu.insns(1);
    assert_eq!(cpu.peek(EIFR) & 1, 0, "not yet at T+2");
    cpu.load_words(&nops(1));
    cpu.insns(1);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "rising sets INTF0");
    cpu.poke(EIFR, 0x00);
    cpu.drive_pin(18, Level::Low);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(
        cpu.peek(EIFR) & 1,
        0,
        "a falling edge leaves INTF0 clear when ISC is rising"
    );

    // Falling. A rising edge must not set the flag.
    let mut cpu = Cpu::new();
    arm_int0(&mut cpu, 0x02);
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(cpu.peek(EIFR) & 1, 0, "rising is not a falling edge");
    cpu.drive_pin(18, Level::Low);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "falling sets INTF0");

    // Any change. Both edges.
    let mut cpu = Cpu::new();
    arm_int0(&mut cpu, 0x01);
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(cpu.peek(EIFR) & 1, 1);
    cpu.poke(EIFR, 0x01);
    cpu.drive_pin(18, Level::Low);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "any change includes falling");

    // Low level. No flag. The request is the level itself.
    let mut cpu = Cpu::new();
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    arm_int0(&mut cpu, 0x00);
    cpu.set_sreg(0x80);
    cpu.load_words(&[0xCFFF]);
    cpu.insns(2);
    assert_eq!(cpu.pc(), 0, "high pin does not request low-level INT0");
    assert_eq!(cpu.peek(EIFR) & 1, 0, "low level keeps INTF0 clear");
    cpu.drive_pin(18, Level::Low);
    // One 2-cycle RJMP covers the synchronizer, then entry runs.
    cpu.insns(1);
    assert_eq!(cpu.pc(), 2, "low level enters the INT0 vector");
    assert_eq!(cpu.peek(EIFR) & 1, 0, "low level has no flag");
}

/// §13.2.3. Write-one-to-clear, and `SBI` clears only the named flag.
#[test]
fn eifr_clears_the_bits_that_are_written_one() {
    let mut cpu = Cpu::new();
    cpu.poke(EICRA, 0x0F);
    cpu.drive_pin(18, Level::High);
    cpu.drive_pin(19, Level::High);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(cpu.peek(EIFR) & 0x03, 0x03);
    cpu.load_words(&[ldi(16, 0x01), out(16, 0x1C)]);
    cpu.insns(2);
    assert_eq!(cpu.peek(EIFR) & 0x03, 0x02, "writing 1 clears only INTF0");

    let mut cpu = Cpu::new();
    cpu.poke(EICRA, 0x0F);
    cpu.drive_pin(18, Level::High);
    cpu.drive_pin(19, Level::High);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    cpu.load_words(&[sbi(0x1C, 0)]);
    cpu.step();
    assert_eq!(cpu.peek(EIFR) & 0x03, 0x02, "SBI INTF0 leaves INTF1 set");
}

/// §13.2.3. Vector entry clears the flag.
#[test]
fn vector_entry_clears_intf0() {
    let mut cpu = Cpu::new();
    arm_int0(&mut cpu, 0x03);
    cpu.set_sreg(0x80);
    cpu.load_words(&[0xCFFF]);
    cpu.drive_pin(18, Level::High);
    cpu.insns(1);
    assert_eq!(cpu.pc(), 2, "INT0 vector");
    assert_eq!(cpu.peek(EIFR) & 1, 0, "entry clears INTF0");
    assert_eq!(cpu.sreg() & 0x80, 0, "entry clears I");
}

/// §13. "even if the INT0 pin is configured as an output".
#[test]
fn an_output_toggle_on_int0_raises_int0() {
    let mut cpu = Cpu::new();
    cpu.poke(DDRD, 1 << 2);
    arm_int0(&mut cpu, 0x03);
    cpu.set_sreg(0x80);
    let program = [ldi(16, 1 << 2), out(16, 0x0B), 0xCFFF];
    cpu.load_words(&program);
    cpu.insns(3);
    assert_eq!(cpu.sreg() & 0x80, 0, "the output edge entered INT0");
    assert_eq!(cpu.sreg() & 0x80, 0);
}

/// §13.2.4–§13.2.8. Each 328P group, and the masked-off pin does not set the flag.
#[test]
fn each_328p_pcint_group_sets_its_own_flag() {
    let cases = [
        (0u8, PCMSK0, 0u32, 0x01u8),
        (1, PCMSK1, 8, 0x02),
        (2, PCMSK2, 16, 0x04),
    ];
    for (group, mask_addr, pin, flag) in cases {
        let mut cpu = Cpu::new();
        cpu.poke(PCICR, 1 << group);
        cpu.poke(mask_addr, 0x01);
        cpu.drive_pin(pin, Level::High);
        cpu.load_words(&nops(2));
        cpu.insns(2);
        assert_eq!(
            cpu.peek(PCIFR) & flag,
            flag,
            "group {group} flag after the synchronizer"
        );
        let mut quiet = Cpu::new();
        quiet.poke(PCICR, 1 << group);
        quiet.poke(mask_addr, 0x00);
        quiet.drive_pin(pin, Level::High);
        quiet.load_words(&nops(2));
        quiet.insns(2);
        assert_eq!(quiet.peek(PCIFR) & flag, 0, "mask bit gates group {group}");
    }
}

/// §13. Edge detection while the I/O clock is stopped. Once it runs, a low that lasts a cycle and
/// the rise after it set `INTF0`.
#[test]
fn edges_wait_while_the_io_clock_is_stopped() {
    let mut cpu = Cpu::new();
    arm_int0(&mut cpu, 0x03);
    orbe_avr::test_hooks::set_io_clock_running(&mut cpu, false);
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(4));
    cpu.insns(4);
    assert_eq!(
        cpu.peek(EIFR) & 1,
        0,
        "no edge flag while clkI/O is stopped"
    );
    orbe_avr::test_hooks::set_io_clock_running(&mut cpu, true);
    cpu.drive_pin(18, Level::Low);
    cpu.load_words(&nops(1));
    cpu.insns(1);
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(2));
    cpu.insns(2);
    assert_eq!(
        cpu.peek(EIFR) & 1,
        1,
        "the clock running again samples the edge"
    );
}

fn plant_tcnt1(cpu: &mut Cpu, value: u16) {
    cpu.poke(TCNT1H, (value >> 8) as u8);
    cpu.poke(TCNT1L, value as u8);
}

fn read_icr_program(nops_first: usize) -> Vec<u16> {
    let mut words = nops(nops_first);
    words.extend(common::lds(16, ICR1L));
    words.extend(common::lds(17, ICR1H));
    words
}

/// §16.6. Without the noise canceler, ICR and ICF update 4 cycles after the edge.
#[test]
fn icp1_captures_four_cycles_after_the_selected_edge() {
    let mut cpu = Cpu::new();
    plant_tcnt1(&mut cpu, 0x1234);
    cpu.poke(TCCR1B, 0x40);
    cpu.load_words(&read_icr_program(4));
    cpu.drive_pin(0, Level::High);
    cpu.insns(3);
    assert_eq!(cpu.cycles(), 3);
    assert_eq!(cpu.peek(TIFR1) & 0x20, 0, "not yet at T+4");
    cpu.insns(3);
    assert_eq!(cpu.peek(TIFR1) & 0x20, 0x20, "ICF1 at T+4");
    let icr = u16::from(cpu.reg(16)) | (u16::from(cpu.reg(17)) << 8);
    assert_eq!(icr, 0x1234, "ICR1 is TCNT1 at the capture");

    let mut fell = Cpu::new();
    plant_tcnt1(&mut fell, 0x00AB);
    fell.poke(TCCR1B, 0x00);
    fell.drive_pin(0, Level::High);
    fell.run_until(4);
    assert_eq!(fell.peek(TIFR1) & 0x20, 0, "ICES1 = 0 ignores the rise");
    fell.drive_pin(0, Level::Low);
    let at = fell.cycles();
    fell.run_until(at + 3);
    assert_eq!(fell.peek(TIFR1) & 0x20, 0);
    fell.run_until(at + 4);
    assert_eq!(fell.peek(TIFR1) & 0x20, 0x20, "falling edge at T+4");
}

/// §16.6.2. Four samples, then the same 4-cycle edge delay: capture at T+8.
/// A pulse that does not hold for four samples does not capture.
#[test]
fn icp1_noise_canceler_adds_four_clocks_and_rejects_a_short_pulse() {
    let mut cpu = Cpu::new();
    plant_tcnt1(&mut cpu, 0x2222);
    cpu.poke(TCCR1B, 0xC0);
    cpu.load_words(&read_icr_program(8));
    cpu.drive_pin(0, Level::High);
    cpu.insns(7);
    assert_eq!(
        cpu.peek(TIFR1) & 0x20,
        0,
        "T+7 is before the canceler delay"
    );
    cpu.insns(3);
    assert_eq!(cpu.peek(TIFR1) & 0x20, 0x20, "capture at T+8");
    let icr = u16::from(cpu.reg(16)) | (u16::from(cpu.reg(17)) << 8);
    assert_eq!(icr, 0x2222);

    let mut glitch = Cpu::new();
    plant_tcnt1(&mut glitch, 0x0001);
    glitch.poke(TCCR1B, 0xC0);
    glitch.drive_pin(0, Level::High);
    glitch.run_until(2);
    glitch.drive_pin(0, Level::Low);
    glitch.run_until(20);
    assert_eq!(
        glitch.peek(TIFR1) & 0x20,
        0,
        "three cycles is not four samples"
    );
}

/// §17.3. Rising `T0` increments `TCNT0` at T+4, not at T+3. Falling is clock-select 6.
/// The stepped loop and `run_until` both match that count.
#[test]
fn t0_and_t1_count_four_cycles_after_the_selected_edge() {
    let count_at = |stepped: bool, until: u64| -> u8 {
        let mut cpu = Cpu::new();
        cpu.poke(TCCR0B, 0x07);
        cpu.drive_pin(20, Level::High);
        if stepped {
            while cpu.cycles() < until {
                cpu.step();
            }
        } else {
            cpu.run_until(until);
        }
        cpu.peek(TCNT0)
    };
    assert_eq!(count_at(false, 3), 0, "T+3 is inside the 2.5-to-3.5 window");
    assert_eq!(count_at(true, 3), 0, "stepped path agrees at T+3");
    assert_eq!(count_at(false, 4), 1, "TCNT0 increments at T+4");
    assert_eq!(count_at(true, 4), 1, "stepped path agrees at T+4");

    let mut fell = Cpu::new();
    fell.poke(TCCR0B, 0x06);
    fell.drive_pin(20, Level::High);
    fell.run_until(4);
    assert_eq!(fell.peek(TCNT0), 0, "clock-select 6 ignores the rise");
    fell.drive_pin(20, Level::Low);
    let at = fell.cycles();
    fell.run_until(at + 4);
    assert_eq!(fell.peek(TCNT0), 1, "falling T0 edge");

    let mut t1 = Cpu::new();
    t1.poke(TCCR1B, 0x07);
    t1.drive_pin(21, Level::High);
    t1.run_until(3);
    assert_eq!(t1.peek(TCNT1L), 0);
    t1.run_until(4);
    assert_eq!(t1.peek(TCNT1L), 1, "T1 is PD5");
}

/// §13.2.5. `PCIFR` is write-one-to-clear and clears on entry.
#[test]
fn pcifr_clears_on_write_one_and_on_entry() {
    let mut cpu = Cpu::new();
    cpu.poke(PCICR, 0x01);
    cpu.poke(PCMSK0, 0x01);
    cpu.set_sreg(0x80);
    cpu.load_words(&[0xCFFF]);
    cpu.drive_pin(0, Level::High);
    cpu.insns(1);
    assert_eq!(cpu.pc(), 6, "PCINT0 vector is word 6");
    assert_eq!(cpu.peek(PCIFR) & 1, 0, "entry clears PCIF0");
}

/// Figure 14-3. One transition is late by up to 1½ clocks, so the first
/// instruction that must see it starts at T+2. A second transition has its
/// own T+2. The latch does not publish a level that has not finished that
/// delay, and it does not drop a level that still has its own window.
///
/// High then low in the same cycle is one sample, and the last level wins.
/// Released and low both read 0, so `IN` at cycles 0, 1, 2, 3 is 0, 0, 0, 0.
///
/// High at cycle 0 and low at cycle 1: `IN` at 0, 1, 2, 3 is 0, 0, 1, 0.
/// The high is first visible at cycle 2 and the low at cycle 3.
#[test]
fn a_second_edge_inside_the_synchronizer_keeps_its_own_delay() {
    let mut cpu = Cpu::new();
    cpu.drive_pin(0, Level::High);
    cpu.drive_pin(0, Level::Low);
    cpu.load_words(&nops(4));
    let mut same_cycle = Vec::new();
    for _ in 0..4 {
        same_cycle.push(cpu.peek(PINB) & 1);
        cpu.step();
    }
    assert_eq!(same_cycle, vec![0, 0, 0, 0], "one sample, the final low");

    let mut cpu = Cpu::new();
    cpu.drive_pin(0, Level::High);
    cpu.load_words(&nops(4));
    assert_eq!(cpu.peek(PINB) & 1, 0, "cycle 0 is still the old level");
    cpu.step();
    cpu.drive_pin(0, Level::Low);
    assert_eq!(
        cpu.peek(PINB) & 1,
        0,
        "cycle 1 does not show the high early"
    );
    cpu.step();
    assert_eq!(cpu.peek(PINB) & 1, 1, "cycle 2 shows the high");
    cpu.step();
    assert_eq!(cpu.peek(PINB) & 1, 0, "cycle 3 shows the low");
}

/// §16.11.2. Bits 7:6 of `TCCR1B` are `ICNC1` and `ICES1` and read back.
/// `TCCR0B` bits 7:6 are `FOC0A`/`FOC0B` and read as zero. A read-modify-write
/// of the form `TCCR1B |= CS10` keeps the edge select, so the rising edge
/// still captures.
#[test]
fn tccr1b_reads_back_icnc_and_ices() {
    let mut eight = Cpu::new();
    eight.load_words(&[ldi(16, 0xC0), out(16, 0x25), in_io(17, 0x25)]);
    eight.insns(3);
    assert_eq!(eight.reg(17), 0, "FOC0A and FOC0B read as zero");

    assert_capture_survives_read_modify(&mut Cpu::new(), 0);
}

fn assert_capture_survives_read_modify(cpu: &mut Cpu, icp_pin: u32) {
    let mut program = Vec::new();
    program.push(ldi(16, 0xC0));
    program.extend(sts(16, TCCR1B));
    program.extend(lds(17, TCCR1B));
    program.extend(lds(18, TCCR1B));
    program.push(imm(0x6000, 18, 0x01));
    program.extend(sts(18, TCCR1B));
    cpu.load_words(&program);
    cpu.insns(6);
    assert_eq!(cpu.cycles(), 10);
    assert_eq!(cpu.reg(17), 0xC0, "LDS of TCCR1B returns ICNC1 and ICES1");
    assert_eq!(cpu.peek(TCCR1B), 0xC1, "the OR of CS10 keeps bits 7:6");
    let at = cpu.cycles();
    cpu.drive_pin(icp_pin, Level::High);
    cpu.run_until(at + 7);
    assert_eq!(cpu.peek(TIFR1) & 0x20, 0, "ICNC1 still delays the capture");
    cpu.run_until(at + 8);
    assert_eq!(
        cpu.peek(TIFR1) & 0x20,
        0x20,
        "ICES1 still captures the rising edge"
    );
}

/// §16.6. Each detected edge produces its own capture, 4 cycles later (the
/// first boundary past the 2.5-to-3.5-clock window). A second capture
/// overwrites `ICR1` when software has not read it. There is no overflow flag
/// for that overwrite.
#[test]
fn each_icp_edge_captures_and_the_next_overwrites_icr() {
    let mut cpu = Cpu::new();
    plant_tcnt1(&mut cpu, 0x0011);
    cpu.poke(TCCR1B, 0x40);
    cpu.load_words(&nops(8));
    cpu.drive_pin(0, Level::High);
    cpu.run_until(1);
    cpu.drive_pin(0, Level::Low);
    cpu.run_until(2);
    cpu.drive_pin(0, Level::High);
    cpu.run_until(4);
    assert_eq!(
        cpu.peek(TIFR1) & 0x20,
        0x20,
        "the edge at cycle 0 captures at T+4"
    );
    assert_eq!(cpu.peek(ICR1L), 0x11, "ICR1 is TCNT1 at that capture");
    cpu.poke(TCNT1H, 0x00);
    cpu.poke(TCNT1L, 0x22);
    cpu.run_until(6);
    assert_eq!(cpu.peek(ICR1L), 0x22, "the edge at cycle 2 overwrites ICR1");
    assert_eq!(cpu.peek(TIFR1), 0x20, "the overwrite sets no overflow flag");
}

/// §16.6.1. Input capture is disconnected in the modes where `ICR1` is TOP
/// (8, 10, 12, and 14). An edge leaves `ICR1` and `ICF1` alone.
#[test]
fn icp_is_disconnected_while_icr1_is_top() {
    // WGM13:0. `TCCR1A` holds WGM11:10. `TCCR1B` bits 4:3 are WGM13:12.
    // `ICES1` is set so a connected pin would capture the rising edge.
    let modes = [
        (8u8, 0x00u8, 0x10u8),
        (10, 0x02, 0x10),
        (12, 0x00, 0x18),
        (14, 0x02, 0x18),
    ];
    for (mode, tccra, tccrb) in modes {
        let mut cpu = Cpu::new();
        cpu.poke(TCCR1A, tccra);
        cpu.poke(TCCR1B, tccrb | 0x40);
        cpu.poke(ICR1H, 0x12);
        cpu.poke(ICR1L, 0x34);
        cpu.drive_pin(0, Level::High);
        cpu.load_words(&nops(8));
        cpu.run_until(8);
        assert_eq!(cpu.peek(TIFR1) & 0x20, 0, "mode {mode} leaves ICF1 clear");
        cpu.load_words(&read_icr_program(0));
        cpu.insns(2);
        let icr = u16::from(cpu.reg(16)) | (u16::from(cpu.reg(17)) << 8);
        assert_eq!(icr, 0x1234, "mode {mode} leaves ICR1");
    }
}

/// §13.2.5. `PCIFR` is write-one-to-clear. `OUT`, `STS`, and `SBI` clear only
/// the bits written as one.
#[test]
fn pcifr_write_one_clears_only_the_written_bits() {
    let arm = || {
        let mut cpu = Cpu::new();
        cpu.poke(PCICR, 0x07);
        cpu.poke(PCMSK0, 0x01);
        cpu.poke(PCMSK1, 0x01);
        cpu.poke(PCMSK2, 0x01);
        cpu.drive_pin(0, Level::High);
        cpu.drive_pin(8, Level::High);
        cpu.drive_pin(16, Level::High);
        cpu.load_words(&nops(2));
        cpu.insns(2);
        assert_eq!(cpu.peek(PCIFR), 0x07, "all three groups are pending");
        cpu
    };

    let mut cpu = arm();
    cpu.load_words(&[ldi(16, 0x01), out(16, 0x1B)]);
    cpu.insns(2);
    assert_eq!(cpu.peek(PCIFR), 0x06, "OUT of 0x01 clears only PCIF0");

    let mut cpu = arm();
    let mut program = vec![ldi(16, 0x02)];
    program.extend(sts(16, PCIFR));
    cpu.load_words(&program);
    cpu.insns(2);
    assert_eq!(cpu.peek(PCIFR), 0x05, "STS of 0x02 clears only PCIF1");

    let mut cpu = arm();
    cpu.load_words(&[sbi(0x1B, 2)]);
    cpu.step();
    assert_eq!(cpu.peek(PCIFR), 0x03, "SBI PCIF2 clears only that bit");
}

/// §17.3. `T0` is sampled as a pin. Driving it as an output still clocks the
/// counter. The count is 0 at T+3 and 1 at T+4.
#[test]
fn t0_as_an_output_still_clocks_the_counter() {
    let mut cpu = Cpu::new();
    cpu.poke(DDRD, 1 << 4);
    cpu.poke(TCCR0B, 0x07);
    cpu.set_reg(16, 1 << 4);
    cpu.load_words(&[out(16, 0x0B), 0, 0, 0, 0]);
    cpu.run_until(3);
    assert_eq!(cpu.peek(TCNT0), 0, "T+3 is inside the 2.5-to-3.5 window");
    cpu.run_until(4);
    assert_eq!(
        cpu.peek(TCNT0),
        1,
        "the output edge increments TCNT0 at T+4"
    );
}

/// §13.2.1 Table 13-2. A sample that comes due while `clkI/O` is stopped does
/// not set `INTF0`, and the sampled level is what the next edge compares
/// against. Any-change: a rise at cycle 0, the clock stopped at 1, then a
/// falling edge once the clock runs sets `INTF0` two cycles later. Low level:
/// the pad is low through that stopped sample, and the request is active once
/// the clock runs.
#[test]
fn a_stopped_sample_keeps_the_level_and_sets_no_flag() {
    let mut cpu = Cpu::new();
    arm_int0(&mut cpu, 0x01);
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(8));
    cpu.run_until(1);
    orbe_avr::test_hooks::set_io_clock_running(&mut cpu, false);
    cpu.run_until(4);
    assert_eq!(
        cpu.peek(EIFR) & 1,
        0,
        "INTF0 stays clear through the sample"
    );
    orbe_avr::test_hooks::set_io_clock_running(&mut cpu, true);
    let at = cpu.cycles();
    cpu.drive_pin(18, Level::Low);
    cpu.run_until(at + 1);
    assert_eq!(cpu.peek(EIFR) & 1, 0, "not yet two cycles after the fall");
    cpu.run_until(at + 2);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "the falling edge sets INTF0");

    let mut low = Cpu::new();
    low.drive_pin(18, Level::High);
    low.load_words(&nops(4));
    low.run_until(2);
    arm_int0(&mut low, 0x00);
    low.set_sreg(0);
    let at = low.cycles();
    low.drive_pin(18, Level::Low);
    low.run_until(at + 1);
    orbe_avr::test_hooks::set_io_clock_running(&mut low, false);
    low.run_until(at + 4);
    assert_eq!(low.peek(EIFR) & 1, 0, "low level has no flag");
    orbe_avr::test_hooks::set_io_clock_running(&mut low, true);
    low.set_sreg(0x80);
    low.load_words(&[0xCFFF]);
    low.insns(1);
    assert_eq!(low.pc(), 2, "low level requests INT0 once the clock runs");
}

/// §13. An edge sampled while `clkI/O` is running does not set `INTF0` if the
/// clock has stopped before that sample comes due.
#[test]
fn a_queued_edge_does_not_flag_after_the_io_clock_stops() {
    let mut cpu = Cpu::new();
    arm_int0(&mut cpu, 0x03);
    cpu.drive_pin(18, Level::High);
    cpu.load_words(&nops(8));
    cpu.run_until(1);
    orbe_avr::test_hooks::set_io_clock_running(&mut cpu, false);
    cpu.run_until(4);
    assert_eq!(
        cpu.peek(EIFR) & 1,
        0,
        "INTF0 stays clear at the sample time"
    );
    cpu.run_until(8);
    assert_eq!(
        cpu.peek(EIFR) & 1,
        0,
        "the dropped sample does not arrive later"
    );
}

const PB0: u32 = 0;
const PD2: u32 = 18;

/// `INT0` armed with `isc` and no `EIMSK`, `PD2` high, `INTF0` clear, at cycle 30 of a flash of
/// `NOP`s, so every run limit lands on its cycle.
fn int0_on_nops(isc: u8) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.load_words(&nops(1));
    cpu.poke(EICRA, isc);
    cpu.drive_pin(PD2, Level::High);
    cpu.run_until(20);
    cpu.poke(EIFR, 0);
    cpu.run_until(30);
    cpu
}

/// Figure 14-3. Forty drives of `PB1` in one cycle are one sample, so a fall of `PD2` a cycle
/// later still reaches `INT0`.
#[test]
fn an_int0_fall_after_forty_drives_of_another_pin_sets_intf0() {
    let mut cpu = int0_on_nops(0x02);
    for n in 0..40 {
        cpu.drive_pin(1, if n % 2 == 0 { Level::Low } else { Level::High });
    }
    let c = cpu.cycles();
    cpu.run_until(c + 1);
    cpu.drive_pin(PD2, Level::Low);
    cpu.run_until(c + 20);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "the fall reaches INTF0");
}

/// Figure 14-3. Every level that lasts a cycle reaches the edge detector, however many pins
/// change. Every watched pin but `PC6`, which is `RESET`, moves on two cycles in a row, the most
/// samples that can wait at once. Each run watches one pin in its group, so a flag after each
/// cycle shows that pin's sample arrived. `PD2` moves last, and `INT0` flags any change.
#[test]
fn every_pin_changing_on_two_cycles_reaches_its_flags() {
    let pins: Vec<u32> = (0..14)
        .chain(16..24)
        .filter(|&pin| pin != PD2)
        .chain([PD2])
        .collect();
    for &watched in &pins {
        let mut cpu = Cpu::new();
        cpu.load_words(&nops(1));
        cpu.poke(EICRA, 0x01);
        cpu.poke(PCICR, 0x07);
        cpu.poke(PCMSK0 + (watched / 8) as u16, 1 << (watched % 8));
        cpu.run_until(30);
        let group = 1 << (watched / 8);
        for (c, level) in [(30, Level::High), (31, Level::Low)] {
            for &pin in &pins {
                cpu.drive_pin(pin, level);
            }
            cpu.run_until(c + 1);
        }
        assert_eq!(cpu.peek(EIFR) & 1, 1, "the rise of PD2 reaches INTF0");
        assert_eq!(
            cpu.peek(PCIFR),
            group,
            "the rise of pin {watched} sets its flag"
        );
        cpu.poke(EIFR, 0);
        cpu.poke(PCIFR, 0);
        cpu.run_until(50);
        assert_eq!(cpu.peek(EIFR) & 1, 1, "the fall of PD2 reaches INTF0");
        assert_eq!(
            cpu.peek(PCIFR),
            group,
            "the fall of pin {watched} sets its flag"
        );
    }
}

/// Figure 14-3. A low that lasts a cycle, then high, low and high in the next. Both levels last
/// a cycle, so `INT0` on a rising edge flags.
#[test]
fn a_return_to_the_sampled_level_behind_a_waiting_sample_sets_intf0() {
    let mut cpu = int0_on_nops(0x03);
    let c = cpu.cycles();
    cpu.drive_pin(PD2, Level::Low);
    cpu.run_until(c + 1);
    for level in [Level::High, Level::Low, Level::High] {
        cpu.drive_pin(PD2, level);
    }
    cpu.run_until(c + 20);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "the rise reaches INTF0");
}

/// Figure 14-3. Low, high and low in one cycle is one sample, the last low. `INT0` on a rising
/// edge sees a fall and no rise, and the next rise sets `INTF0`.
#[test]
fn three_drives_in_one_cycle_keep_the_last_level() {
    let mut cpu = int0_on_nops(0x03);
    for level in [Level::Low, Level::High, Level::Low] {
        cpu.drive_pin(PD2, level);
    }
    let c = cpu.cycles();
    cpu.run_until(c + 20);
    assert_eq!(cpu.peek(EIFR) & 1, 0, "a net fall flags no rise");
    cpu.drive_pin(PD2, Level::High);
    cpu.run_until(c + 40);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "the rise reaches INTF0");
}

/// Figure 14-3. A low shorter than a cycle never reaches the synchronizer, so neither `INT0` on
/// a falling edge nor `PCINT18` flags it. A low that lasts one cycle sets both.
#[test]
fn a_pulse_shorter_than_a_cycle_sets_no_flag() {
    for (cycles, flagged) in [(0, 0), (1, 1)] {
        let mut cpu = int0_on_nops(0x02);
        cpu.poke(PCICR, 1 << 2);
        cpu.poke(PCMSK2, 1 << 2);
        let c = cpu.cycles();
        cpu.drive_pin(PD2, Level::Low);
        cpu.run_until(c + cycles);
        cpu.drive_pin(PD2, Level::High);
        cpu.run_until(c + 20);
        assert_eq!(
            cpu.peek(EIFR) & 1,
            flagged,
            "INTF0 after a low of {cycles} cycles"
        );
        assert_eq!(
            cpu.peek(PCIFR) & (1 << 2),
            flagged << 2,
            "PCIF2 after a low of {cycles} cycles"
        );
    }
}

/// §11.1. While `RESET` is held the flags keep their reset value of 0, and a pin that moves
/// leaves only its last level in the synchronizer. Release flags nothing, even with `INT0` on a
/// rising edge and `PCINT18` armed by the host during the hold. The next rise sets both.
#[test]
fn a_pin_that_moves_while_reset_is_held_sets_no_flag_at_release() {
    for armed in [false, true] {
        let mut cpu = Cpu::new();
        cpu.set_fuses(
            FuseBytes::CRYSTAL.low,
            FuseBytes::CRYSTAL.high,
            FuseBytes::CRYSTAL.extended,
        );
        cpu.load_words(&nops(1));
        cpu.run_until(30);
        cpu.pull_reset();
        if armed {
            cpu.poke(EICRA, 0x03);
            cpu.poke(PCICR, 1 << 2);
            cpu.poke(PCMSK2, 1 << 2);
        }
        for n in 0..40 {
            cpu.drive_pin(PD2, if n % 2 == 0 { Level::High } else { Level::Low });
        }
        for n in 0..80 {
            cpu.run_until(cpu.cycles() + 1);
            cpu.drive_pin(PD2, if n % 2 == 0 { Level::High } else { Level::Low });
        }
        cpu.release_reset();
        let start = cpu.cycles() + 14 + 1_040_000;
        cpu.run_until(start + 20);
        assert_ne!(cpu.pc(), 0, "the core runs");
        assert_eq!(cpu.peek(EIFR), 0, "release sets no INTF, armed {armed}");
        assert_eq!(cpu.peek(PCIFR), 0, "release sets no PCIF, armed {armed}");
        if armed {
            cpu.drive_pin(PD2, Level::High);
            cpu.run_until(start + 40);
            assert_eq!(cpu.peek(EIFR) & 1, 1, "the next rise reaches INTF0");
            assert_eq!(cpu.peek(PCIFR), 1 << 2, "the next rise sets PCIF2");
        }
    }
}

/// §11.1. A pad the host holds high across a held reset is the sampled level at release, so a
/// fall after release sets `INTF0` on a falling edge.
#[test]
fn a_high_held_across_reset_falls_to_intf0() {
    let mut cpu = Cpu::new();
    cpu.load_words(&nops(1));
    cpu.drive_pin(PD2, Level::High);
    cpu.run_until(30);
    cpu.pull_reset();
    cpu.run_until(cpu.cycles() + 10);
    cpu.release_reset();
    cpu.run_until(cpu.cycles() + 2_000_000);
    assert_ne!(cpu.pc(), 0, "the core runs");
    cpu.poke(EICRA, 0x02);
    let c = cpu.cycles();
    cpu.drive_pin(PD2, Level::Low);
    cpu.run_until(c + 10);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "the fall reaches INTF0");
}

/// §11.1. A rise driven during a held reset is the sampled level from the cycle its sample comes
/// due, so a fall after release sets `INTF0` on a falling edge.
#[test]
fn a_rise_due_on_the_last_cycle_of_a_held_reset_falls_to_intf0() {
    let mut cpu = Cpu::new();
    cpu.load_words(&nops(1));
    cpu.run_until(30);
    cpu.pull_reset();
    cpu.run_until(cpu.cycles() + 10);
    let rise = cpu.cycles();
    cpu.drive_pin(PD2, Level::High);
    cpu.run_until(rise + 2);
    cpu.release_reset();
    cpu.run_until(cpu.cycles() + 2_000_000);
    assert_ne!(cpu.pc(), 0, "the core runs");
    cpu.poke(EICRA, 0x02);
    let c = cpu.cycles();
    cpu.drive_pin(PD2, Level::Low);
    cpu.run_until(c + 10);
    assert_eq!(cpu.peek(EIFR) & 1, 1, "the fall reaches INTF0");
}

/// §11.1. A pad the host holds high across a held reset is the sampled level at release on a
/// pin-change pin too, so a fall of `PB0` after release sets `PCIF0`.
#[test]
fn a_high_held_across_reset_falls_to_pcif0() {
    let mut cpu = Cpu::new();
    cpu.load_words(&nops(1));
    cpu.drive_pin(PB0, Level::High);
    cpu.run_until(30);
    cpu.pull_reset();
    cpu.run_until(cpu.cycles() + 10);
    cpu.release_reset();
    cpu.run_until(cpu.cycles() + 2_000_000);
    assert_ne!(cpu.pc(), 0, "the core runs");
    cpu.poke(PCICR, 1);
    cpu.poke(PCMSK0, 1);
    let c = cpu.cycles();
    cpu.drive_pin(PB0, Level::Low);
    cpu.run_until(c + 10);
    assert_eq!(cpu.peek(PCIFR) & 1, 1, "the fall reaches PCIF0");
}

/// Figure 14-3. A rise on the last cycle of the start-up time-out reaches the edge detector one
/// cycle after the core starts. An `STS EICRA` at the reset vector has armed a rising edge by
/// then, so the rise sets `INTF0`. Reset clears the register file, so the host sets `r16`.
#[test]
fn a_rise_on_the_last_cycle_of_the_time_out_reaches_a_first_sts_to_eicra() {
    let mut cpu = Cpu::new();
    let fuses = cpu.fuses();
    // Internal RC, `CKDIV8` unprogrammed, `SUT = 00`. The time-out is 14 CK (Table 9-12).
    cpu.set_fuses(0xC2, fuses.high, fuses.extended);
    let mut program = sts(16, EICRA).to_vec();
    program.extend(nops(4));
    cpu.load_words(&program);
    cpu.drive_pin(PD2, Level::Low);
    cpu.pull_reset();
    cpu.run_until(cpu.cycles() + 10);
    cpu.release_reset();
    let start = cpu.cycles() + 14;
    cpu.set_reg(16, 0x03);
    cpu.run_until(start - 1);
    cpu.drive_pin(PD2, Level::High);
    cpu.run_until(start);
    assert_eq!(cpu.pc(), 0, "the time-out holds the core until {start}");
    cpu.run_until(start + 4);
    assert_eq!(
        cpu.peek(EICRA),
        0x03,
        "the first instruction armed a rising edge"
    );
    assert_eq!(cpu.peek(EIFR) & 1, 1, "the rise reaches INTF0");
}
