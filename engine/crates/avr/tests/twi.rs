//! TWI master on the ATmega328P, DS40002061B §22, transaction level.
//!
//! `T` is `16 + 2·TWBR·4^TWPS`. These tests use `TWBR` = 12, so `T` = 40 and a
//! byte phase is `9·T` = 360. Times are measured from the `TWCR` poke, which
//! launches the phase on that cycle.

use sim_avr::{Cpu, FuseBytes, I2cNote, Level, Port, ResetKind, test_hooks};

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
const BYTE: u64 = 9 * T;

const SDA: u32 = 12;
const SCL: u32 = 13;

fn bare() -> Cpu {
    let mut cpu = Cpu::new();
    let fuses = FuseBytes::CRYSTAL;
    cpu.set_fuses(fuses.low, fuses.high, fuses.extended);
    cpu
}

fn machine() -> Cpu {
    let mut cpu = bare();
    // One-cycle NOPs so `run_until` can stop on an exact cycle.
    cpu.load_words(&vec![0x0000; 30_000]);
    cpu.poke(TWBR, 12);
    cpu
}

fn host(cpu: &mut Cpu, addr: u8, reads: &[u8], repeat: bool, nack: Option<u8>) {
    let at = cpu.cycles();
    cpu.queue_i2c(addr, true, reads.to_vec(), repeat, nack);
    cpu.run_until(at + 2);
}

fn detach(cpu: &mut Cpu, addr: u8) {
    let at = cpu.cycles();
    cpu.queue_i2c(addr, false, Vec::new(), false, None);
    cpu.run_until(at + 2);
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

fn reach(cpu: &mut Cpu, cycle: u64) {
    cpu.run_until(cycle);
}

fn start_at(cpu: &mut Cpu) -> u64 {
    go(cpu, TWSTA | TWEA)
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
        host(&mut cpu, 8, b"", false, None);
        let at = start_at(&mut cpu);
        reach(&mut cpu, at + T / 2);
        test_hooks::reset_cpu(&mut cpu, kind);
        assert_eq!(peek(&cpu, TWBR), 0x00, "{kind:?}");
        assert_eq!(peek(&cpu, TWSR), 0xF8, "{kind:?}");
        assert_eq!(peek(&cpu, TWAR), 0xFE, "{kind:?}");
        assert_eq!(peek(&cpu, TWDR), 0xFF, "{kind:?}");
        assert_eq!(peek(&cpu, TWCR), 0x00, "{kind:?}");
        assert_eq!(peek(&cpu, TWAMR), 0x00, "{kind:?}");
        let later = cpu.cycles() + 4_000;
        reach(&mut cpu, later);
        assert!(pull(&mut cpu).is_empty(), "{kind:?} emitted a phase");
    }
}

#[test]
fn reserved_bits_read_zero_and_twsto_stays_until_stop() {
    let mut cpu = machine();
    poke(&mut cpu, TWCR, TWEN | 0x02);
    assert_eq!(peek(&cpu, TWCR) & 0x02, 0);
    poke(&mut cpu, TWSR, 0x07);
    assert_eq!(peek(&cpu, TWSR), 0xFB);
    poke(&mut cpu, TWSR, 0x00);
    poke(&mut cpu, TWAMR, 0xFF);
    assert_eq!(peek(&cpu, TWAMR), 0xFE);
    poke(&mut cpu, TWCR, TWEN | TWWC);
    assert_eq!(peek(&cpu, TWCR) & TWWC, 0);

    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let write = go(&mut cpu, TWEA);
    reach(&mut cpu, write + BYTE);
    let stop = go(&mut cpu, TWSTO);
    assert_ne!(peek(&cpu, TWCR) & TWSTO, 0);
    reach(&mut cpu, stop + T - 1);
    assert_ne!(peek(&cpu, TWCR) & TWSTO, 0);
    reach(&mut cpu, stop + T);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);

    let mut idle = machine();
    poke(&mut idle, TWCR, TWEN | TWSTO | TWINT);
    assert_eq!(peek(&idle, TWCR) & TWSTO, 0);
    let later = idle.cycles() + T;
    reach(&mut idle, later);
    assert!(pull(&mut idle).is_empty());
}

