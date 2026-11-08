//! USART0 on the ATmega328P, from DS40002061B chapter 20.
//!
//! Cycle numbers are the instruction-set manual's counts plus Table 20-1's
//! bit time. Nothing here is a recording of a previous run.

use crate::common;
use common::Instructions;
use common::{UCSR0B, UDR0, ldi, lds, out, read_reg, sbrs, sts};

use orbe_avr::{Channel, Cpu, Level, Port, SerialFrame, SupplyChannel};

const UCSR0A: u16 = 0x00C0;
const UCSR0C: u16 = 0x00C2;
const UBRR0L: u16 = 0x00C4;
const UBRR0H: u16 = 0x00C5;
const PIND: u16 = 0x0029;
const DDRD: u16 = 0x002A;
const PORTD: u16 = 0x002B;

const TXEN: u8 = 1 << 3;
const RXEN: u8 = 1 << 4;
const UCSZ2: u8 = 1 << 2;
const TXB8: u8 = 1 << 0;
const RXB8: u8 = 1 << 1;
const UDRIE: u8 = 1 << 5;
const TXCIE: u8 = 1 << 6;
const RXCIE: u8 = 1 << 7;
const UDRE: u8 = 1 << 5;
const TXC: u8 = 1 << 6;
const RXC: u8 = 1 << 7;
const FE: u8 = 1 << 4;
const DOR: u8 = 1 << 3;
const UPE: u8 = 1 << 2;
const U2X: u8 = 1 << 1;
const MPCM: u8 = 1 << 0;

const PD0: u32 = 16;
const PD1: u32 = 17;

/// Table 12-6. USART RX is vector 19. The word address is `2 * (19 - 1)`.
const USART_RX_WORD: u16 = 2 * (19 - 1);
/// §7.7.1 gives four cycles until the vector instruction is ready, and four
/// more when the request wakes the core. §10.1 also halts wake-up for the
/// mode's start-up time. Table 10-1 shows the main clock source enabled in
/// idle. It states no start-up time, so this count adds none.
const IDLE_ENTRY: u64 = 4 + 4;
/// `SMCR` is data address `0x53`, so its I/O address is `0x33`.
const SMCR_IO: u16 = 0x33;

fn pins(cpu: &mut Cpu) -> Vec<(u32, Level, u64)> {
    std::iter::from_fn(|| cpu.take_pin())
        .map(|report| (report.pin, report.level, report.cycle))
        .collect()
}

fn tx_edges(cpu: &mut Cpu) -> Vec<(Level, u64)> {
    pins(cpu)
        .into_iter()
        .filter(|(pin, _, _)| *pin == PD1)
        .map(|(_, level, cycle)| (level, cycle))
        .collect()
}

/// `LDI` is 1 cycle, `STS` is 2. Returns the cycle the `STS` to `UDR0` ends.
fn send_bytes(cpu: &mut Cpu, bytes: &[u8]) -> u64 {
    let mut words = vec![ldi(16, u16::from(cpu.peek(UCSR0B) | TXEN))];
    words.extend(sts(16, UCSR0B));
    let mut start = None;
    for (index, byte) in bytes.iter().enumerate() {
        let reg = 17 + index as u16;
        words.push(ldi(reg, u16::from(*byte)));
        words.extend(sts(reg, UDR0));
        if start.is_none() {
            // LDI + STS (TXEN) + LDI + STS (first byte) = 1+2+1+2.
            start = Some(cpu.cycles() + 6);
        }
    }
    let n = u32::try_from(words.len()).unwrap();
    cpu.load_words(&words);
    cpu.insns(n);
    start.unwrap_or(cpu.cycles())
}

fn bit_clocks(ubrr: u16, u2x: bool) -> u64 {
    let mul = if u2x { 8 } else { 16 };
    mul * (u64::from(ubrr) + 1)
}

/// `txclk` after a cycle-0 reload (Figure 20-2, divider phase 0).
///
/// Baud tick `k` (from 1) is at cycle `k × (UBRR + 1)`. `txclk` is every
/// 16th tick, or every 8th when `U2X` is set. The next one at or after `at`
/// is where an idle shifter launched at `at` puts its start bit (§20.6.1).
fn txclk_at_or_after(ubrr: u16, u2x: bool, at: u64) -> u64 {
    let period = u64::from(ubrr) + 1;
    let div = if u2x { 8 } else { 16 };
    let mut k = at.div_ceil(period).max(1);
    let rem = k % div;
    if rem != 0 {
        k += div - rem;
    }
    k * period
}

/// Figure 20-2. One baud tick is `UBRR + 1` cycles.
fn baud_period(ubrr: u16) -> u64 {
    u64::from(ubrr) + 1
}

/// Sample 1 on the grid the last `UBRR0L` store loaded.
///
/// §20.8.1 calls sample 1 the first zero-sample. It does not say how many
/// cycles fall between the pad edge and that sample. A tick on the edge
/// cycle counts as sample 1 here, and otherwise the next tick does. That
/// is this model's rule.
///
/// §20.3.1 loads the down-counter when it reaches zero or when `UBRRnL` is
/// written. §20.11.5 says the `UBRRnL` write updates the prescaler then.
/// Reset loads it at cycle 0, and cycle 0 is not a tick, so tick `k` from 1
/// is at `k * (UBRR + 1)` (Figure 20-2). A later store at `stored` reloads
/// the counter on that cycle. Ticks after the load are
/// `stored + k * (UBRR + 1)`.
fn sample1_cycle(stored: u64, ubrr: u16, edge: u64) -> u64 {
    let period = baud_period(ubrr);
    let steps = edge.saturating_sub(stored).div_ceil(period).max(1);
    stored + steps * period
}

/// §20.8.2 and Figure 20-7. An 8N1 stop bit is bit 9. Its last vote is
/// sample 10 of 16, or sample 6 of 8 when `U2X` is set.
///
/// 153 counts 8 data bits and no parity. The start bit and those 8 bits are
/// 9 groups of 16, and sample 10 of the stop bit adds 9. A parity bit would
/// add another 16 before that vote. `U2X` uses 8 samples per bit, so the
/// same vote is 77.
fn stop_vote_offset(u2x: bool) -> u64 {
    let samples = if u2x { 8 } else { 16 };
    let last = if u2x { 6 } else { 10 };
    9 * samples + (last - 1)
}

/// `RXC` cycle. Sample 1's cycle plus 153 periods, or 77 when `U2X` is set.
fn stop_vote_cycle(stored: u64, ubrr: u16, u2x: bool, edge: u64) -> u64 {
    sample1_cycle(stored, ubrr, edge) + stop_vote_offset(u2x) * baud_period(ubrr)
}

/// On a tick, one cycle after it, and one cycle before the next tick.
/// `UBRR` 0 has a one-cycle tick, so the three edges are three successive ticks.
fn start_edges(ubrr: u16) -> [u64; 3] {
    let period = baud_period(ubrr);
    let on_tick = period * 4;
    if period == 1 {
        [on_tick, on_tick + 1, on_tick + 2]
    } else {
        [on_tick, on_tick + 1, on_tick + period - 1]
    }
}

/// Same three phases, on the grid the store at `stored` set. The on-tick
/// edge is four periods after that store, after the load and after sleep.
fn edges_after_store(stored: u64, ubrr: u16) -> [u64; 3] {
    let period = baud_period(ubrr);
    let on_tick = stored + period * 4;
    [on_tick, on_tick + 1, on_tick + period - 1]
}

/// `LDI` is 1 cycle and `STS` is 2, so the `UBRR0L` store ends at `nops + 3`.
///
/// `prior` is poked into `UBRR0L` at cycle 0 before the sled. Zero leaves
/// the reset value. Idle sleep after the store keeps `clkI/O` running.
/// Table 10-1 shows that clock enabled in idle. Sleep stops the fetch, so
/// the frame cannot wrap the program counter back onto this store and
/// reload the counter again.
fn store_ubrr0l(cpu: &mut Cpu, prior: u16, ubrr: u16, stored: u64) {
    assert!(stored >= 3, "LDI plus STS needs 3 cycles");
    assert!(ubrr <= 0x00FF, "the row stores UBRR0L only");
    assert!(prior <= 0x00FF, "the prior value is UBRR0L only");
    assert_eq!(cpu.cycles(), 0);
    if prior != 0 {
        cpu.poke(UBRR0L, prior as u8);
        assert_eq!(cpu.peek(UBRR0L), prior as u8);
        assert_eq!(cpu.cycles(), 0, "the prior UBRR0L load stays at cycle 0");
    }
    let nops = usize::try_from(stored - 3).unwrap();
    let mut words = vec![0u16; nops];
    words.push(ldi(16, u16::from(ubrr as u8)));
    words.extend(sts(16, UBRR0L));
    words.push(ldi(16, 1));
    words.push(out(16, SMCR_IO));
    words.push(0x9588);
    cpu.load_words(&words);
    cpu.insns(u32::try_from(nops + 2).unwrap());
    assert_eq!(cpu.cycles(), stored, "the STS to UBRR0L ends at {stored}");
    assert_eq!(cpu.peek(UBRR0L), ubrr as u8);
    cpu.insns(3);
    assert!(cpu.is_sleeping(), "idle sleep after the UBRR0L store");
    assert_eq!(cpu.cycles(), stored + 3, "LDI, OUT, and SLEEP");
}

fn step_until(cpu: &mut Cpu, limit: u64) {
    while cpu.cycles() < limit {
        let before = cpu.cycles();
        cpu.step();
        assert_eq!(cpu.cycles(), before + 1, "a NOP advances one cycle");
    }
    assert_eq!(cpu.cycles(), limit);
}

fn arm_baud(cpu: &mut Cpu, ubrr: u16, u2x: bool, ucsrc: u8) {
    cpu.poke(UBRR0L, ubrr as u8);
    cpu.poke(UBRR0H, (ubrr >> 8) as u8);
    cpu.poke(UCSR0A, if u2x { U2X } else { 0 });
    cpu.poke(UCSR0C, ucsrc);
}

#[test]
fn image_transmits_a_line() {
    // UBRR resets to 0 and U2X to 0, so a bit is 16 clocks and an 8N1 frame
    // is 160 (Table 20-1, §20.4). The first STS ends at cycle 6.
    let mut program = Vec::new();
    program.push(ldi(16, 0x08));
    program.extend(sts(16, UCSR0B));
    program.push(ldi(17, 0x41));
    program.push(ldi(18, 0x42));
    program.extend(lds(16, UCSR0A));
    program.push(sbrs(16, 5));
    program.push(0xCFFC);
    program.extend(sts(17, UDR0));
    program.extend(lds(16, UCSR0A));
    program.push(sbrs(16, 5));
    program.push(0xCFFC);
    program.extend(sts(18, UDR0));

    let mut cpu = Cpu::new();
    cpu.load_words(&program);
    cpu.run_until(400);
    let (first, at_a) = cpu.take(Channel::Usart0).unwrap();
    let (second, at_b) = cpu.take(Channel::Usart0).unwrap();
    assert_eq!(first, 0x41);
    assert_eq!(second, 0x42);
    assert_eq!(cpu.take(Channel::Usart0), None);
    assert_eq!(at_b - at_a, 160, "back-to-back 8N1 frames at UBRR 0");
}

