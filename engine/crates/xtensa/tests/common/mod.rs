pub mod board;
pub mod elf;

use sha2::{Digest, Sha256};
use sim_xtensa::{Bus, Config, Core, Cycle, Stop};

pub fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
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
        if stop != Stop::Limit || to == limit {
            return stop;
        }
    }
}
