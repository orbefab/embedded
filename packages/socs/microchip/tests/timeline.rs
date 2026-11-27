//! The drive on a pin report, `run_to_news`, and the core on a host's timeline.

use crate::common::{Instructions, UCSR0B, UDR0, cbi, in_io, ldi, lds, out, sbi, spin, sts};

use orbe_microchip::{Channel, Cpu, Halt, Level, PinDrive, Timeline};

const PINB: u16 = 0x03;
const DDRB: u16 = 0x04;
const PORTB: u16 = 0x05;
const PIND: u16 = 0x09;
const DDRD: u16 = 0x0A;
const TCCR0A: u16 = 0x24;
const TCCR0B: u16 = 0x25;
const OCR0A: u16 = 0x27;
const CLKPR: u16 = 0x0061;
const ADCSRA: u16 = 0x007A;
const ADMUX: u16 = 0x007C;
const ADCL: u16 = 0x0078;
const ADCH: u16 = 0x0079;
const ICR1L: u16 = 0x0086;
const ICR1H: u16 = 0x0087;
const TCCR1B: u16 = 0x0081;
const EIFR: u16 = 0x1C;
const EIMSK: u16 = 0x1D;
const EICRA: u16 = 0x0069;

const PCICR: u16 = 0x0068;
const PCMSK0: u16 = 0x006B;
const EECR: u16 = 0x1F;
const EEDR: u16 = 0x20;
const EEARL: u16 = 0x21;

const PB0: u32 = 0;
const PB5: u32 = 5;
const PC6: u32 = 14;
const PD0: u32 = 16;
const PD2: u32 = 18;
const PD3: u32 = 19;
const PD4: u32 = 20;
const PD5: u32 = 21;
const PD6: u32 = 22;
const PD7: u32 = 23;

const fn sbis(io: u16, bit: u16) -> u16 {
    0x9B00 | ((io & 0x1F) << 3) | (bit & 7)
}

const fn sbic(io: u16, bit: u16) -> u16 {
    0x9900 | ((io & 0x1F) << 3) | (bit & 7)
}

/// `RJMP` by `k` words from the next one.
const fn rjmp(k: i16) -> u16 {
    0xC000 | (k as u16 & 0x0FFF)
}

const SEI: u16 = 0x9478;
const BREAK: u16 = 0x9598;
const RET: u16 = 0x9508;
const RETI: u16 = 0x9518;

/// `CALL` to word `k`.
const fn call(k: u16) -> [u16; 2] {
    [0x940E, k]
}

/// A 16 MHz crystal (`CKSEL` 1111, `SUT` 11), `RESET` on `PC6`, brown-out at 2.7 V.
fn cpu(words: &[u16]) -> Cpu {
    cpu_fused(words, 0xFF)
}

fn cpu_fused(words: &[u16], low: u8) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.set_fuses(low, 0xD9, 0xFD);
    cpu.load_words(words);
    cpu
}

/// `SBI DDRB,5`, then `SBI PINB,5` every four cycles: PB5 is low at 2 and toggles at 4, 8, 12.
fn toggle() -> Vec<u16> {
    vec![sbi(DDRB, 5), sbi(PINB, 5), rjmp(-2)]
}

/// [`toggle`] with `CLKPS` set to `clkps` by the second `STS`, which retires at CPU cycle 8.
fn clkpr_to(clkps: u16) -> Vec<u16> {
    let mut words = vec![sbi(DDRB, 5), ldi(16, 0x80)];
    words.extend(sts(16, CLKPR));
    words.push(ldi(16, clkps));
    words.extend(sts(16, CLKPR));
    words.extend([sbi(PINB, 5), rjmp(-2)]);
    words
}

/// From /1 to /2.
fn clkpr() -> Vec<u16> {
    clkpr_to(0x01)
}

/// PB5 follows PD2.
fn echo() -> Vec<u16> {
    vec![
        sbi(DDRB, 5),
        sbis(PIND, 2),
        cbi(PORTB, 5),
        sbic(PIND, 2),
        sbi(PORTB, 5),
        rjmp(-5),
    ]
}

/// Every halt up to `limit`, in calls split at `splits`, with `pin`'s drive after each, and
/// the core's whole state at the end.
fn run(words: &[u16], splits: &[u64], limit: u64, pin: u32) -> (Vec<(Halt, PinDrive)>, String) {
    run_fused(words, 0xFF, splits, limit, pin)
}

fn run_fused(
    words: &[u16],
    low: u8,
    splits: &[u64],
    limit: u64,
    pin: u32,
) -> (Vec<(Halt, PinDrive)>, String) {
    let mut timeline = Timeline::new(cpu_fused(words, low));
    let mut halts = Vec::new();
    for &split in splits.iter().chain([&limit]) {
        while let Some(halt) = timeline.run_until(split) {
            halts.push((halt, timeline.drives()[pin as usize]));
            if halt.sample {
                timeline.sample([2.5; 8], 5.0, 5.0);
            }
        }
    }
    (halts, format!("{:?}", timeline.cpu()))
}

fn at(halts: &[(Halt, PinDrive)]) -> Vec<(u64, PinDrive)> {
    halts
        .iter()
        .map(|&(halt, drive)| (halt.at, drive))
        .collect()
}

