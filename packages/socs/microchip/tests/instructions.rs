//! One test per mnemonic in the ATmega328P instruction-set summary.
//! Opcodes are the bit patterns in DS40002198C. Flag results follow that manual.

use crate::common;
use common::Instructions;
use common::{
    SPH, SPL, SREG, adiw as enc_adiw, brbc as enc_brbc, brbs as enc_brbs, cbi as cbi_word, imm,
    in_io, ldd, out as out_reg, rr, sbi as sbi_word, sbiw as enc_sbiw,
};

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
    assert_eq!(
        cpu.sreg(),
        sreg,
        "sreg got {:08b} want {:08b}",
        cpu.sreg(),
        sreg
    );
    for &(index, value) in regs {
        assert_eq!(cpu.reg(index), value, "r{index}");
    }
}

fn f(h: u8, s: u8, v: u8, n: u8, z: u8, c: u8) -> u8 {
    (h << 5) | (s << 4) | (v << 3) | (n << 2) | (z << 1) | c
}

fn set_x(cpu: &mut Cpu, addr: u16) {
    cpu.set_reg(26, addr as u8);
    cpu.set_reg(27, (addr >> 8) as u8);
}

fn set_y(cpu: &mut Cpu, addr: u16) {
    cpu.set_reg(28, addr as u8);
    cpu.set_reg(29, (addr >> 8) as u8);
}

fn set_z(cpu: &mut Cpu, addr: u16) {
    cpu.set_reg(30, addr as u8);
    cpu.set_reg(31, (addr >> 8) as u8);
}

#[test]
fn add() {
    let cpu = exec(&[rr(0x0C00, 1, 2)], |cpu| {
        cpu.set_reg(1, 0x0F);
        cpu.set_reg(2, 0x01);
    });
    expect(&cpu, 1, f(1, 0, 0, 0, 0, 0), &[(1, 0x10)]);
    // LSL is ADD Rd, Rd.
    let cpu = exec(&[rr(0x0C00, 16, 16)], |cpu| cpu.set_reg(16, 0x80));
    expect(&cpu, 1, f(0, 1, 1, 0, 1, 1), &[(16, 0x00)]);
}

#[test]
fn adc() {
    let cpu = exec(&[rr(0x1C00, 1, 2)], |cpu| {
        cpu.set_reg(1, 0x0F);
        cpu.set_sreg(0x01);
    });
    expect(&cpu, 1, f(1, 0, 0, 0, 0, 0), &[(1, 0x10)]);
    // ROL is ADC Rd, Rd with C set.
    let cpu = exec(&[rr(0x1C00, 16, 16)], |cpu| {
        cpu.set_reg(16, 0x00);
        cpu.set_sreg(0x01);
    });
    expect(&cpu, 1, 0, &[(16, 0x01)]);
}

#[test]
fn adiw() {
    let cpu = exec(&[enc_adiw(1, 24)], |cpu| {
        cpu.set_reg(24, 0xFF);
        cpu.set_reg(25, 0x7F);
    });
    expect(&cpu, 1, f(0, 0, 1, 1, 0, 0), &[(24, 0x00), (25, 0x80)]);
}

#[test]
fn sub() {
    let cpu = exec(&[rr(0x1800, 1, 2)], |cpu| {
        cpu.set_reg(1, 0x10);
        cpu.set_reg(2, 0x01);
    });
    expect(&cpu, 1, f(1, 0, 0, 0, 0, 0), &[(1, 0x0F)]);
}

#[test]
fn subi() {
    let cpu = exec(&[imm(0x5000, 16, 0x01)], |cpu| {
        cpu.set_reg(16, 0x01);
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 1, 0), &[(16, 0x00)]);
}

#[test]
fn sbc() {
    let cpu = exec(&[rr(0x0800, 1, 2)], |cpu| {
        cpu.set_reg(1, 0x01);
        cpu.set_sreg(f(0, 0, 0, 0, 1, 1));
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 1, 0), &[(1, 0x00)]);
}

