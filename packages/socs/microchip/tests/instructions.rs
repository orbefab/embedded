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
    // PORTB takes the store. An unowned address is plain memory. A reserved
    // waveform mode does not start the counter. The log cap is
    // `firmware.rs`'s ten-second blink.
    let mut cpu = Cpu::new();
    cpu.load_words(&[
        imm(0xE000, 16, 0xA5),
        out_reg(16, 0x05),
        0x9A00,
        imm(0xE000, 16, 0x09),
        out_reg(16, 0x25),
    ]);
    cpu.insns(5);
    assert_eq!(cpu.peek(0x0025), 0xA5, "PORTB took the store");
    assert_eq!(cpu.peek(0x0020), 0x01, "SBI wrote the unowned byte");
    assert_eq!(cpu.peek(0x0046), 0, "reserved WGM does not count");
    let logged: Vec<_> = cpu.io_writes().iter().map(|write| write.addr).collect();
    assert!(
        logged.contains(&0x0020),
        "unowned address is logged: {logged:?}"
    );
    assert!(
        logged.contains(&0x0045),
        "reserved TCCR0B field is logged: {logged:?}"
    );
    assert!(
        !logged.contains(&0x0025),
        "PORTB is modeled and stays out of the log: {logged:?}"
    );

    let mut flood = Cpu::new();
    // LDI, then STS r16, 0x0020 and RJMP back to that STS.
    flood.load_words(&[imm(0xE000, 16, 0x01), 0x9300, 0x0020, 0xCFFD]);
    let extra = 10u64;
    let writes = orbe_microchip::IO_LOG_CAP as u64 + extra;
    flood.insns(u32::try_from(1 + writes * 2).unwrap());
    assert_eq!(flood.io_writes().len(), orbe_microchip::IO_LOG_CAP);
    assert_eq!(flood.io_write_drops(), extra);
}
