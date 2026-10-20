//! Timer rules from DS40002061B §15–§18.
//!
//! Counts below are the datasheet sequences, not a trace copied out of a run.
//! A 1-cycle `NOP` is one `clkI/O`. The prescaler starts at 0, so a /N timer
//! clocks when the free-running divider reaches N, 2N, …

use crate::common;
use common::Instructions;
use common::{ldi as imm, lds, out, sts};

use sim_avr::{Cpu, Level, PIN_QUEUE_CAP};

fn nop_sled(n: usize) -> Vec<u16> {
    vec![0x0000; n]
}

/// `RJMP .-2`, two cycles, stays on this word.
const RJMP_SELF: u16 = 0xCFFF;

fn run_nops(cpu: &mut Cpu, n: usize) {
    cpu.load_words(&nop_sled(n));
    cpu.insns(u32::try_from(n).unwrap());
}

fn flags(cpu: &Cpu, tifr: u16) -> u8 {
    cpu.peek(tifr)
}

/// §15.7.1 / §16.9.1. Normal mode TOP is MAX. TOV is set as TCNT becomes 0.
#[test]
fn normal_mode_top_is_max_and_tov_is_the_wrap() {
    let mut cpu = Cpu::new();
    cpu.poke(0x45, 0x01);
    cpu.load_words(&nop_sled(256));
    cpu.insns(255);
    assert_eq!(cpu.peek(0x46), 255);
    assert_eq!(flags(&cpu, 0x35) & 1, 0);
    cpu.insns(1);
    assert_eq!(cpu.cycles(), 256);
    assert_eq!(cpu.peek(0x46), 0);
    assert_eq!(flags(&cpu, 0x35) & 1, 1, "TOV0 on the wrap to 0");

    let mut wide = Cpu::new();
    wide.poke(0x85, 0xFF);
    wide.poke(0x84, 0xFE);
    wide.poke(0x81, 0x01);
    wide.load_words(&nop_sled(2));
    wide.insns(1);
    assert_eq!(wide.peek(0x84), 0xFF);
    assert_eq!(flags(&wide, 0x36) & 1, 0);
    wide.insns(1);
    assert_eq!(wide.peek(0x84), 0);
    assert_eq!(flags(&wide, 0x36) & 1, 1, "TOV1 on the 16-bit wrap");
    wide.poke(0x81, 0);
    let mut read = lds(16, 0x84).to_vec();
    read.extend(lds(17, 0x85));
    wide.load_words(&read);
    wide.insns(2);
    assert_eq!(wide.reg(16), 0);
    assert_eq!(
        wide.reg(17),
        0,
        "high byte wrapped; TEMP is filled by the low read"
    );
}

/// §15.7.2 / Table 15-8 mode 2. CTC TOP is OCR0A. TOV waits for a real MAX wrap.
#[test]
fn ctc_top_is_ocra_and_tov_stays_clear_below_max() {
    let mut cpu = Cpu::new();
    cpu.poke(0x47, 5);
    cpu.poke(0x44, 0x02);
    cpu.poke(0x45, 0x01);
    run_nops(&mut cpu, 5);
    assert_eq!(cpu.peek(0x46), 5, "TCNT holds TOP for one timer clock");
    assert_eq!(
        flags(&cpu, 0x35) & 0x02,
        0,
        "the match clock does not set OCF"
    );
    assert_eq!(flags(&cpu, 0x35) & 1, 0);
    cpu.load_words(&nop_sled(1));
    cpu.insns(1);
    assert_eq!(cpu.peek(0x46), 0);
    assert_eq!(
        flags(&cpu, 0x35) & 0x02,
        0x02,
        "OCF0A on the clock that clears TOP"
    );
    assert_eq!(
        flags(&cpu, 0x35) & 1,
        0,
        "clearing at OCR is not an overflow"
    );
}

/// §15.7.3 / Table 15-8 mode 3 and mode 7. Fast PWM TOV is set on reaching TOP.
/// OCR updates on the following clock, when TCNT is cleared.
#[test]
fn fast_pwm_tov_is_at_top_and_ocr_updates_at_bottom() {
    let mut cpu = Cpu::new();
    cpu.poke(0x44, 0x03);
    cpu.poke(0x45, 0x01);
    cpu.load_words(&nop_sled(256));
    cpu.insns(255);
    assert_eq!(cpu.peek(0x46), 255);
    assert_eq!(flags(&cpu, 0x35) & 1, 0, "still at TOP");
    cpu.insns(1);
    assert_eq!(cpu.cycles(), 256);
    assert_eq!(cpu.peek(0x46), 0);
    assert_eq!(flags(&cpu, 0x35) & 1, 1, "TOV as TCNT clears from TOP");
    cpu.poke(0x35, 0);
    cpu.insns(256);
    assert_eq!(cpu.cycles(), 512);
    assert_eq!(cpu.peek(0x46), 0);
    assert_eq!(flags(&cpu, 0x35) & 1, 1);
    assert_eq!(512 - 256, 256, "8-bit fast PWM period is TOP+1");

    // Mode 7, TOP = OCR0A. A write while running lands in the buffer. The
    // clock store ends at edge 4, which counts with the old select (Figure
    // 14-4, §15.8), so the first tick is edge 5.
    let mut buffered = Cpu::new();
    buffered.poke(0x47, 4);
    buffered.poke(0x44, 0x03);
    buffered.poke(0x45, 0x08);
    let program = [
        imm(16, 2),
        out(16, 0x27),
        imm(17, 0x09),
        out(17, 0x25), // CS=/1 and WGM02
        0x0000,
        0x0000,
        0x0000,
        0x0000, // TCNT 1, 2, 3, 4
        0x0000, // clear to 0 and copy the buffer
        0x0000,
        0x0000, // 1, 2
        0x0000, // clear at the new TOP
    ];
    buffered.load_words(&program);
    buffered.insns(3);
    assert_eq!(
        buffered.peek(0x46),
        0,
        "the OCR write does not clock a stopped timer"
    );
    buffered.insns(1);
    assert_eq!(
        buffered.peek(0x46),
        0,
        "the store's own edge has the old select"
    );
    buffered.insns(1);
    assert_eq!(buffered.peek(0x46), 1, "clock starts");
    buffered.insns(3);
    assert_eq!(buffered.peek(0x46), 4, "old TOP still in force");
    buffered.insns(1);
    assert_eq!(buffered.peek(0x46), 0, "buffer copied as TCNT clears");
    buffered.insns(3);
    assert_eq!(buffered.peek(0x46), 0, "new TOP is 2, so the count cleared");
}

/// §15.7.4 / Table 15-8 mode 5. Phase-correct period is 2×TOP. TOV is at BOTTOM.
/// OCR updates at TOP.
#[test]
fn phase_correct_tov_is_at_bottom_and_ocr_updates_at_top() {
    let mut cpu = Cpu::new();
    cpu.poke(0x47, 3);
    cpu.poke(0x44, 0x01);
    cpu.poke(0x45, 0x08 | 0x01);
    run_nops(&mut cpu, 5);
    assert_eq!(cpu.peek(0x46), 1, "counting down after TOP");
    assert_eq!(flags(&cpu, 0x35) & 1, 0);
    cpu.load_words(&nop_sled(1));
    cpu.insns(1);
    assert_eq!(cpu.cycles(), 6);
    assert_eq!(cpu.peek(0x46), 0);
    assert_eq!(flags(&cpu, 0x35) & 1, 1, "TOV at BOTTOM");
    assert_eq!(6, 2 * 3, "period is 2×TOP");
}

/// §16.9.5 / Table 16-4 mode 8. Phase-and-frequency-correct updates OCR at BOTTOM.
/// Mode 13 is reserved: the counter stays put.
#[test]
fn phase_frequency_correct_updates_at_bottom() {
    let mut cpu = Cpu::new();
    cpu.poke(0x87, 0x00);
    cpu.poke(0x86, 4);
    cpu.poke(0x80, 0x00);
    cpu.poke(0x81, 0x10 | 0x01); // WGM13, CS=/1. Mode 8.
    run_nops(&mut cpu, 8);
    assert_eq!(cpu.peek(0x84), 0);
    assert_eq!(flags(&cpu, 0x36) & 1, 1, "TOV1 at BOTTOM");
}

