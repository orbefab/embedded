//! The classic probe images, run on `Core<Esp32>`, against the fixtures the classic board printed.

use sim_xtensa::{Cycle, Missing, Stop};

use crate::common::board::{Run, run_probe};
use crate::common::elf::Elf;
use crate::common::{Fixture, first_difference, read_fixture, sha256_hex};

const LIMIT: Cycle = Cycle(1_000_000_000);

/// Runs the probe and checks that its fixture was printed by this image.
fn run(probe: &str) -> (Run, Fixture, Elf) {
    let image = read_fixture(&format!("{probe}.elf"));
    let fixture = Fixture::read(&format!("{probe}.txt"));
    assert_eq!(fixture.get("probe"), probe);
    assert_eq!(
        fixture.get("image_sha256"),
        sha256_hex(&image),
        "{probe}.elf is the image the board ran"
    );
    let elf = Elf::parse(&image);
    (run_probe(&elf, probe, LIMIT), fixture, elf)
}

fn assert_transcript(probe: &str, run: &Run, fixture: &Fixture, elf: &Elf) {
    if let Some((line, have, want)) = first_difference(&run.uart, &fixture.body) {
        panic!("{probe} line {line}: sim printed {have:?}, the board printed {want:?}");
    }
    assert_eq!(run.stop, Stop::Limit, "{probe} runs on after END");
    assert_eq!(run.core.pc(), elf.symbol("halt"), "{probe} returns to halt");
    println!(
        "{probe}: {} of {} lines match",
        run.uart.lines().count(),
        fixture.body.lines().count()
    );
}

#[test]
fn alu_matches_the_classic_board() {
    let (run, fixture, elf) = run("alu");
    assert_transcript("alu", &run, &fixture, &elf);
}

#[test]
fn icount_matches_the_classic_board() {
    let (run, fixture, elf) = run("icount");
    assert_transcript("icount", &run, &fixture, &elf);
}

/// The base classes, in the order the probe measures them. `add.s` comes next, and its FPU ops
/// end the run.
const BASE_CLASSES: [&str; 12] = [
    "overhead", "nop", "add", "addi", "sll", "l32i", "s32i", "beqz_nt", "j_t", "mull", "quos",
    "memw",
];

#[test]
fn ccount_base_rows_fall_inside_the_classic_bounds() {
    let (run, fixture, elf) = run("ccount");
    let Stop::Unimplemented(stop) = run.stop else {
        panic!("ccount stops at the first FPU op, not {:?}", run.stop);
    };
    assert!(
        matches!(stop.missing, Missing::Op(_))
            && (elf.symbol("measure_add_s")..elf.symbol("measure_mul_s")).contains(&stop.pc),
        "ccount stops inside measure_add_s, not at {stop}"
    );

    let bounds: Vec<(String, u32, u32, u32)> = fixture
        .body
        .lines()
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().unwrap().to_string();
            let mut field = |key: &str| {
                let part = parts.next().unwrap();
                u32::from_str_radix(part.strip_prefix(key).unwrap(), 16).unwrap()
            };
            let n = field("n=");
            let _median = field("median=");
            (name, n, field("low="), field("high="))
        })
        .collect();

    let rows: Vec<(String, u32, u32, u32)> = run
        .uart
        .lines()
        .filter(|line| line.contains(" delta="))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().unwrap().to_string();
            let mut field = |key: &str| {
                let part = parts.next().unwrap();
                u32::from_str_radix(part.strip_prefix(key).unwrap(), 16).unwrap()
            };
            (name, field("n="), field("delta="), field("e="))
        })
        .collect();
    let names: Vec<&str> = rows.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(names, BASE_CLASSES);

    let overhead = rows[0].2;
    for (name, n, delta, exception) in &rows {
        let net = if name == "overhead" {
            *delta
        } else {
            delta - overhead
        };
        let (_, want_n, low, high) = bounds
            .iter()
            .find(|row| &row.0 == name)
            .unwrap_or_else(|| panic!("the fixture has {name}"));
        assert_eq!(*exception, 0, "{name} raised no exception");
        assert_eq!(n, want_n, "{name} count");
        assert!(
            (*low..=*high).contains(&net),
            "{name} nets {net} cycles, outside {low}..={high}"
        );
        println!("ccount {name}: net {net} in {low}..={high}");
    }
}