#[test]
fn sbci() {
    let cpu = exec(&[imm(0x4000, 16, 0x00)], |cpu| {
        cpu.set_reg(16, 0x00);
        cpu.set_sreg(f(0, 0, 0, 0, 0, 1));
    });
    expect(&cpu, 1, f(1, 1, 0, 1, 0, 1), &[(16, 0xFF)]);
}

#[test]
fn sbiw() {
    let cpu = exec(&[enc_sbiw(1, 24)], |cpu| {
        cpu.set_reg(24, 0x00);
        cpu.set_reg(25, 0x80);
    });
    expect(&cpu, 1, f(0, 1, 1, 0, 0, 0), &[(24, 0xFF), (25, 0x7F)]);
}

#[test]
fn and() {
    let cpu = exec(&[rr(0x2000, 16, 17)], |cpu| {
        cpu.set_reg(16, 0xF0);
        cpu.set_reg(17, 0x0F);
        cpu.set_sreg(0x01);
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 1, 1), &[(16, 0x00)]);
    // TST is AND Rd, Rd.
    let cpu = exec(&[rr(0x2000, 16, 16)], |cpu| cpu.set_reg(16, 0x80));
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 0), &[(16, 0x80)]);
}

#[test]
fn andi() {
    let cpu = exec(&[imm(0x7000, 16, 0x0F)], |cpu| {
        cpu.set_reg(16, 0xF0);
        cpu.set_sreg(0x01);
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 1, 1), &[(16, 0x00)]);
    // CBR is ANDI Rd, ~mask. The immediate encoder keeps the low eight bits of !0x0F.
    let cpu = exec(&[imm(0x7000, 16, 0xF0)], |cpu| cpu.set_reg(16, 0xFF));
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 0), &[(16, 0xF0)]);
}

#[test]
fn or() {
    let cpu = exec(&[rr(0x2800, 16, 17)], |cpu| {
        cpu.set_reg(16, 0xF0);
        cpu.set_reg(17, 0xF0);
    });
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 0), &[(16, 0xF0)]);
}

#[test]
fn ori() {
    let cpu = exec(&[imm(0x6000, 16, 0xF0)], |cpu| {
        cpu.set_reg(16, 0xF0);
    });
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 0), &[(16, 0xF0)]);
    // SBR is ORI.
    let cpu = exec(&[imm(0x6000, 16, 0x0F)], |cpu| cpu.set_reg(16, 0xF0));
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 0), &[(16, 0xFF)]);
}

#[test]
fn eor() {
    let cpu = exec(&[rr(0x2400, 1, 2)], |cpu| {
        cpu.set_reg(1, 0xFF);
        cpu.set_reg(2, 0x0F);
    });
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 0), &[(1, 0xF0)]);
    // CLR is EOR Rd, Rd. H and C in the incoming SREG stay set.
    let cpu = exec(&[rr(0x2400, 16, 16)], |cpu| {
        cpu.set_reg(16, 0xFF);
        cpu.set_sreg(0x21);
    });
    expect(&cpu, 1, 0x23, &[(16, 0x00)]);
}

#[test]
fn ldi() {
    // SER is LDI Rd, 0xFF. The status register is left alone.
    let cpu = exec(&[imm(0xE000, 16, 0xFF)], |cpu| cpu.set_sreg(0xC0));
    expect(&cpu, 1, 0xC0, &[(16, 0xFF)]);
    let cpu = exec(&[imm(0xE000, 16, 0x42)], |_| {});
    expect(&cpu, 1, 0, &[(16, 0x42)]);
}

#[test]
fn com() {
    let word = 0x9400 | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0x0F));
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 1), &[(1, 0xF0)]);
}

#[test]
fn neg() {
    let word = 0x9401 | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0x80));
    expect(&cpu, 1, f(0, 0, 1, 1, 0, 1), &[(1, 0x80)]);
}

#[test]
fn inc() {
    let word = 0x9403 | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0x7F));
    expect(&cpu, 1, f(0, 0, 1, 1, 0, 0), &[(1, 0x80)]);
}

#[test]
fn dec() {
    let word = 0x940A | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0x80));
    expect(&cpu, 1, f(0, 1, 1, 0, 0, 0), &[(1, 0x7F)]);
}

