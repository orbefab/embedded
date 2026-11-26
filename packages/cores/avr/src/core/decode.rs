//! Each flash word decoded once into an [`Op`]. `SPM` only logs, so flash and this table stay as
//! loaded until the next image. The groups are tested in a fixed order, so an encoding that two
//! groups share decodes to the first. `IN` / `OUT` / `SBI` / `CBI` add [`IO_BASE`] to the I/O
//! address in the word.

use super::execute::{Ptr, Update};
use crate::IO_BASE;

/// One instruction with its operands extracted. A target is the absolute word address. `skip` is
/// the length, in words, of the instruction a skip passes over.
///
/// The ops from `Countdown` on are shapes found when the image loads. Each steps as the
/// instruction at its word. They are last, so every other op keeps its tag. `Countdown` is the
/// `BRNE` that closes a [`Countdown`] loop. `SpinCall` is a `CALL` that a `delay` spin repeats,
/// and `spin` indexes the program's spins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Op {
    Nop,
    Add(u8, u8),
    Adc(u8, u8),
    Sub(u8, u8),
    Sbc(u8, u8),
    And(u8, u8),
    Or(u8, u8),
    Eor(u8, u8),
    Mov(u8, u8),
    Cp(u8, u8),
    Cpc(u8, u8),
    Cpse { d: u8, r: u8, skip: u8 },
    Subi(u8, u8),
    Sbci(u8, u8),
    Andi(u8, u8),
    Ori(u8, u8),
    Ldi(u8, u8),
    Cpi(u8, u8),
    Adiw(u8, u8),
    Sbiw(u8, u8),
    Movw(u8, u8),
    Mul(u8, u8),
    Muls(u8, u8),
    Mulsu(u8, u8),
    Fmul(u8, u8),
    Fmuls(u8, u8),
    Fmulsu(u8, u8),
    Ldd { d: u8, ptr: Ptr, q: u8 },
    Std { r: u8, ptr: Ptr, q: u8 },
    In { d: u8, addr: u16 },
    Out { r: u8, addr: u16 },
    Lds { d: u8, addr: u16 },
    Sts { r: u8, addr: u16 },
    Ld { d: u8, ptr: Ptr, update: Update },
    St { r: u8, ptr: Ptr, update: Update },
    Lpm { d: u8, post: bool },
    Pop(u8),
    Push(u8),
    Rjmp(u16),
    Rcall(u16),
    Brbs { s: u8, to: u16 },
    Brbc { s: u8, to: u16 },
    Jmp(u16),
    Call(u16),
    Ijmp,
    Icall,
    Ret,
    Reti,
    Bst { d: u8, b: u8 },
    Bld { d: u8, b: u8 },
    Sbrc { r: u8, b: u8, skip: u8 },
    Sbrs { r: u8, b: u8, skip: u8 },
    Sbi { addr: u16, b: u8 },
    Cbi { addr: u16, b: u8 },
    Sbic { addr: u16, b: u8, skip: u8 },
    Sbis { addr: u16, b: u8, skip: u8 },
    Bset(u8),
    Sei,
    Bclr(u8),
    Com(u8),
    Neg(u8),
    Swap(u8),
    Inc(u8),
    Asr(u8),
    Lsr(u8),
    Ror(u8),
    Dec(u8),
    Spm,
    Sleep,
    Wdr,
    Break,
    Unknown(u16),
    Countdown(Countdown),
    IdleLoop(IdleLoop),
    SpinCall { to: u16, spin: u16 },
}

// The shapes ride in the op without growing the table.
const _: () = assert!(size_of::<Op>() == 6);

/// The op at each word of `flash` a program counter reaches. A word past the image reads as 0,
/// which is `NOP`, so the table stops at the image.
pub(super) fn table(flash: &[u16], pc_mask: u16) -> Vec<Op> {
    let words = flash.len().min(usize::from(pc_mask) + 1);
    let mut ops: Vec<Op> = (0..words)
        .map(|pc| {
            let following = flash.get(pc + 1).copied().unwrap_or(0);
            decode(flash[pc], following, pc as u16, pc_mask)
        })
        .collect();
    for at in 0..ops.len() {
        if let Op::Brbc { s: 1, to } = ops[at]
            && usize::from(to) < at
            && let Some(countdown) = Countdown::parse(&ops[usize::from(to)..at])
        {
            ops[at] = Op::Countdown(countdown);
        }
    }
    for at in 0..ops.len() {
        if let Op::Brbs { to, .. } | Op::Brbc { to, .. } = ops[at]
            && usize::from(to) <= at
            && let Some(idle) = IdleLoop::parse(ops[at], &ops[usize::from(to)..at])
        {
            ops[at] = Op::IdleLoop(idle);
        }
    }
    ops
}

/// A loop of `NOP`s and one counter decrement, closed by a `BRNE` `back` words after its first
/// op. The counter is `bytes` registers from `lo` up, low byte first, and each pass subtracts `k`
/// from it. The decrement is `SBIW`, `DEC`, or `SUBI` and up to three `SBCI` of 0. The loop exits
/// on the pass that leaves the counter zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Countdown {
    pub(super) back: u8,
    /// Cycles of a pass that branches back.
    pub(super) pass: u8,
    pub(super) lo: u8,
    pub(super) bytes: u8,
    pub(super) k: u8,
}

