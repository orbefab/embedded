//! ADC and analog comparator, DS40002061B §23 and §24.
//!
//! Counts are the datasheet tables, not a trace copied out of a run.
//! A 1-cycle `NOP` is one `clkI/O`. The ADC prescaler starts when `ADEN` is
//! written and produces a rising edge every `N` CPU clocks after that
//! (§24.4). The first edge is `N` clocks after the write.

use crate::common;
use common::Instructions;

use common::{SREG, TCCR0B, TIFR0, TIFR1, ldi};
use orbe_microchip::{Cpu, FuseBytes, Level, Port};

const ADCL: u16 = 0x78;
const ADCH: u16 = 0x79;
const ADCSRA: u16 = 0x7A;
const ADCSRB: u16 = 0x7B;
const ADMUX: u16 = 0x7C;
const DIDR0: u16 = 0x7E;
const DIDR1: u16 = 0x7F;
const ACSR: u16 = 0x50;
const PINC: u16 = 0x26;
const PIND: u16 = 0x29;
const EICRA: u16 = 0x69;
const EIFR: u16 = 0x3C;
const TCCR0A: u16 = 0x44;
const TCCR1B: u16 = 0x81;
const OCR0A: u16 = 0x47;

const ADEN: u8 = 1 << 7;
const ADSC: u8 = 1 << 6;
const ADATE: u8 = 1 << 5;
const ADIF: u8 = 1 << 4;
const ADIE: u8 = 1 << 3;
const ACO: u8 = 1 << 5;
const ACI: u8 = 1 << 4;
const ACIE: u8 = 1 << 3;
const ACIC: u8 = 1 << 2;
const ACBG: u8 = 1 << 6;
const ACME: u8 = 1 << 6;

const PC0: u32 = 8;
const PC1: u32 = 9;
const AIN0: u32 = 22;
const AIN1: u32 = 23;

fn div_of(adps: u8) -> u64 {
    [2, 2, 4, 8, 16, 32, 64, 128][usize::from(adps)]
}

/// Smallest edge `origin + kN` strictly after `after`.
fn edge_after(origin: u64, div: u64, after: u64) -> u64 {
    edge_at(origin, div, after.saturating_add(1))
}

/// Figure 24-3. First rise is `div/2` after the reset, then every `div`.
fn edge_at(origin: u64, div: u64, target: u64) -> u64 {
    let first = origin + div / 2;
    if target <= first {
        return first;
    }
    let steps = (target - first).div_ceil(div);
    first + steps * div
}

fn fresh() -> Cpu {
    let mut cpu = Cpu::new();
    // Table 24-3, `REFS = 01`: AVCC, which defaults to 5.0 V.
    cpu.poke(ADMUX, 0x40);
    cpu
}

fn adsc(cpu: &Cpu) -> bool {
    cpu.peek(ADCSRA) & ADSC != 0
}

fn adif(cpu: &Cpu) -> bool {
    cpu.peek(ADCSRA) & ADIF != 0
}

fn result(cpu: &Cpu) -> u16 {
    u16::from(cpu.peek(ADCL)) | (u16::from(cpu.peek(ADCH)) << 8)
}

/// §24.7 floor, then clamp. `0x3FF` is Vref minus one LSB.
fn code(vin_uv: i64, vref_uv: i64) -> u16 {
    if vin_uv <= 0 || vref_uv <= 0 {
        return 0;
    }
    (vin_uv * 1024 / vref_uv).min(1023) as u16
}

/// §24.4 and Table 24-5. Every divisor, and the start waits for the next edge.
#[test]
fn prescaler_and_the_start_edge() {
    for adps in 0..8u8 {
        let div = div_of(adps);
        let mut cpu = fresh();
        cpu.poke(ADCSRA, ADEN | adps);
        cpu.run_until(div + 3);
        cpu.poke(ADCSRA, ADEN | ADSC | adps);
        let wrote = cpu.cycles();
        let start = edge_after(0, div, wrote);
        let done = start + 25 * div;
        if adps == 0 {
            // `/2`. Edges at 1, 3, 5, 7. `ADSC` at cycle 5 starts at 7.
            assert_eq!((wrote, start, done), (5, 7, 57));
        }
        if adps == 7 {
            // `/128`. First edge after `ADEN` is 64. `ADSC` at 131 starts at 192.
            assert_eq!((wrote, start, done), (131, 192, 3392));
        }
        cpu.run_until(done - 1);
        assert!(adsc(&cpu), "ADPS {adps}: still busy one clock early");
        assert!(
            !adif(&cpu),
            "ADPS {adps}: ADIF waits for the 25th ADC clock"
        );
        cpu.run_until(done);
        assert!(!adsc(&cpu), "ADPS {adps}: ADSC clears at {done}");
        assert!(adif(&cpu), "ADPS {adps}: ADIF at the 25th ADC clock");
        assert_eq!(cpu.cycles(), done);
    }
}

