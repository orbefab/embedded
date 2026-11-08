//! The S3 probe images, run on `Core<Esp32s3>`, against the fixtures the S3 board printed.

use orbe_xtensa::{Cause, Config, Core, Cycle, Decoded, Esp32s3, Missing, Op, Sr, Stop, Width};

use crate::common::board::bytes_of;
use crate::common::elf::Elf;
use crate::common::s3board::{run_answering, run_probe};
use crate::common::{Fixture, first_difference, read_fixture, sha256_hex};
use crate::core::{CODE, Flat, RAM, core_at};

const LIMIT: Cycle = Cycle(20_000_000);
const WOE: u32 = 1 << 18;
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

fn require_ran(probe: &str, run: &crate::common::s3board::Run) {
    if let Stop::Unimplemented(missing) = run.stop {
        panic!("{probe} stopped at {missing}, pc {:#010x}", run.core.pc());
    }
}

/// `masked_m100` reads `ffffffa4` after `isync` and 8 `nop`, one less than 9 counts from -100.
#[test]
fn rsr_icount_reads_one_count_behind_while_level_15_is_counting() {
    let mut bus = Flat::new(&format!("002000{}20ec03", "f02000".repeat(8)));
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE | 6);
    core.set_special(Sr::IcountLevel, 15);
    core.set_special(Sr::Icount, -100i32 as u32);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Bus);
    assert_eq!(core.a(2), 0xffff_ffa4, "rsr.icount");
}

/// `ICOUNT` at -1 with `ICOUNTLEVEL` 15 keeps taking the debug exception, so the vector's
/// instruction never runs. The hang fixtures' samples are all `vecbase + 0x280`.
#[test]
fn icount_at_minus_one_under_level_15_stays_at_the_debug_vector() {
    for name in ["s3-icounthang-masked.txt", "s3-icounthang-open.txt"] {
        let fixture = Fixture::read(name);
        let mut vecbase = None;
        let mut pcs = Vec::new();
        for line in fixture.body.lines() {
            if let Some(value) = line.strip_prefix("vecbase ") {
                vecbase = Some(u32::from_str_radix(value, 16).unwrap());
            }
            if let Some(rest) = line.strip_prefix("sample=") {
                let pc = rest.split_whitespace().nth(1).unwrap();
                pcs.push(u32::from_str_radix(pc.strip_prefix("pc=").unwrap(), 16).unwrap());
            }
        }
        let vecbase = vecbase.unwrap();
        assert!(pcs.iter().all(|pc| *pc == vecbase + 0x280), "{name}");
    }

    for (label, intlevel) in [("masked", 6u32), ("open", 0)] {
        let mut bus = Flat::new(&format!("002000{}", "f02000".repeat(16)));
        let mut core: Core<Esp32s3> = core_at(CODE);
        core.set_special(Sr::Ps, WOE | intlevel);
        core.set_special(Sr::Vecbase, CODE);
        core.set_special(Sr::IcountLevel, 15);
        core.set_special(Sr::Icount, -4i32 as u32);
        let stop = core.run_until(&mut bus, Cycle(400));
        assert_eq!(stop, Stop::Limit, "{label}");
        assert_eq!(core.pc(), CODE + 0x280, "{label}");
    }
}

#[test]
fn icounthold_matches_the_s3_board() {
    let (fixture, elf) = image("icounthold");
    let run = run_probe(&elf, "icounthold", LIMIT, &[]);
    require_ran("icounthold", &run);
    if let Some((line, have, want)) = first_difference(&run.uart, &fixture.body) {
        panic!("icounthold line {line}: sim printed {have:?}, the board printed {want:?}");
    }
}

