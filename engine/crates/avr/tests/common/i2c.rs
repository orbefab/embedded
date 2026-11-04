//! A peer on the TWI pads: the bus pull-up and a target at one address, driven through
//! [`Cpu::drive_pin`] one cycle at a time. It records the cycle of every bus edge, so a test
//! can measure each length of the TWI's bit table.

use std::collections::VecDeque;

use sim_avr::{Cpu, Level, PinDrive};

pub const SDA: u32 = 12;
pub const SCL: u32 = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Scl,
    Sda,
}

/// A bus level change at cycle `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub at: u64,
    pub line: Line,
    pub high: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// Waits for a START.
    Idle,
    /// Bits of SLA+R/W read so far.
    Address {
        bits: u8,
        sla: u8,
    },
    /// Holds the address acknowledge until the next SCL fall.
    AckAddress {
        read: bool,
    },
    Write {
        bits: u8,
        byte: u8,
    },
    AckWrite,
    /// Drives a read byte, `bit` from 0, MSB first.
    Read {
        bit: u8,
        byte: u8,
    },
    /// Released for the master's acknowledge, which the SCL rise samples.
    MasterAck {
        ack: bool,
    },
    /// Not addressed until the next START.
    Ignore,
}

pub struct Peer {
    /// The 7-bit address it acknowledges. `None` acknowledges nothing.
    pub address: Option<u8>,
    /// Bytes it returns on reads. An empty queue leaves SDA released, so the master reads 1s.
    pub reads: VecDeque<u8>,
    /// Data bytes it received, across transfers.
    pub received: Vec<u8>,
    /// It does not acknowledge the data byte with this index in a write, counting from 0.
    pub nack_at: Option<usize>,
    /// From the `k`-th SCL fall it saw, counting from 0, it holds SCL low for `n` cycles.
    pub stretch: Option<(usize, u64)>,
    /// At the `k`-th SCL fall it pulls SDA low and releases it `n` cycles later.
    pub steal: Option<(usize, u64)>,
    /// `d` cycles after the `k`-th SCL rise it flips its own SDA pull.
    pub flip: Option<(usize, u64)>,
    /// Pulls applied once the core has reached the cycle. `(cycle, line, low)`.
    pulls: Vec<(u64, Line, bool)>,
    pub edges: Vec<Edge>,
    falls: usize,
    rises: usize,
    release_scl: Option<u64>,
    release_sda: Option<u64>,
    stolen_until: Option<u64>,
    flip_at: Option<u64>,
    pull_sda: bool,
    pull_scl: bool,
    scl: bool,
    sda: bool,
    target: Target,
    written: usize,
}

fn high(cpu: &Cpu, pin: u32) -> bool {
    cpu.pin_levels().get(pin) == Some(Level::High)
}

impl Peer {
    /// Releases both lines, so each reads high unless the TWI pulls it low.
    pub fn new(cpu: &mut Cpu) -> Peer {
        cpu.drive_pin(SDA, Level::High);
        cpu.drive_pin(SCL, Level::High);
        Peer {
            address: None,
            reads: VecDeque::new(),
            received: Vec::new(),
            nack_at: None,
            stretch: None,
            steal: None,
            flip: None,
            pulls: Vec::new(),
            edges: Vec::new(),
            falls: 0,
            rises: 0,
            release_scl: None,
            release_sda: None,
            stolen_until: None,
            flip_at: None,
            pull_sda: false,
            pull_scl: false,
            scl: high(cpu, SCL),
            sda: high(cpu, SDA),
            target: Target::Idle,
            written: 0,
        }
    }

    pub fn at(cpu: &mut Cpu, address: u8) -> Peer {
        let mut peer = Peer::new(cpu);
        peer.address = Some(address);
        peer
    }

    /// Runs the core to `cycle`, one cycle at a time, and answers each bus edge on the cycle
    /// it sees it.
    pub fn run(&mut self, cpu: &mut Cpu, cycle: u64) {
        while cpu.cycles() < cycle {
            let next = cpu.cycles() + 1;
            cpu.run_until(next);
            self.timers(cpu);
            self.observe(cpu);
        }
    }

    /// Whether the TWI drives `pin` low now.
    pub fn twi_pulls(cpu: &Cpu, pin: u32) -> bool {
        cpu.pin_levels().drive(pin) == Some(PinDrive::Low)
    }

    /// Pull `line` low, or release it, on the cycle the core is on.
    pub fn drive(&mut self, cpu: &mut Cpu, line: Line, low: bool) {
        self.pull(cpu, line, low);
    }

    /// Pull `line` low, or release it, once `cycle` has been reached.
    pub fn pull_at(&mut self, cycle: u64, line: Line, low: bool) {
        self.pulls.push((cycle, line, low));
    }

    fn timers(&mut self, cpu: &mut Cpu) {
        let now = cpu.cycles();
        if self.release_scl.is_some_and(|at| now >= at) {
            self.release_scl = None;
            self.pull(cpu, Line::Scl, false);
        }
        if self.release_sda.is_some_and(|at| now >= at) {
            self.release_sda = None;
            self.pull(cpu, Line::Sda, false);
        }
        if self.flip_at.is_some_and(|at| now >= at) {
            self.flip_at = None;
            let pulled = self.pull_sda;
            self.pull(cpu, Line::Sda, !pulled);
        }
        let pending = std::mem::take(&mut self.pulls);
        let mut later = Vec::new();
        for (at, line, low) in pending {
            if at <= now {
                self.pull(cpu, line, low);
            } else {
                later.push((at, line, low));
            }
        }
        self.pulls = later;
    }

