//! The classic probe images, run on `Core<Esp32>`, against the fixtures the classic board printed.

use orbe_xtensa::{Config, Cycle, Decoded, Esp32, Missing, Stop};

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

#[test]
fn mac_matches_the_classic_board() {
    let (run, fixture, elf) = run("mac", &[], None);
    assert_transcript("mac", &run, &fixture, &elf);
}

/// The double-precision ops this core does not model. The test answers each op's probe function
/// with 0 and skips that op's rows. The three `wur.f64*` rows write and read plain state, and
/// they are compared.
const F64_OPS: [&str; 11] = [
    "f64iter", "f64rnd", "f64addc", "f64subc", "f64sig", "f64cmpl", "f64cmph", "f64norm",
    "f64sexp", "rf64r", "wf64r",
];

#[test]
fn fpu_matches_the_classic_board_but_the_double_ops() {
    let served: Vec<String> = F64_OPS.iter().map(|op| format!("run_{op}")).collect();
    let served: Vec<&str> = served.iter().map(String::as_str).collect();
    let (run, fixture, elf) = run("fpu", &served, None);
    assert_eq!(run.stop, Stop::Limit, "fpu runs on after END");
    assert_eq!(run.core.pc(), elf.symbol("halt"), "fpu returns to halt");
    let kept = |text: &str| -> String {
        text.split_inclusive('\n')
            .filter(|line| !F64_OPS.contains(&line.split_whitespace().next().unwrap_or("")))
            .collect()
    };
    let (have, want) = (kept(&run.uart), kept(&fixture.body));
    if let Some((line, have, want)) = first_difference(&have, &want) {
        panic!("fpu kept line {line}: sim printed {have:?}, the board printed {want:?}");
    }
    let vectors: usize = fixture.get("vectors").parse().unwrap();
    let ops = fixture
        .body
        .lines()
        .filter(|line| line.starts_with("op "))
        .count();
    let rows = want.lines().filter(|line| line.contains(" y=")).count();
    assert_eq!(
        rows,
        (ops - F64_OPS.len()) * vectors,
        "every kept op prints its rows"
    );
    println!(
        "fpu: {} of {ops} ops match, {rows} rows",
        ops - F64_OPS.len()
    );
}

/// Unanswered, the classic `fpu` image ends its run at the first double-precision op it reaches,
/// the first such op the fixture lists, after printing that op's header.
#[test]
fn a_double_op_ends_the_classic_run_as_unimplemented() {
    let (run, fixture, _) = run("fpu", &[], None);
    let first = fixture
        .body
        .lines()
        .filter_map(|line| line.strip_prefix("op "))
        .map(|rest| rest.split_whitespace().next().unwrap())
        .find(|op| F64_OPS.contains(op))
        .unwrap();
    let Stop::Unimplemented(stop) = run.stop else {
        panic!("fpu stopped with {:?}", run.stop);
    };
    let Missing::Op(op) = stop.missing else {
        panic!("fpu stopped at {stop}");
    };
    assert_eq!(op.name(), first);
    assert!(run.uart.contains(&format!("op {first} ")));
    assert!(!run.uart.contains("END fpu"));
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

/// The classes in the order the probe measures them.
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

#[test]
fn ccount_rows_fall_inside_the_classic_bounds() {
    let (run, fixture, elf) = run("ccount", &[], None);
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