#[test]
fn a_pull_up_under_a_host_high_still_reports_its_drive() {
    let mut cpu = cpu(&[sbi(PORTB, 5), spin()]);
    cpu.drive_pin(PB5, Level::High);
    cpu.insns(1);
    let report = cpu.take_pin().expect("the drive changed");
    assert_eq!(
        (report.pin, report.level, report.drive, report.cycle),
        (PB5, Level::High, PinDrive::PullUp, 2)
    );
    let mut timeline = Timeline::new(cpu_with_host_high());
    assert_eq!(
        timeline.run_until(100),
        Some(Halt {
            at: 2,
            sample: false,
            refused: false
        })
    );
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::PullUp);
}

fn cpu_with_host_high() -> Cpu {
    let mut cpu = cpu(&[sbi(PORTB, 5), spin()]);
    cpu.drive_pin(PB5, Level::High);
    cpu
}

#[test]
fn pc6_is_reset_and_pulls_up_whatever_its_port_bits_say() {
    let mut words = vec![ldi(16, 0x40)];
    words.extend(sts(16, 0x0027));
    words.extend(sts(16, 0x0028));
    words.push(spin());
    let mut cpu = cpu(&words);
    assert_eq!(cpu.pin_levels().drive(PC6), Some(PinDrive::PullUp));
    cpu.run_until(10);
    assert_eq!(cpu.take_pin(), None, "DDRC6 and PORTC6 leave RESET alone");
    let mut io = Cpu::new();
    io.set_fuses(0xFF, 0x59, 0xFD);
    assert_eq!(io.pin_levels().drive(PC6), Some(PinDrive::Off), "RSTDISBL");
}

#[test]
fn run_to_news_returns_after_each_step_that_reports() {
    let mut cpu = cpu(&toggle());
    let mut stops = Vec::new();
    while cpu.cycles() < 12 {
        cpu.run_to_news(12);
        let report = cpu.take_pin().unwrap();
        stops.push((cpu.cycles(), report.cycle, report.drive));
    }
    assert_eq!(
        stops,
        [
            (2, 2, PinDrive::Low),
            (4, 4, PinDrive::High),
            (8, 8, PinDrive::Low),
            (12, 12, PinDrive::High),
        ]
    );
}

#[test]
fn run_to_news_never_coasts_past_its_limit() {
    let mut held = cpu(&[spin()]);
    held.run_until(100);
    held.pull_reset();
    held.run_to_news(150);
    assert_eq!(held.cycles(), 150, "held in reset");
    held.release_reset();
    for limit in [151, 500, 70_000] {
        held.run_to_news(limit);
        assert_eq!(held.cycles(), limit, "in the reset time-out");
    }
    let mut asleep = cpu(&[ldi(16, 0x01), out(16, 0x33), 0x9588, spin()]);
    asleep.run_to_news(3);
    for limit in [4, 1000] {
        asleep.run_to_news(limit);
        assert!(asleep.is_sleeping());
        assert_eq!(asleep.cycles(), limit, "asleep");
    }
}

#[test]
fn run_to_news_returns_at_a_clkpr_division() {
    let mut cpu = cpu(&clkpr());
    let _ = cpu.take_pin();
    cpu.run_to_news(2);
    let _ = cpu.take_pin();
    cpu.run_to_news(1000);
    assert_eq!((cpu.cycles(), cpu.clock_div()), (8, 2));
    assert_eq!(cpu.take_pin(), None);
}

/// `REFS` = 01 and ADC0, then `ADEN`, `ADSC`, and /128.
fn convert() -> Vec<u16> {
    let mut words = vec![ldi(16, 0x40)];
    words.extend(sts(16, ADMUX));
    words.push(ldi(16, 0xC7));
    words.extend(sts(16, ADCSRA));
    words.push(spin());
    words
}

