//! TWI master on the ATmega328P, DS40002061B §22, at bit level on its pads.
//!
//! `T` is `16 + 2·TWBR·4^TWPS`. Most tests use `TWBR` = 12, so `T` = 40, `S` = 18, `L` = 22
//! and `D` = 2, and a byte is `9·T` = 360 cycles. Times are measured from the `TWCR` poke, which
//! launches the phase on that cycle. A [`Peer`] holds the bus high, answers as a target, and
//! records each bus edge.

use orbe_avr::{Cpu, FuseBytes, I2cNote, Level, Port, ResetKind, test_hooks};

use crate::common;
use crate::common::i2c::{Edge, Line, Peer, SCL, SDA};

const TWBR: u16 = 0x00B8;
const TWSR: u16 = 0x00B9;
const TWAR: u16 = 0x00BA;
const TWDR: u16 = 0x00BB;
const TWCR: u16 = 0x00BC;
const TWAMR: u16 = 0x00BD;
const DDRC: u16 = 0x0027;
const PORTC: u16 = 0x0028;
const MCUCR: u16 = 0x0055;
const SMCR: u16 = 0x0053;
const PRR: u16 = 0x0064;

const TWINT: u8 = 1 << 7;
const TWEA: u8 = 1 << 6;
const TWSTA: u8 = 1 << 5;
const TWSTO: u8 = 1 << 4;
const TWWC: u8 = 1 << 3;
const TWEN: u8 = 1 << 2;
const TWIE: u8 = 1 << 0;
const PRTWI: u8 = 1 << 7;
const PUD: u8 = 1 << 4;

const T: u64 = 40;
const L: u64 = 22;
const BYTE: u64 = 9 * T;

fn bare() -> Cpu {
    let mut cpu = Cpu::new();
    let fuses = FuseBytes::CRYSTAL;
    cpu.set_fuses(fuses.low, fuses.high, fuses.extended);
    cpu
}

fn machine_at(twbr: u8) -> Cpu {
    let mut cpu = bare();
    // One-cycle NOPs so `run_until` can stop on an exact cycle.
    cpu.load_words(&vec![0x0000; 60_000]);
    cpu.poke(TWBR, twbr);
    cpu
}

fn machine() -> Cpu {
    machine_at(12)
}

fn poke(cpu: &mut Cpu, addr: u16, value: u8) {
    cpu.poke(addr, value);
}

fn peek(cpu: &Cpu, addr: u16) -> u8 {
    cpu.peek(addr)
}

fn go(cpu: &mut Cpu, extra: u8) -> u64 {
    let at = cpu.cycles();
    poke(cpu, TWCR, TWEN | TWINT | extra);
    at
}

fn pull(cpu: &mut Cpu) -> Vec<(u64, I2cNote)> {
    std::iter::from_fn(|| cpu.take_i2c())
        .map(|stamp| (stamp.at, stamp.note))
        .collect()
}

fn start_at(cpu: &mut Cpu) -> u64 {
    go(cpu, TWSTA | TWEA)
}

fn status(cpu: &Cpu) -> u8 {
    peek(cpu, TWSR) & 0xF8
}

fn logged(cpu: &Cpu, addr: u16) -> usize {
    cpu.io_writes()
        .iter()
        .filter(|write| write.addr == addr)
        .count()
}

fn vector_image() -> Vec<u16> {
    let mut words = vec![0xCFFF];
    words.resize(0x30, 0x0000);
    words.push(0xCFFF);
    words
}

/// START, then SLA+R/W for `addr`, each run to its end. Returns the address phase's end.
fn address(cpu: &mut Cpu, peer: &mut Peer, addr: u8, read: bool, period: u64) -> u64 {
    let at = start_at(cpu);
    peer.run(cpu, at + period);
    assert_eq!(status(cpu), 0x08);
    poke(cpu, TWDR, (addr << 1) | u8::from(read));
    let sla = go(cpu, TWEA);
    peer.run(cpu, sla + 9 * period);
    sla + 9 * period
}

#[test]
fn reset_restores_registers_and_drops_a_phase() {
    let fresh = bare();
    assert_eq!(peek(&fresh, TWBR), 0x00);
    assert_eq!(peek(&fresh, TWSR), 0xF8);
    assert_eq!(peek(&fresh, TWAR), 0xFE);
    assert_eq!(peek(&fresh, TWDR), 0xFF);
    assert_eq!(peek(&fresh, TWCR), 0x00);
    assert_eq!(peek(&fresh, TWAMR), 0x00);

    for kind in [
        ResetKind::External,
        ResetKind::Watchdog,
        ResetKind::BrownOut,
        ResetKind::PowerOn,
    ] {
        let mut cpu = machine();
        let mut peer = Peer::at(&mut cpu, 8);
        let at = start_at(&mut cpu);
        peer.run(&mut cpu, at + T / 2);
        test_hooks::reset_cpu(&mut cpu, kind);
        assert_eq!(peek(&cpu, TWBR), 0x00, "{kind:?}");
        assert_eq!(peek(&cpu, TWSR), 0xF8, "{kind:?}");
        assert_eq!(peek(&cpu, TWAR), 0xFE, "{kind:?}");
        assert_eq!(peek(&cpu, TWDR), 0xFF, "{kind:?}");
        assert_eq!(peek(&cpu, TWCR), 0x00, "{kind:?}");
        assert_eq!(peek(&cpu, TWAMR), 0x00, "{kind:?}");
        assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
        let later = cpu.cycles() + 4_000;
        cpu.run_until(later);
        assert!(pull(&mut cpu).is_empty(), "{kind:?} emitted a phase");
    }
}

#[test]
fn reserved_bits_read_zero_and_twsto_stays_until_stop() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    poke(&mut cpu, TWCR, TWEN | 0x02);
    assert_eq!(peek(&cpu, TWCR) & 0x02, 0);
    poke(&mut cpu, TWSR, 0x07);
    assert_eq!(peek(&cpu, TWSR), 0xFB);
    poke(&mut cpu, TWSR, 0x00);
    poke(&mut cpu, TWAMR, 0xFF);
    assert_eq!(peek(&cpu, TWAMR), 0xFE);
    poke(&mut cpu, TWCR, TWEN | TWWC);
    assert_eq!(peek(&cpu, TWCR) & TWWC, 0);

    address(&mut cpu, &mut peer, 8, false, T);
    let stop = go(&mut cpu, TWSTO);
    assert_ne!(peek(&cpu, TWCR) & TWSTO, 0);
    peer.run(&mut cpu, stop + T - 1);
    assert_ne!(peek(&cpu, TWCR) & TWSTO, 0);
    peer.run(&mut cpu, stop + T);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);

    let mut idle = machine();
    let mut peer = Peer::new(&mut idle);
    poke(&mut idle, TWCR, TWEN | TWSTO | TWINT);
    assert_eq!(peek(&idle, TWCR) & TWSTO, 0);
    let later = idle.cycles() + T;
    peer.run(&mut idle, later);
    assert!(pull(&mut idle).is_empty());
}