#[test]
fn mul() {
    let cpu = exec(&[rr(0x9C00, 2, 3)], |cpu| {
        cpu.set_reg(2, 0x10);
        cpu.set_reg(3, 0x10);
    });
    expect(&cpu, 1, 0, &[(0, 0x00), (1, 0x01), (2, 0x10)]);
}

#[test]
fn muls() {
    let cpu = exec(&[0x0201], |cpu| {
        cpu.set_reg(16, 0xFF);
        cpu.set_reg(17, 0xFF);
    });
    expect(&cpu, 1, 0, &[(0, 0x01), (1, 0x00)]);
}

#[test]
fn mulsu() {
    let cpu = exec(&[0x0301], |cpu| {
        cpu.set_reg(16, 0xFF);
        cpu.set_reg(17, 0x02);
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 0, 1), &[(0, 0xFE), (1, 0xFF)]);
}

#[test]
fn fmul() {
    let cpu = exec(&[0x0308], |cpu| {
        cpu.set_reg(16, 0xC0);
        cpu.set_reg(17, 0xC0);
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 0, 1), &[(0, 0x00), (1, 0x20)]);
}

#[test]
fn fmuls() {
    let cpu = exec(&[0x0381], |cpu| {
        cpu.set_reg(16, 0xFF);
        cpu.set_reg(17, 0xFF);
    });
    expect(&cpu, 1, 0, &[(0, 0x02), (1, 0x00)]);
}

#[test]
fn fmulsu() {
    let cpu = exec(&[0x0389], |cpu| {
        cpu.set_reg(16, 0xFF);
        cpu.set_reg(17, 0x02);
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 0, 1), &[(0, 0xFC), (1, 0xFF)]);
}

#[test]
fn rjmp() {
    let cpu = exec(&[0xC001], |_| {});
    expect(&cpu, 2, 0, &[]);
}

#[test]
fn ijmp() {
    let cpu = exec(&[0x9409], |cpu| set_z(cpu, 4));
    expect(&cpu, 4, 0, &[]);
}

#[test]
fn jmp() {
    let cpu = exec(&[0x940C, 0x0004], |_| {});
    expect(&cpu, 4, 0, &[]);
}

#[test]
fn rcall() {
    let mut cpu = exec(&[0xD001, 0x0000, 0x9508], |_| {});
    expect(&cpu, 2, 0, &[]);
    cpu.step();
    expect(&cpu, 1, 0, &[]);
}

#[test]
fn icall() {
    let mut cpu = exec(&[0x9509, 0x0000, 0x9508], |cpu| set_z(cpu, 2));
    expect(&cpu, 2, 0, &[]);
    cpu.step();
    expect(&cpu, 1, 0, &[]);
}

#[test]
fn call() {
    let mut cpu = exec(&[0x940E, 0x0003, 0x0000, 0x9508], |_| {});
    expect(&cpu, 3, 0, &[]);
    cpu.step();
    expect(&cpu, 2, 0, &[]);
}

#[test]
fn ret() {
    let mut cpu = Cpu::new();
    cpu.load_words(&[0xD000, 0x9508]);
    cpu.step();
    cpu.step();
    expect(&cpu, 1, 0, &[]);
}

#[test]
fn reti() {
    let mut cpu = Cpu::new();
    cpu.load_words(&[0xD000, 0x9518]);
    cpu.step();
    cpu.step();
    expect(&cpu, 1, 0x80, &[]);
}

#[test]
fn cpse() {
    let cpu = exec(&[rr(0x1000, 1, 2), 0x0000, 0x0000], |cpu| {
        cpu.set_reg(1, 5);
        cpu.set_reg(2, 5);
    });
    expect(&cpu, 2, 0, &[]);
}

#[test]
fn cp() {
    let cpu = exec(&[rr(0x1400, 1, 2)], |cpu| {
        cpu.set_reg(1, 0x10);
        cpu.set_reg(2, 0x01);
    });
    expect(&cpu, 1, f(1, 0, 0, 0, 0, 0), &[(1, 0x10)]);
}

#[test]
fn cpc() {
    let cpu = exec(&[rr(0x0400, 1, 2)], |cpu| {
        cpu.set_sreg(f(0, 0, 0, 0, 1, 0));
    });
    expect(&cpu, 1, f(0, 0, 0, 0, 1, 0), &[]);
}

