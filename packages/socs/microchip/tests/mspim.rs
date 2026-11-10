//! USART in SPI mode on the ATmega328P, from DS40002061B chapter 21.
//!
//! The baud generator is the free-running one from chapter 20. Table 21-1
//! makes one bit `2 × (UBRR + 1)` cycles, so XCK toggles every `UBRR + 1`.
//! A bit boundary is the next even baud tick at or after the `UDR` store
//! retires. `LDI` is 1 cycle and `STS` is 2, so the store's retire cycle is
//! known before the run.

use crate::common;
use common::Instructions;
use common::{UCSR0B, UDR0, ldi, lds, spin, sts};

use orbe_microchip::{Channel, Cpu, Level, SupplyChannel};

const UCSR0A: u16 = 0x00C0;
const UCSR0C: u16 = 0x00C2;
const UBRR0L: u16 = 0x00C4;
const UBRR0H: u16 = 0x00C5;
const DDRD: u16 = 0x002A;
const PORTD: u16 = 0x002B;

const TXEN: u8 = 1 << 3;
const RXEN: u8 = 1 << 4;
const UDRIE: u8 = 1 << 5;
const TXCIE: u8 = 1 << 6;
const RXCIE: u8 = 1 << 7;
const UDRE: u8 = 1 << 5;
const TXC: u8 = 1 << 6;
const RXC: u8 = 1 << 7;
const FE: u8 = 1 << 4;
const DOR: u8 = 1 << 3;
const UPE: u8 = 1 << 2;

const UMSEL: u8 = 0xC0;
const UDORD: u8 = 1 << 2;
const UCPHA: u8 = 1 << 1;
const UCPOL: u8 = 1 << 0;

/// PD1 is TXD/MOSI. PD4 is XCK. PD0 is RXD/MISO.
const PD1: u32 = 17;
const PD4: u32 = 20;

type LevelAt = (Level, u64);

fn pin_edges(cpu: &mut Cpu) -> (Vec<LevelAt>, Vec<LevelAt>) {
    let mut xck = Vec::new();
    let mut mosi = Vec::new();
    while let Some(report) = cpu.take_pin() {
        if report.pin == PD4 {
            xck.push((report.level, report.cycle));
        } else if report.pin == PD1 {
            mosi.push((report.level, report.cycle));
        }
    }
    (xck, mosi)
}

fn drain(cpu: &mut Cpu) {
    while cpu.take_pin().is_some() {}
}

/// `UCSRC` then `TXEN`/`RXEN`. Returns the cycle the next instruction starts.
fn arm(cpu: &mut Cpu, ubrr: u16, ucsrc: u8, ddr: u8) {
    cpu.poke(UBRR0L, ubrr as u8);
    cpu.poke(UBRR0H, (ubrr >> 8) as u8);
    cpu.poke(DDRD, ddr);
    cpu.poke(PORTD, 0);
    let mut words = vec![ldi(16, u16::from(ucsrc))];
    words.extend(sts(16, UCSR0C));
    words.push(ldi(16, u16::from(TXEN | RXEN)));
    words.extend(sts(16, UCSR0B));
    cpu.load_words(&words);
    cpu.insns(4);
    drain(cpu);
}

fn transfer(byte: u8) -> Vec<u16> {
    let mut words = vec![ldi(16, u16::from(byte))];
    words.extend(sts(16, UDR0));
    words.push(spin());
    words
}