#[test]
fn writing_zero_leaves_twint_and_writing_one_clears_it() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    poke(&mut cpu, TWCR, TWEN | TWEA);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    let later = cpu.cycles() + BYTE;
    peer.run(&mut cpu, later);
    assert_eq!(pull(&mut cpu).len(), 1);
    assert!(
        Peer::twi_pulls(&cpu, SCL),
        "SCL stays low while TWINT is set"
    );
    go(&mut cpu, TWEA);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
}

#[test]
fn vector_25_is_level_and_entry_leaves_twint() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    cpu.load_words(&vector_image());
    cpu.set_sreg(0x80);
    let at = go(&mut cpu, TWSTA | TWIE | TWEA);
    peer.run(&mut cpu, at + T + 4);
    assert_eq!(cpu.pc(), 0x0030);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    assert_eq!(status(&cpu), 0x08);
}

#[test]
fn one_twcr_write_sets_twie_and_clears_twint() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    let _ = pull(&mut cpu);
    poke(&mut cpu, TWDR, 8 << 1);
    let mut words = vec![common::ldi(16, u16::from(TWINT | TWEN | TWIE | TWEA))];
    words.extend(common::sts(16, TWCR));
    words.push(0xCFFF);
    words.resize(0x30, 0x0000);
    words.push(0xCFFF);
    cpu.load_words(&words);
    cpu.set_sreg(0x80);
    let before = cpu.cycles();
    peer.run(&mut cpu, before + 1 + 2 + 2);
    assert_eq!(cpu.pc(), 3, "the write took the TWI vector");
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    assert_ne!(peek(&cpu, TWCR) & TWIE, 0);
}

#[test]
fn twwc_retry_keeps_the_first_successful_byte() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    poke(&mut cpu, TWDR, 0x99);
    assert_ne!(peek(&cpu, TWCR) & TWWC, 0);
    assert_eq!(peek(&cpu, TWDR), 8 << 1);
    peer.run(&mut cpu, sla + BYTE);
    poke(&mut cpu, TWDR, 0x78);
    assert_eq!(peek(&cpu, TWCR) & TWWC, 0);
    assert_eq!(peek(&cpu, TWDR), 0x78);
    let data = go(&mut cpu, TWEA);
    peer.run(&mut cpu, data + BYTE);
    let phases = pull(&mut cpu);
    assert_eq!(
        phases.last(),
        Some(&(
            data + BYTE,
            I2cNote::Data {
                byte: 0x78,
                write: true,
                ack: true,
            }
        ))
    );
    assert_eq!(peer.received, vec![0x78]);
}

#[test]
fn twsr_tracks_twint_in_the_same_cycle() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T - 1);
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    peer.run(&mut cpu, at + T);
    assert_eq!(status(&cpu), 0x08);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
}

#[test]
fn twen_clear_aborts_without_an_event_and_the_next_start_is_08() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + 10);
    poke(&mut cpu, TWCR, 0);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    peer.run(&mut cpu, sla + BYTE + T);
    let phases = pull(&mut cpu);
    assert!(
        phases
            .iter()
            .all(|(_, phase)| !matches!(phase, I2cNote::Address { .. })),
        "{phases:?}"
    );
    assert_eq!(peek(&cpu, TWCR), 0);
    let again = start_at(&mut cpu);
    peer.run(&mut cpu, again + T);
    assert_eq!(status(&cpu), 0x08);
}

#[test]
fn prtwi_freezes_and_idle_still_runs() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + 10);
    let frozen = cpu.cycles();
    poke(&mut cpu, PRR, PRTWI);
    poke(&mut cpu, TWBR, 1);
    assert_eq!(peek(&cpu, TWBR), 12, "a frozen write is discarded");
    peer.run(&mut cpu, frozen + 500);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    let resume = cpu.cycles();
    poke(&mut cpu, PRR, 0);
    let done = at + T + (resume - frozen);
    peer.run(&mut cpu, done - 1);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    peer.run(&mut cpu, done);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);

    let mut down = machine();
    let mut peer = Peer::new(&mut down);
    down.load_words(&[0x9588, 0xCFFF]);
    down.poke(SMCR, 0x05);
    let at = start_at(&mut down);
    peer.run(&mut down, at + 500);
    assert!(down.is_sleeping());
    assert_eq!(peek(&down, TWSR), 0xF8);

    let mut idle = machine();
    let mut peer = Peer::new(&mut idle);
    idle.load_words(&[0x9588, 0xCFFF]);
    idle.poke(SMCR, 0x01);
    let at = start_at(&mut idle);
    peer.run(&mut idle, at + T + 4);
    assert_eq!(status(&idle), 0x08);
}

#[test]
fn twar_and_twamr_are_stored_and_do_not_wake() {
    let mut cpu = machine();
    cpu.load_words(&[0x9588, 0xCFFF]);
    cpu.poke(SMCR, 0x01);
    cpu.run_until(2);
    assert!(cpu.is_sleeping());
    poke(&mut cpu, TWAR, 0x10);
    poke(&mut cpu, TWAMR, 0x0F);
    assert_eq!(peek(&cpu, TWAR), 0x10);
    assert_eq!(peek(&cpu, TWAMR), 0x0E);
    assert_eq!(logged(&cpu, TWAR), 1);
    assert_eq!(logged(&cpu, TWAMR), 1);
    poke(&mut cpu, TWAR, 0x20);
    poke(&mut cpu, TWAMR, 0x22);
    assert_eq!(logged(&cpu, TWAR), 1);
    assert_eq!(logged(&cpu, TWAMR), 1);
    assert!(cpu.is_sleeping());
    let later = cpu.cycles() + 100;
    cpu.run_until(later);
    assert!(cpu.is_sleeping());
}