#[test]
fn cpi() {
    let cpu = exec(&[imm(0x3000, 16, 0x42)], |cpu| cpu.set_reg(16, 0x42));
    expect(&cpu, 1, f(0, 0, 0, 0, 1, 0), &[(16, 0x42)]);
}

#[test]
fn sbrc() {
    let word = 0xFC00 | (1 << 4);
    let cpu = exec(&[word, 0x0000, 0x0000], |_| {});
    expect(&cpu, 2, 0, &[]);
}

#[test]
fn sbrs() {
    let word = 0xFE00 | (1 << 4);
    let cpu = exec(&[word, 0x0000, 0x0000], |cpu| cpu.set_reg(1, 0x01));
    expect(&cpu, 2, 0, &[]);
}

#[test]
fn sbic() {
    let cpu = exec(&[0x9900, 0x0000, 0x0000], |_| {});
    expect(&cpu, 2, 0, &[]);
}

#[test]
fn sbis() {
    let cpu = exec(&[0x9B00, 0x0000, 0x0000], |cpu| cpu.poke(0x20, 0x01));
    expect(&cpu, 2, 0, &[]);
}

fn taken(word: u16, sreg: u8) {
    let cpu = exec(&[word], |cpu| cpu.set_sreg(sreg));
    expect(&cpu, 2, sreg, &[]);
}

#[test]
fn branch_aliases_follow_brbs_and_brbc() {
    // Each alias is BRBS or BRBC on one SREG bit. The row is (bit, BRBS, SREG
    // that makes the branch taken). Cycle counts live in cycles.rs.
    let rows = [
        (1u16, true, 0x02u8),
        (1, false, 0x00),
        (0, true, 0x01),
        (0, false, 0x00),
        (2, true, 0x04),
        (2, false, 0x00),
        (4, false, 0x00),
        (4, true, 0x10),
        (5, true, 0x20),
        (5, false, 0x00),
        (6, true, 0x40),
        (6, false, 0x00),
        (3, true, 0x08),
        (3, false, 0x00),
        (7, true, 0x80),
        (7, false, 0x00),
    ];
    for (bit, brbs_form, sreg) in rows {
        let word = if brbs_form {
            enc_brbs(bit, 1)
        } else {
            enc_brbc(bit, 1)
        };
        taken(word, sreg);
    }
}

#[test]
fn sbi() {
    let cpu = exec(&[0x9A00], |_| {});
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x20), 0x01);
}

#[test]
fn cbi() {
    let cpu = exec(&[0x9800], |cpu| cpu.poke(0x20, 0xFF));
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x20), 0xFE);
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
fn lsr() {
    let word = 0x9406 | (16 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(16, 0x01));
    expect(&cpu, 1, f(0, 1, 1, 0, 1, 1), &[(16, 0x00)]);
}

#[test]
fn ror() {
    let word = 0x9407 | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        cpu.set_reg(1, 0x01);
        cpu.set_sreg(0x01);
    });
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 1), &[(1, 0x80)]);
}

#[test]
fn asr() {
    let word = 0x9405 | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0x81));
    expect(&cpu, 1, f(0, 1, 0, 1, 0, 1), &[(1, 0xC0)]);
}

#[test]
fn swap() {
    let word = 0x9402 | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0xAB));
    expect(&cpu, 1, 0, &[(1, 0xBA)]);
}

#[test]
fn flag_bit_aliases_match_bset_and_bclr() {
    // SEC..CLH are BSET and BCLR. `s` is the SREG bit (DS40002198).
    for s in 0..8u16 {
        let set = 0x9408 | (s << 4);
        let cpu = exec(&[set], |_| {});
        expect(&cpu, 1, 1u8 << s, &[]);
        let clear = 0x9488 | (s << 4);
        let cpu = exec(&[clear], |cpu| cpu.set_sreg(1u8 << s));
        expect(&cpu, 1, 0, &[]);
    }
}

#[test]
fn bst() {
    let word = 0xFA00 | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0x01));
    expect(&cpu, 1, 0x40, &[]);
}

