//! Model parameter 1. A `TCCR0B` or `GTCCR` store takes effect on the edge
//! that ends its instruction (Figure 14-4, §15.8). Every timer clock up to and
//! including that edge runs with the old select: the count, `TOV0`, the
//! delayed `OCF0A`, and the `OC0A` pin.
//!
//! An instruction that starts at cycle c and takes n cycles ends on edges
//! c + 1 to c + n. A timer poked to /1 before cycle 0 counts on every edge.

use crate::common;
use common::Instructions;
use common::{TCCR0B, TCNT0, TIFR0, ldi, out, sts};
use sim_avr::{Cpu, Level};

const TCCR0A: u16 = 0x0044;
const TCNT1L: u16 = 0x0084;
const TCNT1H: u16 = 0x0085;
const TCCR1B: u16 = 0x0081;
const TIFR1: u16 = 0x0036;
const OCR0A: u16 = 0x0047;
const DDRD: u16 = 0x002A;
const TCCR0A_IO: u16 = 0x24;
const TCCR0B_IO: u16 = 0x25;
const OCR0A_IO: u16 = 0x27;
const GTCCR_IO: u16 = 0x23;
const TOV0: u8 = 1 << 0;
const TOV1: u8 = 1 << 0;
const OCF0A: u8 = 1 << 1;
/// `OC0A` is `PD6`.
const OC0A: u32 = 22;

/// Timer0 in normal mode at /1, `OCR0A` = 5, `OC0A` toggling on the match
/// (Table 15-2). `r16` is 0, so a store of it stops the timer.
fn running(tcnt: u8, words: &[u16]) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.poke(DDRD, 1 << 6);
    cpu.poke(TCCR0A, 0x40);
    cpu.poke(OCR0A, 5);
    cpu.poke(TCNT0, tcnt);
    cpu.poke(TCCR0B, 0x01);
    cpu.load_words(words);
    cpu
}

fn oc0a_highs(cpu: &mut Cpu) -> Vec<u64> {
    std::iter::from_fn(|| cpu.take_pin())
        .filter(|report| report.pin == OC0A && report.level == Level::High)
        .map(|report| report.cycle)
        .collect()
}

/// `OC0A` changes from its first rise. The pin starts Low.
fn oc0a_edges(cpu: &mut Cpu) -> Vec<(Level, u64)> {
    std::iter::from_fn(|| cpu.take_pin())
        .filter(|report| report.pin == OC0A)
        .map(|report| (report.level, report.cycle))
        .skip_while(|&(level, _)| level == Level::Low)
        .collect()
}

fn with_nops(head: &[u16], nops: usize) -> Vec<u16> {
    let mut words = head.to_vec();
    words.extend(std::iter::repeat_n(0x0000, nops));
    words
}

/// The stop store ends at edge 1, which counts 0xFF → 0 and sets `TOV0`
/// (§15.7.1).
#[test]
fn tov_on_the_retiring_edge_of_out() {
    let mut cpu = running(0xFF, &with_nops(&[out(16, TCCR0B_IO)], 2));
    cpu.insns(3);
    assert_eq!(cpu.peek(TCNT0), 0);
    assert_eq!(cpu.peek(TIFR0) & TOV0, TOV0);
}

/// `STS` ends on edges 1 and 2: 0xFE → 0xFF → 0, with `TOV0` on the
/// retiring edge.
#[test]
fn tov_on_the_retiring_edge_of_sts() {
    let mut cpu = running(0xFE, &with_nops(&sts(16, TCCR0B), 2));
    cpu.insns(3);
    assert_eq!(cpu.peek(TCNT0), 0);
    assert_eq!(cpu.peek(TIFR0) & TOV0, TOV0);
}

/// Edge 1, the `NOP`, is the match 4 → 5 and toggles `OC0A`. The stop store
/// ends at edge 2, the next timer clock, which raises `OCF0A` (§15.5).
#[test]
fn ocf_due_on_the_retiring_edge_of_out() {
    let mut cpu = running(4, &with_nops(&[0x0000, out(16, TCCR0B_IO)], 2));
    cpu.insns(4);
    assert_eq!(cpu.peek(TCNT0), 6);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, OCF0A);
    assert_eq!(oc0a_highs(&mut cpu), [1]);
}

