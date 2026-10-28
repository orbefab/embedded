//! One row per op arm the probe images do not run, on the flat test bus. Each row's state after
//! comes from the op's operation in the Xtensa ISA, not from this core, except where the ISA
//! leaves the result open. There the unaligned stop rows, the undefined `entry` and `retw` stop
//! rows and the `wsr.ccount` row pin this core's choice. The encodings are GNU `as` output, and
//! each row checks that its bytes decode to its text.

use sim_xtensa::{
    Cause, Config, Core, Cycle, Decoded, Esp32, Esp32s3, Missing, Op, Sr, Stop, Unimplemented,
    Width,
};

use crate::core::{CODE, Flat, RAM, core_at};

const END: u32 = RAM + 0x8_0000;
const WOE: u32 = 1 << 18;
const EXCM: u32 = 1 << 4;
const UM: u32 = 1 << 5;
const CALLINC1: u32 = 1 << 16;
const CALLINC3: u32 = 3 << 16;

const NEXT: u32 = CODE + 3;
const TAKEN: u32 = CODE + 16;
const TO: u32 = CODE + 0x40;
const KERNEL: u32 = 0x4000_0300;
const OF4: u32 = 0x4000_0000;
const UF4: u32 = 0x4000_0040;
const OF8: u32 = 0x4000_0080;
const UF8: u32 = 0x4000_00c0;
const OF12: u32 = 0x4000_0100;
const UF12: u32 = 0x4000_0140;

/// `PS.OWB` holding `base`.
const fn owb(base: u32) -> u32 {
    base << 8
}

/// A piece of state: an address register of the current window, a special register, or the
/// word at an address.
#[derive(Clone, Copy, Debug)]
enum Is {
    A(u32, u32),
    S(Sr, u32),
    M(u32, u32),
}

use Is::{A, M, S};

struct Row {
    text: &'static str,
    hex: &'static str,
    before: &'static [Is],
    after: &'static [Is],
    pc: u32,
    stop: Stop,
}

const fn runs(
    text: &'static str,
    hex: &'static str,
    before: &'static [Is],
    after: &'static [Is],
    pc: u32,
) -> Row {
    Row {
        text,
        hex,
        before,
        after,
        pc,
        stop: Stop::Limit,
    }
}

const fn stops(
    text: &'static str,
    hex: &'static str,
    before: &'static [Is],
    after: &'static [Is],
    pc: u32,
    missing: Missing,
) -> Row {
    Row {
        text,
        hex,
        before,
        after,
        pc,
        stop: Stop::Unimplemented(Unimplemented { pc, missing }),
    }
}

/// `80 91 a2 b3 c4 d5 e6 f7` from `RAM`.
const BYTES: [Is; 2] = [M(RAM, 0xb3a2_9180), M(RAM + 4, 0xf7e6_d5c4)];
const ONES: [Is; 2] = [M(RAM, u32::MAX), M(RAM + 4, u32::MAX)];

const L32I: &str = "222300";
const S32I: &str = "226300";
const L16UI: &str = "221300";
const L16SI: &str = "229300";
const S16I: &str = "225300";

