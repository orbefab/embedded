//! The reference for the run tests: `run_until` and `run_to_news` against a loop of single
//! `Cpu::step` calls, compared on every state the host can read.

use sim_avr::{Channel, Cpu, I2cStamp, PinReport};

pub const MAIN: usize = 0x34;

pub const NOP: u16 = 0x0000;
pub const SEI: u16 = 0x9478;
pub const RETI: u16 = 0x9518;

pub const fn inc(d: u16) -> u16 {
    0x9403 | (d << 4)
}

pub fn rjmp(from: usize, to: usize) -> u16 {
    0xC000 | ((to as i32 - from as i32 - 1) as u16 & 0x0FFF)
}

/// Everything the host can read between two runs.
#[derive(Debug, PartialEq)]
pub struct Seen {
    pub cycles: u64,
    pub pc: u16,
    pub sreg: u8,
    pub regs: Vec<u8>,
    pub data: Vec<u8>,
    pub sleeping: bool,
    pub stopped: bool,
    pub unknown: Option<u16>,
    pub pins: Vec<PinReport>,
    pub serial: Vec<(u8, u64)>,
    pub i2c: Vec<I2cStamp>,
}

pub fn seen(cpu: &mut Cpu) -> Seen {
    Seen {
        cycles: cpu.cycles(),
        pc: cpu.pc(),
        sreg: cpu.sreg(),
        regs: (0..32).map(|r| cpu.reg(r)).collect(),
        data: (0x20..=0x08FF).map(|a| cpu.peek(a)).collect(),
        sleeping: cpu.is_sleeping(),
        stopped: cpu.is_stopped(),
        unknown: cpu.unknown(),
        pins: std::iter::from_fn(|| cpu.take_pin()).collect(),
        serial: std::iter::from_fn(|| cpu.take(Channel::Usart0)).collect(),
        i2c: std::iter::from_fn(|| cpu.take_i2c()).collect(),
    }
}

/// The reference: one instruction per call, with the bus and the interrupt check after each.
pub fn step_until(cpu: &mut Cpu, limit: u64) {
    while cpu.cycles() < limit && !cpu.is_stopped() {
        cpu.step();
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Runner {
    Until,
    ToNews,
}

impl Runner {
    pub fn run(self, cpu: &mut Cpu, limit: u64) {
        match self {
            Runner::Until => cpu.run_until(limit),
            Runner::ToNews => {
                while cpu.cycles() < limit && !cpu.is_stopped() {
                    cpu.run_to_news(limit);
                }
            }
        }
    }
}

/// Runs `words` to each boundary through `runner` and through [`step_until`], and compares.
pub fn matches_stepping(words: &[u16], boundaries: &[u64], label: &str) {
    matches_stepping_from(&|cpu| cpu.load_words(words), boundaries, label);
}

pub fn matches_stepping_from(load: &dyn Fn(&mut Cpu), boundaries: &[u64], label: &str) {
    for runner in [Runner::Until, Runner::ToNews] {
        let mut run = Cpu::new();
        let mut stepped = Cpu::new();
        load(&mut run);
        load(&mut stepped);
        for &limit in boundaries {
            runner.run(&mut run, limit);
            step_until(&mut stepped, limit);
            assert_eq!(
                seen(&mut run),
                seen(&mut stepped),
                "{label}, {runner:?} to {limit}"
            );
        }
    }
}

fn vectors() -> Vec<u16> {
    let mut words = vec![NOP; MAIN];
    words[0] = rjmp(0, MAIN);
    words
}

/// The image with `main` from [`MAIN`] and `isr` placed after it, with each vector in `isrs`
/// jumping to `isr`.
pub fn image(main: &[u16], isrs: &[usize], isr: &[u16]) -> Vec<u16> {
    let mut words = vectors();
    words.extend_from_slice(main);
    let at = words.len();
    for &vector in isrs {
        words[vector] = rjmp(vector, at);
    }
    words.extend_from_slice(isr);
    words
}
