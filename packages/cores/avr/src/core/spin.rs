//! A spin that calls a pure counter, subtracts a saved start, and branches back while the
//! difference is under a limit. The passes that still branch, and that end before the run's
//! bound, advance the cycle count and leave what the last of them leaves.

use super::Core;
use super::decode::Op;
use crate::bus::Bus;

/// One taken pass of the spin. `sample` is the cycle of the counter read, counted from the call.
#[derive(Debug, Clone, Copy)]
pub(super) struct Spin {
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
    /// Low byte of the value the body returns. The body also writes the two registers above it.
    ret: u8,
    /// Register the body saves `SREG` in.
    saved: u8,
    /// Register the counter byte is read into.
    count: u8,
    /// Register that counts the shifts down to zero.
    shift_reg: u8,
}

impl Core {
    /// At the [`Op::SpinCall`] of the program's spin `index`, skips the passes that still
    /// branch and end before `bound`.
    #[cold]
    #[inline(never)]
    pub(super) fn skip_spin<B: Bus>(&mut self, bus: &B, bound: u64, index: u16) {
        if self.cycles >= bound {
            return;
        }
        let Some(&spin) = self.spins.get(usize::from(index)) else {
            return;
        };
        if self.reg(spin.zero) != 0 {
            return;
        }
        // A skipped call stores its return address. Skip only when the stack already holds it,
        // which it does once a pass has run since the last interrupt. Then the skip leaves every
        // byte of data space alone.
        let back = self.pc.wrapping_add(2);
        if !bus.store_idle(self, self.sp, back as u8)
            || !bus.store_idle(self, self.sp.wrapping_sub(1), (back >> 8) as u8)
        {
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
        // Every register a pass writes is rewritten by the next pass. Only the flags carry over,
        // into the next pass's saved status. So the last two passes give the state exactly.
        let first = k.saturating_sub(2);
        let mut counts = [0u8; 2];
        for (slot, n) in counts.iter_mut().zip(first..k) {
            let Some(count) = bus.projected_at(spin.counter, self.sample_cycle(&spin, n)) else {
                return;
            };
            *slot = count;
        }
        let (call, cycles) = (self.pc, self.cycles);
        for &count in &counts[..(k - first) as usize] {
            self.run_spin_pass(&spin, call, count, overflow);
        }
        self.pc = call;
        self.cycles = cycles.wrapping_add(k.wrapping_mul(spin.pass));
    }

    /// Leaves what one taken pass from `call` leaves, with `count` the counter byte it reads.
    /// The body's register writes are set here. The caller's subtract, compare and branch back
    /// run as ops, and they write every flag the body does not restore from the saved status.
    fn run_spin_pass(&mut self, spin: &Spin, call: u16, count: u8, overflow: u32) {
        let micros = (overflow << 8)
            .wrapping_add(u32::from(count))
            .wrapping_shl(spin.shifts);
        self.set_reg(spin.saved, self.sreg);
        self.set_reg(spin.count, count);
        self.set_reg(spin.shift_reg, 0);
        for byte in 0..4u8 {
            self.set_reg(spin.ret + byte, (micros >> (8 * u32::from(byte))) as u8);
        }
        self.set_reg(spin.ret + 4, (overflow >> 8) as u8);
        self.set_reg(spin.ret + 5, (overflow >> 16) as u8);
        self.pc = self.next_word(call, 2);
        for _ in 0..9 {
            let op = self.op(self.pc);
            self.execute_free(self.pc, op);
        }
        debug_assert_eq!(self.pc, call, "a skipped pass branches back to the call");
    }

    fn sample_cycle(&self, spin: &Spin, n: u64) -> u64 {
        self.cycles
            .wrapping_add(spin.sample)
            .wrapping_add(n.wrapping_mul(spin.pass))
    }

    /// `Some(k)` passes still compare below the limit and end before the bound. `None` when a
    /// counter read on one of those passes is not pure, so none of them are skipped.
    ///
    /// The bisection needs the passes that branch to come first. Between two events the counter
    /// moves one way, and the callee only matches when a whole counter range shifted and added to
    /// the limit stays under 2^32. So once the first pass is below the limit, the difference moves
    /// one way from there without wrapping, and it crosses the limit at most once.
    fn spin_skips<B: Bus>(
        &self,
        bus: &B,
        spin: &Spin,
        room: u64,
        start: u32,
        overflow: u32,
    ) -> Option<u64> {
        if !self.spin_below(bus, spin, 0, start, overflow)? {
            return Some(0);
        }
        let mut lo = 1u64;
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
        let count = bus.projected_at(spin.counter, self.sample_cycle(spin, n))?;
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

    fn next_word(&self, pc: u16, words: u16) -> u16 {
        pc.wrapping_add(words) & self.pc_mask
    }
}

/// The decoded table, read while the image loads.
struct Code<'a> {
    ops: &'a [Op],
    pc_mask: u16,
}

/// Replaces each `CALL` that a spin repeats with an [`Op::SpinCall`]. Returns the spins, in the
/// order their calls index them.
pub(super) fn mark(ops: &mut [Op], pc_mask: u16) -> Vec<Spin> {
    let mut spins = Vec::new();
    for at in 0..ops.len() {
        let Op::Call(to) = ops[at] else {
            continue;
        };
        let code = Code { ops, pc_mask };
        if let Some(spin) = code.spin(at as u16, to)
            && let Ok(index) = u16::try_from(spins.len())
        {
            ops[at] = Op::SpinCall { to, spin: index };
            spins.push(spin);
        }
    }
    spins
}

impl Code<'_> {
    fn op(&self, pc: u16) -> Op {
        self.ops.get(usize::from(pc)).copied().unwrap_or(Op::Nop)
    }

