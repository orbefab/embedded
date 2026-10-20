//! Exhaustive flag oracle for the arithmetic instructions.
//!
//! H, V, and C come from a wide integer, not from the core's bit formulas.
//! Half-carry is the carry out of the low nibble. Signed overflow is the
//! result leaving the `i8` or `i16` range. Z is sticky only for SBC, SBCI,
//! and CPC.

use crate::common;
use common::{adiw, imm, rr as reg_op};

use sim_avr::Cpu;

const I: u8 = 1 << 7;
const T: u8 = 1 << 6;
const H: u8 = 1 << 5;
const S: u8 = 1 << 4;
const V: u8 = 1 << 3;
const N: u8 = 1 << 2;
const Z: u8 = 1 << 1;
const C: u8 = 1 << 0;

struct Flags {
    h: bool,
    s: bool,
    v: bool,
    n: bool,
    z: bool,
    c: bool,
    /// Bits the instruction does not modify.
    keep: u8,
}

fn pack(flags: Flags, incoming: u8) -> u8 {
    let computed = u8::from(flags.h) << 5
        | u8::from(flags.s) << 4
        | u8::from(flags.v) << 3
        | u8::from(flags.n) << 2
        | u8::from(flags.z) << 1
        | u8::from(flags.c);
    let touched = H | S | V | N | Z | C;
    (incoming & flags.keep & !touched)
        | (incoming & flags.keep & touched)
        | (computed & !flags.keep)
}

fn add_model(rd: u8, rr: u8, cin: bool) -> (u8, bool, bool, bool) {
    let cin = u16::from(cin);
    let wide = u16::from(rd) + u16::from(rr) + cin;
    let nibble = u16::from(rd & 0x0F) + u16::from(rr & 0x0F) + cin;
    let sum = i16::from(rd as i8) + i16::from(rr as i8) + cin as i16;
    (
        wide as u8,
        nibble > 0x0F,
        !(-128..=127).contains(&sum),
        wide > 0xFF,
    )
}

fn sub_model(rd: u8, rr: u8, cin: bool) -> (u8, bool, bool, bool) {
    let cin = u16::from(cin);
    let wide = i32::from(rd) - i32::from(rr) - i32::from(cin);
    let nibble_rd = u16::from(rd & 0x0F);
    let nibble_rr = u16::from(rr & 0x0F) + cin;
    let diff = i16::from(rd as i8) - i16::from(rr as i8) - cin as i16;
    (
        wide as u8,
        nibble_rd < nibble_rr,
        !(-128..=127).contains(&diff),
        wide < 0,
    )
}

fn byte_flags(result: u8, h: bool, v: bool, c: bool, z_in: bool, sticky: bool) -> Flags {
    let n = result & 0x80 != 0;
    let z_new = result == 0;
    let z = if sticky { z_new && z_in } else { z_new };
    Flags {
        h,
        s: n ^ v,
        v,
        n,
        z,
        c,
        keep: I | T,
    }
}

#[allow(clippy::too_many_arguments)]
fn check(
    cpu: &mut Cpu,
    word: u16,
    rd: u8,
    rr: u8,
    pair: u16,
    incoming: u8,
    expect_reg: Option<u8>,
    flags: Flags,
) {
    cpu.load_words(&[word]);
    cpu.set_reg(16, rd);
    cpu.set_reg(17, rr);
    cpu.set_reg(24, pair as u8);
    cpu.set_reg(25, (pair >> 8) as u8);
    cpu.set_sreg(incoming);
    cpu.step();
    let got = cpu.sreg();
    let want = pack(flags, incoming);
    assert_eq!(
        got, want,
        "word {word:#06x} rd {rd:#04x} rr {rr:#04x} pair {pair:#06x} in {incoming:#04x} got {got:#010b} want {want:#010b}"
    );
    if let Some(value) = expect_reg {
        assert_eq!(cpu.reg(16), value, "word {word:#06x} result");
    }
}

