//! Probe images on the Espressif boards. The core does not name these addresses.
//! The classic ESP32 has no chip crate, so its harness is here beside the ESP32-S3 one.

mod common;

use orbe_xtensa::{Config, Core, Cycle, Decoded, Esp32, Esp32s3, Missing, Op, Sr, Stop};

use common::board::{self, ROM_STUB, Run, bytes_of, run_probe};
use common::elf::Elf;
use common::s3board::{self, Run as S3Run};
use common::{Fixture, Pieces, Straight, first_difference, read_fixture, sha256_hex};

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

fn image(probe: &str) -> (Fixture, Elf) {
    let elf_name = format!("s3-{probe}.elf");
    let bytes = read_fixture(&elf_name);
    let fixture = Fixture::read(&format!("s3-{probe}.txt"));
    assert_eq!(fixture.get("probe"), probe);
    assert_eq!(
        fixture.get("image_sha256"),
        sha256_hex(&bytes),
        "{elf_name} is the image the board ran"
    );
    (fixture, Elf::parse(&bytes))
}

fn require_ran(probe: &str, run: &S3Run) {
    if let Stop::Unimplemented(missing) = run.stop {
        panic!("{probe} stopped at {missing}, pc {:#010x}", run.core.pc());
    }
}

const S3_LIMIT: Cycle = Cycle(20_000_000);
/// The model's nets, not the board medians. `rsr` and `wsr` of `epc1` cost one cycle here, and
/// the board spends more. A vector costs 4, and no row isolates that entry. The board medians
/// are 15, 116 and 39.
const GAPS: &[(&str, u32)] = &[("syscall1", 11), ("syscall8", 84), ("handler_ops", 24)];
/// Rows whose median the cycle model reproduces. The fixture's slack is wider than that median.
const EXACT: [&str; 17] = [
    "j_t",
    "quos",
    "quou",
    "rems",
    "remu",
    "quos_big",
    "quos_small",
    "quos_neg",
    "quos_by1",
    "s32i_memw",
    "l32i_memw",
    "loop1",
    "loop2",
    "loop3",
    "loop4",
    "loop8",
    "loop32",
];

#[test]
fn icounthold_matches_the_s3_board() {
    let (fixture, elf) = image("icounthold");
    let run = s3board::run_probe(&elf, "icounthold", S3_LIMIT, &[]);
    require_ran("icounthold", &run);
    if let Some((line, have, want)) = first_difference(&run.uart, &fixture.body) {
        panic!("icounthold line {line}: sim printed {have:?}, the board printed {want:?}");
    }
}

fn assert_matches(probe: &str) {
    let (fixture, elf) = image(probe);
    let run = s3board::run_probe(&elf, probe, S3_LIMIT, &[]);
    require_ran(probe, &run);
    if let Some((line, have, want)) = first_difference(&run.uart, &fixture.body) {
        panic!("{probe} line {line}: sim printed {have:?}, the board printed {want:?}");
    }
    println!("{probe}: {} lines match", fixture.body.lines().count());
}

#[test]
fn fpu_matches_the_s3_board() {
    assert_matches("fpu");
}

#[test]
fn mac_matches_the_s3_board() {
    assert_matches("mac");
}

#[test]
fn salt_matches_the_s3_board() {
    assert_matches("salt");
}

/// `ee.*` ops are not modeled, so the test carries out the probe's `ee.zero.q` when the core stops
/// at it with coprocessor 3 on. The fixture's rows show which `CPENABLE` bit each op needs, and the
/// core's record names the last op that raised the exception.
#[test]
fn coprocessor_exceptions_match_the_s3_board() {
    let (fixture, elf) = image("cop");
    let mut answered = 0;
    let run = s3board::run_answering(&elf, "cop", S3_LIMIT, &[], |core, stop| {
        if stop.missing != Missing::Op(Op::EE_ZERO_Q) {
            return false;
        }
        let Some(Decoded::Insn(insn)) = Esp32s3::decode(elf.bytes_at(stop.pc, 3), stop.pc) else {
            return false;
        };
        assert_eq!(
            core.special(Sr::CpEnable) & 1 << 3,
            1 << 3,
            "ee.zero.q stops only with coprocessor 3 on"
        );
        core.set_q(insn.fields[0], 0);
        core.set_pc(stop.pc + u32::from(insn.len));
        answered += 1;
        true
    });
    require_ran("cop", &run);
    if let Some((line, have, want)) = first_difference(&run.uart, &fixture.body) {
        panic!("cop line {line}: sim printed {have:?}, the board printed {want:?}");
    }
    let runs = fixture
        .body
        .lines()
        .filter(|line| line.starts_with("ee.zero.q ") && line.contains(" e=ff "))
        .count();
    assert_eq!(answered, runs + 1, "the cp=08 row and the zeroing call");
    let last = fixture
        .body
        .lines()
        .rfind(|line| line.contains(" cp=") && !line.contains(" e=ff "))
        .unwrap();
    let name = last.split_whitespace().next().unwrap();
    let raised = run.core.coprocessor_disabled().unwrap();
    assert_eq!(raised.op.name(), name);
    assert_eq!(
        raised.pc,
        elf.symbol(&format!("cop_op_{}", name.replace('.', "_")))
    );
    let cause = u32::from_str_radix(&last[last.find(" e=").unwrap() + 3..][..2], 16).unwrap();
    assert_eq!(32 + u32::from(raised.coprocessor), cause);
}