#[test]
fn writing_zero_leaves_twint_and_writing_one_clears_it() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    poke(&mut cpu, TWCR, TWEN | TWEA);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    let later = cpu.cycles() + BYTE;
    reach(&mut cpu, later);
    assert_eq!(pull(&mut cpu).len(), 1);
    go(&mut cpu, TWEA);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
}

#[test]
fn vector_25_is_level_and_entry_leaves_twint() {
    let mut cpu = machine();
    cpu.load_words(&vector_image());
    cpu.set_sreg(0x80);
    let at = go(&mut cpu, TWSTA | TWIE | TWEA);
    reach(&mut cpu, at + T + 4);
    assert_eq!(cpu.pc(), 0x0030);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
}

#[test]
fn one_twcr_write_sets_twie_and_clears_twint() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
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
    reach(&mut cpu, before + 1 + 2 + 2);
    assert_eq!(cpu.pc(), 3, "the write took the TWI vector");
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    assert_ne!(peek(&cpu, TWCR) & TWIE, 0);
}

#[test]
fn twwc_retry_keeps_the_first_successful_byte() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    poke(&mut cpu, TWDR, 0x99);
    assert_ne!(peek(&cpu, TWCR) & TWWC, 0);
    assert_eq!(peek(&cpu, TWDR), 8 << 1);
    reach(&mut cpu, sla + BYTE);
    poke(&mut cpu, TWDR, 0x78);
    assert_eq!(peek(&cpu, TWCR) & TWWC, 0);
    assert_eq!(peek(&cpu, TWDR), 0x78);
    let data = go(&mut cpu, TWEA);
    reach(&mut cpu, data + BYTE);
    let phases = pull(&mut cpu);
    assert_eq!(
        phases.last().map(|(_, phase)| *phase),
        Some(I2cNote::Data {
            byte: 0x78,
            write: true,
            ack: true,
        })
    );
    assert_eq!(phases.last().map(|(at, _)| *at), Some(data + BYTE));
}

#[test]
fn twsr_tracks_twint_in_the_same_cycle() {
    let mut cpu = machine();
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T - 1);
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    reach(&mut cpu, at + T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
}

#[test]
fn twen_clear_aborts_without_an_event_and_the_next_start_is_08() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + 10);
    poke(&mut cpu, TWCR, 0);
    reach(&mut cpu, sla + BYTE + T);
    let phases = pull(&mut cpu);
    assert!(
        phases
            .iter()
            .all(|(_, phase)| !matches!(phase, I2cNote::Address { .. })),
        "{phases:?}"
    );
    assert_eq!(peek(&cpu, TWCR), 0);
    let again = start_at(&mut cpu);
    reach(&mut cpu, again + T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
}

#[test]
fn prtwi_freezes_and_idle_still_runs() {
    let mut cpu = machine();
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + 10);
    let frozen = cpu.cycles();
    poke(&mut cpu, PRR, PRTWI);
    poke(&mut cpu, TWBR, 1);
    assert_eq!(peek(&cpu, TWBR), 12, "a frozen write is discarded");
    reach(&mut cpu, frozen + 500);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    let resume = cpu.cycles();
    poke(&mut cpu, PRR, 0);
    let done = at + T + (resume - frozen);
    reach(&mut cpu, done - 1);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    reach(&mut cpu, done);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);

    let mut down = machine();
    down.load_words(&[0x9588, 0xCFFF]);
    down.poke(SMCR, 0x05);
    let at = start_at(&mut down);
    reach(&mut down, at + 500);
    assert!(down.is_sleeping());
    assert_eq!(peek(&down, TWSR), 0xF8);

    let mut idle = machine();
    idle.load_words(&[0x9588, 0xCFFF]);
    idle.poke(SMCR, 0x01);
    let at = start_at(&mut idle);
    reach(&mut idle, at + T + 4);
    assert_eq!(peek(&idle, TWSR) & 0xF8, 0x08);
}

