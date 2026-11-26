//! A spin on `CCOUNT`: a few register ops that read `CCOUNT`, then a branch that compares and
//! goes back to the first of them, such as `rsr.ccount; sub; bltu` waiting for a number of
//! cycles to pass.
//!
//! No op of the loop writes a register the loop reads before writing it, so each pass computes
//! the same thing from the count it reads. The branch compares a value that moves by the same
//! amount each pass with one that stays, so the passes that still branch back can be counted
//! without running them. Within a straight run the core skips the passes that branch back and
//! start every op before the run's horizon, and leaves the registers, the cycle and the PC as the
//! last of those passes leaves them. The pass that falls out of the loop, and any pass that would
//! reach the horizon, runs op by op.
//!
//! Only register ops whose results this module computes take part, each of which a straight run
//! runs in line in one cycle. Anything else leaves the loop to run op by op: a load or a store,
//! another branch, an op that reads a register a later op writes, `SAR`, or a loop end inside
//! it. A check build runs the passes it would skip and requires the core to match what it
//! computed.

use super::decodes::Taken;
use super::{Core, PS_EXCM};
use crate::bus::{Bus, Cycle};
use crate::config::Config;
use crate::decode::{Insn, Op};
use crate::special::Sr;

/// The most ops a spin holds, its branch included.
const MOST_OPS: usize = 8;
/// The furthest back a spin's branch goes.
const MOST_BYTES: u32 = 32;
/// The fewest cycles before a straight run's horizon at which the run looks for a spin: room
/// for a few of the longest passes. With less, a skip saves less than the look costs.
const ROOM: u64 = 64;

const SIGN: u32 = 1 << 31;

/// Whether `insn` at `pc` is a branch a spin can end with.
pub(super) fn closes(insn: &Insn, pc: u32) -> bool {
    test(insn).is_some_and(|test| test.target < pc && pc - test.target <= MOST_BYTES)
}

/// A spin's branch: back to `target` while `a < b`, or while `a >= b` when `less` is clear,
/// signed or unsigned.
#[derive(Clone, Copy)]
struct Test {
    a: u32,
    b: Operand,
    less: bool,
    signed: bool,
    target: u32,
}

#[derive(Clone, Copy)]
enum Operand {
    Reg(u32),
    Imm(u32),
}

fn test(insn: &Insn) -> Option<Test> {
    let f = &insn.fields;
    let (b, less, signed, target) = match insn.op {
        Op::BLTU => (Operand::Reg(f[1]), true, false, f[2]),
        Op::BGEU => (Operand::Reg(f[1]), false, false, f[2]),
        Op::BLT => (Operand::Reg(f[1]), true, true, f[2]),
        Op::BGE => (Operand::Reg(f[1]), false, true, f[2]),
        Op::BLTUI => (Operand::Imm(f[1]), true, false, f[2]),
        Op::BGEUI => (Operand::Imm(f[1]), false, false, f[2]),
        Op::BLTI => (Operand::Imm(f[1]), true, true, f[2]),
        Op::BGEI => (Operand::Imm(f[1]), false, true, f[2]),
        Op::BLTZ => (Operand::Imm(0), true, true, f[1]),
        Op::BGEZ => (Operand::Imm(0), false, true, f[1]),
        _ => return None,
    };
    Some(Test {
        a: f[0],
        b,
        less,
        signed,
        target,
    })
}

/// The registers an op of a spin reads, as bits, and the one it writes, or `None` for an op a
/// spin may not hold.
fn uses(insn: &Insn) -> Option<(u16, Option<u32>)> {
    let f = &insn.fields;
    let bit = |n: u32| 1u16 << n;
    Some(match insn.op {
        Op::RSR_CCOUNT | Op::MOVI | Op::MOVI_N => (0, Some(f[0])),
        Op::ADD | Op::ADD_N | Op::SUB => (bit(f[1]) | bit(f[2]), Some(f[0])),
        Op::ADDI | Op::ADDI_N | Op::ADDMI | Op::NEG | Op::MOV_N => (bit(f[1]), Some(f[0])),
        // `mov` is `or` of a register with itself.
        Op::OR if f[1] == f[2] => (bit(f[1]), Some(f[0])),
        Op::NOP | Op::NOP_N => (0, None),
        _ => return None,
    })
}

/// A register's value in a pass: a constant, or the low 32 bits of the cycle the pass starts at,
/// negated when `down` is set, plus `plus`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Value {
    Known(u32),
    Count { down: bool, plus: u32 },
}