/// The reset `TWAR` is address 0x7F with `TWGCE` clear (§22.9.5). This TWI has no slave mode,
/// so nothing answers 0x7F.
#[test]
fn the_reset_own_address_is_never_answered() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    assert_eq!(peek(&cpu, TWAR), 0xFE);
    address(&mut cpu, &mut peer, 0x7F, false, T);
    assert_eq!(status(&cpu), 0x20);
}

#[test]
fn start_repeated_start_and_off_table_sources() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend(b"A");
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    assert_eq!(status(&cpu), 0x08);
    assert_ne!(peek(&cpu, TWCR) & TWSTA, 0);
    let phases = pull(&mut cpu);
    assert_eq!(phases, vec![(at + T, I2cNote::Start { repeated: false })]);

    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + BYTE);
    assert_eq!(status(&cpu), 0x40);
    // 0x40 has no START row. The write releases this master and is logged once.
    poke(&mut cpu, TWCR, TWINT | TWSTA | TWEN | TWEA);
    assert_eq!(logged(&cpu, TWCR), 1);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    poke(&mut cpu, TWCR, TWINT | TWSTA | TWEN | TWEA);
    assert_eq!(logged(&cpu, TWCR), 1, "the missing-row note is once");
    let again = start_at(&mut cpu);
    peer.run(&mut cpu, again + T);
    assert_eq!(status(&cpu), 0x08);
    let phases = pull(&mut cpu);
    assert!(
        phases
            .iter()
            .all(|(_, phase)| phase != &I2cNote::Start { repeated: true })
    );
}

#[test]
fn address_status_follows_the_target_ack() {
    let cases = [(false, true, 0x18u8), (true, true, 0x40)];
    for (read, present, want) in cases {
        let mut cpu = machine();
        let mut peer = Peer::new(&mut cpu);
        if present {
            peer.address = Some(8);
        }
        address(&mut cpu, &mut peer, 8, read, T);
        assert_eq!(status(&cpu), want, "read={read}");
    }
    let mut missing = machine();
    // §22.3.3. Only the addressed device ACKs. Address 9 must not ACK 8.
    let mut peer = Peer::at(&mut missing, 9);
    address(&mut missing, &mut peer, 8, false, T);
    assert_eq!(status(&missing), 0x20); // §22.7.1. SLA+W NACK.
    let stop = go(&mut missing, TWSTO);
    peer.run(&mut missing, stop + T);
    address(&mut missing, &mut peer, 8, true, T);
    assert_eq!(status(&missing), 0x48); // §22.7.2. SLA+R NACK.
}

#[test]
fn transmit_and_data_after_a_sla_nack() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.nack_at = Some(0);
    address(&mut cpu, &mut peer, 8, false, T);
    assert_eq!(status(&cpu), 0x18);
    poke(&mut cpu, TWDR, 0x11);
    let data = go(&mut cpu, TWEA);
    peer.run(&mut cpu, data + BYTE);
    assert_eq!(status(&cpu), 0x30);
    let phases = pull(&mut cpu);
    assert_eq!(
        phases.last().map(|(_, phase)| *phase),
        Some(I2cNote::Data {
            byte: 0x11,
            write: true,
            ack: false,
        })
    );

    let mut late = machine();
    let mut peer = Peer::new(&mut late);
    address(&mut late, &mut peer, 8, false, T);
    assert_eq!(status(&late), 0x20);
    peer.address = Some(8);
    poke(&mut late, TWDR, 0x22);
    let data = go(&mut late, TWEA);
    peer.run(&mut late, data + BYTE);
    assert_eq!(
        status(&late),
        0x30,
        "a target answers only after its own address"
    );
    assert!(peer.received.is_empty());
}

#[test]
fn receive_drives_the_ninth_bit_from_twea() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend(b"ab");
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x40);
    let data = go(&mut cpu, 0);
    peer.run(&mut cpu, data + BYTE / 2);
    poke(&mut cpu, TWCR, TWEN | TWEA);
    peer.run(&mut cpu, data + BYTE);
    assert_eq!(status(&cpu), 0x50);
    assert_eq!(peek(&cpu, TWDR), b'a');
    let nack = go(&mut cpu, 0);
    peer.run(&mut cpu, nack + BYTE);
    assert_eq!(status(&cpu), 0x58);
    assert_eq!(peek(&cpu, TWDR), b'b');
}

/// §22.7.2. The target drives the eight data bits, and the master only the acknowledge.
/// 0x00 then 0xFF reads back exactly, and the TWI never pulls SDA low inside the data bits.
#[test]
fn a_master_read_releases_sda_for_the_data_bits() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend([0x00, 0xFF]);
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x40);
    for (byte, extra, want) in [(0x00, TWEA, 0x50), (0xFF, 0, 0x58)] {
        let data = go(&mut cpu, extra);
        let mut pulled = Vec::new();
        while cpu.cycles() < data + BYTE {
            let next = cpu.cycles() + 1;
            peer.run(&mut cpu, next);
            if Peer::twi_pulls(&cpu, SDA) {
                pulled.push(next - data);
            }
        }
        // Up to `D` the last acknowledge still holds SDA. From `8·T + D` the ninth bit does.
        let ack_bit = 8 * T + 2..=BYTE;
        assert!(
            pulled.iter().all(|at| *at <= 2 || ack_bit.contains(at)),
            "SDA pulled inside the data bits at {pulled:?}"
        );
        let acked = pulled.iter().any(|at| ack_bit.contains(at));
        assert_eq!(acked, extra == TWEA, "the ninth bit follows TWEA");
        assert_eq!(peek(&cpu, TWDR), byte);
        assert_eq!(status(&cpu), want);
    }
}

#[test]
fn stop_clears_twsto_and_leaves_twint_clear() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    address(&mut cpu, &mut peer, 8, false, T);
    let stop = go(&mut cpu, TWSTO);
    peer.run(&mut cpu, stop + T);
    assert_eq!(peek(&cpu, TWCR) & (TWSTO | TWINT), 0);
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    let phases = pull(&mut cpu);
    assert_eq!(phases.last().copied(), Some((stop + T, I2cNote::Stop)));
}

#[test]
fn stop_then_start_lands_on_08() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    address(&mut cpu, &mut peer, 8, false, T);
    let both = go(&mut cpu, TWSTO | TWSTA);
    peer.run(&mut cpu, both + T);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    assert_ne!(peek(&cpu, TWCR) & TWSTA, 0);
    peer.run(&mut cpu, both + 2 * T);
    assert_eq!(status(&cpu), 0x08);
    let phases = pull(&mut cpu);
    assert_eq!(
        &phases[phases.len() - 2..],
        &[
            (both + T, I2cNote::Stop),
            (both + 2 * T, I2cNote::Start { repeated: false }),
        ]
    );
}

