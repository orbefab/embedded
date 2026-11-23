//! S3 instruction behaviour that the probe fixtures showed, run on a flat bus.

use orbe_xtensa::{Cause, Config, Core, Cycle, Esp32s3, Sr, Stop, Width};

use crate::common::{Fixture, bytes_of};
use crate::core::{CODE, Flat, RAM, core_at};

const WOE: u32 = 1 << 18;

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

/// An aligned byte load from an address the bus puts on the instruction bus is cause 3, before
/// the bus sees it.
#[test]
fn a_byte_load_on_the_instruction_bus_is_cause_3() {
    let l8ui = [0x22u8, 0x03, 0x00];
    let decoded = Esp32s3::decode(&l8ui, CODE).unwrap();
    assert_eq!(Esp32s3::render(&decoded), "l8ui\ta2, a3, 0");
    for addr in [0x4010_0000, 0x4024_0000] {
        let mut bus = Flat::new("220300");
        bus.instruction = addr..addr + 4;
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
