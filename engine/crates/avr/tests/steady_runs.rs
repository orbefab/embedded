//! Steady loops: `run_until` and `run_to_news` against a loop of single `Cpu::step` calls.
//!
//! A run skips the passes of a loop whose steady loads and idle stores change nothing. The polls
//! here put the event or host call that ends a wait at every offset of a skipped pass, and a run
//! limit at every cycle of a poll. Loops that change a register, flip a flag, or read a register
//! that is not steady must not be skipped. Every state the host can read must match the stepped
//! core at each boundary.

use crate::common::runs::{
    MAIN, NOP, RETI, Runner, SEI, image, inc, matches_stepping, matches_stepping_from, rjmp, seen,
    step_until,
};
use crate::common::{
    UCSR0B, UDR0, brbc, brbs, imm, in_io, ldi, lds, out, rr, sbi, sbic, sbis, sbrc, sbrs, sts,
};
use sim_avr::{Cpu, Level, Port, SupplyChannel};

const TCCR0B: u16 = 0x25;
const TCNT0: u16 = 0x26;
const DDRB: u16 = 0x04;
const PINB: u16 = 0x03;
const TIMSK0: u16 = 0x6E;
const UBRR0L: u16 = 0xC4;
const TIMER0_OVF: usize = 0x20;
const TIFR0: u16 = 0x15;
const PORTB: u16 = 0x05;
const ADCSRA: u16 = 0x7A;
const UCSR0A: u16 = 0xC0;
const FLAG: u16 = 0x0100;

const TOV0: u16 = 0;
const ADSC: u16 = 6;
const RXC0: u16 = 7;
const UDRE0: u16 = 5;

/// Polls of `TOV0` in each shape a compiler emits for `while (!(TIFR0 & 1))`: `IN` and `SBRS`,
/// `LDS` and `SBRS`, and `SBIS`. Each ends in `RJMP` back to the read.
fn tov0_polls(at: usize) -> Vec<Vec<u16>> {
    let mut lds_poll = lds(16, 0x20 + TIFR0).to_vec();
    lds_poll.extend([sbrs(16, TOV0), rjmp(at + 3, at)]);
    vec![
        vec![in_io(16, TIFR0), sbrs(16, TOV0), rjmp(at + 2, at)],
        lds_poll,
        vec![sbis(TIFR0, TOV0), rjmp(at + 1, at)],
    ]
}

/// After the poll: count the pass in r20, clear `TOV0`, and poll again.
fn count_and_clear(main: &mut Vec<u16>, poll: usize) {
    main.extend([inc(20), ldi(17, 1), out(17, TIFR0)]);
    main.push(rjmp(MAIN + main.len(), poll));
}

/// `TOV0` sets at every offset of each poll: `TCNT0` starts at each value, and no poll length
/// divides 256. The skipped passes must end where stepping reads the flag set.
#[test]
fn a_flag_poll_ends_at_the_event_as_stepped() {
    for preset in 0..=255u16 {
        let head = [ldi(16, preset), out(16, TCNT0), ldi(16, 1), out(16, TCCR0B)];
        let at = MAIN + head.len();
        for (shape, poll) in tov0_polls(at).into_iter().enumerate() {
            let mut main = head.to_vec();
            main.extend(poll);
            count_and_clear(&mut main, at);
            let words = image(&main, &[], &[]);
            matches_stepping(
                &words,
                &[97, 300, 777, 1501, 3000],
                &format!("TCNT0 from {preset}, poll {shape}"),
            );
        }
    }
}

/// A run limit at every cycle of a poll ends where stepping ends. Timer0 counts at /1024, so the
/// flag stays clear past every limit.
#[test]
fn a_limit_at_every_cycle_of_a_poll_ends_as_stepped() {
    let head = [ldi(16, 5), out(16, TCCR0B)];
    let at = MAIN + head.len();
    for (shape, poll) in tov0_polls(at).into_iter().enumerate() {
        let mut main = head.to_vec();
        main.extend(poll);
        let words = image(&main, &[], &[]);
        for limit in 1..=90 {
            matches_stepping(
                &words,
                &[limit, limit + 23, limit + 5_000],
                &format!("poll {shape}, limit {limit}"),
            );
        }
    }
}