/// Table 24-1. First conversion 25 / S/H 13.5, normal 13 / 1.5, auto 13.5 / 2.
/// A host change before the sample is in the result; one after it is not.
#[test]
fn sample_and_hold_follows_table_24_1() {
    let div = 4u64;
    let mut early = fresh();
    early.set_voltage(PC0, 1.0);
    early.poke(ADCSRA, ADEN | ADSC | 0x02);
    let start = edge_after(0, div, 0);
    let sample = start + 27 * div / 2;
    let done = start + 25 * div;
    // `/4`, `ADEN|ADSC` at cycle 0. First edge is div/2 = 2. S/H 13.5, done 25.
    assert_eq!((start, sample, done), (2, 56, 102));
    early.run_until(sample - 1);
    early.set_voltage(PC0, 2.5);
    early.run_until(done);
    assert_eq!(result(&early), 512, "2.5 V sampled at the S/H point");

    let mut late = fresh();
    late.set_voltage(PC0, 1.0);
    late.poke(ADCSRA, ADEN | ADSC | 0x02);
    late.run_until(sample);
    late.set_voltage(PC0, 2.5);
    late.run_until(done);
    assert_eq!(
        result(&late),
        code(1_000_000, 5_000_000),
        "a change on the sample cycle is after the hold"
    );

    // The init conversion is spent. The next one is 13 clocks, S/H at 1.5.
    let wrote = late.cycles();
    late.set_voltage(PC0, 1.0);
    late.poke(ADCSRA, ADEN | ADIF | ADSC | 0x02);
    let start2 = edge_after(0, div, wrote);
    let sample2 = start2 + 3 * div / 2;
    let done2 = start2 + 13 * div;
    late.run_until(sample2 - 1);
    late.set_voltage(PC0, 2.5);
    late.run_until(done2);
    assert_eq!(
        result(&late),
        512,
        "normal conversion samples at 1.5 ADC clocks"
    );

    // §24.4. S/H is trigger+3+2×div, done is trigger+3+13.5×div. `/4`:
    // 200+3+8 = 211 and 200+3+54 = 257. Counting from the edge at 206
    // (2 clocks later) would sample at 214.
    let mut auto = fresh();
    auto.poke(ADCSRA, ADEN | ADSC | 0x02);
    auto.run_until(done);
    auto.poke(ADCSRA, ADEN | ADIF | ADATE | 0x02);
    auto.poke(ADCSRB, 0x03);
    auto.run_until(200);
    auto.set_voltage(PC0, 1.0);
    auto.poke(TIFR0, 0x02);
    let trigger = auto.cycles();
    assert_eq!(trigger, 200);
    let sample3 = trigger + 3 + 2 * div;
    let done3 = trigger + 3 + 27 * div / 2;
    assert_eq!((sample3, done3), (211, 257));
    auto.run_until(sample3 - 1);
    auto.set_voltage(PC0, 2.5);
    auto.run_until(done3);
    assert_eq!(
        result(&auto),
        512,
        "auto-trigger samples at trigger+3+2 ADC clocks"
    );

    let mut missed = fresh();
    missed.poke(ADCSRA, ADEN | ADSC | 0x02);
    missed.run_until(done);
    missed.poke(ADCSRA, ADEN | ADIF | ADATE | 0x02);
    missed.poke(ADCSRB, 0x03);
    missed.run_until(200);
    missed.set_voltage(PC0, 1.0);
    missed.poke(TIFR0, 0x02);
    missed.run_until(sample3);
    missed.set_voltage(PC0, 2.5);
    missed.run_until(done3);
    assert_eq!(result(&missed), code(1_000_000, 5_000_000));
}

/// §24.4. Sample-and-hold is `trigger + 3 + 2×div`. Completion is
/// `trigger + 3 + 13.5×div`. Checked for `/2`, `/16`, and `/128`.
#[test]
fn auto_trigger_sync_is_three_cpu_clocks() {
    // (ADPS, div, trigger). The init conversion is burned first.
    // Triggers sit after the 25-clock initialization (`div/2 + 25×div`).
    let cases = [(0u8, 2u64, 80u64), (4, 16, 500), (7, 128, 20_000)];
    for (adps, div, at) in cases {
        let mut cpu = fresh();
        cpu.set_voltage(PC0, 1.0);
        cpu.poke(ADCSRA, ADEN | ADSC | adps);
        let init = div / 2 + 25 * div;
        cpu.run_until(init);
        assert!(adif(&cpu), "/{div} init conversion finished at {init}");
        cpu.poke(ADCSRA, ADEN | ADIF | ADATE | adps);
        cpu.poke(ADCSRB, 0x03);
        cpu.run_until(at);
        cpu.poke(TIFR0, 0x02);
        assert_eq!(cpu.cycles(), at);
        let sample = at + 3 + 2 * div;
        let done = at + 3 + 27 * div / 2;
        match div {
            2 => assert_eq!((sample, done), (at + 7, at + 30)),
            16 => assert_eq!((sample, done), (at + 35, at + 219)),
            128 => assert_eq!((sample, done), (at + 259, at + 1731)),
            _ => unreachable!(),
        }
        cpu.run_until(sample - 1);
        cpu.set_voltage(PC0, 2.5);
        cpu.run_until(sample);
        // The hold has happened. A later step must not replace it.
        cpu.set_voltage(PC0, 0.0);
        cpu.run_until(done - 1);
        assert!(adsc(&cpu), "/{div} still busy at {}", done - 1);
        cpu.run_until(done);
        assert!(adif(&cpu) && !adsc(&cpu), "/{div} completes at {done}");
        assert_eq!(
            result(&cpu),
            512,
            "/{div} held 2.5 V from the cycle before sample {sample}"
        );
    }

    // At `/128`: trigger at 20000, 1.0 V held, 2.5 V one
    // cycle after sample-and-hold (20259). The result stays 204.
    let mut late = fresh();
    late.set_voltage(PC0, 1.0);
    late.poke(ADCSRA, ADEN | ADSC | 0x07);
    late.run_until(64 + 25 * 128);
    late.poke(ADCSRA, ADEN | ADIF | ADATE | 0x07);
    late.poke(ADCSRB, 0x03);
    late.run_until(20_000);
    late.poke(TIFR0, 0x02);
    late.run_until(20_260);
    late.set_voltage(PC0, 2.5);
    late.run_until(20_000 + 3 + 27 * 128 / 2);
    assert_eq!(result(&late), code(1_000_000, 5_000_000));
    assert_eq!(code(1_000_000, 5_000_000), 204);
}

