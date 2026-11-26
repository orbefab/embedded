//! The ops a straight run takes most often, run in line.
//!
//! Before each op of a straight run the core makes the checks [`Core::run`] makes: the `j` to
//! itself, the window overflow, the coprocessor and the loop end. When none of them finds
//! anything, an op from the list below runs here, as [`Core::operate`] runs it, with its cycles,
//! its PC and the store bit [`Core::note`] keeps. Every other op, and every op a check stops,
//! runs through [`Core::run`]. So an op here skips the calls and the return through `run` and
//! `operate`, and a register op the long way through `option_op`.
//!
//! A check build also runs each op here that makes no access on a copy of the core through
//! [`Core::run`], and requires both cores to match.

use super::decodes::Kept;
use super::{Core, Missing, PACE_LOOP, PACE_STORE, PS_EXCM, Trap, taken};
use crate::bus::{Bus, Width};
use crate::config::Config;
use crate::decode::Op;
use crate::special::Sr;

impl<C: Config> Core<C> {
    /// Whether the checks [`Core::run`] makes before `kept`'s op find nothing: it is no `j` to
    /// itself, no frame its registers reach needs to spill, its coprocessor is on, and it is
    /// not a loop's last op while the loop goes back.
    #[inline(always)]
    pub(super) fn clear(&self, kept: &Kept) -> bool {
        let insn = &kept.insn;
        let seq = self.pc.wrapping_add(u32::from(insn.len));
        !(insn.op == Op::J && insn.fields[0] == self.pc)
            && self.overflow(u32::from(insn.reach)).is_none()
            && kept
                .coprocessor
                .is_none_or(|n| self.sr[Sr::CpEnable as usize] >> n & 1 != 0)
            && !(seq == self.sr[Sr::Lend as usize]
                && self.sr[Sr::Lcount as usize] != 0
                && self.ps() & PS_EXCM == 0)
    }

