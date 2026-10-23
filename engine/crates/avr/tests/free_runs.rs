//! `run_until` and `run_to_news` against a loop of single `Cpu::step` calls.
//!
//! The run loop retires a stretch of register-only ops without the per-instruction bus work. The
//! programs here put an interrupt, a timer event, an `SEI`, a run limit, or a byte that waits for
//! its stamp at every cycle offset of such a stretch. Others start a run where the last one left
//! a halt from a host poke, or the delay after `SEI` or `RETI`. Every state the host can read must
//! match the stepped core at each boundary.

use std::path::Path;

use crate::common::runs::{
    MAIN, NOP, RETI, Runner, SEI, image, inc, matches_stepping, matches_stepping_from, rjmp, seen,
    step_until,
};
use crate::common::{UCSR0B, UDR0, adiw, brbc, imm, in_io, ldi, out, rr, sts};
use sim_avr::{Channel, Cpu, parse_hex};

const EECR: u16 = 0x3F;
const SMCR: u16 = 0x33;
const TCCR0A: u16 = 0x24;
const TCCR0B: u16 = 0x25;
const TCNT0: u16 = 0x26;
const OCR0A: u16 = 0x27;
const DDRB: u16 = 0x04;
const PINB: u16 = 0x03;
const DDRD: u16 = 0x0A;
const TIMSK0: u16 = 0x6E;
const UBRR0L: u16 = 0xC4;

const EERE: u8 = 1 << 0;

const TIMER0_COMPA: usize = 0x1C;
const TIMER0_OVF: usize = 0x20;

const SLEEP: u16 = 0x9588;
const LPM_R0_Z: u16 = 0x95C8;

/// Runs `words` to `first` through each runner and through [`step_until`], pokes `value` into
/// `addr` in both, and compares at each later limit.
fn matches_stepping_after_poke(
    words: &[u16],
    first: u64,
    (addr, value): (u16, u8),
    limits: &[u64],
    label: &str,
) {
    for runner in [Runner::Until, Runner::ToNews] {
        let mut run = Cpu::new();
        let mut stepped = Cpu::new();
        run.load_words(words);
        stepped.load_words(words);
        runner.run(&mut run, first);
        step_until(&mut stepped, first);
        run.poke(addr, value);
        stepped.poke(addr, value);
        for &limit in limits {
            runner.run(&mut run, limit);
            step_until(&mut stepped, limit);
            assert_eq!(
                seen(&mut run),
                seen(&mut stepped),
                "{label}, {runner:?} to {limit}"
            );
        }
    }
}

/// Free ops of one, two and three cycles, a taken skip over a two-word op, and a branch, as one
/// loop of 22 or 23 cycles. Nothing in it touches the bus.
fn free_loop(at: usize) -> Vec<u16> {
    let mut words = vec![
        NOP,
        adiw(1, 24),
        LPM_R0_Z,
        rr(0x9C00, 2, 3),
        0xC000,
        0x940C,
        (at + 7) as u16,
        rr(0x1000, 0, 0),
        0x940C,
        0x0000,
        inc(20),
        0x0100 | (2 << 4) | 12,
        imm(0x5000, 17, 1),
        brbc(1, 0),
    ];
    words.push(rjmp(at + words.len(), at));
    words
}

/// The ISR reads `TCNT0`, which at clk/1 is the entry cycle, and leaves the return address on
/// the stack.
const STAMP_ISR: [u16; 3] = [in_io(28, TCNT0), inc(29), RETI];

/// `TOV0` sets at every offset of the free loop: `TCNT0` starts at each value, and the loop
/// length does not divide 256.
#[test]
fn an_overflow_interrupt_at_every_offset_enters_as_stepped() {
    for preset in 0..=255u16 {
        let mut main = vec![ldi(16, preset), out(16, TCNT0), ldi(16, 1)];
        main.extend(sts(16, TIMSK0));
        main.extend([SEI, out(16, TCCR0B)]);
        let at = MAIN + main.len();
        main.extend(free_loop(at));
        let words = image(&main, &[TIMER0_OVF], &STAMP_ISR);
        matches_stepping(
            &words,
            &[97, 300, 777, 1501, 3000],
            &format!("TCNT0 from {preset}"),
        );
    }
}

/// CTC on `OCR0A` toggles `OC0A` and requests `TIMER0_COMPA`. The match lands inside the free
/// loop at each offset as `OCR0A` sweeps, with and without the interrupt enabled.
#[test]
fn a_compare_match_inside_a_free_run_matches_stepping() {
    for ocr in 0..=63u16 {
        for timsk in [0, 2] {
            let mut main = vec![
                ldi(16, 0x40),
                out(16, DDRD),
                ldi(16, ocr),
                out(16, OCR0A),
                ldi(16, 0x42),
                out(16, TCCR0A),
                ldi(16, timsk),
            ];
            main.extend(sts(16, TIMSK0));
            main.extend([SEI, ldi(16, 1), out(16, TCCR0B)]);
            let at = MAIN + main.len();
            main.extend(free_loop(at));
            let words = image(&main, &[TIMER0_COMPA], &STAMP_ISR);
            matches_stepping(
                &words,
                &[61, 250, 999, 2000],
                &format!("OCR0A {ocr}, TIMSK0 {timsk}"),
            );
        }
    }
}

