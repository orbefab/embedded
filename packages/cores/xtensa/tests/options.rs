//! One row per option op arm the probe images do not run: the Boolean ops, the FPU loads and
//! stores, the FPU rules no fixture row reaches, `s32c1i`, and the S3's `q` registers and
//! `cop_ai` user registers. Each row's state
//! after comes from the op's operation in the Xtensa ISA, and each width from the chip's table.
//! The encodings are GNU `as` output, and each row checks that its bytes decode to its text.

use orbe_xtensa::{
    Cause, Config, CoprocessorDisabled, Core, Cycle, Esp32, Esp32s3, Missing, Op, Sr, Stop,
    Unimplemented, Width,
};

use crate::common::fixture_path;
use crate::core::{CODE, Flat, RAM, core_at};
use crate::ops::{EXCM, Is, KERNEL, NEXT, OF4, Row, TAKEN, WOE, run_row, runs, stops};

use Is::{A, F, M, Q, S};

const X: u32 = 0x4049_0fdb;
const Y: u32 = 0xc2f6_e979;
const Q2: u128 = 0x0f0e_0d0c_0b0a_0908_0706_0504_0302_0100;

const FPU: Is = S(Sr::CpEnable, 1);
const COP_AI: Is = S(Sr::CpEnable, 1 << 3);