#[test]
fn image_accepts_a_supplied_byte_after_rxen() {
    let mut enable = vec![ldi(16, 0x10)];
    enable.extend(sts(16, UCSR0B));
    let mut cpu = Cpu::new();
    cpu.load_words(&enable);
    cpu.insns(2);
    let edge = cpu.cycles();
    cpu.supply(SupplyChannel::Usart0, &[0x55]);

    let mut program = Vec::new();
    program.extend(lds(16, UCSR0A));
    program.push(sbrs(16, 7));
    program.push(0xCFFC);
    program.extend(lds(17, UDR0));
    cpu.load_words(&program);
    cpu.run_until(edge + 400);
    assert_eq!(cpu.reg(17), 0x55);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0);
}

/// §20.7.1. The received byte moves into the buffer when the first stop bit
/// is received. The receiver decides that bit on its last vote, sample 10
/// of 16, or sample 6 of 8 when `U2X` is set (§20.8.2). `RXC` is first set
/// on that cycle.
///
/// The cycle is sample 1's cycle plus 153 periods for 8N1 with parity off,
/// or 77 periods when `U2X` is set. 153 counts 8 data bits and no parity.
/// The reset rows poke `UBRR0L` at cycle 0. The other rows store it with
/// `LDI` and `STS` at a cycle that is not a multiple of the period. One row
/// runs at `UBRR0` 207, then stores `UBRR0L` 16 at cycle 50.
#[test]
fn rxc_rises_on_the_stop_bit_s_last_vote() {
    for ubrr in [0u16, 16, 103, 207] {
        for u2x in [false, true] {
            for edge in start_edges(ubrr) {
                let got = first_rxc_cycle(0, 0, ubrr, u2x, edge);
                let expect = stop_vote_cycle(0, ubrr, u2x, edge);
                assert_eq!(got, expect, "UBRR {ubrr} U2X {u2x} edge {edge}");
            }
        }
    }
    // Period 1 has no cycle that is off the reset grid.
    for ubrr in [16u16, 103, 207] {
        for stored in [3u64, 50] {
            let period = baud_period(ubrr);
            assert_ne!(stored % period, 0, "UBRR {ubrr} store {stored}");
            for u2x in [false, true] {
                for edge in edges_after_store(stored, ubrr) {
                    let got = first_rxc_cycle(0, stored, ubrr, u2x, edge);
                    let expect = stop_vote_cycle(stored, ubrr, u2x, edge);
                    assert_eq!(
                        got, expect,
                        "UBRR {ubrr} U2X {u2x} store {stored} edge {edge}"
                    );
                }
            }
        }
    }
    // §20.3.1. The counter is already counting a period of 208. Cycle 50 is
    // not one of its zeros, so a load on the store and a load at the next
    // zero are different grids. The new period is 17. Sample 1 is
    // `50 + 4 * 17`, and the stop vote is that cycle plus `153 * 17`.
    let prior = 207u16;
    let ubrr = 16u16;
    let stored = 50u64;
    assert_ne!(
        stored % baud_period(prior),
        0,
        "store 50 is not a zero of 208"
    );
    let edge = stored + baud_period(ubrr) * 4;
    let expect = stop_vote_cycle(stored, ubrr, false, edge);
    assert_eq!(
        expect, 2719,
        "datasheet stop vote after the mid-count store"
    );
    let got = first_rxc_cycle(prior, stored, ubrr, false, edge);
    assert_eq!(
        got, expect,
        "UBRR {ubrr} prior {prior} U2X false store {stored} edge {edge}"
    );
}

/// §20.11.5. A store of `UBRR0H` does not reload the prescaler. §20.3.1 keeps
/// the running count on its old period until that count reaches zero, and
/// that zero loads the new `UBRR0`.
///
/// `UBRR0` is 103 from cycle 0, so the period is 104 and tick `k` is at
/// `k * 104`. Cycle 0 is not a tick. An `STS` of 1 ends at cycle 5003, and
/// `UBRR0` becomes 0x167, period 360. The old count reaches zero at
/// `49 * 104 = 5096`. An edge at 6536 raises `RXC0` at 61616. A grid dated
/// from cycle 104 raises it at 61664.
///
/// The store at 105 ends one cycle after the zero at 104, so the new period
/// starts at 208. The store at 208 lands on a zero. `reload` counts that
/// zero before the store lands, so the load keeps the old period and the
/// new period starts at the next zero. `U2X0` changes the stop vote to 77
/// periods and leaves the baud grid.
#[test]
fn ubrrh_store_waits_for_the_old_count_s_next_zero() {
    let rows = [
        (5003u64, false, 6536u64, 61616u64),
        (105, false, 1648, 56728),
        (208, false, 1752, 56832),
        (5003, true, 6536, 34256),
    ];
    for (stored, u2x, edge, vote) in rows {
        assert_eq!(
            ubrrh_vote(103, 1, stored, u2x),
            (edge, vote),
            "store {stored} U2X {u2x}"
        );
        let got = ubrrh_rxc(103, 1, stored, u2x, edge, vote);
        assert_eq!(got, vote, "store {stored} U2X {u2x} edge {edge}");
    }
}

/// Reset `UBRR0` is 0, so the period is 1. `UBRR0H` = 1 retires at 3 and
/// `UBRR0L` = 4 retires at 6. The zeros at 1, 2, and 3 stay on period 1.
/// The zero at 3 is the high-byte store, so the zero at 4 loads `0x100`.
/// The reload at 6 keeps those four ticks and loads period 261. `txclk`
/// index 16 is twelve of those periods after the reload.
#[test]
fn ubrrh_then_ubrrl_from_reset_puts_the_start_bit_on_txclk_16() {
    let reload_at = 6u64;
    let ticks_before = 4u64;
    let period = baud_period(0x104);
    assert_eq!(period, 261);
    let start = reload_at + (16 - ticks_before) * period;
    assert_eq!(start, 3138);

    let mut cpu = Cpu::new();
    let mut words = vec![ldi(16, 1)];
    words.extend(sts(16, UBRR0H));
    words.push(ldi(17, 4));
    words.extend(sts(17, UBRR0L));
    words.push(ldi(18, u16::from(TXEN)));
    words.extend(sts(18, UCSR0B));
    words.push(ldi(19, 0x55));
    words.extend(sts(19, UDR0));
    words.push(0xCFFF);
    cpu.load_words(&words);
    cpu.run_until(start + period);
    let edges = tx_edges(&mut cpu);
    let got = edges
        .iter()
        .find(|(level, _)| *level == Level::Low)
        .map(|edge| edge.1);
    assert_eq!(got, Some(start), "{edges:?}");
}

/// `UBRR0` 103. `0x55` commits at cycle 6. The start bit is `txclk` index
/// 16 at 1664, bit 0 is at 3328, and bit 1 is at 4992. `UBRR0H` = 1 retires
/// at 5003. The zero at 5096 is index 49 and loads period 360. Bit 2 is
/// index 64, fifteen periods later.
#[test]
fn ubrrh_store_moves_a_bit_in_the_frame_being_sent() {
    let zero = 49u64 * baud_period(103);
    assert_eq!(zero, 5096);
    let bit2 = zero + 15 * baud_period(0x167);
    assert_eq!(bit2, 10496);

    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 103, false, 0x06);
    let mut words = vec![ldi(16, u16::from(TXEN))];
    words.extend(sts(16, UCSR0B));
    words.push(ldi(17, 0x55));
    words.extend(sts(17, UDR0));
    words.extend(vec![0u16; 4994]);
    words.push(ldi(18, 1));
    words.extend(sts(18, UBRR0H));
    words.push(0xCFFF);
    cpu.load_words(&words);
    cpu.run_until(5003);
    assert_eq!(cpu.cycles(), 5003, "the STS to UBRR0H ends at 5003");
    assert_eq!(cpu.peek(UBRR0H), 1);
    cpu.run_until(bit2 + baud_period(103));
    let edges = tx_edges(&mut cpu);
    assert!(
        edges.contains(&(Level::High, bit2)),
        "bit 2 rises at {bit2}, got {edges:?}"
    );
}

/// A poke of `UBRR0H` follows the same next-zero rule as a store.
#[test]
fn ubrrh_poke_waits_for_the_old_count_s_next_zero() {
    let (edge, vote) = ubrrh_vote(103, 1, 5003, false);
    assert_eq!((edge, vote), (6536, 61616));

    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 103, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let mut words = vec![0u16; 5003];
    words.push(ldi(16, 1));
    words.push(out(16, SMCR_IO));
    words.push(0x9588);
    cpu.load_words(&words);
    cpu.run_until(5003);
    assert_eq!(cpu.cycles(), 5003);
    cpu.poke(UBRR0H, 1);
    assert_eq!(cpu.peek(UBRR0H), 1);
    let period = baud_period(0x167);
    let got = rxc_after_supply(&mut cpu, edge, vote, period, "poke UBRR0H at 5003");
    assert_eq!(got, vote);
}

/// Two stores of `UBRR0H`, with `UBRR0` 103 from cycle 0. The first retires
/// at 5003, so the zero at 5096 loads period 360. The second retires at
/// 5503. The zero at 5456 is the last one at or before that store, so the
/// zero at 5816 loads period 616. Sample 1 is four periods after that zero.
#[test]
fn second_ubrrh_store_anchors_on_the_running_period() {
    let first_zero = 49u64 * baud_period(103);
    assert_eq!(first_zero, 5096);
    let after_first = baud_period(0x167);
    assert_eq!(after_first, 360);
    let anchor = first_zero + after_first;
    assert_eq!(anchor, 5456);
    let loaded = anchor + after_first;
    assert_eq!(loaded, 5816);
    let period = baud_period(0x267);
    assert_eq!(period, 616);
    let edge = loaded + 4 * period;
    assert_eq!(edge, 8280);
    let vote = edge + stop_vote_offset(false) * period;
    assert_eq!(vote, 102528);

    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 103, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let mut words = vec![0u16; 5000];
    words.push(ldi(16, 1));
    words.extend(sts(16, UBRR0H));
    words.extend(vec![0u16; 497]);
    words.push(ldi(16, 2));
    words.extend(sts(16, UBRR0H));
    words.push(ldi(16, 1));
    words.push(out(16, SMCR_IO));
    words.push(0x9588);
    cpu.load_words(&words);
    cpu.run_until(5503);
    assert_eq!(cpu.cycles(), 5503, "the second STS to UBRR0H ends at 5503");
    assert_eq!(cpu.peek(UBRR0H), 2);
    let got = rxc_after_supply(&mut cpu, edge, vote, period, "two UBRR0H stores");
    assert_eq!(got, vote);
}