#[test]
fn an_invalid_action_ends_the_frame_with_no_event() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x48);
    let _ = pull(&mut cpu);
    go(&mut cpu, 0);
    let later = cpu.cycles() + BYTE;
    peer.run(&mut cpu, later);
    assert!(pull(&mut cpu).is_empty());
    assert_eq!(logged(&cpu, TWCR), 1);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);
    assert!(
        !Peer::twi_pulls(&cpu, SCL),
        "an invalid write releases the bus"
    );
    let again = start_at(&mut cpu);
    peer.run(&mut cpu, again + T);
    assert_eq!(status(&cpu), 0x08);
}

#[test]
fn a_busy_twcr_write_stores_bits_and_does_not_start() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + 10);
    poke(&mut cpu, TWCR, TWEN | TWSTA | TWEA);
    assert_ne!(peek(&cpu, TWCR) & TWSTA, 0);
    peer.run(&mut cpu, sla + BYTE);
    assert_eq!(status(&cpu), 0x18);
    let phases = pull(&mut cpu);
    assert!(
        phases
            .iter()
            .any(|(_, phase)| matches!(phase, I2cNote::Address { .. }))
    );
    assert_eq!(
        phases
            .iter()
            .filter(|(_, phase)| matches!(phase, I2cNote::Start { .. }))
            .count(),
        1
    );
}

#[test]
fn an_out_of_spec_period_is_logged_and_still_used() {
    let mut cpu = bare();
    cpu.load_words(&[0x0000; 64]);
    let mut peer = Peer::new(&mut cpu);
    // T = 16 + 2·2 = 20. 16 MHz / 20 is 800 kHz, above the 400 kHz limit.
    poke(&mut cpu, TWBR, 2);
    poke(&mut cpu, TWBR, 2);
    assert_eq!(
        logged(&cpu, TWBR),
        0,
        "the check is not on the register write"
    );
    let at = start_at(&mut cpu);
    assert_eq!(logged(&cpu, TWBR), 1);
    peer.run(&mut cpu, at + 19);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    peer.run(&mut cpu, at + 20);
    assert_eq!(status(&cpu), 0x08);
    poke(&mut cpu, TWDR, 0x10);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + 9 * 20);
    assert_eq!(status(&cpu), 0x20);
    let again = go(&mut cpu, TWSTA | TWEA);
    assert_eq!(
        logged(&cpu, TWBR),
        1,
        "one note for the clock, not one per phase"
    );
    // A repeated START is `T + L`, and `L` is 11 at `T` = 20.
    peer.run(&mut cpu, again + 31);
    assert_eq!(status(&cpu), 0x10);
}

#[test]
fn phases_are_exact() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend(b"Z");
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + BYTE);
    let data = go(&mut cpu, 0);
    peer.run(&mut cpu, data + BYTE);
    let stop = go(&mut cpu, TWSTO);
    peer.run(&mut cpu, stop + T);
    let stamps: Vec<_> = std::iter::from_fn(|| cpu.take_i2c()).collect();
    let notes: Vec<_> = stamps.iter().map(|stamp| (stamp.at, stamp.note)).collect();
    assert_eq!(
        notes,
        vec![
            (at + T, I2cNote::Start { repeated: false }),
            (
                sla + BYTE,
                I2cNote::Address {
                    addr: 8,
                    read: true,
                    ack: true,
                }
            ),
            (
                data + BYTE,
                I2cNote::Data {
                    byte: b'Z',
                    write: false,
                    ack: false,
                }
            ),
            (stop + T, I2cNote::Stop),
        ]
    );
    // Each phase starts at its first bus change after the cycle the last one ended: SDA for
    // the START, SCL for SLA+R (0x11 starts with a 0), SCL for the read (the target drives its
    // first bit on the address phase's last cycle), and the SDA fall that opens the STOP.
    let starts: Vec<_> = stamps.iter().map(|stamp| stamp.start).collect();
    assert_eq!(starts, vec![at + L, sla + L, sla + BYTE + L, stop + 2]);
}

#[test]
fn twint_waits_without_a_timeout() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    assert_eq!(
        pull(&mut cpu),
        vec![(at + T, I2cNote::Start { repeated: false })]
    );
    cpu.run_until(at + T + 20_000);
    assert!(pull(&mut cpu).is_empty());
    assert_eq!(status(&cpu), 0x08);
    assert!(Peer::twi_pulls(&cpu, SCL));
}

#[test]
fn a_target_nacks_its_chosen_data_byte() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.nack_at = Some(1);
    address(&mut cpu, &mut peer, 8, false, T);
    poke(&mut cpu, TWDR, 0x01);
    let first = go(&mut cpu, TWEA);
    peer.run(&mut cpu, first + BYTE);
    assert_eq!(status(&cpu), 0x28);
    poke(&mut cpu, TWDR, 0x02);
    let second = go(&mut cpu, TWEA);
    peer.run(&mut cpu, second + BYTE);
    assert_eq!(status(&cpu), 0x30);
    assert_eq!(peer.received, vec![0x01, 0x02]);
}

/// With nothing driving SDA, the pull-up reads back as 1s.
#[test]
fn a_read_past_the_targets_bytes_is_ff() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend(b"a");
    address(&mut cpu, &mut peer, 8, true, T);
    for want in [b'a', 0xFF] {
        let data = go(&mut cpu, TWEA);
        peer.run(&mut cpu, data + BYTE);
        assert_eq!(peek(&cpu, TWDR), want);
    }
}

#[test]
fn a_repeated_start_reads_after_a_write() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend(b"Z");
    address(&mut cpu, &mut peer, 8, false, T);
    poke(&mut cpu, TWDR, 0x11);
    let data = go(&mut cpu, TWEA);
    peer.run(&mut cpu, data + BYTE);
    let again = go(&mut cpu, TWSTA | TWEA);
    peer.run(&mut cpu, again + T + L);
    assert_eq!(status(&cpu), 0x10);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + BYTE);
    assert_eq!(status(&cpu), 0x40);
    let data = go(&mut cpu, 0);
    peer.run(&mut cpu, data + BYTE);
    assert_eq!(peek(&cpu, TWDR), b'Z');
    assert_eq!(status(&cpu), 0x58);
    assert_eq!(peer.received, vec![0x11]);
}