/// Table 21-1 and the even baud tick. XCK idles at `UCPOL` and is already
/// low when `DDR` drives `PORT` 0, so the idle drive at the boundary is an
/// edge only when `UCPOL` is 1. TXD idles high after `TXEN` (§20.6.1, kept
/// for MSPIM by §21.2).
fn expect_frame(retire: u64, ubrr: u16, cpol: bool, cpha: bool, lsb: bool, byte: u8) -> Frame {
    let half = u64::from(ubrr) + 1;
    let mut tick = retire.div_ceil(half).max(1);
    if tick % 2 != 0 {
        tick += 1;
    }
    let boundary = tick * half;
    let idle = if cpol { Level::High } else { Level::Low };
    let mut xck_now = Level::Low;
    let mut mosi_now = Level::High;
    let mut xck = Vec::new();
    let mut mosi = Vec::new();
    let note = |list: &mut Vec<LevelAt>, now: &mut Level, level: Level, at: u64| {
        if level != *now {
            list.push((level, at));
            *now = level;
        }
    };
    let bit = |index: u8| {
        let shift = if lsb { index } else { 7 - index };
        if (byte >> shift) & 1 == 1 {
            Level::High
        } else {
            Level::Low
        }
    };
    note(&mut xck, &mut xck_now, idle, boundary);
    if !cpha {
        note(&mut mosi, &mut mosi_now, bit(0), boundary);
    }
    for index in 0u8..8 {
        let leading = boundary + (2 * u64::from(index) + 1) * half;
        let trailing = boundary + (2 * u64::from(index) + 2) * half;
        if cpha {
            note(&mut mosi, &mut mosi_now, bit(index), leading);
        }
        let lead = if cpol { Level::Low } else { Level::High };
        note(&mut xck, &mut xck_now, lead, leading);
        note(&mut xck, &mut xck_now, idle, trailing);
        if !cpha && index < 7 {
            note(&mut mosi, &mut mosi_now, bit(index + 1), trailing);
        }
    }
    Frame {
        xck,
        mosi,
        end: boundary + 16 * half,
    }
}

struct Frame {
    xck: Vec<LevelAt>,
    mosi: Vec<LevelAt>,
    end: u64,
}

fn run_frame(ubrr: u16, ucsrc: u8, byte: u8) -> (Cpu, u64) {
    let mut cpu = Cpu::new();
    arm(&mut cpu, ubrr, ucsrc, (1 << 1) | (1 << 4));
    let retire = cpu.cycles() + 3;
    cpu.load_words(&transfer(byte));
    let end = expect_frame(
        retire,
        ubrr,
        ucsrc & UCPOL != 0,
        ucsrc & UCPHA != 0,
        ucsrc & UDORD != 0,
        byte,
    )
    .end;
    cpu.run_until(end);
    (cpu, retire)
}

#[test]
fn table_21_1_bit_time_is_two_baud_ticks() {
    // UBRR 0 is fosc/2. UBRR 1 is fosc/4. UBRR 7 is fosc/16. The first XCK
    // edge is the leading edge after the even tick, not the `UDR` write.
    for ubrr in [0u16, 1, 7] {
        let (mut cpu, retire) = run_frame(ubrr, UMSEL, 0xA5);
        let want = expect_frame(retire, ubrr, false, false, false, 0xA5);
        let (xck, mosi) = pin_edges(&mut cpu);
        assert_eq!(xck, want.xck, "UBRR {ubrr} retire {retire}");
        assert_eq!(mosi, want.mosi, "UBRR {ubrr} retire {retire}");
        assert_eq!(cpu.take(Channel::Usart0), Some((0xA5, want.end)));
        let half = u64::from(ubrr) + 1;
        let rise = want.xck.iter().find(|edge| edge.0 == Level::High).unwrap();
        assert_eq!(rise.1, want.end - 16 * half + half);
    }
}

#[test]
fn the_first_edge_follows_the_free_running_even_tick() {
    // A store that retires on tick 9 waits for even tick 10. Two `NOP`s retire
    // on tick 11, so the boundary is the following even tick.
    let mut odd = Cpu::new();
    arm(&mut odd, 0, UMSEL, (1 << 1) | (1 << 4));
    let odd_retire = odd.cycles() + 3;
    odd.load_words(&transfer(0x01));
    let odd_end = expect_frame(odd_retire, 0, false, false, false, 0x01).end;
    odd.run_until(odd_end);
    assert_eq!(odd.take(Channel::Usart0).unwrap().1, odd_end);
    assert_eq!(odd_retire % 2, 1);

    // Two `NOP`s move the retire from cycle 9 to cycle 11, past even tick 10,
    // so the boundary is tick 12 instead of tick 10.
    let mut even = Cpu::new();
    arm(&mut even, 0, UMSEL, (1 << 1) | (1 << 4));
    let even_retire = even.cycles() + 2 + 3;
    let mut words = vec![0x0000, 0x0000];
    words.extend(transfer(0x01));
    even.load_words(&words);
    let even_end = expect_frame(even_retire, 0, false, false, false, 0x01).end;
    even.run_until(even_end);
    assert_eq!(
        odd_end, 26,
        "retire 9 waits for tick 10; eight bits end at 26"
    );
    assert_eq!(even_end, 28, "retire 11 waits for tick 12");
    assert_eq!(even.take(Channel::Usart0).unwrap().1, even_end);
}