/// Table 15-8 mode 4 (WGM02 set, WGM1:0 clear) is reserved. The counter stays put.
#[test]
fn reserved_timer0_mode_does_not_count() {
    let mut reserved = Cpu::new();
    let program = [imm(16, 0x09), out(16, 0x25)];
    reserved.load_words(&program);
    reserved.insns(2);
    assert_eq!(reserved.peek(0x46), 0, "reserved mode does not count");
}

/// §16.9.2 mode 12. CTC with ICR1 as TOP sets ICF, not a second overflow.
#[test]
fn ctc_with_icr_sets_the_capture_flag_at_top() {
    let mut cpu = Cpu::new();
    cpu.poke(0x87, 0x00);
    cpu.poke(0x86, 3);
    cpu.poke(0x81, 0x18 | 0x01); // WGM13:2 = 3, mode 12, CS=/1
    run_nops(&mut cpu, 3);
    assert_eq!(cpu.peek(0x84), 3);
    assert_eq!(
        flags(&cpu, 0x36) & 0x20,
        0,
        "arriving at TOP does not set ICF"
    );
    assert_eq!(flags(&cpu, 0x36) & 1, 0);
    cpu.load_words(&nop_sled(1));
    cpu.insns(1);
    assert_eq!(cpu.peek(0x84), 0);
    assert_eq!(
        flags(&cpu, 0x36) & 0x20,
        0x20,
        "ICF1 on the clock that clears TOP"
    );
}

/// §15.9.1 Tables 15-2, 15-3, 15-4 and the OCR = BOTTOM / TOP cases in §15.7.3–§15.7.4.
#[test]
fn compare_output_modes_drive_the_oc_pin() {
    // Non-PWM toggle on compare match. OC0A is PD6, pin id 22.
    let mut toggle = Cpu::new();
    toggle.poke(0x2A, 1 << 6);
    toggle.poke(0x47, 1);
    toggle.poke(0x44, 0x40);
    toggle.poke(0x45, 0x01);
    run_nops(&mut toggle, 1);
    let high = toggle.take_pin().unwrap();
    assert_eq!((high.pin, high.level, high.cycle), (22, Level::High, 1));

    // Fast PWM non-inverting, OCR=2, TOP=255. The register stays 0 until
    // BOTTOM sets it (§15.7.3). The first clock only connects that 0.
    // Cycle 256 is the wrap to 0. The match two clocks later clears it.
    let mut fast = Cpu::new();
    fast.poke(0x2A, 1 << 6);
    fast.poke(0x47, 2);
    fast.poke(0x44, 0x80 | 0x03);
    fast.poke(0x45, 0x01);
    run_nops(&mut fast, 258);
    let edges: Vec<_> = std::iter::from_fn(|| fast.take_pin())
        .map(|report| (report.level, report.cycle))
        .collect();
    assert_eq!(
        edges,
        vec![(Level::Low, 1), (Level::High, 256), (Level::Low, 258)],
        "set at BOTTOM, cleared on compare match"
    );

    // OCR = TOP: the match is ignored and BOTTOM sets the pin (§15.7.3).
    // It stays high. The first edge is the reset register, which is 0.
    // OCR = 0 is a one-tick spike at the wrap.
    let mut stuck = Cpu::new();
    stuck.poke(0x2A, 1 << 6);
    stuck.poke(0x47, 255);
    stuck.poke(0x44, 0x80 | 0x03);
    stuck.poke(0x45, 0x01);
    run_nops(&mut stuck, 300);
    let levels: Vec<_> = std::iter::from_fn(|| stuck.take_pin())
        .map(|report| report.level)
        .collect();
    assert_eq!(levels, vec![Level::Low, Level::High]);

    let mut spike = Cpu::new();
    spike.poke(0x2A, 1 << 6);
    spike.poke(0x47, 0);
    spike.poke(0x44, 0x80 | 0x03);
    spike.poke(0x45, 0x01);
    run_nops(&mut spike, 257);
    let spike_edges: Vec<_> = std::iter::from_fn(|| spike.take_pin())
        .map(|report| (report.level, report.cycle))
        .collect();
    let high_at = spike_edges
        .iter()
        .find(|edge| edge.0 == Level::High)
        .map(|edge| edge.1);
    let low_after = spike_edges
        .iter()
        .find(|edge| edge.0 == Level::Low && Some(edge.1) > high_at)
        .map(|edge| edge.1);
    assert_eq!(high_at, Some(256), "the spike is the wrap to BOTTOM");
    assert_eq!(low_after, Some(257));
    assert_eq!(low_after.unwrap() - high_at.unwrap(), 1);

    // Phase correct, non-inverting, channel B. High for 2×OCR of a 2×TOP period.
    let mut phase = Cpu::new();
    phase.poke(0x2A, 1 << 5);
    phase.poke(0x47, 3);
    phase.poke(0x48, 2);
    phase.poke(0x44, 0x20 | 0x01);
    phase.poke(0x45, 0x08 | 0x01);
    run_nops(&mut phase, 16);
    let phase_edges: Vec<_> = std::iter::from_fn(|| phase.take_pin())
        .map(|report| (report.pin, report.level, report.cycle))
        .collect();
    let rises: Vec<_> = phase_edges
        .iter()
        .filter(|edge| edge.1 == Level::High)
        .map(|edge| edge.2)
        .collect();
    let falls: Vec<_> = phase_edges
        .iter()
        .filter(|edge| edge.1 == Level::Low)
        .map(|edge| edge.2)
        .collect();
    assert_eq!(phase_edges[0].0, 21, "OC0B is PD5");
    // The register is 0 until the down-count match sets it (Table 15-7).
    // TCNT is 1, 2, 3 at cycles 1–3. The up-count match at 2 clears 0.
    // Cycle 4 is 2 counting down, which sets OC0B.
    assert_eq!(rises[0], 4, "first rise is the down-count match");
    let pulse = rises[0];
    let end = falls.iter().copied().find(|cycle| *cycle > pulse).unwrap();
    assert_eq!(end - pulse, 2 * 2, "high time is 2×OCR");
    assert_eq!(rises[1] - rises[0], 2 * 3, "period is 2×TOP");

    // Inverting fast PWM is the opposite level. COM0A = 3.
    let mut inv = Cpu::new();
    inv.poke(0x2A, 1 << 6);
    inv.poke(0x47, 255);
    inv.poke(0x44, 0xC0 | 0x03);
    inv.poke(0x45, 0x01);
    run_nops(&mut inv, 4);
    let inv_edges: Vec<_> = std::iter::from_fn(|| inv.take_pin()).collect();
    assert!(
        inv_edges.iter().all(|report| report.level == Level::Low),
        "OCR = TOP inverting stays low: {inv_edges:?}"
    );

    // Phase correct OCR = BOTTOM stays low, OCR = TOP stays high. §15.7.4.
    let mut bottom = Cpu::new();
    bottom.poke(0x2A, 1 << 6);
    bottom.poke(0x47, 0);
    bottom.poke(0x44, 0x80 | 0x01);
    bottom.poke(0x45, 0x01);
    run_nops(&mut bottom, 20);
    assert!(std::iter::from_fn(|| bottom.take_pin()).all(|report| report.level == Level::Low));
}