#[test]
fn twi_pads_ignore_ddr_and_read_the_bus() {
    let mut cpu = machine();
    poke(&mut cpu, DDRC, 0xFF);
    poke(&mut cpu, PORTC, 0);
    poke(&mut cpu, TWCR, TWEN);
    assert_eq!(cpu.pin_levels().get(SDA), Some(Level::Released));
    assert_eq!(cpu.pin_levels().get(SCL), Some(Level::Released));
    poke(&mut cpu, PORTC, (1 << 4) | (1 << 5));
    assert_eq!(cpu.pin_levels().get(SDA), Some(Level::High));
    assert_eq!(cpu.pin_levels().get(SCL), Some(Level::High));
    poke(&mut cpu, MCUCR, PUD);
    assert_eq!(cpu.pin_levels().get(SDA), Some(Level::Released));
    poke(&mut cpu, MCUCR, 0);
    assert_eq!(Port::C.pin_id(4), SDA);
    cpu.drive_pin(SDA, Level::Low);
    assert_eq!(cpu.pin_levels().get(SDA), Some(Level::Low));
    assert_eq!(logged(&cpu, TWCR), 0, "the TWI reads a host drive");
    cpu.drive_pin(SDA, Level::Released);
    poke(&mut cpu, TWCR, 0);
    poke(&mut cpu, DDRC, 1 << 4);
    poke(&mut cpu, PORTC, 0);
    assert_eq!(cpu.pin_levels().get(SDA), Some(Level::Low));
}

/// `Wire.begin` sets `PORTC4` and `PORTC5`, so the TWI runs on its own pull-ups with no
/// external one.
#[test]
fn the_internal_pull_ups_carry_a_transfer() {
    let mut cpu = machine();
    poke(&mut cpu, PORTC, (1 << 4) | (1 << 5));
    let at = start_at(&mut cpu);
    cpu.run_until(at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    cpu.run_until(sla + BYTE);
    assert_eq!(status(&cpu), 0x20);
}

const WDTCSR: u16 = 0x0060;
const MCUSR: u16 = 0x0054;
const WDCE: u8 = 1 << 4;
const WDE: u8 = 1 << 3;
const WDRF: u8 = 1 << 3;
const EXTRF: u8 = 1 << 1;

fn arm_wdt(cpu: &mut Cpu) {
    poke(cpu, WDTCSR, WDCE | WDE);
    poke(cpu, WDTCSR, WDE);
}

fn after_startup(cpu: &mut Cpu, flag: u8) {
    let limit = cpu.cycles() + 2_000_000;
    while cpu.cycles() < limit && peek(cpu, MCUSR) & flag == 0 {
        cpu.run_until((cpu.cycles() + 50_000).min(limit));
    }
    assert_ne!(
        peek(cpu, MCUSR) & flag,
        0,
        "reset flag {flag:#04x} did not set"
    );
    let born = cpu.cycles();
    let startup = born + 1_200_000;
    while cpu.cycles() < startup && cpu.pc() == 0 {
        cpu.run_until((cpu.cycles() + 50_000).min(startup));
    }
    cpu.run_until(cpu.cycles() + 4);
    assert_ne!(cpu.pc(), 0, "still in the start-up time-out");
}

/// The target sits outside the chip. A reset releases the pads mid-transfer, and the next
/// START opens a fresh transfer with it.
#[test]
fn a_target_answers_after_watchdog_and_external_reset() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend([0xA1, 0xA2]);
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x40);
    let data = go(&mut cpu, TWEA);
    peer.run(&mut cpu, data + BYTE);
    assert_eq!(peek(&cpu, TWDR), 0xA1);
    arm_wdt(&mut cpu);
    after_startup(&mut cpu, WDRF);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    let now = cpu.cycles();
    peer.run(&mut cpu, now + 1);
    poke(&mut cpu, TWBR, 12);
    address(&mut cpu, &mut peer, 8, false, T);
    assert_eq!(status(&cpu), 0x18);

    let mut external = machine();
    let mut peer = Peer::at(&mut external, 8);
    let at = start_at(&mut external);
    peer.run(&mut external, at + T / 2);
    test_hooks::reset_cpu(&mut external, ResetKind::External);
    assert!(
        pull(&mut external).is_empty(),
        "the open start emits nothing"
    );
    after_startup(&mut external, EXTRF);
    poke(&mut external, TWBR, 12);
    address(&mut external, &mut peer, 8, false, T);
    assert_eq!(status(&external), 0x18);
}

#[test]
fn idle_twsto_without_twint_clears() {
    let mut cpu = machine();
    poke(&mut cpu, TWCR, TWEN | TWSTO);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    assert!(pull(&mut cpu).is_empty());
}

#[test]
fn invalid_stop_at_0x08_clears_twsto() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    assert_eq!(status(&cpu), 0x08);
    poke(&mut cpu, TWCR, TWINT | TWSTO | TWEN);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);
    assert_eq!(logged(&cpu, TWCR), 1);
}

#[test]
fn a_twint_write_during_a_phase_does_not_start_another() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + 10);
    poke(&mut cpu, TWCR, TWINT | TWEN | TWSTA | TWEA);
    peer.run(&mut cpu, sla + BYTE);
    assert_eq!(status(&cpu), 0x18);
    let phases = pull(&mut cpu);
    assert_eq!(
        phases
            .iter()
            .filter(|(_, phase)| matches!(phase, I2cNote::Start { .. }))
            .count(),
        1
    );
}

#[test]
fn twps_scales_the_period() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    poke(&mut cpu, TWSR, 1);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + 111);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    peer.run(&mut cpu, at + 112);
    assert_eq!(status(&cpu), 0x08);
}

/// A `TWSTA` from 0x08 is a row of no table. It is logged once, both lines are
/// released, and the bus is free because this master owned it. The next START
/// is not a repeated START.
#[test]
fn a_start_from_0x08_releases_and_is_logged_once() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    assert_eq!(status(&cpu), 0x08);
    assert!(Peer::twi_pulls(&cpu, SCL));
    assert_eq!(logged(&cpu, TWCR), 0);
    poke(&mut cpu, TWCR, TWINT | TWSTA | TWEN);
    assert_eq!(logged(&cpu, TWCR), 1);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    poke(&mut cpu, TWCR, TWINT | TWSTA | TWEN);
    assert_eq!(logged(&cpu, TWCR), 1);
    let again = start_at(&mut cpu);
    peer.run(&mut cpu, again + T);
    assert_eq!(status(&cpu), 0x08);
    let phases = pull(&mut cpu);
    assert_eq!(
        phases
            .iter()
            .filter(|(_, phase)| matches!(phase, I2cNote::Start { repeated: true }))
            .count(),
        0
    );
}

