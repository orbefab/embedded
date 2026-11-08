//! Countdown loops: `run_until` and `run_to_news` against a loop of single `Cpu::step` calls.
//!
//! A run jumps the passes of a loop that only counts a register down to zero. The loops here put
//! a run limit at every cycle near the loop's start, around each byte the counter borrows out of,
//! and around the exit, and a timer overflow with its interrupt enabled at every offset of a
//! pass. Every state the host can read must match the stepped core at each boundary.

use crate::common::runs::{
    MAIN, NOP, RETI, SEI, image, inc, matches_stepping, rjmp, seen, step_until,
};
use crate::common::{brbc, imm, in_io, ldi, out, rr, sbiw, sts};
use orbe_avr::Cpu;
use std::panic;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

const TCCR0B: u16 = 0x25;
const TCNT0: u16 = 0x26;
const PINB: u16 = 0x03;
const TIMSK0: u16 = 0x6E;
const TIMER0_OVF: usize = 0x20;

const fn subi(d: u16, k: u16) -> u16 {
    imm(0x5000, d, k)
}

const fn sbci(d: u16, k: u16) -> u16 {
    imm(0x4000, d, k)
}

const fn dec(d: u16) -> u16 {
    0x940A | (d << 4)
}

const fn bset(s: u16) -> u16 {
    0x9408 | (s << 4)
}

/// A countdown loop's body before its `BRNE`, the counter's first register and width in bytes,
/// the value each pass takes off, and the cycles of a pass that branches back.
struct Shape {
    name: &'static str,
    lo: u16,
    body: Vec<u16>,
    bytes: u32,
    k: u64,
    pass: u64,
}

fn shapes() -> Vec<Shape> {
    let shape = |name, body: Vec<u16>, bytes, k, pass| Shape {
        name,
        lo: 24,
        body,
        bytes,
        k,
        pass,
    };
    vec![
        shape(
            "subi sbci sbci",
            vec![subi(24, 1), sbci(25, 0), sbci(26, 0)],
            3,
            1,
            5,
        ),
        shape("sbiw", vec![sbiw(1, 24)], 2, 1, 4),
        shape("sbiw by 3", vec![sbiw(3, 24)], 2, 3, 4),
        shape("dec", vec![dec(24)], 1, 1, 3),
        shape("subi", vec![subi(24, 1)], 1, 1, 3),
        shape(
            "nops",
            vec![NOP, subi(24, 1), NOP, sbci(25, 0), NOP],
            2,
            1,
            7,
        ),
        shape("subi sbci by 3", vec![subi(24, 3), sbci(25, 0)], 2, 3, 4),
        shape(
            "four bytes",
            vec![subi(24, 1), sbci(25, 0), sbci(26, 0), sbci(27, 0)],
            4,
            1,
            6,
        ),
        Shape {
            lo: 26,
            ..shape("sbiw on X", vec![sbiw(1, 26)], 2, 1, 4)
        },
        Shape {
            lo: 28,
            ..shape("sbiw by 2 on Y", vec![sbiw(2, 28)], 2, 2, 4)
        },
        Shape {
            lo: 30,
            ..shape("nop sbiw on Z", vec![NOP, sbiw(1, 30)], 2, 1, 5)
        },
        Shape {
            lo: 30,
            ..shape("subi sbci on Z", vec![subi(30, 1), sbci(31, 0)], 2, 1, 4)
        },
        Shape {
            lo: 20,
            ..shape("subi r20", vec![subi(20, 1)], 1, 1, 3)
        },
        Shape {
            lo: 16,
            ..shape("dec r16", vec![dec(16)], 1, 1, 3)
        },
        Shape {
            lo: 0,
            ..shape("dec r0", vec![dec(0)], 1, 1, 3)
        },
        Shape {
            lo: 15,
            ..shape("dec r15", vec![dec(15)], 1, 1, 3)
        },
    ]
}

/// `NOP`s out to the farthest `BRNE`, 63 words back.
fn farthest() -> Shape {
    Shape {
        name: "sbiw after 62 nops",
        lo: 28,
        body: [vec![NOP; 62], vec![sbiw(1, 28)]].concat(),
        bytes: 2,
        k: 1,
        pass: 66,
    }
}

/// The counter loaded with `start`. A byte below r16 goes through the register 16 above it,
/// which is then left holding junk.
fn load(shape: &Shape, start: u64) -> Vec<u16> {
    let mut words = vec![];
    for byte in 0..shape.bytes {
        let (reg, value) = (
            shape.lo + byte as u16,
            ((start >> (8 * byte)) & 0xFF) as u16,
        );
        if reg < 16 {
            words.extend([
                ldi(reg + 16, value),
                rr(0x2C00, reg, reg + 16),
                ldi(reg + 16, 0x55),
            ]);
        } else {
            words.push(ldi(reg, value));
        }
    }
    words
}

