//! Projection of the next event against a timer that steps every clock.
//!
//! Four thousand cycles per timer, plus short fixed stretches. The random
//! trace varies waveform mode, `COM` (disconnect, toggle, clear/non-inverting,
//! set/inverting), clock select, `OCR`, `TOP`, and `TCNT`. Sixteen-bit stores
//! use a non-zero high byte, and some of them leave `TEMP` uncommitted.
//! `PSRSYNC` and `TSM` are written while a timer runs. After every cycle the
//! projected `TCNT`, the flag byte, and the pin reports are compared.

use crate::{Cpu, PinReport};

const CYCLES: u64 = 4_000;

struct Dut {
    name: &'static str,
    seed: u32,
    sixteen: bool,
    tccra: u16,
    tccrb: u16,
    tcnt: u16,
    ocr_a: u16,
    ocr_b: u16,
    icr: u16,
    ddr: &'static [(u16, u8)],
    modes: &'static [u8],
}

const TIMER0: Dut = Dut {
    name: "timer0",
    seed: 0xA5A5_0001,
    sixteen: false,
    tccra: 0x0044,
    tccrb: 0x0045,
    tcnt: 0x0046,
    ocr_a: 0x0047,
    ocr_b: 0x0048,
    icr: 0,
    ddr: &[(0x002A, 1 << 6 | 1 << 5)],
    modes: &[0, 1, 2, 3, 4, 5, 6, 7],
};

const TIMER1: Dut = Dut {
    name: "timer1",
    seed: 0xA5A5_0002,
    sixteen: true,
    tccra: 0x0080,
    tccrb: 0x0081,
    tcnt: 0x0084,
    ocr_a: 0x0088,
    ocr_b: 0x008A,
    icr: 0x0086,
    ddr: &[(0x0024, 1 << 1 | 1 << 2)],
    modes: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
};

const TIMER2: Dut = Dut {
    name: "timer2",
    seed: 0xA5A5_0003,
    sixteen: false,
    tccra: 0x00B0,
    tccrb: 0x00B1,
    tcnt: 0x00B2,
    ocr_a: 0x00B3,
    ocr_b: 0x00B4,
    icr: 0,
    ddr: &[(0x0024, 1 << 3), (0x002A, 1 << 3)],
    modes: &[0, 1, 2, 3, 4, 5, 6, 7],
};

struct Xor(u32);