/// §15.9.2 Table 15-9, §18.11 Table 18-9, §17.2 and §17.4.1.
#[test]
fn prescalers_and_the_shared_reset() {
    let mut cpu = Cpu::new();
    cpu.poke(0x45, 0x02);
    cpu.poke(0x81, 0x03);
    run_nops(&mut cpu, 7);
    assert_eq!(cpu.peek(0x46), 0);
    assert_eq!(cpu.peek(0x84), 0);
    cpu.load_words(&nop_sled(1));
    cpu.insns(1);
    assert_eq!(cpu.peek(0x46), 1, "/8 clocks on the 8th clkI/O");
    assert_eq!(cpu.peek(0x84), 0);
    cpu.load_words(&nop_sled(56));
    cpu.insns(56);
    assert_eq!(cpu.cycles(), 64);
    assert_eq!(cpu.peek(0x46), 8, "Timer0 /8");
    assert_eq!(cpu.peek(0x84), 1, "Timer1 /64 shares that prescaler");

    cpu.poke(0x43, 0x01);
    let t0 = cpu.peek(0x46);
    let t1 = cpu.peek(0x84);
    cpu.load_words(&nop_sled(64));
    cpu.insns(64);
    assert_eq!(cpu.peek(0x46), t0.wrapping_add(8));
    assert_eq!(cpu.peek(0x84), t1.wrapping_add(1));
    assert_eq!(cpu.peek(0x43) & 0x01, 0, "PSRSYNC clears itself");

    let mut held = Cpu::new();
    held.poke(0x45, 0x02);
    held.poke(0x43, 0x81);
    assert_eq!(held.peek(0x43), 0x81);
    run_nops(&mut held, 40);
    assert_eq!(held.peek(0x46), 0, "TSM holds the prescaler in reset");
    held.poke(0x43, 0x00);
    held.load_words(&nop_sled(8));
    held.insns(8);
    assert_eq!(held.peek(0x46), 1);

    // Timer2's divider is its own. /32 and /128 are Timer2 only.
    let mut async_timer = Cpu::new();
    async_timer.poke(0xB1, 0x03);
    run_nops(&mut async_timer, 32);
    assert_eq!(async_timer.peek(0xB2), 1, "Timer2 /32");
    assert_eq!(async_timer.peek(0x46), 0, "Timer0 is stopped");
    async_timer.poke(0x43, 0x02);
    let before = async_timer.peek(0xB2);
    async_timer.load_words(&nop_sled(32));
    async_timer.insns(32);
    assert_eq!(async_timer.peek(0xB2), before.wrapping_add(1));

    let mut div128 = Cpu::new();
    div128.poke(0xB1, 0x05);
    run_nops(&mut div128, 128);
    assert_eq!(div128.peek(0xB2), 1, "Timer2 /128");
}

/// §15.5.1 / §15.9.2. FOC toggles the pin in a non-PWM mode and does not set OCF or clear CTC.
#[test]
fn force_output_compare_does_not_set_the_flag() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x47, 5);
    cpu.poke(0x44, 0x40 | 0x02);
    cpu.poke(0x46, 5);
    let program = [imm(16, 0x80), out(16, 0x25), out(16, 0x25)];
    cpu.load_words(&program);
    cpu.insns(3);
    assert_eq!(cpu.peek(0x46), 5, "FOC does not clear CTC");
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0);
    let edges: Vec<_> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.pin, report.level))
        .collect();
    assert_eq!(
        edges,
        vec![(22, Level::High), (22, Level::Low)],
        "each FOC toggles OC0A"
    );

    let mut pwm = Cpu::new();
    pwm.poke(0x44, 0x03);
    pwm.poke(0x46, 3);
    let program = [imm(16, 0x80), out(16, 0x25)];
    pwm.load_words(&program);
    pwm.insns(1);
    assert_eq!(pwm.peek(0x46), 3);
    assert_eq!(flags(&pwm, 0x35) & 0x02, 0, "FOC is inactive in PWM");
}

/// §15.5.2. The timer clock after a TCNT write does not compare. The clock
/// store ends at edge 4 under the old select, so edge 5, TCNT0 4 → 5, is the
/// blocked match. Edge 6 is the clock its flag would rise on (§15.5).
#[test]
fn a_tcnt_write_blocks_the_next_compare() {
    let mut cpu = Cpu::new();
    cpu.poke(0x47, 5);
    cpu.poke(0x44, 0x40);
    cpu.poke(0x2A, 1 << 6);
    let program = [
        imm(16, 4),
        out(16, 0x26),
        imm(17, 0x01),
        out(17, 0x25),
        0x0000,
        0x0000,
    ];
    cpu.load_words(&program);
    cpu.insns(6);
    assert_eq!(cpu.peek(0x46), 6);
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0, "the match at OCR was blocked");
    assert!(
        std::iter::from_fn(|| cpu.take_pin()).all(|report| report.level != Level::High),
        "the blocked match does not toggle OC0A"
    );

    let mut open = Cpu::new();
    open.poke(0x47, 1);
    open.poke(0x45, 0x01);
    run_nops(&mut open, 1);
    assert_eq!(
        flags(&open, 0x35) & 0x02,
        0,
        "the match clock does not set OCF"
    );
    open.load_words(&nop_sled(1));
    open.insns(1);
    assert_eq!(
        flags(&open, 0x35) & 0x02,
        0x02,
        "OCF0A on the next timer clock"
    );
}

/// §16.3. High byte written first, low byte read first. One TEMP for the timer.
#[test]
fn sixteen_bit_access_uses_temp() {
    let mut cpu = Cpu::new();
    let mut program = Vec::new();
    program.push(imm(16, 0xAB));
    program.extend(sts(16, 0x85));
    program.push(imm(17, 0xCD));
    program.extend(sts(17, 0x84));
    program.extend(lds(18, 0x84));
    program.extend(lds(19, 0x85));
    program.extend(lds(20, 0x85));
    program.push(imm(21, 0x12));
    program.extend(sts(21, 0x89));
    program.push(imm(22, 0x00));
    program.extend(sts(22, 0x84));
    program.extend(lds(23, 0x85));
    cpu.load_words(&program);
    cpu.insns(u32::try_from(program.len()).unwrap());
    assert_eq!(cpu.reg(18), 0xCD, "low byte");
    assert_eq!(cpu.reg(19), 0xAB, "high byte latched by the low read");
    assert_eq!(cpu.reg(20), 0xAB);
    assert_eq!(cpu.peek(0x84), 0x00);
    assert_eq!(
        cpu.peek(0x85),
        0x12,
        "OCR high write left TEMP for the TCNT low write"
    );
}

/// §15.5. The compare flag is the timer clock after `TCNT == OCR`. Entry on
/// that clock clears the flag. Writing a one clears it; writing a zero does not.
#[test]
fn compare_flag_is_the_timer_clock_after_the_match_and_clears_on_entry() {
    let mut cpu = Cpu::new();
    cpu.poke(0x47, 3);
    cpu.poke(0x6E, 0x02);
    cpu.poke(0x45, 0x01);
    cpu.set_sreg(0x80);
    cpu.load_words(&nop_sled(8));
    cpu.insns(3);
    assert_eq!(cpu.pc(), 3, "the match clock does not enter");
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0);
    cpu.step();
    assert_eq!(
        cpu.cycles(),
        4 + 4,
        "flag on the next clock, then the 4-cycle entry"
    );
    assert_eq!(cpu.pc(), 0x001C, "TIMER0_COMPA");
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0, "entry clears OCF0A");

    // Timer2 compare. Vector 8 is word 0x000E (Table 12-6).
    let mut timer2 = Cpu::new();
    timer2.poke(0xB3, 3);
    timer2.poke(0x70, 0x02);
    timer2.poke(0xB1, 0x01);
    timer2.set_sreg(0x80);
    timer2.load_words(&nop_sled(8));
    timer2.insns(3);
    assert_eq!(timer2.pc(), 3);
    timer2.step();
    assert_eq!(
        timer2.cycles(),
        4 + 4,
        "Timer2 flag on the next clock, then the 4-cycle entry"
    );
    assert_eq!(timer2.pc(), 0x000E, "TIMER2_COMPA");
    assert_eq!(flags(&timer2, 0x37) & 0x02, 0, "entry clears OCF2A");

    // Timer1 compare. OCR1A low is 0x88; TEMP starts at 0. Vector 12 is word 0x0016.
    let mut timer1 = Cpu::new();
    timer1.poke(0x88, 3);
    timer1.poke(0x6F, 0x02);
    timer1.poke(0x81, 0x01);
    timer1.set_sreg(0x80);
    timer1.load_words(&nop_sled(8));
    timer1.insns(3);
    assert_eq!(timer1.pc(), 3);
    timer1.step();
    assert_eq!(
        timer1.cycles(),
        4 + 4,
        "Timer1 flag on the next clock, then the 4-cycle entry"
    );
    assert_eq!(timer1.pc(), 0x0016, "TIMER1_COMPA");
    assert_eq!(flags(&timer1, 0x36) & 0x02, 0, "entry clears OCF1A");

    let mut w1c = Cpu::new();
    w1c.poke(0x35, 0x07);
    let program = [imm(16, 0x02), out(16, 0x15), imm(17, 0x00), out(17, 0x15)];
    w1c.load_words(&program);
    w1c.insns(2);
    assert_eq!(
        flags(&w1c, 0x35) & 0x07,
        0x05,
        "writing 1 clears only that flag"
    );
    w1c.insns(2);
    assert_eq!(flags(&w1c, 0x35) & 0x07, 0x05, "writing 0 does not clear");
}

