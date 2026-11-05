use std::time::Instant;

use sim_avr::{Cpu, PIN_QUEUE_CAP, parse_hex};

const CYCLES: u64 = 500_000_000;

#[allow(clippy::disallowed_methods)]
fn main() {
    let text = include_str!("../tests/timing/hex/t0ovf_1.hex");
    let mut cpu = Cpu::new();
    cpu.load_image(&parse_hex(text).unwrap()).unwrap();
    let mut edges = 0u64;
    let start = Instant::now();
    while cpu.cycles() < CYCLES {
        cpu.run_until((cpu.cycles() + PIN_QUEUE_CAP as u64).min(CYCLES));
        while cpu.take_pin().is_some() {
            edges += 1;
        }
    }
    let secs = start.elapsed().as_secs_f64();
    let mhz = cpu.cycles() as f64 / secs / 1e6;
    println!(
        "{} cycles, {edges} pin edges, {secs:.2} s, {mhz:.0} MHz",
        cpu.cycles()
    );
}