#[test]
fn an_asked_sample_waits_for_the_host_and_converts_its_answer() {
    let mut cpu = cpu(&convert());
    cpu.set_channel_voltage(0, 1.0);
    cpu.ask_for_samples();
    cpu.run_to_news(100_000);
    let due = cpu.sample_due().expect("the sample-and-hold asks");
    // The first conversion samples 13.5 ADC clocks after its start (Table 24-1), 1,728 CPU
    // cycles at /128.
    assert!(due > 1_728 && due <= cpu.cycles(), "{due}");
    cpu.answer_sample([3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 5.0, 0.0);
    assert_eq!(cpu.sample_due(), None);
    cpu.run_until(100_000);
    let code = u16::from(cpu.peek(ADCL)) | u16::from(cpu.peek(ADCH)) << 8;
    assert_eq!(code, 614, "3 V of a 5 V AVCC, not the 1 V store");
}

#[test]
fn a_timeline_halts_at_each_drive_change_in_oscillator_cycles() {
    let (halts, _) = run(&toggle(), &[], 13, PB5);
    assert_eq!(
        at(&halts),
        [
            (2, PinDrive::Low),
            (4, PinDrive::High),
            (8, PinDrive::Low),
            (12, PinDrive::High),
        ]
    );
}

/// Low fuse `0x7F` programs `CKDIV8`: /8 from power-on, then /1 from the second `STS`.
const CKDIV8: u8 = 0x7F;

fn speed_up() -> Vec<u16> {
    clkpr_to(0x00)
}

/// §9.11 puts the new rate in force `T1 + T2` to `T1 + 2·T2` after the `STS` that writes it
/// retires, at oscillator cycle `E`, with two edges in between. The `SBI` that follows is those
/// two edges, so its toggle must land in that window whichever way the clock changes.
///
/// From /1 to /2, `E` is 8 and the window 11 to 13: the toggle is at 11, then every four CPU
/// cycles is eight oscillator cycles. From /8 to /1, `E` is 64 and the window 73 to 74: the
/// toggle is at 73, then every four.
#[test]
fn a_clkpr_division_lands_inside_its_window_either_way() {
    let times = |halts: Vec<(Halt, PinDrive)>| -> Vec<u64> {
        halts.iter().map(|(halt, _)| halt.at).collect()
    };
    let (slower, _) = run(&clkpr(), &[], 46, PB5);
    assert_eq!(times(slower), [2, 11, 19, 27, 35, 43]);
    let (faster, _) = run_fused(&speed_up(), CKDIV8, &[], 90, PB5);
    assert_eq!(times(faster), [16, 73, 77, 81, 85, 89]);
}

#[test]
fn a_run_split_at_any_cycle_is_the_same_run() {
    for (words, limit) in [(toggle(), 60), (clkpr(), 80), (convert(), 2_000)] {
        let whole = run(&words, &[], limit, PB5);
        assert!(!whole.0.is_empty());
        for split in 0..limit {
            assert_eq!(run(&words, &[split], limit, PB5), whole, "split at {split}");
        }
    }
    let whole = run_fused(&speed_up(), CKDIV8, &[], 120, PB5);
    for split in 0..120 {
        let parts = run_fused(&speed_up(), CKDIV8, &[split], 120, PB5);
        assert_eq!(parts, whole, "split at {split}");
    }
}

#[test]
fn a_sample_halts_where_the_core_samples_and_only_once() {
    let mut timeline = Timeline::new(cpu(&convert()));
    let halt = timeline.run_until(10_000).unwrap();
    assert!(halt.sample, "{halt:?}");
    timeline.sample([1.0; 8], 5.0, 5.0);
    assert_eq!(timeline.run_until(10_000), None);
}

#[test]
#[should_panic(expected = "answer the sample before running on")]
fn a_run_past_an_unanswered_sample_panics() {
    let mut timeline = Timeline::new(cpu(&convert()));
    assert!(timeline.run_until(10_000).unwrap().sample);
    timeline.run_until(10_000);
}

#[test]
fn an_input_acts_only_after_where_the_host_reached() {
    let mut timeline = Timeline::new(cpu(&echo()));
    while timeline.run_until(100).is_some() {}
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::Low);
    timeline.set_pad(PD2, true);
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::Low);
    let halt = timeline.run_until(1_000).unwrap();
    assert!(halt.at > 100 + 2, "after the synchronizer: {halt:?}");
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::High);
}

#[test]
fn a_brown_out_holds_every_pin_off_until_vcc_returns() {
    let mut timeline = Timeline::new(cpu(&toggle()));
    while timeline.run_until(101).is_some() {}
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::High);
    timeline.set_brown_out(true);
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::High);
    assert_eq!(timeline.run_until(10_000).map(|halt| halt.at), Some(102));
    assert!(timeline.held(), "held from the halt");
    for (pin, drive) in timeline.drives().iter().enumerate() {
        let expected = if pin as u32 == PC6 {
            PinDrive::PullUp
        } else {
            PinDrive::Off
        };
        assert_eq!(*drive, expected, "pin {pin}");
    }
    assert_eq!(timeline.run_until(2_000_000), None, "held while VCC is low");
    timeline.set_brown_out(false);
    assert_eq!(
        timeline.cpu().vcc(),
        5.0,
        "the host's window leaves the supply alone"
    );
    let released = timeline.run_until(4_000_000).unwrap();
    assert!(
        released.at > 2_000_001,
        "after the reset time-out: {released:?}"
    );
    assert!(!timeline.held());
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::Off);
    let halt = timeline.run_until(4_000_000).unwrap();
    assert!(halt.at > released.at, "after the reset time-out: {halt:?}");
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::Low);
}

#[test]
fn reset_low_on_pc6_holds_the_chip_and_high_restarts_it() {
    let mut timeline = Timeline::new(cpu(&toggle()));
    while timeline.run_until(100).is_some() {}
    timeline.set_pad(PC6, false);
    assert_eq!(timeline.run_until(1_000).map(|halt| halt.at), Some(101));
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::Off);
    assert_eq!(timeline.drives()[PC6 as usize], PinDrive::PullUp);
    assert!(timeline.cpu().is_held_in_reset());
    assert!(timeline.held(), "held from the halt");
    assert_eq!(timeline.run_until(2_000_000), None);
    timeline.set_pad(PC6, true);
    let released = timeline.run_until(4_000_000).unwrap();
    assert!(
        released.at > 2_000_001,
        "reset holds through the start-up time-out: {released:?}"
    );
    assert!(!timeline.held());
    assert!(timeline.run_until(4_000_000).is_some());
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::Low);
}

#[test]
fn a_reset_keeps_the_pads_the_host_drives() {
    let mut timeline = Timeline::new(cpu(&echo()));
    timeline.set_pad(PD2, true);
    while timeline.run_until(100).is_some() {}
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::High);
    timeline.set_pad(PC6, false);
    while timeline.run_until(200).is_some() {}
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::Off);
    timeline.set_pad(PC6, true);
    while timeline.run_until(4_000_000).is_some() {}
    assert_eq!(timeline.drives()[PB5 as usize], PinDrive::High);
}

/// Timer1 at /1 capturing a rising `ICP1` on `PB0`, under a main loop of `body` and an `RJMP`.
fn capture(body: &[u16]) -> Vec<u16> {
    let mut words = vec![ldi(16, 0x41)];
    words.extend(sts(16, TCCR1B));
    words.extend_from_slice(body);
    words.push(rjmp(-(body.len() as i16 + 1)));
    words
}