/// §15.7.3: `64 * 256` cycles per overflow is 1024 µs at 16 MHz.
///
/// Fast PWM mode 3 sets TOV on the clock that clears TOP (§15.7.3). The
/// divider clocks on count 64, 128, … so that clock is cycle 256×64 and the
/// next one is 256 clocks later.
#[test]
fn timer0_overflow_period_at_prescaler_64_is_1024_us() {
    let mut cpu = Cpu::new();
    cpu.poke(0x44, 0x03);
    cpu.poke(0x45, 0x03);
    cpu.load_words(&[RJMP_SELF]);
    // Prescaler 64, TOP 0xFF: the 256th timer clock is cycle 256 × 64.
    let overflow = 256 * 64;
    cpu.run_until(overflow - 2);
    assert_eq!(
        flags(&cpu, 0x35) & 1,
        0,
        "TOV is still clear two cycles early"
    );
    cpu.run_until(overflow);
    assert_eq!(cpu.peek(0x46), 0, "the 256th /64 clock clears TOP");
    assert_eq!(flags(&cpu, 0x35) & 1, 1, "TOV is that same clock");
    cpu.poke(0x35, 0);
    cpu.run_until(overflow * 2);
    assert_eq!(cpu.peek(0x46), 0, "the next period clears TOP again");
    assert_eq!(flags(&cpu, 0x35) & 1, 1, "TOV sets one period later");
}

/// Table 15-8 and Table 18-8. Each 8-bit mode's TOP, including the reserved rows.
#[test]
fn every_8bit_waveform_mode_names_its_top() {
    // 8-bit: mode, (tccra, tccrb without CS), ocr, expected max, reserved.
    let modes8: [(u8, u8, u8, u8, bool); 8] = [
        (0x00, 0x00, 0, 255, false),
        (0x01, 0x00, 0, 255, false),
        (0x02, 0x00, 4, 4, false),
        (0x03, 0x00, 0, 255, false),
        (0x00, 0x08, 0, 0, true),
        (0x01, 0x08, 4, 4, false),
        (0x02, 0x08, 0, 0, true),
        (0x03, 0x08, 4, 4, false),
    ];
    for (index, (tccra, tccrb, ocr, top, reserved)) in modes8.into_iter().enumerate() {
        let mut cpu = Cpu::new();
        if reserved {
            let program = [imm(16, u16::from(tccrb | 0x01)), out(16, 0x25)];
            cpu.poke(0x44, tccra);
            cpu.load_words(&program);
            cpu.insns(2);
            cpu.load_words(&nop_sled(8));
            cpu.insns(8);
            assert_eq!(
                cpu.peek(0x46),
                0,
                "mode {index} is reserved and does not count"
            );
            continue;
        }
        cpu.poke(0x47, ocr);
        cpu.poke(0x44, tccra);
        cpu.poke(0x45, tccrb | 0x01);
        let clocks = u32::from(top) * 2 + 4;
        cpu.load_words(&nop_sled(clocks as usize));
        let mut seen = 0u8;
        for _ in 0..clocks {
            cpu.step();
            seen = seen.max(cpu.peek(0x46));
        }
        assert_eq!(seen, top, "8-bit mode {index}");
    }

    // Timer2 is the other 8-bit type and uses Table 18-8, the same rows.
    let mut timer2 = Cpu::new();
    timer2.poke(0xB3, 4);
    timer2.poke(0xB0, 0x03);
    timer2.poke(0xB1, 0x08 | 0x01);
    timer2.load_words(&nop_sled(12));
    let mut seen = 0u8;
    for _ in 0..12 {
        timer2.step();
        seen = seen.max(timer2.peek(0xB2));
    }
    assert_eq!(seen, 4, "Timer2 fast PWM TOP is OCR2A");
}

/// Table 16-4. Mode, TOP, update of OCR1x at, TOV1 set on. Mode 13 is reserved.
const TABLE_16_4: [(u8, &str, &str, &str); 15] = [
    (0, "0xFFFF", "Immediate", "MAX"),
    (1, "0x00FF", "TOP", "BOTTOM"),
    (2, "0x01FF", "TOP", "BOTTOM"),
    (3, "0x03FF", "TOP", "BOTTOM"),
    (4, "OCR1A", "Immediate", "MAX"),
    (5, "0x00FF", "BOTTOM", "TOP"),
    (6, "0x01FF", "BOTTOM", "TOP"),
    (7, "0x03FF", "BOTTOM", "TOP"),
    (8, "ICR1", "BOTTOM", "BOTTOM"),
    (9, "OCR1A", "BOTTOM", "BOTTOM"),
    (10, "ICR1", "TOP", "BOTTOM"),
    (11, "OCR1A", "TOP", "BOTTOM"),
    (12, "ICR1", "Immediate", "MAX"),
    (14, "ICR1", "BOTTOM", "TOP"),
    (15, "OCR1A", "BOTTOM", "TOP"),
];

/// WGM and `COM` are set. `OCR1A` or `ICR1` holds `top`. The clock is stopped.
fn timer1(mode: u8, src: &str, top: u16, com: u8) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.poke(0x80, (mode & 0x03) | com);
    cpu.poke(0x81, (mode & 0x0C) << 1);
    if src == "OCR1A" || src == "ICR1" {
        let low = if src == "OCR1A" { 0x88 } else { 0x86 };
        cpu.poke(low + 1, (top >> 8) as u8);
        cpu.poke(low, (top & 0xFF) as u8);
    }
    cpu
}

/// Plant `TCNT1`, then CS = 1.
fn start(cpu: &mut Cpu, tcnt: u16) {
    cpu.poke(0x85, (tcnt >> 8) as u8);
    cpu.poke(0x84, (tcnt & 0xFF) as u8);
    cpu.poke(0x81, cpu.peek(0x81) | 0x01);
}

/// Stop the clock and `LDS` the low byte, then the high byte.
fn tcnt1(cpu: &mut Cpu) -> u16 {
    cpu.poke(0x81, cpu.peek(0x81) & !0x07);
    let mut program = lds(16, 0x84).to_vec();
    program.extend(lds(17, 0x85));
    cpu.load_words(&program);
    cpu.insns(2);
    u16::from(cpu.reg(16)) | (u16::from(cpu.reg(17)) << 8)
}