const ROWS: &[Row] = &[
    runs(
        "abs\ta2, a3",
        "302160",
        &[A(3, 0xffff_fffb)],
        &[A(2, 5)],
        NEXT,
    ),
    runs(
        "addmi\ta2, a3, 0xffffff00",
        "22d3ff",
        &[A(3, 0x1000)],
        &[A(2, 0xf00)],
        NEXT,
    ),
    runs(
        "addx2\ta2, a3, a4",
        "402390",
        &[A(3, 3), A(4, 10)],
        &[A(2, 16)],
        NEXT,
    ),
    runs(
        "addx4\ta2, a3, a4",
        "4023a0",
        &[A(3, 3), A(4, 10)],
        &[A(2, 22)],
        NEXT,
    ),
    runs(
        "addx8\ta2, a3, a4",
        "4023b0",
        &[A(3, 3), A(4, 10)],
        &[A(2, 34)],
        NEXT,
    ),
    runs(
        "subx2\ta2, a3, a4",
        "4023d0",
        &[A(3, 3), A(4, 10)],
        &[A(2, 0xffff_fffc)],
        NEXT,
    ),
    runs(
        "subx4\ta2, a3, a4",
        "4023e0",
        &[A(3, 3), A(4, 10)],
        &[A(2, 2)],
        NEXT,
    ),
    runs(
        "subx8\ta2, a3, a4",
        "4023f0",
        &[A(3, 3), A(4, 10)],
        &[A(2, 14)],
        NEXT,
    ),
    runs(
        "neg\ta2, a3",
        "302060",
        &[A(3, 7)],
        &[A(2, 0xffff_fff9)],
        NEXT,
    ),
    runs(
        "moveqz\ta2, a3, a4",
        "402383",
        &[A(2, 1), A(3, 9), A(4, 0)],
        &[A(2, 9)],
        NEXT,
    ),
    runs(
        "moveqz\ta2, a3, a4",
        "402383",
        &[A(2, 1), A(3, 9), A(4, 1)],
        &[A(2, 1)],
        NEXT,
    ),
    runs(
        "movnez\ta2, a3, a4",
        "402393",
        &[A(2, 1), A(3, 9), A(4, 5)],
        &[A(2, 9)],
        NEXT,
    ),
    runs(
        "movnez\ta2, a3, a4",
        "402393",
        &[A(2, 1), A(3, 9), A(4, 0)],
        &[A(2, 1)],
        NEXT,
    ),
    runs(
        "movltz\ta2, a3, a4",
        "4023a3",
        &[A(2, 1), A(3, 9), A(4, 1 << 31)],
        &[A(2, 9)],
        NEXT,
    ),
    runs(
        "movltz\ta2, a3, a4",
        "4023a3",
        &[A(2, 1), A(3, 9), A(4, 0)],
        &[A(2, 1)],
        NEXT,
    ),
    runs(
        "movgez\ta2, a3, a4",
        "4023b3",
        &[A(2, 1), A(3, 9), A(4, 0)],
        &[A(2, 9)],
        NEXT,
    ),
    runs(
        "movgez\ta2, a3, a4",
        "4023b3",
        &[A(2, 1), A(3, 9), A(4, u32::MAX)],
        &[A(2, 1)],
        NEXT,
    ),
    runs(
        "srai\ta2, a3, 4",
        "302421",
        &[A(3, 0x8000_0010)],
        &[A(2, 0xf800_0001)],
        NEXT,
    ),
    runs("ssai\t5", "004540", &[], &[S(Sr::Sar, 5)], NEXT),
    runs("ssa8b\ta3", "003340", &[A(3, 1)], &[S(Sr::Sar, 24)], NEXT),
    runs(
        "rsil\ta2, 3",
        "206300",
        &[S(Sr::Ps, WOE)],
        &[A(2, WOE), S(Sr::Ps, WOE | 3)],
        NEXT,
    ),
    // Neither chip has an MMU or an MPU, so the current ring is 0 and user mode is no privilege.
    runs(
        "rsil\ta2, 3",
        "206300",
        &[S(Sr::Ps, WOE | UM)],
        &[A(2, WOE | UM), S(Sr::Ps, WOE | UM | 3)],
        NEXT,
    ),
    runs(
        "rsr.ps\ta2",
        "20e603",
        &[S(Sr::Ps, WOE | UM)],
        &[A(2, WOE | UM)],
        NEXT,
    ),
    runs(
        "rfi\t6",
        "103600",
        &[S(Sr::Ps, WOE | UM), S(Sr::Eps6, WOE | 5), S(Sr::Epc6, TO)],
        &[S(Sr::Ps, WOE | 5)],
        TO,
    ),
    runs(
        "l16si\ta2, a3, 2",
        "229301",
        &[A(3, RAM), M(RAM, 0x8001_1234)],
        &[A(2, 0xffff_8001)],
        NEXT,
    ),
    runs(
        "l16ui\ta2, a3, 2",
        "221301",
        &[A(3, RAM), M(RAM, 0x8001_1234)],
        &[A(2, 0x8001)],
        NEXT,
    ),
    runs(
        "l32ai\ta2, a3, 4",
        "22b301",
        &[A(3, RAM), M(RAM + 4, 0xdead_beef)],
        &[A(2, 0xdead_beef)],
        NEXT,
    ),
    runs(
        "s16i\ta2, a3, 2",
        "225301",
        &[A(2, 0xabcd_1234), A(3, RAM), M(RAM, 0x1111_1111)],
        &[M(RAM, 0x1234_1111)],
        NEXT,
    ),
    runs(
        "s32ri\ta2, a3, 4",
        "22f301",
        &[A(2, 0xdead_beef), A(3, RAM)],
        &[M(RAM + 4, 0xdead_beef)],
        NEXT,
    ),
    stops(
        "l32r\ta2, 0x4036fffc",
        "21ffff",
        &[A(2, 7), S(Sr::Litbase, RAM | 1)],
        &[A(2, 7)],
        CODE,
        Missing::Litbase,
    ),
    // Unaligned loads and stores at each offset in a word, over the bytes in `BYTES` or
    // over `ONES`.
    runs(
        "l32i\ta2, a3, 0",
        L32I,
        &[BYTES[0], BYTES[1], A(3, RAM)],
        &[A(2, 0xb3a2_9180)],
        NEXT,
    ),
    runs(
        "l32i\ta2, a3, 0",
        L32I,
        &[BYTES[0], BYTES[1], A(3, RAM + 1)],
        &[A(2, 0xc4b3_a291)],
        NEXT,
    ),
    runs(
        "l32i\ta2, a3, 0",
        L32I,
        &[BYTES[0], BYTES[1], A(3, RAM + 2)],
        &[A(2, 0xd5c4_b3a2)],
        NEXT,
    ),
    runs(
        "l32i\ta2, a3, 0",
        L32I,
        &[BYTES[0], BYTES[1], A(3, RAM + 3)],
        &[A(2, 0xe6d5_c4b3)],
        NEXT,
    ),
    runs(
        "l16ui\ta2, a3, 0",
        L16UI,
        &[BYTES[0], BYTES[1], A(3, RAM)],
        &[A(2, 0x9180)],
        NEXT,
    ),
    runs(
        "l16ui\ta2, a3, 0",
        L16UI,
        &[BYTES[0], BYTES[1], A(3, RAM + 1)],
        &[A(2, 0xa291)],
        NEXT,
    ),
    runs(
        "l16ui\ta2, a3, 0",
        L16UI,
        &[BYTES[0], BYTES[1], A(3, RAM + 2)],
        &[A(2, 0xb3a2)],
        NEXT,
    ),
    runs(
        "l16ui\ta2, a3, 0",
        L16UI,
        &[BYTES[0], BYTES[1], A(3, RAM + 3)],
        &[A(2, 0xc4b3)],
        NEXT,
    ),
    runs(
        "l16si\ta2, a3, 0",
        L16SI,
        &[BYTES[0], BYTES[1], A(3, RAM)],
        &[A(2, 0xffff_9180)],
        NEXT,
    ),
    runs(
        "l16si\ta2, a3, 0",
        L16SI,
        &[BYTES[0], BYTES[1], A(3, RAM + 1)],
        &[A(2, 0xffff_a291)],
        NEXT,
    ),
    runs(
        "l16si\ta2, a3, 0",
        L16SI,
        &[BYTES[0], BYTES[1], A(3, RAM + 2)],
        &[A(2, 0xffff_b3a2)],
        NEXT,
    ),
    runs(
        "l16si\ta2, a3, 0",
        L16SI,
        &[BYTES[0], BYTES[1], A(3, RAM + 3)],
        &[A(2, 0xffff_c4b3)],
        NEXT,
    ),
    runs(
        "s32i\ta2, a3, 0",
        S32I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM)],
        &[M(RAM, 0x4433_2211), M(RAM + 4, u32::MAX)],
        NEXT,
    ),
    runs(
        "s32i\ta2, a3, 0",
        S32I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM + 1)],
        &[M(RAM, 0x3322_11ff), M(RAM + 4, 0xffff_ff44)],
        NEXT,
    ),
    runs(
        "s32i\ta2, a3, 0",
        S32I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM + 2)],
        &[M(RAM, 0x2211_ffff), M(RAM + 4, 0xffff_4433)],
        NEXT,
    ),
    runs(
        "s32i\ta2, a3, 0",
        S32I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM + 3)],
        &[M(RAM, 0x11ff_ffff), M(RAM + 4, 0xff44_3322)],
        NEXT,
    ),
    runs(
        "s16i\ta2, a3, 0",
        S16I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM)],
        &[M(RAM, 0xffff_2211), M(RAM + 4, u32::MAX)],
        NEXT,
    ),
    runs(
        "s16i\ta2, a3, 0",
        S16I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM + 1)],
        &[M(RAM, 0xff22_11ff), M(RAM + 4, u32::MAX)],
        NEXT,
    ),
    runs(
        "s16i\ta2, a3, 0",
        S16I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM + 2)],
        &[M(RAM, 0x2211_ffff), M(RAM + 4, u32::MAX)],
        NEXT,
    ),
    runs(
        "s16i\ta2, a3, 0",
        S16I,
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM + 3)],
        &[M(RAM, 0x11ff_ffff), M(RAM + 4, 0xffff_ff22)],
        NEXT,
    ),
    // A word across a 4 KB page.
    runs(
        "l32i\ta2, a3, 0",
        L32I,
        &[
            M(RAM + 0xffc, 0xbbaa_0000),
            M(RAM + 0x1000, 0x0000_ddcc),
            A(3, RAM + 0xffe),
        ],
        &[A(2, 0xddcc_bbaa)],
        NEXT,
    ),
    runs(
        "s32i\ta2, a3, 0",
        S32I,
        &[A(2, 0x4433_2211), A(3, RAM + 0xffe)],
        &[M(RAM + 0xffc, 0x2211_0000), M(RAM + 0x1000, 0x0000_4433)],
        NEXT,
    ),
    // An aligned access that faults raises the exception with its address in `EXCVADDR`.
    runs(
        "l32i\ta2, a3, 0",
        L32I,
        &[A(2, 7), A(3, END)],
        &[
            A(2, 7),
            S(Sr::ExcCause, 28),
            S(Sr::ExcVaddr, END),
            S(Sr::Epc1, CODE),
            S(Sr::Ps, WOE | EXCM),
        ],
        KERNEL,
    ),
    runs(
        "s32i\ta2, a3, 0",
        S32I,
        &[A(2, 0x4433_2211), A(3, RAM - 4)],
        &[S(Sr::ExcCause, 29), S(Sr::ExcVaddr, RAM - 4), M(RAM, 0)],
        KERNEL,
    ),
    // An unaligned one stops when any part faults. The ISA lets the chip raise
    // `LoadStoreAlignmentCause` instead and gives no `EXCVADDR` for a later part. A store's
    // earlier part stays written.
    stops(
        "l32i\ta2, a3, 0",
        L32I,
        &[A(2, 7), A(3, END - 2)],
        &[A(2, 7), S(Sr::ExcVaddr, 0), S(Sr::Ps, WOE)],
        CODE,
        Missing::Unaligned(Op::L32I),
    ),
    stops(
        "l32i\ta2, a3, 0",
        L32I,
        &[A(2, 7), A(3, RAM - 2)],
        &[A(2, 7), S(Sr::ExcVaddr, 0), S(Sr::Ps, WOE)],
        CODE,
        Missing::Unaligned(Op::L32I),
    ),
    stops(
        "s32i\ta2, a3, 0",
        S32I,
        &[A(2, 0x4433_2211), A(3, RAM - 2)],
        &[S(Sr::ExcVaddr, 0), M(RAM, 0)],
        CODE,
        Missing::Unaligned(Op::S32I),
    ),
    stops(
        "s32i\ta2, a3, 0",
        S32I,
        &[M(END - 4, u32::MAX), A(2, 0x4433_2211), A(3, END - 2)],
        &[M(END - 4, 0x2211_ffff), S(Sr::ExcVaddr, 0)],
        CODE,
        Missing::Unaligned(Op::S32I),
    ),
    runs(
        "ball\ta2, a3, 0x40370010",
        "37420c",
        &[A(2, 0xff), A(3, 0x0f)],
        &[],
        TAKEN,
    ),
    runs(
        "ball\ta2, a3, 0x40370010",
        "37420c",
        &[A(2, 0xf0), A(3, 0x0f)],
        &[],
        NEXT,
    ),
    runs(
        "bnall\ta2, a3, 0x40370010",
        "37c20c",
        &[A(2, 0xf0), A(3, 0x0f)],
        &[],
        TAKEN,
    ),
    runs(
        "bnall\ta2, a3, 0x40370010",
        "37c20c",
        &[A(2, 0xff), A(3, 0x0f)],
        &[],
        NEXT,
    ),
    runs(
        "bany\ta2, a3, 0x40370010",
        "37820c",
        &[A(2, 0x10), A(3, 0x30)],
        &[],
        TAKEN,
    ),
    runs(
        "bany\ta2, a3, 0x40370010",
        "37820c",
        &[A(2, 0x10), A(3, 0x20)],
        &[],
        NEXT,
    ),
    runs(
        "bnone\ta2, a3, 0x40370010",
        "37020c",
        &[A(2, 0x10), A(3, 0x20)],
        &[],
        TAKEN,
    ),
    runs(
        "bnone\ta2, a3, 0x40370010",
        "37020c",
        &[A(2, 0x10), A(3, 0x30)],
        &[],
        NEXT,
    ),
    runs(
        "bbc\ta2, a3, 0x40370010",
        "37520c",
        &[A(2, 0xffff_fffd), A(3, 1)],
        &[],
        TAKEN,
    ),
    runs(
        "bbc\ta2, a3, 0x40370010",
        "37520c",
        &[A(2, 2), A(3, 33)],
        &[],
        NEXT,
    ),
    runs(
        "bbs\ta2, a3, 0x40370010",
        "37d20c",
        &[A(2, 2), A(3, 33)],
        &[],
        TAKEN,
    ),
    runs(
        "bbs\ta2, a3, 0x40370010",
        "37d20c",
        &[A(2, 1), A(3, 33)],
        &[],
        NEXT,
    ),
    runs(
        "bbsi\ta2, 31, 0x40370010",
        "f7f20c",
        &[A(2, 1 << 31)],
        &[],
        TAKEN,
    ),
    runs(
        "bbsi\ta2, 31, 0x40370010",
        "f7f20c",
        &[A(2, 0x7fff_ffff)],
        &[],
        NEXT,
    ),
    runs("bgei\ta2, -1, 0x40370010", "e6020c", &[A(2, 0)], &[], TAKEN),
    runs(
        "bgei\ta2, -1, 0x40370010",
        "e6020c",
        &[A(2, 0xffff_fffe)],
        &[],
        NEXT,
    ),
    runs(
        "blti\ta2, 1, 0x40370010",
        "a6120c",
        &[A(2, u32::MAX)],
        &[],
        TAKEN,
    ),
    runs("blti\ta2, 1, 0x40370010", "a6120c", &[A(2, 1)], &[], NEXT),
    runs(
        "bgeu\ta2, a3, 0x40370010",
        "37b20c",
        &[A(2, 1 << 31), A(3, 1)],
        &[],
        TAKEN,
    ),
    runs(
        "bgeu\ta2, a3, 0x40370010",
        "37b20c",
        &[A(2, 1), A(3, 1 << 31)],
        &[],
        NEXT,
    ),
    runs(
        "bltu\ta2, a3, 0x40370010",
        "37320c",
        &[A(2, 1), A(3, 1 << 31)],
        &[],
        TAKEN,
    ),
    runs(
        "bltu\ta2, a3, 0x40370010",
        "37320c",
        &[A(2, 1 << 31), A(3, 1)],
        &[],
        NEXT,
    ),
    runs("bgez\ta2, 0x40370010", "d6c200", &[A(2, 0)], &[], TAKEN),
    runs(
        "bgez\ta2, 0x40370010",
        "d6c200",
        &[A(2, 1 << 31)],
        &[],
        NEXT,
    ),
    runs(
        "bltz\ta2, 0x40370010",
        "96c200",
        &[A(2, 1 << 31)],
        &[],
        TAKEN,
    ),
    runs("bltz\ta2, 0x40370010", "96c200", &[A(2, 0)], &[], NEXT),
    runs(
        "blt\ta2, a3, 0x40370010",
        "37220c",
        &[A(2, u32::MAX), A(3, 0)],
        &[],
        TAKEN,
    ),
    runs(
        "blt\ta2, a3, 0x40370010",
        "37220c",
        &[A(2, 0), A(3, u32::MAX)],
        &[],
        NEXT,
    ),
    runs("callx0\ta2", "c00200", &[A(2, TO)], &[A(0, NEXT)], TO),
    runs(
        "call4\t0x40370010",
        "d50000",
        &[],
        &[A(4, 0x4037_0003), S(Sr::Ps, WOE | CALLINC1)],
        TAKEN,
    ),
    runs(
        "call12\t0x40370010",
        "f50000",
        &[],
        &[A(12, 0xc037_0003), S(Sr::Ps, WOE | CALLINC3)],
        TAKEN,
    ),
    runs(
        "callx4\ta2",
        "d00200",
        &[A(2, TO)],
        &[A(4, 0x4037_0003), S(Sr::Ps, WOE | CALLINC1)],
        TO,
    ),
    runs(
        "callx12\ta2",
        "f00200",
        &[A(2, TO)],
        &[A(12, 0xc037_0003), S(Sr::Ps, WOE | CALLINC3)],
        TO,
    ),
    runs("jx\ta2", "a00200", &[A(2, TO)], &[], TO),
    runs("ret.n", "0df0", &[A(0, TO)], &[], TO),
    runs(
        "rfde",
        "003200",
        &[S(Sr::Ps, WOE | EXCM), S(Sr::Depc, TO)],
        &[S(Sr::Ps, WOE)],
        TO,
    ),
    runs(
        "syscall",
        "005000",
        &[],
        &[S(Sr::ExcCause, 1), S(Sr::Epc1, CODE), S(Sr::Ps, WOE | EXCM)],
        KERNEL,
    ),
    runs(
        "break.n\t1",
        "2df1",
        &[],
        &[
            S(Sr::DebugCause, 1 << 4),
            S(Sr::Epc6, CODE),
            S(Sr::Eps6, WOE),
            S(Sr::Ps, WOE | EXCM | 6),
        ],
        0x4000_0280,
    ),
    runs(
        "ill.n",
        "6df0",
        &[S(Sr::Ps, WOE | UM)],
        &[
            S(Sr::ExcCause, 0),
            S(Sr::Epc1, CODE),
            S(Sr::Ps, WOE | UM | EXCM),
        ],
        0x4000_0340,
    ),
    // With `PS.EXCM` set, the double exception: `DEPC` takes the PC and `EPC1` and `PS` stay.
    runs(
        "ill",
        "000000",
        &[
            S(Sr::Ps, WOE | EXCM),
            S(Sr::Epc1, 0x1234),
            S(Sr::ExcCause, 5),
        ],
        &[
            S(Sr::ExcCause, 0),
            S(Sr::Depc, CODE),
            S(Sr::Epc1, 0x1234),
            S(Sr::Ps, WOE | EXCM),
        ],
        0x4000_03c0,
    ),
    runs(
        "xsr.icount\ta2",
        "20ec61",
        &[A(2, 5), S(Sr::Icount, 7)],
        &[A(2, 7), S(Sr::Icount, 5)],
        NEXT,
    ),
    runs(
        "entry\ta1, 32",
        "364100",
        &[S(Sr::Ps, WOE | CALLINC1), A(1, 0x3fc9_0000)],
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b11),
            A(1, 0x3fc8_ffe0),
        ],
        NEXT,
    ),
    // With `CALLINC` 0 the ISA checks no frame, and `entry` writes the same window's `a1`.
    runs(
        "entry\ta1, 32",
        "364100",
        &[A(1, 0x3fc9_0000)],
        &[
            S(Sr::WindowBase, 0),
            S(Sr::WindowStart, 1),
            A(1, 0x3fc8_ffe0),
        ],
        NEXT,
    ),
    // `entry`'s check covers the quarters up to `CALLINC`. Frame 1 is in them, and no frame lies
    // in the two quarters above it, so the handler spills twelve registers.
    runs(
        "entry\ta1, 32",
        "364100",
        &[
            S(Sr::Ps, WOE | CALLINC1),
            S(Sr::WindowStart, 0b11),
            A(1, 0x3fc9_0000),
        ],
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b11),
            S(Sr::Ps, WOE | CALLINC1 | EXCM),
            S(Sr::Epc1, CODE),
        ],
        OF12,
    ),
    runs(
        "entry\ta1, 32",
        "364100",
        &[
            S(Sr::Ps, WOE | CALLINC3),
            S(Sr::WindowStart, 0b1_1001),
            A(1, 0x3fc9_0000),
        ],
        &[
            S(Sr::WindowBase, 3),
            S(Sr::WindowStart, 0b1_1001),
            S(Sr::Ps, WOE | CALLINC3 | EXCM),
            S(Sr::Epc1, CODE),
        ],
        OF4,
    ),
    // A frame in the callee's top quarter waits for the callee's first reference to it.
    runs(
        "entry\ta1, 32",
        "364100",
        &[
            S(Sr::Ps, WOE | CALLINC1),
            S(Sr::WindowStart, 0b1_0001),
            A(1, 0x3fc9_0000),
        ],
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b1_0011),
            A(1, 0x3fc8_ffe0),
        ],
        NEXT,
    ),
    stops(
        "entry\ta1, 32",
        "364100",
        &[S(Sr::Ps, WOE | EXCM | CALLINC1), A(1, 0x3fc9_0000)],
        &[S(Sr::WindowBase, 0), A(1, 0x3fc9_0000)],
        CODE,
        Missing::Op(Op::ENTRY),
    ),
    // A reference to `a4` checks quarter 1. The next frame above the one it spills picks the
    // vector.
    runs(
        "mov.n\ta4, a5",
        "4d05",
        &[S(Sr::WindowStart, 0b111)],
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b111),
            S(Sr::Ps, WOE | EXCM),
            S(Sr::Epc1, CODE),
        ],
        OF4,
    ),
    runs(
        "mov.n\ta4, a5",
        "4d05",
        &[S(Sr::WindowStart, 0b1011)],
        &[S(Sr::WindowBase, 1), S(Sr::Ps, WOE | EXCM)],
        OF8,
    ),
    runs(
        "mov.n\ta4, a5",
        "4d05",
        &[S(Sr::WindowStart, 0b11)],
        &[S(Sr::WindowBase, 1), S(Sr::Ps, WOE | EXCM)],
        OF12,
    ),
    // `PS.OWB` keeps the old `WindowBase`.
    runs(
        "mov.n\ta8, a2",
        "8d02",
        &[
            S(Sr::WindowBase, 5),
            S(Sr::WindowStart, 1 << 5 | 1 << 7 | 1 << 9),
        ],
        &[
            S(Sr::WindowBase, 7),
            S(Sr::Ps, WOE | EXCM | owb(5)),
            S(Sr::Epc1, CODE),
        ],
        OF8,
    ),
    // A reference to `a8` checks quarters 1 and 2, so a frame at quarter 3 waits.
    runs(
        "mov.n\ta8, a2",
        "8d02",
        &[S(Sr::WindowStart, 0b1001), A(2, 9)],
        &[S(Sr::WindowBase, 0), A(8, 9)],
        CODE + 2,
    ),
    runs(
        "mov.n\ta12, a2",
        "cd02",
        &[S(Sr::WindowStart, 0b1001)],
        &[S(Sr::WindowBase, 3), S(Sr::Ps, WOE | EXCM)],
        OF12,
    ),
    // The check wraps past window 15.
    runs(
        "mov.n\ta8, a2",
        "8d02",
        &[S(Sr::WindowBase, 14), S(Sr::WindowStart, 1 << 14 | 1)],
        &[S(Sr::WindowBase, 0), S(Sr::Ps, WOE | EXCM | owb(14))],
        OF12,
    ),
    // `CWOE` is 0 with `PS.EXCM` set or `PS.WOE` clear.
    runs(
        "mov.n\ta4, a5",
        "4d05",
        &[S(Sr::Ps, WOE | EXCM), S(Sr::WindowStart, 0b11), A(5, 9)],
        &[S(Sr::WindowBase, 0), A(4, 9)],
        CODE + 2,
    ),
    runs(
        "mov.n\ta4, a5",
        "4d05",
        &[S(Sr::Ps, 0), S(Sr::WindowStart, 0b11), A(5, 9)],
        &[S(Sr::WindowBase, 0), A(4, 9)],
        CODE + 2,
    ),
    // A call checks the quarters up to the register it writes, before it writes it.
    runs(
        "call4\t0x40370010",
        "d50000",
        &[S(Sr::WindowStart, 0b11)],
        &[
            S(Sr::WindowBase, 1),
            S(Sr::Ps, WOE | EXCM),
            S(Sr::Epc1, CODE),
        ],
        OF12,
    ),
    runs(
        "call8\t0x40370010",
        "e50000",
        &[S(Sr::WindowStart, 0b101)],
        &[
            S(Sr::WindowBase, 2),
            S(Sr::Ps, WOE | EXCM),
            S(Sr::Epc1, CODE),
        ],
        OF12,
    ),
    runs(
        "callx8\ta2",
        "e00200",
        &[S(Sr::WindowStart, 0b101), A(2, TO)],
        &[S(Sr::WindowBase, 2), S(Sr::Ps, WOE | EXCM)],
        OF12,
    ),
    runs(
        "retw",
        "900000",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b11),
            A(0, 0x4037_0040),
        ],
        &[S(Sr::WindowBase, 0), S(Sr::WindowStart, 1)],
        TO,
    ),
    // A spilled caller raises the underflow for the increment in `a0`, with `WindowBase` already
    // at the caller and `WindowStart` as it was.
    runs(
        "retw",
        "900000",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b10),
            A(0, 0x4037_0040),
        ],
        &[
            S(Sr::WindowBase, 0),
            S(Sr::WindowStart, 0b10),
            S(Sr::Ps, WOE | EXCM | owb(1)),
            S(Sr::Epc1, CODE),
        ],
        UF4,
    ),
    runs(
        "retw",
        "900000",
        &[
            S(Sr::WindowBase, 2),
            S(Sr::WindowStart, 0b100),
            A(0, 0x8037_0040),
        ],
        &[S(Sr::WindowBase, 0), S(Sr::Ps, WOE | EXCM | owb(2))],
        UF8,
    ),
    runs(
        "retw",
        "900000",
        &[
            S(Sr::WindowBase, 3),
            S(Sr::WindowStart, 0b1000),
            A(0, 0xc037_0040),
        ],
        &[S(Sr::WindowBase, 0), S(Sr::Ps, WOE | EXCM | owb(3))],
        UF12,
    ),
    runs(
        "retw",
        "900000",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b10),
            A(0, 0x8037_0040),
        ],
        &[S(Sr::WindowBase, 15), S(Sr::Ps, WOE | EXCM | owb(1))],
        UF8,
    ),
    runs(
        "retw.n",
        "1df0",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b10),
            A(0, 0x4037_0040),
        ],
        &[S(Sr::WindowBase, 0), S(Sr::Epc1, CODE)],
        UF4,
    ),
    // The nearest frame below is one quarter down and `a0` says two, which the ISA leaves
    // undefined.
    stops(
        "retw",
        "900000",
        &[
            S(Sr::WindowBase, 3),
            S(Sr::WindowStart, 0b1100),
            A(0, 0x8037_0040),
        ],
        &[S(Sr::WindowBase, 3), S(Sr::WindowStart, 0b1100)],
        CODE,
        Missing::Op(Op::RETW),
    ),
    stops(
        "retw.n",
        "1df0",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b11),
            A(0, 0x0037_0040),
        ],
        &[S(Sr::WindowBase, 1)],
        CODE,
        Missing::Op(Op::RETW_N),
    ),
    stops(
        "retw",
        "900000",
        &[
            S(Sr::Ps, WOE | EXCM),
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b11),
            A(0, 0x4037_0040),
        ],
        &[S(Sr::WindowBase, 1), S(Sr::WindowStart, 0b11)],
        CODE,
        Missing::Op(Op::RETW),
    ),
    // `movsp` moves when a frame lies in the three quarters below, and raises the alloca
    // exception otherwise, with `PS.WOE` clear too.
    runs(
        "movsp\ta1, a2",
        "101200",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b11),
            A(2, 0x3fc8_0000),
        ],
        &[A(1, 0x3fc8_0000)],
        NEXT,
    ),
    runs(
        "movsp\ta1, a2",
        "101200",
        &[
            S(Sr::WindowBase, 3),
            S(Sr::WindowStart, 0b1001),
            A(2, 0x3fc8_0000),
        ],
        &[A(1, 0x3fc8_0000)],
        NEXT,
    ),
    runs(
        "movsp\ta1, a2",
        "101200",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b10),
            A(1, 7),
            A(2, 0x3fc8_0000),
        ],
        &[
            A(1, 7),
            S(Sr::ExcCause, 5),
            S(Sr::Epc1, CODE),
            S(Sr::Ps, WOE | EXCM),
        ],
        KERNEL,
    ),
    runs(
        "movsp\ta1, a2",
        "101200",
        &[S(Sr::WindowBase, 4), S(Sr::WindowStart, 0b1_0001), A(1, 7)],
        &[A(1, 7), S(Sr::ExcCause, 5)],
        KERNEL,
    ),
    runs(
        "movsp\ta1, a2",
        "101200",
        &[S(Sr::Ps, 0), S(Sr::WindowBase, 1), S(Sr::WindowStart, 0b10)],
        &[S(Sr::ExcCause, 5), S(Sr::Ps, EXCM)],
        KERNEL,
    ),
    runs(
        "rotw\t1",
        "108040",
        &[],
        &[S(Sr::WindowBase, 1), S(Sr::WindowStart, 1)],
        NEXT,
    ),
    runs(
        "rotw\t-1",
        "f08040",
        &[],
        &[S(Sr::WindowBase, 15), S(Sr::WindowStart, 1)],
        NEXT,
    ),
    runs(
        "rfwo",
        "003400",
        &[
            S(Sr::Ps, WOE | EXCM | owb(5)),
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b11),
            S(Sr::Epc1, TO),
        ],
        &[
            S(Sr::Ps, WOE | owb(5)),
            S(Sr::WindowBase, 5),
            S(Sr::WindowStart, 0b01),
        ],
        TO,
    ),
    runs(
        "rfwu",
        "003500",
        &[
            S(Sr::Ps, WOE | EXCM | owb(5)),
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b01),
            S(Sr::Epc1, TO),
        ],
        &[
            S(Sr::Ps, WOE | owb(5)),
            S(Sr::WindowBase, 5),
            S(Sr::WindowStart, 0b11),
        ],
        TO,
    ),
    runs(
        "l32e\ta2, a3, -16",
        "20c309",
        &[A(3, RAM + 16), M(RAM, 0xdead_beef)],
        &[A(2, 0xdead_beef)],
        NEXT,
    ),
    // With the Hardware Alignment Option, an unaligned `l32e` or `s32e` acts at its own address
    // (4.6, p 168).
    runs(
        "l32e\ta2, a3, -16",
        "20c309",
        &[BYTES[0], BYTES[1], A(3, RAM + 17)],
        &[A(2, 0xc4b3_a291)],
        NEXT,
    ),
    runs(
        "s32e\ta2, a3, -16",
        "20c349",
        &[A(2, 0xdead_beef), A(3, RAM + 16)],
        &[M(RAM, 0xdead_beef)],
        NEXT,
    ),
    runs(
        "s32e\ta2, a3, -16",
        "20c349",
        &[ONES[0], ONES[1], A(2, 0x4433_2211), A(3, RAM + 19)],
        &[M(RAM, 0x11ff_ffff), M(RAM + 4, 0xff44_3322)],
        NEXT,
    ),
    // A write that leaves a frame in the window completes, and the next reference to it spills.
    runs(
        "wsr.windowstart\ta2",
        "204913",
        &[A(2, 0b11)],
        &[S(Sr::WindowStart, 0b11)],
        NEXT,
    ),
    runs(
        "rfe",
        "003000",
        &[
            S(Sr::Ps, WOE | EXCM),
            S(Sr::WindowStart, 0b11),
            S(Sr::Epc1, TO),
        ],
        &[S(Sr::Ps, WOE)],
        TO,
    ),
    runs(
        "loop\ta2, 0x40370010",
        "76820c",
        &[A(2, 3)],
        &[S(Sr::Lcount, 2), S(Sr::Lbeg, NEXT), S(Sr::Lend, TAKEN)],
        NEXT,
    ),
    // A count of 0 loops 2^32 times.
    runs(
        "loop\ta2, 0x40370010",
        "76820c",
        &[A(2, 0)],
        &[S(Sr::Lcount, u32::MAX)],
        NEXT,
    ),
    // A loop op that skips the body still sets the loop registers.
    runs(
        "loopnez\ta2, 0x40370010",
        "76920c",
        &[A(2, 0)],
        &[
            S(Sr::Lcount, u32::MAX),
            S(Sr::Lbeg, NEXT),
            S(Sr::Lend, TAKEN),
        ],
        TAKEN,
    ),
    runs(
        "loopnez\ta2, 0x40370010",
        "76920c",
        &[A(2, 5)],
        &[S(Sr::Lcount, 4)],
        NEXT,
    ),
    runs(
        "loopgtz\ta2, 0x40370010",
        "76a20c",
        &[A(2, 1 << 31)],
        &[S(Sr::Lcount, 0x7fff_ffff), S(Sr::Lend, TAKEN)],
        TAKEN,
    ),
    runs(
        "loopgtz\ta2, 0x40370010",
        "76a20c",
        &[A(2, 0)],
        &[S(Sr::Lcount, u32::MAX)],
        TAKEN,
    ),
    runs(
        "loopgtz\ta2, 0x40370010",
        "76a20c",
        &[A(2, 1)],
        &[S(Sr::Lcount, 0)],
        NEXT,
    ),
    runs(
        "rsr.lbeg\ta2",
        "200003",
        &[S(Sr::Lbeg, TO)],
        &[A(2, TO)],
        NEXT,
    ),
    runs(
        "wsr.lend\ta2",
        "200113",
        &[A(2, TO)],
        &[S(Sr::Lend, TO)],
        NEXT,
    ),
    runs(
        "rsr.lcount\ta2",
        "200203",
        &[S(Sr::Lcount, 7)],
        &[A(2, 7)],
        NEXT,
    ),
    runs(
        "waiti\t3",
        "007300",
        &[S(Sr::Ps, WOE | 5)],
        &[S(Sr::Ps, WOE | 3)],
        NEXT,
    ),
    // `INTSET` sets only the software interrupts, 7 and 29.
    runs(
        "wsr.intset\ta2",
        "20e213",
        &[A(2, u32::MAX)],
        &[S(Sr::Interrupt, 0x2000_0080)],
        NEXT,
    ),
    // `INTCLEAR` clears the software and edge interrupts. The timer interrupts stay.
    runs(
        "wsr.intclear\ta2",
        "20e313",
        &[S(Sr::Interrupt, u32::MAX), A(2, u32::MAX)],
        &[S(Sr::Interrupt, 0x0001_8040)],
        NEXT,
    ),
    runs(
        "rsr.interrupt\ta2",
        "20e203",
        &[S(Sr::Interrupt, 1 << 7)],
        &[A(2, 1 << 7)],
        NEXT,
    ),
    // Writing `CCOMPARE0` clears timer interrupt 6.
    runs(
        "wsr.ccompare0\ta2",
        "20f013",
        &[S(Sr::Interrupt, 1 << 6 | 1 << 7), A(2, 100)],
        &[S(Sr::Ccompare0, 100), S(Sr::Interrupt, 1 << 7)],
        NEXT,
    ),
    runs(
        "wsr.ccompare2\ta2",
        "20f213",
        &[S(Sr::Interrupt, 1 << 16 | 1 << 15), A(2, 100)],
        &[S(Sr::Ccompare2, 100), S(Sr::Interrupt, 1 << 15)],
        NEXT,
    ),
    // `CCOUNT` takes the value at the write and counts the write's own cycle after it.
    runs(
        "wsr.ccount\ta2",
        "20ea13",
        &[A(2, 1000)],
        &[S(Sr::Ccount, 1001)],
        NEXT,
    ),
];