/// An `STS` of `UBRR0H` that ends on a zero of a grid `UBRR0L` set at cycle
/// 50. The store of 103 reloads at 50, so the next zero is 154, and 154 is
/// not a multiple of 104. That zero stays on period 104. Period 360 starts
/// at 258. Sample 1 is four periods later, and the stop vote is 153 periods
/// after sample 1.
#[test]
fn ubrrh_store_on_a_reloaded_grid_keeps_that_zero() {
    let anchor = 50u64;
    let low = 103u16;
    let old = baud_period(low);
    let stored = anchor + old;
    assert_eq!(stored, 154);
    assert_ne!(stored % old, 0, "154 is not on the cycle-0 grid");
    let loaded = stored + old;
    assert_eq!(loaded, 258);
    let period = baud_period((1u16 << 8) | low);
    assert_eq!(period, 360);
    let edge = loaded + 4 * period;
    assert_eq!(edge, 1698);
    let vote = edge + stop_vote_offset(false) * period;
    assert_eq!(vote, 56778);

    let mut cpu = Cpu::new();
    cpu.poke(UCSR0B, RXEN);
    cpu.poke(UCSR0C, 0x06);
    let mut words = vec![0u16; usize::try_from(anchor - 3).unwrap()];
    words.push(ldi(16, low));
    words.extend(sts(16, UBRR0L));
    let gap = usize::try_from(stored - anchor - 3).unwrap();
    words.extend(vec![0u16; gap]);
    words.push(ldi(17, 1));
    words.extend(sts(17, UBRR0H));
    words.push(ldi(16, 1));
    words.push(out(16, SMCR_IO));
    words.push(0x9588);
    cpu.load_words(&words);
    cpu.run_until(stored);
    assert_eq!(cpu.cycles(), stored, "the STS to UBRR0H ends on the zero");
    assert_eq!(cpu.peek(UBRR0L), low as u8);
    assert_eq!(cpu.peek(UBRR0H), 1);
    let got = rxc_after_supply(&mut cpu, edge, vote, period, "reloaded grid");
    assert_eq!(got, vote);
}

fn rxc_after_supply(cpu: &mut Cpu, edge: u64, vote: u64, period: u64, label: &str) -> u64 {
    cpu.run_until(edge);
    assert_eq!(cpu.cycles(), edge, "{label}");
    cpu.supply(SupplyChannel::Usart0, &[0x55]);
    let window = vote - 4 * period;
    cpu.run_until(window);
    assert_eq!(cpu.cycles(), window, "{label}");
    assert_eq!(
        cpu.peek(UCSR0A) & RXC,
        0,
        "{label} set RXC before the stop vote"
    );
    let limit = vote + 8 * period;
    while cpu.cycles() < limit {
        let before = cpu.cycles();
        cpu.step();
        assert_eq!(cpu.cycles(), before + 1, "{label}");
        if cpu.peek(UCSR0A) & RXC != 0 {
            return cpu.cycles();
        }
    }
    panic!("{label} left RXC clear through {limit}");
}

/// `(edge, RXC cycle)` after an `UBRR0H` store on the cycle-0 grid.
fn ubrrh_vote(prior: u16, high: u8, stored: u64, u2x: bool) -> (u64, u64) {
    let old = baud_period(prior);
    let period = baud_period((u16::from(high) << 8) | prior);
    let start = if stored != 0 && stored.is_multiple_of(old) {
        stored + old
    } else {
        stored.div_ceil(old) * old
    };
    let edge = start + 4 * period;
    (edge, edge + stop_vote_offset(u2x) * period)
}

/// `LDI` plus `STS` to `UBRR0H`, then idle sleep. The store ends at `stored`.
fn store_ubrr0h(cpu: &mut Cpu, high: u8, stored: u64) {
    assert!(stored >= 3, "LDI plus STS needs 3 cycles");
    assert_eq!(cpu.cycles(), 0);
    let nops = usize::try_from(stored - 3).unwrap();
    let mut words = vec![0u16; nops];
    words.push(ldi(16, u16::from(high)));
    words.extend(sts(16, UBRR0H));
    words.push(ldi(16, 1));
    words.push(out(16, SMCR_IO));
    words.push(0x9588);
    cpu.load_words(&words);
    cpu.insns(u32::try_from(nops + 2).unwrap());
    assert_eq!(cpu.cycles(), stored, "the STS to UBRR0H ends at {stored}");
    assert_eq!(cpu.peek(UBRR0H), high & 0x0F);
    cpu.insns(3);
    assert!(cpu.is_sleeping(), "idle sleep after the UBRR0H store");
}

fn ubrrh_rxc(prior: u16, high: u8, stored: u64, u2x: bool, edge: u64, expect: u64) -> u64 {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, prior, u2x, 0x06);
    cpu.poke(UCSR0B, RXEN);
    store_ubrr0h(&mut cpu, high, stored);
    let label = format!("high {high} U2X {u2x} store {stored} edge {edge}");
    cpu.run_until(edge);
    assert_eq!(cpu.cycles(), edge, "{label}");
    cpu.supply(SupplyChannel::Usart0, &[0x55]);
    let period = baud_period((u16::from(high) << 8) | prior);
    let window = expect - 4 * period;
    cpu.run_until(window);
    assert_eq!(cpu.cycles(), window, "{label}");
    assert_eq!(
        cpu.peek(UCSR0A) & RXC,
        0,
        "{label} set RXC before the stop vote"
    );
    let limit = expect + 8 * period;
    while cpu.cycles() < limit {
        let before = cpu.cycles();
        cpu.step();
        assert_eq!(cpu.cycles(), before + 1, "{label}");
        if cpu.peek(UCSR0A) & RXC != 0 {
            return cpu.cycles();
        }
    }
    panic!("{label} left RXC clear through {limit}");
}

fn first_rxc_cycle(prior: u16, stored: u64, ubrr: u16, u2x: bool, edge: u64) -> u64 {
    let mut cpu = Cpu::new();
    if stored == 0 {
        arm_baud(&mut cpu, ubrr, u2x, 0x06);
    } else {
        store_ubrr0l(&mut cpu, prior, ubrr, stored);
        cpu.poke(UCSR0A, if u2x { U2X } else { 0 });
        cpu.poke(UCSR0C, 0x06);
    }
    cpu.poke(UCSR0B, RXEN);
    let label = if stored == 0 {
        format!("UBRR {ubrr} U2X {u2x} edge {edge}")
    } else if prior == 0 {
        format!("UBRR {ubrr} U2X {u2x} store {stored} edge {edge}")
    } else {
        format!("UBRR {ubrr} prior {prior} U2X {u2x} store {stored} edge {edge}")
    };
    cpu.run_until(edge);
    assert_eq!(cpu.cycles(), edge, "{label}");
    cpu.supply(SupplyChannel::Usart0, &[0x55]);
    let expect = stop_vote_cycle(stored, ubrr, u2x, edge);
    let period = baud_period(ubrr);
    let window = expect - 4 * period;
    cpu.run_until(window);
    assert_eq!(cpu.cycles(), window, "{label}");
    assert_eq!(
        cpu.peek(UCSR0A) & RXC,
        0,
        "{label} set RXC before the stop vote"
    );
    let limit = expect + 8 * period;
    while cpu.cycles() < limit {
        let before = cpu.cycles();
        cpu.step();
        assert_eq!(cpu.cycles(), before + 1, "{label}");
        if cpu.peek(UCSR0A) & RXC != 0 {
            return cpu.cycles();
        }
    }
    panic!("{label} left RXC clear through {limit}");
}

/// §7.7.1. Idle sleep with `RXCIE` enters the RX vector eight cycles after
/// `RXC`, and `UDR` reads the byte that arrived. Table 10-1 shows the main
/// clock source enabled in idle and states no start-up time.
#[test]
fn rx_complete_wakes_idle_and_enters_its_vector() {
    for ubrr in [0u16, 16] {
        idle_rx_entry(ubrr);
    }
}

fn idle_rx_image() -> Vec<u16> {
    let mut words = vec![0u16; 0x30];
    // Word 0 jumps past the vector table to the setup at word 0x28.
    words[0] = rjmp_to(0, 0x28);
    let read = lds(17, UDR0);
    let vector = usize::from(USART_RX_WORD);
    words[vector] = read[0];
    words[vector + 1] = read[1];
    let init = 0x28;
    words[init] = ldi(16, u16::from(RXEN | RXCIE));
    let enable = sts(16, UCSR0B);
    words[init + 1] = enable[0];
    words[init + 2] = enable[1];
    words[init + 3] = ldi(16, 1);
    words[init + 4] = out(16, SMCR_IO);
    words[init + 5] = 0x9478;
    words[init + 6] = 0x9588;
    words[init + 7] = 0xCFFF;
    words
}

fn idle_rx_entry(ubrr: u16) {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, ubrr, false, 0x06);
    cpu.load_words(&idle_rx_image());
    let mut steps = 0u32;
    while !cpu.is_sleeping() {
        cpu.step();
        steps += 1;
        assert!(steps < 20, "UBRR {ubrr} did not reach SLEEP");
    }
    let edge = cpu.cycles();
    cpu.supply(SupplyChannel::Usart0, &[0x5A]);
    let rxc_at = stop_vote_cycle(0, ubrr, false, edge);
    let expect = rxc_at + IDLE_ENTRY;
    let limit = expect + baud_period(ubrr);
    let mut entered = None;
    while cpu.cycles() < limit {
        cpu.step();
        if cpu.pc() == USART_RX_WORD {
            entered = Some(cpu.cycles());
            break;
        }
    }
    assert_eq!(
        entered,
        Some(expect),
        "UBRR {ubrr} edge {edge} RXC at {rxc_at}"
    );
    assert_eq!(cpu.pc(), 0x0024, "UBRR {ubrr}");
    assert_eq!(cpu.peek(UCSR0A) & RXC, RXC, "UBRR {ubrr}");
    cpu.step();
    assert_eq!(cpu.reg(17), 0x5A, "UBRR {ubrr}");
}

/// §20.7.6 and §20.7.7. Clearing `RXEN` drops the FIFO and the frame in
/// progress at that instruction. The next frame is a new reception.
#[test]
fn clearing_rxen_mid_frame_drops_the_frame() {
    let ubrr = 0u16;
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, ubrr, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let edge = baud_period(ubrr) * 4;
    step_until(&mut cpu, edge);
    // Four bytes fill the two-deep buffer and the shifter. The fourth start
    // latches the overrun that clearing RXEN has to drop.
    cpu.supply(SupplyChannel::Usart0, &[0x11, 0x22, 0x33, 0x44]);
    let frame = 10 * bit_clocks(ubrr, false);
    let fourth = edge + 3 * frame;
    let mid = fourth + 4 * bit_clocks(ubrr, false);
    step_until(&mut cpu, mid);
    assert_eq!(cpu.peek(UCSR0A) & RXC, RXC, "earlier frames are buffered");

    let mut clear = vec![ldi(16, 0)];
    clear.extend(sts(16, UCSR0B));
    cpu.load_words(&clear);
    cpu.insns(2);
    assert_eq!(cpu.cycles(), mid + 3, "LDI plus STS");
    assert_eq!(cpu.peek(UCSR0B) & RXEN, 0);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "the STS leaves RXC clear");

    let vote = stop_vote_cycle(0, ubrr, false, fourth);
    step_until(&mut cpu, vote);
    assert_eq!(
        cpu.peek(UCSR0A) & RXC,
        0,
        "the dropped frame does not set RXC"
    );
    let stop_end = sample1_cycle(0, ubrr, fourth) + (9 * 16 + 15) * baud_period(ubrr);
    step_until(&mut cpu, stop_end);
    assert_eq!(
        cpu.peek(UCSR0A) & (RXC | FE | DOR),
        0,
        "RXC stays clear through the end of the stop bit"
    );

    let wire_end = edge + 4 * frame;
    step_until(&mut cpu, wire_end);
    let mut enable = vec![ldi(16, u16::from(RXEN))];
    enable.extend(sts(16, UCSR0B));
    cpu.load_words(&enable);
    cpu.insns(2);
    assert_eq!(cpu.peek(UCSR0B) & RXEN, RXEN);
    let again = cpu.cycles();
    cpu.supply(SupplyChannel::Usart0, &[0xA5]);
    let again_vote = stop_vote_cycle(0, ubrr, false, again);
    step_until(&mut cpu, again_vote);
    assert_eq!(
        cpu.peek(UCSR0A) & (RXC | UDRE | FE | DOR | UPE),
        RXC | UDRE,
        "the next frame has no FE or DOR"
    );
    assert_eq!(read_reg(&mut cpu, UDR0), 0xA5);
}

