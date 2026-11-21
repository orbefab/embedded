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
use crate::common::{SREG, UCSR0B, UDR0, adiw, brbc, imm, in_io, ldi, out, rr, sts};
use orbe_microchip::{Channel, Cpu, FuseBytes, Sent, parse_hex};

const EECR: u16 = 0x3F;
const EIMSK: u16 = 0x3D;
const TIFR0: u16 = 0x35;
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

const INT0: usize = 0x02;
const INT1: usize = 0x04;
const TIMER0_COMPA: usize = 0x1C;
const TIMER0_OVF: usize = 0x20;
const USART_UDRE: usize = 0x26;
const EE_READY: usize = 0x2C;

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

/// `TOIE0` and `I` set, Timer0 at clk/1 from `preset`, then the free loop.
fn awake(preset: u16) -> Vec<u16> {
    let mut main = vec![ldi(16, preset), out(16, TCNT0), ldi(16, 1)];
    main.extend(sts(16, TIMSK0));
    main.extend([SEI, out(16, TCCR0B)]);
    let at = MAIN + main.len();
    main.extend(free_loop(at));
    image(&main, &[TIMER0_OVF], &STAMP_ISR)
}

/// `TOIE0` and `I` set, Timer0 at clk/1, then `SLEEP` in idle until each overflow. The ISR opens
/// with `NOP`s.
fn asleep() -> Vec<u16> {
    let mut main = vec![ldi(16, 1), out(16, SMCR), ldi(16, 1)];
    main.extend(sts(16, TIMSK0));
    main.extend([SEI, out(16, TCCR0B)]);
    let sleep = MAIN + main.len();
    main.extend([SLEEP, rjmp(sleep + 1, sleep)]);
    let isr = [NOP, NOP, NOP, in_io(28, TCNT0), inc(29), RETI];
    image(&main, &[TIMER0_OVF], &isr)
}

/// The return address the last interrupt entry pushed.
fn stacked_return(cpu: &Cpu) -> u16 {
    let sp = u16::from(cpu.peek(0x5D)) | (u16::from(cpu.peek(0x5E)) << 8);
    (u16::from(cpu.peek(sp + 1)) << 8) | u16::from(cpu.peek(sp + 2))
}

/// The cycle count, `PC`, `SREG` and the registers.
fn core_state(cpu: &Cpu) -> (u64, u16, u8, [u8; 32]) {
    let regs = std::array::from_fn(|r| cpu.reg(r as u8));
    (cpu.cycles(), cpu.pc(), cpu.sreg(), regs)
}