/// Figure 24-3. With `ADEN|ADSC` at cycle 0 the conversion starts on the
/// first rising edge, `div/2` later, and the first conversion takes 25 clocks.
#[test]
fn single_conversion_starts_half_a_period_after_aden() {
    let cases = [
        (0u8, 2u64, 1u64, 51u64),
        (4, 16, 8, 408),
        (7, 128, 64, 3264),
    ];
    for (adps, div, edge, done) in cases {
        assert_eq!(edge, div / 2);
        assert_eq!(done, edge + 25 * div);
        let mut cpu = fresh();
        cpu.poke(ADCSRA, ADEN | ADSC | adps);
        assert_eq!(cpu.cycles(), 0);
        cpu.run_until(done - 1);
        assert!(adsc(&cpu), "/{div} busy until {done}");
        cpu.run_until(done);
        assert!(!adsc(&cpu) && adif(&cpu), "/{div} done at {done}");
    }
}

/// §24.3. `ADSC` reads as 1 for the whole conversion. Writing 0 does not stop it.
#[test]
fn adsc_stays_set_until_the_conversion_completes() {
    let mut cpu = fresh();
    cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
    assert!(adsc(&cpu), "the bit is 1 before the first ADC clock");
    cpu.poke(ADCSRA, ADEN | 0x02);
    assert!(adsc(&cpu), "writing 0 has no effect");
    cpu.run_until(102);
    assert!(!adsc(&cpu));
}

/// §24.7, §24.9.1, channel 1111.
#[test]
fn conversion_uses_floor_and_clamps() {
    let cases = [
        (0.0_f32, 0u16),
        (2.5, 512),
        (5.0, 1023),
        (6.0, 1023),
        (-1.0, 0),
    ];
    for (volts, expect) in cases {
        let mut cpu = fresh();
        cpu.set_voltage(PC0, volts);
        cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
        cpu.run_until(102);
        assert_eq!(result(&cpu), expect, "{volts} V");
    }
    assert_eq!(code(2_500_000, 5_000_000), 512);
    assert_eq!(code(5_000_000, 5_000_000), 1023);
    assert_eq!(code(0, 5_000_000), 0);

    let mut gnd = fresh();
    gnd.set_voltage(PC0, 2.5);
    gnd.poke(ADMUX, 0x4F);
    gnd.poke(ADCSRA, ADEN | ADSC | 0x02);
    gnd.run_until(102);
    assert_eq!(result(&gnd), 0, "MUX 1111 is analog ground");

    let mut left = fresh();
    left.set_voltage(PC0, 2.5);
    left.poke(ADCSRA, ADEN | ADSC | 0x02);
    left.run_until(102);
    assert_eq!(left.peek(ADCL), 0x00);
    assert_eq!(left.peek(ADCH), 0x02);
    left.poke(ADMUX, 0x60);
    assert_eq!(
        left.peek(ADCH),
        0x80,
        "ADLAR presents the result immediately"
    );
    assert_eq!(left.peek(ADCL), 0x00);
}

