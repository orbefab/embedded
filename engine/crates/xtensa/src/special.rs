//! The special and user registers the core models, and the `rsr`, `wsr`, `xsr`, `rur` and `wur`
//! ops that reach them. A register op this table does not name ends the run as unimplemented.

use crate::decode::Op;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
    Exchange,
}

/// `PS` bits that exist without an MMU: `INTLEVEL`, `EXCM`, `UM`, `OWB`, `CALLINC` and `WOE`.
const PS_MASK: u32 = 0x0007_0f3f;

macro_rules! registers {
    ($($sr:ident: $mask:expr => [$($access:ident $op:ident),+];)+) => {
        /// A modeled special or user register.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Sr {
            $($sr),+
        }

        impl Sr {
            pub(crate) const COUNT: usize = [$(Sr::$sr),+].len();

            /// The bits a write keeps.
            pub(crate) const fn mask(self) -> u32 {
                match self {
                    $(Sr::$sr => $mask),+
                }
            }
        }

        pub(crate) fn register_op(op: Op) -> Option<(Access, Sr)> {
            match op {
                $($(Op::$op => Some((Access::$access, Sr::$sr)),)+)+
                _ => None,
            }
        }
    };
}

registers! {
    Lbeg: u32::MAX => [Read RSR_LBEG, Write WSR_LBEG, Exchange XSR_LBEG];
    Lend: u32::MAX => [Read RSR_LEND, Write WSR_LEND, Exchange XSR_LEND];
    Lcount: u32::MAX => [Read RSR_LCOUNT, Write WSR_LCOUNT, Exchange XSR_LCOUNT];
    Sar: 0x3f => [Read RSR_SAR, Write WSR_SAR, Exchange XSR_SAR];
    Litbase: 0xffff_f001 => [Read RSR_LITBASE, Write WSR_LITBASE, Exchange XSR_LITBASE];
    Scompare1: u32::MAX => [Read RSR_SCOMPARE1, Write WSR_SCOMPARE1, Exchange XSR_SCOMPARE1];
    WindowBase: 0xf => [Read RSR_WINDOWBASE, Write WSR_WINDOWBASE, Exchange XSR_WINDOWBASE];
    WindowStart: 0xffff => [Read RSR_WINDOWSTART, Write WSR_WINDOWSTART, Exchange XSR_WINDOWSTART];
    Ps: PS_MASK => [Read RSR_PS, Write WSR_PS, Exchange XSR_PS];
    Vecbase: 0xffff_fc00 => [Read RSR_VECBASE, Write WSR_VECBASE, Exchange XSR_VECBASE];
    Epc1: u32::MAX => [Read RSR_EPC1, Write WSR_EPC1, Exchange XSR_EPC1];
    Epc2: u32::MAX => [Read RSR_EPC2, Write WSR_EPC2, Exchange XSR_EPC2];
    Epc3: u32::MAX => [Read RSR_EPC3, Write WSR_EPC3, Exchange XSR_EPC3];
    Epc4: u32::MAX => [Read RSR_EPC4, Write WSR_EPC4, Exchange XSR_EPC4];
    Epc5: u32::MAX => [Read RSR_EPC5, Write WSR_EPC5, Exchange XSR_EPC5];
    Epc6: u32::MAX => [Read RSR_EPC6, Write WSR_EPC6, Exchange XSR_EPC6];
    Epc7: u32::MAX => [Read RSR_EPC7, Write WSR_EPC7, Exchange XSR_EPC7];
    Eps2: PS_MASK => [Read RSR_EPS2, Write WSR_EPS2, Exchange XSR_EPS2];
    Eps3: PS_MASK => [Read RSR_EPS3, Write WSR_EPS3, Exchange XSR_EPS3];
    Eps4: PS_MASK => [Read RSR_EPS4, Write WSR_EPS4, Exchange XSR_EPS4];
    Eps5: PS_MASK => [Read RSR_EPS5, Write WSR_EPS5, Exchange XSR_EPS5];
    Eps6: PS_MASK => [Read RSR_EPS6, Write WSR_EPS6, Exchange XSR_EPS6];
    Eps7: PS_MASK => [Read RSR_EPS7, Write WSR_EPS7, Exchange XSR_EPS7];
    Excsave1: u32::MAX => [Read RSR_EXCSAVE1, Write WSR_EXCSAVE1, Exchange XSR_EXCSAVE1];
    Excsave2: u32::MAX => [Read RSR_EXCSAVE2, Write WSR_EXCSAVE2, Exchange XSR_EXCSAVE2];
    Excsave3: u32::MAX => [Read RSR_EXCSAVE3, Write WSR_EXCSAVE3, Exchange XSR_EXCSAVE3];
    Excsave4: u32::MAX => [Read RSR_EXCSAVE4, Write WSR_EXCSAVE4, Exchange XSR_EXCSAVE4];
    Excsave5: u32::MAX => [Read RSR_EXCSAVE5, Write WSR_EXCSAVE5, Exchange XSR_EXCSAVE5];
    Excsave6: u32::MAX => [Read RSR_EXCSAVE6, Write WSR_EXCSAVE6, Exchange XSR_EXCSAVE6];
    Excsave7: u32::MAX => [Read RSR_EXCSAVE7, Write WSR_EXCSAVE7, Exchange XSR_EXCSAVE7];
    Depc: u32::MAX => [Read RSR_DEPC, Write WSR_DEPC, Exchange XSR_DEPC];
    ExcCause: 0x3f => [Read RSR_EXCCAUSE, Write WSR_EXCCAUSE, Exchange XSR_EXCCAUSE];
    ExcVaddr: u32::MAX => [Read RSR_EXCVADDR, Write WSR_EXCVADDR, Exchange XSR_EXCVADDR];
    Ccount: u32::MAX => [Read RSR_CCOUNT, Write WSR_CCOUNT, Exchange XSR_CCOUNT];
    Ccompare0: u32::MAX => [Read RSR_CCOMPARE0, Write WSR_CCOMPARE0, Exchange XSR_CCOMPARE0];
    Ccompare1: u32::MAX => [Read RSR_CCOMPARE1, Write WSR_CCOMPARE1, Exchange XSR_CCOMPARE1];
    Ccompare2: u32::MAX => [Read RSR_CCOMPARE2, Write WSR_CCOMPARE2, Exchange XSR_CCOMPARE2];
    Interrupt: u32::MAX => [Read RSR_INTERRUPT];
    IntEnable: u32::MAX => [Read RSR_INTENABLE, Write WSR_INTENABLE, Exchange XSR_INTENABLE];
    Icount: u32::MAX => [Read RSR_ICOUNT, Write WSR_ICOUNT, Exchange XSR_ICOUNT];
    IcountLevel: 0xf => [Read RSR_ICOUNTLEVEL, Write WSR_ICOUNTLEVEL, Exchange XSR_ICOUNTLEVEL];
    DebugCause: 0xf3f => [Read RSR_DEBUGCAUSE, Write WSR_DEBUGCAUSE, Exchange XSR_DEBUGCAUSE];
    CpEnable: 0xff => [Read RSR_CPENABLE, Write WSR_CPENABLE, Exchange XSR_CPENABLE];
    AtomCtl: 0x3f => [Read RSR_ATOMCTL, Write WSR_ATOMCTL, Exchange XSR_ATOMCTL];
    MemCtl: u32::MAX => [Read RSR_MEMCTL, Write WSR_MEMCTL, Exchange XSR_MEMCTL];
    Misc0: u32::MAX => [Read RSR_MISC0, Write WSR_MISC0, Exchange XSR_MISC0];
    Misc1: u32::MAX => [Read RSR_MISC1, Write WSR_MISC1, Exchange XSR_MISC1];
    Misc2: u32::MAX => [Read RSR_MISC2, Write WSR_MISC2, Exchange XSR_MISC2];
    Misc3: u32::MAX => [Read RSR_MISC3, Write WSR_MISC3, Exchange XSR_MISC3];
    Threadptr: u32::MAX => [Read RUR_THREADPTR, Write WUR_THREADPTR];
}

pub(crate) const CCOMPARE: [Sr; 3] = [Sr::Ccompare0, Sr::Ccompare1, Sr::Ccompare2];

/// `EPC1` to `EPC7` by level. Index 0 is unused.
pub(crate) const EPC: [Sr; 8] = [
    Sr::Epc1,
    Sr::Epc1,
    Sr::Epc2,
    Sr::Epc3,
    Sr::Epc4,
    Sr::Epc5,
    Sr::Epc6,
    Sr::Epc7,
];

/// `EPS2` to `EPS7` by level. Indexes 0 and 1 are unused, since level 1 saves no `PS`.
pub(crate) const EPS: [Sr; 8] = [
    Sr::Eps2,
    Sr::Eps2,
    Sr::Eps2,
    Sr::Eps3,
    Sr::Eps4,
    Sr::Eps5,
    Sr::Eps6,
    Sr::Eps7,
];
