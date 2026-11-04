//! The ops of the ISA's options beyond the base set: the floating-point coprocessor, MAC16, the
//! Boolean registers, the conditional store, and the S3's `cop_ai` registers as plain state.
//! Each costs one cycle. An op none of these names, and no register op the table names, ends
//! the run as unimplemented, among them the classic `f64*` ops and the S3's `ee.*` ops.

use super::{Core, Missing, Trap, taken};
use crate::bus::{Bus, Cause, Width};
use crate::config::Config;
use crate::decode::{Insn, Op};
use crate::fpu::{self, Compare, Out, Round, ToInt};
use crate::special::{Sr, register_op};

/// The MAC16 registers `m0` to `m3`.
const MR: [Sr; 4] = [Sr::M0, Sr::M1, Sr::M2, Sr::M3];

/// What a MAC16 op does with `ACC`.
#[derive(Clone, Copy)]
enum Acc {
    /// `mul`: the signed product.
    Set,
    /// `umul`: the unsigned product.
    Unsigned,
    /// `mula`: `ACC` plus the signed product.
    Add,
    /// `muls`: `ACC` minus the signed product.
    Sub,
}

/// Where a MAC16 op's two operands come from: an address register (`a`) or a MAC16 register
/// (`d`), first and second.
#[derive(Clone, Copy)]
enum Pair {
    AA,
    AD,
    DA,
    DD,
}

/// Which 16-bit half of each operand, first and second. `H` is bits 16 to 31.
#[derive(Clone, Copy)]
enum Half {
    LL,
    HL,
    LH,
    HH,
}

