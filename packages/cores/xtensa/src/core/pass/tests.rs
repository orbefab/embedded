//! The interval rules against the core itself. Each rule's range must hold what the core
//! computes for every pair of values in its input ranges, each decided branch must go that way
//! for every pair, and a divide whose costs the rule calls alike must cost the same. A planted
//! wrong rule must fail the same check.

use super::*;
use crate::bus::{Bus, Cycle, Exit, Reset, Width};
use crate::config::Esp32s3;

/// A small xorshift, so the tests need no crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn u32(&mut self) -> u32 {
        self.next() as u32
    }

    /// A range of a random kind: one value, a narrow range, a wide one, or one whose values
    /// have low zero bits.
    fn val(&mut self) -> Val {
        let a = match self.below(4) {
            0 => self.u32() & 0xff,
            1 => self.u32() >> self.below(32),
            _ => self.u32(),
        };
        let width = match self.below(5) {
            0 => 0,
            1 => self.below(16),
            2 => self.below(5000),
            3 => self.below(1 << 24),
            _ => u64::from(self.u32()),
        };
        let hi = (u64::from(a) + width).min(u64::from(u32::MAX)) as u32;
        if self.below(3) == 0 {
            let k = self.below(24) as u32;
            let (lo, hi) = (a >> k << k, hi >> k << k);
            return Val::range(lo, hi.max(lo), k);
        }
        Val::range(a, hi, 0)
    }

    /// A value in `v`'s range with its low zero bits, often one of its ends.
    fn pick(&mut self, v: Val) -> u32 {
        let x = match self.below(8) {
            0 => v.lo,
            1 => v.hi,
            _ => v.lo + (self.below(u64::from(v.hi - v.lo) + 1)) as u32,
        };
        let tz = v.tz.min(31);
        (x >> tz << tz).max(v.lo)
    }
}

/// A bus no op here reaches.
struct NoBus;

impl Bus for NoBus {
    fn fetch(&mut self, _: Cycle, _: u32) -> Result<u32, Exit> {
        Err(Exit::Stop)
    }

    fn load(&mut self, _: Cycle, _: u32, _: Width) -> Result<u32, Exit> {
        Err(Exit::Stop)
    }

    fn store(&mut self, _: Cycle, _: u32, _: Width, _: u32) -> Result<(), Exit> {
        Err(Exit::Stop)
    }

    fn interrupts(&mut self, _: Cycle) -> u32 {
        0
    }

    fn next_event(&mut self, _: Cycle) -> Option<Cycle> {
        None
    }
}

fn insn(op: Op, fields: [u32; 3], extra: u32) -> Insn {
    Insn {
        op,
        len: 3,
        fields: [fields[0], fields[1], fields[2], extra, 0, 0, 0, 0],
        field_count: 4,
        reach: 0,
    }
}

/// What the core does with `insn` when `a1` holds `x`, `a2` holds `y` and `SAR` holds `sar`,
/// after a divide when `follows` is set: what it leaves in `a0`, its cycles, and whether it
/// branched.
fn run(insn: &Insn, x: u32, y: u32, sar: u32, follows: bool) -> (u32, u64, bool) {
    let mut core = Core::<Esp32s3>::new(Reset {
        pc: 0x4037_8000,
        vecbase: 0x4037_8000,
        prid: 0,
    });
    core.set_a(1, x);
    core.set_a(2, y);
    core.sr[Sr::Sar as usize] = sar;
    if follows {
        core.pace |= PACE_DIVIDE;
    }
    let next = core.pc + 3;
    let Ok((cycles, target)) = core.operate(&mut NoBus, insn, next) else {
        panic!("{insn:?} trapped on {x:#x} {y:#x}");
    };
    (core.a(0), cycles, target.is_some())
}

type Rule = fn(&Insn, Val, Val, Val) -> Option<Val>;
type Alike = fn(Op, bool, Val) -> bool;
type Decide = fn(Op, &[u32; 8], Val, Val) -> Option<bool>;

/// The plain ops with rules, each with its fields: `a0` written from `a1` and `a2`.
fn plain(rng: &mut Rng) -> Insn {
    const OPS: [Op; 19] = [
        Op::MOV_N,
        Op::OR,
        Op::ADD,
        Op::ADDI,
        Op::SUB,
        Op::SLLI,
        Op::SRLI,
        Op::SLL,
        Op::SRL,
        Op::SRC,
        Op::EXTUI,
        Op::MULL,
        Op::MULUH,
        Op::QUOU,
        Op::REMU,
        Op::NSAU,
        Op::NEG,
        Op::SALTU,
        Op::SALT,
    ];
    let op = OPS[rng.below(OPS.len() as u64) as usize];
    match op {
        Op::ADDI => insn(op, [0, 1, rng.u32() >> rng.below(32)], 0),
        Op::SLLI | Op::SRLI => insn(op, [0, 1, rng.below(32) as u32], 0),
        Op::EXTUI => {
            let s = rng.below(32) as u32;
            let w = (rng.below(16) as u32 + 1).min(32 - s);
            insn(op, [0, 1, s], w)
        }
        // `mov`, an `or` of a register with itself, half the time.
        Op::OR => insn(op, [0, 1, 1 + rng.below(2) as u32], 0),
        _ => insn(op, [0, 1, 2], 0),
    }
}