impl Countdown {
    /// `body` is the loop's ops before its `BRNE`.
    fn parse(body: &[Op]) -> Option<Self> {
        let mut chain = body.iter().filter(|op| **op != Op::Nop);
        let first = *chain.next()?;
        let (lo, k, mut bytes) = match first {
            Op::Sbiw(lo, k) => (lo, k, 2),
            Op::Dec(d) => (d, 1, 1),
            Op::Subi(d, k) => (d, k, 1),
            _ => return None,
        };
        for op in chain {
            if !matches!(first, Op::Subi(..)) || bytes == 4 || *op != Op::Sbci(lo + bytes, 0) {
                return None;
            }
            bytes += 1;
        }
        // A backward `BRNE` reaches at most 63 words, so `back` fits and `pass` is at most 66.
        let back = body.len() as u8;
        (k != 0).then_some(Self {
            back,
            pass: back + 2 + u8::from(matches!(first, Op::Sbiw(..))),
            lo,
            bytes,
            k,
        })
    }

    /// The loop's first op, with the `BRNE` at `at`.
    pub(super) fn head(self, at: u16) -> u16 {
        at - u16::from(self.back)
    }
}

/// A loop whose body writes no register, closed by a `BRBS` or `BRBC` `back` words after its
/// first op. The body is `NOP`s, compares, `AND` and `OR` of a register with itself, and `SBIW`
/// and `ADIW` of 0, with no branch or skip, so a pass changes only `SREG` and the cycle count.
/// Arduino's `main` with an empty `loop()` is one: `SBIW r28, 0` and a `BREQ` back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IdleLoop {
    pub(super) back: u8,
    /// Cycles of a pass that branches back.
    pub(super) pass: u8,
    /// The `SREG` bit the branch tests, and the value that branches back.
    pub(super) s: u8,
    pub(super) set: bool,
}

impl IdleLoop {
    /// `body` is the loop's ops before `branch`.
    fn parse(branch: Op, body: &[Op]) -> Option<Self> {
        let (s, set) = match branch {
            Op::Brbs { s, .. } => (s, true),
            Op::Brbc { s, .. } => (s, false),
            _ => return None,
        };
        // A backward branch reaches at most 64 words, and a body op takes at most 2 cycles.
        let mut pass = 2u8;
        for op in body {
            pass += match *op {
                Op::Nop | Op::Cp(..) | Op::Cpc(..) | Op::Cpi(..) => 1,
                Op::And(d, r) | Op::Or(d, r) if d == r => 1,
                Op::Sbiw(_, 0) | Op::Adiw(_, 0) => 2,
                _ => return None,
            };
        }
        Some(Self {
            back: body.len() as u8,
            pass,
            s,
            set,
        })
    }

    /// The loop's first op, with the branch at `at`.
    pub(super) fn head(self, at: u16) -> u16 {
        at - u16::from(self.back)
    }
}

/// `word` at `pc`, with `following` the word after it, or 0 past the image.
pub(super) fn decode(word: u16, following: u16, pc: u16, pc_mask: u16) -> Op {
    let next = pc.wrapping_add(1);
    let skip = group(following, 0, next.wrapping_add(1), pc_mask, 1).words();
    group(word, following, next, pc_mask, skip)
}

/// The decode with the skip length given, so a skip can decode the word it passes over.
fn group(word: u16, following: u16, next: u16, pc_mask: u16, skip: u8) -> Op {
    if word == 0x0000 {
        return Op::Nop;
    }
    alu_reg(word, skip)
        .or_else(|| alu_imm(word))
        .or_else(|| mul(word))
        .or_else(|| transfer(word, following))
        .or_else(|| branch(word, following, next, pc_mask))
        .or_else(|| bitop(word, skip))
        .unwrap_or(Op::Unknown(word))
}

/// The ops that read and write only registers, `SREG` other than setting `I`, `PC` and the cycle
/// count. They reach neither the bus nor the stack, and they do not stop or sleep the core.
macro_rules! free_op {
    () => {
        Op::Nop
            | Op::Add(..)
            | Op::Adc(..)
            | Op::Sub(..)
            | Op::Sbc(..)
            | Op::And(..)
            | Op::Or(..)
            | Op::Eor(..)
            | Op::Mov(..)
            | Op::Cp(..)
            | Op::Cpc(..)
            | Op::Cpse { .. }
            | Op::Subi(..)
            | Op::Sbci(..)
            | Op::Andi(..)
            | Op::Ori(..)
            | Op::Ldi(..)
            | Op::Cpi(..)
            | Op::Adiw(..)
            | Op::Sbiw(..)
            | Op::Movw(..)
            | Op::Mul(..)
            | Op::Muls(..)
            | Op::Mulsu(..)
            | Op::Fmul(..)
            | Op::Fmuls(..)
            | Op::Fmulsu(..)
            | Op::Lpm { .. }
            | Op::Rjmp(_)
            | Op::Brbs { .. }
            | Op::Brbc { .. }
            | Op::Countdown(_)
            | Op::IdleLoop(_)
            | Op::Jmp(_)
            | Op::Ijmp
            | Op::Bst { .. }
            | Op::Bld { .. }
            | Op::Sbrc { .. }
            | Op::Sbrs { .. }
            | Op::Bset(_)
            | Op::Bclr(_)
            | Op::Com(_)
            | Op::Neg(_)
            | Op::Swap(_)
            | Op::Inc(_)
            | Op::Asr(_)
            | Op::Lsr(_)
            | Op::Ror(_)
            | Op::Dec(_)
            | Op::Unknown(_)
    };
}

