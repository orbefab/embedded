use std::path::{Path, PathBuf};
use std::time::Instant;

use orbe_microchip::{Cpu, FuseBytes, Image, PIN_QUEUE_CAP, parse_hex};

/// CPU cycles in one simulated second.
///
/// `Cpu::new` runs at 16 MHz, and [`PART_FUSES`] leaves `CLKPS` at 1, so the
/// default run and the milliseconds-per-second figure use the same clock.
const CPU_HZ: u64 = 16_000_000;
const DEFAULT_CYCLES: u64 = CPU_HZ;
const RUNS: usize = 5;
const _: () = assert!(RUNS % 2 == 1);

/// `parts/atmega328p-tqfp32/part.jsonc` `model.fuses`. That part is the chip on `nano-board`.
const PART_FUSES: FuseBytes = FuseBytes {
    low: 0xFF,
    high: 0xDF,
    extended: 0xFD,
};

const DEFAULT_HEX: &str = "blocks/nano-led-fade/firmware/fade.hex";

struct Sample {
    secs: f64,
    edges: u64,
    cycles: u64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        usage(0);
    }
    if args.len() > 2 {
        eprintln!("throughput: at most two arguments, a hex path and a cycle count");
        usage(2);
    }
    let mut args = args.into_iter();
    let hex_arg = args.next();
    let cycles = match args.next() {
        Some(text) => match text.parse::<u64>() {
            Ok(cycles) if cycles > 0 => cycles,
            _ => {
                eprintln!("throughput: the cycle count `{text}` is not a whole number above 0");
                usage(2);
            }
        },
        None => DEFAULT_CYCLES,
    };
    let (label, path) = match hex_arg {
        Some(text) => (text.clone(), PathBuf::from(&text)),
        None => (DEFAULT_HEX.to_string(), default_hex_path()),
    };
    let image = read_image(&path);
    let _discarded_warmup = time_run(&image, cycles);
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        samples.push(time_run(&image, cycles));
    }
    samples.sort_by(|left, right| ms_per_sim_s(left).total_cmp(&ms_per_sim_s(right)));
    let median = &samples[RUNS / 2];
    let min_ms = ms_per_sim_s(&samples[0]);
    let max_ms = ms_per_sim_s(&samples[RUNS - 1]);
    let median_ms = ms_per_sim_s(median);
    let mhz = median.cycles as f64 / median.secs / 1e6;
    println!(
        "{label}: {} cycles, median {median_ms:.1} ms per simulated second, \
         range {min_ms:.1} to {max_ms:.1}, {mhz:.0} MHz, \
         {} pin edges on the median run",
        median.cycles, median.edges
    );
}

fn default_hex_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(DEFAULT_HEX)
}

fn read_image(path: &Path) -> Image {
    let text = std::fs::read_to_string(path).unwrap_or_else(|err| {
        die(&format!("cannot read {}: {err}", path.display()));
    });
    parse_hex(&text).unwrap_or_else(|err| die(&format!("{}: {err}", path.display())))
}

fn prepare(image: &Image) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.set_fuses(PART_FUSES.low, PART_FUSES.high, PART_FUSES.extended);
    if let Err(err) = cpu.load_image(image) {
        die(&err.to_string());
    }
    cpu
}

#[expect(clippy::disallowed_methods, reason = "the clock is the measurement")]
fn time_run(image: &Image, cycles: u64) -> Sample {
    let mut cpu = prepare(image);
    let mut edges = 0u64;
    let start = Instant::now();
    while cpu.cycles() < cycles {
        let at = cpu.cycles();
        cpu.run_until((at + PIN_QUEUE_CAP as u64).min(cycles));
        if cpu.cycles() == at {
            let why = match cpu.unknown() {
                Some(word) => format!("unknown opcode {word:#06x}"),
                None => "core stopped".to_string(),
            };
            die(&format!("{why} at cycle {at}"));
        }
        while cpu.take_pin().is_some() {
            edges += 1;
        }
    }
    let secs = start.elapsed().as_secs_f64();
    if let Some(word) = cpu.unknown() {
        die(&format!(
            "unknown opcode {word:#06x} at cycle {}",
            cpu.cycles()
        ));
    }
    Sample {
        secs,
        edges,
        cycles: cpu.cycles(),
    }
}

fn ms_per_sim_s(sample: &Sample) -> f64 {
    sample.secs * 1000.0 * CPU_HZ as f64 / sample.cycles as f64
}

fn usage(code: i32) -> ! {
    eprintln!("usage: throughput [hex] [cycles]");
    eprintln!("  hex defaults to {DEFAULT_HEX}, resolved from the repo root");
    eprintln!("  cycles defaults to {DEFAULT_CYCLES}, one second at {CPU_HZ} Hz");
    std::process::exit(code);
}

fn die(message: &str) -> ! {
    eprintln!("throughput: {message}");
    std::process::exit(1);
}