const CLASSIC_ROWS: &[Row] = &[
    stops(
        "l32ai\ta2, a3, 4",
        "22b301",
        &[A(2, 7), A(3, RAM + 1)],
        &[A(2, 7)],
        CODE,
        Missing::Unaligned(Op::L32AI),
    ),
    stops(
        "s32ri\ta2, a3, 4",
        "22f301",
        &[A(2, 0xdead_beef), A(3, RAM + 2)],
        &[M(RAM + 4, 0), M(RAM + 8, 0)],
        CODE,
        Missing::Unaligned(Op::S32RI),
    ),
];

const S3_ROWS: &[Row] = &[
    runs(
        "s32nb\ta2, a3, 0",
        "200359",
        &[A(2, 0x1234_5678), A(3, RAM)],
        &[M(RAM, 0x1234_5678)],
        NEXT,
    ),
    runs(
        "l32ai\ta2, a3, 4",
        "22b301",
        &[A(2, 7), A(3, RAM + 1)],
        &[
            A(2, 7),
            S(Sr::ExcCause, Cause::LoadStoreAlignment as u32),
            S(Sr::ExcVaddr, RAM + 5),
            S(Sr::Epc1, CODE),
            S(Sr::Ps, WOE | EXCM),
        ],
        KERNEL,
    ),
    runs(
        "s32ri\ta2, a3, 4",
        "22f301",
        &[A(2, 0xdead_beef), A(3, RAM + 2)],
        &[
            M(RAM + 4, 0),
            M(RAM + 8, 0),
            S(Sr::ExcCause, Cause::LoadStoreAlignment as u32),
            S(Sr::ExcVaddr, RAM + 6),
        ],
        KERNEL,
    ),
    // Capture `salt 002` / `saltu 002`: `ffffffff` against `0`. Signed is 1, unsigned is 0.
    runs(
        "salt\ta2, a3, a4",
        "402372",
        &[A(3, 0xffff_ffff), A(4, 0)],
        &[A(2, 1)],
        NEXT,
    ),
    runs(
        "saltu\ta2, a3, a4",
        "402362",
        &[A(3, 0xffff_ffff), A(4, 0)],
        &[A(2, 0)],
        NEXT,
    ),
    // Capture `salt 008` / `saltu 008`: `0` against `ffffffff`. Signed is 0, unsigned is 1.
    runs(
        "salt\ta2, a3, a4",
        "402372",
        &[A(3, 0), A(4, 0xffff_ffff)],
        &[A(2, 0)],
        NEXT,
    ),
    runs(
        "saltu\ta2, a3, a4",
        "402362",
        &[A(3, 0), A(4, 0xffff_ffff)],
        &[A(2, 1)],
        NEXT,
    ),
    // Capture `salt 000` / `saltu 000`: equal operands. Both results are 0.
    runs(
        "salt\ta2, a3, a4",
        "402372",
        &[A(3, 0), A(4, 0)],
        &[A(2, 0)],
        NEXT,
    ),
    runs(
        "saltu\ta2, a3, a4",
        "402362",
        &[A(3, 0), A(4, 0)],
        &[A(2, 0)],
        NEXT,
    ),
];