/// The translation-table line records entries this core does not own, so it is not compared.
#[test]
fn unaligned_regions_match_the_s3_board() {
    let (fixture, elf) = image("unaligned");
    let run = s3board::run_probe(&elf, "unaligned", S3_LIMIT, &[]);
    require_ran("unaligned", &run);
    let have: Vec<_> = run
        .uart
        .lines()
        .filter(|line| !line.contains("mmu_valid"))
        .collect();
    let want: Vec<_> = fixture
        .body
        .lines()
        .filter(|line| !line.contains("mmu_valid"))
        .collect();
    for (index, (have, want)) in have.iter().zip(want.iter()).enumerate() {
        assert_eq!(have, want, "unaligned line {}", index + 1);
    }
    assert_eq!(have.len(), want.len(), "unaligned line count");
}

#[test]
fn ccount_rows_match_the_s3_board() {
    let (fixture, elf) = image("ccount");
    let run = s3board::run_probe(&elf, "ccount", S3_LIMIT, &[]);
    require_ran("ccount", &run);

    let bounds: Vec<(String, u32, u32, u32)> = fixture
        .body
        .lines()
        .filter(|line| line.contains("median="))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().unwrap().to_string();
            let mut field = |key: &str| {
                let part = parts.next().unwrap();
                u32::from_str_radix(part.strip_prefix(key).unwrap(), 16).unwrap()
            };
            let _n = field("n=");
            let median = field("median=");
            (name, median, field("low="), field("high="))
        })
        .collect();

    let rows: Vec<(String, u32, u32)> = run
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
            let _n = field("n=");
            (name, field("delta="), field("e="))
        })
        .collect();

    let overhead = rows.iter().find(|row| row.0 == "overhead").unwrap().1;
    let mut bad = Vec::new();
    for (name, delta, exception) in &rows {
        let net = if name == "overhead" {
            *delta
        } else {
            delta - overhead
        };
        let (_, median, low, high) = bounds
            .iter()
            .find(|row| &row.0 == name)
            .unwrap_or_else(|| panic!("the fixture has {name}"));
        if *exception != 0 {
            bad.push(format!("{name} raised cause {exception:#x}"));
            continue;
        }
        if let Some((_, model)) = GAPS.iter().find(|(row, _)| row == name) {
            if net != *model {
                bad.push(format!("{name} nets {net}, the model net is {model}"));
            }
            continue;
        }
        if EXACT.contains(&name.as_str()) && net != *median {
            bad.push(format!("{name} nets {net}, the fixture median is {median}"));
        } else if !(*low..=*high).contains(&net) {
            bad.push(format!("{name} nets {net}, outside {low}..={high}"));
        }
    }
    let names: Vec<_> = rows.iter().map(|row| row.0.as_str()).collect();
    let want: Vec<_> = bounds.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(names, want, "ccount row order");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each classic probe prints the same text, stops alike, ends in the same state and makes the
/// same loads and stores on the same cycles, plain and kept.
#[test]
fn the_classic_probes_run_the_same_straight() {
    for probe in ["alu", "icount", "window", "loop", "fpu", "mac", "ccount"] {
        let elf = Elf::parse(&read_fixture(&format!("{probe}.elf")));
        let limit = Cycle(1_000_000_000);
        let load = || board::Board::load(&elf, &[]);
        let plain = board::run_probe_on(Straight::plain(load()), &elf, probe, limit, None);
        let kept = board::run_probe_on(load().kept(), &elf, probe, limit, None);
        assert_eq!(kept.uart, plain.uart, "{probe}");
        assert_eq!(kept.stop, plain.stop, "{probe}");
        assert_eq!(kept.core, plain.core, "{probe}");
        assert_eq!(kept.trace, plain.trace, "{probe}");
        #[cfg(not(feature = "poll-skip-check"))]
        assert!(kept.steps < plain.steps, "{probe}");
        println!(
            "{probe}: {} full steps kept, {} plain, to cycle {}",
            kept.steps,
            plain.steps,
            kept.core.cycle().0
        );
    }
}

/// The same for each S3 probe. `cop` carries out the `ee.zero.q` its handler reruns.
#[test]
fn the_s3_probes_run_the_same_straight() {
    let probes = [
        "ccount",
        "cop",
        "fpu",
        "mac",
        "salt",
        "unaligned",
        "icounthold",
    ];
    for probe in probes {
        let elf = Elf::parse(&read_fixture(&format!("s3-{probe}.elf")));
        let limit = Cycle(20_000_000);
        let answer = |core: &mut Core<Esp32s3>, stop: orbe_xtensa::Unimplemented| {
            if stop.missing != Missing::Op(Op::EE_ZERO_Q) {
                return false;
            }
            let Some(Decoded::Insn(insn)) = Esp32s3::decode(elf.bytes_at(stop.pc, 3), stop.pc)
            else {
                return false;
            };
            core.set_q(insn.fields[0], 0);
            core.set_pc(stop.pc + u32::from(insn.len));
            true
        };
        let load = || s3board::Board::load(&elf, &[]);
        let plain = s3board::run_answering_on(Straight::plain(load()), &elf, probe, limit, answer);
        let kept = s3board::run_answering_on(load().kept(), &elf, probe, limit, answer);
        assert_eq!(kept.uart, plain.uart, "{probe}");
        assert_eq!(kept.stop, plain.stop, "{probe}");
        assert_eq!(kept.core, plain.core, "{probe}");
        assert_eq!(kept.trace, plain.trace, "{probe}");
        #[cfg(not(feature = "poll-skip-check"))]
        assert!(kept.steps < plain.steps, "{probe}");
        println!(
            "s3 {probe}: {} full steps kept, {} plain, to cycle {}",
            kept.steps,
            plain.steps,
            kept.core.cycle().0
        );
    }
}
