//! What the firmware reaches that the model does not run: a refusal stops the chip there, and a
//! note names a register it keeps without running, once per run. The checks are on for a host
//! that asks, as a timeline does, and off for a bare core.

use crate::common::{imm, in_io, ldi, out, spin, sts};

use orbe_mcu::{Fault, Mcu, Note, Unsupported};
use orbe_microchip::{
    Bridge, Cpu, FuseBytes, Noted, PinDrive, Refusal, Refused, Timeline, Unmodeled,
};
use orbe_model::Time;

const ACSR_IO: u16 = 0x30;
const SMCR_IO: u16 = 0x33;
const MCUCR_IO: u16 = 0x35;
const SPMCSR_IO: u16 = 0x37;
const OSCCAL: u16 = 0x0066;
const ADMUX: u16 = 0x007C;
const ADCSRB: u16 = 0x007B;
const ASSR: u16 = 0x00B6;
const TWBR: u16 = 0x00B8;
const TWSR: u16 = 0x00B9;
const TWAR: u16 = 0x00BA;
const TWCR: u16 = 0x00BC;
const UCSR0C: u16 = 0x00C2;

const SLEEP: u16 = 0x9588;
/// A word that decodes to no instruction.
const UNKNOWN: u16 = 0xFFFF;

fn ori(d: u16, k: u16) -> u16 {
    imm(0x6000, d, k)
}

fn run(words: &[u16], cycles: u64) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.watch_gaps();
    cpu.load_words(words);
    cpu.run_until(cycles);
    cpu
}

fn setting(peripheral: &'static str, setting: &'static str) -> Refused {
    Refused::Setting {
        peripheral,
        setting,
    }
}

/// `ldi r16, value` then a store of r16 to data address `addr`, then a spin.
fn store(addr: u16, value: u8) -> Vec<u16> {
    let mut words = vec![ldi(16, u16::from(value))];
    words.extend(sts(16, addr));
    words.push(spin());
    words
}

fn refused(words: &[u16]) -> Option<Refused> {
    run(words, 5_000).refusal().map(|refusal| refusal.what)
}

#[test]
fn a_bare_core_runs_on_past_what_it_would_refuse() {
    let mut cpu = Cpu::new();
    cpu.load_words(&[UNKNOWN, ldi(16, 0x08), out(16, ACSR_IO), ldi(17, 1), spin()]);
    cpu.run_until(100);
    assert_eq!((cpu.refusal(), cpu.reg(17)), (None, 1));
}

#[test]
fn a_word_that_decodes_to_nothing_stops_the_chip_there() {
    let cpu = run(&[0x0000, UNKNOWN, spin()], 100);
    assert_eq!(
        cpu.refusal(),
        Some(Refusal {
            what: Refused::Instruction("unknown"),
            at: 1,
            pc: 1,
        })
    );
}

#[test]
fn a_read_of_acsr_with_the_comparator_on_stops_at_the_read() {
    // §23.1: ACO is AIN0 above AIN1, voltages the model does not have.
    let cpu = run(&[0x0000, in_io(16, ACSR_IO), spin()], 5_000);
    assert_eq!(
        cpu.refusal(),
        Some(Refusal {
            what: setting("AC", "aco_read"),
            at: 1,
            pc: 1,
        })
    );
}

#[test]
fn a_polled_acsr_stops_at_its_first_read_without_waiting_for_the_bound() {
    // `in`, `sbrs`, `rjmp` back: the second load shows the first was a use.
    let cpu = run(&[in_io(16, ACSR_IO), 0xFF05, 0xCFFD, spin()], 20);
    assert_eq!(
        cpu.refusal().map(|refusal| (refusal.what, refusal.pc)),
        Some((setting("AC", "aco_read"), 0))
    );
}

#[test]
fn turning_the_comparator_off_with_a_read_modify_write_is_no_use() {
    // `ACSR |= _BV(ACD)`, as power-saving code does, then reads with it off.
    let words = [
        in_io(16, ACSR_IO),
        ori(16, 0x80),
        out(16, ACSR_IO),
        in_io(17, ACSR_IO),
        spin(),
    ];
    assert_eq!(refused(&words), None);
}

#[test]
fn a_read_modify_write_that_leaves_the_comparator_on_is_a_use() {
    let words = [in_io(16, ACSR_IO), ori(16, 0x10), out(16, ACSR_IO), spin()];
    assert_eq!(refused(&words), Some(setting("AC", "aco_read")));
}

#[test]
fn the_comparator_interrupt_capture_and_bandgap_each_stop_the_chip() {
    for (bits, name) in [(0x08, "ACIE"), (0x04, "ACIC"), (0x40, "ACBG")] {
        let words = [ldi(16, bits), out(16, ACSR_IO), spin()];
        assert_eq!(refused(&words), Some(setting("AC", name)), "{name}");
    }
    // With ACD set the comparator is off, and its bits do nothing (§23.3.2).
    let words = [ldi(16, 0x80 | 0x08), out(16, ACSR_IO), spin()];
    assert_eq!(refused(&words), None);
}

