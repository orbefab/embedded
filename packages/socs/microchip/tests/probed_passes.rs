//! Loops whose pass calls, pushes and writes SRAM: `run_until` and `run_to_news` against a loop
//! of single `Cpu::step` calls.
//!
//! A run skips the passes of such a loop when a pass comes back to its anchor with the same
//! registers, `SREG` and stack pointer, read only steady bytes, wrote no device, and put back every
//! byte it wrote. Each loop here calls a routine that pushes a register and sets an SRAM byte and
//! clears it again, as Arduino's `Keypad::getKey()` does. The event that ends a wait falls at every
//! offset of a pass, and a run limit at every cycle of one. Passes that read a register that moves
//! with the cycle, write a port, or leave a byte changed must not be skipped.

use std::panic;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::common::runs::{
    MAIN, RETI, Runner, SEI, image, inc, matches_stepping, rjmp, seen, step_until,
};
use crate::common::{adiw, cbi, imm, in_io, ldd, ldi, lds, out, sbi, sbic, sbiw, sbrc, sbrs, sts};
use orbe_microchip::{Cpu, Level, Port};

/// I/O addresses, for `IN`, `OUT`, `SBI` and `CBI`.
const IO_DDRC: u16 = 0x07;
const IO_PORTC: u16 = 0x08;
const IO_TIFR0: u16 = 0x15;
const IO_TCCR0B: u16 = 0x25;
const IO_TCNT0: u16 = 0x26;
const IO_SPL: u16 = 0x3D;
const IO_SPH: u16 = 0x3E;
const IO_SREG: u16 = 0x3F;
/// Data-space addresses, for `LD`, `ST`, `LDS` and `STS`.
const TIFR0: u16 = 0x35;
const TCNT0: u16 = 0x46;
const PORTC: u16 = 0x28;
const WDTCSR: u16 = 0x60;
const TIMSK0: u16 = 0x6E;
/// The byte the routine sets and clears on every pass.
const SCRATCH: u16 = 0x0200;
/// The byte an ISR sets and the loop clears.
const FLAG: u16 = 0x0201;
const COUNT: u16 = 0x0202;

const TIMER0_OVF: usize = 0x20;

const RET: u16 = 0x9508;
const WDR: u16 = 0x95A8;
const CLI: u16 = 0x94F8;

const fn push(r: u16) -> u16 {
    0x920F | (r << 4)
}

const fn pop(d: u16) -> u16 {
    0x900F | (d << 4)
}

/// `LD Rd, X`.
const fn ld_x(d: u16) -> u16 {
    0x900C | (d << 4)
}

/// `ST X, Rr`.
const fn st_x(r: u16) -> u16 {
    0x920C | (r << 4)
}

const fn call(to: usize) -> [u16; 2] {
    [0x940E, to as u16]
}

/// `CPI Rd, k`.
const fn cpi(d: u16, k: u16) -> u16 {
    imm(0x3000, d, k)
}

/// `BRCS` over the next word.
const BRCS_1: u16 = 0xF008;

/// `X` at data-space `addr`.
fn x_at(addr: u16) -> [u16; 2] {
    [ldi(26, addr & 0xFF), ldi(27, addr >> 8)]
}

/// `head`, then a loop that calls a routine and runs `after` once it returns. The routine pushes
/// r16, sets `SCRATCH` to 1, runs `body`, sets `SCRATCH` back to 0 and pops r16. `after` may only
/// skip within itself.
fn calling(head: &[u16], body: &[u16], after: &[u16]) -> Vec<u16> {
    let mut main = head.to_vec();
    let top = MAIN + main.len();
    let routine = top + 2 + after.len() + 1;
    main.extend(call(routine));
    main.extend_from_slice(after);
    main.push(rjmp(MAIN + main.len(), top));
    main.extend([push(16), ldi(16, 1)]);
    main.extend(sts(16, SCRATCH));
    main.extend_from_slice(body);
    main.push(ldi(16, 0));
    main.extend(sts(16, SCRATCH));
    main.extend([pop(16), RET]);
    main
}