/// The ISR sets a flag in SRAM that the main loop polls with `LDS`, `AND` and `BREQ`. `TOV0`
/// enters at every offset of the poll, so a skipped pass must not run past the entry.
#[test]
fn an_sram_flag_set_by_an_isr_ends_the_poll_as_stepped() {
    for preset in 0..=255u16 {
        let mut main = vec![ldi(16, preset), out(16, TCNT0), ldi(16, 1)];
        main.extend(sts(16, TIMSK0));
        main.extend([SEI, out(16, TCCR0B)]);
        let poll = MAIN + main.len();
        main.extend(lds(16, FLAG));
        main.extend([rr(0x2000, 16, 16), brbs(1, 0x7C), inc(20), ldi(16, 0)]);
        main.extend(sts(16, FLAG));
        main.push(rjmp(MAIN + main.len(), poll));
        let mut isr = vec![ldi(17, 1)];
        isr.extend(sts(17, FLAG));
        isr.push(RETI);
        let words = image(&main, &[TIMER0_OVF], &isr);
        matches_stepping(
            &words,
            &[97, 300, 777, 1501, 3000],
            &format!("TCNT0 from {preset}"),
        );
    }
}

/// `TCNT0` changes with the cycle, so a poll of it is not skipped: the core must see each value.
#[test]
fn a_poll_of_tcnt0_reads_each_count_as_stepped() {
    for value in (0..=255u16).step_by(7) {
        let mut main = vec![ldi(16, 1), out(16, TCCR0B)];
        let poll = MAIN + main.len();
        main.extend([in_io(16, TCNT0), imm(0x3000, 16, value), brbc(1, 0x7D)]);
        main.extend([inc(20), rjmp(MAIN + main.len() + 1, poll)]);
        let words = image(&main, &[], &[]);
        matches_stepping(&words, &[97, 300, 777, 1501], &format!("TCNT0 to {value}"));
    }
}

/// A loop that reads a steady register but changes a register each pass is never the same state
/// twice, so none of it is skipped.
#[test]
fn a_loop_that_counts_is_not_skipped() {
    let main = [in_io(16, PINB), inc(20), rjmp(MAIN + 2, MAIN)];
    let words = image(&main, &[], &[]);
    for limit in 1..=40 {
        matches_stepping(&words, &[limit, limit + 3_000], &format!("limit {limit}"));
    }
}

/// `firmware/analog_serial.S`'s wait on `ADSC`, after zero to four `NOP`s so the conversion ends
/// at every offset of the 5-cycle poll. The ADC clock is /2.
#[test]
fn an_adsc_poll_ends_at_the_conversion_as_stepped() {
    for shift in 0..=4 {
        let mut main = vec![NOP; shift];
        let start = MAIN + main.len();
        main.push(ldi(16, 0xC1));
        main.extend(sts(16, ADCSRA));
        let poll = MAIN + main.len();
        main.extend(lds(16, ADCSRA));
        main.extend([sbrc(16, ADSC), rjmp(poll + 3, poll), inc(20)]);
        main.push(rjmp(MAIN + main.len(), start));
        let words = image(&main, &[], &[]);
        let boundaries: Vec<u64> = (1..=40).map(|n| n * 37).collect();
        matches_stepping(&words, &boundaries, &format!("shift {shift}"));
    }
}

/// `firmware/analog_serial.S`'s `emit`: wait on `UDRE0`, then store the next byte. At `UBRR0` 0 a
/// frame is 160 cycles, and the wait ends at every offset of its 5-cycle poll.
#[test]
fn a_udre_poll_ends_at_the_shift_as_stepped() {
    for shift in 0..=4 {
        let mut main = vec![ldi(16, 0x08)];
        main.extend(sts(16, UCSR0B));
        main.push(ldi(16, 0));
        main.extend(sts(16, UBRR0L));
        main.extend(vec![NOP; shift]);
        let poll = MAIN + main.len();
        main.extend(lds(18, UCSR0A));
        main.extend([sbrs(18, UDRE0), rjmp(poll + 3, poll), inc(20)]);
        main.extend(sts(20, UDR0));
        main.push(rjmp(MAIN + main.len(), poll));
        let words = image(&main, &[], &[]);
        let boundaries: Vec<u64> = (1..=30).map(|n| n * 53).collect();
        matches_stepping(&words, &boundaries, &format!("shift {shift}"));
    }
}