fn icr1(words: &[u16], pad_at: u64) -> u16 {
    let mut timeline = Timeline::new(cpu(words));
    while timeline.run_until(pad_at).is_some() {}
    timeline.set_pad(PB0, true);
    while timeline.run_until(pad_at + 200).is_some() {}
    u16::from(timeline.cpu().peek(ICR1L)) | u16::from(timeline.cpu().peek(ICR1H)) << 8
}

/// The `TCCR1B` store retires at cycle 3, so `TCNT1` is `c - 3` at cycle `c`. A pad edge at `c`
/// is captured 4 cycles later (§16.6.1), so `ICR1` is `c + 1`, under a loop of one-cycle `NOP`s
/// or two-cycle `LDS`es alike (§14.2.4: only the synchronizer adds latency).
#[test]
fn a_pad_edge_is_captured_at_its_cycle_whatever_instruction_runs_across_it() {
    let nops = capture(&[0; 8]);
    let lds: Vec<u16> = (0..4).flat_map(|_| lds(17, 0x0100)).collect();
    let lds = capture(&lds);
    for pad_at in 40..52 {
        let expected = pad_at as u16 + 1;
        assert_eq!(icr1(&nops, pad_at), expected, "NOP loop, pad at {pad_at}");
        assert_eq!(icr1(&lds, pad_at), expected, "LDS loop, pad at {pad_at}");
    }
}

/// Under `CKDIV8`, Timer1 at /1 from CPU cycle 3, then `CLKPS` cleared by an `STS` on CPU cycles
/// 7 and 8 that retires at 9, oscillator 72. One more cycle at /8, then /1 from CPU cycle 10,
/// oscillator 80. A pad edge at oscillator cycle `osc` is on the CPU cycle that holds it, and
/// `ICR1` is that cycle plus 1, inside the store as anywhere else.
#[test]
fn a_pad_edge_inside_the_clkpr_store_is_captured_at_its_cycle() {
    let mut words = vec![ldi(16, 0x41)];
    words.extend(sts(16, TCCR1B));
    words.push(ldi(16, 0x80));
    words.extend(sts(16, CLKPR));
    words.push(ldi(16, 0x00));
    words.extend(sts(16, CLKPR));
    words.push(spin());
    for osc in 48..96 {
        let mut timeline = Timeline::new(cpu_fused(&words, CKDIV8));
        assert_eq!(timeline.run_until(osc), None);
        timeline.set_pad(PB0, true);
        assert_eq!(timeline.run_until(osc + 200), None);
        let cpu = timeline.cpu();
        let icr1 = u16::from(cpu.peek(ICR1L)) | u16::from(cpu.peek(ICR1H)) << 8;
        let cycle = if osc < 80 { osc / 8 } else { 10 + osc - 80 };
        assert_eq!(icr1, cycle as u16 + 1, "pad at oscillator {osc}");
    }
}

/// `INT0` on a rising `PD2`: `EICRA` = 0x03 and `EIMSK` = 0x01, from CPU cycle 5.
fn int0_rising() -> Vec<u16> {
    int0_on(0x03)
}

/// `INT0` with `EICRA` = `isc`.
fn int0_on(isc: u16) -> Vec<u16> {
    let mut words = vec![ldi(16, isc)];
    words.extend(sts(16, EICRA));
    words.extend([ldi(16, 0x01), out(16, EIMSK)]);
    words
}

/// With `I` clear, a `CALL` runs CPU cycles 10 to 14 and its subroutine reads `EIFR`. A pad edge
/// at `pad` sets `INTF0` at `pad + 2` (§14.2.4, §13.2.3), so the `IN` at 14 reads it for every
/// `pad` up to 12, including those inside the `CALL`.
#[test]
fn a_flag_the_synchronizer_set_inside_an_instruction_is_read_by_the_next() {
    let mut words = int0_rising();
    words.extend([0; 5]);
    let sub = words.len() as u16 + 2;
    words.extend(call(sub));
    words.extend([in_io(17, EIFR), spin()]);
    for pad in 10..=13 {
        let mut timeline = Timeline::new(cpu(&words));
        timeline.set_pad(PD2, false);
        assert_eq!(timeline.run_until(pad), None);
        timeline.set_pad(PD2, true);
        assert_eq!(timeline.run_until(15), None);
        let read = timeline.cpu().reg(17);
        assert_eq!(read, u8::from(pad + 2 <= 14), "pad at {pad}");
    }
}

/// The `INT0` vector toggles `PB5` and returns. A pad edge at `pad` sets `INTF0` at `pad + 2`,
/// the first instruction boundary at or after it takes the interrupt (§7.7.1), and the toggle is
/// that boundary plus 4 cycles of entry and 2 of `SBI`. Under one-cycle `NOP`s that is `pad + 8`.
/// Under a `CALL`, `RET`, `RJMP` loop whose boundaries are 21, 25, 29, 31, 35, 39, 41, 45, …,
/// it is the same rule on those boundaries.
#[test]
fn an_interrupt_is_taken_at_the_first_boundary_after_its_flag() {
    let mut calls = isr_toggles(0x03);
    let mut nops = calls.clone();
    nops.extend([0; 200]);
    let top = calls.len() as u16;
    calls.extend(call(top + 3));
    calls.extend([rjmp(-3), RET]);
    let toggle = |words: &[u16], pad: u64| {
        let mut timeline = Timeline::new(cpu(words));
        timeline.set_pad(PD2, false);
        while timeline.run_until(pad).is_some() {}
        timeline.set_pad(PD2, true);
        timeline.run_until(pad + 40).map(|halt| halt.at)
    };
    for (pad, at) in [(28, 37), (30, 41), (32, 41), (36, 45), (40, 51)] {
        assert_eq!(toggle(&nops, pad), Some(pad + 8), "NOP sled, pad at {pad}");
        assert_eq!(toggle(&calls, pad), Some(at), "CALL loop, pad at {pad}");
    }
}

