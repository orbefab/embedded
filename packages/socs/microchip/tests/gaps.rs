//! What the firmware reaches that the model does not run: a refusal stops the chip there, and a
//! note names a register it keeps without running, once per run. The checks are on for a host
//! that asks, as a timeline does, and off for a bare core.

use crate::common::{imm, in_io, ldi, out, spin, sts};

use orbe_mcu::{Fault, Mcu, Note, Unsupported};
use orbe_microchip::{
    Bootloader, Bridge, Cpu, FuseBytes, Noted, PinDrive, Refusal, Refused, Timeline, Unmodeled,
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
const TWAMR: u16 = 0x00BD;
const UCSR0C: u16 = 0x00C2;

const SLEEP: u16 = 0x9588;
const BREAK: u16 = 0x9598;
const SPM: u16 = 0x95E8;
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

/// ADC6 and ADC7 are channels like the others. A package without their pins reads 0 V there,
/// which the host's samples give.
#[test]
fn adc6_and_adc7_note_and_refuse_nothing() {
    let mut words = vec![];
    for mux in [6, 7] {
        words.push(ldi(16, 0x40 | mux));
        words.extend(sts(16, ADMUX));
    }
    words.push(spin());
    let cpu = run(&words, 100);
    assert_eq!(cpu.noted(), []);
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

/// A `TWAR` store of `address << 1` and Wire's `TWCR = TWEN | TWIE | TWEA`, in either order.
/// Wire 1.8.6's `begin(address)` stores `TWCR` first, in `twi_init`, then `TWAR`.
fn wire_begin(address: Option<u8>, twar_first: bool) -> Vec<u16> {
    let mut twar = vec![];
    if let Some(address) = address {
        twar.push(ldi(16, u16::from(address) << 1));
        twar.extend(sts(16, TWAR));
    }
    let mut twcr = vec![ldi(16, 0x45)];
    twcr.extend(sts(16, TWCR));
    let mut words = if twar_first {
        [twar, twcr].concat()
    } else {
        [twcr, twar].concat()
    };
    words.push(spin());
    words
}

#[test]
fn twi_slave_is_refused_and_a_master_is_quiet() {
    for twar_first in [false, true] {
        let cpu = run(&wire_begin(Some(0x42), twar_first), 100);
        assert_eq!(
            cpu.refusal().map(|refusal| refusal.what),
            Some(setting("TWI", "slave")),
            "twar_first {twar_first}"
        );
        let master = run(&wire_begin(None, twar_first), 100);
        assert_eq!(master.refusal(), None);
        assert!(master.noted().is_empty());
    }
    // Setting a nonzero mask in TWAMR (bits 7:1) with TWEN | TWEA also enables slave address
    // matching (§22.9.6) and is refused in either write order; bit 0 is reserved and ignored.
    for twamr_first in [false, true] {
        let mut twamr = vec![ldi(16, 0x04)];
        twamr.extend(sts(16, TWAMR));
        let mut twcr = vec![ldi(16, 0x45)];
        twcr.extend(sts(16, TWCR));
        let mut words = if twamr_first {
            [twamr, twcr].concat()
        } else {
            [twcr, twamr].concat()
        };
        words.push(spin());
        assert_eq!(
            refused(&words),
            Some(setting("TWI", "slave")),
            "twamr_first {twamr_first}"
        );
    }
    // TWAMR bit 0 alone (reserved) or TWAMR = 0 with TWEN | TWEA stays quiet.
    let mut quiet = vec![ldi(16, 0x01)];
    quiet.extend(sts(16, TWAMR));
    quiet.push(ldi(16, 0x45));
    quiet.extend(sts(16, TWCR));
    quiet.push(spin());
    assert_eq!(refused(&quiet), None);
    // Writing TWAR or TWAMR without TWEA | TWEN stays quiet.
    assert_eq!(refused(&store(TWAR, 0x84)), None);
    assert_eq!(refused(&store(TWAMR, 0x04)), None);
}

#[test]
fn break_is_a_nop_without_dwen_and_refused_with_dwen() {
    // Default fuses leave DWEN unprogrammed (high = 0xD9): BREAK acts as a 1-cycle NOP.
    let words = [BREAK, ldi(16, 0x5A), spin()];
    let cpu = run(&words, 20);
    assert_eq!(cpu.refusal(), None);
    assert!(!cpu.is_stopped());
    assert_eq!(cpu.reg(16), 0x5A);

    // With DWEN programmed (high bit 6 = 0), BREAK stops the core and refuses.
    let mut dwen = Cpu::new();
    dwen.set_fuses(0xFF, 0x99, 0xFF);
    dwen.watch_gaps();
    dwen.load_words(&words);
    dwen.run_until(20);
    assert!(dwen.is_stopped());
    assert_eq!(
        dwen.refusal().map(|refusal| refusal.what),
        Some(Refused::Instruction("break"))
    );
}

#[test]
fn spm_is_a_nop_in_application_flash_and_refused_in_the_boot_section() {
    // Application section (pc < boot_word): §27.2 disables SPM in hardware, so it is a 1-cycle
    // no-op and does not refuse.
    let app = run(&[SPM, ldi(16, 0x33), spin()], 20);
    assert_eq!(app.refusal(), None);
    assert_eq!(app.reg(16), 0x33);

    // Boot Loader Flash section (pc >= boot_word): refused with Refused::Instruction("spm").
    // Default high fuse 0xD9 selects BOOTSZ 00, boot_word 0x3800.
    let mut words = vec![0x0000u16; 0x3802];
    words[0] = 0x940C; // JMP 0x3800
    words[1] = 0x3800;
    words[0x3800] = SPM;
    words[0x3801] = spin();
    let boot = run(&words, 40);
    assert_eq!(
        boot.refusal(),
        Some(Refusal {
            what: Refused::Instruction("spm"),
            at: 3,
            pc: 0x3800,
        })
    );
}

#[test]
fn calling_optiboot_boot_section_refuses_as_unsimulated() {
    // JMP 0x3FFE (Optiboot's do_spm entry vector) with Bootloader::Optiboot and erased boot flash.
    let mut flash = vec![0xFFu8; 4];
    flash[0..2].copy_from_slice(&0x940Cu16.to_le_bytes());
    flash[2..4].copy_from_slice(&0x3FFEu16.to_le_bytes());
    let fuses = FuseBytes {
        low: 0xFF,
        high: 0xDE,
        extended: 0xFD,
    };
    let mut bridge = Bridge::new(16_000_000, fuses, Bootloader::Optiboot, &flash);
    bridge.power_on();
    let limit = Time::from_ps(100_000_000_000);
    while bridge.run_until(limit).is_some() {}
    let (_, fault) = bridge.fault().expect("do_spm must refuse");
    assert!(matches!(
        fault,
        Fault::UnsupportedSetting {
            peripheral: "Optiboot",
            setting: orbe_mcu::Setting::Unsimulated(_),
            pc,
        } if pc.pc == 0x7FFC
    ));
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
    let mut bridge = Bridge::new(16_000_000, fuses, Bootloader::None, &flash);
    bridge.power_on();
    let limit = Time::from_ps(100_000_000_000);
    let mut stops = vec![];
    while let Some(stop) = bridge.run_until(limit) {
        stops.push(stop.at);
    }
    // The power-on start from reset, Table 9-4 with `CKSEL0` 1 and `SUT` 11: 16K CK, 14 CK and
    // 65 ms. Then OSCCAL's store is at cycle 1; the unknown word at cycle 3, word 3, byte 6.
    let start = 16_384 + 14 + 1_040_000;
    let at = Time::from_ps((start + 3) * 62_500);
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
        (
            Time::from_ps((start + 1) * 62_500),
            0x66,
            "OSCCAL",
            2,
            true,
            true
        )
    );
    assert!(
        bridge
            .notes(Time::from_ps((start + 1) * 62_500 - 1))
            .is_empty()
    );
}
