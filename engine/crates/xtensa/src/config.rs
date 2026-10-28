//! The per-chip setup. One zero-sized type per chip, so `Core<C>` monomorphizes each chip's
//! constants and decoder, and the instruction loop has no per-op chip branch.

use std::fmt;

use crate::decode::{self, Decoded, Op};

/// A chip's core configuration. The constants come from that chip's `core-isa.h`. The decoder
/// comes from that chip's generated table and names only that chip's ops.
pub trait Config: Copy + Eq + fmt::Debug + 'static {
    const NAME: &'static str;
    /// `XCHAL_EXCM_LEVEL`. `PS.EXCM` masks interrupts up to this level.
    const EXCM_LEVEL: u32;
    /// `XCHAL_DEBUGLEVEL`. Debug exceptions are taken at this level.
    const DEBUG_LEVEL: u32;
    const INTERRUPTS: Interrupts;

    /// The instruction length the first byte names, or `None` when it starts no format.
    fn insn_len(first: u8) -> Option<u8>;
    /// Decodes the instruction at the front of `bytes`, whose first byte is at `pc`. An empty
    /// slice is `None`.
    fn decode(bytes: &[u8], pc: u32) -> Option<Decoded>;
    /// The text GNU objdump prints for the instruction, without the address.
    fn render(decoded: &Decoded) -> String;
    /// Whether this chip's table has the op, so its decoder can yield it.
    fn owns(op: Op) -> bool;
    /// The coprocessor whose `CPENABLE` bit the op needs, from the chip's table: 0 for the FPU
    /// ops, and on the S3 3 for the `cop_ai` ops. The Boolean registers are shared and need
    /// none.
    fn coprocessor(op: Op) -> Option<u8>;

    /// Cycles a taken branch, jump, call or return adds to its own one. 32 taken `j` on the
    /// classic board net 144: four each, and one more for each of the 16 whose target runs into
    /// the next word.
    fn redirect_cycles() -> u64 {
        3
    }

    /// Cycles of `memw` that follows a store. Any other `memw` costs one, the same as `nop`.
    fn memw_after_store() -> u64 {
        1
    }

    /// Cycles the loop-back adds beyond the op's own one. `which` is 0 for the first loop-back
    /// after a loop op, 1 for the second, and 2 or more after that. A count of 1 never loops
    /// back. The classic board's 32-pass loop of one `addi` nets 38, which is the 33 op cycles
    /// plus 5 on the first loop-back.
    fn loopback_extra(which: u32) -> u64 {
        if which == 0 { 5 } else { 0 }
    }

    /// Cycles of one `quos`, `quou`, `rems` or `remu`. The classic core uses 6 for every operand.
    /// With the word-crossing refetch, 32 `quos` net 208, the low edge of the classic bounds of
    /// 208 to 216.
    fn divide_cycles(_follows: bool, _remainder: bool, _magnitude: u32) -> u64 {
        6
    }

    /// `ICOUNT` reaching -1 raises the debug exception even when `CINTLEVEL` is at `DEBUG_LEVEL`.
    fn icount_past_debug_level() -> bool {
        false
    }

    /// While `CINTLEVEL` is below `ICOUNTLEVEL`, `rsr.icount` and `xsr.icount` return the register
    /// minus one. [`crate::Core::special`] returns the register. The exception check and the
    /// increment use the register itself.
    fn icount_read_lag() -> bool {
        false
    }

    /// Instruction RAM or instruction ROM, where an unaligned or sub-word data access faults.
    fn instruction_bus(_addr: u32) -> bool {
        false
    }

    /// An `l32ai`, `s32ri` or `s32c1i` that is not word-aligned raises `LoadStoreAlignment`
    /// instead of stopping.
    fn sync_unaligned_excepts() -> bool {
        false
    }
}