#[test]
fn twar_and_twamr_are_stored_and_do_not_wake() {
    let mut cpu = machine();
    cpu.load_words(&[0x9588, 0xCFFF]);
    cpu.poke(SMCR, 0x01);
    reach(&mut cpu, 2);
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
    reach(&mut cpu, later);
    assert!(cpu.is_sleeping());
}

#[test]
fn start_repeated_start_and_off_table_sources() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"A", true, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
    assert_ne!(peek(&cpu, TWCR) & TWSTA, 0);
    let phases = pull(&mut cpu);
    assert_eq!(phases, vec![(at + T, I2cNote::Start { repeated: false })]);

    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x40);
    let again = go(&mut cpu, TWSTA | TWEA);
    reach(&mut cpu, again + T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x10);
    assert_eq!(logged(&cpu, TWCR), 1, "0x40 is an off-table repeated START");
    let third = go(&mut cpu, TWSTA | TWEA);
    reach(&mut cpu, third + T);
    assert_eq!(logged(&cpu, TWCR), 1, "the off-table note is once");
    let phases = pull(&mut cpu);
    assert!(
        phases
            .iter()
            .any(|(_, phase)| phase == &I2cNote::Start { repeated: true })
    );
}

#[test]
fn address_status_follows_the_host_ack() {
    let cases = [(false, true, 0x18u8), (true, true, 0x40)];
    for (read, present, status) in cases {
        let mut cpu = machine();
        if present {
            host(&mut cpu, 8, b"", false, None);
        }
        let at = start_at(&mut cpu);
        reach(&mut cpu, at + T);
        poke(&mut cpu, TWDR, (8 << 1) | u8::from(read));
        let sla = go(&mut cpu, TWEA);
        reach(&mut cpu, sla + BYTE);
        assert_eq!(peek(&cpu, TWSR) & 0xF8, status, "read={read}");
    }
    let mut missing = machine();
    // §22.3.3. Only the addressed device ACKs. Address 9 must not ACK 8.
    host(&mut missing, 9, b"", false, None);
    let at = start_at(&mut missing);
    reach(&mut missing, at + T);
    poke(&mut missing, TWDR, 8 << 1);
    let sla = go(&mut missing, TWEA);
    reach(&mut missing, sla + BYTE);
    assert_eq!(peek(&missing, TWSR) & 0xF8, 0x20); // §22.7.1. SLA+W NACK.
    poke(&mut missing, TWDR, (8 << 1) | 1);
    let at = start_at(&mut missing);
    reach(&mut missing, at + T);
    poke(&mut missing, TWDR, (8 << 1) | 1);
    let sla = go(&mut missing, TWEA);
    reach(&mut missing, sla + BYTE);
    assert_eq!(peek(&missing, TWSR) & 0xF8, 0x48); // §22.7.2. SLA+R NACK.
}

#[test]
fn transmit_and_data_after_a_sla_nack() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, Some(0));
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x18);
    poke(&mut cpu, TWDR, 0x11);
    let data = go(&mut cpu, TWEA);
    reach(&mut cpu, data + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x30);
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
    let at = start_at(&mut late);
    reach(&mut late, at + T);
    poke(&mut late, TWDR, 8 << 1);
    let sla = go(&mut late, TWEA);
    reach(&mut late, sla + BYTE);
    assert_eq!(peek(&late, TWSR) & 0xF8, 0x20);
    host(&mut late, 8, b"", false, None);
    poke(&mut late, TWDR, 0x22);
    let data = go(&mut late, TWEA);
    reach(&mut late, data + BYTE);
    assert_eq!(peek(&late, TWSR) & 0xF8, 0x30);
    let phases = pull(&mut late);
    assert!(phases.iter().any(|(_, phase)| {
        phase
            == &I2cNote::Data {
                byte: 0x22,
                write: true,
                ack: false,
            }
    }));
}