/// The `INT0` vector toggles `PB5` and returns; `EICRA` = `isc`, then `SEI` and one `NOP` end
/// at CPU cycle 11.
fn isr_toggles(isc: u16) -> Vec<u16> {
    let mut words = vec![rjmp(3), 0, sbi(PINB, 5), RETI, sbi(DDRB, 5)];
    words.extend(int0_on(isc));
    words.extend([SEI, 0]);
    words
}

/// `INT0` on any change, under `NOP`s. A rise at 30 is taken at 32 and toggles at 38, and its
/// `RETI` runs 38 to 42. A fall at 39 sets `INTF0` at 41, inside that `RETI`, and §7.7.1 runs one
/// more instruction after a `RETI`: the `NOP` at 42, then the interrupt at 43, toggling at 49.
#[test]
fn reti_still_runs_one_instruction_before_an_overdue_interrupt() {
    let mut words = isr_toggles(0x01);
    words.extend([0; 200]);
    let mut timeline = Timeline::new(cpu(&words));
    timeline.set_pad(PD2, false);
    while timeline.run_until(30).is_some() {}
    timeline.set_pad(PD2, true);
    let first = timeline.run_until(39).map(|halt| halt.at);
    assert_eq!((first, timeline.run_until(39)), (Some(38), None));
    timeline.set_pad(PD2, false);
    assert_eq!(timeline.run_until(80).map(|halt| halt.at), Some(49));
}

const fn inc(d: u16) -> u16 {
    0x9403 | (d << 4)
}

const fn mov(d: u16, r: u16) -> u16 {
    0x2C00 | ((r & 0x10) << 5) | (d << 4) | (r & 0xF)
}

/// `INT0` (vector 1) on a rising `PD2` writes the order it ran in to `r21`, and `PCINT2`
/// (vector 5) on `PD0` to `r22`. Then `SEI` and a `NOP` sled, or a `CALL`, `RJMP`, `RET` loop.
fn two_sources(sled: bool) -> Vec<u16> {
    let mut words = vec![rjmp(17), 0, rjmp(9), 0, 0, 0, 0, 0, 0, 0, rjmp(4), 0];
    words.extend([inc(20), mov(21, 20), RETI]);
    words.extend([inc(20), mov(22, 20), RETI]);
    words.extend(int0_rising());
    words.push(ldi(16, 0x04));
    words.extend(sts(16, 0x68));
    words.push(ldi(16, 0x01));
    words.extend(sts(16, 0x6D));
    words.push(SEI);
    if sled {
        words.extend([0; 300]);
    } else {
        let top = words.len() as u16;
        words.extend(call(top + 3));
        words.extend([rjmp(-3), RET]);
    }
    words
}

/// `INTF0` and `PCIF2` rise on the same synchronizer edge, so the lower vector runs first
/// (§7.7.1), whichever pad the host drives first and wherever its deadline falls.
#[test]
fn inputs_at_one_instant_are_taken_by_vector_whatever_order_they_land() {
    for sled in [true, false] {
        for pad in 30..60 {
            for order in [[PD0, PD2], [PD2, PD0]] {
                let mut timeline = Timeline::new(cpu(&two_sources(sled)));
                while timeline.run_until(pad).is_some() {}
                for pin in order {
                    timeline.set_pad(pin, true);
                }
                while timeline.run_until(pad + 80).is_some() {}
                let ran = (timeline.cpu().reg(21), timeline.cpu().reg(22));
                assert_eq!(ran, (1, 2), "sled {sled}, pad at {pad}, {order:?}");
            }
        }
    }
}

/// A brown-out in the same input as a `PD2` rise lands where it lands alone: `PB5` goes `Off`
/// at the boundary the core sits on, 35 for a pad at 32 and 39 for 36, with no `INT0` entry
/// before it.
#[test]
fn a_brown_out_with_an_interrupt_pad_in_one_input_lands_where_it_lands_alone() {
    let mut calls = isr_toggles(0x03);
    let top = calls.len() as u16;
    calls.extend(call(top + 3));
    calls.extend([rjmp(-3), RET]);
    for (pad, off) in [(32, 35), (36, 39)] {
        for with_pad in [false, true] {
            let mut timeline = Timeline::new(cpu(&calls));
            timeline.set_pad(PD2, false);
            while timeline.run_until(pad).is_some() {}
            if with_pad {
                timeline.set_pad(PD2, true);
            }
            timeline.set_brown_out(true);
            let halt = timeline.run_until(pad + 40);
            let landed = halt.map(|halt| (halt.at, timeline.drives()[PB5 as usize]));
            assert_eq!(
                landed,
                Some((off, PinDrive::Off)),
                "pad at {pad}, {with_pad}"
            );
        }
    }
}

/// `INTF0` set on the `BREAK`'s own cycle is never taken: an input on another pin, or a run,
/// leaves the stopped core where it stopped.
#[test]
fn a_stopped_core_takes_no_interrupt() {
    let mut words = isr_toggles(0x03);
    words.extend([0, 0, 0, 0, 0, BREAK, 0, 0]);
    for pad in 9..20 {
        let mut timeline = Timeline::new(cpu(&words));
        timeline.set_pad(PD2, false);
        while timeline.run_until(pad).is_some() {}
        timeline.set_pad(PD2, true);
        while timeline.run_until(60).is_some() {}
        let stopped = (timeline.cpu().cycles(), timeline.cpu().pc());
        timeline.set_pad(PB0, true);
        let mut cpu = timeline.cpu().clone();
        cpu.run_until(100);
        assert!(cpu.is_stopped(), "pad at {pad}");
        assert_eq!((cpu.cycles(), cpu.pc()), stopped, "pad at {pad}");
    }
}