#[test]
fn an_adc_trigger_on_the_comparator_stops_the_chip_only_while_it_is_on() {
    assert_eq!(refused(&store(ADCSRB, 0x01)), Some(setting("ADC", "ADTS")));
    let mut words = vec![ldi(16, 0x80), out(16, ACSR_IO)];
    words.extend(store(ADCSRB, 0x01));
    assert_eq!(refused(&words), None);
}

#[test]
fn reserved_adc_references_and_channels_stop_the_chip() {
    assert_eq!(refused(&store(ADMUX, 0x80)), Some(setting("ADC", "REFS")));
    for mux in 0x09..=0x0D {
        assert_eq!(
            refused(&store(ADMUX, 0x40 | mux)),
            Some(setting("ADC", "MUX")),
            "MUX {mux}"
        );
    }
    for mux in [0x00, 0x05, 0x08, 0x0E, 0x0F] {
        assert_eq!(refused(&store(ADMUX, 0x40 | mux)), None, "MUX {mux}");
    }
}

#[test]
fn adc6_and_adc7_are_noted_once_each() {
    let mut words = vec![];
    for mux in [6, 6, 7, 6] {
        words.push(ldi(16, 0x40 | mux));
        words.extend(sts(16, ADMUX));
    }
    words.push(spin());
    let cpu = run(&words, 100);
    let noted: Vec<Unmodeled> = cpu.noted().iter().map(|noted| noted.what).collect();
    assert_eq!(
        noted,
        [Unmodeled::AdcNoPad(6), Unmodeled::AdcNoPad(7)],
        "once per run each"
    );
    assert_eq!(cpu.refusal(), None);
}

#[test]
fn moving_the_vectors_to_the_boot_section_stops_the_chip() {
    let words = [
        ldi(16, 0x01),
        out(16, MCUCR_IO),
        ldi(16, 0x02),
        out(16, MCUCR_IO),
        spin(),
    ];
    assert_eq!(
        run(&words, 100).refusal(),
        Some(Refusal {
            what: setting("CPU", "IVSEL"),
            at: 3,
            pc: 3,
        })
    );
    // PUD and the BOD sleep bits are modelled.
    let words = [ldi(16, 0x10), out(16, MCUCR_IO), spin()];
    assert_eq!(refused(&words), None);
}

#[test]
fn self_programming_stops_the_chip_at_spmcsr() {
    // §27.9.1. Every SPM that acts needs SPMEN set here first.
    let words = [ldi(16, 0x01), out(16, SPMCSR_IO), spin()];
    assert_eq!(refused(&words), Some(setting("BOOT_LOAD", "SPMCSR")));
    let words = [ldi(16, 0x21), out(16, SPMCSR_IO), spin()];
    assert_eq!(refused(&words), Some(setting("BOOT_LOAD", "SPMCSR")));
    let words = [ldi(16, 0x80), out(16, SPMCSR_IO), spin()];
    assert_eq!(
        refused(&words),
        Some(Refused::Interrupt {
            peripheral: "BOOT_LOAD"
        })
    );
}

#[test]
fn timer2_on_a_tosc_crystal_or_clock_stops_the_chip() {
    assert_eq!(refused(&store(ASSR, 0x20)), Some(setting("TC2", "AS2")));
    assert_eq!(refused(&store(ASSR, 0x40)), Some(setting("TC2", "EXCLK")));
    assert_eq!(refused(&store(ASSR, 0x00)), None);
}

#[test]
fn synchronous_and_reserved_usart_modes_stop_the_chip() {
    assert_eq!(
        refused(&store(UCSR0C, 0x46)),
        Some(setting("USART0", "synchronous"))
    );
    assert_eq!(
        refused(&store(UCSR0C, 0x86)),
        Some(setting("USART0", "UMSEL"))
    );
    assert_eq!(
        refused(&store(UCSR0C, 0x16)),
        Some(setting("USART0", "UPM"))
    );
    // 8N1, 8E1, and master SPI (UMSEL 11) are modelled.
    for value in [0x06, 0x26, 0xC0] {
        assert_eq!(refused(&store(UCSR0C, value)), None, "{value:#04x}");
    }
}

#[test]
fn a_reserved_sleep_mode_stops_the_chip_at_the_sleep() {
    let words = [ldi(16, 0b1001), out(16, SMCR_IO), SLEEP, spin()];
    assert_eq!(
        run(&words, 100).refusal(),
        Some(Refusal {
            what: Refused::Sleep,
            at: 2,
            pc: 2,
        })
    );
}