/// §20.11.3. Disabling the receiver flushes the buffer. That includes a
/// frame held in the shifter because the two-deep FIFO is already full.
/// The next byte has no `FE`, `DOR`, or `UPE`, and one `UDR` read clears
/// `RXC`.
#[test]
fn clearing_rxen_drops_the_held_frame() {
    let ubrr = 0u16;
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, ubrr, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let edge = baud_period(ubrr) * 4;
    step_until(&mut cpu, edge);
    cpu.supply(SupplyChannel::Usart0, &[0x11, 0x22, 0x33]);
    let frame = 10 * bit_clocks(ubrr, false);
    let third = edge + 2 * frame;
    let vote = stop_vote_cycle(0, ubrr, false, third);
    // Three supplied bytes and no fourth start. Clear RXEN after the third
    // stop vote, while that byte is still the held frame.
    let clear_at = edge + 3 * frame + 4;
    assert!(clear_at > vote, "RXEN clears after the third stop vote");
    step_until(&mut cpu, clear_at);
    let mut program = vec![ldi(16, 0)];
    program.extend(sts(16, UCSR0B));
    program.push(ldi(16, u16::from(RXEN)));
    program.extend(sts(16, UCSR0B));
    cpu.load_words(&program);
    cpu.insns(4);
    assert_eq!(cpu.cycles(), clear_at + 6, "clear RXEN, then set it");
    assert_eq!(cpu.peek(UCSR0B) & RXEN, RXEN);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "the flush clears RXC");
    let again = cpu.cycles();
    cpu.supply(SupplyChannel::Usart0, &[0xA5]);
    let again_vote = stop_vote_cycle(0, ubrr, false, again);
    step_until(&mut cpu, again_vote);
    assert_eq!(
        cpu.peek(UCSR0A) & (RXC | UDRE | FE | DOR | UPE),
        RXC | UDRE,
        "the byte after the flush has no FE, DOR, or UPE"
    );
    assert_eq!(read_reg(&mut cpu, UDR0), 0xA5, "the held byte was dropped");
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "one UDR read clears RXC");
}

/// §20.7.6. Clearing `RXEN` aborts the frame in the shifter. Data from that
/// reception is lost. Setting `RXEN` again before the stop vote does not
/// revive the frame.
#[test]
fn clearing_rxen_aborts_the_frame_in_progress() {
    let ubrr = 0u16;
    let byte = 0xA5u8;
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, ubrr, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let edge = baud_period(ubrr) * 4;
    step_until(&mut cpu, edge);
    cpu.supply(SupplyChannel::Usart0, &[byte]);

    let clear_at = 33u64;
    step_until(&mut cpu, clear_at - 3);
    let mut clear = vec![ldi(16, 0)];
    clear.extend(sts(16, UCSR0B));
    cpu.load_words(&clear);
    cpu.insns(2);
    assert_eq!(cpu.cycles(), clear_at, "the STS that clears RXEN");
    assert_eq!(cpu.peek(UCSR0B) & RXEN, 0);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "RXC is clear at the disable");

    let enable_at = 83u64;
    step_until(&mut cpu, enable_at - 3);
    let mut enable = vec![ldi(16, u16::from(RXEN))];
    enable.extend(sts(16, UCSR0B));
    cpu.load_words(&enable);
    cpu.insns(2);
    assert_eq!(cpu.cycles(), enable_at, "the STS that sets RXEN");
    assert_eq!(cpu.peek(UCSR0B) & RXEN, RXEN);
    assert!(enable_at < stop_vote_cycle(0, ubrr, false, edge));
    assert_eq!(
        cpu.peek(UCSR0A) & RXC,
        0,
        "RXC is still clear when RXEN returns"
    );

    let vote = stop_vote_cycle(0, ubrr, false, edge);
    assert_eq!(vote, 157, "stop vote of the interrupted frame");
    step_until(&mut cpu, vote);
    assert_eq!(
        cpu.peek(UCSR0A) & RXC,
        0,
        "RXC stays clear through the interrupted stop vote"
    );
    let stop_end = sample1_cycle(0, ubrr, edge) + (9 * 16 + 15) * baud_period(ubrr);
    step_until(&mut cpu, stop_end);
    assert_eq!(
        cpu.peek(UCSR0A) & RXC,
        0,
        "RXC stays clear through the end of the interrupted stop bit"
    );
    assert_ne!(
        read_reg(&mut cpu, UDR0),
        byte,
        "the interrupted byte never appears in UDR"
    );
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "UDR held no buffered byte");
}

/// Figure 20-7 and §20.8.2. Normal mode receives a start edge one cycle
/// after the stop bit's last vote, which is point A. Under `U2X` the first
/// low sample waits for point B, the next sample after sample 6.
#[test]
fn a_start_right_after_the_stop_vote_is_heard() {
    let ubrr = 15u16;
    hear_next_start(ubrr, false, 1);
    hear_next_start(ubrr, true, baud_period(ubrr));
}

fn hear_next_start(ubrr: u16, u2x: bool, gap: u64) {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, ubrr, u2x, 0x06);
    cpu.poke(UCSR0B, RXEN);
    cpu.drive_pin(PD0, Level::High);
    let edge = baud_period(ubrr) * 4;
    let bit = bit_clocks(ubrr, u2x);
    drive_rx_frame(&mut cpu, edge, 0x55, bit);
    let vote = stop_vote_cycle(0, ubrr, u2x, edge);
    let second = vote + gap;
    assert!(
        second > edge + 9 * bit,
        "U2X {u2x} the next start falls during the stop bit"
    );
    drive_rx_frame(&mut cpu, second, 0xA3, bit);
    let second_vote = stop_vote_cycle(0, ubrr, u2x, second);
    cpu.run_until(second_vote);
    assert_eq!(cpu.cycles(), second_vote, "U2X {u2x}");
    let flags = RXC | UDRE | FE | DOR | UPE;
    assert_eq!(
        cpu.peek(UCSR0A) & flags,
        RXC | UDRE,
        "U2X {u2x} the first byte has no FE, DOR, or UPE"
    );
    assert_eq!(
        read_reg(&mut cpu, UDR0),
        0x55,
        "U2X {u2x} bytes stay in order"
    );
    assert_eq!(
        cpu.peek(UCSR0A) & flags,
        RXC | UDRE,
        "U2X {u2x} the second byte has no FE, DOR, or UPE"
    );
    assert_eq!(read_reg(&mut cpu, UDR0), 0xA3, "U2X {u2x} the second byte");
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "U2X {u2x} both bytes were read");
}

/// Drive an 8N1 frame on `PD0`. `at` is the start edge. Each bit lasts `bit`
/// cycles. The stop level stays until the next drive.
fn drive_rx_frame(cpu: &mut Cpu, at: u64, byte: u8, bit: u64) {
    let mut levels = vec![false];
    levels.extend((0..8).map(|i| (byte >> i) & 1 != 0));
    levels.push(true);
    for (i, high) in levels.into_iter().enumerate() {
        let cycle = at + i as u64 * bit;
        cpu.run_until(cycle);
        assert_eq!(cpu.cycles(), cycle, "frame bit {i} at {cycle}");
        cpu.drive_pin(PD0, if high { Level::High } else { Level::Low });
    }
}

#[test]
fn a_byte_supplied_before_rxen_is_missed() {
    // §20.7. The waveform still leaves the pad. The receiver is off, so the
    // start edge is not a frame.
    let mut cpu = Cpu::new();
    cpu.supply(SupplyChannel::Usart0, &[0x55]);
    cpu.run_until(4);
    assert_eq!(cpu.peek(PIND) & 1, 0, "start bit is low on PD0");
    cpu.run_until(200);
    let mut enable = vec![ldi(16, u16::from(RXEN))];
    enable.extend(sts(16, UCSR0B));
    enable.extend(lds(17, UDR0));
    cpu.load_words(&enable);
    cpu.insns(3);
    assert_eq!(cpu.reg(17), 0, "the frame finished while RXEN was clear");
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0);
}

#[test]
fn ucsr0c_starts_at_reset_value_and_a_store_sticks() {
    let mut cpu = Cpu::new();
    assert_eq!(cpu.peek(UCSR0C), 0x06);
    let mut program = vec![ldi(16, 0x46)];
    program.extend(sts(16, UCSR0C));
    program.extend(lds(17, UCSR0C));
    cpu.load_words(&program);
    cpu.insns(3);
    assert_eq!(cpu.pc(), 5);
    assert_eq!(cpu.reg(17), 0x46);
    assert_eq!(cpu.peek(UCSR0C), 0x46);
}

#[test]
fn write_without_transmitter_is_ignored() {
    let mut program = Vec::new();
    program.push(ldi(16, 0x41));
    program.extend(sts(16, UDR0));
    let mut cpu = Cpu::new();
    cpu.load_words(&program);
    cpu.run_until(400);
    assert_eq!(cpu.take(Channel::Usart0), None);
}

/// Table 20-1, and the 16 MHz / 9600 row of Table 20-7.
#[test]
fn baud_bit_time_follows_table_20_1() {
    // (ubrr, u2x, bit clocks). 207/U2X and 103/normal are the 9600 rows:
    // 8×208 = 16×104 = 1664. 16_000_000/1664 = 9615, which the table prints
    // as 0.2% high.
    let rows = [
        (0u16, false, 16u64),
        (0, true, 8),
        (1, false, 32),
        (207, true, 1664),
        (103, false, 1664),
    ];
    for (ubrr, u2x, bit) in rows {
        let mut cpu = Cpu::new();
        arm_baud(&mut cpu, ubrr, u2x, 0x06);
        let start = send_bytes(&mut cpu, &[0x55]);
        // Poke reloads at cycle 0, so the divider's phase is the power-on one.
        // The store is not a txclk. §20.6.1 waits for the next one, and that
        // wait is shorter than a bit.
        let origin = txclk_at_or_after(ubrr, u2x, start);
        assert!(origin >= start);
        assert!(
            origin - start < bit,
            "UBRR {ubrr} U2X {u2x}: start-bit wait {origin} - {start} is a bit or more"
        );
        cpu.run_until(origin + bit * 10 + 5);
        let (byte, at) = cpu.take(Channel::Usart0).unwrap();
        assert_eq!(byte, 0x55, "UBRR {ubrr} U2X {u2x}");
        assert_eq!(at, origin + bit * 10, "UBRR {ubrr} U2X {u2x} stamp");
        assert_eq!(bit_clocks(ubrr, u2x), bit);
    }
    let closest = 16_000_000u32 / 1664;
    assert_eq!(closest, 9615);
}