/// A poll that reads `UDR0` pops the receive FIFO on every pass, so it is not skipped. The host
/// supplies bytes at `UBRR0` 0 and the loop counts them.
#[test]
fn a_poll_that_reads_udr0_is_not_skipped() {
    let mut main = vec![ldi(16, 0x10)];
    main.extend(sts(16, UCSR0B));
    main.push(ldi(16, 0));
    main.extend(sts(16, UBRR0L));
    let poll = MAIN + main.len();
    main.extend(lds(17, UDR0));
    main.extend(lds(16, UCSR0A));
    main.extend([sbrs(16, RXC0), rjmp(poll + 5, poll), inc(20)]);
    main.push(rjmp(MAIN + main.len(), poll));
    let words = image(&main, &[], &[]);
    let load = |cpu: &mut Cpu| {
        cpu.load_words(&words);
        cpu.supply(SupplyChannel::Usart0, &[0x11, 0x22, 0x33, 0x44]);
    };
    let boundaries: Vec<u64> = (1..=30).map(|n| n * 41).collect();
    matches_stepping_from(&load, &boundaries, "UDR0 in the poll");
}

/// `firmware/button.S`'s loop on `PB0`, after it sets the pull-up and makes `PB5` an output.
fn button_loop() -> [u16; 9] {
    [
        sbi(PORTB, 0),
        sbi(DDRB, 5),
        in_io(16, PINB),
        0x9500 | (16 << 4),
        0xFA00 | (16 << 4),
        in_io(17, PORTB),
        0xF800 | (17 << 4) | 5,
        out(17, PORTB),
        rjmp(MAIN + 8, MAIN + 2),
    ]
}

/// `firmware/button.S` on `PB0`: `PB5` copies the inverse of `PINB0`, and the `OUT PORTB` stores
/// what `PORTB` already holds on every pass but the one after a change. The host drives `PB0` at
/// every cycle offset of the 8-cycle pass, from a run boundary, and the synchronizer delays what
/// `PINB` shows. A rise is followed by one run past the delay, and a fall by runs of one cycle.
#[test]
fn a_button_poll_follows_a_host_drive_as_stepped() {
    let words = image(&button_loop(), &[], &[]);
    for runner in [Runner::Until, Runner::ToNews] {
        for first in 200..216 {
            let mut run = Cpu::new();
            let mut stepped = Cpu::new();
            run.load_words(&words);
            stepped.load_words(&words);
            let mut at = first;
            for level in [Level::Low, Level::High, Level::Low, Level::High] {
                runner.run(&mut run, at);
                step_until(&mut stepped, at);
                run.drive_pin(0, level);
                stepped.drive_pin(0, level);
                let near: &[u64] = if level == Level::Low {
                    &[1, 2, 3, 9]
                } else {
                    &[]
                };
                let limits = near.iter().chain(&[17, 300]).map(|gap| at + gap);
                for limit in limits {
                    runner.run(&mut run, limit);
                    step_until(&mut stepped, limit);
                    assert_eq!(
                        seen(&mut run),
                        seen(&mut stepped),
                        "{runner:?}, {level:?} from {at}, to {limit}"
                    );
                }
                at += 300 + first % 8;
            }
        }
    }
}

/// A loop whose registers repeat while `C` alternates takes a 5-cycle pass and a 6-cycle pass in
/// turn, so it is not a loop that changed nothing.
#[test]
fn a_loop_that_flips_a_flag_is_not_skipped() {
    let main = [
        in_io(16, PINB),
        brbs(0, 2),
        0x9408,
        rjmp(MAIN + 3, MAIN),
        0x9488,
        rjmp(MAIN + 5, MAIN),
    ];
    let words = image(&main, &[], &[]);
    for limit in 1..=40 {
        matches_stepping(&words, &[limit, limit + 3_001], &format!("limit {limit}"));
    }
}

