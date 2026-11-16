//! A spin that calls a pure counter, subtracts a saved start, and branches back while the
//! difference is under a limit. The passes that still branch, and that end before the run's
//! bound, advance the cycle count and leave the call in place.

use super::Core;
use super::decode::Op;
use crate::bus::Bus;

/// One taken pass of the spin. `sample` is the cycle of the counter read, counted from the call.
struct Spin {
    pass: u64,
    sample: u64,
    counter: u16,
    /// First of four steady bytes, low byte first. The word is the overflow count.
    overflow: u16,
    /// Low byte of the saved start. Four registers, low byte first.
    start: u8,
    limit: u32,
    shifts: u32,
    flag: u16,
    flag_bit: u8,
    /// Register the body adds as zero. It has to read as zero.
    zero: u8,
}

impl Core {
    pub(super) fn skip_spin<B: Bus>(&mut self, bus: &B, bound: u64, dest: u16) {
        if self.cycles >= bound {
            return;
        }
        let Some(spin) = self.spin_at(dest) else {
            return;
        };
        if self.reg(spin.zero) != 0 {
            return;
        }
        let Some(overflow) = self.overflow_word(bus, spin.overflow) else {
            return;
        };
        if !bus.steady(spin.flag) || bus.peek(self, spin.flag) & (1 << spin.flag_bit) != 0 {
            return;
        }
        let room = (bound - 1 - self.cycles) / spin.pass;
        if room == 0 {
            return;
        }
        let start = self.word_at(spin.start);
        let Some(k) = self.spin_skips(bus, &spin, room, start, overflow) else {
            return;
        };
        if k == 0 {
            return;
        }
        self.cycles = self.cycles.wrapping_add(k.wrapping_mul(spin.pass));
    }