/// An edge after the last phase, then a missing-row write. The next phase
/// starts at its own first edge.
#[test]
fn an_invalid_write_drops_the_phase_start() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend([0x00]);
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x40);
    let _ = pull(&mut cpu);
    let stray = cpu.cycles() + 4;
    peer.pull_at(stray, Line::Sda, true);
    peer.pull_at(stray + 2, Line::Sda, false);
    peer.run(&mut cpu, stray + 2);
    poke(&mut cpu, TWCR, TWINT | TWSTO | TWEN);
    assert_eq!(logged(&cpu, TWCR), 1);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    let again = start_at(&mut cpu);
    peer.run(&mut cpu, again + T);
    let stamp = cpu.take_i2c().unwrap();
    assert_eq!(stamp.note, I2cNote::Start { repeated: false });
    assert_eq!(stamp.start, again + L);
    assert_ne!(stamp.start, stray);
}

#[test]
fn a_cpu_store_while_prtwi_is_set_is_ignored() {
    let mut cpu = machine();
    // ldi r16, TWINT|TWSTA|TWEN; sts TWCR, r16; ldi r16, 0x5A; sts TWDR, r16
    cpu.load_words(&[0xEA04, 0x9300, 0x00BC, 0xE50A, 0x9300, 0x00BB]);
    poke(&mut cpu, PRR, PRTWI);
    cpu.run_until(6);
    assert_eq!(peek(&cpu, TWCR), 0);
    assert_eq!(peek(&cpu, TWDR), 0xFF);
    assert_eq!(status(&cpu), 0xF8);
}

#[test]
fn a_pending_twint_does_not_wake_or_interrupt_while_frozen() {
    let mut words = vec![0x0000u16; 0x31];
    words[40] = 0x9478;
    words[41] = 0xCFFF;
    words[0x30] = 0xCFFF;
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    cpu.load_words(&words);
    poke(&mut cpu, TWBR, 12);
    let at = go(&mut cpu, TWSTA | TWIE);
    peer.run(&mut cpu, at + T);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    poke(&mut cpu, PRR, PRTWI);
    peer.run(&mut cpu, at + T + 30);
    assert_eq!(cpu.pc(), 41);

    let mut down = vec![0x0000u16; 48];
    down[40] = 0x9588;
    let mut asleep = machine();
    let mut peer = Peer::new(&mut asleep);
    asleep.load_words(&down);
    poke(&mut asleep, TWBR, 12);
    poke(&mut asleep, SMCR, 0x05);
    let at = go(&mut asleep, TWSTA | TWIE);
    peer.run(&mut asleep, at + T + 2);
    assert!(asleep.is_sleeping());
    peer.run(&mut asleep, at + T + 100);
    assert!(asleep.is_sleeping());
}

/// §22.4. A target that holds SCL low stretches the bit: the TWI counts its high from the
/// cycle SCL reads high. The byte lasts `9·T` plus the hold past the TWI's release.
#[test]
fn a_held_scl_stretches_the_byte() {
    let hold = 500;
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    // Fall 0 is the START's. Fall 3 ends bit 2 of SLA+W.
    peer.stretch = Some((3, hold));
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + BYTE + hold);
    let fall = sla + 3 * T;
    let past_release = fall + hold - (fall + L);
    assert_eq!(
        pull(&mut cpu).last(),
        Some(&(
            sla + BYTE + past_release,
            I2cNote::Address {
                addr: 8,
                read: false,
                ack: true,
            }
        ))
    );
    let rises = peer.times(Line::Scl, true);
    let falls = peer.times(Line::Scl, false);
    let rise = rises.iter().find(|&&at| at > fall).copied().unwrap();
    assert_eq!(rise, fall + hold, "SCL rises when the target lets go");
    let next = falls.iter().find(|&&at| at > rise).copied().unwrap();
    assert_eq!(next - rise, 18, "the high is S from the rise the TWI read");
}

/// §22.4 and Figure 22-21. A 1 the TWI sends that reads back 0 loses arbitration at that SCL
/// rise: it releases both lines and reads 0x38. Wire's release writes `TWINT` with neither
/// `TWSTA` nor `TWSTO`, and a START asked for after that waits for the bus to free.
#[test]
fn a_lost_one_reads_0x38_and_the_next_start_waits_for_a_stop() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    // Another master holds SDA low from the START's SCL fall, fall 0. 0x51 is 1010001, so
    // bit 0 is a 1 this TWI releases and reads back low.
    peer.steal = Some((0, 2_000));
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 0x51 << 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + T);
    let rise = sla + L;
    let phases = pull(&mut cpu);
    assert_eq!(phases.last(), Some(&(rise, I2cNote::ArbitrationLost)));
    assert_eq!(status(&cpu), 0x38);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    poke(&mut cpu, TWCR, TWINT | TWEN | TWEA | TWIE);
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    assert_eq!(logged(&cpu, TWCR), 0, "the release is a Table 22-2 row");
    start_at(&mut cpu);
    let free = peer.release_at();
    peer.run(&mut cpu, free - 1);
    assert_eq!(
        peek(&cpu, TWCR) & TWINT,
        0,
        "the bus is busy until its STOP"
    );
    peer.run(&mut cpu, free + T);
    assert_eq!(status(&cpu), 0x08);
    let stop = peer
        .times(Line::Sda, true)
        .into_iter()
        .find(|&at| at >= free)
        .unwrap();
    let start = peer
        .times(Line::Sda, false)
        .into_iter()
        .find(|&at| at > stop)
        .unwrap();
    assert_eq!(start - stop, L, "tBUF from the STOP the TWI read");
}