/// Timer0 from `preset` at clock select `cs`.
fn timer0(preset: u16, cs: u16) -> Vec<u16> {
    vec![
        ldi(16, preset),
        out(16, IO_TCNT0),
        ldi(16, cs),
        out(16, IO_TCCR0B),
    ]
}

/// The routine reads `TIFR0` through `X`. Once `TOV0` is up, the loop counts in r20 and clears it.
/// `TOV0` sets at every offset of a pass, at three prescalers: a skipped pass must end where
/// stepping reads the flag set, and the prescaler's phase must survive the skip.
#[test]
fn a_timer_flag_read_in_a_call_ends_the_skip_as_stepped() {
    for (cs, step, boundaries) in [
        (1, 1, &[97, 300, 777, 1501, 3000, 9000][..]),
        (2, 3, &[700, 2100, 4097, 9000, 20_000][..]),
        (3, 17, &[5000, 16_385, 40_000, 70_000][..]),
    ] {
        for preset in (0..=255u16).step_by(step) {
            let mut head = timer0(preset, cs);
            head.extend(x_at(TIFR0));
            let after = [sbrc(17, 0), inc(20), sbrc(17, 0), out(17, IO_TIFR0)];
            let words = image(&calling(&head, &[ld_x(17)], &after), &[], &[]);
            matches_stepping(
                &words,
                boundaries,
                &format!("clock select {cs}, TCNT0 from {preset}"),
            );
        }
    }
}

/// The overflow ISR sets an SRAM flag that the routine reads with `LDS`. The entry falls at every
/// offset of a pass, and it pushes its return address into the bytes the call pushes into, so the
/// first pass after it leaves those bytes changed and the next one puts them back.
#[test]
fn an_isr_that_sets_a_flag_read_in_a_call_ends_the_skip_as_stepped() {
    for preset in 0..=255u16 {
        let mut head = vec![ldi(16, 1)];
        head.extend(sts(16, TIMSK0));
        head.push(SEI);
        head.extend(timer0(preset, 1));
        let mut after = vec![sbrc(17, 0), inc(20), sbrc(17, 0)];
        after.extend(sts(1, FLAG));
        let main = calling(&head, &lds(17, FLAG), &after);
        let mut isr = vec![push(16), ldi(16, 1)];
        isr.extend(sts(16, FLAG));
        isr.extend([pop(16), RETI]);
        let words = image(&main, &[TIMER0_OVF], &isr);
        matches_stepping(
            &words,
            &[97, 300, 777, 1501, 3000, 9000],
            &format!("TCNT0 from {preset}"),
        );
    }
}

/// A run limit at every cycle of the first passes ends where stepping ends. Timer0 counts at
/// /1024, so no event falls inside the limits.
#[test]
fn a_limit_at_every_cycle_of_a_call_pass_ends_as_stepped() {
    let mut head = timer0(0, 5);
    head.extend(x_at(TIFR0));
    let after = [sbrc(17, 0), inc(20), sbrc(17, 0), out(17, IO_TIFR0)];
    let words = image(&calling(&head, &[ld_x(17)], &after), &[], &[]);
    for limit in 1..=150 {
        matches_stepping(
            &words,
            &[limit, limit + 61, limit + 5_000],
            &format!("limit {limit}"),
        );
    }
}

/// Counts in r20 while `TCNT0` reads 200 or more, and leaves r17 at 0 so the registers repeat.
const COUNT_FROM_200: [u16; 4] = [cpi(17, 200), BRCS_1, inc(20), ldi(17, 0)];

/// The routine reads `TCNT0` through `X`, a load that is not steady. A skip to the overflow would
/// miss every pass that reads 200 or more.
#[test]
fn a_call_pass_that_loads_tcnt0_is_not_skipped() {
    let mut head = timer0(0, 5);
    head.extend(x_at(TCNT0));
    let words = image(&calling(&head, &[ld_x(17)], &COUNT_FROM_200), &[], &[]);
    matches_stepping(
        &words,
        &[1000, 100_000, 204_000, 205_000, 210_000, 262_000],
        "LD of TCNT0",
    );
}