impl Value {
    /// The sum, unless it counts the cycle twice.
    fn add(self, other: Value) -> Option<Value> {
        Some(match (self, other) {
            (Value::Known(x), Value::Known(y)) => Value::Known(x.wrapping_add(y)),
            (Value::Count { down, plus }, Value::Known(k))
            | (Value::Known(k), Value::Count { down, plus }) => Value::Count {
                down,
                plus: plus.wrapping_add(k),
            },
            (Value::Count { down: d1, plus: p1 }, Value::Count { down: d2, plus: p2 }) => {
                if d1 == d2 {
                    return None;
                }
                Value::Known(p1.wrapping_add(p2))
            }
        })
    }

    fn neg(self) -> Value {
        match self {
            Value::Known(x) => Value::Known(x.wrapping_neg()),
            Value::Count { down, plus } => Value::Count {
                down: !down,
                plus: plus.wrapping_neg(),
            },
        }
    }

    /// With the sign bit flipped, so a signed compare of two values is an unsigned one.
    fn flip(self) -> Value {
        match self {
            Value::Known(x) => Value::Known(x ^ SIGN),
            Value::Count { down, plus } => Value::Count {
                down,
                plus: plus ^ SIGN,
            },
        }
    }

    /// The value in a pass that starts at a cycle whose low 32 bits are `start`.
    fn at(self, start: u32) -> u32 {
        match self {
            Value::Known(x) => x,
            Value::Count { down: false, plus } => start.wrapping_add(plus),
            Value::Count { down: true, plus } => plus.wrapping_sub(start),
        }
    }
}

/// A spin's ops, from its first to its branch, by slot.
struct Body {
    slots: [usize; MOST_OPS],
    len: usize,
}

enum Refused {
    /// The ops make no spin at this code version.
    Never,
    /// Not now, though they may later.
    Now,
}

/// The ops from `head` to the branch in slot `branch`, all kept at `version`, if they can make a
/// spin.
fn body(table: &Taken, version: u64, head: u32, branch: usize) -> Result<Body, Refused> {
    let mut body = Body {
        slots: [0; MOST_OPS],
        len: 0,
    };
    let mut pc = head;
    let mut read = 0u16;
    let mut written = 0u16;
    loop {
        if body.len == MOST_OPS {
            return Err(Refused::Never);
        }
        let slot = table.find(pc, version).ok_or(Refused::Now)?;
        body.slots[body.len] = slot;
        body.len += 1;
        let insn = &table.kept(slot).insn;
        if slot == branch {
            let Some(test) = test(insn) else {
                return Err(Refused::Never);
            };
            read |= !written & (1 << test.a);
            if let Operand::Reg(b) = test.b {
                read |= !written & (1 << b);
            }
            // A register read before it is written keeps its value from pass to pass.
            return if read & written == 0 && test.target == head {
                Ok(body)
            } else {
                Err(Refused::Never)
            };
        }
        let (reads, writes) = uses(insn).ok_or(Refused::Never)?;
        read |= reads & !written;
        if let Some(n) = writes {
            written |= 1 << n;
        }
        pc = pc.wrapping_add(u32::from(insn.len));
    }
}