fn assert_matches(probe: &str) {
    let (fixture, elf) = image(probe);
    let run = run_probe(&elf, probe, LIMIT, &[]);
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
    let run = run_answering(&elf, "cop", LIMIT, &[], |core, stop| {
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
    let run = run_probe(&elf, "unaligned", LIMIT, &[]);
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
    let run = run_probe(&elf, "ccount", LIMIT, &[]);
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

/// Saving and restoring `ICOUNT` through `special` keeps the count while level 15 is counting.
#[test]
fn icount_special_round_trip_keeps_the_count() {
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::IcountLevel, 15);
    let written = -100i32 as u32;
    core.set_special(Sr::Icount, written);
    let saved = core.special(Sr::Icount);
    core.set_special(Sr::Icount, 0);
    core.set_special(Sr::Icount, saved);
    assert_eq!(core.special(Sr::Icount), written);
}

/// `ICOUNT` at -1 with `ICOUNTLEVEL` 15 does not take debug from level 7, and the level stays.
#[test]
fn icount_at_minus_one_at_level_7_runs_the_instruction() {
    let mut bus = Flat::new("f02000");
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE | 7);
    core.set_special(Sr::Vecbase, CODE);
    core.set_special(Sr::IcountLevel, 15);
    core.set_special(Sr::Icount, u32::MAX);
    assert_eq!(core.run_until(&mut bus, Cycle(10)), Stop::Bus);
    assert_eq!(core.pc(), CODE + 3);
    assert_eq!(core.special(Sr::Ps) & 0xf, 7);
    assert_eq!(core.special(Sr::Epc6), 0);
    assert_eq!(core.special(Sr::Icount), 0);
}

/// `xsr.icount` while level 15 is counting returns one behind and keeps the written count.
#[test]
fn xsr_icount_returns_one_behind_and_keeps_the_written_count() {
    let mut bus = Flat::new("20ec61");
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::IcountLevel, 15);
    core.set_special(Sr::Icount, 10);
    core.set_a(2, 5);
    assert_eq!(core.run_until(&mut bus, Cycle(10)), Stop::Bus);
    assert_eq!(core.a(2), 9);
    assert_eq!(core.special(Sr::Icount), 5);
}

/// Significant bits pick the follower. The three timed magnitudes stay put. 8 costs 6 and
/// `0x20000` costs 11. A shorter dividend does not cost more than a longer one.
#[test]
fn divide_follower_does_not_decrease_as_the_dividend_grows() {
    let samples = [
        0,
        1,
        7,
        8,
        0xff,
        0x1_0000,
        0x1_0001,
        0x7fff_fffe,
        0x7fff_ffff,
        0x8000_0000,
    ];
    let mut previous = 0;
    for magnitude in samples {
        let cost = Esp32s3::divide_cycles(true, false, magnitude);
        assert!(
            cost >= previous,
            "{magnitude:#x} costs {cost}, after {previous}"
        );
        previous = cost;
    }
    assert_eq!(Esp32s3::divide_cycles(true, false, 7), 1);
    assert_eq!(Esp32s3::divide_cycles(true, false, 8), 6);
    assert_eq!(Esp32s3::divide_cycles(true, false, 0x1_0000), 6);
    assert_eq!(Esp32s3::divide_cycles(true, false, 0x2_0000), 11);
    assert_eq!(Esp32s3::divide_cycles(true, false, 0x7fff_ffff), 11);
}

/// Two `quos a2, a3, a4`. The second starts on a word, so its cycles are the follower cost.
fn quos_follower(dividend: u32) -> (u64, u32) {
    let quos = [0x40u8, 0x23, 0xd2];
    let decoded = Esp32s3::decode(&quos, CODE).unwrap();
    assert_eq!(Esp32s3::render(&decoded), "quos\ta2, a3, a4");
    let code = [0x00, quos[0], quos[1], quos[2], quos[0], quos[1], quos[2]];
    let hex: String = code.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut bus = Flat::new(&hex);
    let mut core: Core<Esp32s3> = core_at(CODE + 1);
    core.set_special(Sr::Ps, WOE);
    core.set_a(3, dividend);
    core.set_a(4, 3);
    assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit);
    assert_eq!(core.cycle(), Cycle(16));
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Bus);
    (core.cycle().0 - 16, core.a(2))
}

/// `-0x10000` takes the same follower as `0x10000`. The raw bits would take the 32-bit cost.
#[test]
fn a_negative_dividend_uses_its_absolute_value() {
    let (positive, positive_q) = quos_follower(0x1_0000);
    let (negative, negative_q) = quos_follower(-0x10000i32 as u32);
    assert_eq!(positive, 6);
    assert_eq!(negative, positive);
    assert_eq!(positive_q, 0x5555);
    assert_eq!(negative_q, 0xffff_aaab);
}

/// The wide `s32i`, then `memw`, is 5 cycles. A narrow store is a different encoding.
#[test]
fn wide_s32i_then_memw_costs_five() {
    let s32i = [0x22u8, 0x63, 0x00];
    let memw = [0xc0u8, 0x20, 0x00];
    let decoded = Esp32s3::decode(&s32i, CODE).unwrap();
    assert_eq!(Esp32s3::render(&decoded), "s32i\ta2, a3, 0");
    let code = [s32i[0], s32i[1], s32i[2], memw[0], memw[1], memw[2]];
    let hex: String = code.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut bus = Flat::new(&hex);
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_a(2, 0x11);
    core.set_a(3, RAM);
    assert_eq!(core.run_until(&mut bus, Cycle(20)), Stop::Bus);
    assert_eq!(core.cycle(), Cycle(5));
    assert_eq!(bus.ram.read(RAM, Width::Word), 0x11);
}

