//! `Cpu::take_sent`: each asynchronous frame with its start bit and its end on the oscillator.
//!
//! The expected cycles come from `TXD` itself. Two frames go back to back, so the second start
//! bit is the first frame's end, and `TXEN` is already clear, so `TXD` is released on the cycle
//! the second frame ends (§20.6.1). `0xFF` makes every fall a start bit. At `UBRR0` 0 a bit is
//! 16 CPU cycles and an 8N1 frame 160 (Table 20-1).

use crate::common::runs::{NOP, RETI, SEI, image};
use crate::common::{UCSR0B, UDR0, ldi, spin, sts};
use orbe_avr::{Channel, Cpu, FuseBytes, PinDrive, ResetKind, Sent, Timeline};

const UCSR0A: u16 = 0x00C0;
const UCSR0C: u16 = 0x00C2;
const DDRD: u16 = 0x002A;
const SMCR: u16 = 0x0053;
const MCUSR: u16 = 0x0054;
const WDTCSR: u16 = 0x0060;
const CLKPR: u16 = 0x0061;

const TXEN: u16 = 1 << 3;
const UDRE: u8 = 1 << 5;
const WDRF: u8 = 1 << 3;
const WDE: u8 = 1 << 3;
const WDCE: u8 = 1 << 4;
const WDIE: u8 = 1 << 6;
const SLEEP: u16 = 0x9588;
/// `SE` and `SM` = 010, power-down, which stops `clkI/O` and so the USART (Table 10-1).
const POWER_DOWN: u16 = 0x05;
const WDT_VECTOR: usize = 0x0C;

/// PD1 is `TXD`.
const PD1: usize = 17;

fn fresh() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.set_fuses(
        FuseBytes::CRYSTAL.low,
        FuseBytes::CRYSTAL.high,
        FuseBytes::CRYSTAL.extended,
    );
    cpu
}

fn store(words: &mut Vec<u16>, addr: u16, value: u16) {
    words.push(ldi(16, value));
    words.extend(sts(16, addr));
}

/// `CLKPCE`, then `CLKPS` = `clkps` four cycles inside its window (§9.12.2).
fn divide(words: &mut Vec<u16>, clkps: u16) {
    store(words, CLKPR, 0x80);
    store(words, CLKPR, clkps);
}

/// `TXEN`, then two `0xFF` frames queued, the second in the buffer.
fn send_two(words: &mut Vec<u16>) {
    store(words, UCSR0B, TXEN);
    words.push(ldi(17, 0xFF));
    words.extend(sts(17, UDR0));
    words.extend(sts(17, UDR0));
}

/// `TXD`'s drive changes through oscillator cycle `limit`, at the halts the timeline makes.
fn txd(timeline: &mut Timeline, limit: u64) -> Vec<(u64, PinDrive)> {
    let mut changes = Vec::new();
    let mut last = timeline.drives()[PD1];
    while let Some(halt) = timeline.run_until(limit) {
        let drive = timeline.drives()[PD1];
        if drive != last {
            changes.push((halt.at, drive));
            last = drive;
        }
    }
    changes
}

/// The two frames `TXD` shows: each start bit's fall, and the release that ends the second.
fn frames_on_txd(changes: &[(u64, PinDrive)]) -> Vec<Sent> {
    let falls: Vec<u64> = changes
        .iter()
        .filter(|(_, drive)| *drive == PinDrive::Low)
        .map(|(at, _)| *at)
        .collect();
    let released = changes
        .iter()
        .find(|(_, drive)| *drive == PinDrive::Off)
        .map(|(at, _)| *at);
    assert_eq!(falls.len(), 2, "{changes:?}");
    let frame = |start, end| Sent {
        usart: 0,
        byte: 0xFF,
        start,
        end,
    };
    vec![
        frame(falls[0], falls[1]),
        frame(falls[1], released.expect("TXD released")),
    ]
}

fn all_sent(timeline: &mut Timeline) -> Vec<Sent> {
    std::iter::from_fn(|| timeline.take_sent(u64::MAX)).collect()
}