/// §20.4 and §20.6. Every format, bit by bit, on PD1.
#[test]
fn every_frame_format_is_on_pd1() {
    struct Case {
        label: &'static str,
        ucsrc: u8,
        extra: u8,
        byte: u8,
        txb8: bool,
        n_data: u8,
        /// `None` is no parity. `Some(false)` is even. `Some(true)` is odd.
        parity_odd: Option<bool>,
        stops: u8,
    }
    let cases = [
        Case {
            label: "5N1",
            ucsrc: 0x00,
            extra: 0,
            byte: 0x1F,
            txb8: false,
            n_data: 5,
            parity_odd: None,
            stops: 1,
        },
        Case {
            label: "6N1",
            ucsrc: 0x02,
            extra: 0,
            byte: 0x2A,
            txb8: false,
            n_data: 6,
            parity_odd: None,
            stops: 1,
        },
        Case {
            label: "7N1",
            ucsrc: 0x04,
            extra: 0,
            byte: 0x55,
            txb8: false,
            n_data: 7,
            parity_odd: None,
            stops: 1,
        },
        Case {
            label: "8N1",
            ucsrc: 0x06,
            extra: 0,
            byte: 0x55,
            txb8: false,
            n_data: 8,
            parity_odd: None,
            stops: 1,
        },
        Case {
            label: "8E1",
            ucsrc: 0x26,
            extra: 0,
            byte: 0x55,
            txb8: false,
            n_data: 8,
            parity_odd: Some(false),
            stops: 1,
        },
        Case {
            label: "8O1",
            ucsrc: 0x36,
            extra: 0,
            byte: 0x55,
            txb8: false,
            n_data: 8,
            parity_odd: Some(true),
            stops: 1,
        },
        Case {
            label: "8N2",
            ucsrc: 0x0E,
            extra: 0,
            byte: 0x81,
            txb8: false,
            n_data: 8,
            parity_odd: None,
            stops: 2,
        },
        Case {
            label: "9N1",
            ucsrc: 0x06,
            extra: UCSZ2,
            byte: 0x01,
            txb8: true,
            n_data: 9,
            parity_odd: None,
            stops: 1,
        },
    ];
    for case in cases {
        let Case {
            label,
            ucsrc,
            extra,
            byte,
            txb8,
            n_data,
            parity_odd,
            stops,
        } = case;
        let mut cpu = Cpu::new();
        arm_baud(&mut cpu, 1, false, ucsrc);
        if extra != 0 || txb8 {
            let mut b = TXEN | extra;
            if txb8 {
                b |= TXB8;
            }
            cpu.poke(UCSR0B, b & !TXEN);
        }
        let start = send_bytes(&mut cpu, &[byte]);
        let bit = 32u64;
        // UBRR 1 reloaded at cycle 0. Commit is cycle 6. Tick k is at 2k,
        // txclk at k = 16, cycle 32 (§20.6.1, Figure 20-2).
        let origin = txclk_at_or_after(1, false, start);
        assert_eq!(origin, 32, "{label}");
        let n_parity = u8::from(parity_odd.is_some());
        let n = 1 + u64::from(n_data) + u64::from(n_parity) + u64::from(stops);
        cpu.run_until(origin + bit * n + 5);
        let edges = tx_edges(&mut cpu);
        let mut expect = Vec::new();
        let mut level = Level::Released;
        let mut push = |next: Level, at: u64| {
            if level != next {
                expect.push((next, at));
                level = next;
            }
        };
        // TXEN's STS ends at cycle 3 and drives the idle mark.
        push(Level::High, 3);
        push(Level::Low, origin);
        let mut data = u16::from(byte) & ((1u16 << n_data) - 1);
        if n_data == 9 && txb8 {
            data |= 0x100;
        }
        for index in 0..n_data {
            let high = (data >> index) & 1 == 1;
            push(
                if high { Level::High } else { Level::Low },
                origin + bit * u64::from(1 + index),
            );
        }
        if let Some(odd) = parity_odd {
            let ones = (data & ((1u16 << n_data) - 1)).count_ones() % 2 == 1;
            let high = if odd { !ones } else { ones };
            push(
                if high { Level::High } else { Level::Low },
                origin + bit * u64::from(1 + n_data),
            );
        }
        push(Level::High, origin + bit * (n - u64::from(stops)));
        assert_eq!(edges, expect, "{label}");
        let (_, at) = cpu.take(Channel::Usart0).unwrap();
        assert_eq!(at, origin + bit * n, "{label} stamp");
    }
}

/// §20.6.1 and §20.6.3. The buffer holds the second byte while the first shifts.
#[test]
fn udre_and_txc_across_two_buffered_writes() {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 1, false, 0x06);
    let start = send_bytes(&mut cpu, &[0x11, 0x22]);
    // The second STS ends 3 cycles after the first (LDI + STS).
    let second = start + 3;
    // UBRR 1, phase 0: start bit at txclk 32, frame 320. §20.6.1.
    let origin = txclk_at_or_after(1, false, start);
    assert_eq!(origin, 32);
    assert_eq!(
        cpu.peek(UCSR0A) & (UDRE | TXC),
        0,
        "buffer holds the second byte"
    );
    cpu.run_until(origin + 32 * 10);
    assert_eq!(
        cpu.peek(UCSR0A) & UDRE,
        UDRE,
        "first frame moved the buffer into the shifter"
    );
    assert_eq!(
        cpu.peek(UCSR0A) & TXC,
        0,
        "the second frame is still shifting"
    );
    let (byte, at) = cpu.take(Channel::Usart0).unwrap();
    assert_eq!((byte, at), (0x11, origin + 320));
    cpu.run_until(origin + 640);
    assert_eq!(
        cpu.peek(UCSR0A) & TXC,
        TXC,
        "both frames are out and the buffer is empty"
    );
    let (byte, at) = cpu.take(Channel::Usart0).unwrap();
    assert_eq!((byte, at), (0x22, origin + 640));
    assert!(second > start);
}

/// §20.6.5. Clearing TXEN finishes the shifter and the buffer, then releases PD1.
#[test]
fn clearing_txen_finishes_both_frames_then_releases_the_pin() {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 1, false, 0x06);
    let start = send_bytes(&mut cpu, &[0x01, 0x02]);
    let origin = txclk_at_or_after(1, false, start);
    let mut clear = vec![ldi(16, 0x00)];
    clear.extend(sts(16, UCSR0B));
    cpu.load_words(&clear);
    cpu.insns(2);
    assert_eq!(cpu.peek(UCSR0B) & TXEN, 0);
    // Two frames from txclk 32: 32 + 2×320 = 672 (§20.6.5).
    cpu.run_until(origin + 640);
    let bytes: Vec<_> = std::iter::from_fn(|| cpu.take(Channel::Usart0)).collect();
    assert_eq!(
        bytes.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
        vec![0x01, 0x02]
    );
    assert_eq!(bytes[1].1, origin + 640);
    let edges = tx_edges(&mut cpu);
    assert_eq!(
        edges.last().copied(),
        Some((Level::Released, origin + 640)),
        "the pin returns to the port after the buffered frame"
    );
    let mut late = vec![ldi(16, 0x7E)];
    late.extend(sts(16, UDR0));
    cpu.load_words(&late);
    cpu.run_until(cpu.cycles() + 100);
    assert_eq!(
        cpu.take(Channel::Usart0),
        None,
        "a write after TXEN clears is ignored"
    );
}

/// §20.8.1. Two high samples of 8/9/10 reject the start bit. A later real frame is kept.
#[test]
fn a_false_start_is_rejected() {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 15, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let sp = 16u64;
    cpu.drive_pin(PD0, Level::High);
    // UBRR 15 reloaded at cycle 0: baud ticks at 16, 32, …. Sample 1 is the
    // tick the falling edge lands on (§20.8.1), so the edge is placed on one.
    cpu.run_until(16);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    assert_eq!(edge, 16);
    // Samples 8/9/10 are 7, 8 and 9 baud ticks after sample 1. Rise before them.
    cpu.run_until(edge + 6 * sp);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 20 * sp);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "majority-high start is noise");

    cpu.drive_pin(PD0, Level::Low);
    let real = cpu.cycles();
    // 0x00, stop high. Bit is 256 clocks. Hold low through the data, then mark.
    cpu.run_until(real + 256 * 9);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(real + 256 * 11);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x00);
}

/// §20.8.2. One of samples 8/9/10 does not decide the bit. Two of them do.
#[test]
fn one_sample_glitch_loses_the_majority_vote() {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 15, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let sp = 16u64;
    // Data bit 0 of 0x00. Sample 1 is the baud tick. Vote samples 8/9/10 of
    // that bit are 23, 24 and 25 baud ticks later (§20.8.2, Figure 20-6).
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(16);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    assert_eq!(edge, 16);
    let s8 = edge + 23 * sp;
    let s9 = edge + 24 * sp;
    let s10 = edge + 25 * sp;
    cpu.run_until(s9 - 1);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(s9 + 1);
    cpu.drive_pin(PD0, Level::Low);
    cpu.run_until(edge + 16 * sp * 10);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 16 * sp * 11);
    assert_eq!(
        read_reg(&mut cpu, UDR0),
        0x00,
        "sample 9 alone does not set bit 0"
    );
    let _ = (s8, s10);

    // Samples 8 and 10 high, sample 9 low: bit 0 becomes 1.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 15, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(32);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    assert_eq!(edge, 32);
    let s8 = edge + 23 * sp;
    let s9 = edge + 24 * sp;
    let s10 = edge + 25 * sp;
    cpu.run_until(s8 - 1);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(s8 + 1);
    cpu.drive_pin(PD0, Level::Low);
    cpu.run_until(s10 - 1);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(s10 + 1);
    cpu.drive_pin(PD0, Level::Low);
    // Rest of the zero data, then a high stop.
    cpu.run_until(edge + 16 * sp * 9);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 16 * sp * 11);
    assert_eq!(
        read_reg(&mut cpu, UDR0),
        0x01,
        "samples 8 and 10 outvote sample 9"
    );
    let _ = s9;
}