/// The MAC16 ops, with the step a `ldinc` or `lddec` form moves its address register by.
fn mac16(op: Op) -> Option<(Acc, Pair, Half, i32)> {
    Some(match op {
        Op::MUL_AA_LL => (Acc::Set, Pair::AA, Half::LL, 0),
        Op::MUL_AA_HL => (Acc::Set, Pair::AA, Half::HL, 0),
        Op::MUL_AA_LH => (Acc::Set, Pair::AA, Half::LH, 0),
        Op::MUL_AA_HH => (Acc::Set, Pair::AA, Half::HH, 0),
        Op::MUL_AD_LL => (Acc::Set, Pair::AD, Half::LL, 0),
        Op::MUL_AD_HL => (Acc::Set, Pair::AD, Half::HL, 0),
        Op::MUL_AD_LH => (Acc::Set, Pair::AD, Half::LH, 0),
        Op::MUL_AD_HH => (Acc::Set, Pair::AD, Half::HH, 0),
        Op::MUL_DA_LL => (Acc::Set, Pair::DA, Half::LL, 0),
        Op::MUL_DA_HL => (Acc::Set, Pair::DA, Half::HL, 0),
        Op::MUL_DA_LH => (Acc::Set, Pair::DA, Half::LH, 0),
        Op::MUL_DA_HH => (Acc::Set, Pair::DA, Half::HH, 0),
        Op::MUL_DD_LL => (Acc::Set, Pair::DD, Half::LL, 0),
        Op::MUL_DD_HL => (Acc::Set, Pair::DD, Half::HL, 0),
        Op::MUL_DD_LH => (Acc::Set, Pair::DD, Half::LH, 0),
        Op::MUL_DD_HH => (Acc::Set, Pair::DD, Half::HH, 0),
        Op::UMUL_AA_LL => (Acc::Unsigned, Pair::AA, Half::LL, 0),
        Op::UMUL_AA_HL => (Acc::Unsigned, Pair::AA, Half::HL, 0),
        Op::UMUL_AA_LH => (Acc::Unsigned, Pair::AA, Half::LH, 0),
        Op::UMUL_AA_HH => (Acc::Unsigned, Pair::AA, Half::HH, 0),
        Op::MULA_AA_LL => (Acc::Add, Pair::AA, Half::LL, 0),
        Op::MULA_AA_HL => (Acc::Add, Pair::AA, Half::HL, 0),
        Op::MULA_AA_LH => (Acc::Add, Pair::AA, Half::LH, 0),
        Op::MULA_AA_HH => (Acc::Add, Pair::AA, Half::HH, 0),
        Op::MULA_AD_LL => (Acc::Add, Pair::AD, Half::LL, 0),
        Op::MULA_AD_HL => (Acc::Add, Pair::AD, Half::HL, 0),
        Op::MULA_AD_LH => (Acc::Add, Pair::AD, Half::LH, 0),
        Op::MULA_AD_HH => (Acc::Add, Pair::AD, Half::HH, 0),
        Op::MULA_DA_LL => (Acc::Add, Pair::DA, Half::LL, 0),
        Op::MULA_DA_HL => (Acc::Add, Pair::DA, Half::HL, 0),
        Op::MULA_DA_LH => (Acc::Add, Pair::DA, Half::LH, 0),
        Op::MULA_DA_HH => (Acc::Add, Pair::DA, Half::HH, 0),
        Op::MULA_DD_LL => (Acc::Add, Pair::DD, Half::LL, 0),
        Op::MULA_DD_HL => (Acc::Add, Pair::DD, Half::HL, 0),
        Op::MULA_DD_LH => (Acc::Add, Pair::DD, Half::LH, 0),
        Op::MULA_DD_HH => (Acc::Add, Pair::DD, Half::HH, 0),
        Op::MULS_AA_LL => (Acc::Sub, Pair::AA, Half::LL, 0),
        Op::MULS_AA_HL => (Acc::Sub, Pair::AA, Half::HL, 0),
        Op::MULS_AA_LH => (Acc::Sub, Pair::AA, Half::LH, 0),
        Op::MULS_AA_HH => (Acc::Sub, Pair::AA, Half::HH, 0),
        Op::MULS_AD_LL => (Acc::Sub, Pair::AD, Half::LL, 0),
        Op::MULS_AD_HL => (Acc::Sub, Pair::AD, Half::HL, 0),
        Op::MULS_AD_LH => (Acc::Sub, Pair::AD, Half::LH, 0),
        Op::MULS_AD_HH => (Acc::Sub, Pair::AD, Half::HH, 0),
        Op::MULS_DA_LL => (Acc::Sub, Pair::DA, Half::LL, 0),
        Op::MULS_DA_HL => (Acc::Sub, Pair::DA, Half::HL, 0),
        Op::MULS_DA_LH => (Acc::Sub, Pair::DA, Half::LH, 0),
        Op::MULS_DA_HH => (Acc::Sub, Pair::DA, Half::HH, 0),
        Op::MULS_DD_LL => (Acc::Sub, Pair::DD, Half::LL, 0),
        Op::MULS_DD_HL => (Acc::Sub, Pair::DD, Half::HL, 0),
        Op::MULS_DD_LH => (Acc::Sub, Pair::DD, Half::LH, 0),
        Op::MULS_DD_HH => (Acc::Sub, Pair::DD, Half::HH, 0),
        Op::MULA_DA_LL_LDINC => (Acc::Add, Pair::DA, Half::LL, 4),
        Op::MULA_DA_LL_LDDEC => (Acc::Add, Pair::DA, Half::LL, -4),
        Op::MULA_DA_HL_LDINC => (Acc::Add, Pair::DA, Half::HL, 4),
        Op::MULA_DA_HL_LDDEC => (Acc::Add, Pair::DA, Half::HL, -4),
        Op::MULA_DA_LH_LDINC => (Acc::Add, Pair::DA, Half::LH, 4),
        Op::MULA_DA_LH_LDDEC => (Acc::Add, Pair::DA, Half::LH, -4),
        Op::MULA_DA_HH_LDINC => (Acc::Add, Pair::DA, Half::HH, 4),
        Op::MULA_DA_HH_LDDEC => (Acc::Add, Pair::DA, Half::HH, -4),
        Op::MULA_DD_LL_LDINC => (Acc::Add, Pair::DD, Half::LL, 4),
        Op::MULA_DD_LL_LDDEC => (Acc::Add, Pair::DD, Half::LL, -4),
        Op::MULA_DD_HL_LDINC => (Acc::Add, Pair::DD, Half::HL, 4),
        Op::MULA_DD_HL_LDDEC => (Acc::Add, Pair::DD, Half::HL, -4),
        Op::MULA_DD_LH_LDINC => (Acc::Add, Pair::DD, Half::LH, 4),
        Op::MULA_DD_LH_LDDEC => (Acc::Add, Pair::DD, Half::LH, -4),
        Op::MULA_DD_HH_LDINC => (Acc::Add, Pair::DD, Half::HH, 4),
        Op::MULA_DD_HH_LDDEC => (Acc::Add, Pair::DD, Half::HH, -4),
        _ => return None,
    })
}

