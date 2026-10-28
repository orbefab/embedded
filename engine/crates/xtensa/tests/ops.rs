//! One row per op arm the probe images do not run, on the flat test bus. Each row's state after
//! comes from the op's operation in the Xtensa ISA, not from this core, except where the ISA
//! leaves the result open. There the unaligned stop rows and the early `entry` stop rows pin this
//! core's choice. The encodings are GNU `as` output, and each row checks that its bytes decode to
//! its text.

use sim_xtensa::{
    Config, Core, Cycle, Decoded, Esp32, Esp32s3, Missing, Op, Sr, Stop, Unimplemented, Width,
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
    stops(
        "l32ai\ta2, a3, 4",
        "22b301",
        &[A(2, 7), A(3, RAM + 1)],
        &[A(2, 7)],
        CODE,
        Missing::Unaligned(Op::L32AI),
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
        "s32ri\ta2, a3, 4",
        "22f301",
        &[A(2, 0xdead_beef), A(3, RAM + 2)],
        &[M(RAM + 4, 0), M(RAM + 8, 0)],
        CODE,
        Missing::Unaligned(Op::S32RI),
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
    // The ISA's own check: the frame at `WindowBase + 1` for `CALLINC` 1.
    stops(
        "entry\ta1, 32",
        "364100",
        &[
            S(Sr::Ps, WOE | CALLINC1),
            S(Sr::WindowStart, 0b11),
            A(1, 0x3fc9_0000),
        ],
        &[
            S(Sr::WindowBase, 0),
            S(Sr::WindowStart, 0b11),
            A(1, 0x3fc9_0000),
        ],
        CODE,
        Missing::WindowOverflow,
    ),
    // A frame in the callee's top quarter, which the chip spills on the first reference.
    stops(
        "entry\ta1, 32",
        "364100",
        &[
            S(Sr::Ps, WOE | CALLINC1),
            S(Sr::WindowStart, 0b1_0001),
            A(1, 0x3fc9_0000),
        ],
        &[
            S(Sr::WindowBase, 0),
            S(Sr::WindowStart, 0b1_0001),
            A(1, 0x3fc9_0000),
        ],
        CODE,
        Missing::WindowOverflow,
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
    stops(
        "retw",
        "900000",
        &[
            S(Sr::WindowBase, 1),
            S(Sr::WindowStart, 0b10),
            A(0, 0x4037_0040),
        ],
        &[S(Sr::WindowBase, 1), S(Sr::WindowStart, 0b10)],
        CODE,
        Missing::WindowUnderflow,
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
    // A write that leaves a frame in the window with overflow on stops before the next op.
    stops(
        "wsr.windowstart\ta2",
        "204913",
        &[A(2, 0b11)],
        &[S(Sr::WindowStart, 0b11)],
        NEXT,
        Missing::WindowOverflow,
    ),
    runs(
        "wsr.windowstart\ta2",
        "204913",
        &[S(Sr::Ps, WOE | EXCM), A(2, 0b11)],
        &[S(Sr::WindowStart, 0b11)],
        NEXT,
    ),
    stops(
        "rfe",
        "003000",
        &[
            S(Sr::Ps, WOE | EXCM),
            S(Sr::WindowStart, 0b11),
            S(Sr::Epc1, TO),
        ],
        &[S(Sr::Ps, WOE)],
        TO,
        Missing::WindowOverflow,
    ),
];

const S3_ROWS: &[Row] = &[runs(
    "s32nb\ta2, a3, 0",
    "200359",
    &[A(2, 0x1234_5678), A(3, RAM)],
    &[M(RAM, 0x1234_5678)],
    NEXT,
)];

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
    for row in S3_ROWS {
        run_row::<Esp32s3>(row);
    }
    println!(
        "ops: {} rows on both chips, {} on the S3",
        ROWS.len(),
        S3_ROWS.len()
    );
}

/// `call12` and `entry` in a loop rotate by three quarters a call, and the fourth `entry` makes
/// window 12. The chip would run the fifth `entry` into window 15, whose second quarter is frame
/// 0's, and spill frame 0 at the next `call12`, whose `WindowCheck` tests `WindowBase + 1`. The
/// core stops at the fifth `entry` in window 12, since frame 0 lies in the callee's window.
#[test]
fn a_call12_chain_stops_at_the_entry_that_would_reach_frame_0() {
    // call12 1f; .byte 0; 1: entry a1, 16; call12 1b
    let mut bus = Flat::new("35000000362100f5ffff");
    let mut core: Core<Esp32> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_a(1, 0x3fc9_0000);
    let stop = core.run_until(&mut bus, Cycle(1000));
    let want = Unimplemented {
        pc: CODE + 4,
        missing: Missing::WindowOverflow,
    };
    assert_eq!(stop, Stop::Unimplemented(want));
    assert_eq!(core.special(Sr::WindowBase), 12);
    assert_eq!(core.special(Sr::WindowStart), 0b1_0010_0100_1001);
    assert_eq!(core.a(1), 0x3fc9_0000 - 4 * 16);
    assert_eq!(
        want.to_string(),
        "window overflow at 0x40370004 is not modeled"
    );
}

/// The window check after a write stays pending until it passes, so a run that resumes after
/// the stop stops again before the next op.
#[test]
fn a_resumed_run_stops_again_at_an_exposed_frame() {
    // wsr.windowstart a2; nop
    let mut bus = Flat::new("204913f02000");
    let mut core: Core<Esp32> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_a(2, 0b11);
    let want = Stop::Unimplemented(Unimplemented {
        pc: NEXT,
        missing: Missing::WindowOverflow,
    });
    assert_eq!(core.run_until(&mut bus, Cycle(1)), want);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), want);
    assert_eq!(core.cycle(), Cycle(1));
    core.set_special(Sr::WindowStart, 1);
    assert_eq!(core.run_until(&mut bus, Cycle(2)), Stop::Limit);
    assert_eq!(core.pc(), NEXT + 3);
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
}
