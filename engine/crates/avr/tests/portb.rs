//! A program image drives PORTB bit 5. The instruction loop does not stop.

use crate::common;
use common::Instructions;
use common::{imm, out, sbi};

use sim_avr::{Channel, Cpu, Level, PinDrive};

#[test]
fn image_toggles_pin_5() {
    // LDI r16, 0x20
    // OUT DDRB, r16   ; bit 5 is an output, driven low
    // OUT PORTB, r16  ; driven high
    // LDI r16, 0
    // OUT PORTB, r16  ; driven low
    let program = [
        imm(0xE000, 16, 0x20),
        out(16, 0x04),
        out(16, 0x05),
        imm(0xE000, 16, 0x00),
        out(16, 0x05),
    ];
    let mut cpu = Cpu::new();
    cpu.load_words(&program);
    cpu.insns(5);
    assert!(!cpu.is_stopped());
    assert_eq!(cpu.pc(), 5);
    let levels: Vec<Level> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| {
            assert_eq!(report.pin, 5);
            report.level
        })
        .collect();
    assert_eq!(levels, vec![Level::Low, Level::High, Level::Low]);
}

#[test]
fn other_port_bits_do_not_report_pin_5() {
    // PB0 is an output. PORTC bit 0 while PC0 is an input enables the pull-up
    // (DS40002061B Table 14-1), so that pin reports high. Pin 5 does not.
    let program = [
        imm(0xE000, 16, 0x01),
        out(16, 0x04),
        out(16, 0x05),
        out(16, 0x08),
    ];
    let mut cpu = Cpu::new();
    cpu.load_words(&program);
    cpu.insns(4);
    let reports: Vec<_> = std::iter::from_fn(|| cpu.take_pin()).collect();
    assert!(reports.iter().all(|report| report.pin != 5));
    assert_eq!(
        reports
            .iter()
            .map(|report| (report.pin, report.level))
            .collect::<Vec<_>>(),
        vec![(0, Level::Low), (0, Level::High), (8, Level::High)]
    );
}

#[test]
fn write_one_to_pinb_toggles_the_driven_pin() {
    // Both bit 0 and bit 5 are outputs. SBI must toggle only the bit it names.
    let program = [
        imm(0xE000, 16, 0x21),
        out(16, 0x04),
        out(16, 0x05),
        imm(0xE000, 16, 0x20),
        out(16, 0x03),
        imm(0xE000, 16, 0x00),
        out(16, 0x03),
        sbi(0x03, 5),
    ];
    let mut cpu = Cpu::new();
    cpu.load_words(&program);
    cpu.insns(8);
    assert_eq!(cpu.pc(), 8);
    assert_eq!(cpu.peek(0x0025), 0x21);
    assert_eq!(cpu.peek(0x0023), 0x21);
    let edges: Vec<_> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.pin, report.level))
        .collect();
    // DDR 0x21 drives PB0 and PB5 low, PORT 0x21 drives both high, then only bit 5 toggles.
    assert_eq!(
        edges,
        vec![
            (0, Level::Low),
            (5, Level::Low),
            (0, Level::High),
            (5, Level::High),
            (5, Level::Low),
            (5, Level::High),
        ]
    );
}

#[test]
fn pinb_toggle_on_an_input_enables_the_pullup() {
    // Writing 1 to PINB toggles PORTB. DDR is clear, so bit 0 becomes a pull-up
    // (DS40002061B Table 14-1) and the pin reports high. The PINx synchronizer
    // (§14.2.4) has not caught up by the end of the one-cycle OUT.
    let program = [imm(0xE000, 16, 0x01), out(16, 0x03)];
    let mut cpu = Cpu::new();
    cpu.load_words(&program);
    cpu.insns(2);
    assert_eq!(cpu.peek(0x0025), 0x01);
    assert_eq!(
        cpu.peek(0x0023),
        0x00,
        "the OUT's own cycle still reads the old pin"
    );
    let report = cpu.take_pin().expect("pull-up");
    assert_eq!((report.pin, report.level), (0, Level::High));
}