impl<C: Config> Core<C> {
    /// Runs an op outside the base set, or the register op the table names. Returns the target
    /// of a taken `bt` or `bf`.
    pub(super) fn option_op<B: Bus>(
        &mut self,
        bus: &mut B,
        insn: &Insn,
    ) -> Result<Option<u32>, Trap> {
        let f = insn.fields;
        let op = insn.op;
        let rm = Round::of_fcr(self.sr[Sr::Fcr as usize]);
        match op {
            Op::LSI => {
                let value = self.load(bus, op, self.a(f[1]).wrapping_add(f[2]), Width::Word)?;
                self.set_f(f[0], value);
            }
            Op::LSX => {
                let addr = self.a(f[1]).wrapping_add(self.a(f[2]));
                let value = self.load(bus, op, addr, Width::Word)?;
                self.set_f(f[0], value);
            }
            Op::LSIP | Op::LSXP => {
                let addr = self.a(f[1]);
                let step = if op == Op::LSIP { f[2] } else { self.a(f[2]) };
                let value = self.load(bus, op, addr, Width::Word)?;
                self.set_f(f[0], value);
                self.set_a(f[1], addr.wrapping_add(step));
            }
            Op::SSI => {
                let addr = self.a(f[1]).wrapping_add(f[2]);
                self.store(bus, op, addr, Width::Word, self.f(f[0]))?;
            }
            Op::SSX => {
                let addr = self.a(f[1]).wrapping_add(self.a(f[2]));
                self.store(bus, op, addr, Width::Word, self.f(f[0]))?;
            }
            Op::SSIP | Op::SSXP => {
                let addr = self.a(f[1]);
                let step = if op == Op::SSIP { f[2] } else { self.a(f[2]) };
                self.store(bus, op, addr, Width::Word, self.f(f[0]))?;
                self.set_a(f[1], addr.wrapping_add(step));
            }

            Op::ADD_S => self.float(f[0], fpu::add(self.f(f[1]), self.f(f[2]), rm)),
            Op::SUB_S => self.float(f[0], fpu::sub(self.f(f[1]), self.f(f[2]), rm)),
            Op::MUL_S => self.float(f[0], fpu::mul(self.f(f[1]), self.f(f[2]), rm)),
            Op::MADD_S => self.float(
                f[0],
                fpu::fused(self.f(f[0]), self.f(f[1]), self.f(f[2]), false, rm),
            ),
            Op::MSUB_S => self.float(
                f[0],
                fpu::fused(self.f(f[0]), self.f(f[1]), self.f(f[2]), true, rm),
            ),
            Op::MADDN_S => self.set_f(f[0], fpu::maddn(self.f(f[0]), self.f(f[1]), self.f(f[2]))),
            Op::DIVN_S => self.float(
                f[0],
                fpu::divn(self.f(f[0]), self.f(f[1]), self.f(f[2]), rm),
            ),
            Op::FLOAT_S => self.float(f[0], fpu::from_int(self.a(f[1]), f[2], true, rm)),
            Op::UFLOAT_S => self.float(f[0], fpu::from_int(self.a(f[1]), f[2], false, rm)),
            Op::TRUNC_S => self.integer(f[0], fpu::to_signed(self.f(f[1]), f[2], ToInt::Trunc)),
            Op::ROUND_S => self.integer(f[0], fpu::to_signed(self.f(f[1]), f[2], ToInt::Nearest)),
            Op::FLOOR_S => self.integer(f[0], fpu::to_signed(self.f(f[1]), f[2], ToInt::Floor)),
            Op::CEIL_S => self.integer(f[0], fpu::to_signed(self.f(f[1]), f[2], ToInt::Ceil)),
            Op::UTRUNC_S => self.integer(f[0], fpu::to_unsigned(self.f(f[1]), f[2])),
            Op::OEQ_S => self.compare(f, Compare::Oeq),
            Op::OLT_S => self.compare(f, Compare::Olt),
            Op::OLE_S => self.compare(f, Compare::Ole),
            Op::UEQ_S => self.compare(f, Compare::Ueq),
            Op::ULT_S => self.compare(f, Compare::Ult),
            Op::ULE_S => self.compare(f, Compare::Ule),
            Op::UN_S => self.compare(f, Compare::Un),
            Op::MOV_S => self.set_f(f[0], self.f(f[1])),
            Op::NEG_S => self.set_f(f[0], self.f(f[1]) ^ 0x8000_0000),
            Op::ABS_S => self.set_f(f[0], self.f(f[1]) & 0x7fff_ffff),
            Op::MOVEQZ_S => self.move_float_if(f, self.a(f[2]) == 0),
            Op::MOVNEZ_S => self.move_float_if(f, self.a(f[2]) != 0),
            Op::MOVLTZ_S => self.move_float_if(f, (self.a(f[2]) as i32) < 0),
            Op::MOVGEZ_S => self.move_float_if(f, (self.a(f[2]) as i32) >= 0),
            Op::MOVF_S => self.move_float_if(f, !self.b(f[2])),
            Op::MOVT_S => self.move_float_if(f, self.b(f[2])),
            Op::RFR => self.set_a(f[0], self.f(f[1])),
            Op::WFR => self.set_f(f[0], self.a(f[1])),
            Op::CONST_S => self.set_f(f[0], fpu::constant(f[1])),
            Op::NEXP01_S => self.set_f(f[0], fpu::nexp01(self.f(f[1]))),
            Op::ADDEXP_S => self.set_f(f[0], fpu::addexp(self.f(f[0]), self.f(f[1]))),
            Op::ADDEXPM_S => self.set_f(f[0], fpu::addexpm(self.f(f[0]), self.f(f[1]))),
            Op::MKSADJ_S => self.float(f[0], fpu::mksadj(self.f(f[1]))),
            Op::MKDADJ_S => self.float(f[0], fpu::mkdadj(self.f(f[0]), self.f(f[1]))),
            Op::DIV0_S => self.set_f(f[0], fpu::div0(self.f(f[1]))),
            Op::SQRT0_S => self.set_f(f[0], fpu::sqrt0(self.f(f[1]))),
            Op::RECIP0_S => self.float(f[0], fpu::recip0(self.f(f[1]))),
            Op::RSQRT0_S => self.float(f[0], fpu::rsqrt0(self.f(f[1]))),

            Op::ANDB => self.set_b(f[0], self.b(f[1]) & self.b(f[2])),
            Op::ANDBC => self.set_b(f[0], self.b(f[1]) & !self.b(f[2])),
            Op::ORB => self.set_b(f[0], self.b(f[1]) | self.b(f[2])),
            Op::ORBC => self.set_b(f[0], self.b(f[1]) | !self.b(f[2])),
            Op::XORB => self.set_b(f[0], self.b(f[1]) ^ self.b(f[2])),
            Op::ANY4 => self.set_b(f[0], self.booleans(f[1], 4) != 0),
            Op::ALL4 => self.set_b(f[0], self.booleans(f[1], 4) == 0xf),
            Op::ANY8 => self.set_b(f[0], self.booleans(f[1], 8) != 0),
            Op::ALL8 => self.set_b(f[0], self.booleans(f[1], 8) == 0xff),
            Op::BF => return Ok(taken(!self.b(f[0]), f[1])),
            Op::BT => return Ok(taken(self.b(f[0]), f[1])),
            Op::MOVF => {
                if !self.b(f[2]) {
                    self.set_a(f[0], self.a(f[1]));
                }
            }
            Op::MOVT => {
                if self.b(f[2]) {
                    self.set_a(f[0], self.a(f[1]));
                }
            }

            // The memory's old word goes to `at` whether or not it matched. The ISA lets some
            // implementations return `!SCOMPARE1` when the store does not happen, and has the op
            // check store permission whether or not it stores, never raising `LoadProhibited`
            // (8.3.281, p 590). This core always returns the old word, raises a refused compare
            // read as `StoreProhibited`, and asks the bus about the store when the compare
            // fails.
            Op::S32C1I => {
                let addr = self.a(f[1]).wrapping_add(f[2]);
                let old = self
                    .load(bus, op, addr, Width::Word)
                    .map_err(|trap| match trap {
                        Trap::Exception(Cause::LoadProhibited, at) => {
                            Trap::Exception(Cause::StoreProhibited, at)
                        }
                        trap => trap,
                    })?;
                if old == self.sr[Sr::Scompare1 as usize] {
                    self.store(bus, op, addr, Width::Word, self.a(f[0]))?;
                } else {
                    bus.check_store(self.cycle, addr)
                        .map_err(|exit| self.access_trap(exit, op, addr, Width::Word))?;
                }
                self.set_a(f[0], old);
            }

            // `ld.qr` and `st.qr` move 16 bytes at `as` plus the offset, as four words, lowest
            // first. No fixture row uses an address that is not a multiple of 16, so the core
            // stops before one. The four words then share one 16-byte block, and a `st.qr`
            // keeps the words before a faulting one only on a bus that faults inside a block.
            Op::ld_QR => {
                let addr = self.q_address(op, f)?;
                let mut words = [0; 4];
                for (i, word) in words.iter_mut().enumerate() {
                    *word = self.load(bus, op, addr.wrapping_add(4 * i as u32), Width::Word)?;
                }
                self.q[f[0] as usize] = words;
            }
            Op::st_QR => {
                let addr = self.q_address(op, f)?;
                for (i, word) in self.q[f[0] as usize].into_iter().enumerate() {
                    self.store(bus, op, addr.wrapping_add(4 * i as u32), Width::Word, word)?;
                }
            }
            Op::mv_QR => self.q[f[0] as usize] = self.q[f[1] as usize],

            Op::LDINC => self.load_mr(bus, insn, 4)?,
            Op::LDDEC => self.load_mr(bus, insn, -4)?,
            op => {
                if let Some((acc, pair, half, step)) = mac16(op) {
                    self.mac16(bus, insn, acc, pair, half, step)?;
                } else if let Some((access, sr)) = register_op(op) {
                    self.register_op(access, sr, f[0]);
                } else {
                    return Err(Trap::missing(self.pc, Missing::Op(op)));
                }
            }
        }
        Ok(None)
    }