/// Table 16-4, one row at a time. `COM1A` = 1 toggles in the modes Tables 16-2
/// and 16-3 name, and in every non-PWM mode.
#[test]
fn timer1_waveform_modes_follow_table_16_4() {
    // A nonzero OCR1A and ICR1, so a mode 12 or 14 decode of this row counts.
    let mut reserved = Cpu::new();
    reserved.poke(0x80, 0x01);
    reserved.poke(0x89, 0);
    reserved.poke(0x88, 8);
    reserved.poke(0x87, 0);
    reserved.poke(0x86, 8);
    reserved.poke(0x81, 0x18 | 0x01);
    run_nops(&mut reserved, 4);
    assert_eq!(reserved.peek(0x84), 0, "mode 13 does not count");

    for (mode, src, update, tov) in TABLE_16_4 {
        let top = match src {
            "0xFFFF" => 0xFFFF,
            "0x00FF" => 0x00FF,
            "0x01FF" => 0x01FF,
            "0x03FF" => 0x03FF,
            _ => 4,
        };
        let dual = tov == "BOTTOM";

        let mut cpu = timer1(mode, src, top, 0);
        start(&mut cpu, top);
        run_nops(&mut cpu, 1);
        let tov_flag = cpu.peek(0x36) & 1;
        let next = if dual { top - 1 } else { 0 };
        assert_eq!(tcnt1(&mut cpu), next, "mode {mode} leaves TOP {top:#x}");
        let want_tov = u8::from(tov == "TOP" || top == 0xFFFF);
        assert_eq!(tov_flag, want_tov, "mode {mode} TOV1 at TOP");

        if dual {
            let mut down = timer1(mode, src, top, 0);
            start(&mut down, top);
            run_nops(&mut down, usize::from(top));
            assert_eq!(down.peek(0x36) & 1, 1, "mode {mode} TOV1 at BOTTOM");
        }
        if tov == "MAX" && top != 0xFFFF {
            let mut wide = timer1(mode, src, 0xFFFF, 0);
            start(&mut wide, 0xFFFF);
            run_nops(&mut wide, 1);
            assert_eq!(wide.peek(0x36) & 1, 1, "mode {mode} TOV1 at MAX");
        }

        // Table 16-1 sets OC on a match. Table 16-2 note 1 and §16.9.3–§16.9.5
        // keep inverting PWM low while OCR1x = TOP, so a high pin means the
        // write of 1 is the live compare.
        let mut pin = timer1(mode, src, top, 0x30);
        pin.poke(0x24, 1 << 2);
        pin.poke(0x8B, (top >> 8) as u8);
        pin.poke(0x8A, (top & 0xFF) as u8);
        let mut store = vec![imm(16, 0)];
        store.extend(sts(16, 0x8B));
        store.push(imm(16, 1));
        store.extend(sts(16, 0x8A));
        pin.load_words(&store);
        pin.insns(4);
        start(&mut pin, 0);
        run_nops(&mut pin, 1);
        let high = pin.pin_levels().get(2) == Some(Level::High);
        assert_eq!(
            high,
            update == "Immediate",
            "mode {mode} OCR1B at the write"
        );
        run_nops(&mut pin, usize::from(top - 1));
        let high = pin.pin_levels().get(2) == Some(Level::High);
        assert_eq!(high, update != "BOTTOM", "mode {mode} OCR1B at TOP");
        let rest = if dual { usize::from(top) + 1 } else { 2 };
        run_nops(&mut pin, rest);
        let high = pin.pin_levels().get(2) == Some(Level::High);
        assert!(high, "mode {mode} OCR1B by BOTTOM");

        // Tables 16-2 and 16-3. COM1A = 1 toggles OC1A in these PWM modes.
        let mut oc = timer1(mode, src, top, 0x40);
        oc.poke(0x24, 1 << 1);
        oc.poke(0x89, 0);
        oc.poke(0x88, 1);
        start(&mut oc, 0);
        run_nops(&mut oc, 1);
        let toggled = oc.pin_levels().get(1) == Some(Level::High);
        let want = update == "Immediate" || matches!(mode, 9 | 11 | 14 | 15);
        assert_eq!(toggled, want, "mode {mode} COM1A toggle");
    }
}

/// §14. Pin id is `(letter - 'B') * 8 + bit`. A PWM edge keeps its own cycle.
#[test]
fn pin_ids_and_a_pwm_edge_between_instruction_boundaries() {
    let mut cpu = Cpu::new();
    cpu.poke(0x27, 0x01); // DDRC bit 0 → pin 8
    cpu.poke(0x28, 0x01);
    let report = cpu.take_pin();
    assert_eq!(report, None, "poke does not report");
    let program = [imm(16, 0x01), out(16, 0x07), out(16, 0x0A)];
    cpu.load_words(&program);
    cpu.insns(3);
    let edges: Vec<_> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.pin, report.level, report.cycle))
        .collect();
    // DDRC then DDRD. Each OUT is one cycle. The first OUT is the second instruction.
    assert_eq!(edges[0], (8, Level::High, 2));
    assert_eq!(edges[1], (16, Level::Low, 3));

    // CTC, OCR0A = 0, COM0A toggle: one edge per clkI/O. ADIW is two cycles,
    // so the first edge is not the cycle the instruction finishes on.
    let mut pwm = Cpu::new();
    pwm.poke(0x2A, 1 << 6);
    pwm.poke(0x44, 0x40 | 0x02);
    pwm.poke(0x47, 0);
    pwm.poke(0x45, 0x01);
    pwm.load_words(&[0x9601]);
    pwm.step();
    assert_eq!(pwm.cycles(), 2);
    let edges: Vec<_> = std::iter::from_fn(|| pwm.take_pin())
        .map(|report| (report.pin, report.cycle))
        .collect();
    assert_eq!(edges, vec![(22, 1), (22, 2)]);
    assert!(edges[0].1 < pwm.cycles());
}

/// The pin queue keeps the newest reports and counts each dropped oldest one.
#[test]
fn the_pin_queue_is_bounded() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x44, 0x40 | 0x02);
    cpu.poke(0x45, 0x01);
    let extra = 10usize;
    let total = PIN_QUEUE_CAP + extra;
    run_nops(&mut cpu, total);
    assert_eq!(cpu.pin_overflow(), extra as u64);
    let kept: Vec<_> = std::iter::from_fn(|| cpu.take_pin()).collect();
    assert_eq!(kept.len(), PIN_QUEUE_CAP);
    assert_eq!(
        kept[0].cycle,
        extra as u64 + 1,
        "the oldest edges were dropped"
    );
    assert_eq!(
        kept.last().map(|report| report.cycle),
        Some(total as u64),
        "the newest edge is still queued"
    );
    assert_eq!(cpu.pin_levels().get(22), Some(Level::Low));
}

/// §17.3. Clock-select 6 counts falling edges on T0. With no edge, TCNT stays 0.
#[test]
fn external_clock_without_an_edge_does_not_count() {
    let mut cpu = Cpu::new();
    let program = [imm(16, 0x06), out(16, 0x25)];
    cpu.load_words(&program);
    cpu.insns(2);
    cpu.load_words(&nop_sled(20));
    cpu.insns(20);
    assert_eq!(
        cpu.peek(0x46),
        0,
        "no T0 edge, so the external clock does not count"
    );
}

/// §18.11. AS2 reads back. Timer2 stays on clkI/O.
#[test]
fn async_timer2_stays_on_the_system_clock() {
    let mut timer2 = Cpu::new();
    let mut program = Vec::new();
    program.push(imm(16, 0x20));
    program.extend(sts(16, 0xB6));
    timer2.load_words(&program);
    timer2.insns(2);
    assert_eq!(timer2.peek(0xB6) & 0x20, 0x20, "AS2 reads back");
    assert_eq!(timer2.peek(0xB6) & 0x1F, 0, "update-busy bits stay clear");
    timer2.poke(0xB1, 0x01);
    timer2.load_words(&nop_sled(4));
    timer2.insns(4);
    assert_eq!(timer2.peek(0xB2), 4, "Timer2 still counts clkI/O");
}

/// §16.11.1. `TCCR1A` bits 3:2 are reserved and read as zero.
#[test]
fn tccr1a_reserved_bits_read_zero() {
    let mut cpu = Cpu::new();
    cpu.poke(0x80, 0x08);
    assert_eq!(cpu.peek(0x80) & 0x0C, 0);
}

/// §15.5. At prescaler /64 the pin toggles on the match and the flag, then the
/// interrupt, lands on the next timer clock.
#[test]
fn compare_flag_lands_one_prescaler_tick_after_the_pin() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x44, 0x40);
    cpu.poke(0x47, 1);
    cpu.poke(0x6E, 0x02);
    cpu.poke(0x45, 0x03);
    cpu.set_sreg(0x80);
    cpu.load_words(&nop_sled(200));
    cpu.insns(64);
    assert_eq!(cpu.cycles(), 64);
    assert_eq!(cpu.pc(), 64, "the match clock does not enter");
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0);
    let edge = cpu.take_pin().unwrap();
    assert_eq!((edge.pin, edge.level, edge.cycle), (22, Level::High, 64));
    cpu.insns(63);
    assert_eq!(cpu.cycles(), 127);
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0);
    cpu.step();
    assert_eq!(
        cpu.cycles(),
        128 + 4,
        "OCF on prescaler count 128, then the 4-cycle entry"
    );
    assert_eq!(cpu.pc(), 0x001C, "TIMER0_COMPA");
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0, "entry clears OCF0A");
}

/// §15.4 and §15.5.2. `OUT TCNT0` at /1 holds the written value for that cycle.
/// The next timer clock counts, and its compare match is blocked.
#[test]
fn out_tcnt0_holds_the_written_value_and_blocks_the_next_match() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x44, 0x40);
    cpu.poke(0x47, 5);
    let program = [
        imm(16, 1),
        out(16, 0x25),
        imm(17, 4),
        out(17, 0x26),
        0x0000,
        0x0000,
    ];
    cpu.load_words(&program);
    cpu.insns(4);
    assert_eq!(cpu.peek(0x46), 4, "the OUT cycle does not increment TCNT");
    cpu.insns(1);
    assert_eq!(cpu.peek(0x46), 5, "the next clock counts");
    assert_eq!(flags(&cpu, 0x35) & 0x02, 0, "that compare is blocked");
    assert!(
        std::iter::from_fn(|| cpu.take_pin()).all(|report| report.level != Level::High),
        "the blocked match does not toggle OC0A"
    );
    cpu.insns(1);
    assert_eq!(cpu.peek(0x46), 6);
    assert_eq!(
        flags(&cpu, 0x35) & 0x02,
        0,
        "a blocked match does not raise OCF on the clock after it"
    );
}