/// Checks `rule` and the divide cost rule `alike` on `cases` random inputs, 8 values each,
/// against the core. Returns how many values it checked, or the first that falls outside the
/// rule's range or costs apart from the others.
fn check_rules(rule: Rule, alike: Alike, cases: u32, seed: u64) -> Result<u64, String> {
    let mut rng = Rng(seed);
    let mut checked = 0;
    for _ in 0..cases {
        let insn = plain(&mut rng);
        let op = insn.op;
        let mut a = rng.val();
        let mut b = if rng.below(2) == 0 {
            Val::known(rng.u32() >> rng.below(32))
        } else {
            rng.val()
        };
        if matches!(op, Op::QUOU | Op::REMU) && b.lo == 0 {
            b = Val::known(b.hi.max(1));
        }
        if op == Op::SRC && rng.below(2) == 0 {
            a = Val::known(a.lo);
        }
        // Both read the same load, so they hold the same value in each pass.
        let same = op == Op::SUB && rng.below(8) == 0;
        if same {
            a.load = Some(3);
            b = a;
        }
        let sar = Val::known(rng.below(64) as u32);
        let Some(out) = rule(&insn, a, b, sar) else {
            continue;
        };
        let follows = rng.below(2) == 0;
        let alike = matches!(op, Op::QUOU | Op::REMU) && alike(op, follows, a);
        let mut cost = None;
        for _ in 0..8 {
            let x = rng.pick(a);
            let y = if same || (op == Op::OR && insn.fields[2] == 1) {
                x
            } else {
                rng.pick(b)
            };
            if matches!(op, Op::QUOU | Op::REMU) && y == 0 {
                continue;
            }
            let (z, cycles, _) = run(&insn, x, y, sar.lo, follows);
            let fits =
                (out.lo..=out.hi).contains(&z) && (out.tz == 0 || z.trailing_zeros() >= out.tz);
            if !fits {
                return Err(format!(
                    "{op:?} {:?} a {a:?} b {b:?} sar {} gave {out:?}, but {x:#x} {y:#x} -> {z:#x}",
                    &insn.fields[..4],
                    sar.lo
                ));
            }
            if alike && *cost.get_or_insert(cycles) != cycles {
                return Err(format!(
                    "{op:?} a {a:?} after a divide {follows} costs apart"
                ));
            }
            checked += 1;
        }
    }
    Ok(checked)
}

/// Checks `decide` on `cases` random inputs, 8 values each, against the core. Returns how many
/// decided values it checked, or the first that goes the other way.
fn check_branches(decide: Decide, cases: u32, seed: u64) -> Result<u64, String> {
    const OPS: [Op; 18] = [
        Op::BEQZ,
        Op::BNEZ,
        Op::BEQ,
        Op::BNE,
        Op::BGEU,
        Op::BLTU,
        Op::BGE,
        Op::BLT,
        Op::BGEZ,
        Op::BLTZ,
        Op::BBCI,
        Op::BBSI,
        Op::BEQI,
        Op::BNEI,
        Op::BGEUI,
        Op::BLTUI,
        Op::BGEI,
        Op::BLTI,
    ];
    let mut rng = Rng(seed);
    let mut checked = 0;
    const TARGET: u32 = 0x4037_9000;
    for _ in 0..cases {
        let op = OPS[rng.below(OPS.len() as u64) as usize];
        let k = match op {
            Op::BBCI | Op::BBSI => rng.below(32) as u32,
            _ => rng.u32() >> rng.below(32),
        };
        let insn = match op {
            Op::BEQZ | Op::BNEZ | Op::BGEZ | Op::BLTZ => insn(op, [1, TARGET, 0], 0),
            Op::BEQ | Op::BNE | Op::BGEU | Op::BLTU | Op::BGE | Op::BLT => {
                insn(op, [1, 2, TARGET], 0)
            }
            _ => insn(op, [1, k, TARGET], 0),
        };
        let a = rng.val();
        let mut b = rng.val();
        let same = rng.below(8) == 0;
        let a = if same { Val { load: Some(5), ..a } } else { a };
        if same {
            b = a;
        }
        let Some(goes) = decide(op, &insn.fields, a, b) else {
            continue;
        };
        for _ in 0..8 {
            let x = rng.pick(a);
            let y = if same { x } else { rng.pick(b) };
            let (_, _, taken) = run(&insn, x, y, 0, false);
            if taken != goes {
                return Err(format!("{op:?} k {k:#x} a {a:?} b {b:?} at {x:#x} {y:#x}"));
            }
            checked += 1;
        }
    }
    Ok(checked)
}