/// Every op that is not [`free_op`].
macro_rules! bus_op {
    () => {
        Op::Ldd { .. }
            | Op::Std { .. }
            | Op::In { .. }
            | Op::Out { .. }
            | Op::Lds { .. }
            | Op::Sts { .. }
            | Op::Ld { .. }
            | Op::St { .. }
            | Op::Pop(_)
            | Op::Push(_)
            | Op::Rcall(_)
            | Op::Call(_)
            | Op::SpinCall { .. }
            | Op::Icall
            | Op::Ret
            | Op::Reti
            | Op::Sbi { .. }
            | Op::Cbi { .. }
            | Op::Sbic { .. }
            | Op::Sbis { .. }
            | Op::Sei
            | Op::Spm
            | Op::Sleep
            | Op::Wdr
            | Op::Break
    };
}

pub(super) use {bus_op, free_op};

impl Op {
    /// The op is a [`free_op`].
    #[inline(always)]
    pub(super) fn is_free(self) -> bool {
        match self {
            free_op!() => true,
            bus_op!() => false,
        }
    }

    /// [`Core::execute_steady`](super::Core::execute_steady) may run the op.
    #[inline(always)]
    pub(super) fn may_be_steady(self) -> bool {
        matches!(
            self,
            Op::In { .. }
                | Op::Lds { .. }
                | Op::Sbic { .. }
                | Op::Sbis { .. }
                | Op::Out { .. }
                | Op::Sts { .. }
        )
    }

    /// The instruction's length in flash words.
    fn words(self) -> u8 {
        match self {
            Op::Lds { .. } | Op::Sts { .. } | Op::Jmp(_) | Op::Call(_) | Op::SpinCall { .. } => 2,
            _ => 1,
        }
    }
}

fn rd_rr(word: u16) -> (u8, u8) {
    let d = ((word >> 4) & 0x1F) as u8;
    let r = (((word >> 5) & 0x10) | (word & 0x0F)) as u8;
    (d, r)
}

fn rd5(word: u16) -> u8 {
    ((word >> 4) & 0x1F) as u8
}

fn alu_reg(word: u16, skip: u8) -> Option<Op> {
    let (d, r) = rd_rr(word);
    Some(match word & 0xFC00 {
        0x0C00 => Op::Add(d, r),
        0x1C00 => Op::Adc(d, r),
        0x1800 => Op::Sub(d, r),
        0x0800 => Op::Sbc(d, r),
        0x2000 => Op::And(d, r),
        0x2800 => Op::Or(d, r),
        0x2400 => Op::Eor(d, r),
        0x2C00 => Op::Mov(d, r),
        0x1400 => Op::Cp(d, r),
        0x0400 => Op::Cpc(d, r),
        0x1000 => Op::Cpse { d, r, skip },
        _ => return None,
    })
}

fn alu_imm(word: u16) -> Option<Op> {
    let d = 16 + ((word >> 4) & 0x0F) as u8;
    let k = (((word >> 4) & 0xF0) | (word & 0x0F)) as u8;
    match word & 0xF000 {
        0x5000 => return Some(Op::Subi(d, k)),
        0x4000 => return Some(Op::Sbci(d, k)),
        0x7000 => return Some(Op::Andi(d, k)),
        0x6000 => return Some(Op::Ori(d, k)),
        0xE000 => return Some(Op::Ldi(d, k)),
        0x3000 => return Some(Op::Cpi(d, k)),
        _ => {}
    }
    let lo = 24 + ((word >> 4) & 0x3) as u8 * 2;
    let k = (((word >> 2) & 0x30) | (word & 0x0F)) as u8;
    match word & 0xFF00 {
        0x9600 => Some(Op::Adiw(lo, k)),
        0x9700 => Some(Op::Sbiw(lo, k)),
        0x0100 => Some(Op::Movw(
            (((word >> 4) & 0xF) as u8) * 2,
            ((word & 0xF) as u8) * 2,
        )),
        _ => None,
    }
}

fn mul(word: u16) -> Option<Op> {
    if (word & 0xFC00) == 0x9C00 {
        let (d, r) = rd_rr(word);
        return Some(Op::Mul(d, r));
    }
    if (word & 0xFF00) == 0x0200 {
        let d = 16 + ((word >> 4) & 0xF) as u8;
        let r = 16 + (word & 0xF) as u8;
        return Some(Op::Muls(d, r));
    }
    if (word & 0xFF00) == 0x0300 {
        let d = 16 + ((word >> 4) & 0x7) as u8;
        let r = 16 + (word & 0x7) as u8;
        return Some(match ((word & 0x0080) != 0, (word & 0x0008) != 0) {
            (false, false) => Op::Mulsu(d, r),
            (false, true) => Op::Fmul(d, r),
            (true, false) => Op::Fmuls(d, r),
            (true, true) => Op::Fmulsu(d, r),
        });
    }
    None
}