    fn pull(&mut self, cpu: &mut Cpu, line: Line, low: bool) {
        let (pin, held) = match line {
            Line::Scl => (SCL, &mut self.pull_scl),
            Line::Sda => (SDA, &mut self.pull_sda),
        };
        if *held == low {
            return;
        }
        *held = low;
        cpu.drive_pin(pin, if low { Level::Low } else { Level::High });
    }

    fn observe(&mut self, cpu: &mut Cpu) {
        for _ in 0..8 {
            let (scl, sda) = (high(cpu, SCL), high(cpu, SDA));
            if scl == self.scl && sda == self.sda {
                return;
            }
            let at = cpu.cycles();
            if sda != self.sda {
                self.sda = sda;
                self.edges.push(Edge {
                    at,
                    line: Line::Sda,
                    high: sda,
                });
                self.on_sda();
            }
            if scl != self.scl {
                self.scl = scl;
                self.edges.push(Edge {
                    at,
                    line: Line::Scl,
                    high: scl,
                });
                if scl {
                    self.on_rise(cpu);
                } else {
                    self.on_fall(cpu);
                }
            }
        }
        panic!("the bus did not settle at cycle {}", cpu.cycles());
    }

    fn on_sda(&mut self) {
        if !self.scl {
            return;
        }
        if self.sda {
            self.target = Target::Idle;
        } else if !self.pull_sda {
            self.target = Target::Address { bits: 0, sla: 0 };
            self.written = 0;
        }
    }

    fn on_rise(&mut self, cpu: &mut Cpu) {
        let k = self.rises;
        self.rises += 1;
        if let Some((at, delay)) = self.flip
            && at == k
        {
            self.flip_at = Some(cpu.cycles() + delay);
        }
        let bit = u8::from(self.sda);
        self.target = match self.target {
            Target::Address { bits, sla } => Target::Address {
                bits: bits + 1,
                sla: (sla << 1) | bit,
            },
            Target::Write { bits, byte } => Target::Write {
                bits: bits + 1,
                byte: (byte << 1) | bit,
            },
            Target::MasterAck { .. } => Target::MasterAck { ack: !self.sda },
            other => other,
        };
    }

    fn on_fall(&mut self, cpu: &mut Cpu) {
        let k = self.falls;
        self.falls += 1;
        if let Some((at, n)) = self.stretch
            && at == k
        {
            self.pull(cpu, Line::Scl, true);
            self.release_scl = Some(cpu.cycles() + n);
        }
        if let Some((at, n)) = self.steal
            && at == k
        {
            self.pull(cpu, Line::Sda, true);
            self.release_sda = Some(cpu.cycles() + n);
            self.stolen_until = self.release_sda;
            self.target = Target::Ignore;
            return;
        }
        self.target = match self.target {
            Target::Address { bits: 8, sla } => {
                if self.address == Some(sla >> 1) {
                    self.pull(cpu, Line::Sda, true);
                    Target::AckAddress { read: sla & 1 == 1 }
                } else {
                    Target::Ignore
                }
            }
            Target::AckAddress { read: false } | Target::AckWrite => {
                self.pull(cpu, Line::Sda, false);
                Target::Write { bits: 0, byte: 0 }
            }
            Target::AckAddress { read: true } => self.next_read(cpu),
            Target::Write { bits: 8, byte } => {
                self.received.push(byte);
                let nack = self.nack_at == Some(self.written);
                self.written += 1;
                if nack {
                    Target::Ignore
                } else {
                    self.pull(cpu, Line::Sda, true);
                    Target::AckWrite
                }
            }
            Target::Read { bit: 7, .. } => {
                self.pull(cpu, Line::Sda, false);
                Target::MasterAck { ack: false }
            }
            Target::Read { bit, byte } => {
                let bit = bit + 1;
                self.pull(cpu, Line::Sda, byte & (0x80 >> bit) == 0);
                Target::Read { bit, byte }
            }
            Target::MasterAck { ack: true } => self.next_read(cpu),
            Target::MasterAck { ack: false } => Target::Ignore,
            other => other,
        };
    }

    fn next_read(&mut self, cpu: &mut Cpu) -> Target {
        match self.reads.pop_front() {
            Some(byte) => {
                self.pull(cpu, Line::Sda, byte & 0x80 == 0);
                Target::Read { bit: 0, byte }
            }
            None => {
                self.pull(cpu, Line::Sda, false);
                Target::Ignore
            }
        }
    }

    /// The cycle a [`Peer::steal`] lets SDA go.
    pub fn release_at(&self) -> u64 {
        self.stolen_until.expect("no steal happened")
    }

    /// The cycles of every edge of `line` to `high`.
    pub fn times(&self, line: Line, high: bool) -> Vec<u64> {
        self.edges
            .iter()
            .filter(|edge| edge.line == line && edge.high == high)
            .map(|edge| edge.at)
            .collect()
    }
}