/// A chip's 32 interrupts: each one's level and type, as `core-isa.h` gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interrupts {
    /// `XCHAL_INTn_LEVEL` by interrupt number.
    pub level: [u8; 32],
    /// `XCHAL_INTTYPE_MASK_EXTERN_LEVEL`. `INTERRUPT` shows the line itself.
    pub extern_level: u32,
    /// `XCHAL_INTTYPE_MASK_EXTERN_EDGE`. A rising line sets the bit, and `INTCLEAR` clears it.
    pub extern_edge: u32,
    /// `XCHAL_INTTYPE_MASK_NMI`. A rising line takes the NMI level whatever the masks.
    pub nmi: u32,
    /// `XCHAL_INTTYPE_MASK_SOFTWARE`. `INTSET` sets the bit, and `INTCLEAR` clears it.
    pub software: u32,
    /// `XCHAL_TIMERn_INTERRUPT`, the interrupt `CCOMPAREn` raises.
    pub timer: [u8; 3],
    /// `XCHAL_NMILEVEL`.
    pub nmi_level: u32,
}

impl Interrupts {
    /// The lines a SoC drives.
    pub const fn external(&self) -> u32 {
        self.extern_level | self.extern_edge | self.nmi
    }

    /// The interrupts at each level, indexed by level.
    pub const fn by_level(&self) -> [u32; 8] {
        let mut masks = [0; 8];
        let mut n = 0;
        while n < 32 {
            masks[self.level[n] as usize] |= 1 << n;
            n += 1;
        }
        masks
    }

    pub(crate) const fn timer_mask(&self, i: usize) -> u32 {
        1 << self.timer[i]
    }
}

/// Both chips' `core-isa.h` give the same table: the classic one at lines 373-458 and the S3's
/// at lines 383-471.
const XEA2_INTERRUPTS: Interrupts = Interrupts {
    level: [
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 3, 1, 1, 7, 3, 5, 1, 1, 2, 2, 2, 3, 3, 4, 4, 5, 3, 4, 3,
        4, 5,
    ],
    extern_level: 0x8fbe_333f,
    extern_edge: 0x5040_0400,
    nmi: 0x0000_4000,
    software: 0x2000_0080,
    timer: [6, 15, 16],
    nmi_level: 7,
};

/// The classic ESP32. Its `Op`s are the 513 of the classic table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esp32 {}

/// The ESP32-S3. Its `Op`s are the 753 of the S3 table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esp32s3 {}

impl Config for Esp32 {
    const NAME: &'static str = "esp32";
    const EXCM_LEVEL: u32 = 3;
    const DEBUG_LEVEL: u32 = 6;
    const INTERRUPTS: Interrupts = XEA2_INTERRUPTS;

    fn insn_len(first: u8) -> Option<u8> {
        decode::classic::len(first)
    }

    fn decode(bytes: &[u8], pc: u32) -> Option<Decoded> {
        decode::classic::decode(bytes, pc)
    }

    fn render(decoded: &Decoded) -> String {
        decode::classic::render(decoded)
    }

    fn owns(op: Op) -> bool {
        decode::classic::owns(op)
    }

    fn coprocessor(op: Op) -> Option<u8> {
        decode::classic::coprocessor(op)
    }
}

impl Config for Esp32s3 {
    const NAME: &'static str = "esp32s3";
    const EXCM_LEVEL: u32 = 3;
    const DEBUG_LEVEL: u32 = 6;
    const INTERRUPTS: Interrupts = XEA2_INTERRUPTS;

    fn insn_len(first: u8) -> Option<u8> {
        decode::s3::len(first)
    }

    fn decode(bytes: &[u8], pc: u32) -> Option<Decoded> {
        decode::s3::decode(bytes, pc)
    }

    fn render(decoded: &Decoded) -> String {
        decode::s3::render(decoded)
    }

    fn owns(op: Op) -> bool {
        decode::s3::owns(op)
    }

    fn coprocessor(op: Op) -> Option<u8> {
        decode::s3::coprocessor(op)
    }