/// §24.9.3. Reading `ADCL` locks the pair. A conversion that finishes before
/// `ADCH` is read is lost, and `ADIF` still sets.
#[test]
fn adcl_read_locks_and_drops_the_conversion_that_finishes() {
    let mut cpu = fresh();
    cpu.set_voltage(PC0, 2.5);
    cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
    cpu.run_until(102);
    assert_eq!(result(&cpu), 512);

    let mut prog = common::lds(16, ADCL).to_vec();
    prog.extend(std::iter::repeat_n(0, 8));
    cpu.load_words(&prog);
    cpu.insns(1);
    assert_eq!(cpu.reg(16), 0x00, "ADCL is the low byte");

    cpu.set_voltage(PC0, 5.0);
    cpu.poke(ADCSRA, ADEN | ADIF | ADSC | 0x02);
    let wrote = cpu.cycles();
    let done = edge_after(0, 4, wrote) + 13 * 4;
    cpu.run_until(done);
    assert!(adif(&cpu), "ADIF sets even though the result is dropped");
    assert_eq!(cpu.peek(ADCL), 0x00);
    assert_eq!(cpu.peek(ADCH), 0x02, "the locked pair still shows 512");

    cpu.load_words(&common::lds(17, ADCH));
    cpu.insns(1);
    assert_eq!(cpu.reg(17), 0x02);
    assert_eq!(
        result(&cpu),
        512,
        "unlocking does not reveal the dropped 1023"
    );

    cpu.set_voltage(PC0, 0.0);
    cpu.poke(ADCSRA, ADEN | ADIF | ADSC | 0x02);
    let wrote = cpu.cycles();
    cpu.run_until(edge_after(0, 4, wrote) + 13 * 4);
    assert_eq!(result(&cpu), 0, "the next conversion is stored again");
}

/// §24.5. The channel in effect at the start is the one that is sampled.
#[test]
fn admux_changes_during_a_conversion_wait_for_the_next_one() {
    let mut cpu = fresh();
    cpu.set_voltage(PC0, 2.5);
    cpu.set_voltage(PC1, 0.0);
    cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
    cpu.run_until(10);
    cpu.poke(ADMUX, 0x41);
    cpu.run_until(102);
    assert_eq!(
        result(&cpu),
        512,
        "the sample uses the channel latched at the start"
    );

    cpu.poke(ADCSRA, ADEN | ADIF | ADSC | 0x02);
    let wrote = cpu.cycles();
    cpu.run_until(edge_after(0, 4, wrote) + 13 * 4);
    assert_eq!(result(&cpu), 0, "the following conversion uses ADC1");

    // `/128`. `ADEN|ADSC` at cycle 0. Figure 24-3: the start edge is
    // div/2 = 64. Cycle 63 is the last cycle before that edge (§24.5), so
    // the result is ADC1's. A full-period edge at 128 would still be open
    // at cycle 64; this anchor is the half-period edge.
    let mut window = fresh();
    window.set_voltage(PC0, 2.5);
    window.set_voltage(PC1, 0.0);
    window.poke(ADCSRA, ADEN | ADSC | 0x07);
    assert_eq!(window.cycles(), 0);
    window.run_until(63);
    window.poke(ADMUX, 0x41);
    assert_eq!(window.cycles(), 63);
    window.run_until(64 + 25 * 128);
    assert!(
        !adsc(&window) && adif(&window),
        "the /128 start edge is 64, so 25 clocks end at 3264"
    );
    assert_eq!(
        result(&window),
        0,
        "ADC1 selected before the start edge at 64"
    );
}