impl Xor {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

fn encode(sixteen: bool, wgm: u8, cs: u8, com_a: u8, com_b: u8) -> (u8, u8) {
    let tccra = ((com_a & 3) << 6) | ((com_b & 3) << 4) | (wgm & 0x03);
    let tccrb = if sixteen {
        ((wgm & 0x0C) << 1) | (cs & 0x07)
    } else {
        ((wgm & 0x04) << 1) | (cs & 0x07)
    };
    (tccra, tccrb)
}

fn push_low(writes: &mut Vec<(u16, u8)>, sixteen: bool, addr: u16, value: u8) {
    if sixteen {
        writes.push((addr + 1, 0));
    }
    writes.push((addr, value));
}

fn mode_writes(dut: &Dut, wgm: u8, cs: u8, com_a: u8, com_b: u8) -> Vec<(u16, u8)> {
    let (a, b) = encode(dut.sixteen, wgm, cs, com_a, com_b);
    vec![(dut.tccra, a), (dut.tccrb, b)]
}

/// High byte is never 0 on a 16-bit register. `lone` leaves `TEMP` latched.
fn push_wide(writes: &mut Vec<(u16, u8)>, sixteen: bool, addr: u16, rng: &mut Xor, lone: bool) {
    if !sixteen {
        writes.push((addr, rng.below(256) as u8));
        return;
    }
    let high = 1 + rng.below(255) as u8;
    writes.push((addr + 1, high));
    if !lone {
        writes.push((addr, rng.below(256) as u8));
    }
}

fn above_top(dut: &Dut, phase_freq: bool, tcnt: u8) -> Vec<(u16, u8)> {
    let mut writes = if dut.sixteen {
        let wgm = if phase_freq { 8 } else { 10 };
        let mut writes = mode_writes(dut, wgm, 1, 2, 2);
        push_low(&mut writes, true, dut.icr, 10);
        writes
    } else {
        let mut writes = mode_writes(dut, 5, 1, 2, 2);
        writes.push((dut.ocr_a, 10));
        writes
    };
    push_low(&mut writes, dut.sixteen, dut.tcnt, tcnt);
    writes
}

fn mutation(dut: &Dut, rng: &mut Xor) -> Vec<(u16, u8)> {
    match rng.below(6) {
        0 => {
            let wgm = dut.modes[rng.below(dut.modes.len() as u32) as usize];
            let cs = [0, 1, 2, 3][rng.below(4) as usize];
            mode_writes(dut, wgm, cs, rng.below(4) as u8, rng.below(4) as u8)
        }
        1 => {
            let mut writes = Vec::new();
            let lone = rng.below(5) == 0;
            push_wide(&mut writes, dut.sixteen, dut.ocr_a, rng, lone);
            writes
        }
        2 => {
            let mut writes = Vec::new();
            let lone = rng.below(5) == 0;
            push_wide(&mut writes, dut.sixteen, dut.ocr_b, rng, lone);
            writes
        }
        3 => {
            let mut writes = Vec::new();
            let use_icr = dut.sixteen && rng.below(2) == 0;
            if use_icr {
                let lone = rng.below(5) == 0;
                push_wide(&mut writes, true, dut.icr, rng, lone);
            } else {
                push_wide(&mut writes, dut.sixteen, dut.ocr_a, rng, false);
            }
            writes
        }
        4 => {
            let mut writes = Vec::new();
            let lone = rng.below(4) == 0;
            push_wide(&mut writes, dut.sixteen, dut.tcnt, rng, lone);
            writes
        }
        _ => {
            // `PSRSYNC`, `TSM`, both, or neither. `TSM|PSRSYNC` holds the divider.
            let value = [0x00, 0x01, 0x80, 0x81][rng.below(4) as usize];
            vec![(0x0043, value)]
        }
    }
}

fn poke_all(cpu: &mut Cpu, writes: &[(u16, u8)]) {
    for &(addr, value) in writes {
        cpu.poke(addr, value);
    }
}

fn pins(cpu: &mut Cpu) -> Vec<PinReport> {
    std::iter::from_fn(|| cpu.take_pin()).collect()
}

fn check(dut: &Dut) {
    let mut projected = Cpu::new();
    let mut stepped = Cpu::new();
    stepped.arm_stepped_timers();
    let nops = vec![0u16; CYCLES as usize];
    projected.load_words(&nops);
    stepped.load_words(&nops);
    for &(addr, value) in dut.ddr {
        projected.poke(addr, value);
        stepped.poke(addr, value);
    }
    let start = above_top(dut, false, 0);
    poke_all(&mut projected, &start);
    poke_all(&mut stepped, &start);

    let mut rng = Xor(dut.seed);
    let mut hold_until = 0u64;
    for cycle in 0..CYCLES {
        let writes = if cycle == 64 {
            hold_until = cycle + 32;
            above_top(dut, false, 0x29)
        } else if cycle == 1_500 {
            hold_until = cycle + 32;
            // Phase and frequency correct exists on the 16-bit timer. The
            // 8-bit timers plant phase-correct above TOP again.
            above_top(dut, dut.sixteen, 0x21)
        } else if cycle >= hold_until && cycle % 23 == 0 {
            mutation(dut, &mut rng)
        } else {
            Vec::new()
        };
        poke_all(&mut projected, &writes);
        poke_all(&mut stepped, &writes);

        let next = cycle + 1;
        projected.run_until(next);
        stepped.run_until(next);
        assert_eq!(
            projected.timer_observe(),
            stepped.timer_observe(),
            "{} cycle {cycle}: TCNT and flags diverged",
            dut.name
        );
        assert_eq!(
            pins(&mut projected),
            pins(&mut stepped),
            "{} cycle {cycle}: pin reports diverged",
            dut.name
        );
    }
}

#[test]
fn timer0_projection_matches_step() {
    check(&TIMER0);
}

#[test]
fn timer1_projection_matches_step() {
    check(&TIMER1);
}

#[test]
fn timer2_projection_matches_step() {
    check(&TIMER2);
}

fn fresh() -> (Cpu, Cpu) {
    let mut stepped = Cpu::new();
    stepped.arm_stepped_timers();
    (Cpu::new(), stepped)
}

fn step_both(projected: &mut Cpu, stepped: &mut Cpu, limit: u64, what: &str) {
    projected.run_until(limit);
    stepped.run_until(limit);
    assert_eq!(
        projected.timer_observe(),
        stepped.timer_observe(),
        "{what} at cycle {limit}"
    );
    assert_eq!(
        pins(projected),
        pins(stepped),
        "{what} pins at cycle {limit}"
    );
}

fn ldi(reg: u16, k: u16) -> u16 {
    0xE000 | ((k & 0xF0) << 4) | ((reg - 16) << 4) | (k & 0x0F)
}

fn out(reg: u16, io: u16) -> u16 {
    0xB800 | ((reg & 0x1F) << 4) | ((io & 0x30) << 5) | (io & 0x0F)
}

fn sts(reg: u16, addr: u16) -> [u16; 2] {
    [0x9200 | ((reg & 0x1F) << 4), addr]
}

/// `STS` of `TCNT1H` then `TCNT1L` with a non-zero high byte, then the width wrap.
#[test]
fn timer1_temp_store_wraps_from_fff0() {
    let (mut projected, mut stepped) = fresh();
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x80, 0x02); // WGM11
        cpu.poke(0x81, 0x10); // WGM13, clock stopped
        cpu.poke(0x87, 0x00);
        cpu.poke(0x86, 10); // ICR1 = 10
        cpu.poke(0x24, 0x06); // DDRB1 and DDRB2
    }
    let mut prog = vec![ldi(16, 0xFF)];
    prog.extend(sts(16, 0x0085));
    prog.push(ldi(16, 0xF0));
    prog.extend(sts(16, 0x0084));
    prog.extend(std::iter::repeat_n(0u16, 40));
    projected.load_words(&prog);
    stepped.load_words(&prog);
    // LDI 1, STS high ends at 3, LDI ends at 4, STS low ends at 6.
    for cycle in 1..=6 {
        step_both(&mut projected, &mut stepped, cycle, "sts");
        if cycle == 3 {
            assert_eq!(projected.timer_observe()[1].0, 0, "TEMP latched");
            assert_eq!(stepped.timer_observe()[1].0, 0);
        }
    }
    assert_eq!(projected.timer_observe()[1].0, 0xFFF0);
    assert_eq!(stepped.timer_observe()[1].0, 0xFFF0);
    projected.poke(0x81, 0x11);
    stepped.poke(0x81, 0x11);
    // Clock 16 (cycle 22) wraps to 0. Clock 26 (cycle 32) is TOP. Clock 27 turns.
    for cycle in 7..=40 {
        step_both(&mut projected, &mut stepped, cycle, "fff0");
        let (count, flags) = projected.timer_observe()[1];
        if cycle == 22 {
            assert_eq!(count, 0, "width wrap");
            assert_eq!(flags & 1, 0, "width wrap is not BOTTOM");
        } else if cycle == 32 {
            assert_eq!(count, 10);
        } else if cycle == 33 {
            assert_eq!(count, 9, "turned at TOP");
        }
    }
}