    /// `j_t` in the ccount fixture, `median=00000070`. 32 taken `j` of 3 bytes from a word-aligned
    /// start cross a word 16 times, and 32 * (1 + 2) + 16 is 112.
    fn redirect_cycles() -> u64 {
        2
    }

    /// `s32i_memw` in the ccount fixture, `median=000000a0`. 32 `s32i` plus 32 `memw` net 160, so
    /// each `memw` after a store is 4. `l32i_memw` `median=00000040` keeps `memw` after a load at
    /// 1. `memw` alone, `median=00000022`, has bounds 30 to 38, which already hold a cost of 1.
    fn memw_after_store() -> u64 {
        4
    }

    /// `loop2` `median=00000004`, `loop3` `median=00000006`, `loop32` `median=00000023`. The first
    /// loop-back and the second each add 1, and a later one adds nothing. `loop1` `median=00000002`
    /// has no loop-back.
    fn loopback_extra(which: u32) -> u64 {
        u64::from(which < 2)
    }

    /// Back-to-back divides. The first costs 16. A follower's cost follows the dividend's
    /// significant-bit count, `32 - magnitude.leading_zeros()`, and a dividend of 0 has none.
    /// Up to 3 bits costs 1, up to 17 bits costs 6, and a longer dividend costs 11. The timed
    /// rows are 7 (`quos_small` `median=0000003f`), `0x10000` (`quos` `median=000000da`),
    /// and `0x7fffffff` (`quos_big` `median=00000175`). Lengths between those rows were not
    /// timed. A shorter dividend never costs more than a longer one. A remainder follower
    /// costs one more (`rems` and `remu` `median=000000f9`); the first stays 16. Signed ops
    /// use the absolute value, so `quos_neg` matches `quos`. A divisor of 1 does not change
    /// it (`quos_by1` `median=000000da`). The word-crossing refetch is unchanged, 16 of them
    /// in each of those rows.
    fn divide_cycles(follows: bool, remainder: bool, magnitude: u32) -> u64 {
        if !follows {
            return 16;
        }
        let bits = 32 - magnitude.leading_zeros();
        let steady = if bits <= 3 {
            1
        } else if bits <= 17 {
            6
        } else {
            11
        };
        steady + u64::from(remainder)
    }

    /// The hang fixtures sample `pc=40378280` with `vecbase 40378000` on every sample, for
    /// `INTLEVEL` 6 and for `INTLEVEL` 0, both with `ICOUNTLEVEL` 15. Counting stays on after the
    /// debug exception, `ICOUNT` stays -1, and the core takes the vector again. A level above
    /// debug level is not in those fixtures, and the core does not take the exception there.
    fn icount_past_debug_level() -> bool {
        true
    }

    /// `masked_m100 icount=ffffffa4`. `isync` and 8 `nop` from -100 leave the register at
    /// `ffffffa5`, and the read is one less. `level1_int1 icount=fffffffc` is the written value,
    /// read while `INTLEVEL` is not below `ICOUNTLEVEL`.
    fn icount_read_lag() -> bool {
        true
    }

    /// `iram l32 +=01` is cause 9 at `40378dc1`, `iram l16u +=00` is cause 3, `iram l8 +=01` is
    /// cause 3, and `irom l32 +=01` is cause 9 at `40000401`. The board probed `0x40378dc0` and
    /// `0x40000400`. The windows around those two addresses come from the memory map.
    /// `dram l32 +=01` and `drom l32 +=01` complete, so data RAM and data ROM are not in
    /// this range.
    fn instruction_bus(addr: u32) -> bool {
        matches!(addr, 0x4037_0000..0x403e_0000 | 0x4000_0000..0x4006_0000)
    }

    /// `dram l32ai +=01` is cause 9 at `3fcb0101` and the words stay as they were. `dram s32ri +=02`
    /// is the same. `s32c1i` is unprobed, and the ISA lists it with those two (4.6, p 168).
    fn sync_unaligned_excepts() -> bool {
        true
    }
}
