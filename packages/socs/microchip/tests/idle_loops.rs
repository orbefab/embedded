//! Idle loops: `run_until` and `run_to_news` against a loop of single `Cpu::step` calls.
//!
//! A run skips the passes of a loop whose body writes no register, once one pass leaves `SREG`
//! as it found it. Arduino's `main` with an empty `loop()` is `SBIW r28, 0` and a `BREQ` back.
//! The loops here put a run limit at every cycle near the loop's start and a few passes in, and a
//! timer overflow with its interrupt enabled at every offset of a pass, so an interrupt becomes
//! pending inside a stretch the run would skip. Every state the host can read must match the
//! stepped core at each boundary.

use crate::common::runs::{
    MAIN, NOP, RETI, SEI, image, inc, matches_stepping, rjmp, seen, step_until,
};
use crate::common::{adiw, brbc, brbs, imm, ldi, out, rr, sbiw, sts};
use orbe_microchip::Cpu;
use std::panic;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

const TCCR0B: u16 = 0x25;
const TCNT0: u16 = 0x26;
const TIMSK0: u16 = 0x6E;
const TIMER0_OVF: usize = 0x20;

const fn cp(d: u16, r: u16) -> u16 {
    rr(0x1400, d, r)
}

const fn cpc(d: u16, r: u16) -> u16 {
    rr(0x0400, d, r)
}

const fn and(d: u16, r: u16) -> u16 {
    rr(0x2000, d, r)
}

const fn or(d: u16, r: u16) -> u16 {
    rr(0x2800, d, r)
}

const fn cpi(d: u16, k: u16) -> u16 {
    imm(0x3000, d, k)
}

const fn lsr(d: u16) -> u16 {
    0x9406 | (d << 4)
}

const fn bset(s: u16) -> u16 {
    0x9408 | (s << 4)
}

/// A loop's body, the branch that closes it as `(branch-if-set, bit)`, and the cycles of a pass
/// that branches back.
struct Shape {
    name: &'static str,
    body: Vec<u16>,
    set: bool,
    bit: u16,
    pass: u64,
}

fn shape(name: &'static str, body: Vec<u16>, set: bool, bit: u16, pass: u64) -> Shape {
    Shape {
        name,
        body,
        set,
        bit,
        pass,
    }
}

/// r24 and r25 hold 0x40, r26 holds 0x41, r27 holds 0x80 and r28 and r29 hold 0. Each loop
/// branches back on what those values give, and leaves on anything else.
fn shapes() -> Vec<Shape> {
    vec![
        shape("sbiw r28 0, breq", vec![sbiw(0, 28)], true, 1, 4),
        shape("nop cp, breq", vec![NOP, cp(24, 25)], true, 1, 4),
        shape("cpi, brne", vec![cpi(24, 0x41)], false, 1, 3),
        shape("cp, brcs", vec![cp(24, 26)], true, 0, 3),
        shape("and, brne", vec![and(24, 24)], false, 1, 3),
        shape("or, brmi", vec![or(27, 27)], true, 2, 3),
        shape("adiw z 0, brpl", vec![adiw(0, 30)], false, 2, 4),
        // `CPC` first reads the `C` and `Z` the pass before left, so the first pass from the
        // entry flags can leave other flags than the next.
        shape("cpc cpc, brcc", vec![cpc(24, 25), cpc(26, 24)], false, 0, 4),
        shape("cpc, breq", vec![cpc(24, 25)], true, 1, 3),
        shape("branch to itself", vec![], true, 1, 2),
        shape(
            "62 nops, cp, brcc",
            [vec![NOP; 62], vec![cp(26, 24)]].concat(),
            false,
            0,
            65,
        ),
    ]
}

/// `head`, the registers [`shapes`] names, the loop, `INC r21`, `INC r28`, and back to the load.
/// A body taken for the wrong loop ends elsewhere. Returns the image.
fn program(shape: &Shape, head: &[u16], isr: &[u16]) -> Vec<u16> {
    let mut main = head.to_vec();
    let load_at = MAIN + main.len();
    main.extend([
        ldi(24, 0x40),
        ldi(25, 0x40),
        ldi(26, 0x41),
        ldi(27, 0x80),
        ldi(28, 0),
        ldi(29, 0),
    ]);
    let top = MAIN + main.len();
    main.extend(&shape.body);
    let at = MAIN + main.len();
    let back = (top as i32 - at as i32 - 1) as u16;
    main.push(if shape.set {
        brbs(shape.bit, back)
    } else {
        brbc(shape.bit, back)
    });
    main.extend([inc(21), inc(28)]);
    main.push(rjmp(MAIN + main.len(), load_at));
    let isrs: &[usize] = if isr.is_empty() { &[] } else { &[TIMER0_OVF] };
    image(&main, isrs, isr)
}

/// The cycle the first pass of [`program`] starts on, with a head of single-cycle ops.
fn first_pass(head: usize) -> u64 {
    4 + head as u64 + 6
}