/// An FPU store, a `q` store and a `s32c1i` that stores each count as a store before `memw`, so
/// the pair costs 5, as `s32i` then `memw` does. A `s32c1i` whose compare fails stores nothing,
/// and the pair costs 2. No board row times these.
#[test]
fn other_stores_then_memw_cost_as_s32i_does() {
    for (hex, text, scompare1, cycles) in [
        ("134300", "ssi\tf1, a3, 0", 0, 5),
        ("34e0cd", "st.qr\tq1, a3, 0", 0, 5),
        ("22e300", "s32c1i\ta2, a3, 0", 0, 5),
        ("22e300", "s32c1i\ta2, a3, 0", 1, 2),
    ] {
        let mut bus = Flat::new(&format!("{hex}c02000"));
        let decoded = Esp32s3::decode(&bytes_of(hex), CODE).unwrap();
        assert_eq!(Esp32s3::render(&decoded), text);
        let mut core: Core<Esp32s3> = core_at(CODE);
        core.set_special(Sr::Ps, WOE);
        core.set_special(Sr::CpEnable, 1 | 1 << 3);
        core.set_special(Sr::Scompare1, scompare1);
        core.set_a(2, 0x11);
        core.set_a(3, RAM);
        assert_eq!(core.run_until(&mut bus, Cycle(20)), Stop::Bus, "{text}");
        assert_eq!(
            core.cycle(),
            Cycle(cycles),
            "{text} with SCOMPARE1 {scompare1}"
        );
    }
}

fn placed(parts: &[(usize, &[u8])]) -> String {
    let len = parts
        .iter()
        .map(|(at, bytes)| at + bytes.len())
        .max()
        .unwrap();
    let mut code = vec![0u8; len];
    for (at, bytes) in parts {
        code[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    code.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `0x40370000` and `0x40040000` sit in the instruction windows and outside the two probed
/// addresses. An aligned byte load there is cause 3. Dropping either address from the window
/// lets the load fault as prohibited instead.
#[test]
fn instruction_windows_follow_the_memory_map_past_the_probes() {
    let l8ui = [0x22u8, 0x03, 0x00];
    let decoded = Esp32s3::decode(&l8ui, CODE).unwrap();
    assert_eq!(Esp32s3::render(&decoded), "l8ui\ta2, a3, 0");
    for addr in [0x4037_0000, 0x4004_0000] {
        let mut bus = Flat::new("220300");
        let mut core: Core<Esp32s3> = core_at(CODE);
        core.set_special(Sr::Ps, WOE);
        core.set_a(2, 0x5a);
        core.set_a(3, addr);
        assert_eq!(core.run_until(&mut bus, Cycle(20)), Stop::Bus, "{addr:#x}");
        assert_eq!(
            core.special(Sr::ExcCause),
            Cause::LoadStoreError as u32,
            "{addr:#x}"
        );
        assert_eq!(core.special(Sr::ExcVaddr), addr, "{addr:#x}");
        assert_eq!(core.a(2), 0x5a, "{addr:#x}");
    }
}

/// An exception between a store and `memw`, or between two divides, drops both follower bits.
#[test]
fn a_vector_clears_the_store_and_divide_followers() {
    let s32i = [0x22u8, 0x63, 0x00];
    let memw = [0xc0u8, 0x20, 0x00];
    let syscall = [0x00u8, 0x50, 0x00];
    let quos = [0x40u8, 0x23, 0xd2];

    let mut bus = Flat::new(&placed(&[(0, &s32i), (3, &syscall), (0x300, &memw)]));
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::Vecbase, CODE);
    core.set_a(2, 0x22);
    core.set_a(3, RAM);
    assert_eq!(core.run_until(&mut bus, Cycle(40)), Stop::Bus);
    assert_eq!(core.cycle(), Cycle(6));
    assert_eq!(bus.ram.read(RAM, Width::Word), 0x22);

    let mut bus = Flat::new(&placed(&[(0, &quos), (3, &syscall), (0x300, &quos)]));
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::Vecbase, CODE);
    core.set_a(3, 7);
    core.set_a(4, 3);
    assert_eq!(core.run_until(&mut bus, Cycle(80)), Stop::Bus);
    assert_eq!(core.a(2), 2);
    assert_eq!(core.cycle(), Cycle(37));
}