/// The `INT0` vector toggles `PB5`, and timer 0 in CTC toggles `OC0A` (`PD6`) every 4 cycles,
/// under a `CALL`, `RJMP`, `RET` loop.
fn int0_beside_a_toggling_timer() -> Vec<u16> {
    let mut words = vec![rjmp(3), 0, sbi(PINB, 5), RETI, sbi(DDRB, 5), sbi(DDRD, 6)];
    words.extend([ldi(16, 0x42), out(16, TCCR0A), ldi(16, 3), out(16, OCR0A)]);
    words.extend([ldi(16, 0x01), out(16, TCCR0B)]);
    words.extend(int0_on(0x03));
    words.push(SEI);
    let top = words.len() as u16;
    words.extend(call(top + 3));
    words.extend([rjmp(-3), RET]);
    words
}

/// An interrupt already overdue when a run starts is entered first, and an `OC0A` edge inside
/// that entry stops the run there. A host that runs to the halt's own cycle and one that runs
/// far past it see the same halt, and a brown-out there lands the same way for both.
#[test]
fn an_overdue_interrupt_entry_stops_for_its_own_news() {
    let mut overdue = 0;
    for pad in 20..80 {
        let mut after_brown_out = |first: u64| {
            let mut timeline = Timeline::new(cpu(&int0_beside_a_toggling_timer()));
            timeline.set_pad(PD2, false);
            while timeline.run_until(pad).is_some() {}
            if timeline.cpu().cycles() >= pad + 2 {
                overdue += 1;
            }
            timeline.set_pad(PD2, true);
            let halt = timeline.run_until(first).map(|halt| halt.at);
            timeline.set_brown_out(true);
            let mut landed = Vec::new();
            while let Some(halt) = timeline.run_until(pad + 60) {
                let drives = timeline.drives();
                landed.push((halt.at, drives[PB5 as usize], drives[PD6 as usize]));
            }
            (halt, landed)
        };
        let far = after_brown_out(pad + 60);
        let Some(at) = far.0 else { continue };
        assert_eq!(after_brown_out(at), far, "pad at {pad}");
    }
    assert!(
        overdue > 0,
        "no pad left INTF0 overdue at the start of a run"
    );
}

/// Timer 1 captures on `ICP1` and counts `T1`, timer 0 counts `T0`, `INT0`, `INT1` and `PCINT0`
/// are armed with no interrupt enabled, and one EEPROM write is in flight. The loop counts in
/// SRAM, sends the count on the USART, and toggles PB5.
fn every_queue() -> Vec<u16> {
    let mut words = vec![sbi(DDRB, 5), ldi(16, 0x47)];
    words.extend(sts(16, TCCR1B));
    words.extend([ldi(16, 0x07), out(16, TCCR0B), ldi(16, 0x05)]);
    words.extend(sts(16, EICRA));
    words.extend([ldi(16, 0x03), out(16, EIMSK), ldi(16, 0x01)]);
    words.extend(sts(16, PCICR));
    words.extend(sts(16, PCMSK0));
    words.push(ldi(16, 0x08));
    words.extend(sts(16, UCSR0B));
    words.extend([ldi(16, 0x5A), out(16, EEDR), ldi(16, 3), out(16, EEARL)]);
    words.extend([sbi(EECR, 2), sbi(EECR, 1)]);
    let top = words.len();
    words.extend(lds(17, 0x0100));
    words.push(inc(17));
    words.extend(sts(17, 0x0100));
    words.extend(sts(17, UDR0));
    words.push(sbi(PINB, 5));
    words.push(rjmp(top as i16 - words.len() as i16 - 1));
    words
}

/// The pins `every_queue` arms: `ICP1`, `INT0`, `INT1`, `T0` and `T1`.
const ARMED: [u32; 5] = [PB0, PD2, PD3, PD4, PD5];

/// `clone_from` over an earlier state of the same run, and back, leaves the state `clone` does.
fn assert_clone_from<T: Clone + std::fmt::Debug>(early: &T, late: &T) {
    for (onto, from) in [(early, late), (late, early)] {
        let mut copy = onto.clone();
        copy.clone_from(from);
        assert_eq!(format!("{copy:?}"), format!("{:?}", from.clone()));
    }
}

/// On a timeline, which takes each pin report as it runs. Each state has a rising edge waiting
/// on every armed pin, and the EEPROM write lands between the two.
#[test]
fn a_clone_from_leaves_the_state_a_clone_does() {
    let mut timeline = Timeline::new(cpu(&every_queue()));
    let mut states = Vec::new();
    for until in [2_000, 70_000] {
        for (high, at) in [(false, until - 8), (true, until)] {
            while timeline.run_until(at).is_some() {}
            for pin in ARMED {
                timeline.set_pad(pin, high);
            }
        }
        states.push(timeline.clone());
    }
    assert_ne!(states[0].cpu().eeprom(), states[1].cpu().eeprom());
    assert_clone_from(&states[0], &states[1]);
}