/// The routine reads `TCNT0` with `IN`, which a free run answers with the projected count.
#[test]
fn a_call_pass_that_reads_tcnt0_with_in_is_not_skipped() {
    let head = timer0(0, 5);
    let words = image(
        &calling(&head, &[in_io(17, IO_TCNT0)], &COUNT_FROM_200),
        &[],
        &[],
    );
    matches_stepping(
        &words,
        &[1000, 100_000, 204_000, 205_000, 210_000, 262_000],
        "IN of TCNT0",
    );
}

/// The routine adds one to an SRAM byte and puts its registers back, so only that byte differs
/// from one pass to the next.
#[test]
fn a_call_pass_that_leaves_a_byte_changed_is_not_skipped() {
    let mut body = lds(17, COUNT).to_vec();
    body.push(inc(17));
    body.extend(sts(17, COUNT));
    body.push(ldi(17, 0));
    let words = image(&calling(&[], &body, &[]), &[], &[]);
    matches_stepping(&words, &[100, 1000, 10_000, 100_000], "COUNT");
}

/// The routine sets `PC7` and clears it, through `ST` and through `SBI` and `CBI`. Each pass is
/// two pin edges. `PC7` is in no pin-change group, so an edge queues no event, and only the
/// probe's check of each store keeps the pass from being skipped.
#[test]
fn a_call_pass_that_writes_a_port_is_not_skipped() {
    let head = [ldi(16, 0x80), out(16, IO_DDRC), ldi(26, PORTC), ldi(27, 0)];
    let st = [ldi(17, 0x80), st_x(17), ldi(17, 0), st_x(17)];
    let bits = [sbi(IO_PORTC, 7), cbi(IO_PORTC, 7)];
    for (name, body) in [("ST", &st[..]), ("SBI and CBI", &bits[..])] {
        let words = image(&calling(&head, body, &[]), &[], &[]);
        matches_stepping(&words, &[100, 1000, 10_000, 30_000], name);
    }
}

/// The watchdog in reset mode at its shortest timeout, 2K cycles of its 128 kHz clock. With `WDR`
/// in the routine it never fires. Without, it resets the chip at each timeout. No boundary falls
/// in the reset delay, which a step crosses whole.
#[test]
fn a_call_pass_under_the_watchdog_ends_as_stepped() {
    let mut head = vec![ldi(16, 0x18)];
    head.extend(sts(16, WDTCSR));
    head.push(ldi(16, 0x08));
    head.extend(sts(16, WDTCSR));
    for (name, body) in [("with WDR", &[WDR][..]), ("without WDR", &[][..])] {
        let words = image(&calling(&head, body, &[]), &[], &[]);
        matches_stepping(
            &words,
            &[1000, 255_000, 1_300_000, 1_500_000, 2_600_000],
            name,
        );
    }
}

/// A routine that makes a 4-byte frame as avr-gcc does, through `SPL` and `SPH` under `CLI`,
/// stores r16 into it, reads it back, and frees it.
fn framed_routine() -> Vec<u16> {
    vec![
        in_io(28, IO_SPL),
        in_io(29, IO_SPH),
        sbiw(4, 28),
        in_io(0, IO_SREG),
        CLI,
        out(29, IO_SPH),
        out(0, IO_SREG),
        out(28, IO_SPL),
        ldd(0x0008, true, 16, 1),
        ldd(0x0008, false, 17, 1),
        adiw(4, 28),
        in_io(0, IO_SREG),
        CLI,
        out(29, IO_SPH),
        out(0, IO_SREG),
        out(28, IO_SPL),
    ]
}