/// Table 24-6. Each source is the rising edge of its flag. A flag that stays
/// set does not start another conversion.
#[test]
fn each_auto_trigger_source_is_a_flag_edge() {
    // Free running is source 0 and is not a foreign flag.
    let mut free = fresh();
    free.set_voltage(PC0, 2.5);
    free.poke(ADCSRA, ADEN | ADATE | ADSC | 0x02);
    free.run_until(102);
    assert_eq!(result(&free), 512);
    assert!(adsc(&free), "ADSC stays 1 in free running");
    free.set_voltage(PC0, 0.0);
    free.run_until(102 + 13 * 4);
    assert_eq!(
        result(&free),
        0,
        "the next conversion started without clearing ADIF"
    );
    assert!(adif(&free));

    // Already set: OCF0A rises once, the conversion finishes, the flag is
    // still 1, and nothing starts until it falls and rises again.
    let mut held = fresh();
    held.poke(ADCSRA, ADEN | ADATE | 0x02);
    held.poke(ADCSRB, 0x03);
    held.poke(TIFR0, 0x02);
    assert!(adsc(&held));
    let trigger = held.cycles();
    let done = edge_at(trigger, 4, trigger + 3) + 25 * 4;
    held.run_until(done);
    assert!(!adsc(&held));
    held.poke(TIFR0, 0x02);
    held.run_until(done + 400);
    assert!(!adsc(&held), "a flag that is already 1 does not retrigger");
    held.poke(TIFR0, 0);
    held.poke(TIFR0, 0x02);
    assert!(adsc(&held), "clearing and setting the flag starts another");

    // §24.9.4. Moving onto a source whose flag is already 1 is an edge.
    // Free running is not, even though a flag is set.
    let mut switched = fresh();
    switched.poke(TIFR0, 0x02);
    switched.poke(ADCSRA, ADEN | ADATE | 0x02);
    switched.poke(ADCSRB, 0x00);
    assert!(!adsc(&switched), "free running does not fire on a set flag");
    switched.poke(ADCSRB, 0x03);
    assert!(adsc(&switched), "ADTS change onto a set OCF0A is an edge");

    // The other six sources, one poke or one real edge each.
    expect_trigger(0x02, EIFR, 0x01, "INTF0");
    expect_trigger(0x04, TIFR0, 0x01, "TOV0");
    expect_trigger(0x05, TIFR1, 0x04, "OCF1B");
    expect_trigger(0x06, TIFR1, 0x01, "TOV1");
    expect_trigger(0x07, TIFR1, 0x20, "ICF1");

    // Analog comparator: ACI rises when ACO rises and ACIS selects rising.
    let mut aci = fresh();
    aci.set_voltage(AIN1, 0.0);
    aci.poke(ACSR, 0x03);
    aci.poke(ADCSRA, ADEN | ADATE | 0x02);
    aci.poke(ADCSRB, 0x01);
    let t = aci.cycles();
    aci.set_voltage(AIN0, 4.0);
    assert!(!adsc(&aci), "ACI waits for the synchronized ACO");
    aci.run_until(t + 2);
    assert!(adsc(&aci), "ACI rising edge is trigger source 1");

    // Real Timer0 overflow, not a planted flag. §15.7.1: TOV0 at cycle 256
    // with clk/1 from reset.
    let mut tov = fresh();
    tov.poke(TCCR0B, 0x01);
    tov.poke(ADCSRA, ADEN | ADATE);
    tov.poke(ADCSRB, 0x04);
    tov.run_until(255);
    assert!(!adsc(&tov), "TOV0 is not set on the clock that reaches 255");
    tov.run_until(256);
    assert!(adsc(&tov), "TOV0 at the wrap starts the conversion");

    // Real OCF0A. §15.5: the flag is one timer clock after the match.
    // CTC TOP 5, clk/1: match while TCNT is 5, OCF0A on the clear, cycle 6.
    let mut ocf = fresh();
    ocf.poke(OCR0A, 5);
    ocf.poke(TCCR0A, 0x02);
    ocf.poke(TCCR0B, 0x01);
    ocf.poke(ADCSRA, ADEN | ADATE);
    ocf.poke(ADCSRB, 0x03);
    ocf.run_until(5);
    assert!(!adsc(&ocf));
    ocf.run_until(6);
    assert!(adsc(&ocf), "OCF0A on the clock after the match");

    // Real INT0. The synchronizer shows the pin at T+2 (§14.2.4).
    let mut int0 = fresh();
    int0.poke(EICRA, 0x03);
    int0.poke(ADCSRA, ADEN | ADATE);
    int0.poke(ADCSRB, 0x02);
    int0.drive_pin(18, Level::Low);
    int0.run_until(20);
    int0.drive_pin(18, Level::High);
    let rose = int0.cycles();
    int0.run_until(rose + 1);
    assert!(!adsc(&int0));
    int0.run_until(rose + 2);
    assert!(adsc(&int0), "INTF0 at the synchronizer sample");
}

fn expect_trigger(adts: u8, flag: u16, bit: u8, name: &str) {
    let mut cpu = fresh();
    cpu.poke(ADCSRA, ADEN | ADATE | 0x02);
    cpu.poke(ADCSRB, adts);
    assert!(!adsc(&cpu), "{name} is idle until the flag rises");
    cpu.poke(flag, bit);
    assert!(adsc(&cpu), "{name} rising edge starts a conversion");
}

/// §24.9.2. Write-one-to-clear, and vector entry clears it too.
#[test]
fn adif_clears_on_write_one_and_on_vector_entry() {
    let mut cpu = fresh();
    cpu.set_voltage(PC0, 2.5);
    cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
    cpu.run_until(102);
    assert!(adif(&cpu));
    cpu.poke(ADCSRA, ADEN | 0x02);
    assert!(adif(&cpu), "writing 0 leaves ADIF set");
    cpu.poke(ADCSRA, ADEN | ADIF | 0x02);
    assert!(!adif(&cpu), "writing 1 clears ADIF");

    let mut words = vec![0xCFFFu16; 48];
    words[42] = ldi(16, 0x5A);
    words[43] = 0xCFFF;
    cpu.load_words(&words);
    cpu.poke(SREG, 0x80);
    cpu.poke(ADCSRA, ADEN | ADIE | ADIF | ADSC | 0x02);
    cpu.run_until(cpu.cycles() + 200);
    assert_eq!(cpu.reg(16), 0x5A, "the ADC vector ran");
    assert!(!adif(&cpu), "vector entry clears ADIF");
}