#[test]
fn osccal_is_noted_and_the_run_goes_on() {
    let mut words = store(OSCCAL, 0x80);
    words.insert(words.len() - 1, ldi(17, 1));
    let cpu = run(&words, 100);
    assert_eq!(
        cpu.noted(),
        [Noted {
            what: Unmodeled::Osccal,
            at: 1,
            pc: 1,
            write: true,
        }]
    );
    assert_eq!(cpu.reg(17), 1);
}

/// `Wire.begin(address)`: `TWAR = address << 1`, then `TWCR = TWEN | TWIE | TWEA`.
fn wire_begin(address: Option<u8>) -> Vec<u16> {
    let mut words = vec![];
    if let Some(address) = address {
        words.push(ldi(16, u16::from(address) << 1));
        words.extend(sts(16, TWAR));
    }
    words.push(ldi(16, 0x45));
    words.extend(sts(16, TWCR));
    words.push(spin());
    words
}

#[test]
fn a_twi_slave_address_is_noted_and_a_master_is_not() {
    let cpu = run(&wire_begin(Some(0x42)), 100);
    let noted: Vec<Unmodeled> = cpu.noted().iter().map(|noted| noted.what).collect();
    assert_eq!(noted, [Unmodeled::TwiSlave]);
    assert_eq!(cpu.refusal(), None);
    assert!(run(&wire_begin(None), 100).noted().is_empty());
}

#[test]
fn a_start_below_about_18_khz_is_noted_for_its_hold_time() {
    // TWBR 255 and TWPS 3 is 16 MHz / (16 + 2 * 255 * 64) = 489 Hz.
    let start = |twbr: u8, twps: u8| {
        let mut words = vec![ldi(16, u16::from(twbr))];
        words.extend(sts(16, TWBR));
        words.push(ldi(16, u16::from(twps)));
        words.extend(sts(16, TWSR));
        words.push(ldi(16, 0xA4));
        words.extend(sts(16, TWCR));
        words.push(spin());
        run(&words, 100)
            .noted()
            .iter()
            .map(|noted| noted.what)
            .collect::<Vec<_>>()
    };
    assert_eq!(start(255, 3), [Unmodeled::TwiHoldTime]);
    // 100 kHz, as Wire runs.
    assert_eq!(start(72, 0), []);
}

#[test]
fn a_refusal_halts_the_timeline_and_nothing_after_it_shows() {
    // PB5 rises, then the chip reaches a word it refuses; the clear after it never shows.
    let words = [
        ldi(16, 0x20),
        out(16, 0x04),
        out(16, 0x05),
        UNKNOWN,
        ldi(16, 0x00),
        out(16, 0x05),
        spin(),
    ];
    let mut cpu = Cpu::new();
    cpu.load_words(&words);
    let mut timeline = Timeline::new(cpu);
    let mut halts = vec![];
    while let Some(halt) = timeline.run_until(1_000) {
        halts.push((halt.at, halt.refused));
    }
    assert_eq!(halts.last(), Some(&(4, true)));
    assert_eq!(halts.iter().filter(|&&(_, refused)| refused).count(), 1);
    assert_eq!(timeline.drives()[5], PinDrive::High, "PB5 stays high");
    assert_eq!(timeline.run_until(10_000), None);
}

#[test]
fn the_bridge_reports_the_refusal_as_its_fault_and_notes_as_unmodeled_registers() {
    let words: Vec<u16> = [store(OSCCAL, 0x80), vec![UNKNOWN]]
        .concat()
        .into_iter()
        .filter(|&word| word != spin())
        .collect();
    let flash: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let fuses = FuseBytes {
        low: 0xFF,
        high: 0xDE,
        extended: 0xFD,
    };
    let mut bridge = Bridge::new(16_000_000, fuses, &flash);
    let limit = Time::from_ps(1_000_000);
    let mut stops = vec![];
    while let Some(stop) = bridge.run_until(limit) {
        stops.push(stop.at);
    }
    // OSCCAL's store is at cycle 1; the unknown word at cycle 3, word 3, byte 6.
    let at = Time::from_ps(3 * 62_500);
    assert_eq!(stops.last(), Some(&at));
    let (fault_at, fault) = bridge.fault().unwrap();
    assert_eq!(fault_at, at);
    assert!(matches!(
        fault,
        Fault::UnsupportedInstruction {
            op: "unknown",
            reason: Unsupported::Op,
            pc,
        } if pc.pc == 6
    ));
    let notes = bridge.notes(limit);
    assert_eq!(notes.len(), 1);
    let (
        note_at,
        Note::UnmodeledRegister {
            address,
            register,
            pc,
            write,
            kept,
        },
    ) = &notes[0]
    else {
        panic!("{notes:?}");
    };
    assert_eq!(
        (*note_at, *address, register.as_str(), pc.pc, *write, *kept),
        (Time::from_ps(62_500), 0x66, "OSCCAL", 2, true, true)
    );
    assert!(bridge.notes(Time::from_ps(62_499)).is_empty());
}