    fn float(&mut self, r: u32, (bits, flags): Out) {
        self.set_f(r, bits);
        self.sr[Sr::Fsr as usize] |= flags;
    }

    fn integer(&mut self, r: u32, (bits, flags): Out) {
        self.set_a(r, bits);
        self.sr[Sr::Fsr as usize] |= flags;
    }

    fn compare(&mut self, f: [u32; 8], kind: Compare) {
        let (result, flags) = fpu::compare(self.f(f[1]), self.f(f[2]), kind);
        self.set_b(f[0], result);
        self.sr[Sr::Fsr as usize] |= flags;
    }

    fn move_float_if(&mut self, f: [u32; 8], test: bool) {
        if test {
            self.set_f(f[0], self.f(f[1]));
        }
    }

    /// Boolean register `n`.
    fn b(&self, n: u32) -> bool {
        self.sr[Sr::Br as usize] >> n & 1 != 0
    }

    fn set_b(&mut self, n: u32, value: bool) {
        let br = &mut self.sr[Sr::Br as usize];
        *br = (*br & !(1 << n)) | (u32::from(value) << n);
    }

    /// The address of a `ld.qr` or `st.qr`, `as` plus the offset, which must be a multiple of 16.
    fn q_address(&self, op: Op, f: [u32; 8]) -> Result<u32, Trap> {
        let addr = self.a(f[1]).wrapping_add(f[2]);
        if addr & 15 != 0 {
            return Err(Trap::missing(self.pc, Missing::Unaligned(op)));
        }
        Ok(addr)
    }

