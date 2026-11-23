pub mod board;
pub mod elf;
pub mod s3board;

use std::cell::Cell;
use std::ops::Range;

use orbe_xtensa::{Bus, Config, Core, Cycle, Exit, Stop, Width};
use sha2::{Digest, Sha256};

pub fn fixture_path(name: &str) -> String {
    // The probe images stay with the core tests. This crate only names the board that runs them.
    format!(
        "{}/../../cores/xtensa/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

pub fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).unwrap_or_else(|err| panic!("fixture {name}: {err}"))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A board fixture: its `key: value` header and the UART text after `---`.
pub struct Fixture {
    header: Vec<(String, String)>,
    pub body: String,
}

impl Fixture {
    pub fn read(name: &str) -> Fixture {
        let text = String::from_utf8(read_fixture(name)).unwrap();
        let (head, body) = text.split_once("---\n").expect("fixture has a --- line");
        let header = head
            .lines()
            .map(|line| {
                let (key, value) = line.split_once(": ").expect("header line is key: value");
                (key.to_string(), value.to_string())
            })
            .collect();
        Fixture {
            header,
            body: body.to_string(),
        }
    }

    pub fn get(&self, key: &str) -> &str {
        &self
            .header
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("fixture header has {key}"))
            .1
    }
}

/// The 1-based line where two texts first differ, with both lines, or `None` when they are equal.
pub fn first_difference(have: &str, want: &str) -> Option<(usize, String, String)> {
    if have == want {
        return None;
    }
    let mut have_lines = have.split_inclusive('\n');
    let mut want_lines = want.split_inclusive('\n');
    let mut line = 1;
    loop {
        match (have_lines.next(), want_lines.next()) {
            (Some(a), Some(b)) if a == b => line += 1,
            (a, b) => {
                return Some((
                    line,
                    a.unwrap_or("<end>").escape_debug().to_string(),
                    b.unwrap_or("<end>").escape_debug().to_string(),
                ));
            }
        }
    }
}

/// Run lengths for a split run, from a seeded xorshift: mostly short, some up to 2^16 cycles.
pub struct Pieces(u64);

impl Pieces {
    pub fn new(seed: u64) -> Pieces {
        Pieces(seed | 1)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        let bits = self.0 % 17;
        1 + (self.0 >> 8) % (1 << bits)
    }
}

/// Runs to `limit`, in one call or in pieces, and returns the stop that is not an early limit.
pub fn run_in_pieces<C: Config, B: Bus>(
    core: &mut Core<C>,
    bus: &mut B,
    limit: Cycle,
    pieces: &mut Option<Pieces>,
) -> Stop {
    loop {
        let to = match pieces {
            Some(pieces) => Cycle(limit.0.min(core.cycle().0 + pieces.next())),
            None => limit,
        };
        let stop = core.run_until(bus, to);
        if stop == Stop::Spin {
            continue;
        }
        if stop != Stop::Limit || to == limit {
            return stop;
        }
    }
}

/// `bus` with a trace of its loads and stores, and with a code version when it is kept. A kept
/// bus moves its version at each store into `code`. When it is also still, it says no access
/// moves the lines, which is true of a bus whose lines move only with time, so the core's
/// straight runs go on past its loads and stores. A plain bus keeps no version, so the core
/// makes a full step before every op on it.
pub struct Straight<B> {
    pub bus: B,
    version: Option<u64>,
    code: Range<u32>,
    still: bool,
    /// A hash of each load and store, in order: its cycle, address, width and value.
    pub trace: u64,
    /// How many times the core asked whether to yield, which it does before each full step and
    /// never inside a straight run.
    pub steps: Cell<u64>,
}

impl<B: Bus> Straight<B> {
    pub fn plain(bus: B) -> Self {
        Straight {
            bus,
            version: None,
            code: 0..0,
            still: false,
            trace: 0,
            steps: Cell::new(0),
        }
    }

    pub fn kept(bus: B, code: Range<u32>, still: bool) -> Self {
        Straight {
            version: Some(0),
            code,
            still,
            ..Straight::plain(bus)
        }
    }

    fn note(&mut self, words: [u64; 4]) {
        for word in words {
            self.trace = (self.trace ^ word).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

impl<B: Bus> Bus for Straight<B> {
    fn instruction_bus(&self, addr: u32) -> bool {
        self.bus.instruction_bus(addr)
    }

    fn fetch(&mut self, now: Cycle, addr: u32) -> Result<u32, Exit> {
        self.bus.fetch(now, addr)
    }

    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        let value = self.bus.load(now, addr, width);
        let read = value.map_or(u64::MAX, u64::from);
        self.note([now.0, u64::from(addr), u64::from(width.bytes()), read]);
        value
    }

    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        let done = self.bus.store(now, addr, width, value);
        let wrote = u64::from(value) | u64::from(done.is_ok()) << 32 | 1 << 33;
        self.note([now.0, u64::from(addr), u64::from(width.bytes()), wrote]);
        if done.is_ok()
            && self.code.contains(&addr)
            && let Some(version) = &mut self.version
        {
            *version += 1;
        }
        done
    }

    fn check_store(&mut self, now: Cycle, addr: u32) -> Result<(), Exit> {
        self.bus.check_store(now, addr)
    }

    fn interrupts(&mut self, now: Cycle) -> u32 {
        self.bus.interrupts(now)
    }

    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        self.bus.next_event(now)
    }

    fn yields(&self) -> bool {
        self.steps.set(self.steps.get() + 1);
        self.bus.yields()
    }

    fn begin_step(&mut self, now: Cycle) {
        self.bus.begin_step(now);
    }

    fn access_moves_lines(&self, addr: u32) -> bool {
        !self.still && self.bus.access_moves_lines(addr)
    }

    fn load_moves_lines(&self, addr: u32) -> bool {
        !self.still && self.bus.load_moves_lines(addr)
    }

    fn code_version(&self) -> Option<u64> {
        self.version
    }

    fn fetch_held(&self) -> bool {
        self.bus.fetch_held()
    }
}