/// §20.7.4. FE, DOR, UPE, and the 9-bit RXB8/TXB8 bits.
#[test]
fn fe_dor_upe_and_the_ninth_bit() {
    // Frame error: stop bit low. UBRR 0, bit 16, stop vote at 153.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    cpu.run_until(edge + 200);
    assert_eq!(cpu.peek(UCSR0A) & (RXC | FE), RXC | FE);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x00);
    assert_eq!(
        cpu.peek(UCSR0A) & FE,
        0,
        "FE belongs to the byte that was read"
    );

    // Even parity, wrong parity bit. 0x01 has one 1, so even parity is 1.
    // Send parity 0. Bit time 16. Layout: start, 8 data, parity, stop.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x26);
    cpu.poke(UCSR0B, RXEN);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    // data 0x01: bit 0 high at 16, then low through the parity slot (bit index 9, at 144).
    cpu.run_until(edge + 16);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 32);
    cpu.drive_pin(PD0, Level::Low);
    // Stop high, which is bit index 10, at 160.
    cpu.run_until(edge + 160);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 200);
    assert_eq!(cpu.peek(UCSR0A) & (RXC | UPE), RXC | UPE);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x01);

    // Two in the buffer plus one in the shifter, then a fourth start drops the shifter.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    cpu.supply(SupplyChannel::Usart0, &[0x11, 0x22, 0x33]);
    cpu.run_until(3 * 160 + 20);
    assert_eq!(
        cpu.peek(UCSR0A) & DOR,
        0,
        "three frames fit in the buffer and the shifter"
    );
    assert_eq!(read_reg(&mut cpu, UDR0), 0x11);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x22);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x33);

    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    cpu.supply(SupplyChannel::Usart0, &[0x11, 0x22, 0x33, 0x44]);
    cpu.run_until(4 * 160 + 20);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x11);
    assert_eq!(cpu.peek(UCSR0A) & DOR, 0);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x22);
    assert_eq!(
        cpu.peek(UCSR0A) & (RXC | DOR),
        RXC | DOR,
        "0x33 was lost ahead of 0x44"
    );
    assert_eq!(read_reg(&mut cpu, UDR0), 0x44);
    assert_eq!(cpu.peek(UCSR0A) & DOR, 0);

    // 9-bit RXB8. Bit 8 high, low 8 bits 0x05, stop high.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    cpu.poke(UCSR0B, RXEN | UCSZ2);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    // data bits of 0x05 are bit0=1, bit1=0, bit2=1, rest 0, bit8=1.
    cpu.run_until(edge + 16);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 32);
    cpu.drive_pin(PD0, Level::Low);
    cpu.run_until(edge + 48);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 64);
    cpu.drive_pin(PD0, Level::Low);
    // Ninth bit is frame bit 9, at 9×16.
    cpu.run_until(edge + 144);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 160);
    cpu.drive_pin(PD0, Level::Low);
    // Stop, frame bit 10.
    cpu.run_until(edge + 176);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 220);
    assert_eq!(cpu.peek(UCSR0B) & RXB8, RXB8);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x05);
    assert_eq!(cpu.peek(UCSR0B) & RXB8, 0);
}

/// §20.9. With MPCM set, a 9-bit data frame (RXB8 = 0) is ignored and an address is kept.
#[test]
fn mpcm_keeps_address_frames_only() {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    cpu.poke(UCSR0A, MPCM);
    cpu.poke(UCSR0B, RXEN | UCSZ2);

    // Address: ninth bit high, data 0x0A.
    drive_nine(&mut cpu, 0x0A, true);
    assert_eq!(cpu.peek(UCSR0A) & RXC, RXC);
    assert_eq!(cpu.peek(UCSR0B) & RXB8, RXB8);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x0A);

    // Data: ninth bit low. Not stored.
    drive_nine(&mut cpu, 0x0B, false);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0, "MPCM drops a data frame");

    drive_nine(&mut cpu, 0x0C, true);
    assert_eq!(read_reg(&mut cpu, UDR0), 0x0C);
}

fn drive_nine(cpu: &mut Cpu, byte: u8, ninth: bool) {
    let origin = cpu.cycles();
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(origin + 20);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    for bit in 0..8u8 {
        let at = edge + 16 * (1 + u64::from(bit));
        cpu.run_until(at);
        let high = (byte >> bit) & 1 == 1;
        cpu.drive_pin(PD0, if high { Level::High } else { Level::Low });
    }
    let ninth_at = edge + 16 * 9;
    cpu.run_until(ninth_at);
    cpu.drive_pin(PD0, if ninth { Level::High } else { Level::Low });
    cpu.run_until(edge + 16 * 10);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 16 * 12);
}

/// §20.8.3 for 8N1. Normal mode prints +4.58/−4.64; `U2X` prints +3.90/−4.00.
///
/// `Rslow = (D+1)S / (S − 1 + D·S + SF)` and `Rfast = (D+2)S / ((D+1)S + SM)`
/// with D = 8. The receiver samples on baud ticks, so the start edge can fall
/// anywhere in one sample period. A host bit time inside both ratios is
/// received at every phase. Just past `Rfast`, the stop bit's majority vote
/// fails when sample 1 is late. The 2-of-3 vote (§20.8.1) still accepts one
/// sample outside the bit, so the first host bit time past `Rslow` does not
/// fail; the vote fails once the middle sample leaves, which is still past
/// the table.
#[test]
fn table_20_2_and_20_3_track_the_start_edge_phase() {
    // Normal: S=16, SF=8, SM=9. 144/151 and 160/153.
    assert_eq!((9 * 16, 16 - 1 + 8 * 16 + 8), (144, 151));
    assert_eq!((10 * 16, 9 * 16 + 9), (160, 153));
    let slow = (151 - 144) as f64 / 151.0 * 100.0;
    let fast = (160 - 153) as f64 / 153.0 * 100.0;
    assert!((slow - 4.64).abs() < 0.01, "{slow}");
    assert!((fast - 4.58).abs() < 0.01, "{fast}");
    // U2X: S=8, SF=4, SM=5. 72/75 and 80/77.
    assert_eq!((9 * 8, 8 - 1 + 8 * 8 + 4), (72, 75));
    assert_eq!((10 * 8, 9 * 8 + 5), (80, 77));
    let slow_u2x = (75 - 72) as f64 / 75.0 * 100.0;
    let fast_u2x = (80 - 77) as f64 / 77.0 * 100.0;
    assert!((slow_u2x - 4.00).abs() < 0.01, "{slow_u2x}");
    assert!((fast_u2x - 3.90).abs() < 0.01, "{fast_u2x}");

    // Receiver bit is 1664 either way (UBRR 103, or UBRR 207 with U2X).
    // R = 1664 / host_bit. Past Rfast means host_bit <= 1591 (normal) or
    // 1601 (U2X). Past Rslow means host_bit >= 1745 or 1734.
    sweep_inside(103, false, 16, 8, 9, 1744, 1592);
    sweep_inside(207, true, 8, 4, 5, 1733, 1602);

    // Normal, one host cycle past Rfast (1591). Stop samples 8/9/10 of bit 9
    // sit at phase + 151/152/153 sample periods. The next start is at 10×1591
    // = 15910. Samples 9 and 10 are both in that start when phase >= 102.
    let fast_miss = stop_vote_fails(104, 16, 8, 9, 1591, true);
    assert_eq!(fast_miss, (102..104).collect::<Vec<_>>());
    assert_phases(103, false, 1591, 0x55, true, &fast_miss);

    // U2X, host bit 1601. Samples 4/5/6 are offsets 75/76/77. Next start is
    // 16010. phase + 76×208 >= 16010 when phase >= 202.
    let fast_u2x = stop_vote_fails(208, 8, 4, 5, 1601, true);
    assert_eq!(fast_u2x, (202..208).collect::<Vec<_>>());
    assert_phases(207, true, 1601, 0x55, true, &fast_u2x);

    // One host cycle past Rslow still has only sample SF outside the stop, so
    // the vote holds at every phase. The middle sample leaves at host bit
    // 1757: phase + 152×104 < 9×1757 when phase < 5 (the U2X spacing 76×208
    // is the same product).
    assert!(stop_vote_fails(104, 16, 8, 9, 1745, false).is_empty());
    assert!(stop_vote_fails(208, 8, 4, 5, 1734, false).is_empty());
    let slow_miss = stop_vote_fails(104, 16, 8, 9, 1757, false);
    assert_eq!(slow_miss, vec![0, 1, 2, 3, 4]);
    assert_phases(103, false, 1757, 0x00, false, &slow_miss);
    let slow_u2x = stop_vote_fails(208, 8, 4, 5, 1757, false);
    assert_eq!(slow_u2x, vec![0, 1, 2, 3, 4]);
    assert_phases(207, true, 1757, 0x00, false, &slow_u2x);
}

/// Phases in `0..sp` whose stop-bit majority is off the stop bit.
///
/// Sample `n` of the stop (bit 9 of an 8N1 frame) is `phase + (9S + n − 1)`
/// sample periods after the falling edge. Two of samples SF/SM/SM+1 have to
/// miss the stop (§20.8.1). A fast host's next start begins at 10 bit times;
/// a slow host's stop begins at 9.
fn stop_vote_fails(sp: u64, s: u64, sf: u64, sm: u64, hb: u64, fast: bool) -> Vec<u64> {
    let mut missed = Vec::new();
    for phase in 0..sp {
        let mut outside = 0;
        for sample in [sf, sm, sm + 1] {
            let t = phase + (9 * s + sample - 1) * sp;
            let out = if fast { t >= 10 * hb } else { t < 9 * hb };
            if out {
                outside += 1;
            }
        }
        if outside >= 2 {
            missed.push(phase);
        }
    }
    missed
}

fn sweep_inside(ubrr: u16, u2x: bool, s: u64, sf: u64, sm: u64, slow_hb: u64, fast_hb: u64) {
    let sp = u64::from(ubrr) + 1;
    assert!(stop_vote_fails(sp, s, sf, sm, slow_hb, false).is_empty());
    assert!(stop_vote_fails(sp, s, sf, sm, fast_hb, true).is_empty());
    for phase in 0..sp {
        let (rxc, byte, fe) = hear(ubrr, u2x, slow_hb, phase, 0x55, false);
        assert!(rxc && byte == 0x55 && !fe, "slow {slow_hb} phase {phase}");
        let (rxc, byte, fe) = hear(ubrr, u2x, fast_hb, phase, 0x55, true);
        assert!(rxc && byte == 0x55 && !fe, "fast {fast_hb} phase {phase}");
    }
}

fn assert_phases(ubrr: u16, u2x: bool, hb: u64, byte: u8, follow: bool, miss: &[u64]) {
    let sp = u64::from(ubrr) + 1;
    let mut got = Vec::new();
    for phase in 0..sp {
        let (rxc, data, fe) = hear(ubrr, u2x, hb, phase, byte, follow);
        let bad = !rxc || data != byte || fe;
        if bad {
            got.push(phase);
        }
    }
    assert_eq!(got, miss, "UBRR {ubrr} U2X {u2x} host bit {hb}");
}

/// `(RXC, byte, FE)`. `phase` is how many cycles sample 1 lags the start edge.
fn hear(ubrr: u16, u2x: bool, hb: u64, phase: u64, byte: u8, follow: bool) -> (bool, u8, bool) {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, ubrr, u2x, 0x06);
    cpu.poke(UCSR0B, RXEN);
    let sp = u64::from(ubrr) + 1;
    let edge = sp - phase;
    cpu.set_clock_hz(u32::try_from(hb.saturating_mul(1000)).unwrap());
    cpu.set_host_serial(Channel::Usart0, 1000, SerialFrame::EIGHT_N_1);
    cpu.run_until(edge);
    if follow {
        cpu.supply(SupplyChannel::Usart0, &[byte, 0x00]);
    } else {
        cpu.supply(SupplyChannel::Usart0, &[byte]);
    }
    cpu.run_until(edge + hb * 14 + sp);
    let status = cpu.peek(UCSR0A);
    let rxc = status & RXC != 0;
    let fe = status & FE != 0;
    let data = read_reg(&mut cpu, UDR0);
    (rxc, data, fe)
}

