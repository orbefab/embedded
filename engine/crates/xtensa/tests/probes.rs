//! The classic probe images, run on `Core<Esp32>`, against the fixtures the classic board printed.

use sim_xtensa::{Config, Cycle, Decoded, Esp32, Stop};

use crate::common::board::{ROM_STUB, Run, bytes_of, run_probe};
use crate::common::elf::Elf;
use crate::common::{Fixture, Pieces, first_difference, read_fixture, sha256_hex};

const LIMIT: Cycle = Cycle(1_000_000_000);

/// Runs the probe and checks that its fixture was printed by this image.
fn run(probe: &str, served: &[&str], pieces: Option<Pieces>) -> (Run, Fixture, Elf) {
    let image = read_fixture(&format!("{probe}.elf"));
    let fixture = Fixture::read(&format!("{probe}.txt"));
    assert_eq!(fixture.get("probe"), probe);
    assert_eq!(
        fixture.get("image_sha256"),
        sha256_hex(&image),
        "{probe}.elf is the image the board ran"
    );
    let elf = Elf::parse(&image);
    (run_probe(&elf, probe, LIMIT, served, pieces), fixture, elf)
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
    let (run, fixture, elf) = run("alu", &[], None);
    assert_transcript("alu", &run, &fixture, &elf);
}

#[test]
fn icount_matches_the_classic_board() {
    let (run, fixture, elf) = run("icount", &[], None);
    assert_transcript("icount", &run, &fixture, &elf);
}

#[test]
fn window_matches_the_classic_board() {
    let (run, fixture, elf) = run("window", &[], None);
    assert_transcript("window", &run, &fixture, &elf);
}

#[test]
fn loop_matches_the_classic_board() {
    let (run, fixture, elf) = run("loop", &[], None);
    assert_transcript("loop", &run, &fixture, &elf);
}

/// The `window` and `loop` images cut at random limits print the same text and end in the same
/// state as one run.
#[test]
fn random_limit_splits_of_the_probes_equal_one_run() {
    for probe in ["window", "loop"] {
        let (whole, _, _) = run(probe, &[], None);
        let (split, _, _) = run(probe, &[], Some(Pieces::new(1)));
        assert_eq!(split.uart, whole.uart, "{probe}");
        assert_eq!(split.stop, whole.stop, "{probe}");
        assert_eq!(split.core, whole.core, "{probe}");
        println!(
            "split {probe}: equals one run of {} cycles",
            whole.core.cycle().0
        );
    }
}

/// The classes in the order the probe measures them. The test serves the FPU and MAC16
/// measures, whose ops this core does not model, and their rows print 0.
const CLASSES: [&str; 19] = [
    "overhead",
    "nop",
    "add",
    "addi",
    "sll",
    "l32i",
    "s32i",
    "beqz_nt",
    "j_t",
    "mull",
    "quos",
    "memw",
    "add.s",
    "mul.s",
    "mula.aa.ll",
    "loop1",
    "loop32",
    "call0_ret",
    "call8_retw",
];
const SERVED: [&str; 3] = ["measure_add_s", "measure_mul_s", "measure_mula_aa_ll"];
const SERVED_CLASSES: [&str; 3] = ["add.s", "mul.s", "mula.aa.ll"];

#[test]
fn ccount_rows_fall_inside_the_classic_bounds() {
    let (run, fixture, elf) = run("ccount", &SERVED, None);
    assert_eq!(run.stop, Stop::Limit, "ccount runs on after END");
    assert_eq!(run.core.pc(), elf.symbol("halt"), "ccount returns to halt");

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
    assert_eq!(names, CLASSES);

    let overhead = rows[0].2;
    for (name, n, delta, exception) in &rows {
        if SERVED_CLASSES.contains(&name.as_str()) {
            assert_eq!(*delta, 0, "{name} is served");
            continue;
        }
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

/// The ROM stand-ins' bytes decode to their text.
#[test]
fn the_rom_stub_is_its_text() {
    for (addr, hex, text) in ROM_STUB {
        let bytes = bytes_of(hex);
        let mut at = 0;
        for line in text {
            let decoded = Esp32::decode(&bytes[at..], addr + at as u32).unwrap();
            let Decoded::Insn(insn) = &decoded else {
                panic!("{addr:#010x}+{at} decodes");
            };
            assert_eq!(Esp32::render(&decoded), *line, "{addr:#010x}+{at}");
            at += insn.len as usize;
        }
        assert_eq!(at, bytes.len(), "{addr:#010x} has no bytes past its text");
    }
}