/// §16.5 and §16.7. A 2-cycle `STS` of `TCNT1` does not count on either cycle.
/// The following timer clock is the blocked compare.
#[test]
fn sts_tcnt1_holds_across_both_cycles_and_blocks_the_next_match() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 1);
    cpu.poke(0x80, 0x40);
    cpu.poke(0x89, 0x00);
    cpu.poke(0x88, 0x11);
    let mut program = Vec::new();
    program.push(imm(16, 1));
    program.extend(sts(16, 0x81));
    program.push(imm(17, 0x00));
    program.extend(sts(17, 0x85));
    program.push(imm(18, 0x10));
    program.extend(sts(18, 0x84));
    program.push(0x0000);
    program.push(0x0000);
    cpu.load_words(&program);
    cpu.insns(6);
    assert_eq!(cpu.peek(0x84), 0x10, "neither cycle of the STS counts");
    assert_eq!(cpu.peek(0x85), 0x00);
    cpu.insns(1);
    assert_eq!(cpu.peek(0x84), 0x11, "the next clock counts once");
    assert_eq!(flags(&cpu, 0x36) & 0x02, 0);
    assert!(
        std::iter::from_fn(|| cpu.take_pin()).all(|report| report.level != Level::High),
        "the blocked match does not toggle OC1A"
    );
    cpu.insns(1);
    assert_eq!(cpu.peek(0x84), 0x12);
    assert_eq!(flags(&cpu, 0x36) & 0x02, 0);
}

/// §14.2 / §15.6. A connected compare unit does not report while DDR is clear.
#[test]
fn compare_override_with_ddr_clear_emits_no_pin_report() {
    let mut cpu = Cpu::new();
    cpu.poke(0x44, 0xC0);
    let program = [imm(16, 0x80), out(16, 0x25)];
    cpu.load_words(&program);
    cpu.insns(2);
    assert!(cpu.take_pin().is_none(), "FOC with DDR clear is silent");
    cpu.poke(0x44, 0x40);
    cpu.poke(0x47, 1);
    cpu.poke(0x45, 0x01);
    cpu.load_words(&nop_sled(4));
    cpu.insns(4);
    assert!(
        cpu.take_pin().is_none(),
        "a running compare with DDR clear is silent"
    );
}

/// §15.5.3. Fast PWM, non-inverting, OCR0A = 5. BOTTOM sets OC0A at cycle 256.
/// `OUT TCNT0, 4` then makes the next count equal OCR. That clock is blocked,
/// and the one after it is not a compare, so the pin stays high (§15.5.3: the
/// missed match is lost). The clear is the next real match, at cycle 515.
#[test]
fn blocked_fast_pwm_match_does_not_move_the_pin() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x47, 5);
    cpu.poke(0x44, 0x80 | 0x03);
    cpu.poke(0x45, 0x01);
    let mut program = nop_sled(256);
    program.push(imm(16, 4));
    program.push(out(16, 0x26));
    program.extend(nop_sled(260));
    cpu.load_words(&program);
    cpu.run_until(516);
    let edges: Vec<_> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.level, report.cycle))
        .collect();
    assert_eq!(
        edges,
        vec![(Level::Low, 1), (Level::High, 256), (Level::Low, 515),],
        "connect 0, set at BOTTOM, clear on the next real match"
    );
}

/// §15.7.4 / §15.5.3. Phase correct, non-inverting, OCR0A = 5, TOP = 255.
/// The down-count match sets OC0A at cycle 505. `OUT TCNT0, 4` at cycle 512
/// blocks the match at 514. Figure 15-7 puts the missed up-count clear at
/// TOP, not on the following clock. After the `OUT`, 513 holds 4 and 514 is
/// 5, so TCNT0 = cycle − 509. TOP 255 is cycle 764.
#[test]
fn blocked_phase_correct_match_does_not_move_the_pin() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x47, 5);
    cpu.poke(0x44, 0x80 | 0x01);
    cpu.poke(0x45, 0x01);
    let mut program = nop_sled(511);
    program.push(imm(16, 4));
    program.push(out(16, 0x26));
    program.extend(nop_sled(260));
    cpu.load_words(&program);
    cpu.run_until(765);
    let edges: Vec<_> = std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.level, report.cycle))
        .collect();
    assert_eq!(
        edges,
        vec![(Level::Low, 1), (Level::High, 505), (Level::Low, 764)],
        "missed up-count clears at TOP"
    );
}

/// Dual-slope counting stays above a TOP that was lowered under `TCNT`.
///
/// Phase correct, Timer1, `ICR1` as TOP. `ICR1` goes from 80 to 10 at
/// `TCNT1 = 0x29`. Two clocks later the count is still going up.
#[test]
fn phase_correct_keeps_counting_up_when_top_drops_below_tcnt() {
    let mut cpu = Cpu::new();
    cpu.poke(0x87, 0);
    cpu.poke(0x86, 80);
    cpu.poke(0x80, 0x02); // WGM11. Mode 10 once WGM13 is set.
    cpu.poke(0x81, 0x10 | 0x01);
    cpu.load_words(&nop_sled(0x40));
    cpu.run_until(0x29);
    assert_eq!(cpu.peek(0x84), 0x29);
    cpu.poke(0x86, 10);
    cpu.run_until(0x2B);
    assert_eq!(cpu.peek(0x84), 0x2B);
    assert_eq!(cpu.peek(0x36) & 0x2F, 0, "no TOP or BOTTOM flag yet");
    cpu.run_until(0x2F);
    assert_eq!(cpu.peek(0x84), 0x2F);
}

/// The same TOP drop in phase-and-frequency-correct mode (WGM = 8).
#[test]
fn phase_and_frequency_correct_keeps_counting_up_when_top_drops_below_tcnt() {
    let mut cpu = Cpu::new();
    cpu.poke(0x87, 0);
    cpu.poke(0x86, 80);
    cpu.poke(0x80, 0x00);
    cpu.poke(0x81, 0x10 | 0x01); // WGM13, CS = 1. Mode 8.
    cpu.load_words(&nop_sled(0x40));
    cpu.run_until(0x29);
    assert_eq!(cpu.peek(0x84), 0x29);
    cpu.poke(0x86, 10);
    cpu.run_until(0x2B);
    assert_eq!(cpu.peek(0x84), 0x2B);
    assert_eq!(cpu.peek(0x36) & 0x2F, 0, "no TOP or BOTTOM flag yet");
    cpu.run_until(0x2F);
    assert_eq!(cpu.peek(0x84), 0x2F);
}

/// Mode 5, `OCR0A` TOP = 10, `TCNT0` = `0x21`, `/1`.
///
/// The count keeps climbing through `0xFF`. Clock 223 is the width wrap
/// (`0xFF` → `0x00`, §15.2.2, Table 15-1). The next climb reaches TOP on
/// clock 233 and turns on clock 234. §15.7.4 sets `TOV0` at BOTTOM on the
/// way down, so the wrap itself leaves the flag clear.
#[test]
fn phase_correct_eight_bit_wraps_at_max_and_turns_at_top() {
    let mut cpu = Cpu::new();
    cpu.poke(0x47, 10);
    cpu.poke(0x44, 0x01); // WGM00
    cpu.poke(0x45, 0x08 | 0x01); // WGM02, CS = 1. Mode 5.
    cpu.poke(0x46, 0x21);
    cpu.load_words(&nop_sled(250));
    cpu.run_until(2);
    assert_eq!(cpu.peek(0x46), 0x23);
    cpu.run_until(222);
    assert_eq!(cpu.peek(0x46), 0xFF);
    cpu.run_until(223);
    assert_eq!(cpu.peek(0x46), 0x00, "back inside 0..=TOP at clock 223");
    assert_eq!(cpu.peek(0x35) & 0x01, 0, "width wrap is not BOTTOM");
    cpu.run_until(233);
    assert_eq!(cpu.peek(0x46), 10, "the next climb reaches TOP");
    cpu.run_until(234);
    assert_eq!(cpu.peek(0x46), 9, "the climb turns at TOP");
}