/// A run limit at every cycle of the first passes and of the passes around pass `m`, with the
/// `SREG` bits each loop never writes set and clear at the entry.
#[test]
fn a_limit_at_every_cycle_ends_as_stepped() {
    let m = 40;
    for shape in shapes() {
        for head in [vec![], vec![bset(0), bset(5), bset(6)], vec![bset(1)]] {
            let words = program(&shape, &head, &[]);
            let near = |pass: u64| first_pass(head.len()) + pass * shape.pass;
            let mut limits: Vec<u64> = (1..near(4)).collect();
            limits.extend(near(m - 2)..near(m + 2));
            for limit in limits {
                matches_stepping(
                    &words,
                    &[limit, limit + 2 * shape.pass + 1, limit + 3_001],
                    &format!("{}, head {head:x?}, limit {limit}", shape.name),
                );
            }
        }
    }
}

/// `TOV0` sets at every offset of a pass: `TCNT0` starts at each value. The ISR either counts
/// in r20, which the loop does not read, or also sets r28 and the `C` the next `CPC` reads, so
/// the loop leaves after the interrupt.
#[test]
fn a_timer_overflow_at_every_offset_of_a_pass_ends_as_stepped() {
    let isrs: [&[u16]; 2] = [&[inc(20), RETI], &[inc(20), inc(28), bset(0), RETI]];
    for shape in shapes() {
        for isr in isrs {
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
                let words = program(&shape, &head, isr);
                matches_stepping(
                    &words,
                    &[97, 300, 777, 1501, 3000, 9000],
                    &format!("{}, isr {isr:x?}, TCNT0 from {preset}", shape.name),
                );
            }
        }
    }
}

/// `TOV0` at /64 sets every 16,384 cycles, thousands of passes into a skip. The head's `NOP`s
/// move the overflow through every offset of a pass. With `SEI` the interrupt enters as stepped;
/// without it the flag stays pending, the run goes on skipping, and the flag reads as stepped.
#[test]
fn a_timer_overflow_in_the_middle_of_a_long_skip_ends_as_stepped() {
    for shape in shapes() {
        for sei in [true, false] {
            for nops in 0..=shape.pass {
                let mut head = vec![NOP; nops as usize];
                head.push(ldi(16, 1));
                head.extend(sts(16, TIMSK0));
                if sei {
                    head.push(SEI);
                }
                head.extend([ldi(16, 3), out(16, TCCR0B)]);
                let words = program(&shape, &head, &[inc(20), RETI]);
                matches_stepping(
                    &words,
                    &[16_000, 16_390, 40_000, 70_001],
                    &format!("{}, sei {sei}, {nops} nops", shape.name),
                );
            }
        }
    }
}

/// A host write to a register the loop reads, between runs, at every offset of a pass: the next
/// run reads the register it finds and leaves the loop as stepping does.
#[test]
fn a_host_write_between_runs_is_seen() {
    for shape in shapes() {
        let words = program(&shape, &[], &[]);
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
                let (reg, value) = [(24u8, 0x41u8), (28, 1), (27, 0x7F), (26, 0x3F)][i];
                run.set_reg(reg, value);
                stepped.set_reg(reg, value);
            }
        }
    }
}

/// A loop whose body writes a register is not idle and runs as stepped: a shift, an `AND` of
/// two registers, an `SBIW` of 1, and an `LDI` before a compare.
#[test]
fn a_loop_that_writes_a_register_runs_as_stepped() {
    let bodies = [
        vec![lsr(24)],
        vec![and(24, 27)],
        vec![sbiw(1, 28), NOP],
        vec![ldi(25, 0x40), cp(24, 25)],
    ];
    for body in bodies {
        let shape = shape("writes", body, false, 1, 0);
        let words = program(&shape, &[], &[]);
        for limit in [3u64, 40, 401, 1_501, 70_001] {
            matches_stepping(
                &words,
                &[limit, limit + 5_000],
                &format!("{:x?}, limit {limit}", shape.body),
            );
        }
    }
}

/// An empty Arduino `loop()` runs 25.8e9 cycles in one run with no event on the bus. Stepping
/// that takes minutes and the skip milliseconds, so a run past 30 s means the skip did not fire.
#[test]
fn an_empty_loop_runs_four_billion_passes_in_one_run() {
    let shape = shape("sbiw r28 0, breq", vec![sbiw(0, 28)], true, 1, 4);
    let words = program(&shape, &[], &[]);
    let end = 25_800_000_001u64;
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut cpu = Cpu::new();
        cpu.load_words(&words);
        cpu.run_until(end);
        send.send(cpu).ok();
    });
    let cpu = match receive.recv_timeout(Duration::from_secs(30)) {
        Ok(cpu) => cpu,
        Err(RecvTimeoutError::Timeout) => panic!("the idle loop skip did not fire"),
        Err(RecvTimeoutError::Disconnected) => panic::resume_unwind(worker.join().unwrap_err()),
    };
    // The run ends inside a pass, after the `SBIW` or the `BREQ`, still in the loop.
    let head = (MAIN + 6) as u16;
    assert!((end..end + 2).contains(&cpu.cycles()), "{}", cpu.cycles());
    assert!([head, head + 1].contains(&cpu.pc()), "{}", cpu.pc());
    assert_eq!((cpu.reg(21), cpu.sreg() & 0x02), (0, 0x02));
}