fn transfer(word: u16, following: u16) -> Option<Op> {
    let y = (word & 0xD208) == 0x8008 || (word & 0xD208) == 0x8208;
    let z = (word & 0xD208) == 0x8000 || (word & 0xD208) == 0x8200;
    let reg = rd5(word);
    if y || z {
        // DS40002198 LDD/STD: q is bit 13, bits 11:10, and bits 2:0.
        let q = (((word & 0x2000) >> 8) | ((word & 0x0C00) >> 7) | (word & 0x0007)) as u8;
        let ptr = if y { Ptr::Y } else { Ptr::Z };
        return Some(if (word & 0x0200) == 0 {
            Op::Ldd { d: reg, ptr, q }
        } else {
            Op::Std { r: reg, ptr, q }
        });
    }
    if (word & 0xFC00) != 0x9000 {
        let addr = (((word >> 5) & 0x30) | (word & 0x0F)).wrapping_add(IO_BASE);
        return match word & 0xF800 {
            0xB000 => Some(Op::In { d: reg, addr }),
            0xB800 => Some(Op::Out { r: reg, addr }),
            _ => None,
        };
    }
    let ld = |ptr, update| {
        Some(Op::Ld {
            d: reg,
            ptr,
            update,
        })
    };
    let st = |ptr, update| {
        Some(Op::St {
            r: reg,
            ptr,
            update,
        })
    };
    match (word & 0x000F, (word & 0x0200) != 0) {
        (0x0, false) => Some(Op::Lds {
            d: reg,
            addr: following,
        }),
        (0x0, true) => Some(Op::Sts {
            r: reg,
            addr: following,
        }),
        (0x1, false) => ld(Ptr::Z, Update::Post),
        (0x1, true) => st(Ptr::Z, Update::Post),
        (0x2, false) => ld(Ptr::Z, Update::Pre),
        (0x2, true) => st(Ptr::Z, Update::Pre),
        (0x4, false) => Some(Op::Lpm {
            d: reg,
            post: false,
        }),
        (0x5, false) => Some(Op::Lpm { d: reg, post: true }),
        (0x9, false) => ld(Ptr::Y, Update::Post),
        (0x9, true) => st(Ptr::Y, Update::Post),
        (0xA, false) => ld(Ptr::Y, Update::Pre),
        (0xA, true) => st(Ptr::Y, Update::Pre),
        (0xC, false) => ld(Ptr::X, Update::None),
        (0xC, true) => st(Ptr::X, Update::None),
        (0xD, false) => ld(Ptr::X, Update::Post),
        (0xD, true) => st(Ptr::X, Update::Post),
        (0xE, false) => ld(Ptr::X, Update::Pre),
        (0xE, true) => st(Ptr::X, Update::Pre),
        (0xF, false) => Some(Op::Pop(reg)),
        (0xF, true) => Some(Op::Push(reg)),
        _ => None,
    }
}

fn branch(word: u16, following: u16, next: u16, pc_mask: u16) -> Option<Op> {
    let top = word & 0xF000;
    if top == 0xC000 || top == 0xD000 {
        let k = word & 0x0FFF;
        let k = if k & 0x0800 == 0 { k } else { k | 0xF000 };
        let to = next.wrapping_add(k);
        return Some(if top == 0xC000 {
            Op::Rjmp(to)
        } else {
            Op::Rcall(to)
        });
    }
    if (word & 0xFC00) == 0xF000 || (word & 0xFC00) == 0xF400 {
        let s = (word & 0x7) as u8;
        let k = (word >> 3) & 0x7F;
        let k = if k & 0x40 == 0 { k } else { k | 0xFF80 };
        let to = next.wrapping_add(k);
        return Some(if (word & 0x0400) == 0 {
            Op::Brbs { s, to }
        } else {
            Op::Brbc { s, to }
        });
    }
    // Bits 21:16 of a `JMP` / `CALL` sit in the first word. This part has a 14-bit PC.
    match word & 0xFE0E {
        0x940C => return Some(Op::Jmp(following & pc_mask)),
        0x940E => return Some(Op::Call(following & pc_mask)),
        _ => {}
    }
    match word {
        0x9409 => Some(Op::Ijmp),
        0x9509 => Some(Op::Icall),
        0x9508 => Some(Op::Ret),
        0x9518 => Some(Op::Reti),
        _ => None,
    }
}