/// Table 22-3, 0x38, "Arbitration lost in SLA+R or NOT ACK bit". The master NACKs a
/// received byte, so it sends 1 on that bit. Another master holds SDA low, an ACK. This
/// one reads 0 at the SCL rise, releases both lines and reads 0x38.
#[test]
fn a_nack_on_a_read_that_reads_low_loses_arbitration() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend([0xA5]);
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x40);
    // Fall 0 is the START's. Fall 17 ends the eighth data bit and opens the ACK bit.
    peer.steal = Some((17, 100));
    let data = go(&mut cpu, 0);
    peer.run(&mut cpu, data + BYTE);
    let rise = data + 8 * T + L;
    assert_eq!(
        pull(&mut cpu).last(),
        Some(&(rise, I2cNote::ArbitrationLost))
    );
    assert_eq!(status(&cpu), 0x38);
    assert_eq!(peek(&cpu, TWDR), 0xA5);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    poke(&mut cpu, TWCR, TWINT | TWEN | TWEA | TWIE);
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    assert_eq!(logged(&cpu, TWCR), 0, "the release is a Table 22-3 row");
}

/// A master that drives the read ACK low is sending 0. Another master holding SDA low
/// matches that bit, so this one keeps the bus and reads 0x50.
#[test]
fn an_ack_on_a_read_does_not_lose_when_sda_is_low() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend([0xA5]);
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x40);
    peer.steal = Some((17, 100));
    let data = go(&mut cpu, TWEA);
    peer.run(&mut cpu, data + BYTE);
    assert_eq!(status(&cpu), 0x50);
    assert_eq!(peek(&cpu, TWDR), 0xA5);
    assert!(
        pull(&mut cpu)
            .iter()
            .all(|(_, note)| !matches!(note, I2cNote::ArbitrationLost))
    );
}

/// §22.8. A START inside a byte is a bus error: 0x00, both lines released. Wire answers with
/// `TWSTO`, which clears at once and puts no STOP on the bus (Table 22-6).
#[test]
fn a_start_inside_a_byte_is_a_bus_error() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    // 0x51: bit 0 is a 1. Rise 0 is bit 0's. The peer pulls SDA low 4 cycles into its high.
    poke(&mut cpu, TWDR, 0x51 << 1);
    peer.flip = Some((0, 4));
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + T);
    assert_eq!(
        pull(&mut cpu).last(),
        Some(&(sla + L + 4, I2cNote::BusError))
    );
    assert_eq!(status(&cpu), 0x00);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    let edges = peer.edges.len();
    poke(&mut cpu, TWCR, TWINT | TWSTO | TWEN | TWEA | TWIE);
    assert_eq!(peek(&cpu, TWCR) & (TWSTO | TWINT), 0);
    assert_eq!(logged(&cpu, TWCR), 0);
    peer.run(&mut cpu, sla + 3 * T);
    assert_eq!(peer.edges.len(), edges, "no STOP goes on the bus");
}

/// A STOP inside a byte: the target drives a 0, then lets SDA go while SCL is high.
#[test]
fn a_stop_inside_a_read_byte_is_a_bus_error() {
    let mut cpu = machine();
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend([0x00]);
    address(&mut cpu, &mut peer, 8, true, T);
    assert_eq!(status(&cpu), 0x40);
    // Rises 0 to 8 are SLA+R's. Rise 9 is the first data bit's.
    peer.flip = Some((9, 4));
    let data = go(&mut cpu, TWEA);
    peer.run(&mut cpu, data + T);
    assert_eq!(
        pull(&mut cpu).last(),
        Some(&(data + L + 4, I2cNote::BusError))
    );
    assert_eq!(status(&cpu), 0x00);
}

/// Another master owns the bus. An illegal `TWCR` write from this one releases
/// only its own lines and leaves `busy` set, so its START waits. The other
/// master finishes its byte and its STOP, and this one starts `L` after that.
#[test]
fn an_illegal_write_while_another_master_owns_the_bus_waits() {
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    poke(&mut cpu, TWCR, TWEN);
    peer.drive(&mut cpu, Line::Sda, true);
    peer.drive(&mut cpu, Line::Scl, true);
    poke(&mut cpu, TWCR, TWINT | TWEN);
    assert_eq!(logged(&cpu, TWCR), 1);
    assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    let asked = go(&mut cpu, TWSTA | TWEA);
    peer.run(&mut cpu, asked + L);
    assert!(
        !Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL),
        "the START waits while the other master holds the bus"
    );
    for _ in 0..8 {
        peer.drive(&mut cpu, Line::Scl, false);
        let high = cpu.cycles() + 2;
        peer.run(&mut cpu, high);
        assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
        peer.drive(&mut cpu, Line::Scl, true);
        let low = cpu.cycles() + 2;
        peer.run(&mut cpu, low);
        assert!(!Peer::twi_pulls(&cpu, SDA) && !Peer::twi_pulls(&cpu, SCL));
    }
    peer.drive(&mut cpu, Line::Scl, false);
    let ready = cpu.cycles() + 2;
    peer.run(&mut cpu, ready);
    let stop = cpu.cycles();
    peer.drive(&mut cpu, Line::Sda, false);
    peer.run(&mut cpu, stop + T);
    assert_eq!(status(&cpu), 0x08);
    let ours = peer
        .times(Line::Sda, false)
        .into_iter()
        .find(|&at| at > stop)
        .unwrap();
    assert_eq!(ours, stop + L, "tBUF from the other master's STOP");
}

/// §22.4. A peer pulls SCL low during this master's high. This master pulls
/// SCL low on that cycle, and releases it `L` after the peer's fall.
#[test]
fn a_peer_scl_fall_during_the_high_is_followed() {
    let k = 8;
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    let at = start_at(&mut cpu);
    peer.run(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 0x00);
    let sla = go(&mut cpu, TWEA);
    let fall = sla + L + k;
    peer.pull_at(fall, Line::Scl, true);
    peer.run(&mut cpu, fall - 1);
    assert!(!Peer::twi_pulls(&cpu, SCL));
    peer.run(&mut cpu, fall);
    assert!(
        Peer::twi_pulls(&cpu, SCL),
        "SCL follows on the cycle the peer pulls it"
    );
    peer.run(&mut cpu, fall + L - 1);
    assert!(Peer::twi_pulls(&cpu, SCL));
    peer.run(&mut cpu, fall + L);
    assert!(
        !Peer::twi_pulls(&cpu, SCL),
        "the next release is L after the fall"
    );
}