#[test]
fn receive_samples_twea_at_the_end_of_the_phase() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"ab", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x40);
    let data = go(&mut cpu, 0);
    reach(&mut cpu, data + BYTE / 2);
    poke(&mut cpu, TWCR, TWEN | TWEA);
    reach(&mut cpu, data + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x50);
    assert_eq!(peek(&cpu, TWDR), b'a');
    let nack = go(&mut cpu, 0);
    reach(&mut cpu, nack + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x58);
    assert_eq!(peek(&cpu, TWDR), b'b');
}

#[test]
fn stop_clears_twsto_and_leaves_twint_clear() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    let stop = go(&mut cpu, TWSTO);
    reach(&mut cpu, stop + T);
    assert_eq!(peek(&cpu, TWCR) & (TWSTO | TWINT), 0);
    assert_eq!(peek(&cpu, TWSR), 0xF8);
    let phases = pull(&mut cpu);
    assert_eq!(phases.last().copied(), Some((stop + T, I2cNote::Stop)));
}

#[test]
fn stop_then_start_lands_on_08() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    let both = go(&mut cpu, TWSTO | TWSTA);
    reach(&mut cpu, both + T);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    assert_ne!(peek(&cpu, TWCR) & TWSTA, 0);
    reach(&mut cpu, both + 2 * T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
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
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x48);
    let _ = pull(&mut cpu);
    go(&mut cpu, 0);
    let later = cpu.cycles() + BYTE;
    reach(&mut cpu, later);
    assert!(pull(&mut cpu).is_empty());
    assert_eq!(logged(&cpu, TWCR), 1);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);
    let again = start_at(&mut cpu);
    reach(&mut cpu, again + T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
}

#[test]
fn a_busy_twcr_write_stores_bits_and_does_not_start() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + 10);
    poke(&mut cpu, TWCR, TWEN | TWSTA | TWEA);
    assert_ne!(peek(&cpu, TWCR) & TWSTA, 0);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x18);
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
    reach(&mut cpu, at + 19);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    reach(&mut cpu, at + 20);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
    let again = start_at(&mut cpu);
    assert_eq!(
        logged(&cpu, TWBR),
        1,
        "one note for the clock, not one per phase"
    );
    reach(&mut cpu, again + 20);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x10);
}

#[test]
fn phases_are_exact() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"Z", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    let data = go(&mut cpu, 0);
    reach(&mut cpu, data + BYTE);
    let stop = go(&mut cpu, TWSTO);
    reach(&mut cpu, stop + T);
    assert_eq!(
        pull(&mut cpu),
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
}

#[test]
fn twint_waits_without_a_timeout() {
    let mut cpu = machine();
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    assert_eq!(
        pull(&mut cpu),
        vec![(at + T, I2cNote::Start { repeated: false })]
    );
    reach(&mut cpu, at + T + 20_000);
    assert!(pull(&mut cpu).is_empty());
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
}

#[test]
fn nack_paths_and_a_replace_wait_for_the_next_address() {
    let mut cpu = machine();
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x20);

    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, Some(1));
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    poke(&mut cpu, TWDR, 0x01);
    let first = go(&mut cpu, TWEA);
    reach(&mut cpu, first + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x28);
    poke(&mut cpu, TWDR, 0x02);
    let second = go(&mut cpu, TWEA);
    reach(&mut cpu, second + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x30);

    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + 20);
    detach(&mut cpu, 8);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(
        peek(&cpu, TWSR) & 0xF8,
        0x18,
        "detach waits for the next address"
    );
    poke(&mut cpu, TWDR, 0x01);
    let data = go(&mut cpu, TWEA);
    reach(&mut cpu, data + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x28);
    let stop = go(&mut cpu, TWSTO);
    reach(&mut cpu, stop + T);
    let again = start_at(&mut cpu);
    reach(&mut cpu, again + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x20);
}