fn bitop(word: u16, skip: u8) -> Option<Op> {
    let reg = rd5(word);
    let b = (word & 0x7) as u8;
    match word & 0xFE08 {
        0xFA00 => return Some(Op::Bst { d: reg, b }),
        0xF800 => return Some(Op::Bld { d: reg, b }),
        0xFC00 => return Some(Op::Sbrc { r: reg, b, skip }),
        0xFE00 => return Some(Op::Sbrs { r: reg, b, skip }),
        _ => {}
    }
    let addr = ((word >> 3) & 0x1F) + IO_BASE;
    match word & 0xFF00 {
        0x9A00 => return Some(Op::Sbi { addr, b }),
        0x9800 => return Some(Op::Cbi { addr, b }),
        0x9900 => return Some(Op::Sbic { addr, b, skip }),
        0x9B00 => return Some(Op::Sbis { addr, b, skip }),
        _ => {}
    }
    if (word & 0xFF0F) == 0x9408 {
        let s = ((word >> 4) & 0x7) as u8;
        return Some(if (word & 0x0080) != 0 {
            Op::Bclr(s)
        } else if s == 7 {
            Op::Sei
        } else {
            Op::Bset(s)
        });
    }
    if (word & 0xFE00) != 0x9400 {
        return None;
    }
    match word & 0x000F {
        0x0 => Some(Op::Com(reg)),
        0x1 => Some(Op::Neg(reg)),
        0x2 => Some(Op::Swap(reg)),
        0x3 => Some(Op::Inc(reg)),
        0x5 => Some(Op::Asr(reg)),
        0x6 => Some(Op::Lsr(reg)),
        0x7 => Some(Op::Ror(reg)),
        0xA => Some(Op::Dec(reg)),
        _ => match word {
            0x95C8 => Some(Op::Lpm { d: 0, post: false }),
            0x95E8 => Some(Op::Spm),
            0x9588 => Some(Op::Sleep),
            0x95A8 => Some(Op::Wdr),
            0x9598 => Some(Op::Break),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::check::{BusCall, FNV_OFFSET, INITIAL_SP, MASK, Mem, Rig};
    use super::super::execute::{Ptr, Update};
    use super::{Op, decode, table};
    use crate::{Bus, Core};

    /// Two little-endian bytes for each word, folded from every run of it in [`Rig::cases`]
    /// on the flat [`Mem`] bus. `record_the_effects` writes them.
    const RECORDED: &[u8] = include_bytes!("effects.bin");

    fn fingerprint(rig: &mut Rig, swept: u16) -> u16 {
        let mut hash = FNV_OFFSET;
        for case in rig.cases(swept) {
            for effect in rig.effects(&case, |core, bus| core.execute_op(bus, case.pc, case.op)) {
                effect.digest(&mut hash);
            }
        }
        (hash ^ (hash >> 16) ^ (hash >> 32) ^ (hash >> 48)) as u16
    }

    #[test]
    fn every_word_runs_as_recorded() {
        let mut rig = Rig::new();
        assert_eq!(RECORDED.len(), 2 << 16);
        for (swept, recorded) in (0..=u16::MAX).zip(RECORDED.chunks(2)) {
            if fingerprint(&mut rig, swept).to_le_bytes() != recorded {
                let cases: Vec<String> = rig.cases(swept).iter().map(|c| c.to_string()).collect();
                panic!("word {swept:#06x} runs unlike the recording in one of {cases:#?}");
            }
        }
    }

    #[test]
    #[ignore = "writes effects.bin"]
    fn record_the_effects() {
        let mut rig = Rig::new();
        let bytes: Vec<u8> = (0..=u16::MAX)
            .flat_map(|swept| fingerprint(&mut rig, swept).to_le_bytes())
            .collect();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/core/effects.bin");
        let old = std::fs::read(path).unwrap_or_default();
        let moved: Vec<usize> = (0..=usize::from(u16::MAX))
            .filter(|&word| old.get(2 * word..2 * word + 2) != Some(&bytes[2 * word..2 * word + 2]))
            .collect();
        for word in moved.iter().take(64) {
            println!("{word:#06x} moved");
        }
        if moved.is_empty() {
            println!("no word moved");
        } else {
            println!("{} words moved", moved.len());
        }
        std::fs::write(path, bytes).expect("effects.bin is writable");
    }

    #[test]
    fn every_second_word_is_the_address_or_the_target() {
        for word in 0..=u16::MAX {
            for second in [
                0x0000, 0x0001, 0x00C6, 0x08FF, 0x3FFF, 0x4000, 0xC0DE, 0xFFFF,
            ] {
                match decode(word, second, 0, MASK) {
                    Op::Lds { addr, .. } | Op::Sts { addr, .. } => assert_eq!(addr, second),
                    Op::Jmp(to) | Op::Call(to) => assert_eq!(to, second & MASK),
                    _ => {}
                }
            }
        }
        let wide = (0..=u16::MAX)
            .filter(|&word| {
                matches!(
                    decode(word, 0x1234, 0, MASK),
                    Op::Lds { .. } | Op::Sts { .. } | Op::Jmp(_) | Op::Call(_)
                )
            })
            .count();
        assert_eq!(wide, 192);
    }

    /// One word per form, with operands that differ from each other, against DS40002198.
    #[test]
    fn each_form_decodes_its_operands() {
        let at = |word, following, pc| decode(word, following, pc, MASK);
        let cases = [
            (0x0000, 0x0000, 0x0010, Op::Nop),
            (0x0C12, 0x0000, 0x0010, Op::Add(1, 2)),
            (0x1E3A, 0x0000, 0x0010, Op::Adc(3, 26)),
            (0x1A45, 0x0000, 0x0010, Op::Sub(4, 21)),
            (0x0856, 0x0000, 0x0010, Op::Sbc(5, 6)),
            (0x2267, 0x0000, 0x0010, Op::And(6, 23)),
            (0x2878, 0x0000, 0x0010, Op::Or(7, 8)),
            (0x2689, 0x0000, 0x0010, Op::Eor(8, 25)),
            (0x2C9A, 0x0000, 0x0010, Op::Mov(9, 10)),
            (0x16AB, 0x0000, 0x0010, Op::Cp(10, 27)),
            (0x04BC, 0x0000, 0x0010, Op::Cpc(11, 12)),
            (
                0x12CD,
                0x0000,
                0x0010,
                Op::Cpse {
                    d: 12,
                    r: 29,
                    skip: 1,
                },
            ),
            (
                0x12CD,
                0x9310,
                0x0010,
                Op::Cpse {
                    d: 12,
                    r: 29,
                    skip: 2,
                },
            ),
            (0x5A1B, 0x0000, 0x0010, Op::Subi(17, 0xAB)),
            (0x4C2D, 0x0000, 0x0010, Op::Sbci(18, 0xCD)),
            (0x7E3F, 0x0000, 0x0010, Op::Andi(19, 0xEF)),
            (0x6143, 0x0000, 0x0010, Op::Ori(20, 0x13)),
            (0xE256, 0x0000, 0x0010, Op::Ldi(21, 0x26)),
            (0x3369, 0x0000, 0x0010, Op::Cpi(22, 0x39)),
            (0x96D5, 0x0000, 0x0010, Op::Adiw(26, 0x35)),
            (0x97E7, 0x0000, 0x0010, Op::Sbiw(28, 0x37)),
            (0x01A3, 0x0000, 0x0010, Op::Movw(20, 6)),
            (0x9E12, 0x0000, 0x0010, Op::Mul(1, 18)),
            (0x02A5, 0x0000, 0x0010, Op::Muls(26, 21)),
            (0x0325, 0x0000, 0x0010, Op::Mulsu(18, 21)),
            (0x032D, 0x0000, 0x0010, Op::Fmul(18, 21)),
            (0x03A5, 0x0000, 0x0010, Op::Fmuls(18, 21)),
            (0x03AD, 0x0000, 0x0010, Op::Fmulsu(18, 21)),
            (
                0xA9FB,
                0x0000,
                0x0010,
                Op::Ldd {
                    d: 31,
                    ptr: Ptr::Y,
                    q: 0x33,
                },
            ),
            (
                0x8C64,
                0x0000,
                0x0010,
                Op::Ldd {
                    d: 6,
                    ptr: Ptr::Z,
                    q: 0x1C,
                },
            ),
            (
                0xAE7D,
                0x0000,
                0x0010,
                Op::Std {
                    r: 7,
                    ptr: Ptr::Y,
                    q: 0x3D,
                },
            ),
            (
                0x8281,
                0x0000,
                0x0010,
                Op::Std {
                    r: 8,
                    ptr: Ptr::Z,
                    q: 0x01,
                },
            ),
            (0xB59A, 0x0000, 0x0010, Op::In { d: 25, addr: 0x4A }),
            (0xBE4F, 0x0000, 0x0010, Op::Out { r: 4, addr: 0x5F }),
            (
                0x9110,
                0x08AB,
                0x0010,
                Op::Lds {
                    d: 17,
                    addr: 0x08AB,
                },
            ),
            (
                0x9320,
                0x00C6,
                0x0010,
                Op::Sts {
                    r: 18,
                    addr: 0x00C6,
                },
            ),
            (
                0x9131,
                0x0000,
                0x0010,
                Op::Ld {
                    d: 19,
                    ptr: Ptr::Z,
                    update: Update::Post,
                },
            ),
            (
                0x9342,
                0x0000,
                0x0010,
                Op::St {
                    r: 20,
                    ptr: Ptr::Z,
                    update: Update::Pre,
                },
            ),
            (
                0x9159,
                0x0000,
                0x0010,
                Op::Ld {
                    d: 21,
                    ptr: Ptr::Y,
                    update: Update::Post,
                },
            ),
            (
                0x936A,
                0x0000,
                0x0010,
                Op::St {
                    r: 22,
                    ptr: Ptr::Y,
                    update: Update::Pre,
                },
            ),
            (
                0x917C,
                0x0000,
                0x0010,
                Op::Ld {
                    d: 23,
                    ptr: Ptr::X,
                    update: Update::None,
                },
            ),
            (
                0x938D,
                0x0000,
                0x0010,
                Op::St {
                    r: 24,
                    ptr: Ptr::X,
                    update: Update::Post,
                },
            ),
            (
                0x919E,
                0x0000,
                0x0010,
                Op::Ld {
                    d: 25,
                    ptr: Ptr::X,
                    update: Update::Pre,
                },
            ),
            (
                0x8010,
                0x0000,
                0x0010,
                Op::Ldd {
                    d: 1,
                    ptr: Ptr::Z,
                    q: 0,
                },
            ),
            (0x9104, 0x0000, 0x0010, Op::Lpm { d: 16, post: false }),
            (0x9115, 0x0000, 0x0010, Op::Lpm { d: 17, post: true }),
            (0x95C8, 0x0000, 0x0010, Op::Lpm { d: 0, post: false }),
            (0x91AF, 0x0000, 0x0010, Op::Pop(26)),
            (0x93BF, 0x0000, 0x0010, Op::Push(27)),
            (0xC005, 0x0000, 0x0010, Op::Rjmp(0x0016)),
            (0xCFFF, 0x0000, 0x0010, Op::Rjmp(0x0010)),
            (0xDFF0, 0x0000, 0x0010, Op::Rcall(0x0001)),
            (0xF01B, 0x0000, 0x0010, Op::Brbs { s: 3, to: 0x0014 }),
            (0xF7F9, 0x0000, 0x0010, Op::Brbc { s: 1, to: 0x0010 }),
            (0x940C, 0x1234, 0x0010, Op::Jmp(0x1234)),
            (0x95FF, 0xC321, 0x0010, Op::Call(0x0321)),
            (0x9409, 0x0000, 0x0010, Op::Ijmp),
            (0x9509, 0x0000, 0x0010, Op::Icall),
            (0x9508, 0x0000, 0x0010, Op::Ret),
            (0x9518, 0x0000, 0x0010, Op::Reti),
            (0xFBC5, 0x0000, 0x0010, Op::Bst { d: 28, b: 5 }),
            (0xF9D6, 0x0000, 0x0010, Op::Bld { d: 29, b: 6 }),
            (
                0xFDE7,
                0x940C,
                0x0010,
                Op::Sbrc {
                    r: 30,
                    b: 7,
                    skip: 2,
                },
            ),
            (
                0xFFF1,
                0x0000,
                0x0010,
                Op::Sbrs {
                    r: 31,
                    b: 1,
                    skip: 1,
                },
            ),
            (0x9A2A, 0x0000, 0x0010, Op::Sbi { addr: 0x25, b: 2 }),
            (0x983B, 0x0000, 0x0010, Op::Cbi { addr: 0x27, b: 3 }),
            (
                0x99FC,
                0x9200,
                0x0010,
                Op::Sbic {
                    addr: 0x3F,
                    b: 4,
                    skip: 2,
                },
            ),
            (
                0x9B05,
                0x0000,
                0x0010,
                Op::Sbis {
                    addr: 0x20,
                    b: 5,
                    skip: 1,
                },
            ),
            (0x9468, 0x0000, 0x0010, Op::Bset(6)),
            (0x9478, 0x0000, 0x0010, Op::Sei),
            (0x94A8, 0x0000, 0x0010, Op::Bclr(2)),
            (0x9410, 0x0000, 0x0010, Op::Com(1)),
            (0x9421, 0x0000, 0x0010, Op::Neg(2)),
            (0x9432, 0x0000, 0x0010, Op::Swap(3)),
            (0x9443, 0x0000, 0x0010, Op::Inc(4)),
            (0x9455, 0x0000, 0x0010, Op::Asr(5)),
            (0x9466, 0x0000, 0x0010, Op::Lsr(6)),
            (0x9477, 0x0000, 0x0010, Op::Ror(7)),
            (0x948A, 0x0000, 0x0010, Op::Dec(8)),
            (0x95E8, 0x0000, 0x0010, Op::Spm),
            (0x9588, 0x0000, 0x0010, Op::Sleep),
            (0x95A8, 0x0000, 0x0010, Op::Wdr),
            (0x9598, 0x0000, 0x0010, Op::Break),
            (0x9404, 0x0000, 0x0010, Op::Unknown(0x9404)),
            (0xFFFF, 0x0000, 0x0010, Op::Unknown(0xFFFF)),
        ];
        for (word, following, pc, op) in cases {
            assert_eq!(
                at(word, following, pc),
                op,
                "word {word:#06x} before {following:#06x}"
            );
        }
    }

    #[test]
    fn the_table_stops_at_the_image_and_reads_zero_past_it() {
        let image = [0x9110, 0x12CD, 0x940C];
        assert_eq!(
            table(&image, MASK),
            [
                Op::Lds {
                    d: 17,
                    addr: 0x12CD
                },
                Op::Cpse {
                    d: 12,
                    r: 29,
                    skip: 2
                },
                Op::Jmp(0x0000),
            ]
        );
        let oversized = vec![0x0000; usize::from(MASK) + 9];
        assert_eq!(table(&oversized, MASK).len(), usize::from(MASK) + 1);
        let mut core = Core::new(INITIAL_SP, MASK);
        core.load_words(&[0x0000]);
        core.pc = 5;
        let mut bus = Mem::new();
        core.step(&mut bus);
        assert_eq!((core.pc, core.cycles), (6, 1));
    }

    /// The closing `BRNE` of a countdown with room before the bound jumps to the last pass that
    /// leaves the counter above zero. `DEC` covers every register. `SUBI` covers r16 and up,
    /// alone and followed by one, two or three `SBCI` of 0, and one `SUBI` subtracts 3. `SBIW`
    /// covers the four pairs from r24, once with the high byte clear and once with it set. A
    /// loop run pass by pass still holds its counter here.
    #[test]
    fn a_countdown_jumps_on_every_register() {
        let dec = (0..32u16).map(|d| vec![0x940A | d << 4, brne(1), 0xCFFF]);
        let one = (16..32u16).map(|d| vec![subi(d, 1), brne(1), 0xCFFF]);
        let sbiw = (0..4u16).map(|pair| vec![0x9701 | pair << 4, brne(1), 0xCFFF]);
        for words in dec.chain(one).chain(sbiw) {
            jumped(&words, 200, 1);
        }
        jumped(&[subi(24, 3), brne(1), 0xCFFF], 200, 3);
        for pair in 0..4u16 {
            jumped(&[0x9701 | pair << 4, brne(1), 0xCFFF], 0x01C8, 1);
        }
        for extra in 1..=3u16 {
            for d in 16..=(31 - extra) {
                let mut words = vec![subi(d, 1)];
                for n in 1..=extra {
                    words.push(sbci(d + n, 0));
                }
                words.push(brne(words.len() as u16));
                words.push(0xCFFF);
                jumped(&words, 200, 1);
            }
        }
    }

    /// `SUBI` or `SBCI` of `k` into r`d`.
    fn subi(d: u16, k: u16) -> u16 {
        imm(0x5000, d, k)
    }

    fn sbci(d: u16, k: u16) -> u16 {
        imm(0x4000, d, k)
    }

    fn imm(op: u16, d: u16, k: u16) -> u16 {
        op | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)
    }

    /// `BRNE` back over `back` words.
    fn brne(back: u16) -> u16 {
        let k = (-1 - back as i16) as u16 & 0x7F;
        0xF401 | (k << 3)
    }

    /// Starts at the `BRNE` with `counter` already in the loop's registers. The jump leaves the
    /// last pass that keeps the counter above zero still to run.
    fn jumped(words: &[u16], counter: u64, k: u64) {
        let mut core = Core::new(INITIAL_SP, MASK);
        core.load_words(words);
        let at = u16::try_from(words.len() - 2).unwrap();
        let countdown = match core.op(at) {
            Op::Countdown(countdown) => countdown,
            other => panic!("word at {at} is {other:?}, not a countdown"),
        };
        assert_eq!(u64::from(countdown.k), k, "{words:?}");
        let mut rest = counter;
        for offset in 0..countdown.bytes {
            core.set_reg(countdown.lo + offset, rest as u8);
            rest >>= 8;
        }
        core.pc = at;
        core.execute_in_run(at, core.op(at), u64::MAX);
        let passes = (counter - 1) / k;
        let skip = passes - 1;
        let mut left = counter - skip * k;
        let back = words.len() - 2;
        let wide = matches!(core.op(0), Op::Sbiw(..));
        let pass = back as u64 + 2 + u64::from(wide);
        assert_eq!(core.pc, countdown.head(at), "{words:?}");
        for offset in 0..countdown.bytes {
            assert_eq!(
                core.reg(countdown.lo + offset),
                left as u8,
                "{words:?} byte {offset}"
            );
            left >>= 8;
        }
        assert_eq!(core.cycles, 2 + skip * pass, "{words:?}");
    }

    #[test]
    fn spm_leaves_the_table_on_the_loaded_image() {
        let image = [0x95E8, 0xE50A];
        let mut core = Core::new(INITIAL_SP, MASK);
        core.load_words(&image);
        let (flash, ops) = (Arc::clone(&core.flash), Arc::clone(&core.ops));
        core.set_reg(30, 2);
        core.set_reg(0, 0x34);
        core.set_reg(1, 0x12);
        let mut bus = Mem::new();
        core.step(&mut bus);
        assert_eq!(core.spm_notes().len(), 1);
        assert!(Arc::ptr_eq(&core.flash, &flash) && Arc::ptr_eq(&core.ops, &ops));
        assert_eq!(&core.flash[..], &image);
        assert_eq!(&core.ops[..], table(&image, MASK));
        core.step(&mut bus);
        assert_eq!(core.reg(16), 0x5A);
    }

    /// A stall the bus reports is part of the instruction. `WDR` tells the bus, and `SLEEP`
    /// arms it only when the bus says sleep is enabled.
    #[test]
    fn the_bus_hears_halt_sleep_and_watchdog() {
        let mut core = Core::new(INITIAL_SP, MASK);
        core.load_words(&[0x0000]);
        let mut bus = Mem::new();
        bus.set_halt(4);
        core.step(&mut bus);
        assert_eq!(core.cycles(), 5);
        assert_eq!(bus.cpu_halt(), 0);

        core.load_words(&[0x95A8]);
        core.pc = 0;
        let mut bus = Mem::new();
        core.step(&mut bus);
        assert_eq!((core.wdr_executions(), bus.wdrs), (1, 1));

        core.load_words(&[0x9588]);
        core.pc = 0;
        let mut asleep = Mem::new();
        asleep.set_sleep_enabled(true);
        core.step(&mut asleep);
        assert!(asleep.sleep_armed());
        assert!(core.is_sleeping());

        let mut awake = Core::new(INITIAL_SP, MASK);
        awake.load_words(&[0x9588]);
        let mut open = Mem::new();
        awake.step(&mut open);
        assert!(!open.sleep_armed());
        assert!(!awake.is_sleeping());
    }

    /// `SBI` and `CBI` are one `write_bit`. A load or a store would hide write-one-to-clear.
    #[test]
    fn sbi_and_cbi_use_write_bit() {
        let mut set = Core::new(INITIAL_SP, MASK);
        set.load_words(&[0x9A2A]);
        let mut bus = Mem::new();
        bus.poke(&mut set, 0x25, 0x01);
        bus.record_touches();
        set.step(&mut bus);
        assert_eq!(bus.peek(&set, 0x25), 0x05);
        assert_eq!(
            bus.touches(),
            &[BusCall::WriteBit {
                addr: 0x25,
                bit: 2,
                set: true,
            }]
        );

        let mut clear = Core::new(INITIAL_SP, MASK);
        clear.load_words(&[0x983B]);
        let mut bus = Mem::new();
        bus.poke(&mut clear, 0x27, 0x08);
        bus.record_touches();
        clear.step(&mut bus);
        assert_eq!(bus.peek(&clear, 0x27), 0x00);
        assert_eq!(
            bus.touches(),
            &[BusCall::WriteBit {
                addr: 0x27,
                bit: 3,
                set: false,
            }]
        );
    }
}