/// §24.8 Table 24-2 and Table 24-4 channels 1110 / 1000. Typical, not calibrated.
#[test]
fn bandgap_and_temperature_use_the_typical_table() {
    let mut band = fresh();
    band.poke(ADMUX, 0x4E);
    band.poke(ADCSRA, ADEN | ADSC | 0x02);
    band.run_until(102);
    assert_eq!(result(&band), code(1_100_000, 5_000_000));
    assert_eq!(code(1_100_000, 5_000_000), 225);

    let mut temp = fresh();
    temp.set_temperature_c(25.0);
    temp.poke(ADMUX, 0xC8);
    temp.poke(ADCSRA, ADEN | ADSC | 0x02);
    temp.run_until(102);
    // 314 mV at 25 °C against the 1.1 V reference.
    assert_eq!(result(&temp), code(314_000, 1_100_000));
    assert_eq!(code(314_000, 1_100_000), 292);

    temp.set_temperature_c(-45.0);
    temp.poke(ADCSRA, ADEN | ADIF | ADSC | 0x02);
    let wrote = temp.cycles();
    temp.run_until(edge_after(0, 4, wrote) + 13 * 4);
    assert_eq!(
        result(&temp),
        code(242_000, 1_100_000),
        "Table 24-2 at −45 °C"
    );

    temp.set_temperature_c(85.0);
    temp.poke(ADCSRA, ADEN | ADIF | ADSC | 0x02);
    let wrote = temp.cycles();
    temp.run_until(edge_after(0, 4, wrote) + 13 * 4);
    assert_eq!(code(380_000, 1_100_000), 353);
    assert_eq!(result(&temp), 353, "Table 24-2 at +85 °C is 380 mV");

    // The 25 °C…85 °C segment extended past 85 °C, not clamped.
    // 105 °C → 314 + (380−314)×(105−25)/(85−25) = 402 mV.
    temp.set_temperature_c(105.0);
    temp.poke(ADCSRA, ADEN | ADIF | ADSC | 0x02);
    let wrote = temp.cycles();
    temp.run_until(edge_after(0, 4, wrote) + 13 * 4);
    assert_eq!(code(402_000, 1_100_000), 374);
    assert_eq!(result(&temp), 374, "105 °C extends the 25…85 °C segment");
}

/// §24.9.5 and §23.3.3. The digital buffer reads 0 while the bit is set.
#[test]
fn didr_forces_the_pin_register_to_zero() {
    let mut cpu = fresh();
    cpu.set_voltage(PC0, 4.0);
    let t = cpu.cycles();
    cpu.run_until(t + 2);
    assert_eq!(cpu.peek(PINC) & 1, 1);
    cpu.poke(DIDR0, 0x01);
    assert_eq!(cpu.peek(PINC) & 1, 0, "DIDR0 bit 0 disables ADC0");

    cpu.set_voltage(AIN0, 4.0);
    let t = cpu.cycles();
    cpu.run_until(t + 2);
    assert_ne!(cpu.peek(PIND) & (1 << 6), 0);
    cpu.poke(DIDR1, 0x01);
    assert_eq!(cpu.peek(PIND) & (1 << 6), 0, "AIN0D disables PD6");
}

/// Table 30-1 at VCC = 5.0 V. VIL max 1.5 V, VIH min 3.0 V. Between them the
/// previous level holds.
#[test]
fn digital_input_has_hysteresis_between_the_thresholds() {
    let mut cpu = fresh();
    let see = |cpu: &mut Cpu| {
        let t = cpu.cycles();
        cpu.run_until(t + 2);
        cpu.peek(PINC) & 1
    };
    cpu.set_voltage(PC0, 2.0);
    assert_eq!(see(&mut cpu), 0, "2.0 V holds the reset level, low");
    cpu.set_voltage(PC0, 1.5);
    assert_eq!(see(&mut cpu), 0, "1.5 V is VIL max");
    cpu.set_voltage(PC0, 3.0);
    assert_eq!(see(&mut cpu), 1, "3.0 V is VIH min");
    cpu.set_voltage(PC0, 2.0);
    assert_eq!(see(&mut cpu), 1, "2.0 V holds the high level");
    cpu.set_voltage(PC0, 1.4);
    assert_eq!(see(&mut cpu), 0);
    cpu.set_voltage(PC0, 2.0);
    assert_eq!(see(&mut cpu), 0, "2.0 V holds the low level");

    cpu.drive_pin(PC0, Level::High);
    cpu.set_voltage(PC0, 2.0);
    assert_eq!(
        see(&mut cpu),
        1,
        "the voltage replaces the drive and holds its level"
    );
    cpu.drive_pin(PC0, Level::High);
    cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
    let wrote = cpu.cycles();
    cpu.run_until(edge_after(wrote, 4, wrote) + 25 * 4);
    assert_eq!(
        result(&cpu),
        1023,
        "a later digital high reads as the 5.0 V rail"
    );
}