#[test]
fn table_21_2_modes_and_both_data_orders() {
    // 0xA5 is 0b10100101. Mode bits are UCPOL (Table 21-2's column) and UCPHA.
    let cases = [
        (0u8, false, false),
        (UCPHA, false, true),
        (UCPOL, true, false),
        (UCPOL | UCPHA, true, true),
    ];
    for (bits, cpol, cpha) in cases {
        for (order, lsb) in [(0u8, false), (UDORD, true)] {
            let ucsrc = UMSEL | bits | order;
            let (mut cpu, retire) = run_frame(1, ucsrc, 0xA5);
            let want = expect_frame(retire, 1, cpol, cpha, lsb, 0xA5);
            let (xck, mosi) = pin_edges(&mut cpu);
            assert_eq!(xck, want.xck, "mode {bits:#x} order {order:#x}");
            assert_eq!(mosi, want.mosi, "mode {bits:#x} order {order:#x}");
        }
    }
}

#[test]
fn buffered_tx_keeps_udre_and_txc_and_has_no_gap() {
    // §21.5. Two `UDR` writes: the idle shifter takes the first immediately
    // (`UDRE` stays set) and the second waits (`UDRE` clears). The second
    // byte starts on the first byte's last trailing edge. `TXC` sets only
    // when the shifter goes idle (§21.6.1, same flag as the UART).
    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, UMSEL, (1 << 1) | (1 << 4));
    let base = cpu.cycles();
    let mut words = vec![ldi(16, 0x11)];
    words.extend(sts(16, UDR0));
    words.push(ldi(17, 0x22));
    words.extend(sts(17, UDR0));
    words.push(spin());
    cpu.load_words(&words);
    let first = base + 3;
    let second = first + 3;
    cpu.run_until(second);
    assert_eq!(cpu.peek(UCSR0A) & UDRE, 0);
    assert_eq!(cpu.peek(UCSR0A) & TXC, 0);
    let end1 = expect_frame(first, 0, false, false, false, 0x11).end;
    cpu.run_until(end1);
    assert_eq!(cpu.peek(UCSR0A) & UDRE, UDRE);
    assert_eq!(cpu.peek(UCSR0A) & TXC, 0);
    let end2 = end1 + 16;
    cpu.run_until(end2);
    assert_eq!(cpu.peek(UCSR0A) & TXC, TXC);
    assert_eq!(cpu.take(Channel::Usart0), Some((0x11, end1)));
    assert_eq!(cpu.take(Channel::Usart0), Some((0x22, end2)));
}

#[test]
fn miso_comes_from_the_usart_channel_and_error_bits_read_zero() {
    // §21.2. Parity, framing, and the data-recovery logic are disabled, so
    // `FE`/`DOR`/`UPE` read as zero. An empty host queue shifts in 0xFF.
    // `Channel::Usart0` is the MISO queue while `UMSEL` is 11.
    let mut empty = Cpu::new();
    arm(&mut empty, 0, UMSEL, (1 << 1) | (1 << 4));
    let retire = empty.cycles() + 3;
    empty.load_words(&transfer(0x00));
    let end = expect_frame(retire, 0, false, false, false, 0x00).end;
    empty.run_until(end);
    empty.load_words(&lds(17, UDR0));
    empty.insns(1);
    assert_eq!(empty.reg(17), 0xFF);
    assert_eq!(empty.peek(UCSR0A) & (FE | DOR | UPE), 0);

    let mut supplied = Cpu::new();
    arm(&mut supplied, 0, UMSEL, (1 << 1) | (1 << 4));
    supplied.supply(SupplyChannel::Usart0, &[0x3C]);
    let retire = supplied.cycles() + 3;
    supplied.load_words(&transfer(0xA5));
    let end = expect_frame(retire, 0, false, false, false, 0xA5).end;
    supplied.run_until(end);
    assert_eq!(supplied.peek(UCSR0A) & RXC, RXC);
    supplied.load_words(&lds(17, UDR0));
    supplied.insns(1);
    assert_eq!(supplied.reg(17), 0x3C);
    assert_eq!(supplied.peek(UCSR0A) & RXC, 0);
}