/// The SPI master's wait on `SPIF`, then the read of `SPDR` that clears it and the next byte. At
/// /4 a byte is 32 cycles, and zero to four `NOP`s put its end at every offset of the 4-cycle poll.
#[test]
fn an_spif_poll_ends_at_the_byte_as_stepped() {
    const SPCR: u16 = 0x2C;
    const SPSR: u16 = 0x2D;
    const SPDR: u16 = 0x2E;
    for shift in 0..=4 {
        let mut main = vec![ldi(16, 0x2C), out(16, DDRB), ldi(16, 0x50), out(16, SPCR)];
        main.extend(vec![NOP; shift]);
        main.push(out(20, SPDR));
        let poll = MAIN + main.len();
        main.extend([in_io(16, SPSR), sbrs(16, 7), rjmp(poll + 2, poll)]);
        main.extend([in_io(17, SPDR), inc(20), out(20, SPDR)]);
        main.push(rjmp(MAIN + main.len(), poll));
        let words = image(&main, &[], &[]);
        let boundaries: Vec<u64> = (1..=30).map(|n| n * 23).collect();
        matches_stepping(&words, &boundaries, &format!("shift {shift}"));
    }
}

/// A read of `ADCL` locks the data pair until `ADCH` is read (§24.9.3), so a loop that reads
/// only `ADCL` keeps the first result while the free-running ADC converts each new voltage.
#[test]
fn a_poll_of_adcl_keeps_the_locked_result_as_stepped() {
    const ADMUX: u16 = 0x7C;
    let mut main = vec![ldi(16, 0x40)];
    main.extend(sts(16, ADMUX));
    main.push(ldi(16, 0xE1));
    main.extend(sts(16, ADCSRA));
    let poll = MAIN + main.len();
    main.extend(lds(17, 0x78));
    main.push(rjmp(poll + 2, poll));
    let words = image(&main, &[], &[]);
    for runner in [Runner::Until, Runner::ToNews] {
        let mut run = Cpu::new();
        let mut stepped = Cpu::new();
        run.load_words(&words);
        stepped.load_words(&words);
        for (n, volts) in [1.0, 2.0, 3.0, 4.0, 0.5].into_iter().enumerate() {
            run.set_channel_voltage(0, volts);
            stepped.set_channel_voltage(0, volts);
            let limit = 200 * (n as u64 + 1);
            runner.run(&mut run, limit);
            step_until(&mut stepped, limit);
            assert_eq!(seen(&mut run), seen(&mut stepped), "{runner:?} to {limit}");
        }
    }
}

/// The first cycle a stepped run of `words` reaches with `r20` at `count`.
fn stepped_until_count(words: &[u16], count: u8) -> u64 {
    let mut cpu = Cpu::new();
    cpu.load_words(words);
    while cpu.reg(20) != count {
        assert!(cpu.cycles() < 1_000_000, "r20 never reached {count}");
        cpu.step();
    }
    cpu.cycles()
}

/// A limit at every cycle within `span` of the end of the first wait in `words`, which `r20`
/// counts, then one run on past the second wait. Returns the cycle the first wait ends on.
fn every_limit_near_the_first_wait(words: &[u16], span: u64, label: &str) -> u64 {
    let first = stepped_until_count(words, 1);
    let second = stepped_until_count(words, 2);
    for limit in first - span..=first + span {
        matches_stepping(
            words,
            &[limit, second + span],
            &format!("{label}, limit {limit}"),
        );
    }
    first
}

/// `while (EECR & _BV(EEPE));` after an EEPROM write, as `SBIC` and `RJMP` and as `IN`, `SBRC` and
/// `RJMP`. Zero to three `NOP`s between the write and the poll put the end of the write at every
/// offset of the pass.
#[test]
fn an_eepe_poll_ends_at_the_write_as_stepped() {
    const EECR_IO: u16 = 0x1F;
    const EEDR: u16 = 0x20;
    const EEPE: u16 = 1;
    const EEMPE: u16 = 2;
    for (shape, pass) in [(0, 3), (1, 4)] {
        for shift in 0..pass {
            let start = MAIN;
            let mut main = vec![out(20, EEDR), sbi(EECR_IO, EEMPE), sbi(EECR_IO, EEPE)];
            main.extend(vec![NOP; shift]);
            let poll = MAIN + main.len();
            if shape == 0 {
                main.extend([sbic(EECR_IO, EEPE), rjmp(poll + 1, poll)]);
            } else {
                main.extend([in_io(16, EECR_IO), sbrc(16, EEPE), rjmp(poll + 2, poll)]);
            }
            main.push(inc(20));
            main.push(rjmp(MAIN + main.len(), start));
            let words = image(&main, &[], &[]);
            let label = format!("shape {shape}, shift {shift}");
            let first = every_limit_near_the_first_wait(&words, pass as u64 + 1, &label);
            assert!(first > 52_000, "{label}: the write ends at {first}");
        }
    }
}