impl<C: Config> Core<C> {
    /// At the head of a loop that the branch in slot `branch` just went back to, inside a
    /// straight run, skips the passes that branch back again and start each op before `bound`.
    /// A pass costs a cycle for each op and the branch's redirect, plus the refetch of its first
    /// op when that op runs into the next word. Out of line and cold, so the straight run that
    /// calls it after each branch back stays as it was.
    #[cold]
    #[inline(never)]
    pub(super) fn skip_spin<B: Bus>(
        &mut self,
        bus: &mut B,
        table: &mut Taken,
        branch: usize,
        bound: Cycle,
    ) {
        // Skipping is worth the look only with room for several passes before the horizon.
        if table.kept(branch).spinless || bound.0.saturating_sub(self.cycle.0) < ROOM {
            return;
        }
        let Some(version) = bus.code_version() else {
            return;
        };
        let head = self.pc;
        let body = match body(table, version, head, branch) {
            Ok(body) => body,
            Err(Refused::Never) => {
                table.set_spinless(branch);
                return;
            }
            Err(Refused::Now) => return,
        };
        let ops = &body.slots[..body.len];
        // The checks a straight run makes before each op that can find something here: a window
        // overflow and a loop end. No op here changes what they read.
        let mut pc = head;
        for &slot in ops {
            let kept = table.kept(slot);
            pc = pc.wrapping_add(u32::from(kept.insn.len));
            let loops = pc == self.sr[Sr::Lend as usize]
                && self.sr[Sr::Lcount as usize] != 0
                && self.ps() & PS_EXCM == 0;
            if loops
                || kept.coprocessor.is_some()
                || self.overflow(u32::from(kept.insn.reach)).is_some()
            {
                return;
            }
        }
        let refetch = u64::from((head & 3) + u32::from(table.kept(ops[0]).insn.len) > 4);
        let count = ops.len() as u64;
        let pass = refetch + count + C::redirect_cycles();
        // The start of a pass's last op, from the pass's start.
        let last = refetch + count - 1;

        // One pass from the registers as they stand. An op reads `CCOUNT` at the pass's start
        // plus the cycles before it.
        let ccount = self.sr[Sr::Ccount as usize];
        let mut regs: [Value; 16] = std::array::from_fn(|n| Value::Known(self.a(n as u32)));
        let mut written = 0u16;
        for (i, &slot) in ops[..ops.len() - 1].iter().enumerate() {
            let insn = &table.kept(slot).insn;
            let f = &insn.fields;
            let reg = |n: u32| regs[n as usize];
            let value = match insn.op {
                Op::RSR_CCOUNT => Some(Value::Count {
                    down: false,
                    plus: ccount.wrapping_add((refetch + i as u64) as u32),
                }),
                Op::MOVI | Op::MOVI_N => Some(Value::Known(f[1])),
                Op::ADD | Op::ADD_N => reg(f[1]).add(reg(f[2])),
                Op::SUB => reg(f[1]).add(reg(f[2]).neg()),
                Op::ADDI | Op::ADDI_N | Op::ADDMI => reg(f[1]).add(Value::Known(f[2])),
                Op::NEG => Some(reg(f[1]).neg()),
                Op::MOV_N | Op::OR => Some(reg(f[1])),
                _ => continue,
            };
            let Some(value) = value else {
                return;
            };
            regs[f[0] as usize] = value;
            written |= 1 << f[0];
        }
        let Some(test) = test(&table.kept(ops[ops.len() - 1]).insn) else {
            return;
        };
        let mut a = regs[test.a as usize];
        let mut b = match test.b {
            Operand::Reg(n) => regs[n as usize],
            Operand::Imm(k) => Value::Known(k),
        };
        if test.signed {
            (a, b) = (a.flip(), b.flip());
        }
        // The branch goes back while the moving value lies in `lo..hi`.
        let (moving, lo, hi) = match (a, b, test.less) {
            (Value::Count { .. }, Value::Known(n), true) => (a, 0, u64::from(n)),
            (Value::Count { .. }, Value::Known(n), false) => (a, u64::from(n), 1 << 32),
            (Value::Known(n), Value::Count { .. }, true) => (b, u64::from(n) + 1, 1 << 32),
            (Value::Known(n), Value::Count { .. }, false) => (b, 0, u64::from(n) + 1),
            _ => return,
        };
        let Value::Count { down, .. } = moving else {
            return;
        };
        let start = self.cycle;
        let now = u64::from(moving.at(start.0 as u32));
        if !(lo..hi).contains(&now) {
            return;
        }
        // The passes that branch back. The value moves by `pass` each pass, and leaves `lo..hi`
        // before it could wrap.
        let back = if down {
            (now - lo) / pass + 1
        } else {
            (hi - now).div_ceil(pass)
        };
        // The passes whose last op starts before `bound`.
        let Some(room) = bound.0.checked_sub(start.0 + last + 1) else {
            return;
        };
        let passes = back.min(room / pass + 1);
        let final_start = Cycle(start.0 + (passes - 1) * pass);
        let skip = |core: &mut Self| {
            for (n, value) in regs.iter().enumerate() {
                if written & (1 << n) != 0 {
                    core.set_a(n as u32, value.at(final_start.0 as u32));
                }
            }
            core.cycle = Cycle(start.0 + passes * pass);
        };

        #[cfg(feature = "poll-skip-check")]
        {
            let mut skipped = self.clone();
            skip(&mut skipped);
            for _ in 0..passes {
                for &slot in ops {
                    self.keep_step(bus);
                    let kept = table.kept(slot);
                    let ran = self.straight_op(bus, kept);
                    assert!(ran.is_ok() && !kept.rechecks, "a skipped pass stopped");
                }
            }
            assert!(
                *self == skipped,
                "a skipped spin at {head:#x} ran apart over {passes} passes"
            );
        }
        #[cfg(not(feature = "poll-skip-check"))]
        {
            skip(self);
            bus.begin_step(Cycle(final_start.0 + last));
        }
    }
}