#[test]
fn bld() {
    let word = 0xF800 | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_sreg(0x40));
    expect(&cpu, 1, 0x40, &[(1, 0x01)]);
}

#[test]
fn mov() {
    let cpu = exec(&[rr(0x2C00, 1, 2)], |cpu| {
        cpu.set_reg(2, 0x55);
        cpu.set_sreg(0xFF);
    });
    expect(&cpu, 1, 0xFF, &[(1, 0x55)]);
}

#[test]
fn movw() {
    let cpu = exec(&[0x0101], |cpu| {
        cpu.set_reg(2, 0x11);
        cpu.set_reg(3, 0x22);
    });
    expect(&cpu, 1, 0, &[(0, 0x11), (1, 0x22)]);
}

#[test]
fn ld_x() {
    let word = 0x900C | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_x(cpu, 0x0100);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (26, 0x00)]);
}

#[test]
fn ld_x_plus() {
    let word = 0x900D | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_x(cpu, 0x0100);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (26, 0x01)]);
}

#[test]
fn ld_minus_x() {
    let word = 0x900E | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_x(cpu, 0x0101);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (26, 0x00)]);
}

#[test]
fn ld_y() {
    let word = ldd(0x0008, false, 1, 0);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0100);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A)]);
}

#[test]
fn ld_y_plus() {
    let word = 0x9009 | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0100);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (28, 0x01)]);
}

#[test]
fn ld_minus_y() {
    let word = 0x900A | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0101);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (28, 0x00)]);
}

#[test]
fn ldd_y_displacement_uses_the_middle_q_bits() {
    // q = 28 is bit 13 clear, bits 11:10 = 11, bits 2:0 = 100.
    let word = ldd(0x0008, false, 24, 28);
    assert_eq!(word, 0x8D8C);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0100);
        cpu.poke(0x011C, 0x5A);
    });
    expect(&cpu, 1, 0, &[(24, 0x5A), (28, 0x00)]);
}

#[test]
fn ldd_y() {
    let word = ldd(0x0008, false, 1, 3);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0100);
        cpu.poke(0x0103, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (28, 0x00)]);
}

#[test]
fn ld_z() {
    let word = ldd(0x0000, false, 1, 0);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0100);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A)]);
}

#[test]
fn ld_z_plus() {
    let word = 0x9001 | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0100);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (30, 0x01)]);
}

#[test]
fn ld_minus_z() {
    let word = 0x9002 | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0101);
        cpu.poke(0x0100, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A), (30, 0x00)]);
}

#[test]
fn ldd_z() {
    let word = ldd(0x0000, false, 1, 3);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0100);
        cpu.poke(0x0103, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A)]);
}

#[test]
fn lds() {
    let cpu = exec(&[0x9010, 0x0100], |cpu| cpu.poke(0x0100, 0x5A));
    expect(&cpu, 2, 0, &[(1, 0x5A)]);
}

#[test]
fn st_x() {
    let word = 0x920C | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_x(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A)]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn st_x_plus() {
    let word = 0x920D | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_x(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(26, 0x01)]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn st_minus_x() {
    let word = 0x920E | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_x(cpu, 0x0101);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(26, 0x00)]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn st_y() {
    let word = ldd(0x0008, true, 1, 0);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn st_y_plus() {
    let word = 0x9209 | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(28, 0x01)]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn st_minus_y() {
    let word = 0x920A | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0101);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(28, 0x00)]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn std_y() {
    let word = ldd(0x0008, true, 1, 3);
    let cpu = exec(&[word], |cpu| {
        set_y(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(28, 0x00)]);
    assert_eq!(cpu.peek(0x0103), 0x5A);
}

#[test]
fn st_z() {
    let word = ldd(0x0000, true, 1, 0);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn st_z_plus() {
    let word = 0x9201 | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(30, 0x01)]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn st_minus_z() {
    let word = 0x9202 | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0101);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[(30, 0x00)]);
    assert_eq!(cpu.peek(0x0100), 0x5A);
}

#[test]
fn std_z() {
    let word = ldd(0x0000, true, 1, 3);
    let cpu = exec(&[word], |cpu| {
        set_z(cpu, 0x0100);
        cpu.set_reg(1, 0x5A);
    });
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x0103), 0x5A);
}