/// `STS` ends on the match 4 → 5 at edge 1 and the flag clock 5 → 6 at
/// edge 2.
#[test]
fn ocf_due_on_the_retiring_edge_of_sts() {
    let mut cpu = running(4, &with_nops(&sts(16, TCCR0B), 2));
    cpu.insns(3);
    assert_eq!(cpu.peek(TCNT0), 6);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, OCF0A);
    assert_eq!(oc0a_highs(&mut cpu), [1]);
}

/// The match 4 → 5 is the stop store's retiring edge, so `OC0A` toggles on
/// it. The flag waits for a timer clock the stopped timer never gives.
#[test]
fn oc0a_toggles_on_the_retiring_edge_of_out() {
    let mut cpu = running(4, &with_nops(&[out(16, TCCR0B_IO)], 2));
    cpu.insns(3);
    assert_eq!(cpu.peek(TCNT0), 5);
    assert_eq!(oc0a_highs(&mut cpu), [1]);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, 0);
}

/// `STS` ends on 3 → 4 at edge 1 and the match 4 → 5 at edge 2.
#[test]
fn oc0a_toggles_on_the_retiring_edge_of_sts() {
    let mut cpu = running(3, &with_nops(&sts(16, TCCR0B), 2));
    cpu.insns(3);
    assert_eq!(cpu.peek(TCNT0), 5);
    assert_eq!(oc0a_highs(&mut cpu), [2]);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, 0);
}

/// `OCR0A` = 2 with `OC0A` toggling. The store that starts /1 ends at edge 7,
/// so edge 8 counts 0 → 1 and edge 9 is the match 1 → 2. The ninth word
/// starts at cycle 8 and ends on the match edge. `r18` is /1 and `r19` is /8.
fn rewrite_on_the_match(tail: &[u16]) -> Cpu {
    let head = [
        ldi(16, 0x40),
        out(16, TCCR0A_IO),
        ldi(17, 2),
        out(17, OCR0A_IO),
        ldi(19, 2),
        ldi(18, 1),
        out(18, TCCR0B_IO),
        0x0000,
    ];
    let mut words = head.to_vec();
    words.extend_from_slice(tail);
    words.extend([0x0000; 16]);
    let mut cpu = Cpu::new();
    cpu.poke(DDRD, 1 << 6);
    cpu.load_words(&words);
    cpu
}

/// A `NOP` on the match edge. Edge 10 raises `OCF0A` (§15.5).
#[test]
fn nop_on_the_match_edge() {
    let mut cpu = rewrite_on_the_match(&[0x0000]);
    cpu.run_until(10);
    assert_eq!(cpu.peek(TCNT0), 3);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, OCF0A);
    assert_eq!(oc0a_highs(&mut cpu), [9]);
}

/// Rewriting /1 on the match edge changes nothing: the match and its flag
/// land as with the `NOP`.
#[test]
fn same_prescaler_rewrite_on_the_match_edge() {
    let mut cpu = rewrite_on_the_match(&[out(18, TCCR0B_IO)]);
    cpu.run_until(10);
    assert_eq!(cpu.peek(TCNT0), 3);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, OCF0A);
    assert_eq!(oc0a_highs(&mut cpu), [9]);
}

/// Switching to /8 on the match edge keeps the match at edge 9. The next
/// timer clock is the divider's next /8 count, cycle 16 (§17.2), and the
/// flag rises on it.
#[test]
fn different_prescaler_rewrite_on_the_match_edge() {
    let mut cpu = rewrite_on_the_match(&[out(19, TCCR0B_IO)]);
    cpu.run_until(15);
    assert_eq!(cpu.peek(TCNT0), 2);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, 0);
    assert_eq!(oc0a_highs(&mut cpu), [9]);
    cpu.run_until(16);
    assert_eq!(cpu.peek(TCNT0), 3);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, OCF0A);
}

