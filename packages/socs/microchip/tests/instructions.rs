//! Instructions whose result is the chip's: `SBI` on a flag register that a one clears,
//! `SLEEP` while `SMCR` leaves sleep off, and stores to addresses the chip does not model.
//! orbe-avr's `tests/instructions.rs` tests each instruction on a flat data space.

use crate::common;
use common::Instructions;
use common::{cbi as cbi_word, imm, out as out_reg, sbi as sbi_word};

use orbe_microchip::Cpu;

fn exec(words: &[u16], prep: impl FnOnce(&mut Cpu)) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.load_words(words);
    prep(&mut cpu);
    cpu.step();
    cpu
}

fn expect(cpu: &Cpu, pc: u16, sreg: u8, regs: &[(u8, u8)]) {
    assert_eq!(cpu.pc(), pc, "pc");
    assert_eq!(cpu.sreg(), sreg, "sreg");
    for &(index, value) in regs {
        assert_eq!(cpu.reg(index), value, "r{index}");
    }
}

#[test]
fn sbi_and_cbi_change_only_the_named_bit() {
    // DS40002061B §7.4: "the CBI and SBI instructions will only operate on the
    // specified bit, and can therefore be used on registers containing such
    // Status Flags." A one clears that flag. A zero leaves the others set.
    let mut cpu = Cpu::new();
    cpu.poke(0x35, 0x07);
    cpu.load_words(&[sbi_word(0x15, 1)]);
    cpu.step();
    assert_eq!(cpu.peek(0x35) & 0x07, 0x05, "SBI TIFR0, OCF0A");

    let mut cpu = Cpu::new();
    cpu.poke(0x35, 0x07);
    cpu.load_words(&[cbi_word(0x15, 2)]);
    cpu.step();
    assert_eq!(cpu.peek(0x35) & 0x07, 0x07, "CBI TIFR0, OCF0B");

    // TIFR1: TOV1, OCF1A, OCF1B, and ICF1.
    let mut cpu = Cpu::new();
    cpu.poke(0x36, 0x27);
    cpu.load_words(&[sbi_word(0x16, 1)]);
    cpu.step();
    assert_eq!(cpu.peek(0x36) & 0x27, 0x25, "SBI TIFR1, OCF1A");

    let mut cpu = Cpu::new();
    cpu.poke(0x36, 0x27);
    cpu.load_words(&[cbi_word(0x16, 5)]);
    cpu.step();
    assert_eq!(cpu.peek(0x36) & 0x27, 0x27, "CBI TIFR1, ICF1");

    let mut cpu = Cpu::new();
    cpu.poke(0x37, 0x07);
    cpu.load_words(&[sbi_word(0x17, 0)]);
    cpu.step();
    assert_eq!(cpu.peek(0x37) & 0x07, 0x06, "SBI TIFR2, TOV2");

    // PINB is write-one-to-toggle. SBI flips one PORTB bit. CBI writes a zero.
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 0xFF);
    cpu.poke(0x25, 0xFF);
    cpu.load_words(&[sbi_word(0x03, 0)]);
    cpu.step();
    assert_eq!(cpu.peek(0x25), 0xFE, "SBI PINB, 0");

    let mut cpu = Cpu::new();
    cpu.poke(0x24, 0xFF);
    cpu.poke(0x25, 0xFF);
    cpu.load_words(&[cbi_word(0x03, 1)]);
    cpu.step();
    assert_eq!(cpu.peek(0x25), 0xFF, "CBI PINB, 1");
}

#[test]
fn sleep() {
    // §10.11.1. Without SE, SLEEP does not sleep.
    let mut cpu = exec(&[0x9588, 0x0000], |cpu| cpu.poke(0x0053, 0x01));
    expect(&cpu, 1, 0, &[]);
    assert!(cpu.is_sleeping());
    cpu.step();
    expect(&cpu, 1, 0, &[]);
}

#[test]
fn unowned_and_reserved_stores_change_only_what_they_model() {
    // PORTB and GPIOR0..=GPIOR2 take stores; reserved I/O addresses (0x0020, 0x0058) and
    // DWDR (0x0051) ignore writes and read as zero (§25.6.1, §36). Reserved bits in TCCR0A
    // (bits 3:2, §15.9.1) read as zero, and a reserved waveform mode does not start the counter.
    let mut cpu = Cpu::new();
    cpu.load_words(&[
        imm(0xE000, 16, 0xA5),
        out_reg(16, 0x05), // PORTB = 0xA5
        0x9A00,            // SBI 0x00 (reserved 0x0020), 0
        0x9300,
        0x0058, // STS 0x0058 (reserved), r16
        0x9300,
        0x0051, // STS 0x0051 (DWDR), r16
        0x9300,
        0x003E, // STS 0x003E (GPIOR0), r16
        0x9300,
        0x004A, // STS 0x004A (GPIOR1), r16
        0x9300,
        0x004B, // STS 0x004B (GPIOR2), r16
        imm(0xE000, 16, 0x0C),
        out_reg(16, 0x24), // TCCR0A = 0x0C (reserved bits 3:2)
        imm(0xE000, 16, 0x09),
        out_reg(16, 0x25), // TCCR0B = 0x09 (reserved WGM 4 + prescaler 1)
    ]);
    cpu.insns(12);
    assert_eq!(cpu.peek(0x0025), 0xA5, "PORTB took the store");
    assert_eq!(cpu.peek(0x0020), 0, "reserved I/O 0x0020 reads zero");
    assert_eq!(cpu.peek(0x0058), 0, "reserved I/O 0x0058 reads zero");
    assert_eq!(cpu.peek(0x0051), 0, "DWDR reads zero");
    assert_eq!(cpu.peek(0x003E), 0xA5, "GPIOR0 stores and reads back");
    assert_eq!(cpu.peek(0x004A), 0xA5, "GPIOR1 stores and reads back");
    assert_eq!(cpu.peek(0x004B), 0xA5, "GPIOR2 stores and reads back");
    assert_eq!(cpu.peek(0x0044), 0, "TCCR0A reserved bits 3:2 read zero");
    assert_eq!(cpu.peek(0x0046), 0, "reserved WGM does not count");
}