#[test]
fn poke_does_not_report() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 0x20);
    cpu.poke(0x25, 0x20);
    // A poke of PINB is not the toggle a store does.
    cpu.poke(0x23, 0x20);
    assert_eq!(cpu.peek(0x0025), 0x20, "PINB poke leaves PORTB");
    assert_eq!(cpu.take_pin(), None);
}

const MCUCR: u16 = 0x0055;
const EICRA: u16 = 0x0069;
const SPCR: u16 = 0x004C;
const SPDR: u16 = 0x004E;
const ACSR: u16 = 0x0050;
const ADCSRA: u16 = 0x007A;
const DIDR0: u16 = 0x007E;
const DIDR1: u16 = 0x007F;

/// GPIO, the external interrupts, the ADC, and the comparator.
///
/// A pin report lands on the next tick, so the NOPs are what make a leaked
/// report visible. TWI is the known exception: its poke runs the store and
/// the launch on the real context, and the TWI tests drive the master that way.
#[test]
fn poke_of_rerouted_devices_is_quiet() {
    let mut cpu = Cpu::new();
    cpu.poke(common::DDRB, 0x00);
    cpu.poke(common::PORTB, 0x01);
    cpu.poke(EICRA, 0x02);
    cpu.poke(ADCSRA, 0x80);
    cpu.poke(DIDR0, 0x01);
    cpu.poke(ACSR, 0x80);
    cpu.poke(DIDR1, 0x03);
    cpu.load_words(&[0x0000, 0x0000]);
    cpu.insns(2);

    assert_eq!(cpu.peek(common::PORTB), 0x01);
    assert_eq!(cpu.peek(EICRA), 0x02);
    assert_eq!(cpu.peek(ADCSRA) & 0x80, 0x80);
    assert_eq!(cpu.peek(DIDR0), 0x01);
    assert_eq!(cpu.peek(ACSR) & 0x80, 0x80);
    assert_eq!(cpu.peek(DIDR1), 0x03);
    assert_eq!(cpu.take_pin(), None);
    assert!(cpu.io_writes().is_empty());

    // A store reports the pull-up. The following PUD poke must not report
    // its removal.
    let mut pud = Cpu::new();
    pud.load_words(&[imm(0xE000, 16, 0x01), out(16, 0x05)]);
    pud.insns(2);
    let pull = pud.take_pin().expect("pull-up");
    assert_eq!((pull.pin, pull.level), (0, Level::High));
    pud.poke(MCUCR, 0x10);
    pud.load_words(&[0x0000, 0x0000]);
    pud.insns(2);
    assert_eq!(pud.peek(MCUCR) & 0x10, 0x10);
    assert_eq!(pud.take_pin(), None);
    assert!(pud.io_writes().is_empty());
}

/// PB5, PB3, and PB2 are outputs. A poke on the real context reports SS at
/// cycle 0 when `SPCR` selects master. Do not step: a poked `SPDR` in master
/// mode really starts the transfer.
#[test]
fn poke_of_spi_is_quiet() {
    let mut cpu = Cpu::new();
    cpu.poke(common::DDRB, 0x2C);
    cpu.poke(SPCR, 0x50);
    cpu.poke(SPDR, 0xA5);
    assert_eq!(cpu.peek(common::DDRB), 0x2C);
    assert_eq!(cpu.peek(SPCR), 0x50);
    assert_eq!(cpu.take_pin(), None);
    assert!(cpu.io_writes().is_empty());
    assert_eq!(cpu.take(Channel::Spi), None);
    assert_eq!(cpu.take(Channel::Usart0), None);
}

/// The host already holds PB0 low, so turning the pin into an output low
/// does not change the pad.
#[test]
fn an_output_at_the_host_level_reports_its_drive_at_the_same_level() {
    let mut cpu = Cpu::new();
    cpu.drive_pin(0, Level::Low);
    let _ = cpu.take_pin();
    cpu.load_words(&[imm(0xE000, 16, 0x01), out(16, 0x04)]);
    cpu.insns(2);
    let report = cpu.take_pin().expect("the drive changed");
    assert_eq!(
        (report.level, report.drive, report.cycle),
        (Level::Low, PinDrive::Low, 2)
    );
    assert_eq!(cpu.take_pin(), None);
}