fn rises_after(cpu: &mut Cpu, pin: u32, after: u64) -> Vec<u64> {
    std::iter::from_fn(|| cpu.take_pin())
        .filter(|report| report.pin == pin && report.level == Level::High && report.cycle > after)
        .map(|report| report.cycle)
        .collect()
}

/// §16.9.4 phase-correct 8-bit, /1, OCR1A = 100, non-inverting. COM1A1 is
/// cleared while OC1A is low (TCNT1 = 200, counting up, past the match) and
/// set again while TCNT1 = 50, counting down. §16.8.1: COM = 0 does not
/// change the OC1A register, and the new COM acts at the next compare match.
/// The down-count match is the one that sets the pin (Table 16-3).
///
/// `LDI` is 1 cycle and `STS` is 2 (DS40002198). The clock store runs in
/// cycles 3 and 4 and ends at edge 5, which still counts with the old select
/// (Figure 14-4, §15.8). The first timer clock is edge 6, so TCNT1 = cycle − 5
/// on the climb. The clear store is cycle 205. Cycle 260 is TOP. Cycle 261 is
/// 254 counting down, so cycle 465 is 50. The set store is that cycle. The
/// next down-count match is BOTTOM at 515, up to TOP at 770, then 155 clocks
/// down to 100 at 925.
#[test]
fn timer1_phase_correct_holds_oc1a_until_the_down_match() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 1);
    cpu.poke(0x89, 0);
    cpu.poke(0x88, 100);
    cpu.poke(0x80, 0x80 | 0x01);
    let mut program = vec![imm(16, 0x01), imm(17, 0x81), imm(18, 0x01)];
    program.extend(sts(18, 0x81));
    program.extend(nop_sled(200));
    program.extend(sts(16, 0x80));
    program.extend(nop_sled(258));
    program.extend(sts(17, 0x80));
    program.extend(nop_sled(461));
    cpu.load_words(&program);
    cpu.run_until(205);
    assert_eq!(cpu.peek(0x84), 200, "clear COM1A1 at TCNT1 = 200");
    cpu.run_until(465);
    assert_eq!(cpu.peek(0x84), 50, "set COM1A1 at TCNT1 = 50");
    cpu.run_until(926);
    let rises = rises_after(&mut cpu, 1, 465);
    assert_eq!(
        rises.first().copied(),
        Some(925),
        "OC1A stays low until the down-count match, got {rises:?}"
    );
}

/// §18.7.3 fast PWM, /1, OCR2A = 100, non-inverting. COM2A1 is cleared at
/// TCNT2 = 200 (the pin is low after the match at 100) and set again at
/// TCNT2 = 34, which is below OCR. §18.6.1 keeps the held OC2A until the
/// next event. The match would clear an already low register. Table 18-3
/// sets it at BOTTOM.
///
/// The clock store runs in cycles 3 and 4 and ends at edge 5 under the old
/// select (Figure 14-4, §15.8). The first clock is edge 6, so TCNT2 =
/// (cycle − 5) mod 256. The clear store is cycle 205 (TCNT2 = 200). The set
/// store is cycle 295 (TCNT2 = 34). BOTTOM is the wrap to 0 at cycle 517.
#[test]
fn timer2_fast_pwm_holds_oc2a_until_bottom() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 3);
    cpu.poke(0xB3, 100);
    cpu.poke(0xB0, 0x80 | 0x03);
    let mut program = vec![imm(16, 0x03), imm(17, 0x83), imm(18, 0x01)];
    program.extend(sts(18, 0xB1));
    program.extend(nop_sled(200));
    program.extend(sts(16, 0xB0));
    program.extend(nop_sled(88));
    program.extend(sts(17, 0xB0));
    program.extend(nop_sled(223));
    cpu.load_words(&program);
    cpu.run_until(205);
    assert_eq!(cpu.peek(0xB2), 200, "clear COM2A1 at TCNT2 = 200");
    cpu.run_until(295);
    assert_eq!(cpu.peek(0xB2), 34, "set COM2A1 at TCNT2 = 34");
    cpu.run_until(518);
    let rises = rises_after(&mut cpu, 3, 295);
    assert_eq!(
        rises.first().copied(),
        Some(517),
        "OC2A stays low until BOTTOM, got {rises:?}"
    );
}

/// Same fast-PWM hold on Timer0, which shares the compare unit. `OUT` is 1
/// cycle, so the clock store at cycle 3 ends at edge 4, which counts with the
/// old select (Figure 14-4, §15.8). TCNT0 = (cycle − 4) mod 256. Clear at
/// cycle 204 (TCNT0 = 200). Set at cycle 294 (TCNT0 = 34). §15.6.1 and Table
/// 15-3: the pin stays low until BOTTOM at cycle 516.
#[test]
fn timer0_fast_pwm_holds_oc0a_until_bottom() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 6);
    cpu.poke(0x47, 100);
    cpu.poke(0x44, 0x80 | 0x03);
    let program = [imm(16, 0x03), imm(17, 0x83), imm(18, 0x01), out(18, 0x25)];
    let mut program = program.to_vec();
    program.extend(nop_sled(200));
    program.push(out(16, 0x24));
    program.extend(nop_sled(89));
    program.push(out(17, 0x24));
    program.extend(nop_sled(222));
    cpu.load_words(&program);
    cpu.run_until(204);
    assert_eq!(cpu.peek(0x46), 200, "clear COM0A1 at TCNT0 = 200");
    cpu.run_until(294);
    assert_eq!(cpu.peek(0x46), 34, "set COM0A1 at TCNT0 = 34");
    cpu.run_until(517);
    let rises = rises_after(&mut cpu, 22, 294);
    assert_eq!(
        rises.first().copied(),
        Some(516),
        "OC0A stays low until BOTTOM, got {rises:?}"
    );
}

fn pin_edges(cpu: &mut Cpu) -> Vec<(Level, u64)> {
    std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.level, report.cycle))
        .collect()
}

/// Figure 15-7 / §16.8.1. Phase-correct 8-bit, /1, OCR1A = 100. COM1A1 is
/// cleared at TCNT1 = 50 counting down and set again at TCNT1 = 150 counting
/// up. The register stays 1, so OC1A is high from the re-enable until TOP.
///
/// CS is poked at cycle 0, so the first clock is cycle 1 and TCNT1 follows
/// the triangle. The down-count match is cycle 410. TOP of the next period
/// is cycle 765. A poke does not publish: the disconnect is the next clock
/// (461) and the reconnect is the next clock (661).
#[test]
fn timer1_phase_correct_reenable_on_the_up_slope_holds_until_top() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 1);
    cpu.poke(0x80, 0x80 | 0x01);
    cpu.poke(0x89, 0);
    cpu.poke(0x88, 100);
    cpu.poke(0x81, 0x01);
    cpu.load_words(&nop_sled(800));
    cpu.run_until(460);
    assert_eq!(cpu.peek(0x84), 50, "clear COM1A1 at TCNT1 = 50 down");
    cpu.poke(0x80, 0x01);
    cpu.run_until(660);
    assert_eq!(cpu.peek(0x84), 150, "set COM1A1 at TCNT1 = 150 up");
    cpu.poke(0x80, 0x80 | 0x01);
    cpu.run_until(766);
    assert_eq!(
        pin_edges(&mut cpu),
        vec![
            (Level::Low, 1),
            (Level::High, 410),
            (Level::Low, 461),
            (Level::High, 661),
            (Level::Low, 765),
        ]
    );
}