/// §20.7. `drive_pin` is the same RXD wire `supply` modulates.
#[test]
fn drive_pin_and_supply_are_the_same_rxd_wire() {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    cpu.supply(SupplyChannel::Usart0, &[0xA5]);
    cpu.run_until(200);
    assert_eq!(read_reg(&mut cpu, UDR0), 0xA5);

    // Idle high, then the same 0xA5 by driving the pad. Bit time 16.
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(cpu.cycles() + 20);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    let byte = 0xA5u8;
    for bit in 0..8u8 {
        cpu.run_until(edge + 16 * (1 + u64::from(bit)));
        let high = (byte >> bit) & 1 == 1;
        cpu.drive_pin(PD0, if high { Level::High } else { Level::Low });
    }
    cpu.run_until(edge + 160);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(edge + 200);
    assert_eq!(read_reg(&mut cpu, UDR0), 0xA5);
}

/// Figure 20-2. `UBRRL` reloads the down-counter and does not clear the divider.
#[test]
fn ubrrl_write_reloads_the_counter_and_keeps_the_divider() {
    let mut cpu = Cpu::new();
    // UBRR 1 at cycle 0: baud ticks at 2, 4, …. txclk when the tick index is
    // a multiple of 16, so at cycle 32. The store at cycle 6 launches there.
    arm_baud(&mut cpu, 1, false, 0x06);
    let start = send_bytes(&mut cpu, &[0x01]);
    assert_eq!(start, 6);
    let origin = txclk_at_or_after(1, false, start);
    assert_eq!(origin, 32);
    // Eight cycles into the start bit, write UBRR 3. The tick at cycle 40 is
    // index 20. The divider stays at 20, so the next txclk is index 32:
    // 40 + 12 × 4 = 88. A divider reset would make that 40 + 16 × 4 = 104,
    // which is also what stretching the bit to one new bit-time would do.
    let write_at = 40u64;
    cpu.run_until(write_at - 3);
    let mut program = vec![ldi(16, 3)];
    program.extend(sts(16, UBRR0L));
    cpu.load_words(&program);
    cpu.insns(2);
    let wrote = cpu.cycles();
    assert_eq!(wrote, write_at);
    cpu.run_until(wrote + 64 * 12);
    let edges = tx_edges(&mut cpu);
    assert!(
        edges.contains(&(Level::High, 88)),
        "data bit 0 rises at the kept-divider txclk, got {edges:?}"
    );
    assert!(
        !edges.iter().any(|(_, at)| *at == 104),
        "the divider was not cleared and the bit was not stretched to write+64"
    );
    let (byte, at) = cpu.take(Channel::Usart0).unwrap();
    assert_eq!(byte, 0x01, "the character is not rewritten");
    // Start bit ends at 88. Nine bits remain at the new bit time 16 × 4 = 64.
    assert_eq!(at, 88 + 9 * 64);
}

/// Table 12-6 vectors 19–21. UDRE is level-sensitive. Entry clears TXC.
#[test]
fn udre_reenters_and_txc_clears_on_entry() {
    // Vector 20 is word 38. The handler counts entries and leaves UDRIE set.
    let mut words = vec![0u16; 48];
    words[0] = rjmp_to(0, 42);
    words[38] = 0x9503; // INC r16
    words[39] = 0x9518; // RETI
    words[42] = ldi(16, 0);
    words[43] = ldi(17, u16::from(TXEN | UDRIE));
    words[44] = sts(17, UCSR0B)[0];
    words[45] = UCSR0B;
    words[46] = 0x9478; // SEI
    words[47] = 0xCFFF;
    let mut cpu = Cpu::new();
    cpu.load_words(&words);
    cpu.run_until(80);
    assert!(
        cpu.reg(16) >= 2,
        "UDRE stayed set, so the vector re-entered, r16={}",
        cpu.reg(16)
    );

    // One byte, wait for TXC, then take the TX vector. Entry clears TXC.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    let start = send_bytes(&mut cpu, &[0x41]);
    // UBRR 0, phase 0: txclk at cycle 16, 8N1 ends at 176. §20.6.3.
    let origin = txclk_at_or_after(0, false, start);
    assert_eq!(origin, 16);
    cpu.run_until(origin + 160);
    assert_eq!(cpu.peek(UCSR0A) & TXC, TXC);
    let mut words = vec![0u16; 48];
    words[0] = rjmp_to(0, 42);
    // Vector 21 is word 40. LDS r17, UCSR0A.
    let lds_tx = lds(17, UCSR0A);
    words[40] = lds_tx[0];
    words[41] = lds_tx[1];
    words[42] = ldi(18, u16::from(TXEN | TXCIE));
    words[43] = sts(18, UCSR0B)[0];
    words[44] = UCSR0B;
    words[45] = 0x9478;
    words[46] = 0xCFFF;
    cpu.load_words(&words);
    cpu.run_until(cpu.cycles() + 40);
    assert_eq!(
        cpu.reg(17) & TXC,
        0,
        "the handler's first read sees TXC already clear"
    );
    assert_eq!(cpu.peek(UCSR0A) & TXC, 0);

    // RX vector while RXC is set. Reading UDR in the handler clears it.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 0, false, 0x06);
    cpu.poke(UCSR0B, RXEN);
    cpu.supply(SupplyChannel::Usart0, &[0x5A]);
    cpu.run_until(200);
    assert_eq!(cpu.peek(UCSR0A) & RXC, RXC);
    let mut words = vec![0u16; 48];
    words[0] = rjmp_to(0, 42);
    let read = lds(17, UDR0);
    words[36] = read[0];
    words[37] = read[1];
    words[42] = ldi(18, u16::from(RXEN | RXCIE));
    words[43] = sts(18, UCSR0B)[0];
    words[44] = UCSR0B;
    words[45] = 0x9478;
    words[46] = 0xCFFF;
    cpu.load_words(&words);
    cpu.run_until(cpu.cycles() + 30);
    assert_eq!(cpu.reg(17), 0x5A);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0);
}

fn rjmp_to(from: u16, to: u16) -> u16 {
    let k = to.wrapping_sub(from.wrapping_add(1)) & 0x0FFF;
    0xC000 | k
}

/// A jump to the next bit and a NOP per cycle see the same wire.
/// The stamp is the datasheet frame length, not merely the other path.
#[test]
fn sleep_jump_matches_stepped_nops_and_the_datasheet_stamp() {
    let stepped = frame_trace(false);
    let jumped = frame_trace(true);
    assert_eq!(stepped.0, jumped.0, "pin edges");
    assert_eq!(stepped.1, jumped.1, "byte stamp");
    let (byte, at) = stepped.1;
    assert_eq!(byte, 0x55);
    // UBRR 1 reloaded at cycle 0 (phase 0). STS ends at cycle 6. The next
    // txclk is baud tick 16 at cycle 32, and the stop bit ends 10 bits later.
    assert_eq!(at, 32 + 32 * 10);
}

fn frame_trace(sleep: bool) -> (Vec<(Level, u64)>, (u8, u64)) {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 1, false, 0x06);
    let mut words = vec![ldi(16, u16::from(TXEN))];
    words.extend(sts(16, UCSR0B));
    words.push(ldi(17, 0x55));
    words.extend(sts(17, UDR0));
    if sleep {
        words.push(0x9588);
    } else {
        words.push(0xCFFF);
    }
    cpu.load_words(&words);
    // Commit at cycle 6, txclk at 32, frame ends at 352.
    cpu.run_until(32 + 32 * 10 + 5);
    let edges = tx_edges(&mut cpu);
    let byte = cpu.take(Channel::Usart0).unwrap();
    (edges, byte)
}

/// §20.11.4. Synchronous `UMSEL` does not shift an async frame.
/// `UBRRHn` bits 7:4 read back as zero.
#[test]
fn synchronous_umsel_does_not_shift_and_ubrrh_reserved_bits_read_zero() {
    let mut cpu = Cpu::new();
    let mut words = vec![ldi(16, u16::from(TXEN))];
    words.extend(sts(16, UCSR0B));
    words.push(ldi(17, 0x41));
    words.extend(sts(17, UDR0));
    words.push(ldi(18, 0x46)); // UMSEL = 01: synchronous mode
    words.extend(sts(18, UCSR0C));
    words.push(ldi(19, 0xF0));
    words.extend(sts(19, UBRR0H));
    words.push(ldi(20, 0x86)); // UMSEL = 10: reserved
    words.extend(sts(20, UCSR0C));
    cpu.load_words(&words);
    cpu.insns(16);
    assert_eq!(
        cpu.peek(UDR0),
        0,
        "the data register is not a readable latch"
    );
    assert_eq!(cpu.peek(UBRR0H), 0, "UBRRH bits 7:4 read back as zero");
    cpu.run_until(cpu.cycles() + 400);
    assert_eq!(
        cpu.take(Channel::Usart0),
        None,
        "synchronous mode does not shift an async frame"
    );
}

/// §14.3.3. TXEN forces PD1 to an output. RXEN forces PD0 to an input and keeps the pull-up.
#[test]
fn txen_and_rxen_override_the_port_direction() {
    let mut cpu = Cpu::new();
    // DDRD1 = 0, PORTD1 = 0. TXEN still drives a mark.
    let mut words = vec![ldi(16, u16::from(TXEN))];
    words.extend(sts(16, UCSR0B));
    cpu.load_words(&words);
    cpu.insns(2);
    assert_eq!(
        cpu.pin_levels().get(PD1),
        Some(Level::High),
        "TXD idles high while DDR is an input"
    );
    assert_eq!(Port::D.pin_id(1), PD1);

    // DDRD0 = 1 would have driven PORTD0. RXEN forces an input, and PORTD0 pulls it up.
    let mut cpu = Cpu::new();
    cpu.poke(DDRD, 1);
    cpu.poke(PORTD, 1);
    let mut words = vec![ldi(16, u16::from(RXEN))];
    words.extend(sts(16, UCSR0B));
    cpu.load_words(&words);
    cpu.insns(2);
    assert_eq!(cpu.pin_levels().get(PD0), Some(Level::High));
    cpu.load_words(&nops_then_in());
    cpu.run_until(cpu.cycles() + 4);
    // The synchronizer publishes the pull-up. A read after two more cycles sees it.
    assert_eq!(
        cpu.peek(PIND) & 1,
        1,
        "RXEN leaves the pull-up under PORTD0"
    );
}
fn nops_then_in() -> Vec<u16> {
    vec![0x0000, 0x0000]
}