/// DS40002198 `SEI`: the instruction after it runs before a pending interrupt. `TOV0` is pending
/// with `I` clear, then `SEI` and three `INC`s. Exactly the first `INC` runs before the entry.
/// Once `SEI` retires the interrupt can be taken, so no free run starts.
#[test]
fn sei_with_a_pending_interrupt_runs_one_instruction_first() {
    let mut main = vec![ldi(16, 1)];
    main.extend(sts(16, TIMSK0));
    main.extend([out(16, TCCR0B), ldi(24, 100)]);
    main.extend([0x950A | (24 << 4), brbc(1, 0x7E)]);
    main.extend([SEI, inc(20), inc(21), inc(22)]);
    let after_first = MAIN + main.len() - 2;
    let spin = MAIN + main.len();
    main.push(rjmp(spin, spin));
    let isr_spin = MAIN + main.len();
    let words = image(&main, &[TIMER0_OVF], &[rjmp(isr_spin, isr_spin)]);

    let mut cpu = Cpu::new();
    cpu.load_words(&words);
    cpu.run_until(1_000);
    assert_eq!(cpu.pc() as usize, isr_spin, "the interrupt was taken");
    assert_eq!(
        (cpu.reg(20), cpu.reg(21), cpu.reg(22)),
        (1, 0, 0),
        "one instruction ran after SEI"
    );
    let sp = u16::from(cpu.peek(0x5D)) | (u16::from(cpu.peek(0x5E)) << 8);
    let ret = (u16::from(cpu.peek(sp + 1)) << 8) | u16::from(cpu.peek(sp + 2));
    assert_eq!(
        usize::from(ret),
        after_first,
        "it returns to the second INC"
    );
    matches_stepping(&words, &[350, 1_000], "SEI with TOV0 pending");
}

/// DS40002198 `SEI` and `RETI`. A run that starts where either retired still runs one instruction
/// before the pending interrupt. `TOV0` and `OCF0A` are pending with `I` clear. `SEI` lets the
/// first `INC` run, then `TIMER0_COMPA` enters and returns, and its `RETI` lets the second run
/// before `TIMER0_OVF`.
#[test]
fn a_run_from_sei_or_reti_runs_one_instruction_first() {
    let mut main = vec![ldi(16, 0x80), out(16, OCR0A), ldi(16, 3)];
    main.extend(sts(16, TIMSK0));
    main.extend([ldi(16, 1), out(16, TCCR0B), ldi(24, 100)]);
    main.extend([0x950A | (24 << 4), brbc(1, 0x7E)]);
    main.extend([ldi(16, 0), out(16, TCCR0B)]);
    let sei = MAIN + main.len();
    main.extend([SEI, inc(20), inc(21), inc(22)]);
    let spin = MAIN + main.len();
    main.push(rjmp(spin, spin));
    let mut words = image(&main, &[TIMER0_COMPA], &[RETI]);
    let reti = words.len() - 1;
    words[TIMER0_OVF] = rjmp(TIMER0_OVF, words.len());
    words.push(rjmp(words.len(), words.len()));

    let mut cpu = Cpu::new();
    cpu.load_words(&words);
    let mut retired = |at: usize| {
        while cpu.pc() as usize != at {
            cpu.step();
        }
        cpu.step();
        cpu.cycles()
    };
    let after_sei = retired(sei);
    let after_reti = retired(reti);
    for (split, what) in [(after_sei, "SEI"), (after_reti, "RETI")] {
        matches_stepping(&words, &[split, 1_000], &format!("split after {what}"));
    }
    cpu.run_until(1_000);
    assert_eq!(
        (cpu.reg(20), cpu.reg(21), cpu.reg(22)),
        (1, 1, 0),
        "one instruction ran after SEI and one after RETI"
    );
}

/// DS40002061B §8.6.3. Setting `EERE` halts the CPU four cycles before the next instruction. A
/// host poke of it between runs leaves that halt, and the window flag every EEPROM store sets,
/// for the next instruction. The run that follows must not start a free run on the `NOP`s, so the
/// `TCNT0` read after them sees the halt.
#[test]
fn a_host_eeprom_read_halts_the_next_instruction() {
    let mut main = vec![ldi(16, 1), out(16, TCCR0B)];
    main.extend([NOP; 24]);
    main.push(in_io(5, TCNT0));
    let at = MAIN + main.len();
    main.extend(free_loop(at));
    let words = image(&main, &[], &[]);
    for first in [3, 9, 17] {
        for gap in 1..=6 {
            matches_stepping_after_poke(
                &words,
                first,
                (EECR, EERE),
                &[first + gap, first + 80],
                &format!("EERE at {first}, then {gap}"),
            );
        }
    }
}