/// The TWI master's waits on `TWINT` after a START and after SLA+W, then on `TWSTO` after a
/// STOP. At `TWBR` 0 a START is 16 cycles and the address phase 144, and zero to four `NOP`s after
/// each store put the phase's end at every offset of the 5-cycle poll. The pads' own pull-ups
/// hold the bus high, as `Wire.begin` sets them.
#[test]
fn a_twint_poll_ends_at_the_phase_as_stepped() {
    const PORTC_IO: u16 = 0x08;
    const TWDR: u16 = 0xBB;
    const TWCR: u16 = 0xBC;
    const TWINT: u16 = 7;
    const TWSTO: u16 = 4;
    for shift in 0..5 {
        let start = MAIN;
        let mut main = vec![sbi(PORTC_IO, 4), sbi(PORTC_IO, 5)];
        let wait = |main: &mut Vec<u16>, control: u16, bit: u16, set: bool| {
            main.push(ldi(16, control));
            main.extend(sts(16, TWCR));
            main.extend(vec![NOP; shift]);
            let poll = MAIN + main.len();
            main.extend(lds(16, TWCR));
            main.push(if set { sbrs(16, bit) } else { sbrc(16, bit) });
            main.push(rjmp(poll + 3, poll));
        };
        wait(&mut main, 0xA4, TWINT, true);
        main.push(ldi(17, 0xA0));
        main.extend(sts(17, TWDR));
        wait(&mut main, 0x84, TWINT, true);
        wait(&mut main, 0x94, TWSTO, false);
        main.push(inc(20));
        main.push(rjmp(MAIN + main.len(), start));
        let words = image(&main, &[], &[]);
        let first = every_limit_near_the_first_wait(&words, 6, &format!("shift {shift}"));
        assert!(first > 160, "shift {shift}: the waits end at {first}");
        let boundaries: Vec<u64> = (1..=40).map(|n| n * 29).collect();
        matches_stepping(&words, &boundaries, &format!("shift {shift}"));
    }
}

/// Starting at every offset of a poll's pass, applies each of `acts` to both cores `gap` cycles
/// apart. Each act follows a run that skips passes up to it, and is followed by a run of `cut`
/// cycles, which takes every value up to `near`, then a run on to the next act. The loop counts
/// the waits it ends in `r20`, and `ends` is that count after the last act.
fn follows_host(
    load: &dyn Fn(&mut Cpu),
    acts: &[&dyn Fn(&mut Cpu)],
    (pass, near, gap, ends): (u64, u64, u64, u8),
    label: &str,
) {
    for runner in [Runner::Until, Runner::ToNews] {
        for first in 200..200 + pass {
            for cut in 1..=near {
                let mut run = Cpu::new();
                let mut stepped = Cpu::new();
                load(&mut run);
                load(&mut stepped);
                let mut at = first;
                for (n, act) in acts.iter().enumerate() {
                    for limit in [at, at + cut] {
                        runner.run(&mut run, limit);
                        step_until(&mut stepped, limit);
                        assert_eq!(
                            seen(&mut run),
                            seen(&mut stepped),
                            "{label}, {runner:?}, act {n} from {at}, to {limit}"
                        );
                        if limit == at {
                            act(&mut run);
                            act(&mut stepped);
                        }
                    }
                    at += gap;
                }
                runner.run(&mut run, at);
                assert_eq!(run.reg(20), ends, "{label}, {runner:?}: waits ended");
            }
        }
    }
}