/// `/2` before the frames. Then, `gap` cycles after the stores, `/4` and `/1` six cycles apart,
/// inside one frame for most gaps. The gaps put those divisions at every cycle of the first
/// frame and across the boundary between the two, so a start or an end lands on the cycle a
/// `CLKPR` store retires, the one cycle the previous division still converts. The `/2` puts
/// every end off its CPU cycle, so a bound one oscillator cycle before an end leaves the frame
/// queued only when the bound is read on the oscillator.
#[test]
fn a_frame_is_stamped_on_the_oscillator_across_clkpr_writes() {
    let mut lengths = Vec::new();
    for gap in 0..=190 {
        let mut words = Vec::new();
        divide(&mut words, 0x01);
        send_two(&mut words);
        words.extend(vec![NOP; gap]);
        divide(&mut words, 0x02);
        divide(&mut words, 0x00);
        store(&mut words, UCSR0B, 0);
        words.push(spin());
        let mut cpu = fresh();
        cpu.load_words(&words);
        let mut timeline = Timeline::new(cpu);
        let changes = txd(&mut timeline, 5_000);
        let mut copy = timeline.cpu().clone();
        let cycles: Vec<u64> = std::iter::from_fn(|| copy.take(Channel::Usart0))
            .map(|(_, at)| at)
            .collect();
        assert_eq!(
            cycles.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>(),
            [160],
            "take stays on CPU cycles, gap {gap}"
        );
        let frames = frames_on_txd(&changes);
        for frame in &frames {
            assert_eq!(
                timeline.take_sent(frame.end - 1),
                None,
                "gap {gap}, {frame:?}"
            );
            assert_eq!(timeline.take_sent(frame.end), Some(*frame), "gap {gap}");
        }
        assert_eq!(timeline.take_sent(u64::MAX), None, "gap {gap}");
        lengths.push(frames[0].end - frames[0].start);
    }
    assert!(
        lengths.iter().any(|&osc| osc != 2 * 160 && osc != 160),
        "some gap puts both divisions inside the first frame: {lengths:?}"
    );
}

/// Power-down stops `clkI/O`, so the USART freezes and resumes at the wake (§10.9). `wait`
/// NOPs before `SLEEP` put the sleep before the first start bit and at every cycle of both
/// frames. A start already on the wire keeps its cycle, and a start that waits for the wake is
/// the cycle it reaches `TXD`.
#[test]
fn a_frame_keeps_its_start_across_a_sleep() {
    let (mut frozen, mut after_wake) = (0, 0);
    for wait in 0..=300 {
        let mut main = Vec::new();
        send_two(&mut main);
        main.extend(vec![NOP; wait]);
        store(&mut main, SMCR, POWER_DOWN);
        main.extend([SEI, SLEEP]);
        store(&mut main, UCSR0B, 0);
        main.push(spin());
        let words = image(&main, &[WDT_VECTOR], &[RETI]);
        let mut cpu = fresh();
        cpu.set_wdt_hz(16_000_000);
        cpu.poke(WDTCSR, WDCE | WDE);
        cpu.poke(WDTCSR, WDIE);
        cpu.load_words(&words);
        let mut timeline = Timeline::new(cpu);
        let changes = txd(&mut timeline, 40_000);
        let sent = all_sent(&mut timeline);
        assert_eq!(sent, frames_on_txd(&changes), "wait {wait}");
        if sent.iter().any(|frame| frame.end - frame.start > 160) {
            frozen += 1;
        }
        if sent[0].start > 2_048 {
            after_wake += 1;
        }
    }
    assert!(
        frozen > 0 && after_wake > 0,
        "frozen {frozen}, after wake {after_wake}"
    );
}

/// Frames that ended at or before a reset's cycle are still taken after it, in order. The frame
/// on the wire and the byte waiting in the buffer are not (§11). Four bytes go back to back, so
/// 80 and 1 cycles before the third frame's end two frames have ended, the third is on the wire
/// and the fourth waits in `UDR0`. The watchdog resets with a time-out, brown-out and `RESET`
/// hold the chip.
#[test]
fn a_reset_keeps_sent_frames_and_drops_the_one_on_the_wire() {
    const LEAD: usize = 3_000;
    let mut words = vec![NOP; LEAD];
    store(&mut words, UCSR0B, TXEN);
    store(&mut words, UDR0, 0x41);
    store(&mut words, UDR0, 0x42);
    words.extend(vec![NOP; 200]);
    store(&mut words, UDR0, 0x43);
    words.extend(vec![NOP; 160]);
    store(&mut words, UDR0, 0x44);
    words.extend(vec![NOP; 4_000]);
    let mut clean = fresh();
    clean.load_words(&words);
    clean.run_until(LEAD as u64 + 1_000);
    let frames: Vec<Sent> = std::iter::from_fn(|| clean.take(Channel::Usart0))
        .map(|(byte, end)| Sent {
            usart: 0,
            byte,
            start: end - 160,
            end,
        })
        .collect();
    assert_eq!(
        frames.iter().map(|frame| frame.byte).collect::<Vec<_>>(),
        [0x41, 0x42, 0x43, 0x44]
    );
    assert!(
        frames.windows(2).all(|w| w[1].start == w[0].end),
        "back to back: {frames:?}"
    );
    let end = frames[2].end;
    let mut waiting = fresh();
    waiting.load_words(&words);
    waiting.run_until(end - 80);
    assert_eq!(waiting.peek(UCSR0A) & UDRE, 0, "0x44 waits in UDR0");
    let timeout = watchdog_timeout();

    for at in [end - 80, end - 1, end, end + 1] {
        let kept: Vec<Sent> = frames
            .iter()
            .copied()
            .filter(|frame| frame.end <= at)
            .collect();
        assert_eq!(kept.len(), if at >= end { 3 } else { 2 });
        for kind in [Reset::Watchdog, Reset::BrownOut, Reset::Pin] {
            let mut cpu = fresh();
            cpu.set_wdt_hz(16_000_000);
            cpu.load_words(&words);
            match kind {
                Reset::Watchdog => {
                    cpu.run_until(at - timeout);
                    arm_watchdog_reset(&mut cpu);
                    while cpu.peek(MCUSR) & WDRF == 0 {
                        cpu.step();
                    }
                }
                Reset::BrownOut => {
                    cpu.run_until(at);
                    cpu.set_brown_out(true);
                }
                Reset::Pin => {
                    cpu.run_until(at);
                    cpu.pull_reset();
                }
            }
            assert_eq!(cpu.cycles(), at, "{kind:?} resets on cycle {at}");
            cpu.run_until(at + 1_000);
            let taken: Vec<Sent> = std::iter::from_fn(|| cpu.take_sent(u64::MAX)).collect();
            assert_eq!(
                taken, kept,
                "{kind:?} at {at}, the third frame ends at {end}"
            );
        }
    }
}