#[test]
fn prescaler_tsm_holds_the_count() {
    let (mut projected, mut stepped) = fresh();
    let nops = vec![0u16; 40];
    projected.load_words(&nops);
    stepped.load_words(&nops);
    projected.poke(0x45, 0x01);
    stepped.poke(0x45, 0x01);
    step_both(&mut projected, &mut stepped, 10, "run");
    assert_eq!(projected.peek(0x46), 10);
    // Bare `PSRSYNC` restarts the divider and the /1 count continues.
    projected.poke(0x43, 0x01);
    stepped.poke(0x43, 0x01);
    step_both(&mut projected, &mut stepped, 15, "psrsync");
    assert_eq!(projected.peek(0x46), 15);
    assert_eq!(stepped.peek(0x46), 15);
    projected.poke(0x43, 0x81);
    stepped.poke(0x43, 0x81);
    step_both(&mut projected, &mut stepped, 25, "held");
    assert_eq!(projected.peek(0x46), 15, "TSM|PSRSYNC holds Timer0");
    assert_eq!(stepped.peek(0x46), 15);
    projected.poke(0x43, 0x00);
    stepped.poke(0x43, 0x00);
    step_both(&mut projected, &mut stepped, 35, "released");
    assert_eq!(projected.peek(0x46), 25);
    assert_eq!(stepped.peek(0x46), 25);
}