    /// `count` Boolean registers from `first`, as bits.
    fn booleans(&self, first: u32, count: u32) -> u32 {
        (self.sr[Sr::Br as usize] >> first) & ((1 << count) - 1)
    }

    /// `ACC` is `ACCHI` sign-extended over `ACCLO`, 40 bits. The product is of the chosen 16-bit
    /// halves of the two operands. A `ldinc` or `lddec` form multiplies the registers as they
    /// were, then loads `mw` from `as` plus the step and writes that address to `as`. Its
    /// operands are `mw, as, mx, at` or `mw, as, mx, my`.
    fn mac16<B: Bus>(
        &mut self,
        bus: &mut B,
        insn: &Insn,
        acc: Acc,
        pair: Pair,
        half: Half,
        step: i32,
    ) -> Result<(), Trap> {
        let f = insn.fields;
        let operands = if step == 0 { &f[..2] } else { &f[2..4] };
        let m = |n: u32| self.sr[MR[n as usize] as usize];
        let (x, y) = match pair {
            Pair::AA => (self.a(operands[0]), self.a(operands[1])),
            Pair::AD => (self.a(operands[0]), m(operands[1])),
            Pair::DA => (m(operands[0]), self.a(operands[1])),
            Pair::DD => (m(operands[0]), m(operands[1])),
        };
        let (x_high, y_high) = match half {
            Half::LL => (false, false),
            Half::HL => (true, false),
            Half::LH => (false, true),
            Half::HH => (true, true),
        };
        let x = if x_high { x >> 16 } else { x } as u16;
        let y = if y_high { y >> 16 } else { y } as u16;
        let signed = i64::from(x as i16) * i64::from(y as i16);
        let old = i64::from(self.special(Sr::Acchi) as i32) << 32
            | i64::from(self.sr[Sr::Acclo as usize]);
        let value = match acc {
            Acc::Set => signed,
            Acc::Unsigned => i64::from(u32::from(x) * u32::from(y)),
            Acc::Add => old.wrapping_add(signed),
            Acc::Sub => old.wrapping_sub(signed),
        };
        if step != 0 {
            self.load_mr(bus, insn, step)?;
        }
        self.sr[Sr::Acclo as usize] = value as u32;
        self.sr[Sr::Acchi as usize] = (value >> 32) as u32 & 0xff;
        Ok(())
    }

    /// `ldinc` and `lddec`, and the load of a MAC16 op that has one: `mw` from `as` plus `step`,
    /// and that address to `as`.
    fn load_mr<B: Bus>(&mut self, bus: &mut B, insn: &Insn, step: i32) -> Result<(), Trap> {
        let (w, s) = (insn.fields[0], insn.fields[1]);
        let addr = self.a(s).wrapping_add(step as u32);
        let value = self.load(bus, insn.op, addr, Width::Word)?;
        self.sr[MR[w as usize] as usize] = value;
        self.set_a(s, addr);
        Ok(())
    }
}
