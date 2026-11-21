//! The compiled sketches in `packages/engine/tests/firmware/sketch` and the Blocks' `firmware`,
//! built by `packages/engine/scripts/sketches.sh` against the standard AVR runtime for an
//! ATmega328P at 16 MHz.
//!
//! Each expected time comes from the runtime's own setup. Its `init()` runs Timer0 in fast PWM
//! at /64, so Timer0 overflows every 256 * 64 cycles. The overflow ISR adds 1 ms to `millis()`
//! and 3/125 ms to a fraction it carries, and `micros()` counts Timer0 ticks of 4 us. Timer1
//! runs 8-bit phase-correct PWM at /64. `HardwareSerial::begin` sets `U2X0` and
//! `UBRR0 = (F_CPU / 4 / baud - 1) / 2`, so a bit is 8 * (UBRR0 + 1) cycles (Table 20-1).
//!
//! These tests catch a wrong timer, prescaler, `millis()` or baud, which moves an edge by
//! hundreds to thousands of cycles. The serial tests also check frame spacing to the cycle. An
//! interrupt entry one cycle late moves edges a few dozen cycles and stays inside the timing
//! windows. The cycle-exact tests in `src/irq.rs`, `oc_pin`, `system`, `timeline`, `timers`
//! and `timing` catch that.

use orbe_microchip::{Cpu, Level, parse_hex};

const F_CPU: u64 = 16_000_000;
const MS: u64 = F_CPU / 1000;
const T0_OVERFLOW: u64 = 256 * 64;
/// One Timer0 tick. `micros()`, which `delay()` polls, moves in these steps.
const T0_TICK: u64 = 64;
/// What follows a timer event before the pin moves: the overflow ISR, a pass of `loop()`, and a
/// `digitalWrite`. A wrong `millis()` or `delay()` moves an edge by a millisecond or more. A
/// check that counts from cycle 0 also spends part of it on the start-up before Timer0 runs.
const SLACK: u64 = 512;

const PB0: u32 = 0;
const PB1: u32 = 1;
const PB5: u32 = 5;
const PD1: u32 = 17;
const PD2: u32 = 18;

/// An engine test sketch's image.
fn sketch(name: &str) -> String {
    format!("packages/engine/tests/firmware/sketch/{name}.hex")
}