/// §22.4. A peer sends START while this master waits to send its own. This
/// master's SDA follows on that cycle, SCL falls `S` later, and the START
/// row begins at the peer's fall.
#[test]
fn a_peer_start_while_waiting_is_joined() {
    let k = 8;
    let mut cpu = machine();
    let mut peer = Peer::new(&mut cpu);
    let w = start_at(&mut cpu);
    let fall = w + k;
    peer.pull_at(fall, Line::Sda, true);
    peer.run(&mut cpu, fall - 1);
    assert!(!Peer::twi_pulls(&cpu, SDA));
    peer.run(&mut cpu, fall);
    assert!(
        Peer::twi_pulls(&cpu, SDA),
        "SDA follows on the cycle the peer pulls it"
    );
    assert!(!Peer::twi_pulls(&cpu, SCL));
    let s = T - L;
    peer.run(&mut cpu, fall + s);
    assert!(Peer::twi_pulls(&cpu, SCL));
    let stamp = cpu.take_i2c().unwrap();
    assert_eq!(stamp.note, I2cNote::Start { repeated: false });
    assert_eq!(stamp.start, fall);
    assert_eq!(stamp.at, fall + s);
}

/// Table 29-14's timing rows, in nanoseconds, for the rate's column. tHD;DAT is a maximum and
/// every other row a minimum. The rise and fall rows are not here.
struct Rows {
    hd_sta: f64,
    low: f64,
    high: f64,
    su_sta: f64,
    hd_dat_max: f64,
    su_dat: f64,
    su_sto: f64,
    buf: f64,
}

const SLOW: Rows = Rows {
    hd_sta: 4000.0,
    low: 4700.0,
    high: 4000.0,
    su_sta: 4700.0,
    hd_dat_max: 3450.0,
    su_dat: 250.0,
    su_sto: 4000.0,
    buf: 4700.0,
};

const FAST: Rows = Rows {
    hd_sta: 600.0,
    low: 1300.0,
    high: 600.0,
    su_sta: 600.0,
    hd_dat_max: 900.0,
    su_dat: 100.0,
    su_sto: 600.0,
    buf: 1300.0,
};

/// Every interval of a transfer of every phase kind, against the bit table and the rows.
/// Each SCL low here is one the TWI times, because each `TWCR` write lands on its `TWINT`.
fn bit_table(twbr: u8, period: u64, rows: &Rows) -> Vec<(&'static str, u64)> {
    let (s, l, d) = (period * 45 / 100, period - period * 45 / 100, period / 16);
    let ns = |cycles: u64| cycles as f64 * 62.5;
    let mut cpu = machine_at(twbr);
    let mut peer = Peer::at(&mut cpu, 8);
    peer.reads.extend([0x00, 0xFF]);
    address(&mut cpu, &mut peer, 8, false, period);
    poke(&mut cpu, TWDR, 0x5A);
    let data = go(&mut cpu, TWEA);
    peer.run(&mut cpu, data + 9 * period);
    let again = go(&mut cpu, TWSTA | TWEA);
    peer.run(&mut cpu, again + period + l);
    assert_eq!(status(&cpu), 0x10);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    peer.run(&mut cpu, sla + 9 * period);
    for extra in [TWEA, 0] {
        let read = go(&mut cpu, extra);
        peer.run(&mut cpu, read + 9 * period);
    }
    assert_eq!(status(&cpu), 0x58);
    let both = go(&mut cpu, TWSTO | TWSTA);
    peer.run(&mut cpu, both + 2 * period);
    assert_eq!(status(&cpu), 0x08);

    let mut seen = Vec::new();
    let mut check = |name: &'static str, cycles: u64, want: u64, floor: f64| {
        assert_eq!(cycles, want, "{name} at TWBR {twbr}");
        assert!(
            ns(cycles) >= floor,
            "{name} {} ns under {floor} ns",
            ns(cycles)
        );
        seen.push((name, cycles));
    };
    let edges: &[Edge] = &peer.edges;
    let mut scl = true;
    let (mut rise, mut fall, mut sda_fall, mut stop_at) = (None, None, None, None::<u64>);
    for edge in edges {
        match (edge.line, edge.high) {
            (Line::Scl, true) => {
                if let Some(fall) = fall {
                    check("tLOW", edge.at - fall, l, rows.low);
                }
                rise = Some(edge.at);
            }
            (Line::Scl, false) => {
                let r = rise.unwrap_or(0);
                match sda_fall {
                    Some(at) if at > r || (rise.is_none() && at > 0) => {
                        check("tHD;STA", edge.at - at, s, rows.hd_sta);
                    }
                    _ => check("tHIGH", edge.at - r, s, rows.high),
                }
                sda_fall = None;
                fall = Some(edge.at);
            }
            (Line::Sda, _) if !scl => {
                let f = fall.expect("an SDA change after an SCL fall");
                let hold = edge.at - f;
                if hold != 0 {
                    check("tHD;DAT", hold, d, 0.0);
                    assert!(ns(hold) <= rows.hd_dat_max, "tHD;DAT over its maximum");
                    check("tSU;DAT", f + l - edge.at, l - d, rows.su_dat);
                }
            }
            (Line::Sda, true) => {
                check("tSU;STO", edge.at - rise.unwrap(), s, rows.su_sto);
                stop_at = Some(edge.at);
            }
            (Line::Sda, false) => {
                match stop_at.take() {
                    Some(stop) => check("tBUF", edge.at - stop, l, rows.buf),
                    None if rise.is_some() => {
                        check("tSU;STA", edge.at - rise.unwrap(), l, rows.su_sta);
                    }
                    None => {}
                }
                sda_fall = Some(edge.at);
            }
        }
        if edge.line == Line::Scl {
            scl = edge.high;
        }
    }
    seen
}

fn rows_seen(seen: &[(&'static str, u64)]) -> Vec<&'static str> {
    let mut names: Vec<_> = seen.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names.dedup();
    names
}

const EVERY_ROW: [&str; 8] = [
    "tBUF", "tHD;DAT", "tHD;STA", "tHIGH", "tLOW", "tSU;DAT", "tSU;STA", "tSU;STO",
];

/// DS40002061B Table 29-14 at 100 kHz: `T` 160, `S` 72, `L` 88, `D` 10.
#[test]
fn every_interval_meets_table_29_14_at_100_khz() {
    let seen = bit_table(72, 160, &SLOW);
    assert_eq!(rows_seen(&seen), EVERY_ROW);
}

/// DS40002061B Table 29-14 above 100 kHz: `T` 40, `S` 18, `L` 22, `D` 2.
#[test]
fn every_interval_meets_table_29_14_at_400_khz() {
    let seen = bit_table(12, 40, &FAST);
    assert_eq!(rows_seen(&seen), EVERY_ROW);
}