/// The halt from a host `EERE` poke outlives a wake and an interrupt entry, which close the
/// enable window but run no instruction. The `NOP`s at the start of the ISR must not run as a free
/// run, so its `TCNT0` read sees the halt.
#[test]
fn a_halt_from_a_host_poke_outlives_a_wake_and_an_entry() {
    let mut main = vec![ldi(16, 1), out(16, SMCR), ldi(16, 1)];
    main.extend(sts(16, TIMSK0));
    main.extend([SEI, out(16, TCCR0B)]);
    let sleep = MAIN + main.len();
    main.extend([SLEEP, rjmp(sleep + 1, sleep)]);
    let isr = [NOP, NOP, NOP, in_io(28, TCNT0), inc(29), RETI];
    let words = image(&main, &[TIMER0_OVF], &isr);
    let mut limits: Vec<u64> = (250..=290).collect();
    limits.extend([700, 1_200]);
    for first in [40, 120, 200] {
        matches_stepping_after_poke(
            &words,
            first,
            (EECR, EERE),
            &limits,
            &format!("EERE asleep at {first}"),
        );
    }
}

/// A run limit at every cycle of a free run ends where stepping ends. Timer0 runs, so the next
/// device event is `TOV0`, past every limit.
#[test]
fn a_limit_at_every_cycle_of_a_free_run_ends_as_stepped() {
    let mut main = vec![ldi(16, 1), out(16, TCCR0B)];
    let at = MAIN + main.len();
    main.extend(free_loop(at));
    let words = image(&main, &[], &[]);
    for limit in 1..=90 {
        matches_stepping(&words, &[limit, limit + 23], &format!("limit {limit}"));
    }
}

/// A pin toggle is news. `run_to_news` returns after the `SBI` that made it, ahead of the free
/// loop that follows. Called again with the report not taken, it runs one instruction.
#[test]
fn run_to_news_stops_at_the_toggle_before_a_free_run() {
    let mut main = vec![0x9A00 | (PINB << 3) | 5];
    let at = MAIN + main.len();
    main.extend(free_loop(at));
    let words = image(&main, &[], &[]);
    let toggled = 2 + 2;
    for limit in toggled + 1..toggled + 60 {
        let mut cpu = Cpu::new();
        cpu.load_words(&words);
        cpu.poke(0x20 + DDRB, 0x20);
        while cpu.take_pin().is_some() {}
        cpu.run_to_news(limit);
        assert_eq!(
            cpu.cycles(),
            toggled,
            "limit {limit}: stopped at the toggle"
        );
        cpu.run_to_news(limit);
        assert_eq!(
            cpu.cycles(),
            toggled + 1,
            "limit {limit}: one NOP with news waiting"
        );
        assert!(
            cpu.take_pin().is_some(),
            "limit {limit}: the toggle was reported"
        );
    }
}

/// DS40002061B §20.6.1. A byte stored to `UDR0` is stamped at the store's retire, and its frame
/// starts at the first transmit clock at or after that. At `UBRR0` 0 the clock ticks every 16
/// cycles from power-on. The store retires on a tick, so a stamp even one cycle late, left
/// waiting by the free loop that follows, starts the frame a bit later.
#[test]
fn a_byte_stored_before_a_free_run_is_stamped_at_its_store() {
    let mut main = vec![ldi(16, 0x08)];
    main.extend(sts(16, UCSR0B));
    main.extend([ldi(16, 0)]);
    main.extend(sts(16, UBRR0L));
    main.extend([NOP, NOP, NOP, NOP, NOP, ldi(16, 0x5A)]);
    main.extend(sts(16, UDR0));
    let stored = 2 + 1 + 2 + 1 + 2 + 5 + 1 + 2;
    assert_eq!(stored % 16, 0);
    let at = MAIN + main.len();
    main.extend(free_loop(at));
    let words = image(&main, &[], &[]);
    let mut cpu = Cpu::new();
    cpu.load_words(&words);
    cpu.run_until(400);
    assert_eq!(cpu.take(Channel::Usart0), Some((0x5A, stored + 10 * 16)));
    matches_stepping(&words, &[5, 9, 40, 200, 400], "UDR0 before a free loop");
}

/// Each shipped firmware image and the interrupt-heavy timing programs, compared with stepping
/// at boundaries that fall at odd places in their loops.
#[test]
fn shipped_and_interrupt_heavy_images_run_as_stepped() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let firmware = crate_dir.join("../../firmware");
    let timing = crate_dir.join("tests/timing/hex");
    let mut images: Vec<_> = std::fs::read_dir(&firmware)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "hex"))
        .collect();
    images.sort();
    assert!(images.len() >= 4, "the shipped examples' firmware is there");
    for name in [
        "t0ovf_1",
        "t1ovf_1",
        "int_entry",
        "int_reti",
        "t0_double_cs",
    ] {
        images.push(timing.join(format!("{name}.hex")));
    }
    let boundaries: Vec<u64> = (1..=24).map(|n| n * 9_973).collect();
    for path in images {
        let image = parse_hex(&std::fs::read_to_string(&path).unwrap()).unwrap();
        matches_stepping_from(
            &|cpu| cpu.load_image(&image).unwrap(),
            &boundaries,
            &path.display().to_string(),
        );
    }
}