#[test]
fn sts() {
    let cpu = exec(&[0x9210, 0x0025], |cpu| cpu.set_reg(1, 0xA5));
    expect(&cpu, 2, 0, &[]);
    assert_eq!(cpu.peek(0x0025), 0xA5);
}

#[test]
fn lpm() {
    let cpu = exec(&[0x95C8, 0xABCD], |cpu| set_z(cpu, 2));
    expect(&cpu, 1, 0, &[(0, 0xCD), (30, 0x02)]);
}

#[test]
fn lpm_z() {
    let word = 0x9004 | (4 << 4);
    let cpu = exec(&[word, 0xABCD], |cpu| set_z(cpu, 3));
    expect(&cpu, 1, 0, &[(4, 0xAB), (30, 0x03)]);
}

#[test]
fn lpm_z_plus() {
    let word = 0x9005 | (4 << 4);
    let cpu = exec(&[word, 0xABCD], |cpu| set_z(cpu, 2));
    expect(&cpu, 1, 0, &[(4, 0xCD), (30, 0x03)]);
}

#[test]
fn spm() {
    let cpu = exec(&[0x95E8, 0x0000], |cpu| {
        set_z(cpu, 2);
        cpu.set_reg(0, 0x34);
        cpu.set_reg(1, 0x12);
    });
    expect(&cpu, 1, 0, &[(0, 0x34), (30, 0x02)]);
    assert_eq!(cpu.flash_word(1), 0x0000);
    assert_eq!(
        cpu.spm_notes(),
        &[orbe_microchip::SpmNote { z: 2, word: 0x1234 }]
    );
}

#[test]
fn in_port() {
    let cpu = exec(&[0xB105], |cpu| cpu.poke(0x0025, 0xA5));
    expect(&cpu, 1, 0, &[(16, 0xA5)]);
}

#[test]
fn in_and_out_reach_sreg() {
    // I/O 0x3F is SREG. OUT then IN reads the byte back.
    let mut cpu = Cpu::new();
    cpu.set_reg(16, 0x85);
    cpu.load_words(&[out_reg(16, 0x3F), in_io(17, 0x3F)]);
    cpu.insns(2);
    assert_eq!(cpu.sreg(), 0x85);
    assert_eq!(cpu.reg(17), 0x85);
    assert_eq!(cpu.peek(SREG), 0x85);
}

#[test]
fn out_port() {
    let cpu = exec(&[0xB905], |cpu| cpu.set_reg(16, 0xA5));
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x0025), 0xA5);
}

#[test]
fn push() {
    // DS40002198 §6.86: the byte is stored at SP, then SP is post-decremented.
    // Reset SP is RAMEND, 0x08FF (DS40002061B).
    let word = 0x920F | (1 << 4);
    let cpu = exec(&[word], |cpu| cpu.set_reg(1, 0x5A));
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x08FF), 0x5A);
    assert_eq!(cpu.peek(0x005D), 0xFE);
    assert_eq!(cpu.peek(0x005E), 0x08);
}

#[test]
fn pop() {
    // DS40002198 §6.85: SP is pre-incremented, then the byte at SP is loaded.
    // Plant SP at 0x08FE so the increment reads the byte at RAMEND.
    let word = 0x900F | (1 << 4);
    let cpu = exec(&[word], |cpu| {
        cpu.poke(0x08FF, 0x5A);
        cpu.poke(0x005D, 0xFE);
        cpu.poke(0x005E, 0x08);
    });
    expect(&cpu, 1, 0, &[(1, 0x5A)]);
    assert_eq!(cpu.peek(0x005D), 0xFF);
    assert_eq!(cpu.peek(0x005E), 0x08);
}

#[test]
fn out_of_one_stack_byte_keeps_the_other() {
    // I/O 0x3D is SPL and 0x3E is SPH. Reset SP is 0x08FF.
    let mut cpu = Cpu::new();
    cpu.set_reg(16, 0x10);
    cpu.load_words(&[out_reg(16, 0x3D)]);
    cpu.step();
    assert_eq!(cpu.peek(SPL), 0x10);
    assert_eq!(cpu.peek(SPH), 0x08, "OUT SPL keeps SPH");

    let mut cpu = Cpu::new();
    cpu.set_reg(16, 0x01);
    cpu.load_words(&[out_reg(16, 0x3E)]);
    cpu.step();
    assert_eq!(cpu.peek(SPL), 0xFF, "OUT SPH keeps SPL");
    assert_eq!(cpu.peek(SPH), 0x01);
}