    fn next_word(&self, pc: u16, words: u16) -> u16 {
        pc.wrapping_add(words) & self.pc_mask
    }

    /// The three words after `pc` hold `byte(1)` to `byte(3)`, the carry ops of a four-byte
    /// subtract or add.
    fn carries(&self, pc: u16, byte: impl Fn(u8) -> Op) -> bool {
        (1..4u8).all(|i| self.op(self.next_word(pc, u16::from(i))) == byte(i))
    }

    /// The spin that the `CALL` at `call` to `dest` repeats. The caller subtracts the start from
    /// the return, compares it with the limit, and branches back to the call while it is below.
    fn spin(&self, call: u16, dest: u16) -> Option<Spin> {
        let mut pc = self.next_word(call, 2);
        let Op::Sub(d0, s0) = self.op(pc) else {
            return None;
        };
        if d0 > 28 || s0 > 28 || !self.carries(pc, |i| Op::Sbc(d0 + i, s0 + i)) {
            return None;
        }
        pc = self.next_word(pc, 4);
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
        let Op::Ldi(shift_reg, shifts) = self.op(pc) else {
            return None;
        };
        if !(1..32).contains(&shifts) {
            return None;
        }
        pc = self.next_word(pc, 1);
        cycles += 1;
        let loop_at = pc;
        if self.op(pc) != Op::Add(d0, d0) || !self.carries(pc, |i| Op::Adc(d0 + i, d0 + i)) {
            return None;
        }
        pc = self.next_word(pc, 4);
        if self.op(pc) != Op::Dec(shift_reg) {
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
        let limit = u32::from(lo) | (u32::from(hi) << 8);
        // Every counter byte, shifted, plus the limit fits in 32 bits, so the difference moves
        // one way between two events. See `spin_skips`.
        if (255u64 << shifts) + u64::from(limit) >= 1 << 32 {
            return None;
        }
        if cycles <= sample || !distinct(s0, zero, saved, count, d0, shift_reg) {
            return None;
        }
        Some(Spin {
            pass: cycles,
            sample,
            counter,
            overflow,
            start: s0,
            limit,
            shifts: u32::from(shifts),
            flag,
            flag_bit: b,
            zero,
            ret: d0,
            saved,
            count,
            shift_reg,
        })
    }
}

/// The registers the body writes are `saved`, `count`, `shift_reg` and the six from `ret` up.
/// They have to be distinct, so each ends a pass holding the value a skip sets, and none of them
/// may be the start or the zero register, which a pass only reads.
fn distinct(start: u8, zero: u8, saved: u8, count: u8, ret: u8, shift_reg: u8) -> bool {
    let bit = |reg: u8| 1u32 << reg;
    let span = |lo: u8, n: u8| (lo..lo + n).fold(0u32, |mask, reg| mask | bit(reg));
    let written = [bit(saved), bit(count), bit(shift_reg), span(ret, 6)];
    let all = written.iter().fold(0u32, |mask, w| mask | w);
    all.count_ones() == 9 && all & (span(start, 4) | bit(zero)) == 0
}