/// On a bare core, whose pin reports and serial bytes wait for the host.
#[test]
fn a_clone_from_over_untaken_reports_leaves_the_state_a_clone_does() {
    let mut cpu = cpu(&every_queue());
    let mut states = Vec::new();
    for until in [2_000, 70_000] {
        for (level, at) in [(Level::Low, until - 8), (Level::High, until)] {
            cpu.run_until(at);
            for pin in ARMED {
                cpu.drive_pin(pin, level);
            }
        }
        states.push(cpu.clone());
    }
    for state in &states {
        let mut state = state.clone();
        assert!(state.take_pin().is_some() && state.take(Channel::Usart0).is_some());
    }
    assert_clone_from(&states[0], &states[1]);
}

/// Each pass counts in r17 and writes the count to `GPIOR0`, the EEPROM address and data, the
/// three timers, the comparator, the external and pin-change interrupts, and `PRR`. Two states
/// of one run differ in each.
fn every_register() -> Vec<u16> {
    const SET: [u16; 28] = [
        0x3E, 0x41, 0x40, 0x44, 0x45, 0x47, 0x48, 0x6E, 0x80, 0x81, 0x89, 0x88, 0x87, 0x86, 0x6F,
        0xB0, 0xB1, 0xB3, 0xB6, 0x70, 0x50, 0x69, 0x3D, 0x68, 0x6B, 0x6C, 0x6D, 0x64,
    ];
    let mut words = vec![inc(17)];
    for addr in SET {
        words.extend(sts(17, addr));
    }
    words.push(rjmp(-(words.len() as i16) - 1));
    words
}

/// Runs `cpu` to oscillator cycle `until` in eight chunks. Each chunk answers the samples it
/// halts on with its own levels, then flips `pads`.
fn ran(cpu: Cpu, until: u64, pads: &[u32]) -> Timeline {
    let mut timeline = Timeline::new(cpu);
    for k in 1..=8u64 {
        while let Some(halt) = timeline.run_until(until * k / 8) {
            if halt.sample {
                timeline.sample([0.4 * k as f32; 8], 5.0, 5.0);
            }
        }
        for &pad in pads {
            timeline.set_pad(pad, k % 2 == 0);
        }
    }
    timeline
}

/// Runs `words` on the 2.7 V brown-out fuses to oscillator cycle `until`. A pad in `pads`
/// rises at `until - 2`.
fn stopped(words: &[u16], until: u64, pads: &[u32]) -> Timeline {
    let mut timeline = Timeline::new(cpu(words));
    while timeline.run_until(until - 2).is_some() {}
    for &pad in pads {
        timeline.set_pad(pad, true);
    }
    while timeline.run_until(until).is_some() {}
    timeline
}

/// States a host stops in for a few cycles at most: inside the `BODSE`, `BODS` and `EEMPE`
/// windows, asleep in power-down, timer 0 counting down past a compare in phase-correct PWM, a
/// compare blocked by a `TCNT0` write, a compare flag waiting for the next timer clock, `OC0A`
/// toggled by a compare, and the `ICP1` noise canceler before and after its four samples.
fn corners() -> Vec<Timeline> {
    const MCUCR: u16 = 0x35;
    const SMCR: u16 = 0x33;
    const TCNT0: u16 = 0x26;
    const SLEEP: u16 = 0x9588;
    let bods = [
        ldi(16, 0x60),
        out(16, MCUCR),
        ldi(16, 0x40),
        out(16, MCUCR),
        rjmp(-1),
    ];
    let eempe = [ldi(16, 0x04), out(16, EECR), rjmp(-1)];
    let power_down = [ldi(16, 0x05), out(16, SMCR), SLEEP, rjmp(-1)];
    let phase = [
        ldi(16, 100),
        out(16, OCR0A),
        ldi(16, 1),
        out(16, TCCR0A),
        out(16, TCCR0B),
        rjmp(-1),
    ];
    let blocked = [ldi(16, 5), out(16, TCCR0B), out(16, TCNT0), rjmp(-1)];
    let due = [
        ldi(16, 1),
        out(16, OCR0A),
        ldi(16, 5),
        out(16, TCCR0B),
        rjmp(-1),
    ];
    let toggle = [
        ldi(16, 0x42),
        out(16, TCCR0A),
        ldi(16, 3),
        out(16, OCR0A),
        ldi(16, 1),
        out(16, TCCR0B),
        rjmp(-1),
    ];
    let mut canceler = vec![ldi(16, 0x81)];
    canceler.extend(sts(16, TCCR1B));
    canceler.push(rjmp(-1));
    let mut settled = stopped(&canceler, 100, &[PB0]);
    while settled.run_until(200).is_some() {}
    vec![
        stopped(&bods, 3, &[]),
        stopped(&bods, 5, &[]),
        stopped(&eempe, 3, &[]),
        stopped(&power_down, 50, &[]),
        stopped(&phase, 450, &[]),
        stopped(&blocked, 100, &[]),
        stopped(&due, 1_500, &[]),
        stopped(&toggle, 101, &[]),
        stopped(&toggle, 105, &[]),
        stopped(&canceler, 100, &[PB0]),
        settled,
    ]
}