    /// `Some(k)` passes still compare below the limit and end before the bound. `None` when a
    /// counter read on one of those passes is not pure, so none of them are skipped.
    fn spin_skips<B: Bus>(
        &self,
        bus: &B,
        spin: &Spin,
        room: u64,
        start: u32,
        overflow: u32,
    ) -> Option<u64> {
        let mut lo = 0u64;
        let mut hi = room;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.spin_below(bus, spin, mid, start, overflow)? {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Some(lo)
    }

    /// The compare at pass `n` still branches. `None` when the counter read is not pure.
    fn spin_below<B: Bus>(
        &self,
        bus: &B,
        spin: &Spin,
        n: u64,
        start: u32,
        overflow: u32,
    ) -> Option<bool> {
        let at = self
            .cycles
            .wrapping_add(spin.sample)
            .wrapping_add(n.wrapping_mul(spin.pass));
        let count = bus.projected_at(spin.counter, at)?;
        let micros = (overflow << 8)
            .wrapping_add(u32::from(count))
            .wrapping_shl(spin.shifts);
        Some(micros.wrapping_sub(start) < spin.limit)
    }

    fn overflow_word<B: Bus>(&self, bus: &B, addr: u16) -> Option<u32> {
        let mut word = 0u32;
        for byte in 0..4u16 {
            let at = addr.wrapping_add(byte);
            if !bus.steady(at) {
                return None;
            }
            word |= u32::from(bus.peek(self, at)) << (8 * u32::from(byte));
        }
        Some(word)
    }

    fn word_at(&self, lo: u8) -> u32 {
        (0..4u8).fold(0u32, |word, byte| {
            word | (u32::from(self.reg(lo + byte)) << (8 * u32::from(byte)))
        })
    }

    fn spin_at(&self, dest: u16) -> Option<Spin> {
        let call = self.pc;
        let mut pc = call.wrapping_add(2) & self.pc_mask;
        let Op::Sub(d0, s0) = self.op(pc) else {
            return None;
        };
        if d0 > 28 || s0 > 28 {
            return None;
        }
        pc = self.next_word(pc, 1);
        for i in 1..4u8 {
            let Op::Sbc(d, s) = self.op(pc) else {
                return None;
            };
            if d != d0 + i || s != s0 + i {
                return None;
            }
            pc = self.next_word(pc, 1);
        }
        let Op::Cpi(d, lo) = self.op(pc) else {
            return None;
        };
        if d != d0 {
            return None;
        }
        pc = self.next_word(pc, 1);
        let Op::Sbci(d, hi) = self.op(pc) else {
            return None;
        };
        if d != d0 + 1 {
            return None;
        }
        pc = self.next_word(pc, 1);
        let Op::Cpc(d, zero) = self.op(pc) else {
            return None;
        };
        if d != d0 + 2 {
            return None;
        }
        pc = self.next_word(pc, 1);
        let Op::Cpc(d, zero2) = self.op(pc) else {
            return None;
        };
        if d != d0 + 3 || zero2 != zero {
            return None;
        }
        pc = self.next_word(pc, 1);
        let Op::Brbs { s: 0, to } = self.op(pc) else {
            return None;
        };
        if to != call {
            return None;
        }
        self.callee(dest, d0, s0, zero, lo, hi)
    }

    /// The call target. `d0` is the low byte of the value it returns. The clear-flag path is
    /// `IN`, `CLI`, four loads, the counter `IN`, a skip that falls into a jump, then the
    /// multiply. A set flag takes another path, and the caller skips nothing then.
    fn callee(&self, dest: u16, d0: u8, s0: u8, zero: u8, lo: u8, hi: u8) -> Option<Spin> {
        let mut pc = dest & self.pc_mask;
        // The call is 4. The eight subtracts and the taken branch back are added after the body.
        let mut cycles = 4u64;
        let Op::In {
            d: saved,
            addr: sreg,
        } = self.op(pc)
        else {
            return None;
        };
        pc = self.next_word(pc, 1);
        cycles += 1;
        if self.op(pc) != Op::Bclr(7) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        let Op::Lds {
            d: v0,
            addr: overflow,
        } = self.op(pc)
        else {
            return None;
        };
        if v0 > 28 || overflow.checked_add(3).is_none() {
            return None;
        }
        pc = self.next_word(pc, 2);
        cycles += 2;
        for i in 1..4u8 {
            let Op::Lds { d, addr } = self.op(pc) else {
                return None;
            };
            if d != v0 + i || addr != overflow + u16::from(i) {
                return None;
            }
            pc = self.next_word(pc, 2);
            cycles += 2;
        }
        let sample = cycles;
        let Op::In {
            d: count,
            addr: counter,
        } = self.op(pc)
        else {
            return None;
        };
        pc = self.next_word(pc, 1);
        cycles += 1;
        let Op::Sbis { addr: flag, b, .. } = self.op(pc) else {
            return None;
        };
        pc = self.next_word(pc, 1);
        cycles += 1;
        let Op::Rjmp(epilogue) = self.op(pc) else {
            return None;
        };
        pc = epilogue & self.pc_mask;
        cycles += 2;
        if self.op(pc)
            != (Op::Out {
                r: saved,
                addr: sreg,
            })
        {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        if self.op(pc) != Op::Mov(v0 + 3, v0 + 2) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        if self.op(pc) != Op::Mov(v0 + 2, v0 + 1) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        if self.op(pc) != Op::Mov(v0 + 1, v0) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        if self.op(pc) != Op::Eor(v0, v0) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        // The low half of the return is the shifted overflow. Its high half is `v0:v0+1`.
        if v0 != d0 + 2 || self.op(pc) != Op::Movw(d0, v0) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        if self.op(pc) != Op::Movw(v0, v0 + 2) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        if self.op(pc) != Op::Add(d0, count) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        for i in 1..4u8 {
            if self.op(pc) != Op::Adc(d0 + i, zero) {
                return None;
            }
            pc = self.next_word(pc, 1);
            cycles += 1;
        }
        let Op::Ldi(counter_reg, shifts) = self.op(pc) else {
            return None;
        };
        if !(1..32).contains(&shifts) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        let loop_at = pc;
        if self.op(pc) != Op::Add(d0, d0) {
            return None;
        }
        pc = self.next_word(pc, 1);
        for i in 1..4u8 {
            if self.op(pc) != Op::Adc(d0 + i, d0 + i) {
                return None;
            }
            pc = self.next_word(pc, 1);
        }
        if self.op(pc) != Op::Dec(counter_reg) {
            return None;
        }
        pc = self.next_word(pc, 1);
        let Op::Brbc { s: 1, to } = self.op(pc) else {
            return None;
        };
        if to != loop_at {
            return None;
        }
        pc = self.next_word(pc, 1);
        if self.op(pc) != Op::Ret {
            return None;
        }
        // A taken pass is four adds, the decrement and a taken branch (7). The last branch
        // falls through (6), and `RET` is 4. The compare after the return is 8, plus the taken
        // branch back (2).
        cycles += u64::from(shifts - 1) * 7 + 6 + 4 + 10;
        if cycles <= sample || clobbers(s0, zero, saved, v0, count, d0, counter_reg) {
            return None;
        }
        Some(Spin {
            pass: cycles,
            sample,
            counter,
            overflow,
            start: s0,
            limit: u32::from(lo) | (u32::from(hi) << 8),
            shifts: u32::from(shifts),
            flag,
            flag_bit: b,
            zero,
        })
    }

    fn next_word(&self, pc: u16, words: u16) -> u16 {
        pc.wrapping_add(words) & self.pc_mask
    }
}

/// A skipped pass has to leave the start and the zero register alone. The status byte is saved
/// and restored, so that register is a clobber only when the start lives there or the body writes
/// it before the restore.
fn clobbers(start: u8, zero: u8, saved: u8, v0: u8, count: u8, d0: u8, counter_reg: u8) -> bool {
    for reg in start..start + 4 {
        if reg == saved
            || (v0..v0 + 4).contains(&reg)
            || reg == count
            || (d0..d0 + 4).contains(&reg)
            || reg == counter_reg
        {
            return true;
        }
    }
    saved == zero
        || (v0..v0 + 4).contains(&zero)
        || count == zero
        || (d0..d0 + 4).contains(&zero)
        || counter_reg == zero
        || (d0..d0 + 4).contains(&counter_reg)
}