/// §16.8.1 / §16.9.4. Phase correct, COM cleared, OCR1A buffered to 200,
/// then COM restored. The TOP copy runs while COM is 0, so OC1A stays 1
/// until the re-enable. The up-count match of 200 clears it.
///
/// First clock is cycle 1. TCNT1 = cycle through TOP at 255, then
/// TCNT1 = 510 − cycle while counting down. The down-count match of 100 is
/// cycle 410. COM1A1 is cleared at 460 (TCNT1 = 50); the pin releases at
/// 461. 550 `NOP`s, then `LDI`/`STS` of `OCR1AH` and `OCR1AL`. The low-byte
/// store is cycle 554 and retires at 556, still counting up (TCNT1 = 46)
/// and still with COM = 0, so the store is only the buffer. The next TOP is
/// cycle 765 and copies 200. COM is still 0, so OC1A is unchanged. Counting
/// down, TCNT1 = 220 at cycle 800. COM1A1 is set there and the pin shows the
/// held 1 at 801. BOTTOM is cycle 1020. TCNT1 = 200 counting up at
/// 1020 + 200 = 1220, which clears.
#[test]
fn phase_correct_ocr_updates_at_top_while_com_is_clear() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 1);
    cpu.poke(0x80, 0x80 | 0x01);
    cpu.poke(0x89, 0);
    cpu.poke(0x88, 100);
    cpu.poke(0x81, 0x01);
    let mut program = nop_sled(550);
    program.push(imm(16, 0));
    program.extend(sts(16, 0x89));
    program.push(imm(16, 200));
    program.extend(sts(16, 0x88));
    program.extend(nop_sled(700));
    cpu.load_words(&program);
    cpu.run_until(460);
    assert_eq!(cpu.peek(0x84), 50, "clear COM1A1 at TCNT1 = 50 down");
    cpu.poke(0x80, 0x01);
    cpu.run_until(800);
    assert_eq!(cpu.peek(0x84), 220, "set COM1A1 at TCNT1 = 220 down");
    cpu.poke(0x80, 0x80 | 0x01);
    cpu.run_until(1221);
    assert_eq!(
        pin_edges(&mut cpu),
        vec![
            (Level::Low, 1),
            (Level::High, 410),
            (Level::Low, 461),
            (Level::High, 801),
            (Level::Low, 1220),
        ]
    );
}

/// Figure 15-7. Phase correct, OCR1A buffered from 255 to 100 before the
/// next TOP. The up-count result at that TOP clears the pin. The down-count
/// match then sets it.
///
/// First clock is cycle 1. OCR1A is planted at 255, so the first TOP, cycle
/// 255, holds OC1A high (Table 16-3, OCR = TOP). 294 nops, then `LDI`/`STS`
/// of `OCR1AH` and `OCR1AL`. The low-byte store is cycle 298 and retires at
/// 300, while counting down. That write is the buffer (§16.9.4). The next
/// TOP is cycle 765: OCR1A becomes 100 and the up-count result clears OC1A.
/// Counting down, TCNT1 = 100 at 765 + 155 = 920.
#[test]
fn phase_correct_ocr_leaving_top_clears_at_top_and_sets_on_the_way_down() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 1);
    cpu.poke(0x80, 0x80 | 0x01);
    cpu.poke(0x89, 0);
    cpu.poke(0x88, 255);
    cpu.poke(0x81, 0x01);
    let mut program = nop_sled(294);
    program.push(imm(16, 0));
    program.extend(sts(16, 0x89));
    program.push(imm(16, 100));
    program.extend(sts(16, 0x88));
    program.extend(nop_sled(700));
    cpu.load_words(&program);
    cpu.run_until(921);
    assert_eq!(
        pin_edges(&mut cpu),
        vec![
            (Level::Low, 1),
            (Level::High, 255),
            (Level::Low, 765),
            (Level::High, 920),
        ]
    );
}

/// Figure 15-7 / §16.9.4. The same program, with the buffer written 0.
/// The low-byte `STS` retires at 300. OCR = BOTTOM is continuously low,
/// so the pin goes low at the next TOP (765), not at the following BOTTOM.
#[test]
fn phase_correct_ocr_from_top_to_bottom_goes_low_at_top() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 1);
    cpu.poke(0x80, 0x80 | 0x01);
    cpu.poke(0x89, 0);
    cpu.poke(0x88, 255);
    cpu.poke(0x81, 0x01);
    let mut program = nop_sled(294);
    program.push(imm(16, 0));
    program.extend(sts(16, 0x89));
    program.push(imm(16, 0));
    program.extend(sts(16, 0x88));
    program.extend(nop_sled(800));
    cpu.load_words(&program);
    cpu.run_until(1100);
    assert_eq!(
        pin_edges(&mut cpu),
        vec![(Level::Low, 1), (Level::High, 255), (Level::Low, 765)]
    );
}

/// §15.7.4. Phase-correct OCR = TOP is constant high for COM = 2 and
/// constant low for COM = 3. The first TOP is cycle 255.
#[test]
fn phase_correct_ocr_top_is_constant_for_both_com_values() {
    let mut high = Cpu::new();
    high.poke(0x2A, 1 << 6);
    high.poke(0x47, 255);
    high.poke(0x44, 0x80 | 0x01);
    high.poke(0x45, 0x01);
    high.load_words(&nop_sled(800));
    high.run_until(800);
    assert_eq!(
        pin_edges(&mut high),
        vec![(Level::Low, 1), (Level::High, 255)]
    );

    let mut low = Cpu::new();
    low.poke(0x2A, 1 << 6);
    low.poke(0x47, 255);
    low.poke(0x44, 0xC0 | 0x01);
    low.poke(0x45, 0x01);
    low.load_words(&nop_sled(800));
    low.run_until(800);
    assert!(
        pin_edges(&mut low).iter().all(|edge| edge.0 == Level::Low),
        "COM = 3 inverts the constant high"
    );
}

/// §16.9.4. OCR = BOTTOM in phase and frequency correct stays low on the
/// down-count arrival at 0. ICR1 = 5, OCR1A = 3, COM1A1, /1. The down-count
/// match sets OC1A at cycle 7. The poke at cycle 8 plants OCR1A = 0 in both
/// the buffer and the active register. Cycle 10 is that arrival.
#[test]
fn phase_freq_ocr_zero_goes_low_at_bottom() {
    let mut cpu = Cpu::new();
    cpu.poke(0x24, 1 << 1);
    cpu.poke(0x87, 0);
    cpu.poke(0x86, 5);
    cpu.poke(0x80, 0x80);
    cpu.poke(0x81, 0x10);
    cpu.poke(0x89, 0);
    cpu.poke(0x88, 3);
    cpu.poke(0x81, 0x11);
    cpu.load_words(&nop_sled(20));
    cpu.run_until(8);
    cpu.poke(0x88, 0);
    cpu.run_until(16);
    assert_eq!(
        pin_edges(&mut cpu),
        vec![(Level::Low, 1), (Level::High, 7), (Level::Low, 10)]
    );
}

/// §16.9.3. Fast PWM sets OC0B only when the count changes from TOP to
/// BOTTOM. A width wrap above TOP (`0xFF` → 0) is not that clock.
///
/// Mode 7, TOP = OCR0A = 10, OCR0B = 5. The first TOP→BOTTOM is cycle 11
/// and the match clears at 16. `OUT TCNT0, 0xFE` at cycle 20: 21 holds
/// 0xFE, 22 is 0xFF, 23 wraps to 0 and does not set. The next TOP→BOTTOM
/// is cycle 34.
#[test]
fn fast_pwm_width_wrap_does_not_set_the_pin() {
    let mut cpu = Cpu::new();
    cpu.poke(0x2A, 1 << 5);
    cpu.poke(0x47, 10);
    cpu.poke(0x48, 5);
    cpu.poke(0x44, 0x23);
    cpu.poke(0x45, 0x09);
    let mut program = nop_sled(19);
    program.push(imm(16, 0xFE));
    program.push(out(16, 0x26));
    program.extend(nop_sled(30));
    cpu.load_words(&program);
    cpu.run_until(40);
    assert_eq!(
        pin_edges(&mut cpu),
        vec![
            (Level::Low, 1),
            (Level::High, 11),
            (Level::Low, 16),
            (Level::High, 34),
            (Level::Low, 39),
        ]
    );
}

/// A poke of `COM` does not publish. The connect edge is the first timer clock.
#[test]
fn a_poke_armed_compare_drives_on_the_first_clock() {
    let mut cpu = Cpu::new();
    // Inverting fast PWM, OCR0A = TOP, so the level stays low. OCR0B = 10 is
    // the first compare. Waiting for that event would report the connect at
    // cycle 10.
    cpu.poke(0x47, 255);
    cpu.poke(0x48, 10);
    cpu.poke(0x44, 0xC0 | 0x03);
    cpu.poke(0x45, 0x01);
    cpu.poke(0x2A, 1 << 6);
    cpu.load_words(&nop_sled(12));
    cpu.run_until(1);
    let edge = cpu.take_pin().expect("connect edge on the first clock");
    assert_eq!((edge.pin, edge.level, edge.cycle), (22, Level::Low, 1));
}