/// The framed routine, with limits at every cycle of the first passes.
#[test]
fn a_call_pass_that_moves_the_stack_pointer_ends_as_stepped() {
    let words = image(&calling(&[], &framed_routine(), &[]), &[], &[]);
    for limit in 1..=120 {
        matches_stepping(
            &words,
            &[limit, limit + 97, limit + 20_000],
            &format!("limit {limit}"),
        );
    }
}

/// The routine sets 24 bytes and clears them, then adds one to `COUNT`, the 25th byte it writes.
/// The probe notes 16, so it cannot see that `COUNT` changed, and the pass is not skipped.
#[test]
fn a_call_pass_that_writes_more_bytes_than_the_probe_notes_is_not_skipped() {
    let mut body = vec![];
    for n in 0..24 {
        body.extend(sts(16, 0x0300 + n));
    }
    for n in 0..24 {
        body.extend(sts(1, 0x0300 + n));
    }
    body.extend(lds(17, COUNT));
    body.push(inc(17));
    body.extend(sts(17, COUNT));
    body.push(ldi(17, 0));
    let words = image(&calling(&[], &body, &[]), &[], &[]);
    matches_stepping(&words, &[100, 1000, 10_000, 30_000], "25 bytes");
}

/// The framed routine for 2^36 cycles, with no event. Stepping that takes hours, and the skip takes
/// milliseconds, so a run past 30 s means no pass was skipped.
#[test]
fn a_call_loop_with_no_event_ends_in_one_run() {
    let main = calling(&[], &framed_routine(), &[]);
    let words = image(&main, &[], &[]);
    let limit = 1u64 << 36;
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut cpu = Cpu::new();
        cpu.load_words(&words);
        cpu.run_until(limit);
        send.send(cpu).ok();
    });
    let cpu = match receive.recv_timeout(Duration::from_secs(30)) {
        Ok(cpu) => cpu,
        Err(RecvTimeoutError::Timeout) => panic!("no pass was skipped"),
        Err(RecvTimeoutError::Disconnected) => panic::resume_unwind(worker.join().unwrap_err()),
    };
    assert!(cpu.cycles() >= limit && cpu.cycles() < limit + 8);
}

/// The routine runs `SEI`, which steps, then reads `PC7` with `SBIC`, and the loop counts the
/// passes that read it high in an SRAM byte. A drive is first seen by a read that starts two
/// cycles later, so a pass anchored at the `SEI` reads the old level one cycle after the drive.
/// `PC7` is in no pin-change group, so no event marks the cycle `PINC` changes. The host drives
/// `PC7` at every cycle offset of a pass, and one run follows it past the settle.
#[test]
fn a_call_pass_follows_a_host_drive_as_stepped() {
    const IO_PINC: u16 = 0x06;
    let pc7 = Port::C.pin_id(7);
    let body = [ldi(17, 0), SEI, sbic(IO_PINC, 7), ldi(17, 1)];
    // At `MAIN + 2`, after the call: add one to `COUNT` when the read was high.
    let mut after = vec![sbrs(17, 0), rjmp(MAIN + 3, MAIN + 9)];
    after.extend(lds(18, COUNT));
    after.push(inc(18));
    after.extend(sts(18, COUNT));
    after.push(ldi(18, 0));
    let words = image(&calling(&[], &body, &after), &[], &[]);
    for runner in [Runner::Until, Runner::ToNews] {
        for first in 300..340 {
            let mut run = Cpu::new();
            let mut stepped = Cpu::new();
            run.load_words(&words);
            stepped.load_words(&words);
            let mut at = first;
            for level in [Level::High, Level::Low, Level::High] {
                for limit in [at, at + 500] {
                    runner.run(&mut run, limit);
                    step_until(&mut stepped, limit);
                    assert_eq!(
                        seen(&mut run),
                        seen(&mut stepped),
                        "{runner:?}, {level:?} at {at}, to {limit}"
                    );
                    if limit == at {
                        run.drive_pin(pc7, level);
                        stepped.drive_pin(pc7, level);
                    }
                }
                at += 500;
            }
        }
    }
}