/// `TOV0` sets at every offset of the free loop: `TCNT0` starts at each value, and the loop
/// length does not divide 256.
#[test]
fn an_overflow_interrupt_at_every_offset_enters_as_stepped() {
    for preset in 0..=255u16 {
        matches_stepping(
            &awake(preset),
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
    assert_eq!(
        usize::from(stacked_return(&cpu)),
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
    let words = asleep();
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
    let sent = Sent {
        usart: 0,
        byte: 0x5A,
        start: stored,
        end: stored + 10 * 16,
    };
    assert_eq!(cpu.clone().take_sent(u64::MAX), Some(sent));
    assert_eq!(cpu.take(Channel::Usart0), Some((0x5A, stored + 10 * 16)));
    let every: Vec<u64> = (1..=400).collect();
    matches_stepping(&words, &every, "UDR0 before a free loop");
}

/// Each shipped firmware image and the interrupt-heavy timing programs, compared with stepping
/// at boundaries that fall at odd places in their loops.
#[test]
fn shipped_and_interrupt_heavy_images_run_as_stepped() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let firmware = crate_dir.join("../../engine/tests/firmware");
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

/// The Blocks' firmware and every other image whose `delay()` spins on `micros()`, run to
/// boundaries a few cycles to a few thousand cycles apart. A boundary lands inside every part of
/// a spin pass, and a run between two boundaries can skip from none to dozens of passes. Each
/// state the host can read, registers, `SREG` and the stack included, must match stepping.
#[test]
fn delay_images_run_as_stepped_at_dense_boundaries() {
    // `parts/atmega328p-tqfp32/part.jsonc` `model.fuses`, the chip on the Nano and Uno boards.
    const PART_FUSES: FuseBytes = FuseBytes {
        low: 0xFF,
        high: 0xDF,
        extended: 0xFD,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut images: Vec<_> = std::fs::read_dir(root.join("blocks"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|dir| dir.join("firmware"))
        .flat_map(|dir| std::fs::read_dir(dir).unwrap())
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "hex"))
        .collect();
    images.sort();
    assert!(images.len() >= 4, "every Block's firmware is there");
    let sketch = root.join("packages/engine/tests/firmware/sketch");
    for name in ["blink", "tone", "dht22", "analog_read_serial", "cst816"] {
        images.push(sketch.join(format!("{name}.hex")));
    }
    for name in ["split", "pulse"] {
        images.push(root.join(format!("apps/orbe/tests/firmware/{name}/{name}.hex")));
    }
    let gaps = [89, 1, 151, 7, 397, 2, 1009, 13, 2003, 3];
    let boundaries: Vec<u64> = gaps
        .iter()
        .cycle()
        .scan(0, |at, gap| {
            *at += gap;
            Some(*at)
        })
        .take_while(|&at| at <= 1_000_000)
        .collect();
    for path in images {
        let image = parse_hex(&std::fs::read_to_string(&path).unwrap()).unwrap();
        matches_stepping_from(
            &|cpu| {
                cpu.set_fuses(PART_FUSES.low, PART_FUSES.high, PART_FUSES.extended);
                cpu.load_image(&image).unwrap();
            },
            &boundaries,
            &path.display().to_string(),
        );
    }
}

/// DS40002061B §7.7. A host poke between runs lands at the boundary the core sits on. An
/// interrupt it makes takeable is entered there, before the next instruction, by a run and by a
/// step alike. Every I/O register gets ten values, awake and asleep. The core state is compared at
/// five limits, and everything the host can read at the last.
#[test]
fn a_poke_of_any_io_register_runs_as_stepped() {
    let values = [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0xFF];
    let mut differ = Vec::new();
    for (name, words, firsts) in [
        ("awake", awake(0), [37, 101]),
        ("asleep", asleep(), [40, 200]),
    ] {
        for first in firsts {
            for runner in [Runner::Until, Runner::ToNews] {
                let mut run_at = Cpu::new();
                run_at.load_words(&words);
                let mut stepped_at = run_at.clone();
                runner.run(&mut run_at, first);
                step_until(&mut stepped_at, first);
                for addr in 0x20..=0xFF {
                    for value in values {
                        let mut run = run_at.clone();
                        let mut stepped = stepped_at.clone();
                        run.poke(addr, value);
                        stepped.poke(addr, value);
                        let split = [1, 3, 5, 9, 60].into_iter().find(|&gap| {
                            runner.run(&mut run, first + gap);
                            step_until(&mut stepped, first + gap);
                            core_state(&run) != core_state(&stepped)
                        });
                        if split.is_some() || seen(&mut run) != seen(&mut stepped) {
                            differ.push(format!(
                                "{name} {addr:#04x} {value:#04x} {first} {runner:?} {split:?}"
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        differ.is_empty(),
        "{} differ:\n{}",
        differ.len(),
        differ.join("\n")
    );
}

/// DS40002061B §7.7.1. Each poke the sweep finds raising an interrupt, one per kind. The next
/// `step` is the four-cycle entry, and the stacked return is where the core sat, so no
/// instruction of the loop ran first. A run to the next cycle ends in the same state.
#[test]
fn a_step_after_a_poke_that_raises_an_interrupt_enters_it_first() {
    for (kind, addr, value, vector) in [
        ("INT0 at low level", EIMSK, 0x01, INT0),
        ("INT1 at low level", EIMSK, 0x02, INT1),
        ("TOV0", TIFR0, 0x01, TIMER0_OVF),
        ("UDRE0 with UDRIE0", UCSR0B, 0x20, USART_UDRE),
        ("EEPROM ready with EERIE", EECR, 0x08, EE_READY),
    ] {
        let mut stepped = Cpu::new();
        stepped.load_words(&awake(0));
        step_until(&mut stepped, 37);
        let (pc, at) = (stepped.pc(), stepped.cycles());
        stepped.poke(addr, value);
        let mut run = stepped.clone();
        stepped.step();
        assert_eq!(
            (
                usize::from(stepped.pc()),
                stepped.cycles(),
                stacked_return(&stepped)
            ),
            (vector, at + 4, pc),
            "{kind}"
        );
        run.run_until(at + 1);
        assert_eq!(
            seen(&mut run),
            seen(&mut stepped),
            "{kind}: run to {}",
            at + 1
        );
    }
}

/// DS40002061B §7.7. A host poke that sets `I` is not `SEI`. `TOV0` and `TOIE0` are already
/// pending and the image is `INC r16`, so the next step is the four-cycle `TIMER0_OVF` entry.
/// The `INC` does not run, and the stacked return is the word the core sat on.
#[test]
fn a_poke_of_sreg_that_sets_i_enters_before_the_instruction() {
    let mut cpu = Cpu::new();
    cpu.load_words(&[inc(16)]);
    cpu.poke(TIMSK0, 0x01);
    cpu.poke(TIFR0, 0x01);
    cpu.poke(SREG, 0x80);
    cpu.step();
    assert_eq!(
        (
            usize::from(cpu.pc()),
            cpu.cycles(),
            cpu.reg(16),
            stacked_return(&cpu)
        ),
        (TIMER0_OVF, 4, 0, 0)
    );
}

/// DS40002061B §7.7. After `RETI` the main program runs one more instruction before a pending
/// interrupt, and that holds for a flag a host poke raised after it. An `EERIE` poke enters
/// `EE_READY`, whose ISR clears `EERIE`. A `UDRIE0` poke after its `RETI` waits for one `INC`.
#[test]
fn a_poke_after_reti_waits_for_one_instruction() {
    let mut main = vec![SEI];
    main.extend([inc(20); 32]);
    let words = image(&main, &[EE_READY], &[out(0, EECR - 0x20), RETI]);
    let mut stepped = Cpu::new();
    stepped.load_words(&words);
    step_until(&mut stepped, 10);
    let counted = stepped.reg(20);
    stepped.poke(EECR, 0x08);
    stepped.step();
    assert_eq!(usize::from(stepped.pc()), EE_READY, "EE_READY entered");
    for _ in 0..3 {
        stepped.step();
    }
    assert_eq!(stepped.reg(20), counted, "back from RETI, no INC yet");
    stepped.poke(UCSR0B, 0x20);
    let (pc, at) = (stepped.pc(), stepped.cycles());
    let mut run = stepped.clone();
    stepped.step();
    assert_eq!(
        (stepped.reg(20), usize::from(stepped.pc()), stepped.cycles()),
        (counted + 1, USART_UDRE, at + 1 + 4),
        "one INC, then USART_UDRE"
    );
    assert_eq!(stacked_return(&stepped), pc + 1, "it returns past that INC");
    run.run_until(at + 1);
    assert_eq!(seen(&mut run), seen(&mut stepped), "a run to {}", at + 1);
}