/// `head`, junk in r24 and r25, then the [`load`], the loop, `INC r21`, an optional steady
/// `IN`, and back to the load. A body taken for the wrong counter ends elsewhere. With
/// `steady`, a run that starts at the `IN` reaches the loop through the steady pass check, and
/// otherwise through the free run. Returns the image, and [`first_pass`] is the cycle the first
/// pass starts on.
fn program(shape: &Shape, start: u64, head: &[u16], steady: bool, isr: &[u16]) -> Vec<u16> {
    let mut main = head.to_vec();
    main.extend([ldi(24, 0x77), ldi(25, 0x66)]);
    let load_at = MAIN + main.len();
    main.extend(load(shape, start));
    let top = MAIN + main.len();
    main.extend(&shape.body);
    let at = MAIN + main.len();
    main.push(brbc(1, (top as i32 - at as i32 - 1) as u16));
    main.push(inc(21));
    if steady {
        main.push(in_io(16, PINB));
    }
    main.push(rjmp(MAIN + main.len(), load_at));
    let isrs: &[usize] = if isr.is_empty() { &[] } else { &[TIMER0_OVF] };
    image(&main, isrs, isr)
}

/// The cycle the first pass of [`program`] starts on, with a head of single-cycle ops.
fn first_pass(shape: &Shape, head: usize) -> u64 {
    4 + (head + load(shape, 0).len()) as u64
}

/// The overflow ISR counts in r20 and takes 7 off the counter's low byte, or 1 with `DEC` where
/// `SUBI` cannot reach. Under `subi r20` both write r20, so it takes 6 and counts nothing.
fn isr(shape: &Shape) -> [u16; 3] {
    let take = if shape.lo < 16 {
        dec(shape.lo)
    } else {
        subi(shape.lo, 7)
    };
    [inc(20), take, RETI]
}