/// §23. Comparator output, interrupt modes, bandgap, mux, capture, vector.
#[test]
fn comparator_output_modes_and_capture() {
    let mut cpu = fresh();
    cpu.set_voltage(AIN1, 1.0);
    let t = cpu.cycles();
    cpu.set_voltage(AIN0, 3.0);
    assert_eq!(
        cpu.peek(ACSR) & ACO,
        0,
        "ACO is not updated in the same cycle"
    );
    cpu.run_until(t + 1);
    assert_eq!(
        cpu.peek(ACSR) & ACO,
        0,
        "the 1-clock end of the range is not used"
    );
    cpu.run_until(t + 2);
    assert_ne!(cpu.peek(ACSR) & ACO, 0, "ACO at +2, the stated maximum");

    // Rising mode sets ACI. Falling mode does not, on this same edge.
    let mut rise = fresh();
    rise.poke(ACSR, 0x03);
    rise.set_voltage(AIN1, 0.0);
    let t = rise.cycles();
    rise.set_voltage(AIN0, 2.0);
    rise.run_until(t + 2);
    assert_ne!(rise.peek(ACSR) & ACI, 0, "ACIS 11 is a rising edge");

    let mut fall = fresh();
    fall.poke(ACSR, 0x02);
    fall.set_voltage(AIN1, 0.0);
    let t = fall.cycles();
    fall.set_voltage(AIN0, 2.0);
    fall.run_until(t + 2);
    assert_eq!(fall.peek(ACSR) & ACI, 0, "ACIS 10 ignores a rising edge");
    let t = fall.cycles();
    fall.set_voltage(AIN0, 0.0);
    fall.run_until(t + 2);
    assert_ne!(fall.peek(ACSR) & ACI, 0, "ACIS 10 takes the falling edge");

    let mut toggle = fresh();
    toggle.poke(ACSR, 0x00);
    toggle.set_voltage(AIN1, 0.0);
    let t = toggle.cycles();
    toggle.set_voltage(AIN0, 2.0);
    toggle.run_until(t + 2);
    assert_ne!(toggle.peek(ACSR) & ACI, 0, "ACIS 00 takes a toggle");

    let mut reserved = fresh();
    reserved.poke(ACSR, 0x01);
    reserved.set_voltage(AIN1, 0.0);
    let t = reserved.cycles();
    reserved.set_voltage(AIN0, 2.0);
    reserved.run_until(t + 2);
    assert_eq!(
        reserved.peek(ACSR) & ACI,
        0,
        "ACIS 01 is reserved and does not set ACI"
    );

    let mut gap = fresh();
    gap.poke(ACSR, ACBG);
    gap.set_voltage(AIN1, 0.0);
    let t = gap.cycles();
    gap.run_until(t + 2);
    assert_ne!(
        gap.peek(ACSR) & ACO,
        0,
        "ACBG puts 1.1 V on the positive input"
    );
    gap.set_voltage(AIN1, 1.1);
    let t = gap.cycles();
    gap.run_until(t + 2);
    assert_eq!(gap.peek(ACSR) & ACO, 0, "equal inputs are not greater");

    // Table 23-1. ACME with ADEN clear selects ADC1 instead of AIN1.
    let mut muxed = fresh();
    muxed.poke(ADCSRB, ACME);
    muxed.poke(ADMUX, 0x01);
    muxed.set_voltage(AIN0, 2.0);
    muxed.set_voltage(AIN1, 0.0);
    muxed.set_voltage(PC1, 4.0);
    let t = muxed.cycles();
    muxed.run_until(t + 2);
    assert_eq!(
        muxed.peek(ACSR) & ACO,
        0,
        "ADC1 at 4 V is the negative input"
    );
    muxed.poke(ADCSRB, 0);
    let t = muxed.cycles();
    muxed.run_until(t + 2);
    assert_ne!(muxed.peek(ACSR) & ACO, 0, "clearing ACME restores AIN1");

    // ACIC feeds Timer1 capture and disconnects ICP1. ICES1 rising, no noise canceler.
    let mut cap = fresh();
    cap.poke(TCCR1B, 0x40);
    cap.poke(ACSR, ACIC | 0x03);
    cap.set_voltage(AIN1, 0.0);
    let t = cap.cycles();
    cap.set_voltage(AIN0, 3.0);
    cap.run_until(t + 2);
    assert_ne!(
        cap.peek(TIFR1) & 0x20,
        0,
        "ICF1 follows ACO, without the pin synchronizer"
    );
    cap.poke(TIFR1, 0);
    // `ACO` is already high. Enabling `ACIC` copies that level and does not
    // capture. The following falling edge, with `ICES1` clear, sets `ICF1`.
    let mut stale = fresh();
    stale.set_voltage(AIN1, 0.0);
    stale.set_voltage(AIN0, 3.0);
    let t = stale.cycles();
    stale.run_until(t + 2);
    assert_ne!(stale.peek(ACSR) & ACO, 0);
    assert_eq!(stale.peek(TIFR1) & 0x20, 0);
    stale.poke(ACSR, ACIC);
    assert_eq!(
        stale.peek(TIFR1) & 0x20,
        0,
        "enabling ACIC is not itself a capture"
    );
    let t = stale.cycles();
    stale.set_voltage(AIN0, 0.0);
    stale.run_until(t + 2);
    assert_ne!(
        stale.peek(TIFR1) & 0x20,
        0,
        "the fall is seen against the level stored while ACIC was off"
    );

    cap.drive_pin(0, Level::Low);
    cap.run_until(cap.cycles() + 4);
    cap.drive_pin(0, Level::High);
    let edge = cap.cycles();
    cap.run_until(edge + 12);
    assert_eq!(
        cap.peek(TIFR1) & 0x20,
        0,
        "ICP1 is disconnected while ACIC is set"
    );

    let mut words = vec![0xCFFFu16; 50];
    words[46] = ldi(16, 0xA5);
    words[47] = 0xCFFF;
    let mut irq = fresh();
    irq.load_words(&words);
    irq.poke(SREG, 0x80);
    irq.poke(ACSR, ACIE | 0x03);
    irq.set_voltage(AIN1, 0.0);
    let t = irq.cycles();
    irq.set_voltage(AIN0, 3.0);
    irq.run_until(t + 30);
    assert_eq!(irq.reg(16), 0xA5, "the comparator vector ran");
    assert_eq!(irq.peek(ACSR) & ACI, 0, "vector entry clears ACI");
}