    /// Runs `kept`, which [`Core::clear`] passed, as [`Core::run`] does.
    #[inline(always)]
    pub(super) fn run_clear<B: Bus>(&mut self, bus: &mut B, kept: &Kept) -> Result<(), Trap> {
        #[cfg(feature = "poll-skip-check")]
        let mut check = pure(kept.insn.op).then(|| {
            let mut copy = self.clone();
            copy.run(bus, kept).map(|_| copy)
        });
        let insn = &kept.insn;
        let f = &insn.fields;
        let seq = self.pc.wrapping_add(u32::from(insn.len));
        let base = self.sr[Sr::WindowBase as usize] * 4;
        let at = |n: u32| ((base + n) & 63) as usize;
        let a = |core: &Self, n: u32| core.ar[at(n)];
        let write = |core: &mut Self, value: u32| core.ar[at(f[0])] = value;
        let sar = self.sr[Sr::Sar as usize];
        let mut cycles = 1;
        let mut stored = false;
        let mut target = None;
        match insn.op {
            Op::ADD | Op::ADD_N => write(self, a(self, f[1]).wrapping_add(a(self, f[2]))),
            Op::ADDX2 => write(self, (a(self, f[1]) << 1).wrapping_add(a(self, f[2]))),
            Op::ADDX4 => write(self, (a(self, f[1]) << 2).wrapping_add(a(self, f[2]))),
            Op::ADDX8 => write(self, (a(self, f[1]) << 3).wrapping_add(a(self, f[2]))),
            Op::SUB => write(self, a(self, f[1]).wrapping_sub(a(self, f[2]))),
            Op::SUBX2 => write(self, (a(self, f[1]) << 1).wrapping_sub(a(self, f[2]))),
            Op::SUBX4 => write(self, (a(self, f[1]) << 2).wrapping_sub(a(self, f[2]))),
            Op::SUBX8 => write(self, (a(self, f[1]) << 3).wrapping_sub(a(self, f[2]))),
            Op::ADDI | Op::ADDI_N | Op::ADDMI => write(self, a(self, f[1]).wrapping_add(f[2])),
            Op::AND => write(self, a(self, f[1]) & a(self, f[2])),
            Op::OR => write(self, a(self, f[1]) | a(self, f[2])),
            Op::XOR => write(self, a(self, f[1]) ^ a(self, f[2])),
            Op::NEG => write(self, a(self, f[1]).wrapping_neg()),
            Op::MOVI | Op::MOVI_N => write(self, f[1]),
            Op::MOV_N => write(self, a(self, f[1])),
            Op::MOVEQZ => {
                if a(self, f[2]) == 0 {
                    write(self, a(self, f[1]));
                }
            }
            Op::MOVNEZ => {
                if a(self, f[2]) != 0 {
                    write(self, a(self, f[1]));
                }
            }
            Op::MINU => write(self, a(self, f[1]).min(a(self, f[2]))),
            Op::MAXU => write(self, a(self, f[1]).max(a(self, f[2]))),
            Op::NSAU => write(self, a(self, f[1]).leading_zeros()),
            Op::EXTUI => write(self, (a(self, f[1]) >> f[2]) & ((1u32 << f[3]) - 1)),
            Op::SLLI => write(self, a(self, f[1]).checked_shl(f[2]).unwrap_or(0)),
            Op::SRLI => write(self, a(self, f[1]) >> f[2]),
            Op::SRAI => write(self, ((a(self, f[1]) as i32) >> f[2]) as u32),
            Op::SLL => write(self, ((u64::from(a(self, f[1])) << 32) >> sar) as u32),
            Op::SRL => write(self, (u64::from(a(self, f[1])) >> sar) as u32),
            Op::SRA => write(self, (i64::from(a(self, f[1]) as i32) >> sar) as u32),
            Op::SRC => {
                let pair = (u64::from(a(self, f[1])) << 32) | u64::from(a(self, f[2]));
                write(self, (pair >> sar) as u32);
            }
            Op::SSR => self.sr[Sr::Sar as usize] = a(self, f[0]) & 31,
            Op::SSL => self.sr[Sr::Sar as usize] = 32 - (a(self, f[0]) & 31),
            Op::SSA8L => self.sr[Sr::Sar as usize] = (a(self, f[0]) & 3) * 8,
            Op::SSAI => self.sr[Sr::Sar as usize] = f[0],
            Op::MULL => write(self, a(self, f[1]).wrapping_mul(a(self, f[2]))),
            // `register_op`'s read of `CCOUNT`, which no `ICOUNT` lag touches.
            Op::RSR_CCOUNT => write(self, self.special(Sr::Ccount)),
            Op::MEMW => {
                if (self.pace & PACE_STORE) != 0 {
                    cycles = C::memw_after_store();
                }
            }
            Op::NOP | Op::NOP_N => {}

            Op::L8UI | Op::L16UI | Op::L16SI | Op::L32I | Op::L32I_N => {
                let width = match insn.op {
                    Op::L8UI => Width::Byte,
                    Op::L16UI | Op::L16SI => Width::Half,
                    _ => Width::Word,
                };
                let value = self.load(bus, insn.op, a(self, f[1]).wrapping_add(f[2]), width)?;
                let value = if insn.op == Op::L16SI {
                    value as i16 as u32
                } else {
                    value
                };
                write(self, value);
            }
            Op::L32R => {
                if self.sr[Sr::Litbase as usize] & 1 != 0 {
                    return Err(Trap::missing(self.pc, Missing::Litbase));
                }
                let value = self.load(bus, insn.op, f[1], Width::Word)?;
                write(self, value);
            }
            Op::S8I | Op::S16I | Op::S32I | Op::S32I_N => {
                let width = match insn.op {
                    Op::S8I => Width::Byte,
                    Op::S16I => Width::Half,
                    _ => Width::Word,
                };
                let addr = a(self, f[1]).wrapping_add(f[2]);
                self.store(bus, insn.op, addr, width, a(self, f[0]))?;
                stored = true;
            }

            Op::BEQZ | Op::BEQZ_N => target = taken(a(self, f[0]) == 0, f[1]),
            Op::BNEZ | Op::BNEZ_N => target = taken(a(self, f[0]) != 0, f[1]),
            Op::BGEZ => target = taken((a(self, f[0]) as i32) >= 0, f[1]),
            Op::BLTZ => target = taken((a(self, f[0]) as i32) < 0, f[1]),
            Op::BEQ => target = taken(a(self, f[0]) == a(self, f[1]), f[2]),
            Op::BNE => target = taken(a(self, f[0]) != a(self, f[1]), f[2]),
            Op::BGE => target = taken(a(self, f[0]) as i32 >= a(self, f[1]) as i32, f[2]),
            Op::BLT => target = taken((a(self, f[0]) as i32) < a(self, f[1]) as i32, f[2]),
            Op::BGEU => target = taken(a(self, f[0]) >= a(self, f[1]), f[2]),
            Op::BLTU => target = taken(a(self, f[0]) < a(self, f[1]), f[2]),
            Op::BANY => target = taken(a(self, f[0]) & a(self, f[1]) != 0, f[2]),
            Op::BNONE => target = taken(a(self, f[0]) & a(self, f[1]) == 0, f[2]),
            Op::BBCI => target = taken(a(self, f[0]) >> f[1] & 1 == 0, f[2]),
            Op::BBSI => target = taken(a(self, f[0]) >> f[1] & 1 != 0, f[2]),
            Op::BEQI => target = taken(a(self, f[0]) == f[1], f[2]),
            Op::BNEI => target = taken(a(self, f[0]) != f[1], f[2]),
            Op::BGEI => target = taken(a(self, f[0]) as i32 >= f[1] as i32, f[2]),
            Op::BLTI => target = taken((a(self, f[0]) as i32) < f[1] as i32, f[2]),
            Op::BGEUI => target = taken(a(self, f[0]) >= f[1], f[2]),
            Op::BLTUI => target = taken(a(self, f[0]) < f[1], f[2]),
            Op::J => target = Some(f[0]),
            Op::JX => target = Some(a(self, f[0])),
            Op::CALL0
            | Op::CALLX0
            | Op::RET
            | Op::RET_N
            | Op::CALL4
            | Op::CALL8
            | Op::CALL12
            | Op::CALLX4
            | Op::CALLX8
            | Op::CALLX12
            | Op::ENTRY
            | Op::RETW
            | Op::RETW_N => target = self.call(insn.op, f, seq)?,
            _ => return self.run(bus, kept).map(drop),
        }
        self.pace &= PACE_LOOP;
        if stored {
            self.pace |= PACE_STORE;
        }
        self.cycle.0 += cycles;
        match target {
            Some(to) => {
                self.cycle.0 += C::redirect_cycles();
                self.refetch = true;
                self.pc = to;
            }
            None => self.pc = seq,
        }
        #[cfg(feature = "poll-skip-check")]
        if let Some(check) = check.take() {
            let Ok(check) = check else {
                panic!("{:?} trapped through run and not here", insn.op);
            };
            assert!(check == *self, "{:?} ran apart at {:#x}", insn.op, self.pc);
        }
        Ok(())
    }
}