#[test]
fn stack_pointer_wraps_through_zero_and_ffff() {
    // SP is a 16-bit register (DS40002061B §7.5). Address 0 is r0. A store
    // above RAMEND is dropped and a load from there reads 0.
    let push = 0x920F | (1 << 4);
    let pop = 0x900F | (2 << 4);

    let mut cpu = Cpu::new();
    cpu.set_reg(1, 0x5A);
    cpu.poke(SPL, 0x00);
    cpu.poke(SPH, 0x00);
    cpu.load_words(&[push]);
    cpu.step();
    assert_eq!(cpu.reg(0), 0x5A, "the byte is stored at address 0");
    assert_eq!(cpu.peek(SPL), 0xFF);
    assert_eq!(cpu.peek(SPH), 0xFF);

    cpu.load_words(&[pop]);
    cpu.step();
    assert_eq!(cpu.reg(2), 0x5A, "pop wraps back onto that byte");
    assert_eq!(cpu.peek(SPL), 0x00);
    assert_eq!(cpu.peek(SPH), 0x00);

    cpu.set_reg(1, 0xA5);
    cpu.poke(SPL, 0xFF);
    cpu.poke(SPH, 0xFF);
    cpu.load_words(&[push]);
    cpu.step();
    assert_eq!(cpu.peek(0xFFFF), 0, "0xFFFF is above RAMEND");
    assert_eq!(cpu.peek(SPL), 0xFE);
    assert_eq!(cpu.peek(SPH), 0xFF);

    cpu.load_words(&[pop]);
    cpu.step();
    assert_eq!(cpu.reg(2), 0);
    assert_eq!(cpu.peek(SPL), 0xFF);
    assert_eq!(cpu.peek(SPH), 0xFF);
}

#[test]
fn call_leaves_pch_at_sp_plus_one_and_pcl_at_sp_plus_two() {
    // DS40002198 §6.32 and §6.87: post-decrement, SP ← SP − 2. Those sections
    // do not say which byte is stored first. The model stores PCL first, so
    // [SP+1] is PCH and [SP+2] is PCL.
    // RCALL +0 at word 0 returns to word 1. RAMEND is 0x08FF, so SP ends at 0x08FD.
    let cpu = exec(&[0xD000], |_| {});
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.peek(0x005D), 0xFD);
    assert_eq!(cpu.peek(0x005E), 0x08);
    assert_eq!(cpu.peek(0x08FE), 0x00, "PCH at SP+1");
    assert_eq!(cpu.peek(0x08FF), 0x01, "PCL at SP+2");
}

#[test]
fn nop() {
    expect(&exec(&[0x0000], |_| {}), 1, 0, &[]);
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
fn wdr() {
    let cpu = exec(&[0x95A8], |_| {});
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.wdr_executions(), 1);
}

#[test]
fn break_insn() {
    let mut cpu = exec(&[0x9598, 0x0000], |_| {});
    expect(&cpu, 1, 0, &[]);
    assert!(cpu.is_stopped());
    let cycles = cpu.cycles();
    cpu.step();
    expect(&cpu, 1, 0, &[]);
    assert_eq!(cpu.cycles(), cycles, "a second step does not retire");
}

#[test]
fn unknown_opcode_is_recorded() {
    let cpu = exec(&[0xFFFF], |_| {});
    assert_eq!(cpu.unknown(), Some(0xFFFF));
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

#[test]
fn load_flash_is_little_endian() {
    let mut cpu = Cpu::new();
    cpu.load_flash(&[0x12, 0x0C]);
    cpu.set_reg(1, 0x0F);
    cpu.set_reg(2, 0x01);
    cpu.step();
    expect(&cpu, 1, f(1, 0, 0, 0, 0, 0), &[(1, 0x10)]);
}