/// Phase correct, register high, then a `TCNT` write, an OCR change, and a
/// COM re-enable above OCR on the up slope. The last `run_until` jumps
/// across TOP. Projected and stepped pins agree, including the up-count clear.
///
/// Mode 5, TOP = OCR0A = 20, OCR0B = 8, COM0B = 2, /1. The down-count match
/// is cycle 32. COM drops at 42 (TCNT0 = 2, up). TCNT0 is written 15 at 44.
/// COM returns at 46. The missed up-count clears at the next TOP.
#[test]
fn pwm_reenable_and_tcnt_write_match_the_stepped_timer() {
    let (mut projected, mut stepped) = fresh();
    // Cycles 0..=42 are nops. `LDI` is cycle 43 and `OUT TCNT0` is cycle 44.
    let mut prog = vec![0u16; 43];
    prog.push(ldi(16, 15));
    prog.push(out(16, 0x26));
    prog.extend(std::iter::repeat_n(0u16, 40));
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x2A, 1 << 5);
        cpu.poke(0x47, 20);
        cpu.poke(0x48, 8);
        cpu.poke(0x44, 0x21);
        cpu.poke(0x45, 0x09);
        cpu.load_words(&prog);
    }
    step_both(&mut projected, &mut stepped, 42, "down match");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x44, 0x01);
    }
    step_both(&mut projected, &mut stepped, 46, "out tcnt");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x44, 0x21);
        cpu.poke(0x48, 3);
    }
    step_both(&mut projected, &mut stepped, 70, "reenable through top");
}

/// Fast PWM. COM drops, a poke plants OCR0B = 0, COM returns, and one
/// `run_until` crosses BOTTOM, the compare, and the spike. Projected pins
/// match the step.
#[test]
fn fast_pwm_reenable_and_ocr_match_the_stepped_timer() {
    let (mut projected, mut stepped) = fresh();
    let nops = vec![0u16; 220];
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x2A, 1 << 5);
        cpu.poke(0x47, 40);
        cpu.poke(0x48, 15);
        cpu.poke(0x44, 0x23);
        cpu.poke(0x45, 0x09);
        cpu.load_words(&nops);
    }
    step_both(&mut projected, &mut stepped, 50, "first period");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x44, 0x03);
    }
    step_both(&mut projected, &mut stepped, 52, "com off");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x48, 0);
    }
    step_both(&mut projected, &mut stepped, 54, "ocr zero");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x44, 0x23);
    }
    step_both(&mut projected, &mut stepped, 200, "reenable across periods");
}

/// Phase and frequency correct. COM drops, a poke plants OCR1A, and COM
/// returns. One `run_until` crosses BOTTOM. Projected pins match the step.
#[test]
fn phase_freq_reenable_and_ocr_match_the_stepped_timer() {
    let (mut projected, mut stepped) = fresh();
    let nops = vec![0u16; 120];
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x24, 1 << 1);
        cpu.poke(0x87, 0);
        cpu.poke(0x86, 20);
        cpu.poke(0x89, 0);
        cpu.poke(0x88, 8);
        cpu.poke(0x80, 0x80);
        cpu.poke(0x81, 0x11);
        cpu.load_words(&nops);
    }
    step_both(&mut projected, &mut stepped, 24, "down match");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x80, 0x00);
    }
    step_both(&mut projected, &mut stepped, 28, "com off");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x88, 4);
    }
    step_both(&mut projected, &mut stepped, 30, "ocr");
    for cpu in [&mut projected, &mut stepped] {
        cpu.poke(0x80, 0x80);
    }
    step_both(&mut projected, &mut stepped, 100, "reenable across bottom");
}

fn pair(prog: &[u16]) -> (Cpu, Cpu) {
    let (mut projected, mut stepped) = fresh();
    projected.load_words(prog);
    stepped.load_words(prog);
    (projected, stepped)
}