impl<C: Config> Core<C> {
    /// A call, an `entry`, or a return, as [`Core::operate`] runs it: the target, if any. Out
    /// of line, so the ops [`Core::run_clear`] takes most stay close together.
    #[inline(never)]
    fn call(&mut self, op: Op, f: &[u32; 8], next: u32) -> Result<Option<u32>, Trap> {
        Ok(Some(match op {
            Op::CALL0 => {
                self.set_a(0, next);
                f[0]
            }
            Op::CALLX0 => {
                let to = self.a(f[0]);
                self.set_a(0, next);
                to
            }
            Op::RET | Op::RET_N => self.a(0),
            Op::CALL4 => self.window_call(1, f[0], next),
            Op::CALL8 => self.window_call(2, f[0], next),
            Op::CALL12 => self.window_call(3, f[0], next),
            Op::CALLX4 => self.window_call(1, self.a(f[0]), next),
            Op::CALLX8 => self.window_call(2, self.a(f[0]), next),
            Op::CALLX12 => self.window_call(3, self.a(f[0]), next),
            Op::ENTRY => {
                self.entry(self.pc, f[0], f[1])?;
                return Ok(None);
            }
            _ => self.retw(self.pc, op)?,
        }))
    }
}

/// Whether `run_clear` runs `op` in line with no load or store, so a check build can run it
/// again on a copy.
#[cfg(feature = "poll-skip-check")]
fn pure(op: Op) -> bool {
    matches!(
        op,
        Op::ADD
            | Op::ADD_N
            | Op::ADDX2
            | Op::ADDX4
            | Op::ADDX8
            | Op::SUB
            | Op::SUBX2
            | Op::SUBX4
            | Op::SUBX8
            | Op::ADDI
            | Op::ADDI_N
            | Op::ADDMI
            | Op::AND
            | Op::OR
            | Op::XOR
            | Op::NEG
            | Op::MOVI
            | Op::MOVI_N
            | Op::MOV_N
            | Op::MOVEQZ
            | Op::MOVNEZ
            | Op::MINU
            | Op::MAXU
            | Op::NSAU
            | Op::EXTUI
            | Op::SLLI
            | Op::SRLI
            | Op::SRAI
            | Op::SLL
            | Op::SRL
            | Op::SRA
            | Op::SRC
            | Op::SSR
            | Op::SSL
            | Op::SSA8L
            | Op::SSAI
            | Op::MULL
            | Op::RSR_CCOUNT
            | Op::MEMW
            | Op::NOP
            | Op::NOP_N
            | Op::BEQZ
            | Op::BEQZ_N
            | Op::BNEZ
            | Op::BNEZ_N
            | Op::BGEZ
            | Op::BLTZ
            | Op::BEQ
            | Op::BNE
            | Op::BGE
            | Op::BLT
            | Op::BGEU
            | Op::BLTU
            | Op::BANY
            | Op::BNONE
            | Op::BBCI
            | Op::BBSI
            | Op::BEQI
            | Op::BNEI
            | Op::BGEI
            | Op::BLTI
            | Op::BGEUI
            | Op::BLTUI
            | Op::J
            | Op::JX
            | Op::CALL0
            | Op::CALLX0
            | Op::RET
            | Op::RET_N
            | Op::CALL4
            | Op::CALL8
            | Op::CALL12
            | Op::CALLX4
            | Op::CALLX8
            | Op::CALLX12
            | Op::ENTRY
            | Op::RETW
            | Op::RETW_N
    )
}