/// `STS` of /1 starts at cycle 8 and ends on the match at edge 9 and the
/// flag clock at edge 10.
#[test]
fn sts_rewrite_across_the_match_and_the_flag_clock() {
    let mut cpu = rewrite_on_the_match(&sts(18, TCCR0B));
    cpu.run_until(10);
    assert_eq!(cpu.peek(TCNT0), 3);
    assert_eq!(cpu.peek(TIFR0) & OCF0A, OCF0A);
    assert_eq!(oc0a_highs(&mut cpu), [9]);
}

/// Timer0 at /8 from cycle 0. The `PSRSYNC` store starts at cycle 7 and ends
/// at edge 8, the old phase's first count, so 0 → 1 still happens there. The
/// divider restarts on that edge and counts again at edge 16 (§17.2,
/// §17.4.1).
#[test]
fn psrsync_resets_the_divider_at_the_retiring_edge() {
    let mut cpu = Cpu::new();
    cpu.poke(TCCR0B, 0x02);
    let mut words = with_nops(&[ldi(16, 1)], 6);
    words.push(out(16, GTCCR_IO));
    words.extend([0x0000; 16]);
    cpu.load_words(&words);
    cpu.run_until(8);
    assert_eq!(cpu.peek(TCNT0), 1);
    cpu.run_until(15);
    assert_eq!(cpu.peek(TCNT0), 1);
    cpu.run_until(16);
    assert_eq!(cpu.peek(TCNT0), 2);
}

/// Fast PWM, non-inverting, at /1 with `OCR0A` = 10 and 20 in its buffer
/// (Table 15-3, Table 15-8 mode 3). The stop store ends at edge 3, so the
/// stop retiring on 0xFF → 0 copies the buffer, and the stop retiring on
/// 0xFE → 0xFF does not. The second buffer write, 30, waits for the next
/// BOTTOM. The restart ends at edge 7 and the timer counts again from edge 8.
fn stop_then_restart_in_fast_pwm(tcnt: u8) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.poke(DDRD, 1 << 6);
    cpu.poke(TCCR0A, 0x83);
    cpu.poke(OCR0A, 10);
    cpu.poke(TCNT0, tcnt);
    cpu.poke(TCCR0B, 0x01);
    let words = [
        ldi(16, 20),
        out(16, OCR0A_IO),
        out(17, TCCR0B_IO),
        ldi(16, 30),
        out(16, OCR0A_IO),
        ldi(18, 1),
        out(18, TCCR0B_IO),
    ];
    cpu.load_words(&with_nops(&words, 64));
    cpu.run_until(60);
    cpu
}

/// BOTTOM at edge 3 sets `OC0A` and loads `OCR0A` = 20. TCNT0 reaches 20 at
/// edge 27, 20 counts after the restart, and clears it (§15.7.3).
#[test]
fn a_stop_on_bottom_copies_the_buffered_ocr() {
    let mut cpu = stop_then_restart_in_fast_pwm(0xFD);
    assert_eq!(oc0a_edges(&mut cpu), [(Level::High, 3), (Level::Low, 27)]);
}

/// The stop leaves TCNT0 at 0xFF with `OCR0A` = 10. BOTTOM at edge 8 sets
/// `OC0A` and loads 30, and TCNT0 reaches 30 at edge 38.
#[test]
fn a_stop_before_bottom_leaves_the_buffer() {
    let mut cpu = stop_then_restart_in_fast_pwm(0xFC);
    assert_eq!(oc0a_edges(&mut cpu), [(Level::High, 8), (Level::Low, 38)]);
}

/// Timer1 in normal mode at /1 from 0xFFFE. The `STS` stop store ends on
/// edges 1 and 2: 0xFFFE → 0xFFFF → 0, with `TOV1` on the retiring edge
/// (§16.9.1).
#[test]
fn tov1_on_the_retiring_edge_of_sts() {
    let mut cpu = Cpu::new();
    cpu.poke(TCNT1H, 0xFF);
    cpu.poke(TCNT1L, 0xFE);
    cpu.poke(TCCR1B, 0x01);
    cpu.load_words(&with_nops(&sts(16, TCCR1B), 2));
    cpu.insns(3);
    assert_eq!(cpu.peek(TCNT1L), 0);
    assert_eq!(cpu.peek(TIFR1) & TOV1, TOV1);
}