#[test]
fn repeat_restarts_and_a_queue_does_not_and_the_tail_is_ff() {
    let mut queued = machine();
    host(&mut queued, 8, b"ab", false, None);
    read_one(&mut queued, b'a', 0x58);
    let stop = go(&mut queued, TWSTO);
    reach(&mut queued, stop + T);
    read_one(&mut queued, b'b', 0x58);
    let stop = go(&mut queued, TWSTO);
    reach(&mut queued, stop + T);
    read_one(&mut queued, 0xFF, 0x58);
    assert_eq!(logged(&queued, TWDR), 1);
    read_one(&mut queued, 0xFF, 0x58);
    assert_eq!(logged(&queued, TWDR), 1);

    let mut repeating = machine();
    host(&mut repeating, 8, b"ab", true, None);
    read_one(&mut repeating, b'a', 0x58);
    let stop = go(&mut repeating, TWSTO);
    reach(&mut repeating, stop + T);
    read_one(&mut repeating, b'a', 0x58);
}

fn read_one(cpu: &mut Cpu, byte: u8, status: u8) {
    let at = start_at(cpu);
    reach(cpu, at + T);
    poke(cpu, TWDR, (8 << 1) | 1);
    let sla = go(cpu, TWEA);
    reach(cpu, sla + BYTE);
    let data = go(cpu, 0);
    reach(cpu, data + BYTE);
    assert_eq!(peek(cpu, TWDR), byte);
    assert_eq!(peek(cpu, TWSR) & 0xF8, status);
}

#[test]
fn a_repeated_start_reads_after_a_write() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"Z", true, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    poke(&mut cpu, TWDR, 0x11);
    let data = go(&mut cpu, TWEA);
    reach(&mut cpu, data + BYTE);
    let again = go(&mut cpu, TWSTA | TWEA);
    reach(&mut cpu, again + T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x10);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x40);
    let data = go(&mut cpu, 0);
    reach(&mut cpu, data + BYTE);
    assert_eq!(peek(&cpu, TWDR), b'Z');
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x58);
}

#[test]
fn twi_pads_ignore_ddr_and_log_a_host_drive() {
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
    assert_eq!(logged(&cpu, TWCR), 1);
    cpu.drive_pin(SCL, Level::High);
    assert_eq!(logged(&cpu, TWCR), 1);
    cpu.drive_pin(SDA, Level::Released);
    cpu.drive_pin(SCL, Level::Released);
    poke(&mut cpu, TWCR, 0);
    poke(&mut cpu, DDRC, 1 << 4);
    poke(&mut cpu, PORTC, 0);
    assert_eq!(cpu.pin_levels().get(SDA), Some(Level::Low));
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
        reach(cpu, (cpu.cycles() + 50_000).min(limit));
    }
    assert_ne!(
        peek(cpu, MCUSR) & flag,
        0,
        "reset flag {flag:#04x} did not set"
    );
    let born = cpu.cycles();
    let startup = born + 1_200_000;
    while cpu.cycles() < startup && cpu.pc() == 0 {
        reach(cpu, (cpu.cycles() + 50_000).min(startup));
    }
    reach(cpu, cpu.cycles() + 4);
    assert_ne!(cpu.pc(), 0, "still in the start-up time-out");
}

fn sla_w(cpu: &mut Cpu) {
    poke(cpu, TWBR, 12);
    let at = start_at(cpu);
    reach(cpu, at + T);
    poke(cpu, TWDR, 8 << 1);
    let sla = go(cpu, TWEA);
    reach(cpu, sla + BYTE);
    assert_eq!(peek(cpu, TWSR) & 0xF8, 0x18);
}