#[test]
fn arithmetic_flags_match_a_wide_integer() {
    let mut cpu = Cpu::new();
    let preserve_it = I | T;
    for rd in 0..=255u8 {
        for rr in 0..=255u8 {
            for incoming_c in [false, true] {
                for incoming_z in [false, true] {
                    let incoming =
                        preserve_it | H | u8::from(incoming_c) | (u8::from(incoming_z) << 1);
                    let (sum, h, v, c) = add_model(rd, rr, false);
                    check(
                        &mut cpu,
                        reg_op(0x0C00, 16, 17),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(sum),
                        byte_flags(sum, h, v, c, incoming_z, false),
                    );
                    let (sum, h, v, c) = add_model(rd, rr, incoming_c);
                    check(
                        &mut cpu,
                        reg_op(0x1C00, 16, 17),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(sum),
                        byte_flags(sum, h, v, c, incoming_z, false),
                    );
                    let (diff, h, v, c) = sub_model(rd, rr, false);
                    check(
                        &mut cpu,
                        reg_op(0x1800, 16, 17),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(diff),
                        byte_flags(diff, h, v, c, incoming_z, false),
                    );
                    let (diff, h, v, c) = sub_model(rd, rr, incoming_c);
                    check(
                        &mut cpu,
                        reg_op(0x0800, 16, 17),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(diff),
                        byte_flags(diff, h, v, c, incoming_z, true),
                    );
                    let (diff, h, v, c) = sub_model(rd, rr, false);
                    check(
                        &mut cpu,
                        imm(0x5000, 16, u16::from(rr)),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(diff),
                        byte_flags(diff, h, v, c, incoming_z, false),
                    );
                    let (diff, h, v, c) = sub_model(rd, rr, incoming_c);
                    check(
                        &mut cpu,
                        imm(0x4000, 16, u16::from(rr)),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(diff),
                        byte_flags(diff, h, v, c, incoming_z, true),
                    );
                    let (diff, h, v, c) = sub_model(rd, rr, false);
                    check(
                        &mut cpu,
                        reg_op(0x1400, 16, 17),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(rd),
                        byte_flags(diff, h, v, c, incoming_z, false),
                    );
                    let (diff, h, v, c) = sub_model(rd, rr, incoming_c);
                    check(
                        &mut cpu,
                        reg_op(0x0400, 16, 17),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(rd),
                        byte_flags(diff, h, v, c, incoming_z, true),
                    );
                    let (diff, h, v, c) = sub_model(rd, rr, false);
                    check(
                        &mut cpu,
                        imm(0x3000, 16, u16::from(rr)),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(rd),
                        byte_flags(diff, h, v, c, incoming_z, false),
                    );

                    let (neg, h, v, c) = sub_model(0, rd, false);
                    check(
                        &mut cpu,
                        0x9401 | (16 << 4),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(neg),
                        byte_flags(neg, h, v, c, incoming_z, false),
                    );
                }
            }
        }
    }

    // INC, DEC, COM, ASR, LSR, and ROR keep H. NEG and the
    // arithmetic ops above replace it, so one incoming H is enough for them.
    for rd in 0..=255u8 {
        for incoming_c in [false, true] {
            for incoming_z in [false, true] {
                for plant_h in [false, true] {
                    let incoming = preserve_it
                        | if plant_h { H } else { 0 }
                        | u8::from(incoming_c)
                        | (u8::from(incoming_z) << 1);
                    let rr = 0u8;
                    let inc = rd.wrapping_add(1);
                    let inc_v = i16::from(rd as i8) + 1 > 127;
                    let n = inc & 0x80 != 0;
                    check(
                        &mut cpu,
                        0x9403 | (16 << 4),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(inc),
                        Flags {
                            h: true,
                            s: n ^ inc_v,
                            v: inc_v,
                            n,
                            z: inc == 0,
                            c: incoming_c,
                            keep: I | T | H | C,
                        },
                    );

                    let dec = rd.wrapping_sub(1);
                    let dec_diff = i16::from(rd as i8) - 1;
                    let dec_v = dec_diff < -128;
                    let n = dec & 0x80 != 0;
                    check(
                        &mut cpu,
                        0x940A | (16 << 4),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(dec),
                        Flags {
                            h: true,
                            s: n ^ dec_v,
                            v: dec_v,
                            n,
                            z: dec == 0,
                            c: incoming_c,
                            keep: I | T | H | C,
                        },
                    );

                    let com = !rd;
                    let n = com & 0x80 != 0;
                    check(
                        &mut cpu,
                        0x9400 | (16 << 4),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(com),
                        Flags {
                            h: true,
                            s: n,
                            v: false,
                            n,
                            z: com == 0,
                            c: true,
                            keep: I | T | H,
                        },
                    );

                    let asr = ((rd as i8) >> 1) as u8;
                    let asr_c = rd & 1 != 0;
                    let n = asr & 0x80 != 0;
                    let v = n ^ asr_c;
                    check(
                        &mut cpu,
                        0x9405 | (16 << 4),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(asr),
                        Flags {
                            h: true,
                            s: n ^ v,
                            v,
                            n,
                            z: asr == 0,
                            c: asr_c,
                            keep: I | T | H,
                        },
                    );

                    let lsr = rd >> 1;
                    let lsr_c = rd & 1 != 0;
                    let v = lsr_c;
                    check(
                        &mut cpu,
                        0x9406 | (16 << 4),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(lsr),
                        Flags {
                            h: true,
                            s: v,
                            v,
                            n: false,
                            z: lsr == 0,
                            c: lsr_c,
                            keep: I | T | H,
                        },
                    );

                    let ror = (rd >> 1) | (u8::from(incoming_c) << 7);
                    let ror_c = rd & 1 != 0;
                    let n = ror & 0x80 != 0;
                    let v = n ^ ror_c;
                    check(
                        &mut cpu,
                        0x9407 | (16 << 4),
                        rd,
                        rr,
                        0,
                        incoming,
                        Some(ror),
                        Flags {
                            h: true,
                            s: n ^ v,
                            v,
                            n,
                            z: ror == 0,
                            c: ror_c,
                            keep: I | T | H,
                        },
                    );
                }
            }
        }
    }

    for pair in 0..=65535u16 {
        for k in 0..64u16 {
            for incoming_c in [false, true] {
                for incoming_z in [false, true] {
                    let incoming =
                        preserve_it | H | u8::from(incoming_c) | (u8::from(incoming_z) << 1);
                    let wide = u32::from(pair) + u32::from(k);
                    let result = wide as u16;
                    let signed = i32::from(pair as i16) + i32::from(k);
                    let v = signed > i32::from(i16::MAX);
                    let n = result & 0x8000 != 0;
                    let c = wide > 0xFFFF;
                    check(
                        &mut cpu,
                        adiw(k, 24),
                        0,
                        0,
                        pair,
                        incoming,
                        None,
                        Flags {
                            h: true,
                            s: n ^ v,
                            v,
                            n,
                            z: result == 0,
                            c,
                            keep: I | T | H,
                        },
                    );
                    assert_eq!(cpu.reg(24), result as u8);
                    assert_eq!(cpu.reg(25), (result >> 8) as u8);

                    let result = pair.wrapping_sub(k);
                    let signed = i32::from(pair as i16) - i32::from(k);
                    let v = signed < i32::from(i16::MIN);
                    let n = result & 0x8000 != 0;
                    let c = u32::from(pair) < u32::from(k);
                    check(
                        &mut cpu,
                        adiw(k, 24) | 0x0100,
                        0,
                        0,
                        pair,
                        incoming,
                        None,
                        Flags {
                            h: true,
                            s: n ^ v,
                            v,
                            n,
                            z: result == 0,
                            c,
                            keep: I | T | H,
                        },
                    );
                    assert_eq!(cpu.reg(24), result as u8, "sbiw low");
                    assert_eq!(cpu.reg(25), (result >> 8) as u8, "sbiw high");
                }
            }
        }
    }

    // H is preserved, so the clear case is the sign boundaries and three immediate values.
    for pair in [
        0x0000u16, 0x0001, 0x003F, 0x7FC0, 0x7FFF, 0x8000, 0x803F, 0xFFC0, 0xFFFF,
    ] {
        for k in [0u16, 1, 63] {
            for incoming_c in [false, true] {
                for incoming_z in [false, true] {
                    let incoming = preserve_it | u8::from(incoming_c) | (u8::from(incoming_z) << 1);
                    let wide = u32::from(pair) + u32::from(k);
                    let result = wide as u16;
                    let signed = i32::from(pair as i16) + i32::from(k);
                    let v = signed > i32::from(i16::MAX);
                    let n = result & 0x8000 != 0;
                    let c = wide > 0xFFFF;
                    check(
                        &mut cpu,
                        adiw(k, 24),
                        0,
                        0,
                        pair,
                        incoming,
                        None,
                        Flags {
                            h: true,
                            s: n ^ v,
                            v,
                            n,
                            z: result == 0,
                            c,
                            keep: I | T | H,
                        },
                    );
                    let result = pair.wrapping_sub(k);
                    let signed = i32::from(pair as i16) - i32::from(k);
                    let v = signed < i32::from(i16::MIN);
                    let n = result & 0x8000 != 0;
                    let c = u32::from(pair) < u32::from(k);
                    check(
                        &mut cpu,
                        adiw(k, 24) | 0x0100,
                        0,
                        0,
                        pair,
                        incoming,
                        None,
                        Flags {
                            h: true,
                            s: n ^ v,
                            v,
                            n,
                            z: result == 0,
                            c,
                            keep: I | T | H,
                        },
                    );
                }
            }
        }
    }
}