#[test]
fn xck_is_silent_until_ddr_is_an_output() {
    // §21.3. DDR_XCK must be one. TXD is still forced by TXEN (§21.2).
    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, UMSEL, 1 << 1);
    let retire = cpu.cycles() + 3;
    cpu.load_words(&transfer(0xA5));
    let end = expect_frame(retire, 0, false, false, false, 0xA5).end;
    cpu.run_until(end);
    assert_eq!(pin_edges(&mut cpu).0, Vec::<LevelAt>::new());
}

#[test]
fn umsel_11_reads_back_as_mspim() {
    let mut cpu = Cpu::new();
    let mut words = vec![ldi(16, u16::from(UMSEL))];
    words.extend(sts(16, UCSR0C));
    cpu.load_words(&words);
    cpu.insns(2);
    assert_eq!(cpu.peek(UCSR0C) & UMSEL, UMSEL, "MSPIM mode is stored");
}

#[test]
fn rxc_txc_and_udre_use_the_usart_vectors() {
    // Table 12-6. USART_RX is vector 19 (word 0x24), UDRE is 20 (0x26),
    // TX is 21 (0x28). Entry clears TXC, the same as asynchronous mode.
    let mut rx = Cpu::new();
    arm(&mut rx, 0, UMSEL, (1 << 1) | (1 << 4));
    rx.poke(UCSR0B, TXEN | RXEN | RXCIE);
    rx.set_sreg(0x80);
    let retire = rx.cycles() + 3;
    let mut words = transfer(0xA5);
    words.resize(0x24, 0x0000);
    words.push(spin());
    rx.load_words(&words);
    let end = expect_frame(retire, 0, false, false, false, 0xA5).end;
    rx.run_until(end + 16);
    assert_eq!(rx.pc(), 0x0024);

    let mut tx = Cpu::new();
    arm(&mut tx, 0, UMSEL, (1 << 1) | (1 << 4));
    tx.poke(UCSR0B, TXEN | RXEN | TXCIE);
    tx.set_sreg(0x80);
    let retire = tx.cycles() + 3;
    let mut words = transfer(0xA5);
    words.resize(0x28, 0x0000);
    words.push(spin());
    tx.load_words(&words);
    let end = expect_frame(retire, 0, false, false, false, 0xA5).end;
    tx.run_until(end + 16);
    assert_eq!(tx.pc(), 0x0028);
    assert_eq!(tx.peek(UCSR0A) & TXC, 0);

    let mut udre = Cpu::new();
    arm(&mut udre, 0, UMSEL, (1 << 1) | (1 << 4));
    let base = udre.cycles();
    let mut words = vec![ldi(16, 0x11)];
    words.extend(sts(16, UDR0));
    words.push(ldi(17, 0x22));
    words.extend(sts(17, UDR0));
    words.push(spin());
    words.resize(0x26, 0x0000);
    words.push(spin());
    udre.load_words(&words);
    let first = base + 3;
    udre.run_until(first + 3);
    udre.poke(UCSR0B, TXEN | RXEN | UDRIE);
    udre.set_sreg(0x80);
    let end1 = expect_frame(first, 0, false, false, false, 0x11).end;
    udre.run_until(end1 + 16);
    assert_eq!(udre.pc(), 0x0026);
}