/// §20.6.1. An idle store just before a `txclk` waits one clock; a store just
/// after waits until the next one. Both waits are shorter than a bit.
#[test]
fn idle_store_launches_on_the_surrounding_txclk() {
    // UBRR 0 from reset. Baud tick k is cycle k. txclk is cycle 16, 32, ….
    // A bit is 16 clocks (Table 20-1) and an 8N1 stop ends 160 clocks later.
    for (commit, txclk) in [(15u64, 16u64), (16, 16), (17, 32)] {
        let mut cpu = Cpu::new();
        let words = commit_program(commit, 0x55);
        cpu.load_words(&words);
        cpu.run_until(commit);
        assert_eq!(cpu.cycles(), commit);
        assert_eq!(cpu.peek(UCSR0A) & UDRE, UDRE, "the shifter took the byte");
        let wait = txclk - commit;
        assert!(wait < 16, "commit {commit} waited a full bit");
        cpu.run_until(txclk + 160);
        let edges = tx_edges(&mut cpu);
        assert!(
            edges.contains(&(Level::Low, txclk)),
            "commit {commit} start bit, got {edges:?}"
        );
        assert_eq!(cpu.take(Channel::Usart0), Some((0x55, txclk + 160)));
    }
}

fn commit_program(commit: u64, byte: u8) -> Vec<u16> {
    // LDI+STS ends at cycle 3. LDI+STS of the byte is 3 cycles. The NOPs
    // fill commit - 6 so the data STS ends on `commit`.
    let nops = commit - 6;
    let mut words = vec![ldi(16, u16::from(TXEN))];
    words.extend(sts(16, UCSR0B));
    words.extend(std::iter::repeat_n(0u16, usize::try_from(nops).unwrap()));
    words.push(ldi(17, u16::from(byte)));
    words.extend(sts(17, UDR0));
    words.push(0xCFFF);
    words
}

/// Figure 20-2. The write moves the next baud tick by at most `UBRR + 1`.
#[test]
fn ubrrl_write_moves_the_next_baud_tick_by_at_most_one_period() {
    // U2X from the store at cycle 3, UBRR still 0, so a baud tick is a clock
    // and txclk is every 8th (cycles 8, 16, …). UBRRL = 4 retires at cycle 9.
    // The pending tick was cycle 10. The reload makes the next one 9+5 = 14.
    // |14 − 10| = 4, and UBRR + 1 = 5.
    let mut cpu = Cpu::new();
    let mut words = vec![ldi(16, u16::from(U2X))];
    words.extend(sts(16, UCSR0A));
    words.extend([0u16, 0, 0]);
    words.push(ldi(17, 4));
    words.extend(sts(17, UBRR0L));
    words.push(ldi(18, u16::from(TXEN)));
    words.extend(sts(18, UCSR0B));
    words.push(ldi(19, 0x01));
    words.extend(sts(19, UDR0));
    words.push(0xCFFF);
    cpu.load_words(&words);
    cpu.run_until(15);
    assert_eq!(cpu.cycles(), 15, "the byte is committed at cycle 15");
    let old_tick = 10u64;
    let new_tick = 9u64 + 5;
    assert!(new_tick.abs_diff(old_tick) <= 4 + 1);
    assert_eq!(new_tick - 9, 5, "the next zero is one loaded period later");
    // Tick index 9 is kept, so txclk is index 16: 9 + 7×5 = 44. Resetting
    // the divider would put it at 9 + 8×5 = 49. Data bit 0 of 0x01 then
    // rises one txclk later, index 24, at 9 + 15×5 = 84.
    cpu.run_until(100);
    let edges = tx_edges(&mut cpu);
    assert!(
        edges.contains(&(Level::Low, 44)),
        "start bit at the kept divider, got {edges:?}"
    );
    assert!(edges.contains(&(Level::High, 84)), "{edges:?}");
    assert!(!edges.iter().any(|(_, at)| *at == 49));
}

/// A power-on reset drops the shifter, the host waveform, the queues, and the divider. An in-run
/// reset keeps the bytes already sent, which `tests/sent.rs` checks.
#[test]
fn reset_clears_usart_state_waveform_queues_and_phase() {
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 3, true, 0x06);
    cpu.poke(UCSR0B, TXEN | RXEN);
    let start = send_bytes(&mut cpu, &[0x55]);
    cpu.supply(SupplyChannel::Usart0, &[0x11, 0x22]);
    cpu.run_until(start + 40);
    orbe_avr::test_hooks::reset_cpu(&mut cpu, orbe_avr::ResetKind::PowerOn);
    assert_eq!(cpu.cycles(), 0);
    assert_eq!(cpu.peek(UBRR0L), 0);
    assert_eq!(cpu.peek(UBRR0H), 0);
    assert_eq!(cpu.peek(UCSR0B), 0);
    assert_eq!(cpu.peek(UCSR0A) & UDRE, UDRE);
    assert_eq!(cpu.peek(UCSR0A) & (RXC | TXC | FE | DOR | UPE), 0);
    assert_eq!(cpu.take(Channel::Usart0), None);
    assert_eq!(cpu.take_pin(), None);

    // Phase 0 again: UBRR 0, commit at 6, txclk at 16, stop at 176.
    let mut fresh = Cpu::new();
    let again = send_bytes(&mut cpu, &[0x41]);
    let fresh_at = send_bytes(&mut fresh, &[0x41]);
    assert_eq!((again, fresh_at), (6, 6));
    cpu.run_until(176);
    fresh.run_until(176);
    assert_eq!(cpu.take(Channel::Usart0), Some((0x41, 176)));
    assert_eq!(fresh.take(Channel::Usart0), Some((0x41, 176)));

    let mut cpu = Cpu::new();
    cpu.poke(UCSR0B, RXEN);
    cpu.supply(SupplyChannel::Usart0, &[0x5A]);
    cpu.run_until(40);
    orbe_avr::test_hooks::reset_cpu(&mut cpu, orbe_avr::ResetKind::PowerOn);
    let mut enable = vec![ldi(16, u16::from(RXEN))];
    enable.extend(sts(16, UCSR0B));
    enable.extend(lds(17, UDR0));
    cpu.load_words(&enable);
    cpu.run_until(400);
    assert_eq!(cpu.reg(17), 0, "the host waveform did not survive reset");
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0);
}

/// §20.9. For a 5- to 8-bit character the first stop bit is the address flag.
#[test]
fn mpcm_uses_the_stop_bit_as_the_address_flag() {
    for (n_data, ucsrc) in [(5u8, 0x00u8), (6, 0x02), (7, 0x04), (8, 0x06)] {
        let byte = 0x15u8 & ((1u16 << n_data) - 1) as u8;
        let mut cpu = Cpu::new();
        arm_baud(&mut cpu, 0, false, ucsrc);
        cpu.poke(UCSR0A, MPCM);
        cpu.poke(UCSR0B, RXEN);
        drive_char(&mut cpu, n_data, byte, true);
        assert_eq!(cpu.peek(UCSR0A) & (RXC | FE), RXC, "{n_data}-bit address");
        assert_eq!(read_reg(&mut cpu, UDR0), byte);

        drive_char(&mut cpu, n_data, byte, false);
        assert_eq!(
            cpu.peek(UCSR0A) & RXC,
            0,
            "{n_data}-bit data frame (stop low) is dropped"
        );
    }

    // UBRR 15, edge at cycle 20. Sample 1 is the next baud tick, cycle 32.
    // Stop samples 8/9/10 are 32 + 151/152/153 × 16 = 2448, 2464, 2480.
    // High only across the first two, so this model keeps the address frame.
    // An edge-aligned sampler's samples are 12 clocks earlier (2436, 2452,
    // 2468) and vote the stop low.
    let mut cpu = Cpu::new();
    arm_baud(&mut cpu, 15, false, 0x06);
    cpu.poke(UCSR0A, MPCM);
    cpu.poke(UCSR0B, RXEN);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(20);
    cpu.drive_pin(PD0, Level::Low);
    assert_eq!(cpu.cycles(), 20);
    cpu.run_until(2447);
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(2465);
    cpu.drive_pin(PD0, Level::Low);
    cpu.run_until(2520);
    assert_eq!(cpu.peek(UCSR0A) & RXC, RXC);
    assert_eq!(cpu.peek(UCSR0A) & FE, 0, "the stop vote is high");
    assert_eq!(read_reg(&mut cpu, UDR0), 0x00);
}

fn drive_char(cpu: &mut Cpu, n_data: u8, byte: u8, stop_high: bool) {
    if cpu.cycles() < 1 {
        cpu.run_until(1);
    }
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(cpu.cycles() + 20);
    cpu.drive_pin(PD0, Level::Low);
    let edge = cpu.cycles();
    for bit in 0..n_data {
        cpu.run_until(edge + 16 * (1 + u64::from(bit)));
        let high = (byte >> bit) & 1 == 1;
        cpu.drive_pin(PD0, if high { Level::High } else { Level::Low });
    }
    cpu.run_until(edge + 16 * u64::from(1 + n_data));
    cpu.drive_pin(PD0, if stop_high { Level::High } else { Level::Low });
    cpu.run_until(edge + 16 * u64::from(n_data + 4));
    cpu.drive_pin(PD0, Level::High);
    cpu.run_until(cpu.cycles() + 30);
}

#[test]
fn txc_clears_when_that_bit_is_written() {
    let mut program = vec![ldi(16, 0x08)];
    program.extend(sts(16, UCSR0B));
    program.push(ldi(17, 0x41));
    program.extend(sts(17, UDR0));
    program.extend(lds(18, UCSR0A));
    program.push(ldi(19, u16::from(TXC)));
    program.extend(sts(19, UCSR0A));
    program.extend(lds(20, UCSR0A));

    let mut cpu = Cpu::new();
    cpu.load_words(&program);
    cpu.insns(6);
    // UBRR is 0, so the 8N1 frame is 160 clocks and has not finished. UDRE is
    // set again because the idle shifter took the byte at the store (§20.6.1).
    assert_eq!(cpu.reg(18) & (TXC | (1 << 5)), 1 << 5);
    cpu.run_until(400);
    assert_eq!(
        cpu.peek(UCSR0A) & TXC,
        TXC,
        "TXC sets when the stop bit ends"
    );
    let mut clear = vec![ldi(19, u16::from(TXC))];
    clear.extend(sts(19, UCSR0A));
    clear.extend(lds(20, UCSR0A));
    cpu.load_words(&clear);
    cpu.insns(3);
    assert_eq!(cpu.reg(20) & (TXC | (1 << 5)), 1 << 5);
    assert_eq!(cpu.take(Channel::Usart0).map(|(byte, _)| byte), Some(0x41));
}

#[test]
fn poke_of_usart_does_not_move_a_byte() {
    let mut cpu = Cpu::new();
    cpu.supply(SupplyChannel::Usart0, &[0x55]);
    cpu.poke(UCSR0B, 0x10);
    assert_eq!(cpu.peek(UCSR0A) & RXC, 0);
    cpu.poke(UCSR0B, 0x08);
    cpu.poke(UDR0, 0x41);
    assert_eq!(cpu.peek(UDR0), 0, "poke does not plant a data byte");
    assert_eq!(cpu.peek(UCSR0A) & TXC, 0);
    assert_eq!(cpu.take(Channel::Usart0).map(|(byte, _)| byte), None);
}