#[test]
fn every_rule_holds_what_the_core_computes() {
    let checked = check_rules(
        compute,
        divide_costs_alike::<Esp32s3>,
        400_000,
        0x9e37_79b9_7f4a_7c15,
    )
    .unwrap();
    assert!(checked > 1_000_000, "only {checked} values checked");
}

#[test]
fn a_decided_branch_goes_that_way_for_every_value() {
    let checked = check_branches(decide, 400_000, 0x2545_f491_4f6c_dd1d).unwrap();
    assert!(checked > 1_000_000, "only {checked} values checked");
}

/// Each planted mistake is one a rule could plausibly make. The checks above must find it.
#[test]
fn a_wrong_rule_fails_the_checks() {
    let rules: [(&str, Rule); 4] = [
        ("remu across a quotient", |insn, a, b, sar| match insn.op {
            Op::REMU => {
                let d = b.single().filter(|&d| d != 0)?;
                Some(Val::range(
                    (a.lo % d).min(a.hi % d),
                    (a.lo % d).max(a.hi % d),
                    0,
                ))
            }
            _ => compute(insn, a, b, sar),
        }),
        ("add that wraps", |insn, a, b, sar| match insn.op {
            Op::ADD => {
                let (lo, hi) = (a.lo.wrapping_add(b.lo), a.hi.wrapping_add(b.hi));
                Some(Val::range(lo.min(hi), lo.max(hi), 0))
            }
            _ => compute(insn, a, b, sar),
        }),
        ("shift that drops high bits", |insn, a, b, sar| {
            match insn.op {
                Op::SLLI => {
                    let k = insn.fields[2];
                    let (lo, hi) = (a.lo << k, a.hi << k);
                    Some(Val::range(lo.min(hi), lo.max(hi), 0))
                }
                _ => compute(insn, a, b, sar),
            }
        }),
        (
            "sll that reads SAR as the left shift",
            |insn, a, b, sar| match insn.op {
                Op::SLL => Some(a.shl(sar.single()?)),
                _ => compute(insn, a, b, sar),
            },
        ),
    ];
    let alike = divide_costs_alike::<Esp32s3>;
    for (name, rule) in rules {
        assert!(
            check_rules(rule, alike, 400_000, 0x9e37_79b9_7f4a_7c15).is_err(),
            "the checks passed {name}"
        );
    }
    // A divide after a divide costs what its dividend's bits say.
    let every_cost_alike: Alike = |_, _, _| true;
    assert!(
        check_rules(compute, every_cost_alike, 400_000, 0x9e37_79b9_7f4a_7c15).is_err(),
        "the checks passed divides that all cost alike"
    );

    let branches: [(&str, Decide); 3] = [
        ("signed compare across the sign", |op, f, a, b| match op {
            Op::BGE | Op::BLT if a.lo >= b.hi => Some(op == Op::BGE),
            _ => decide(op, f, a, b),
        }),
        ("bit test past a carry", |op, f, a, b| match op {
            Op::BBCI | Op::BBSI => Some((a.lo >> f[1] & 1 == 0) == (op == Op::BBCI)),
            _ => decide(op, f, a, b),
        }),
        ("equal when ranges overlap", |op, f, a, b| match op {
            Op::BEQ | Op::BNE if a.lo <= b.hi && b.lo <= a.hi => Some(op == Op::BEQ),
            _ => decide(op, f, a, b),
        }),
    ];
    for (name, decide) in branches {
        assert!(
            check_branches(decide, 400_000, 0x2545_f491_4f6c_dd1d).is_err(),
            "the checks passed {name}"
        );
    }
}

/// A divide after a divide costs what its dividend's bits say, so a range that crosses a cost
/// step must not count as alike, and one inside a step must.
#[test]
fn a_follower_divide_costs_alike_only_inside_a_step() {
    let range = |lo, hi| Val::range(lo, hi, 0);
    assert!(divide_costs_alike::<Esp32s3>(
        Op::QUOU,
        true,
        range(0x1_0000, 0x1_ffff)
    ));
    assert!(!divide_costs_alike::<Esp32s3>(
        Op::QUOU,
        true,
        range(0x1_ffff, 0x2_0000)
    ));
    assert!(!divide_costs_alike::<Esp32s3>(Op::REMU, true, range(7, 8)));
    // A first divide costs the same for every dividend.
    assert!(divide_costs_alike::<Esp32s3>(
        Op::REMU,
        false,
        range(0, u32::MAX)
    ));
}