/// `while (!(EIFR & _BV(INTF0)));` as `SBIS` and `RJMP` and as `IN`, `SBRS` and `RJMP`, then a
/// clear and the next wait. `INT0` senses a falling edge, and the host drives `PD2` high and low
/// in turn, so the flag rises two cycles after every second drive.
#[test]
fn an_intf0_poll_follows_a_host_drive_as_stepped() {
    const EIFR: u16 = 0x1C;
    const EICRA: u16 = 0x69;
    const PD2: u32 = 18;
    for (shape, pass) in [(0, 3), (1, 4)] {
        let mut main = vec![ldi(16, 0x02)];
        main.extend(sts(16, EICRA));
        let poll = MAIN + main.len();
        if shape == 0 {
            main.extend([sbis(EIFR, 0), rjmp(poll + 1, poll)]);
        } else {
            main.extend([in_io(16, EIFR), sbrs(16, 0), rjmp(poll + 2, poll)]);
        }
        main.extend([ldi(16, 1), out(16, EIFR), inc(20)]);
        main.push(rjmp(MAIN + main.len(), poll));
        let words = image(&main, &[], &[]);
        let high = |cpu: &mut Cpu| cpu.drive_pin(PD2, Level::High);
        let low = |cpu: &mut Cpu| cpu.drive_pin(PD2, Level::Low);
        follows_host(
            &|cpu| cpu.load_words(&words),
            &[&high, &low, &high, &low],
            (pass, 8, 301, 2),
            &format!("shape {shape}"),
        );
    }
}

/// `while (!(ACSR & _BV(ACI)));` as `IN`, `SBRS` and `RJMP`, then a clear and the next wait. The comparator's interrupt mode
/// is toggle, and the host moves `AIN0` across `AIN1`, so `ACI` rises two cycles after each move.
#[test]
fn an_aci_poll_follows_a_host_voltage_as_stepped() {
    const ACSR: u16 = 0x30;
    const AIN0: u32 = 22;
    const AIN1: u32 = 23;
    let poll = MAIN;
    let mut main = vec![in_io(16, ACSR), sbrs(16, 4), rjmp(poll + 2, poll)];
    main.extend([ldi(16, 0x10), out(16, ACSR), inc(20)]);
    main.push(rjmp(MAIN + main.len(), poll));
    let words = image(&main, &[], &[]);
    let load = |cpu: &mut Cpu| {
        cpu.load_words(&words);
        cpu.set_voltage(AIN1, 1.0);
    };
    let above = |cpu: &mut Cpu| cpu.set_voltage(AIN0, 3.0);
    let below = |cpu: &mut Cpu| cpu.set_voltage(AIN0, 0.0);
    follows_host(
        &load,
        &[&above, &below, &above, &below],
        (4, 8, 301, 4),
        "ACI",
    );
}

/// `SEI`, and each `RETI`, is followed by a steady `IN`, so the run after it starts on a steady
/// load with the one-instruction delay still to clear. `TCNT0` starts at every value, so the
/// overflow that ends such a run lands at every offset of the poll and must enter as stepped.
#[test]
fn a_steady_load_after_sei_or_reti_enters_as_stepped() {
    for preset in 0..=255u16 {
        let mut main = vec![ldi(16, preset), out(16, TCNT0), ldi(16, 1)];
        main.extend(sts(16, TIMSK0));
        main.extend([out(16, TCCR0B), SEI]);
        let poll = MAIN + main.len();
        main.extend([in_io(17, PINB), rjmp(poll + 1, poll)]);
        let words = image(&main, &[TIMER0_OVF], &[inc(29), RETI]);
        let label = format!("TCNT0 from {preset}");
        matches_stepping(&words, &[600, 1_400], &label);
        let mut cpu = Cpu::new();
        cpu.load_words(&words);
        step_until(&mut cpu, 1_400);
        assert!(
            cpu.reg(29) >= 4,
            "{label}: the overflow entered {} times",
            cpu.reg(29)
        );
    }
}

/// `PC7` is in no pin-change group, so a drive of it queues no `ExtInt` sample and no event. Only
/// the `PINx` settle guard keeps a run from reading `PINC` before the synchronizer shows the drive.
#[test]
fn a_poll_of_a_pin_no_group_watches_follows_a_host_drive_as_stepped() {
    const PINC: u16 = 0x06;
    let pc7 = Port::C.pin_id(7);
    let poll = MAIN;
    let main = [
        sbis(PINC, 7),
        rjmp(poll + 1, poll),
        inc(20),
        sbic(PINC, 7),
        rjmp(poll + 4, poll + 3),
        inc(20),
        rjmp(poll + 6, poll),
    ];
    let words = image(&main, &[], &[]);
    let high = |cpu: &mut Cpu| cpu.drive_pin(pc7, Level::High);
    let low = |cpu: &mut Cpu| cpu.drive_pin(pc7, Level::Low);
    follows_host(
        &|cpu| cpu.load_words(&words),
        &[&high, &low, &high, &low],
        (3, 8, 301, 4),
        "PC7",
    );
}