/// The image at `path`, from the repository root.
fn image(path: &str) -> Cpu {
    let path = format!("{}/../../../{path}", env!("CARGO_MANIFEST_DIR"));
    let image = orbe_microchip::parse_hex(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut cpu = Cpu::new();
    cpu.load_flash(&image);
    cpu
}

/// `clone_from` between any two of: a fresh core, the shipped and compiled firmwares,
/// [`every_register`] at three times with comparator volts, host SPI queued and a TWI START under way,
/// [`every_queue`] with an edge waiting on each armed pin, the [`corners`], an ADC sample the
/// host has not answered, a slower, lower, brown-out core with a host baud, a core whose `EECR`
/// a host poked, and a core stopped one instruction short of a pin write. Each pair leaves the
/// state `clone` does. A host may copy any core over any other.
#[test]
fn a_clone_from_across_runs_leaves_the_state_a_clone_does() {
    const MS: u64 = 16_000;
    let mut states = vec![Timeline::new(Cpu::new())];
    for (name, pads) in [
        (
            "packages/engine/tests/firmware/sketch/analog_read_serial.hex",
            &[][..],
        ),
        ("packages/engine/tests/firmware/sketch/blink.hex", &[]),
        (
            "packages/engine/tests/firmware/sketch/blink_without_delay.hex",
            &[],
        ),
        ("blocks/nano-button-led/build/button.ino.hex", &[PD2]),
        ("blocks/nano-led-fade/build/fade.ino.hex", &[]),
        (
            "packages/engine/tests/firmware/sketch/serial_counter.hex",
            &[PD0],
        ),
        ("packages/engine/tests/firmware/sketch/tone.hex", &[]),
        ("packages/engine/tests/firmware/adc.hex", &[]),
        ("packages/engine/tests/firmware/echo.hex", &[PD0]),
        ("packages/engine/tests/firmware/speedup.hex", &[]),
    ] {
        states.push(ran(image(name), 3 * MS, pads));
    }
    for until in [700, 1_900, 4_100] {
        let mut cpu = cpu(&every_register());
        cpu.set_voltage(PD6, 3.0);
        cpu.set_voltage(PD7, 1.0);
        let clock = orbe_microchip::SpiClock {
            period: 8,
            mode: 0,
            lsb_first: false,
        };
        assert!(cpu.queue_spi_master(until / 2, &[0xA5], clock));
        // TWBR 255 and TWPS 3 make a START of 32_656 cycles, so each run stops inside it.
        cpu.poke(0x00B8, 0xFF);
        cpu.poke(0x00B9, 0x03);
        cpu.poke(0x00BC, 0xA4);
        states.push(ran(cpu, until, &[PB0, PD4, PD5]));
    }
    states.push(ran(cpu(&every_queue()), 70_000, &ARMED));
    states.extend(corners());
    let mut asked = Timeline::new(image("packages/engine/tests/firmware/adc.hex"));
    while !asked.run_until(MS).is_some_and(|halt| halt.sample) {}
    states.push(asked);
    let mut slow = cpu(&every_queue());
    slow.set_clock_hz(8_000_000);
    slow.set_vcc(3.3);
    slow.set_host_serial(
        Channel::Usart0,
        9_600,
        orbe_microchip::SerialFrame::EIGHT_N_1,
    );
    let mut slow = ran(slow, 2 * MS, &ARMED);
    slow.set_brown_out(true);
    while slow.run_until(3 * MS).is_some() {}
    states.push(slow);
    states.extend(eeprom_pokes());
    states.push(short_of_a_pin_write());
    let want: Vec<String> = states
        .iter()
        .map(|state| format!("{:?}", state.clone()))
        .collect();
    for (i, onto) in states.iter().enumerate() {
        for (j, from) in states.iter().enumerate() {
            let mut copy = onto.clone();
            copy.clone_from(from);
            assert!(format!("{copy:?}") == want[j], "state {j} over state {i}");
        }
    }
}

/// Data-space `EECR`. `0x01` is `EERE`. `0xFF` is the other poke that leaves the four-cycle
/// halt set. Both also leave the register window set until the next instruction.
fn eeprom_pokes() -> Vec<Timeline> {
    const EECR_DATA: u16 = 0x003F;
    [0x01, 0xFF]
        .map(|value| {
            let mut cpu = cpu(&[0x0000]);
            cpu.run_until(4);
            cpu.poke(EECR_DATA, value);
            Timeline::new(cpu)
        })
        .into()
}

/// A host pokes `EECR`, copies that core over a fresh one, and steps both. The copy takes the
/// halt the poke left, so the two cores match after the step as well as before it.
#[test]
fn a_clone_from_after_an_eeprom_poke_leaves_the_state_a_clone_does() {
    for poked in eeprom_pokes() {
        let mut copy = Timeline::new(Cpu::new());
        copy.clone_from(&poked);
        assert_eq!(format!("{copy:?}"), format!("{:?}", poked.clone()));
        let mut stepped = poked.clone();
        let halt = stepped.run_until(stepped.cpu().cycles() + 1);
        let copied = copy.run_until(copy.cpu().cycles() + 1);
        assert_eq!(
            (copied, format!("{copy:?}")),
            (halt, format!("{stepped:?}"))
        );
    }
}

/// `SBI DDRB,5` drives PB5 at cycle 2. The host stops at cycle 1, so that drive waits past
/// the cycle it reached.
fn short_of_a_pin_write() -> Timeline {
    let mut timeline = Timeline::new(cpu(&[sbi(DDRB, 5), rjmp(-1)]));
    assert!(
        timeline.run_until(1).is_none(),
        "PB5's drive is still ahead of the cycle the host reached"
    );
    timeline
}

/// Stopped one instruction short of a pin write, `clone_from` over a core that has not run
/// leaves the state `clone` does. The waiting drive is in this crate.
#[test]
fn a_clone_from_short_of_a_pin_write_leaves_the_state_a_clone_does() {
    let waiting = short_of_a_pin_write();
    let fresh = Timeline::new(Cpu::new());
    assert_ne!(format!("{waiting:?}"), format!("{fresh:?}"));
    assert_clone_from(&fresh, &waiting);
}