/// A power-on reset restarts the chip and its cycle count, so it keeps no frame (§11).
#[test]
fn a_power_on_reset_keeps_no_sent_frame() {
    let mut words = Vec::new();
    send_two(&mut words);
    words.push(spin());
    let mut cpu = fresh();
    cpu.load_words(&words);
    cpu.run_until(1_000);
    let mut copy = cpu.clone();
    let ended = std::iter::from_fn(|| copy.take_sent(u64::MAX)).count();
    assert_eq!(ended, 2, "both frames ended");
    orbe_avr::test_hooks::reset_cpu(&mut cpu, ResetKind::PowerOn);
    assert_eq!(cpu.take_sent(u64::MAX), None);
}

#[derive(Clone, Copy, Debug)]
enum Reset {
    Watchdog,
    BrownOut,
    Pin,
}

fn arm_watchdog_reset(cpu: &mut Cpu) {
    cpu.poke(WDTCSR, WDCE | WDE);
    cpu.poke(WDTCSR, WDE);
}

/// CPU cycles from arming the watchdog to its reset, with its oscillator at the CPU's rate.
fn watchdog_timeout() -> u64 {
    let mut cpu = fresh();
    cpu.set_wdt_hz(16_000_000);
    cpu.load_words(&[NOP; 4_000]);
    cpu.run_until(10);
    arm_watchdog_reset(&mut cpu);
    while cpu.peek(MCUSR) & WDRF == 0 {
        cpu.step();
    }
    cpu.cycles() - 10
}

/// A Master SPI mode byte ahead of an async frame on the same queue (§21). Dropping it does
/// not end the scan, and a bound before the async frame's end leaves that frame queued.
#[test]
fn an_mspim_byte_ahead_of_a_frame_does_not_hide_it() {
    let mut words = Vec::new();
    store(&mut words, DDRD, 1 << 4);
    store(&mut words, UCSR0C, 0xC0);
    store(&mut words, UCSR0B, TXEN);
    store(&mut words, UDR0, 0xA5);
    words.extend(vec![NOP; 40]);
    store(&mut words, UCSR0C, 0x06);
    store(&mut words, UDR0, 0x41);
    words.push(spin());
    let mut cpu = Cpu::new();
    cpu.load_words(&words);
    cpu.run_until(600);
    let mut copy = cpu.clone();
    let taken: Vec<(u8, u64)> = std::iter::from_fn(|| copy.take(Channel::Usart0)).collect();
    let [(0xA5, spi_end), (0x41, end)] = taken[..] else {
        panic!("the SPI byte, then the frame: {taken:?}");
    };
    let frame = Sent {
        usart: 0,
        byte: 0x41,
        start: end - 160,
        end,
    };

    let mut bounded = cpu.clone();
    assert_eq!(bounded.take_sent(spi_end), None);
    assert_eq!(bounded.take_sent(end - 1), None);
    assert_eq!(bounded.take_sent(end), Some(frame));
    assert_eq!(bounded.take_sent(u64::MAX), None);

    assert_eq!(cpu.take_sent(u64::MAX), Some(frame));
    assert_eq!(cpu.take(Channel::Usart0), None);
}
