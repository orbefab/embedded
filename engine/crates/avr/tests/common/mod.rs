#![allow(dead_code)]

//! Instruction words and I/O addresses shared by the integration tests.
//!
//! [`Instructions::insns`] runs a fixed instruction count.
//!
//! Addresses are the data-space addresses in DS40002061B Table 31-1.

pub mod i2c;
pub mod runs;

pub const SPL: u16 = 0x005D;
pub const SPH: u16 = 0x005E;
pub const SREG: u16 = 0x005F;
pub const PINB: u16 = 0x0023;
pub const DDRB: u16 = 0x0024;
pub const PORTB: u16 = 0x0025;
pub const TIFR0: u16 = 0x0035;
pub const TIFR1: u16 = 0x0036;
pub const TIFR2: u16 = 0x0037;
pub const TCCR0B: u16 = 0x0045;
pub const TCNT0: u16 = 0x0046;
pub const UCSR0B: u16 = 0x00C1;
pub const UDR0: u16 = 0x00C6;

pub const fn rr(base: u16, d: u16, r: u16) -> u16 {
    base | ((r & 0x10) << 5) | ((d & 0x1F) << 4) | (r & 0x0F)
}

pub const fn imm(base: u16, d: u16, k: u16) -> u16 {
    base | ((k & 0xF0) << 4) | (((d - 16) & 0x0F) << 4) | (k & 0x0F)
}

pub const fn ldi(d: u16, k: u16) -> u16 {
    imm(0xE000, d, k)
}

pub const fn in_io(reg: u16, io: u16) -> u16 {
    0xB000 | ((reg & 0x1F) << 4) | ((io & 0x30) << 5) | (io & 0x0F)
}

pub const fn out(reg: u16, io: u16) -> u16 {
    0xB800 | ((reg & 0x1F) << 4) | ((io & 0x30) << 5) | (io & 0x0F)
}

pub const fn sts(reg: u16, addr: u16) -> [u16; 2] {
    [0x9200 | ((reg & 0x1F) << 4), addr]
}

pub const fn lds(reg: u16, addr: u16) -> [u16; 2] {
    [0x9000 | ((reg & 0x1F) << 4), addr]
}

pub const fn sbi(io: u16, bit: u16) -> u16 {
    0x9A00 | ((io & 0x1F) << 3) | (bit & 7)
}

pub const fn cbi(io: u16, bit: u16) -> u16 {
    0x9800 | ((io & 0x1F) << 3) | (bit & 7)
}

pub const fn sbrc(reg: u16, bit: u16) -> u16 {
    0xFC00 | (reg << 4) | bit
}

pub const fn sbis(io: u16, bit: u16) -> u16 {
    0x9B00 | (io << 3) | bit
}

pub const fn sbic(io: u16, bit: u16) -> u16 {
    0x9900 | (io << 3) | bit
}

pub const fn sbrs(reg: u16, bit: u16) -> u16 {
    0xFE00 | ((reg & 0x1F) << 4) | bit
}

pub const fn adiw(k: u16, pair: u16) -> u16 {
    let dd = (pair - 24) / 2;
    0x9600 | ((k & 0x30) << 2) | (dd << 4) | (k & 0x0F)
}

pub const fn sbiw(k: u16, pair: u16) -> u16 {
    adiw(k, pair) | 0x0100
}

pub const fn brbs(s: u16, k: u16) -> u16 {
    0xF000 | ((k & 0x7F) << 3) | s
}

pub const fn brbc(s: u16, k: u16) -> u16 {
    0xF400 | ((k & 0x7F) << 3) | s
}

pub const fn ldd(base_bit3: u16, store: bool, reg: u16, q: u16) -> u16 {
    let mut word = 0x8000 | ((q & 0x20) << 8) | ((q & 0x18) << 7) | ((reg & 0x1F) << 4) | (q & 0x7);
    word |= base_bit3;
    if store {
        word |= 0x0200;
    }
    word
}

/// `RJMP .-2`. Holds the core on this instruction.
pub fn spin() -> u16 {
    0xCFFF
}

/// One `LDS` into r17.
pub fn read_reg(cpu: &mut sim_avr::Cpu, addr: u16) -> u8 {
    cpu.load_words(&lds(17, addr));
    cpu.insns(1);
    cpu.reg(17)
}

/// Run `n` instructions via [`sim_avr::Cpu::step`].
///
/// One `step` is one instruction while the core can fetch.
pub trait Instructions {
    fn insns(&mut self, n: u32);
}

/// Arm a host master at the current cycle and run until `SS` rises.
///
/// An empty `bytes`, or a transfer already active, returns an empty vec and
/// does not run.
pub fn clock_spi(cpu: &mut sim_avr::Cpu, bytes: &[u8], clock: sim_avr::SpiClock) -> Vec<u8> {
    let start = cpu.cycles();
    if !cpu.queue_spi_master(start, bytes, clock) {
        return Vec::new();
    }
    let half = (u64::from(clock.period) / 2).max(1);
    let end = start + (bytes.len() as u64) * 16 * half + 1;
    cpu.run_until(end.saturating_add(1));
    let mut out = Vec::new();
    while let Some((byte, _)) = cpu.take(sim_avr::Channel::SpiHostMaster) {
        out.push(byte);
    }
    out
}

impl Instructions for sim_avr::Cpu {
    fn insns(&mut self, n: u32) {
        for _ in 0..n {
            if self.is_stopped() {
                break;
            }
            self.step();
        }
    }
}