/// §14.3.2 and §14.3.3. ADC0 is `PC0`, AIN0 is `PD6`, and AIN1 is `PD7`.
/// ADC6 has no GPIO pad (§1.1.9).
#[test]
fn analog_pins_match_the_328p_channels() {
    assert_eq!(Port::C.pin_id(0), PC0);
    assert_eq!(Port::D.pin_id(6), AIN0);
    assert_eq!(Port::D.pin_id(7), AIN1);

    let mut only = fresh();
    only.poke(ADMUX, 0x46);
    only.set_channel_voltage(6, 2.5);
    only.poke(ADCSRA, ADEN | ADSC | 0x02);
    only.run_until(102);
    assert_eq!(
        result(&only),
        512,
        "ADC6 has no GPIO pad and still converts"
    );
}

/// The scheduled completion matches a cycle-by-cycle step, and both match
/// Table 24-1's 25 ADC clocks (differential tests also carry an anchor).
#[test]
fn scheduler_matches_the_stepped_path_and_the_anchor() {
    let anchor = 2 + 25 * 4;
    assert_eq!(anchor, 102);
    let mut jumped = fresh();
    jumped.set_voltage(PC0, 2.5);
    jumped.poke(ADCSRA, ADEN | ADSC | 0x02);
    jumped.run_until(anchor);
    assert!(adif(&jumped));
    assert_eq!(result(&jumped), 512);

    let mut stepped = fresh();
    stepped.set_voltage(PC0, 2.5);
    stepped.poke(ADCSRA, ADEN | ADSC | 0x02);
    while !adif(&stepped) {
        stepped.step();
        assert!(
            stepped.cycles() <= anchor,
            "the stepped path ran past the anchor"
        );
    }
    assert_eq!(stepped.cycles(), jumped.cycles());
    assert_eq!(result(&stepped), 512);
}

/// §24.6. The named hook starts one conversion and refuses auto-trigger mode.
#[test]
fn noise_reduction_hook_starts_a_single_conversion() {
    let mut cpu = fresh();
    cpu.set_voltage(PC0, 2.5);
    cpu.poke(ADCSRA, ADEN | 0x02);
    orbe_microchip::test_hooks::adc_noise_reduction(&mut cpu);
    assert!(adsc(&cpu));
    cpu.run_until(102);
    assert_eq!(result(&cpu), 512);

    let mut armed = fresh();
    armed.poke(ADCSRA, ADEN | ADATE | 0x02);
    orbe_microchip::test_hooks::adc_noise_reduction(&mut armed);
    assert!(
        !adsc(&armed),
        "auto-trigger mode is not the noise-reduction start"
    );
}

/// A reset returns the ADC to the power-on state and drops a conversion
/// that has not finished.
#[test]
fn reset_aborts_a_conversion() {
    let mut cpu = fresh();
    cpu.set_voltage(PC0, 2.5);
    cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
    cpu.run_until(10);
    assert!(adsc(&cpu));
    orbe_microchip::test_hooks::reset_cpu(&mut cpu, orbe_microchip::ResetKind::PowerOn);
    assert_eq!(cpu.cycles(), 0);
    assert_eq!(cpu.peek(ADCSRA), 0, "ADCSRA is 0 after reset");
    cpu.run_until(200);
    assert!(!adsc(&cpu));
    assert_eq!(result(&cpu), 0, "the aborted conversion is not delivered");
}

/// §24.7 and Table 24-3. `REFS` = 00 converts against the AREF pin.
/// `Vcc` is the supply the host set, in volts.
#[test]
fn aref_pin_is_the_reference_and_vcc_is_the_supply() {
    let mut cpu = Cpu::new();
    let fuses = FuseBytes::CRYSTAL;
    cpu.set_fuses(fuses.low, fuses.high, fuses.extended);
    cpu.load_words(&[0x0000; 64]);
    cpu.set_aref(2.5);
    cpu.set_voltage(PC0, 1.25);
    cpu.set_vcc(3.3);
    cpu.run_until(1);
    cpu.poke(ADMUX, 0x00);
    cpu.poke(ADCSRA, ADEN | ADSC | 0x02);
    let start = cpu.cycles();
    cpu.run_until(start + 200);
    assert_eq!(
        result(&cpu),
        code(1_250_000, 2_500_000),
        "1.25 V against a 2.5 V AREF"
    );
    assert_eq!(cpu.vcc(), 3.3);
}