/// Fast PWM, OCR0A = 0. `OUT TCNT0, 0xFF` is cycle 301. Cycle 302 holds
/// `0xFF`. Cycle 303 is TOP→BOTTOM and the compare is blocked, so the set
/// happens and `OCFn` does not. The spike clears on cycle 304 (§15.7.3).
///
/// Six setup instructions, 294 `NOP`s, `LDI`, `OUT`. `OUT DDRD` is cycle 1,
/// `OUT TCCR0A` is cycle 3, `OUT TCCR0B` is cycle 5, and the first /1 clock
/// is cycle 6.
#[test]
fn fast_pwm_blocked_bottom_clears_the_spike_on_the_next_clock() {
    let mut prog = vec![
        ldi(16, 1 << 6),
        out(16, 0x0A),
        ldi(16, 0x83),
        out(16, 0x24),
        ldi(16, 0x01),
        out(16, 0x25),
    ];
    prog.extend(std::iter::repeat_n(0u16, 294));
    prog.push(ldi(16, 0xFF));
    prog.push(out(16, 0x26));
    prog.extend(std::iter::repeat_n(0u16, 280));
    let (mut projected, mut stepped) = pair(&prog);
    step_both(&mut projected, &mut stepped, 580, "blocked bottom");
}

/// Fast PWM from reset, COM0A = 3, OCR0A = 0. The pin is driven low when
/// `OUT DDRD` retires at cycle 2. `OUT TCCR0B` is cycle 5, so the first
/// clock is cycle 6: TCNT0 leaves 0 and the inverted spike goes high.
/// TOP→BOTTOM at cycle 261 drives it low, and the next spike is cycle 262.
#[test]
fn fast_pwm_inverted_spike_from_reset_matches_the_stepped_timer() {
    let mut prog = vec![
        ldi(16, 1 << 6),
        out(16, 0x0A),
        ldi(16, 0xC3),
        out(16, 0x24),
        ldi(16, 0x01),
        out(16, 0x25),
    ];
    prog.extend(std::iter::repeat_n(0u16, 280));
    let (mut projected, mut stepped) = pair(&prog);
    step_both(&mut projected, &mut stepped, 280, "inverted spike");
}

/// `FOC0A` in normal mode with COM0A = 3 sets the register. Fast PWM with
/// COM0A = 2 then starts at TCNT0 = 0. `OUT TCCR0B` is cycle 9, so the
/// first clock is cycle 10 and the spike clears the forced high.
#[test]
fn fast_pwm_spike_after_force_matches_the_stepped_timer() {
    let mut prog = vec![
        ldi(16, 1 << 6),
        out(16, 0x0A),
        ldi(16, 0xC0),
        out(16, 0x24),
        ldi(16, 0x80),
        out(16, 0x25),
        ldi(16, 0x83),
        out(16, 0x24),
        ldi(16, 0x01),
        out(16, 0x25),
    ];
    prog.extend(std::iter::repeat_n(0u16, 300));
    let (mut projected, mut stepped) = pair(&prog);
    step_both(&mut projected, &mut stepped, 300, "spike after force");
}

/// Fast PWM, OCR0A = 50. `OUT TCNT0, 10` is cycle 301, so 302 holds 10 and
/// 303 is 11 with the compare blocked. The match at TCNT0 = 50 is cycle 342,
/// not the clock `block` forced.
#[test]
fn fast_pwm_out_tcnt_away_from_the_match_matches_the_stepped_timer() {
    let mut prog = vec![
        ldi(16, 1 << 6),
        out(16, 0x0A),
        ldi(16, 0x83),
        out(16, 0x24),
        ldi(16, 50),
        out(16, 0x27),
        ldi(16, 0x01),
        out(16, 0x25),
    ];
    prog.extend(std::iter::repeat_n(0u16, 292));
    prog.push(ldi(16, 10));
    prog.push(out(16, 0x26));
    prog.extend(std::iter::repeat_n(0u16, 120));
    let (mut projected, mut stepped) = pair(&prog);
    step_both(&mut projected, &mut stepped, 400, "out tcnt far from match");
}