fn bytes_of(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// `PS` has `WOE` set and nothing else unless the row sets it.
fn run_row<C: Config>(row: &Row) {
    let at = format!("{} {:?} from {:?}", C::NAME, row.text, row.before);
    let decoded = C::decode(&bytes_of(row.hex), CODE).unwrap();
    assert!(matches!(decoded, Decoded::Insn(_)), "{at}: decodes");
    assert_eq!(C::render(&decoded), row.text, "{at}: the bytes are the op");

    let mut bus = Flat::new(row.hex);
    let mut core: Core<C> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    for is in row.before {
        match *is {
            A(n, value) => core.set_a(n, value),
            S(sr, value) => core.set_special(sr, value),
            M(addr, value) => bus.ram.write(addr, Width::Word, value),
        }
    }
    assert_eq!(core.run_until(&mut bus, Cycle(1)), row.stop, "{at}: stop");
    assert_eq!(core.pc(), row.pc, "{at}: pc");
    for is in row.after {
        let (have, want) = match *is {
            A(n, value) => (core.a(n), value),
            S(sr, value) => (core.special(sr), value),
            M(addr, value) => (bus.ram.read(addr, Width::Word), value),
        };
        assert_eq!(have, want, "{at}: {is:?}, have {have:#x}");
    }
}

#[test]
fn every_unprobed_arm_matches_the_isa() {
    for row in ROWS {
        run_row::<Esp32>(row);
        run_row::<Esp32s3>(row);
    }
    for row in CLASSIC_ROWS {
        run_row::<Esp32>(row);
    }
    for row in S3_ROWS {
        run_row::<Esp32s3>(row);
    }
    println!(
        "ops: {} rows on both chips, {} on the S3",
        ROWS.len(),
        S3_ROWS.len()
    );
}

/// `call12` and `entry` in a loop rotate by three quarters a call. The fifth `entry` makes
/// window 15, whose next quarter is frame 0's, so the `call12` after it spills frame 0 into the
/// twelve-register handler.
#[test]
fn a_call12_chain_spills_frame_0_at_the_sixth_call() {
    // call12 1f; .byte 0; 1: entry a1, 16; call12 1b
    let mut bus = Flat::new("35000000362100f5ffff");
    let mut core: Core<Esp32> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_a(1, 0x3fc9_0000);
    assert_eq!(core.run_until(&mut bus, Cycle(1000)), Stop::Bus);
    assert_eq!(core.pc(), OF12);
    assert_eq!(core.special(Sr::Epc1), CODE + 7);
    assert_eq!(core.special(Sr::WindowBase), 0);
    assert_eq!(core.special(Sr::WindowStart), 0b1001_0010_0100_1001);
    assert_eq!(core.special(Sr::Ps), WOE | EXCM | CALLINC3 | owb(15));
    assert_eq!(core.a(1), 0x3fc9_0000);
}

#[test]
fn the_new_stops_say_what_is_missing() {
    let at = |missing| Unimplemented { pc: CODE, missing }.to_string();
    assert_eq!(
        at(Missing::Unaligned(Op::L32AI)),
        "unaligned `l32ai` at 0x40370000 is not modeled"
    );
    assert_eq!(
        at(Missing::Litbase),
        "`l32r` with LITBASE at 0x40370000 is not modeled"
    );
    assert_eq!(
        at(Missing::Unaligned(Op::S32I)),
        "unaligned `s32i` at 0x40370000 is not modeled"
    );
    assert_eq!(
        at(Missing::LoopEnd(Op::CALL0)),
        "`call0` as the last op of a loop at 0x40370000 is not modeled"
    );
}