/// Counter starts whose passes borrow out of each byte, leave each byte zero, and cross each
/// byte's sign bit, `m` passes after the start.
fn starts(shape: &Shape, m: u64) -> Vec<u64> {
    let top = 1u64 << (8 * shape.bytes);
    let mut out = vec![0, 1, 2, 3, m * shape.k, m * shape.k + 1, top - 1];
    for bit in [7u32, 8, 15, 16, 23, 24, 31] {
        if bit < 8 * shape.bytes {
            for edge in [1u64 << bit, (1u64 << bit) - 1] {
                out.push((edge + m * shape.k) % top);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// A run limit at every cycle of the passes near the start, near pass `m`, where the counter
/// crosses a byte, and near the exit of a short count. The passes in between are jumped.
#[test]
fn a_limit_at_every_cycle_ends_as_stepped() {
    let m = 9;
    for shape in shapes() {
        for steady in [false, true] {
            for start in starts(&shape, m) {
                let words = program(&shape, start, &[], steady, &[]);
                let begin = first_pass(&shape, 0);
                let near = |pass: u64| begin + pass * shape.pass;
                let mut limits: Vec<u64> = (1..near(3)).collect();
                limits.extend(near(m - 2)..near(m + 2));
                for limit in limits {
                    matches_stepping(
                        &words,
                        &[limit, limit + 2 * shape.pass + 1, limit + 3_001],
                        &format!(
                            "{}, start {start:#x}, steady {steady}, limit {limit}",
                            shape.name
                        ),
                    );
                }
            }
        }
    }
}

/// `TOV0` sets at every offset of a pass: `TCNT0` starts at each value, and the [`isr`] runs.
/// Each overflow ends a run inside the loop, and the jumped passes must stop where stepping takes
/// the entry.
#[test]
fn a_timer_overflow_at_every_offset_of_a_pass_ends_as_stepped() {
    for shape in shapes() {
        let isr = isr(&shape);
        for preset in 0..=255u16 {
            let mut head = vec![ldi(16, 1)];
            head.extend(sts(16, TIMSK0));
            head.extend([
                SEI,
                ldi(16, preset),
                out(16, TCNT0),
                ldi(16, 1),
                out(16, TCCR0B),
            ]);
            for start in [0x30, (1u64 << (8 * shape.bytes)) - 0x21] {
                let words = program(&shape, start, &head, false, &isr);
                matches_stepping(
                    &words,
                    &[97, 300, 777, 1501, 3000, 9000],
                    &format!("{}, TCNT0 from {preset}, start {start:#x}", shape.name),
                );
            }
        }
    }
}

/// `TOV0` at /64 cuts a run every 16,384 cycles, thousands of passes into a long trip. The head's
/// `NOP`s move the overflow through every offset of a pass.
#[test]
fn a_timer_overflow_in_the_middle_of_a_long_trip_ends_as_stepped() {
    for shape in shapes() {
        let isr = isr(&shape);
        for nops in 0..=shape.pass {
            let mut head = vec![NOP; nops as usize];
            head.push(ldi(16, 1));
            head.extend(sts(16, TIMSK0));
            head.extend([SEI, ldi(16, 3), out(16, TCCR0B)]);
            let start = (1u64 << (8 * shape.bytes)) - 0x21;
            let words = program(&shape, start, &head, false, &isr);
            matches_stepping(
                &words,
                &[16_000, 16_390, 40_000, 70_001],
                &format!("{}, {nops} nops", shape.name),
            );
        }
    }
}

/// One run jumps from the start of an `SBIW` trip across the borrows out of a nonzero high byte.
#[test]
fn a_jump_across_a_high_byte_borrow_ends_as_stepped() {
    for shape in shapes()
        .into_iter()
        .filter(|shape| shape.name.contains("sbiw"))
    {
        for start in [0x0305, 0xA001, 0xFF00] {
            let words = program(&shape, start, &[], false, &[]);
            let begin = first_pass(&shape, 0);
            for pass in [5u64, 0x105, 0x205] {
                let near = begin + pass * shape.pass;
                for limit in near - shape.pass..near + shape.pass {
                    matches_stepping(
                        &words,
                        &[limit],
                        &format!("{}, start {start:#x}, limit {limit}", shape.name),
                    );
                }
            }
        }
    }
}

/// `SREG` starts at `0x61`, `T`, `H` and `C`. A loop writes none of `T`, `H` under `SBIW`, or `C`
/// and `H` under `DEC`, so a jump must leave them as stepping does. A limit falls at every cycle
/// of a pass after a jump.
#[test]
fn flags_the_loop_never_writes_keep_their_value() {
    let head = [bset(0), bset(5), bset(6)];
    for shape in shapes().into_iter().chain([farthest()]) {
        let words = program(&shape, 0x0123, &head, false, &[]);
        let begin = first_pass(&shape, head.len());
        for pass in [3u64, 50, 0x101] {
            let near = begin + pass * shape.pass;
            for limit in near..near + shape.pass {
                matches_stepping(
                    &words,
                    &[limit, limit + 9_001],
                    &format!("{}, limit {limit}", shape.name),
                );
            }
        }
    }
}

/// A loop of 2^32 - 1 passes ends in one run, entered from a free run and, after a steady
/// `IN`, from the steady run. Stepping those 25.8e9 cycles takes minutes, and the jump takes
/// milliseconds, so a run past 30 s means the jump did not fire.
#[test]
fn a_four_billion_pass_delay_ends_in_one_run() {
    for (steady, run) in [(false, "free run"), (true, "steady run")] {
        let mut main = if steady {
            vec![in_io(16, PINB)]
        } else {
            vec![NOP]
        };
        main.push(bset(6));
        main.extend([ldi(24, 0xFF), ldi(25, 0xFF), ldi(26, 0xFF), ldi(27, 0xFF)]);
        main.extend([
            subi(24, 1),
            sbci(25, 0),
            sbci(26, 0),
            sbci(27, 0),
            brbc(1, 0x7B),
        ]);
        main.extend([inc(21), rjmp(MAIN + main.len() + 1, MAIN + main.len() + 1)]);
        let words = image(&main, &[], &[]);
        let exit = 8 + 6 * 0xFFFF_FFFFu64 - 1;
        let (send, receive) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut cpu = Cpu::new();
            cpu.load_words(&words);
            cpu.run_until(exit + 1);
            send.send(cpu).ok();
        });
        let cpu = match receive.recv_timeout(Duration::from_secs(30)) {
            Ok(cpu) => cpu,
            Err(RecvTimeoutError::Timeout) => {
                panic!("the countdown jump did not fire in the {run}")
            }
            Err(RecvTimeoutError::Disconnected) => panic::resume_unwind(worker.join().unwrap_err()),
        };
        assert_eq!(
            (cpu.cycles(), cpu.pc(), cpu.sreg(), cpu.reg(21)),
            (exit + 1, (MAIN + 12) as u16, 0x40, 1),
            "{run}"
        );
        assert_eq!(
            (cpu.reg(24), cpu.reg(25), cpu.reg(26), cpu.reg(27)),
            (0, 0, 0, 0),
            "{run}"
        );
    }
}

/// A host write to the counter between runs, at every offset of a pass: the next run reads the
/// counter it finds. A 0 written between the decrement and the `BRNE` reaches the head.
#[test]
fn a_host_write_to_the_counter_between_runs_is_seen() {
    for shape in shapes() {
        let words = program(&shape, (1u64 << (8 * shape.bytes)) - 3, &[], false, &[]);
        for offset in 0..shape.pass {
            let mut run = Cpu::new();
            let mut stepped = Cpu::new();
            run.load_words(&words);
            stepped.load_words(&words);
            for (i, limit) in [200u64, 433, 1000, 1777].into_iter().enumerate() {
                let limit = limit + offset;
                run.run_until(limit);
                step_until(&mut stepped, limit);
                assert_eq!(
                    seen(&mut run),
                    seen(&mut stepped),
                    "{}, to {limit}",
                    shape.name
                );
                let value = [5u8, 0, 0x80, 1][i];
                run.set_reg(shape.lo as u8, value);
                stepped.set_reg(shape.lo as u8, value);
            }
        }
    }
}

/// An outer `DEC` loop around an inner `SBIW` loop. The inner loop's passes are jumped on each
/// outer pass, and the outer loop runs as stepped.
#[test]
fn a_nested_delay_ends_as_stepped() {
    let mut main = vec![ldi(18, 3)];
    let outer = MAIN + main.len();
    main.extend([ldi(24, 0x40), ldi(25, 0x01)]);
    let inner = MAIN + main.len();
    main.push(sbiw(1, 24));
    main.push(brbc(
        1,
        (inner as i32 - (MAIN + main.len()) as i32 - 1) as u16,
    ));
    main.push(dec(18));
    main.push(brbc(
        1,
        (outer as i32 - (MAIN + main.len()) as i32 - 1) as u16,
    ));
    main.push(inc(21));
    main.push(rjmp(MAIN + main.len(), MAIN));
    let words = image(&main, &[], &[]);
    let pass = 4 * 0x140 + 3 + 3;
    for limit in (1..40)
        .chain(pass - 20..pass + 20)
        .chain(3 * pass - 20..3 * pass + 20)
    {
        matches_stepping(&words, &[limit, limit + 7_001], &format!("limit {limit}"));
    }
}

/// A `BRNE` loop that is not a countdown runs as stepped: another op in the body, an `SBCI` that
/// is not of 0 on the next byte up, an `SBCI` after `DEC` or `SBIW`, or five bytes. The counter's
/// upper bytes are not 0, so a body taken for the wrong counter ends elsewhere.
#[test]
fn a_loop_with_another_op_runs_as_stepped() {
    let bodies = [
        vec![subi(24, 1), inc(20)],
        vec![inc(20), subi(24, 1)],
        vec![subi(24, 1), sbci(24, 0)],
        vec![sbci(24, 1)],
        vec![sbiw(0, 24)],
        vec![sbiw(1, 24), inc(20)],
        vec![dec(24), inc(20)],
        vec![dec(24), dec(25)],
        vec![subi(24, 3), sbci(25, 1)],
        vec![subi(24, 1), sbci(26, 0)],
        vec![dec(24), sbci(25, 0)],
        vec![sbiw(1, 24), sbci(26, 0)],
        vec![
            subi(24, 1),
            sbci(25, 0),
            sbci(26, 0),
            sbci(27, 0),
            sbci(28, 0),
        ],
    ];
    for body in bodies {
        let mut main = vec![ldi(24, 0x40), ldi(25, 0x02), ldi(26, 0x01)];
        main.extend(&body);
        let at = MAIN + main.len();
        main.push(brbc(1, (MAIN as i32 + 3 - at as i32 - 1) as u16));
        main.push(rjmp(MAIN + main.len(), MAIN));
        let words = image(&main, &[], &[]);
        for limit in [3u64, 40, 401, 1_501] {
            matches_stepping(
                &words,
                &[limit, limit + 5_000],
                &format!("{body:x?}, limit {limit}"),
            );
        }
    }
}

/// An `SBCI` first reads the `C` the head was left with. From 0 the first pass wraps and leaves
/// `C` set, so a jump would move that borrow into the pass that runs and change its `H`. A
/// leading `NOP` lets a limit fall before the next `SBCI` rewrites `H`.
#[test]
fn an_sbci_first_loop_runs_as_stepped() {
    let mut main = vec![ldi(24, 0)];
    main.extend([NOP, sbci(24, 1)]);
    let at = MAIN + main.len();
    main.push(brbc(1, (MAIN as i32 + 1 - at as i32 - 1) as u16));
    main.push(rjmp(MAIN + main.len(), MAIN));
    let words = image(&main, &[], &[]);
    for limit in 1..2_000u64 {
        matches_stepping(&words, &[limit], &format!("limit {limit}"));
    }
}