const ROWS: &[Row] = &[
    runs(
        "andb\tb0, b1, b2",
        "200102",
        &[S(Sr::Br, 0b0110)],
        &[S(Sr::Br, 0b0111)],
        NEXT,
    ),
    runs(
        "andbc\tb0, b1, b2",
        "200112",
        &[S(Sr::Br, 0b0010)],
        &[S(Sr::Br, 0b0011)],
        NEXT,
    ),
    runs(
        "orb\tb3, b1, b2",
        "203122",
        &[S(Sr::Br, 0b0100)],
        &[S(Sr::Br, 0b1100)],
        NEXT,
    ),
    runs(
        "orbc\tb0, b1, b2",
        "200132",
        &[S(Sr::Br, 0b0100)],
        &[S(Sr::Br, 0b0100)],
        NEXT,
    ),
    runs(
        "orbc\tb0, b1, b2",
        "200132",
        &[S(Sr::Br, 0)],
        &[S(Sr::Br, 0b0001)],
        NEXT,
    ),
    runs(
        "xorb\tb0, b1, b2",
        "200142",
        &[S(Sr::Br, 0b0111)],
        &[S(Sr::Br, 0b0110)],
        NEXT,
    ),
    runs(
        "any4\tb0, b4:b5:b6:b7",
        "008400",
        &[S(Sr::Br, 0x40)],
        &[S(Sr::Br, 0x41)],
        NEXT,
    ),
    runs(
        "any4\tb0, b4:b5:b6:b7",
        "008400",
        &[S(Sr::Br, 0x0f01)],
        &[S(Sr::Br, 0x0f00)],
        NEXT,
    ),
    runs(
        "all4\tb0, b4:b5:b6:b7",
        "009400",
        &[S(Sr::Br, 0xf0)],
        &[S(Sr::Br, 0xf1)],
        NEXT,
    ),
    runs(
        "all4\tb0, b4:b5:b6:b7",
        "009400",
        &[S(Sr::Br, 0x71)],
        &[S(Sr::Br, 0x70)],
        NEXT,
    ),
    runs(
        "any8\tb0, b8:b9:b10:b11:b12:b13:b14:b15",
        "00a800",
        &[S(Sr::Br, 0x8000)],
        &[S(Sr::Br, 0x8001)],
        NEXT,
    ),
    runs(
        "all8\tb0, b8:b9:b10:b11:b12:b13:b14:b15",
        "00b800",
        &[S(Sr::Br, 0xff00)],
        &[S(Sr::Br, 0xff01)],
        NEXT,
    ),
    runs(
        "all8\tb0, b8:b9:b10:b11:b12:b13:b14:b15",
        "00b800",
        &[S(Sr::Br, 0xfe01)],
        &[S(Sr::Br, 0xfe00)],
        NEXT,
    ),
    runs(
        "wsr.br\ta2",
        "200413",
        &[A(2, 0x1_2345)],
        &[S(Sr::Br, 0x2345)],
        NEXT,
    ),
    runs(
        "bt\tb1, 0x40370010",
        "76110c",
        &[S(Sr::Br, 0b10)],
        &[],
        TAKEN,
    ),
    runs(
        "bt\tb1, 0x40370010",
        "76110c",
        &[S(Sr::Br, 0b01)],
        &[],
        NEXT,
    ),
    runs(
        "bf\tb1, 0x40370010",
        "76010c",
        &[S(Sr::Br, 0b01)],
        &[],
        TAKEN,
    ),
    runs(
        "bf\tb1, 0x40370010",
        "76010c",
        &[S(Sr::Br, 0b10)],
        &[],
        NEXT,
    ),
    runs(
        "movf\ta2, a3, b1",
        "1023c3",
        &[S(Sr::Br, 0b01), A(2, 1), A(3, 7)],
        &[A(2, 7)],
        NEXT,
    ),
    runs(
        "movf\ta2, a3, b1",
        "1023c3",
        &[S(Sr::Br, 0b10), A(2, 1), A(3, 7)],
        &[A(2, 1)],
        NEXT,
    ),
    runs(
        "movt\ta2, a3, b1",
        "1023d3",
        &[S(Sr::Br, 0b10), A(2, 1), A(3, 7)],
        &[A(2, 7)],
        NEXT,
    ),
    runs(
        "movt\ta2, a3, b1",
        "1023d3",
        &[S(Sr::Br, 0b01), A(2, 1), A(3, 7)],
        &[A(2, 1)],
        NEXT,
    ),
    runs(
        "lsi\tf1, a3, 4",
        "130301",
        &[FPU, A(3, RAM), M(RAM + 4, X)],
        &[F(1, X), A(3, RAM)],
        NEXT,
    ),
    runs(
        "lsip\tf1, a3, 4",
        "138301",
        &[FPU, A(3, RAM), M(RAM, X), M(RAM + 4, Y)],
        &[F(1, X), A(3, RAM + 4)],
        NEXT,
    ),
    runs(
        "lsx\tf1, a3, a4",
        "401308",
        &[FPU, A(3, RAM), A(4, 8), M(RAM + 8, X)],
        &[F(1, X), A(3, RAM)],
        NEXT,
    ),
    runs(
        "lsxp\tf1, a3, a4",
        "401318",
        &[FPU, A(3, RAM), A(4, 8), M(RAM, X), M(RAM + 8, Y)],
        &[F(1, X), A(3, RAM + 8)],
        NEXT,
    ),
    runs(
        "ssi\tf1, a3, 4",
        "134301",
        &[FPU, A(3, RAM), F(1, X)],
        &[M(RAM, 0), M(RAM + 4, X), A(3, RAM)],
        NEXT,
    ),
    runs(
        "ssip\tf1, a3, 4",
        "13c301",
        &[FPU, A(3, RAM), F(1, X)],
        &[M(RAM, X), M(RAM + 4, 0), A(3, RAM + 4)],
        NEXT,
    ),
    runs(
        "ssx\tf1, a3, a4",
        "401348",
        &[FPU, A(3, RAM), A(4, 8), F(1, X)],
        &[M(RAM, 0), M(RAM + 8, X), A(3, RAM)],
        NEXT,
    ),
    runs(
        "ssxp\tf1, a3, a4",
        "401358",
        &[FPU, A(3, RAM), A(4, 8), F(1, X)],
        &[M(RAM, X), M(RAM + 8, 0), A(3, RAM + 8)],
        NEXT,
    ),
    // `CPENABLE` bit 0 clear: Coprocessor0Disabled, before the op changes anything.
    runs(
        "lsi\tf1, a3, 4",
        "130301",
        &[A(3, RAM), M(RAM + 4, X), F(1, Y)],
        &[
            F(1, Y),
            S(Sr::ExcCause, Cause::Coprocessor0Disabled as u32),
            S(Sr::Epc1, CODE),
            S(Sr::Ps, WOE | EXCM),
        ],
        KERNEL,
    ),
    runs(
        "add.s\tf1, f2, f3",
        "30120a",
        &[S(Sr::CpEnable, 0xfe), F(1, Y), F(2, X), F(3, X)],
        &[
            F(1, Y),
            S(Sr::ExcCause, Cause::Coprocessor0Disabled as u32),
            S(Sr::Epc1, CODE),
        ],
        KERNEL,
    ),
    // `a4` names a quarter that has to spill, and the window overflow comes first (4.4.1.11,
    // p 146).
    runs(
        "lsi\tf1, a4, 4",
        "130401",
        &[S(Sr::WindowStart, 0b111), A(4, RAM), F(1, Y)],
        &[
            F(1, Y),
            S(Sr::WindowBase, 1),
            S(Sr::Ps, WOE | EXCM),
            S(Sr::Epc1, CODE),
            S(Sr::ExcCause, 0),
        ],
        OF4,
    ),
    // No fixture row reaches the next eight. The ISA detects tininess after rounding (4.3.11.3,
    // p 109), so `2^-126 - 2^-152`, which rounds to `2^-126`, is inexact and not underflow.
    runs(
        "madd.s\tf0, f1, f2",
        "20014a",
        &[FPU, F(0, 0x0080_0000), F(1, 0x9980_0000), F(2, 0x1980_0000)],
        &[F(0, 0x0080_0000), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    // An exact zero sum of two zeros of one sign keeps that sign. Any other exact zero sum is
    // +0, or -0 when rounding down, as IEEE 754 has it.
    runs(
        "add.s\tf1, f2, f3",
        "30120a",
        &[FPU, F(2, 0x8000_0000), F(3, 0x8000_0000)],
        &[F(1, 0x8000_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "add.s\tf1, f2, f3",
        "30120a",
        &[
            FPU,
            S(Sr::Fcr, 3),
            F(1, 7),
            F(2, 0x3f80_0000),
            F(3, 0xbf80_0000),
        ],
        &[F(1, 0x8000_0000)],
        NEXT,
    ),
    runs(
        "add.s\tf1, f2, f3",
        "30120a",
        &[
            FPU,
            S(Sr::Fcr, 2),
            F(1, 7),
            F(2, 0x3f80_0000),
            F(3, 0xbf80_0000),
        ],
        &[F(1, 0)],
        NEXT,
    ),
    // An exact quotient leaves a zero remainder, and `divn.s` then gives the estimate exact in
    // every rounding mode, as IEEE 754 division does for `1 / 1`.
    runs(
        "divn.s\tf0, f1, f2",
        "20017a",
        &[
            FPU,
            S(Sr::Fcr, 0),
            F(0, 0x3f80_0000),
            F(1, 0),
            F(2, 0x3f80_0000),
        ],
        &[F(0, 0x3f80_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "divn.s\tf0, f1, f2",
        "20017a",
        &[
            FPU,
            S(Sr::Fcr, 1),
            F(0, 0x3f80_0000),
            F(1, 0x8000_0000),
            F(2, 0x3f80_0000),
        ],
        &[F(0, 0x3f80_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "divn.s\tf0, f1, f2",
        "20017a",
        &[
            FPU,
            S(Sr::Fcr, 2),
            F(0, 0x3f80_0000),
            F(1, 0),
            F(2, 0x3f80_0000),
        ],
        &[F(0, 0x3f80_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "divn.s\tf0, f1, f2",
        "20017a",
        &[
            FPU,
            S(Sr::Fcr, 3),
            F(0, 0x3f80_0000),
            F(1, 0x8000_0000),
            F(2, 0x3f80_0000),
        ],
        &[F(0, 0x3f80_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    // A fused exact zero sum takes its sign by the same rule, with the product's sign as one
    // addend's.
    runs(
        "madd.s\tf0, f1, f2",
        "20014a",
        &[FPU, F(0, 0x8000_0000), F(1, 0), F(2, 0x8000_0000)],
        &[F(0, 0x8000_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "msub.s\tf0, f1, f2",
        "20015a",
        &[FPU, F(0, 0x8000_0000), F(1, 0), F(2, 0)],
        &[F(0, 0x8000_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    // Round to nearest takes a tie to the even neighbour, as IEEE 754 has it. No fixture row
    // lands on a tie.
    runs(
        "float.s\tf0, a2, 0",
        "0002ca",
        &[FPU, A(2, 0x0100_0001)],
        &[F(0, 0x4b80_0000), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "float.s\tf0, a2, 0",
        "0002ca",
        &[FPU, A(2, 0x0100_0003)],
        &[F(0, 0x4b80_0002), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "float.s\tf0, a2, 0",
        "0002ca",
        &[FPU, A(2, 0xfeff_ffff)],
        &[F(0, 0xcb80_0000), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "round.s\ta2, f1, 0",
        "00218a",
        &[FPU, F(1, 0x4020_0000)],
        &[A(2, 2), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "round.s\ta2, f1, 0",
        "00218a",
        &[FPU, F(1, 0x4060_0000)],
        &[A(2, 4), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "round.s\ta2, f1, 0",
        "00218a",
        &[FPU, F(1, 0xc020_0000)],
        &[A(2, -2i32 as u32), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "round.s\ta2, f1, 0",
        "00218a",
        &[FPU, F(1, 0xc060_0000)],
        &[A(2, -4i32 as u32), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    // `FSR`'s flags are sticky: an op ORs in what it raises, and one that raises nothing leaves
    // them.
    runs(
        "mul.s\tf1, f2, f3",
        "30122a",
        &[FPU, S(Sr::Fsr, 0x80), F(2, 0), F(3, 0x7f80_0000)],
        &[F(1, 0x7fc0_0000), S(Sr::Fsr, 0x880)],
        NEXT,
    ),
    runs(
        "add.s\tf1, f2, f3",
        "30120a",
        &[FPU, S(Sr::Fsr, 0x80), F(2, 0x3f80_0000), F(3, 0x3f80_0000)],
        &[F(1, 0x4000_0000), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "round.s\ta2, f1, 0",
        "00218a",
        &[FPU, S(Sr::Fsr, 0x800), F(1, 0x4020_0000)],
        &[A(2, 2), S(Sr::Fsr, 0x880)],
        NEXT,
    ),
    runs(
        "round.s\ta2, f1, 0",
        "00218a",
        &[FPU, S(Sr::Fsr, 0x80), F(1, 0x4000_0000)],
        &[A(2, 2), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "ueq.s\tb0, f1, f2",
        "20013b",
        &[FPU, S(Sr::Fsr, 0x80), F(1, 0x7fc0_0000), F(2, X)],
        &[S(Sr::Br, 1), S(Sr::Fsr, 0x80)],
        NEXT,
    ),
    runs(
        "olt.s\tb0, f1, f2",
        "20014b",
        &[FPU, S(Sr::Fsr, 0x80), F(1, 0x7fc0_0000), F(2, X)],
        &[S(Sr::Br, 0), S(Sr::Fsr, 0x880)],
        NEXT,
    ),
    // The seeds read the ISA's tables (4.3.11.5, p 115). The board rows reach neither entry.
    runs(
        "rsqrt0.s\tf0, f1",
        "a001fa",
        &[FPU, F(1, 0x3f81_0000)],
        &[F(0, 0x3f7f_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "rsqrt0.s\tf0, f1",
        "a001fa",
        &[FPU, F(1, 0x401a_0000)],
        &[F(0, 0x3f25_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "sqrt0.s\tf0, f1",
        "9001fa",
        &[FPU, F(1, 0x3f81_0000)],
        &[F(0, 0x3f7f_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "sqrt0.s\tf0, f1",
        "9001fa",
        &[FPU, F(1, 0x401a_0000)],
        &[F(0, 0x3f25_0000), S(Sr::Fsr, 0)],
        NEXT,
    ),
    runs(
        "s32c1i\ta2, a3, 4",
        "22e301",
        &[S(Sr::Scompare1, 5), A(2, 9), A(3, RAM), M(RAM + 4, 5)],
        &[M(RAM + 4, 9), A(2, 5)],
        NEXT,
    ),
    // `s32c1i` checks store permission only, so a refused address is `StoreProhibited`, never
    // `LoadProhibited` (8.3.281, p 589).
    runs(
        "s32c1i\ta2, a3, 4",
        "22e301",
        &[S(Sr::Scompare1, 5), A(2, 9), A(3, 0x1000)],
        &[
            A(2, 9),
            S(Sr::ExcCause, Cause::StoreProhibited as u32),
            S(Sr::ExcVaddr, 0x1004),
            S(Sr::Epc1, CODE),
        ],
        KERNEL,
    ),
    runs(
        "s32c1i\ta2, a3, 4",
        "22e301",
        &[S(Sr::Scompare1, 6), A(2, 9), A(3, RAM), M(RAM + 4, 5)],
        &[M(RAM + 4, 5), A(2, 5)],
        NEXT,
    ),
];

/// A `s32c1i` that is not word-aligned stops on the classic core, as `l32ai` and `s32ri` do.
const CLASSIC_ROWS: &[Row] = &[stops(
    "s32c1i\ta2, a3, 4",
    "22e301",
    &[S(Sr::Scompare1, 5), A(2, 9), A(3, RAM + 1), M(RAM + 4, 5)],
    &[M(RAM + 4, 5), A(2, 9)],
    CODE,
    Missing::Unaligned(Op::S32C1I),
)];

const S3_ROWS: &[Row] = &[
    stops(
        "ld.qr\tq1, a3, 16",
        "34a1cd",
        &[COP_AI, A(3, RAM + 4), Q(1, Q2)],
        &[Q(1, Q2)],
        CODE,
        Missing::Unaligned(Op::ld_QR),
    ),
    stops(
        "st.qr\tq1, a3, 16",
        "34e1cd",
        &[COP_AI, A(3, RAM + 8), Q(1, Q2), M(RAM + 24, 5)],
        &[M(RAM + 24, 5)],
        CODE,
        Missing::Unaligned(Op::st_QR),
    ),
    runs(
        "mv.qr\tq1, q2",
        "04848f",
        &[COP_AI, Q(2, Q2)],
        &[Q(1, Q2), Q(2, Q2)],
        NEXT,
    ),
    runs(
        "ld.qr\tq1, a3, 16",
        "34a1cd",
        &[
            COP_AI,
            A(3, RAM),
            M(RAM + 16, 0x0302_0100),
            M(RAM + 20, 0x0706_0504),
            M(RAM + 24, 0x0b0a_0908),
            M(RAM + 28, 0x0f0e_0d0c),
        ],
        &[Q(1, Q2), A(3, RAM)],
        NEXT,
    ),
    runs(
        "ld.qr\tq1, a3, -16",
        "34afcd",
        &[
            COP_AI,
            A(3, RAM + 32),
            M(RAM + 16, 0x0302_0100),
            M(RAM + 20, 0x0706_0504),
            M(RAM + 24, 0x0b0a_0908),
            M(RAM + 28, 0x0f0e_0d0c),
        ],
        &[Q(1, Q2)],
        NEXT,
    ),
    runs(
        "st.qr\tq1, a3, 16",
        "34e1cd",
        &[COP_AI, A(3, RAM), Q(1, Q2)],
        &[
            M(RAM + 16, 0x0302_0100),
            M(RAM + 20, 0x0706_0504),
            M(RAM + 24, 0x0b0a_0908),
            M(RAM + 28, 0x0f0e_0d0c),
        ],
        NEXT,
    ),
    runs(
        "wur.accx_1\ta2",
        "2001f3",
        &[COP_AI, A(2, 0x1ff)],
        &[S(Sr::Accx1, 0xff)],
        NEXT,
    ),
    // `GPIO_OUT` is not `cop_ai` state, so it needs no `CPENABLE` bit.
    runs(
        "wur.gpio_out\ta2",
        "200cf3",
        &[A(2, 0x1a5)],
        &[S(Sr::GpioOut, 0xa5)],
        NEXT,
    ),
    // `CPENABLE` bit 3 clear: Coprocessor3Disabled. Bit 0 does not stand in for it.
    runs(
        "mv.qr\tq1, q2",
        "04848f",
        &[FPU, Q(1, 0), Q(2, Q2)],
        &[
            Q(1, 0),
            S(Sr::ExcCause, Cause::Coprocessor3Disabled as u32),
            S(Sr::Epc1, CODE),
        ],
        KERNEL,
    ),
    runs(
        "wur.accx_1\ta2",
        "2001f3",
        &[A(2, 0x1ff)],
        &[
            S(Sr::Accx1, 0),
            S(Sr::ExcCause, Cause::Coprocessor3Disabled as u32),
        ],
        KERNEL,
    ),
    // Not word-aligned: LoadStoreAlignment, as for `l32ai` and `s32ri`, and no store.
    runs(
        "s32c1i\ta2, a3, 4",
        "22e301",
        &[S(Sr::Scompare1, 5), A(2, 9), A(3, RAM + 1), M(RAM + 4, 5)],
        &[
            M(RAM + 4, 5),
            A(2, 9),
            S(Sr::ExcCause, Cause::LoadStoreAlignment as u32),
            S(Sr::ExcVaddr, RAM + 5),
        ],
        KERNEL,
    ),
];

#[test]
fn every_unprobed_option_arm_matches_the_isa() {
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
        "options: {} rows on both chips, {} on the classic core, {} on the S3",
        ROWS.len(),
        CLASSIC_ROWS.len(),
        S3_ROWS.len()
    );
}

/// `s32c1i` needs store permission whether or not its compare matches (8.3.281, p 590), so a
/// word the bus lets the core read and not write raises `StoreProhibited` either way.
fn s32c1i_on_a_read_only_word<C: Config>() {
    for scompare1 in [5, 6] {
        let mut bus = Flat::new("22e301");
        bus.ram.write(RAM + 4, Width::Word, 5);
        bus.read_only = RAM + 4..RAM + 8;
        let mut core: Core<C> = core_at(CODE);
        core.set_special(Sr::Ps, WOE);
        core.set_special(Sr::Scompare1, scompare1);
        core.set_a(2, 9);
        core.set_a(3, RAM);
        let at = format!("{} with SCOMPARE1 {scompare1}", C::NAME);
        assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit, "{at}");
        assert_eq!(core.pc(), KERNEL, "{at}");
        assert_eq!(
            core.special(Sr::ExcCause),
            Cause::StoreProhibited as u32,
            "{at}"
        );
        assert_eq!(core.special(Sr::ExcVaddr), RAM + 4, "{at}");
        assert_eq!(core.a(2), 9, "{at}");
        assert_eq!(bus.ram.read(RAM + 4, Width::Word), 5, "{at}");
    }
}

#[test]
fn s32c1i_needs_store_permission_when_its_compare_fails() {
    s32c1i_on_a_read_only_word::<Esp32>();
    s32c1i_on_a_read_only_word::<Esp32s3>();
}

/// The reciprocal and the reciprocal square-root tables, as `isa-seeds.txt` copies them from the
/// ISA.
fn isa_seeds() -> (Vec<u8>, Vec<u8>) {
    let text = std::fs::read_to_string(fixture_path("isa-seeds.txt")).unwrap();
    let (mut reciprocal, mut rsqrt) = (Vec::new(), Vec::new());
    let mut table = &mut reciprocal;
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        match line {
            "reciprocal" => table = &mut reciprocal,
            "rsqrt" => table = &mut rsqrt,
            _ => table.extend(line.split(", ").map(|n| n.parse::<u8>().unwrap())),
        }
    }
    (reciprocal, rsqrt)
}

/// `f0` and `FSR` after the op `hex`, which reads `f1`, on `a`.
fn seed_of<C: Config>(hex: &str, a: u32) -> (u32, u32) {
    let mut bus = Flat::new(hex);
    let mut core: Core<C> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::CpEnable, 1);
    core.set_f(1, a);
    assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit);
    (core.f(0), core.special(Sr::Fsr))
}

/// Each input is in `[1, 4)` with the exponent that gives the estimate as `estimate / 256`.
fn every_seed_reads_its_entry<C: Config>(reciprocal: &[u8], rsqrt: &[u8]) {
    let want = |estimate: u8| (f32::from(estimate) / 256.0).to_bits();
    for (k, &estimate) in reciprocal.iter().enumerate() {
        let a = 0x3f80_0000 | (k as u32) << 16;
        let at = format!("{} entry {k}", C::NAME);
        assert_eq!(
            seed_of::<C>("8001fa", a),
            (want(estimate), 0),
            "recip0.s {at}"
        );
        assert_eq!(
            seed_of::<C>("7001fa", a),
            (want(estimate), 0),
            "div0.s {at}"
        );
    }
    for (k, &estimate) in rsqrt.iter().enumerate() {
        let k = k as u32;
        let a = (128 - (k >> 6)) << 23 | (k & 63) << 17;
        let at = format!("{} entry {k}", C::NAME);
        assert_eq!(
            seed_of::<C>("a001fa", a),
            (want(estimate), 0),
            "rsqrt0.s {at}"
        );
        assert_eq!(
            seed_of::<C>("9001fa", a),
            (want(estimate), 0),
            "sqrt0.s {at}"
        );
    }
}

/// `recip0.s` and `div0.s` read the 128 reciprocal estimates the ISA prints, and `rsqrt0.s` and
/// `sqrt0.s` the 128 reciprocal square-root estimates (4.3.11.5, pp 114 and 115).
#[test]
fn every_seed_is_the_one_the_isa_prints() {
    let (reciprocal, rsqrt) = isa_seeds();
    assert_eq!((reciprocal.len(), rsqrt.len()), (128, 128));
    every_seed_reads_its_entry::<Esp32>(&reciprocal, &rsqrt);
    every_seed_reads_its_entry::<Esp32s3>(&reciprocal, &rsqrt);
}

/// The flow the S3's lazy `cop_ai` switch takes: an `ee.*` op with coprocessor 3 off raises the
/// exception and the core records it. With the bit set, the same op ends the run as unimplemented,
/// and the record still names it.
#[test]
fn a_vector_op_is_recorded_when_it_raises_and_stops_when_it_runs() {
    let mut bus = Flat::new("a47fcd");
    let mut core: Core<Esp32s3> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    assert_eq!(core.coprocessor_disabled(), None);
    assert_eq!(core.run_until(&mut bus, Cycle(1)), Stop::Limit);
    assert_eq!(core.pc(), KERNEL);
    let record = CoprocessorDisabled {
        coprocessor: 3,
        op: Op::EE_ZERO_Q,
        pc: CODE,
    };
    assert_eq!(core.coprocessor_disabled(), Some(record));
    assert_eq!(
        core.special(Sr::ExcCause),
        Cause::Coprocessor3Disabled as u32
    );

    core.set_special(Sr::CpEnable, 1 << 3);
    core.set_special(Sr::Ps, WOE);
    core.set_pc(CODE);
    let stop = core.run_until(&mut bus, Cycle(100));
    let want = Unimplemented {
        pc: CODE,
        missing: Missing::Op(Op::EE_ZERO_Q),
    };
    assert_eq!(stop, Stop::Unimplemented(want));
    assert_eq!(core.coprocessor_disabled(), Some(record));
}

/// The ISA's single-precision divide sequence (4.3.11.5, p 111), registers `a` f0, `b` f1, `y0`
/// f2, `bN` f3, `e` f4, `y` f5, `ex` f6, `aN` f7, `q` f8 and `r` f9, then `rfdo` to stop on.
const DIVIDE: &str = "7021fab031fa3041fa20436a0052fa0061fab070fa20546a3041fa3080fa6097fa50436a20896ad060fa50546a80936a3041fa50436a50896a6097fa50546a80936af086fae056fa50897a00e0f1";

/// `a / b` rounded in `FCR` mode `mode`, from the host's round-to-nearest quotient. `q * b` has
/// at most 48 significant bits and lies within a factor of two of `a`, so `q * b - a` is exact
/// in `f64` and tells which side of `a / b` the quotient `q` lies on.
fn quotient(a: f32, b: f32, mode: u32) -> u32 {
    let q = a / b;
    let error = (f64::from(q) * f64::from(b) - f64::from(a)) * f64::from(b).signum();
    let up = if error < 0.0 { q.next_up() } else { q };
    let down = if error > 0.0 { q.next_down() } else { q };
    match mode {
        0 => q,
        1 if q > 0.0 => down,
        1 => up,
        2 => up,
        _ => down,
    }
    .to_bits()
}

/// The divide sequence gives the correctly rounded quotient in each rounding mode, on exact
/// quotients such as `1 / 1`, where the remainder `divn.s` sees is zero, and on pseudo-random
/// normal operands.
#[test]
fn the_isa_divide_sequence_rounds_like_ieee_division() {
    let mut pairs = Vec::new();
    for i in 1..=24u16 {
        for j in 1..=24u16 {
            let (i, j) = (f32::from(i), f32::from(j));
            pairs.push((i * j, j));
            pairs.push((-i * j, j));
        }
    }
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for _ in 0..2000 {
        let normal = |bits: u32| f32::from_bits(bits & 0x807f_ffff | (64 + bits % 128) << 23);
        pairs.push((normal(next()), normal(next())));
    }
    for (a, b) in pairs {
        for mode in 0..4 {
            let mut bus = Flat::new(DIVIDE);
            let mut core: Core<Esp32s3> = core_at(CODE);
            core.set_special(Sr::Ps, WOE);
            core.set_special(Sr::CpEnable, 1);
            core.set_special(Sr::Fcr, mode);
            core.set_f(0, a.to_bits());
            core.set_f(1, b.to_bits());
            let stop = core.run_until(&mut bus, Cycle(1000));
            let want = Unimplemented {
                pc: CODE + 75,
                missing: Missing::Op(Op::RFDO),
            };
            assert_eq!(stop, Stop::Unimplemented(want), "{a} / {b} in mode {mode}");
            assert_eq!(
                core.f(8),
                quotient(a, b, mode),
                "{a} / {b} in mode {mode}, have {:#x}",
                core.f(8)
            );
        }
    }
}