#[test]
fn host_i2c_device_survives_watchdog_and_external_reset() {
    let mut cpu = machine();
    host(&mut cpu, 8, &[0xA1, 0xA2], false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x40);
    let data = go(&mut cpu, 0);
    reach(&mut cpu, data + BYTE);
    assert_eq!(peek(&cpu, TWDR), 0xA1);
    arm_wdt(&mut cpu);
    after_startup(&mut cpu, WDRF);
    sla_w(&mut cpu);
    let stop = go(&mut cpu, TWSTO);
    reach(&mut cpu, stop + T);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, (8 << 1) | 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + BYTE);
    let data = go(&mut cpu, 0);
    reach(&mut cpu, data + BYTE);
    assert_eq!(
        peek(&cpu, TWDR),
        0xA2,
        "the byte already read stays consumed"
    );

    let mut external = machine();
    host(&mut external, 8, b"", false, None);
    let at = start_at(&mut external);
    reach(&mut external, at + T / 2);
    test_hooks::reset_cpu(&mut external, ResetKind::External);
    assert!(
        pull(&mut external).is_empty(),
        "the open start emits nothing"
    );
    after_startup(&mut external, EXTRF);
    sla_w(&mut external);
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
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
    poke(&mut cpu, TWCR, TWINT | TWSTO | TWEN);
    assert_eq!(peek(&cpu, TWCR) & TWSTO, 0);
    assert_eq!(logged(&cpu, TWCR), 1);
}

#[test]
fn a_twint_write_during_a_phase_does_not_start_another() {
    let mut cpu = machine();
    host(&mut cpu, 8, b"", false, None);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    poke(&mut cpu, TWDR, 8 << 1);
    let sla = go(&mut cpu, TWEA);
    reach(&mut cpu, sla + 10);
    poke(&mut cpu, TWCR, TWINT | TWEN | TWSTA | TWEA);
    reach(&mut cpu, sla + BYTE);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x18);
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
    poke(&mut cpu, TWSR, 1);
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + 111);
    assert_eq!(peek(&cpu, TWCR) & TWINT, 0);
    reach(&mut cpu, at + 112);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0x08);
}

#[test]
fn repeated_start_from_0x08_is_logged_once() {
    let mut cpu = machine();
    let at = start_at(&mut cpu);
    reach(&mut cpu, at + T);
    assert_eq!(logged(&cpu, TWCR), 0);
    poke(&mut cpu, TWCR, TWINT | TWSTA | TWEN);
    assert_eq!(logged(&cpu, TWCR), 1);
    poke(&mut cpu, TWCR, TWINT | TWSTA | TWEN);
    assert_eq!(logged(&cpu, TWCR), 1);
}

#[test]
fn a_cpu_store_while_prtwi_is_set_is_ignored() {
    let mut cpu = machine();
    // ldi r16, TWINT|TWSTA|TWEN; sts TWCR, r16; ldi r16, 0x5A; sts TWDR, r16
    cpu.load_words(&[0xEA04, 0x9300, 0x00BC, 0xE50A, 0x9300, 0x00BB]);
    poke(&mut cpu, PRR, PRTWI);
    reach(&mut cpu, 6);
    assert_eq!(peek(&cpu, TWCR), 0);
    assert_eq!(peek(&cpu, TWDR), 0xFF);
    assert_eq!(peek(&cpu, TWSR) & 0xF8, 0xF8);
}

#[test]
fn a_pending_twint_does_not_wake_or_interrupt_while_frozen() {
    let mut words = vec![0x0000u16; 0x31];
    words[40] = 0x9478;
    words[41] = 0xCFFF;
    words[0x30] = 0xCFFF;
    let mut cpu = machine();
    cpu.load_words(&words);
    poke(&mut cpu, TWBR, 12);
    let at = go(&mut cpu, TWSTA | TWIE);
    reach(&mut cpu, at + T);
    assert_ne!(peek(&cpu, TWCR) & TWINT, 0);
    poke(&mut cpu, PRR, PRTWI);
    reach(&mut cpu, at + T + 30);
    assert_eq!(cpu.pc(), 41);

    let mut down = vec![0x0000u16; 48];
    down[40] = 0x9588;
    let mut asleep = machine();
    asleep.load_words(&down);
    poke(&mut asleep, TWBR, 12);
    poke(&mut asleep, SMCR, 0x05);
    let at = go(&mut asleep, TWSTA | TWIE);
    reach(&mut asleep, at + T + 2);
    assert!(asleep.is_sleeping());
    reach(&mut asleep, at + T + 100);
    assert!(asleep.is_sleeping());
}

use crate::common;