/// The image at `path`, from the repository root.
fn load(path: &str) -> Cpu {
    let path = format!("{}/../../../{path}", env!("CARGO_MANIFEST_DIR"));
    let image = parse_hex(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut cpu = Cpu::new();
    cpu.load_image(&image).unwrap();
    cpu
}

/// Runs to `limit` and returns `pin`'s reports as `(cycle, high)`.
fn edges(cpu: &mut Cpu, pin: u32, limit: u64) -> Vec<(u64, bool)> {
    let mut edges = Vec::new();
    while cpu.cycles() < limit {
        cpu.run_until((cpu.cycles() + 1_000_000).min(limit));
        edges.extend(
            std::iter::from_fn(|| cpu.take_pin())
                .filter(|report| report.pin == pin)
                .map(|report| (report.cycle, report.level == Level::High)),
        );
    }
    assert_eq!(cpu.pin_overflow(), 0);
    edges
}

/// The first Timer0 overflow at which `millis()` reaches `ms`.
fn overflow_reaching(ms: u64) -> u64 {
    (0..).find(|&n| n + 3 * n / 125 >= ms).unwrap()
}

fn bit_cycles(baud: u64) -> u64 {
    let ubrr = (F_CPU / 4 / baud - 1) / 2;
    8 * (ubrr + 1)
}

/// The 8N1 frames on TXD that end by `end`, as `(start cycle, byte)`, sampled at the middle of
/// each bit.
fn decode_txd(edges: &[(u64, bool)], bit: u64, end: u64) -> Vec<(u64, u8)> {
    let level_at = |at: u64| {
        let index = edges.partition_point(|&(cycle, _)| cycle <= at);
        index > 0 && edges[index - 1].1
    };
    let mut frames = Vec::new();
    let mut from = 0;
    loop {
        let next = edges.partition_point(|&(cycle, _)| cycle < from);
        let Some(&(start, _)) = edges[next..].iter().find(|&&(_, high)| !high) else {
            break;
        };
        if start + 10 * bit > end {
            break;
        }
        let mid = |k: u64| start + k * bit + bit / 2;
        let byte = (0..8).fold(0u8, |byte, k| byte | u8::from(level_at(mid(k + 1))) << k);
        assert!(level_at(mid(9)), "stop bit of the frame at {start}");
        frames.push((start, byte));
        from = start + 10 * bit;
    }
    frames
}

fn text(frames: &[(u64, u8)]) -> String {
    frames.iter().map(|&(_, byte)| char::from(byte)).collect()
}

/// Every frame starts as the one before it ends.
fn assert_back_to_back(frames: &[(u64, u8)], bit: u64) {
    for pair in frames.windows(2) {
        assert_eq!(pair[1].0 - pair[0].0, 10 * bit, "frame at {}", pair[0].0);
    }
}

#[test]
fn blink_holds_each_level_for_delay_1000() {
    let mut cpu = load(&sketch("blink"));
    // 10 ms past the second hold, so its closing edge lands inside the run.
    let edges = edges(&mut cpu, PB5, 2 * 1000 * MS + 10 * MS);
    let levels: Vec<bool> = edges.iter().map(|&(_, high)| high).collect();
    assert_eq!(levels, [false, true, false, true], "{edges:?}");
    for pair in edges[1..].windows(2) {
        let held = pair[1].0 - pair[0].0;
        assert!(
            (1000 * MS - T0_TICK..1000 * MS + SLACK).contains(&held),
            "held {held} cycles from {}",
            pair[0].0
        );
    }
}

#[test]
fn blink_without_delay_toggles_as_millis_passes_each_second() {
    let mut cpu = load(&sketch("blink_without_delay"));
    let limit = overflow_reaching(3000) * T0_OVERFLOW + SLACK;
    let edges = edges(&mut cpu, PB5, limit);
    let levels: Vec<bool> = edges.iter().map(|&(_, high)| high).collect();
    assert_eq!(levels, [false, true, false, true], "{edges:?}");
    for (second, &(cycle, _)) in (1..).zip(&edges[1..]) {
        let overflow = overflow_reaching(1000 * second) * T0_OVERFLOW;
        assert!(
            (overflow - T0_TICK..overflow + SLACK).contains(&cycle),
            "toggle {second} at {cycle}, millis() reaches it at {overflow}"
        );
    }
}

#[test]
fn button_copies_pin_2_to_the_led() {
    let mut cpu = load("blocks/nano-button-led/firmware/button.hex");
    let mut led = edges(&mut cpu, PB5, 100_000);
    assert_eq!(
        led.iter().map(|&(_, high)| high).collect::<Vec<_>>(),
        [false]
    );
    for (n, level) in [Level::High, Level::Low, Level::High, Level::Low]
        .into_iter()
        .enumerate()
    {
        let at = cpu.cycles();
        cpu.drive_pin(PD2, level);
        led = edges(&mut cpu, PB5, 100_000 * (n as u64 + 2));
        assert_eq!(led.len(), 1, "one LED edge per press, {led:?}");
        let (cycle, high) = led[0];
        assert_eq!(high, level == Level::High);
        assert!(
            cycle > at && cycle < at + SLACK,
            "LED at {cycle}, pin at {at}"
        );
    }
}

/// `analogWrite(9, b)` sets `OCR1A`. In phase-correct PWM a non-inverting OC1A is high from the
/// down-count match to the up-count match, `2 * b` timer clocks of the 510 in a period (§16.9.4),
/// so the duty is `b / 255`, and each pulse is centered on BOTTOM. `analogWrite(9, 255)` is
/// `digitalWrite(9, HIGH)`, which first disconnects OC1A, so a pulse it lands in drops to
/// `PORTB1`, still low, until the store that sets it. `delay(30)` holds each step for 14 or 15
/// periods.
#[test]
fn fade_steps_the_duty_on_pin_9_every_30_ms() {
    const PERIOD: u64 = 510 * 64;
    // The glitch spans the few instructions of `digitalWrite` between its two stores. This bound
    // is a tenth of the narrowest pulse, 128 * 5 cycles.
    const GLITCH: u64 = 64;
    let mut cpu = load("blocks/nano-led-fade/firmware/fade.hex");
    let edges = edges(&mut cpu, PB1, 1700 * MS);
    assert!(!edges[0].1, "pinMode drives the pin low");
    let pulses: Vec<(u64, u64)> = edges[1..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[(rise, up), (fall, down)]| {
            assert!(up && !down, "pulse at {rise}");
            (rise, fall - rise)
        })
        .collect();
    let mut runs: Vec<(u64, usize)> = Vec::new();
    for &(_, width) in &pulses {
        match runs.last_mut() {
            Some((last, count)) if *last == width => *count += 1,
            _ => runs.push((width, 1)),
        }
    }

    let up: Vec<u64> = (5..=250).step_by(5).collect();
    let (rising, rest) = runs.split_at(up.len());
    let ([(cut, 1), (high, 1)], falling) = rest.split_at(2) else {
        panic!("one cut pulse and one high stretch at 255, {rest:?}");
    };
    let widths = |runs: &[(u64, usize)]| runs.iter().map(|&(width, _)| width).collect::<Vec<_>>();
    assert_eq!(
        widths(rising),
        up.iter().map(|b| 128 * b).collect::<Vec<_>>()
    );
    let down: Vec<u64> = (5..=250).rev().step_by(5).take(falling.len()).collect();
    assert_eq!(
        widths(falling),
        down.iter().map(|b| 128 * b).collect::<Vec<_>>()
    );
    let whole = rising[..rising.len() - 1]
        .iter()
        .chain(&falling[..falling.len() - 1]);
    for &(width, count) in whole {
        assert!((14..=15).contains(&count), "{count} pulses {width} wide");
    }
    assert!(*cut < 128 * 250);
    let cut_at = runs[..up.len()]
        .iter()
        .map(|&(_, count)| count)
        .sum::<usize>();
    let gap = pulses[cut_at + 1].0 - pulses[cut_at].0 - cut;
    assert!(gap < GLITCH, "low for {gap} cycles inside digitalWrite");
    assert!(
        (30 * MS - PERIOD..30 * MS + PERIOD).contains(high),
        "high for {high}"
    );

    let centers: Vec<u64> = pulses
        .iter()
        .enumerate()
        .filter(|&(n, _)| n != cut_at && n != cut_at + 1)
        .map(|(_, &(rise, width))| 2 * rise + width)
        .collect();
    for pair in centers.windows(2) {
        assert_eq!(
            (pair[1] - pair[0]) % (2 * PERIOD),
            0,
            "pulse centered at {}",
            pair[1] / 2
        );
    }
}

#[test]
fn analog_read_serial_prints_the_code_of_a0_at_9600_baud() {
    let bit = bit_cycles(9600);
    assert_eq!(bit, 1664);
    let mut cpu = load(&sketch("analog_read_serial"));
    cpu.set_channel_voltage(0, 3.3);
    let mut txd = edges(&mut cpu, PD1, 1_000_000);
    cpu.set_channel_voltage(0, 1.0);
    txd.extend(edges(&mut cpu, PD1, 3_500_000));
    let frames = decode_txd(&txd, bit, cpu.cycles());
    assert_back_to_back(&frames, bit);
    let text = text(&frames);
    let lines: Vec<&str> = text.split_terminator("\r\n").collect();
    let before = lines.iter().take_while(|&&line| line == "675").count();
    let after = lines[before..]
        .iter()
        .take_while(|&&line| line == "204")
        .count();
    assert!(before > 5 && after > 5, "{text:?}");
    assert!(before + after + 1 >= lines.len(), "{text:?}");
}

#[test]
fn serial_counter_prints_each_count_at_115200_baud() {
    let bit = bit_cycles(115_200);
    assert_eq!(bit, 136);
    let mut cpu = load(&sketch("serial_counter"));
    let frames = decode_txd(&edges(&mut cpu, PD1, 500 * MS), bit, cpu.cycles());
    assert_back_to_back(&frames, bit);
    let text = text(&frames);
    let want: String = (0..1300).map(|n| format!("{n}\r\n")).collect();
    assert!(text.contains("\r\n1000\r\n"), "{} bytes", text.len());
    assert_eq!(text, want[..text.len()]);
}

/// `tone(8, 440, 500)` picks the smallest Timer2 prescaler that fits `F_CPU / 440 / 2 - 1` in
/// `OCR2A`, which is /128 with `OCR2A = 141`, and toggles the pin in the compare ISR
/// `2 * 440 * 500 / 1000 = 440` times. The ISR toggles late by its latency, which the Timer0
/// overflow ISR can stretch, but a wrong `OCR2A` or prescaler would move every toggle a Timer2
/// tick further off the grid. `tone()` keeps `TCNT2` as it is, so the next call's first toggle
/// is within one period of the call.
#[test]
fn tone_plays_440_toggles_of_440_hz_once_a_second() {
    const HALF: u64 = 142 * 128;
    const LATENCY: u64 = 128;
    let mut cpu = load(&sketch("tone"));
    let edges = edges(&mut cpu, PB0, 1000 * MS + 10 * MS);
    assert!(!edges[0].1, "pinMode drives the pin low");
    let burst: Vec<(u64, bool)> = edges[1..]
        .iter()
        .copied()
        .take_while(|&(cycle, _)| cycle < 600 * MS)
        .collect();
    assert_eq!(burst.len(), 440);
    let first = burst[0].0;
    for (k, &(cycle, high)) in (0..).zip(&burst) {
        assert_eq!(high, k % 2 == 0);
        let late = cycle.abs_diff(first + k * HALF);
        assert!(late < LATENCY, "toggle {k} is {late} cycles off the grid");
    }
    assert!(edges.len() > 441, "no second tone in {} edges", edges.len());
    let next = edges[441].0;
    assert!(
        (1000 * MS - 2 * HALF..1000 * MS + 2 * HALF).contains(&(next - first)),
        "the next tone starts {} cycles after the first",
        next - first
    );
}
