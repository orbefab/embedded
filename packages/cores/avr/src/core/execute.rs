//! Execute a decoded [`Op`]. Data-space operands go through the bus.

use super::Core;
use super::decode::{Op, bus_op, free_op};
use crate::SpmNote;
use crate::bus::Bus;

const C: u8 = 1 << 0;
const N: u8 = 1 << 2;
const V: u8 = 1 << 3;
const S: u8 = 1 << 4;
const H: u8 = 1 << 5;
const T: u8 = 1 << 6;
const I: u8 = 1 << 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ptr {
    X,
    Y,
    Z,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Update {
    None,
    Pre,
    Post,
}

impl Core {
    /// Runs `op`, the decoded word at `pc`.
    #[inline(always)]
    pub(super) fn execute_op<B: Bus>(&mut self, bus: &mut B, pc: u16, op: Op) {
        if op.is_free() {
            self.execute_free(pc, op);
        } else {
            self.execute_bus(bus, pc, op);
        }
    }

    /// Runs `op` when it is a load of a [`Bus::steady`] address or a [`Bus::store_idle`] store,
    /// reading through [`Bus::peek`] and leaving the bus as it is. Otherwise runs nothing and
    /// returns `false`.
    #[inline(always)]
    pub(super) fn execute_steady<B: Bus>(&mut self, bus: &B, pc: u16, op: Op) -> bool {
        let next = pc.wrapping_add(1);
        match op {
            Op::In { d, addr } if bus.steady(addr) => {
                self.put(d, bus.peek(self, addr));
                self.finish(next, 1);
            }
            Op::Lds { d, addr } if bus.steady(addr) => {
                self.put(d, bus.peek(self, addr));
                self.finish(pc.wrapping_add(2), 2);
            }
            Op::Sbic { addr, b, skip } if bus.steady(addr) => {
                let set = bus.peek(self, addr) & (1 << b) != 0;
                self.skip_if(!set, next, skip);
            }
            Op::Sbis { addr, b, skip } if bus.steady(addr) => {
                let set = bus.peek(self, addr) & (1 << b) != 0;
                self.skip_if(set, next, skip);
            }
            Op::Out { r, addr } if bus.store_idle(self, addr, self.at(r)) => self.finish(next, 1),
            Op::Sts { r, addr } if bus.store_idle(self, addr, self.at(r)) => {
                self.finish(pc.wrapping_add(2), 2);
            }
            _ => return false,
        }
        true
    }

    /// Runs `op`, which [`Op::is_free`].
    #[inline(always)]
    pub(super) fn execute_free(&mut self, pc: u16, op: Op) {
        let next = pc.wrapping_add(1);
        match op {
            Op::Nop => self.finish(next, 1),
            Op::Add(d, r) => {
                let (rd, rr) = (self.at(d), self.at(r));
                let result = rd.wrapping_add(rr);
                self.put(d, result);
                self.add_flags(rd, rr, result);
                self.finish(next, 1);
            }
            Op::Adc(d, r) => {
                let (rd, rr) = (self.at(d), self.at(r));
                let result = rd.wrapping_add(rr).wrapping_add(u8::from(self.flag(0)));
                self.put(d, result);
                self.add_flags(rd, rr, result);
                self.finish(next, 1);
            }
            Op::Sub(d, r) => self.subtract(next, d, self.at(r), false, true),
            Op::Sbc(d, r) => self.subtract(next, d, self.at(r), true, true),
            Op::Cp(d, r) => self.subtract(next, d, self.at(r), false, false),
            Op::Cpc(d, r) => self.subtract(next, d, self.at(r), true, false),
            Op::Subi(d, k) => self.subtract(next, d, k, false, true),
            Op::Sbci(d, k) => self.subtract(next, d, k, true, true),
            Op::Cpi(d, k) => self.subtract(next, d, k, false, false),
            Op::And(d, r) => self.logic(next, d, self.at(d) & self.at(r)),
            Op::Or(d, r) => self.logic(next, d, self.at(d) | self.at(r)),
            Op::Eor(d, r) => self.logic(next, d, self.at(d) ^ self.at(r)),
            Op::Andi(d, k) => self.logic(next, d, self.at(d) & k),
            Op::Ori(d, k) => self.logic(next, d, self.at(d) | k),
            Op::Mov(d, r) => {
                self.put(d, self.at(r));
                self.finish(next, 1);
            }
            Op::Ldi(d, k) => {
                self.put(d, k);
                self.finish(next, 1);
            }
            Op::Cpse { d, r, skip } => self.skip_if(self.at(d) == self.at(r), next, skip),
            Op::Adiw(lo, k) => self.word_imm(next, lo, k, true),
            Op::Sbiw(lo, k) => self.word_imm(next, lo, k, false),
            Op::Movw(d, r) => {
                self.put(d, self.at(r));
                self.put(d + 1, self.at(r + 1));
                self.finish(next, 1);
            }
            Op::Mul(d, r) => {
                self.set_product(u16::from(self.at(d)) * u16::from(self.at(r)));
                self.finish(next, 2);
            }
            Op::Muls(d, r) => {
                self.set_product(signed_pair(self.at(d), self.at(r)));
                self.finish(next, 2);
            }
            Op::Mulsu(d, r) => {
                self.set_product(signed_unsigned(self.at(d), self.at(r)));
                self.finish(next, 2);
            }
            Op::Fmul(d, r) => {
                self.mul_flags_shifted(u16::from(self.at(d)) * u16::from(self.at(r)));
                self.finish(next, 2);
            }
            Op::Fmuls(d, r) => {
                self.mul_flags_shifted(signed_pair(self.at(d), self.at(r)));
                self.finish(next, 2);
            }
            Op::Fmulsu(d, r) => {
                self.mul_flags_shifted(signed_unsigned(self.at(d), self.at(r)));
                self.finish(next, 2);
            }
            Op::Lpm { d, post } => {
                self.lpm_into(d, post);
                self.finish(next, 3);
            }
            Op::Rjmp(to) => self.finish(to, 2),
            Op::Brbs { s, to } => self.branch_if(self.flag(s), next, to),
            Op::Brbc { s, to } => self.branch_if(!self.flag(s), next, to),
            Op::Countdown(c) => self.branch_if(!self.flag(1), next, c.head(pc)),
            Op::Jmp(to) => self.finish(to, 3),
            Op::Ijmp => self.finish(self.z(), 2),
            Op::Bst { d, b } => {
                self.set_flag_bit(6, (self.at(d) >> b) & 1 == 1);
                self.finish(next, 1);
            }
            Op::Bld { d, b } => {
                let value = if self.flag(6) {
                    self.at(d) | (1 << b)
                } else {
                    self.at(d) & !(1 << b)
                };
                self.put(d, value);
                self.finish(next, 1);
            }
            Op::Sbrc { r, b, skip } => self.skip_if((self.at(r) >> b) & 1 == 0, next, skip),
            Op::Sbrs { r, b, skip } => self.skip_if((self.at(r) >> b) & 1 == 1, next, skip),
            Op::Bset(s) => {
                self.set_flag_bit(s, true);
                self.finish(next, 1);
            }
            Op::Bclr(s) => {
                self.set_flag_bit(s, false);
                self.finish(next, 1);
            }
            Op::Com(d) => {
                let result = !self.at(d);
                self.put(d, result);
                let n = result >> 7;
                let z = u8::from(result == 0);
                let keep = self.sreg & (I | T | H);
                self.sreg = keep | (n << 4) | (n << 2) | (z << 1) | C;
                self.finish(next, 1);
            }
            Op::Neg(d) => {
                let rd = self.at(d);
                let result = 0u8.wrapping_sub(rd);
                self.put(d, result);
                let h = ((result >> 3) & 1) | ((rd >> 3) & 1);
                let n = result >> 7;
                let v = u8::from(result == 0x80);
                let z = u8::from(result == 0);
                let c = u8::from(result != 0);
                self.write_arith(h, n ^ v, v, n, z, c);
                self.finish(next, 1);
            }
            Op::Swap(d) => {
                self.put(d, self.at(d).rotate_left(4));
                self.finish(next, 1);
            }
            Op::Inc(d) => self.step_by_one(next, d, self.at(d).wrapping_add(1), 0x80),
            Op::Dec(d) => self.step_by_one(next, d, self.at(d).wrapping_sub(1), 0x7F),
            Op::Asr(d) => {
                let rd = self.at(d);
                self.shift(next, d, (rd >> 1) | (rd & 0x80), rd & 1);
            }
            Op::Lsr(d) => {
                let rd = self.at(d);
                self.shift(next, d, rd >> 1, rd & 1);
            }
            Op::Ror(d) => {
                let rd = self.at(d);
                self.shift(next, d, (rd >> 1) | (u8::from(self.flag(0)) << 7), rd & 1);
            }
            Op::Unknown(word) => {
                self.unknown = Some(word);
                self.finish(next, 1);
            }
            bus_op!() => unreachable!("{op:?} is not free"),
        }
    }

    #[inline(always)]
    fn execute_bus<B: Bus>(&mut self, bus: &mut B, pc: u16, op: Op) {
        let next = pc.wrapping_add(1);
        match op {
            Op::Ldd { d, ptr, q } => {
                let addr = self.ptr_val(ptr).wrapping_add(u16::from(q));
                let value = bus.load(self, addr);
                self.put(d, value);
                self.finish(next, 2);
            }
            Op::Std { r, ptr, q } => {
                let addr = self.ptr_val(ptr).wrapping_add(u16::from(q));
                let value = self.at(r);
                bus.store(self, addr, value, 2);
                self.finish(next, 2);
            }
            Op::In { d, addr } => {
                let value = bus.load(self, addr);
                self.put(d, value);
                self.finish(next, 1);
            }
            Op::Out { r, addr } => {
                let value = self.at(r);
                bus.store(self, addr, value, 1);
                self.finish(next, 1);
            }
            Op::Lds { d, addr } => {
                let value = bus.load(self, addr);
                self.put(d, value);
                self.finish(pc.wrapping_add(2), 2);
            }
            Op::Sts { r, addr } => {
                let value = self.at(r);
                bus.store(self, addr, value, 2);
                self.finish(pc.wrapping_add(2), 2);
            }
            Op::Ld { d, ptr, update } => self.load_ptr(bus, pc, d, ptr, update),
            Op::St { r, ptr, update } => self.store_ptr(bus, pc, r, ptr, update),
            Op::Pop(d) => {
                let value = self.pop(bus);
                self.put(d, value);
                self.finish(next, 2);
            }
            Op::Push(r) => {
                let value = self.at(r);
                self.push(bus, value, 2);
                self.finish(next, 2);
            }
            Op::Rcall(to) => {
                self.push_pc(bus, next, 3);
                self.finish(to, 3);
            }
            Op::Call(to) => {
                self.push_pc(bus, pc.wrapping_add(2), 4);
                self.finish(to, 4);
            }
            Op::Icall => {
                self.push_pc(bus, next, 3);
                self.finish(self.z(), 3);
            }
            Op::Ret => {
                let addr = self.pop_pc(bus);
                self.finish(addr, 4);
            }
            Op::Reti => {
                let addr = self.pop_pc(bus);
                self.set_flag_bit(7, true);
                // The instruction after RETI runs before the next interrupt.
                self.irq_delay = true;
                self.finish(addr, 4);
            }
            Op::Sbi { addr, b } => {
                bus.write_bit(self, addr, b, true);
                self.finish(next, 2);
            }
            Op::Cbi { addr, b } => {
                bus.write_bit(self, addr, b, false);
                self.finish(next, 2);
            }
            // SBIC/SBIS encode an I/O address in 0x00–0x1F (data 0x20–0x3F). `peek` is the read.
            Op::Sbic { addr, b, skip } => {
                let set = bus.peek(self, addr) & (1 << b) != 0;
                self.skip_if(!set, next, skip);
            }
            Op::Sbis { addr, b, skip } => {
                let set = bus.peek(self, addr) & (1 << b) != 0;
                self.skip_if(set, next, skip);
            }
            Op::Spm => {
                let z = self.z();
                let data = u16::from(self.at(0)) | (u16::from(self.at(1)) << 8);
                self.spm.push(SpmNote { z, word: data });
                self.finish(next, 1);
            }
            Op::Sleep => {
                // §10.1. SE must be set.
                if bus.sleep_enabled() {
                    self.set_activity(super::Activity::Sleep);
                    bus.arm_sleep();
                }
                self.finish(next, 1);
            }
            Op::Wdr => {
                self.wdr_executions = self.wdr_executions.wrapping_add(1);
                bus.watchdog_reset_timer();
                self.finish(next, 1);
            }
            Op::Break => {
                self.stopped = true;
                self.finish(next, 1);
            }
            Op::Sei => {
                self.set_flag_bit(7, true);
                // The following instruction runs before a pending interrupt.
                self.irq_delay = true;
                self.finish(next, 1);
            }
            free_op!() => unreachable!("{op:?} is free"),
        }
    }

    /// Register `d` of a decoded op, which is under 32.
    #[inline(always)]
    fn at(&self, d: u8) -> u8 {
        self.r[usize::from(d & 31)]
    }

    #[inline(always)]
    fn put(&mut self, d: u8, value: u8) {
        self.r[usize::from(d & 31)] = value;
    }

    /// `SUB`, `SBC`, `CP`, `CPC` and their immediate forms. `carry` takes `C` in, and keeps `Z`
    /// only if it was set.
    fn subtract(&mut self, next: u16, d: u8, rr: u8, carry: bool, store: bool) {
        let rd = self.at(d);
        let result = rd
            .wrapping_sub(rr)
            .wrapping_sub(u8::from(carry && self.flag(0)));
        if store {
            self.put(d, result);
        }
        self.sub_flags(rd, rr, result, carry);
        self.finish(next, 1);
    }

    fn logic(&mut self, next: u16, d: u8, result: u8) {
        self.put(d, result);
        self.logic_flags(result);
        self.finish(next, 1);
    }

    /// `INC` and `DEC`. `V` is set when the result is `overflow`. `C` and `H` are kept.
    fn step_by_one(&mut self, next: u16, d: u8, result: u8, overflow: u8) {
        self.put(d, result);
        let n = result >> 7;
        let v = u8::from(result == overflow);
        let z = u8::from(result == 0);
        let c = u8::from(self.flag(0));
        let h = u8::from(self.flag(5));
        self.write_arith(h, n ^ v, v, n, z, c);
        self.finish(next, 1);
    }

    fn shift(&mut self, next: u16, d: u8, result: u8, carry: u8) {
        self.put(d, result);
        self.shift_flags(result, carry, result >> 7);
        self.finish(next, 1);
    }

    fn word_imm(&mut self, next: u16, lo: u8, k: u8, add: bool) {
        let before = self.pair(lo);
        let after = if add {
            before.wrapping_add(u16::from(k))
        } else {
            before.wrapping_sub(u16::from(k))
        };
        let old7 = (before >> 15) & 1;
        let new7 = (after >> 15) & 1;
        let (v, c) = if add {
            ((1 - old7) & new7, (1 - new7) & old7)
        } else {
            (old7 & (1 - new7), new7 & (1 - old7))
        };
        self.set_pair(lo, after);
        let n = new7 as u8;
        let z = u8::from(after == 0);
        let s = n ^ (v as u8);
        let keep = self.sreg & (I | T | H);
        self.sreg = keep | (s << 4) | ((v as u8) << 3) | (n << 2) | (z << 1) | (c as u8);
        self.finish(next, 2);
    }

    fn branch_if(&mut self, taken: bool, next: u16, to: u16) {
        if taken {
            self.finish(to, 2);
        } else {
            self.finish(next, 1);
        }
    }

    fn skip_if(&mut self, taken: bool, next: u16, skip: u8) {
        if taken {
            self.finish(next.wrapping_add(u16::from(skip)), u64::from(skip) + 1);
        } else {
            self.finish(next, 1);
        }
    }

    fn load_ptr<B: Bus>(&mut self, bus: &mut B, pc: u16, d: u8, ptr: Ptr, update: Update) {
        let addr = self.ptr_addr(ptr, update);
        let value = bus.load(self, addr);
        if update != Update::None {
            self.set_ptr(ptr, addr);
        }
        self.put(d, value);
        if update == Update::Post {
            self.set_ptr(ptr, addr.wrapping_add(1));
        }
        self.finish(pc.wrapping_add(1), 2);
    }

    fn store_ptr<B: Bus>(&mut self, bus: &mut B, pc: u16, r: u8, ptr: Ptr, update: Update) {
        let value = self.at(r);
        let addr = self.ptr_addr(ptr, update);
        bus.store(self, addr, value, 2);
        if update == Update::Post {
            self.set_ptr(ptr, addr.wrapping_add(1));
        }
        self.finish(pc.wrapping_add(1), 2);
    }

    fn ptr_addr(&mut self, ptr: Ptr, update: Update) -> u16 {
        let current = self.ptr_val(ptr);
        if update == Update::Pre {
            let addr = current.wrapping_sub(1);
            self.set_ptr(ptr, addr);
            addr
        } else {
            current
        }
    }

    fn ptr_val(&self, ptr: Ptr) -> u16 {
        match ptr {
            Ptr::X => self.x(),
            Ptr::Y => self.y(),
            Ptr::Z => self.z(),
        }
    }

    fn set_ptr(&mut self, ptr: Ptr, value: u16) {
        let lo = match ptr {
            Ptr::X => 26,
            Ptr::Y => 28,
            Ptr::Z => 30,
        };
        self.set_pair(lo, value);
    }

    fn lpm_into(&mut self, d: u8, post: bool) {
        let z = self.z();
        let word = self.fetch(z >> 1);
        let byte = if z & 1 == 0 {
            word as u8
        } else {
            (word >> 8) as u8
        };
        self.put(d, byte);
        if post {
            self.set_pair(30, z.wrapping_add(1));
        }
    }

    pub fn enter_interrupt<B: Bus>(&mut self, bus: &mut B, word: u16, cycles: u64) {
        let return_pc = self.pc;
        self.push_pc(bus, return_pc, cycles);
        self.sreg &= !I;
        self.set_activity(super::Activity::Fetch);
        self.finish(word, cycles);
    }

    /// DS40002198 CALL / RCALL: the return address is stored with a
    /// post-decrement.
    fn push_pc<B: Bus>(&mut self, bus: &mut B, addr: u16, cycles: u64) {
        self.push(bus, addr as u8, cycles);
        self.push(bus, (addr >> 8) as u8, cycles);
    }

    /// DS40002198 RET / RETI: pre-increment.
    fn pop_pc<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let hi = self.pop(bus);
        let lo = self.pop(bus);
        u16::from(lo) | (u16::from(hi) << 8)
    }

    fn pair(&self, lo: u8) -> u16 {
        u16::from(self.r[usize::from(lo)]) | (u16::from(self.r[usize::from(lo) + 1]) << 8)
    }

    fn set_pair(&mut self, lo: u8, value: u16) {
        self.r[usize::from(lo)] = value as u8;
        self.r[usize::from(lo) + 1] = (value >> 8) as u8;
    }

    fn x(&self) -> u16 {
        self.pair(26)
    }

    fn y(&self) -> u16 {
        self.pair(28)
    }

    fn z(&self) -> u16 {
        self.pair(30)
    }

    fn flag(&self, bit: u8) -> bool {
        self.sreg & (1 << bit) != 0
    }

    fn set_flag_bit(&mut self, bit: u8, value: bool) {
        if value {
            self.sreg |= 1 << bit;
        } else {
            self.sreg &= !(1 << bit);
        }
    }

    fn write_arith(&mut self, h: u8, s: u8, v: u8, n: u8, z: u8, c: u8) {
        let keep = self.sreg & (I | T);
        self.sreg = keep | (h << 5) | (s << 4) | (v << 3) | (n << 2) | (z << 1) | c;
    }

    fn add_flags(&mut self, rd: u8, rr: u8, result: u8) {
        let rd3 = (rd >> 3) & 1;
        let rr3 = (rr >> 3) & 1;
        let r3 = (result >> 3) & 1;
        let h = (rd3 & rr3) | (rr3 & (1 - r3)) | ((1 - r3) & rd3);
        let rd7 = rd >> 7;
        let rr7 = rr >> 7;
        let r7 = result >> 7;
        let v = (rd7 & rr7 & (1 - r7)) | ((1 - rd7) & (1 - rr7) & r7);
        let c = (rd7 & rr7) | (rr7 & (1 - r7)) | ((1 - r7) & rd7);
        let n = r7;
        let z = u8::from(result == 0);
        let s = n ^ v;
        self.write_arith(h, s, v, n, z, c);
    }

    fn sub_flags(&mut self, rd: u8, rr: u8, result: u8, keep_z: bool) {
        let rd3 = (rd >> 3) & 1;
        let rr3 = (rr >> 3) & 1;
        let r3 = (result >> 3) & 1;
        let h = ((1 - rd3) & rr3) | (rr3 & r3) | (r3 & (1 - rd3));
        let rd7 = rd >> 7;
        let rr7 = rr >> 7;
        let r7 = result >> 7;
        let v = (rd7 & (1 - rr7) & (1 - r7)) | ((1 - rd7) & rr7 & r7);
        let c = ((1 - rd7) & rr7) | (rr7 & r7) | (r7 & (1 - rd7));
        let n = r7;
        let z_new = u8::from(result == 0);
        let z = if keep_z {
            z_new & u8::from(self.flag(1))
        } else {
            z_new
        };
        let s = n ^ v;
        self.write_arith(h, s, v, n, z, c);
    }

    fn logic_flags(&mut self, result: u8) {
        let n = result >> 7;
        let z = u8::from(result == 0);
        let s = n;
        let keep = self.sreg & (I | T | H | C);
        self.sreg = keep | (s << 4) | (n << 2) | (z << 1);
    }

    fn shift_flags(&mut self, result: u8, carry: u8, neg: u8) {
        let z = u8::from(result == 0);
        let v = neg ^ carry;
        let s = neg ^ v;
        let keep = self.sreg & (I | T | H);
        self.sreg = keep | (s << 4) | (v << 3) | (neg << 2) | (z << 1) | carry;
    }

    fn mul_flags(&mut self, product: u16) {
        let c = u8::from((product >> 15) & 1 == 1);
        let z = u8::from(product == 0);
        let keep = self.sreg & (I | T | H | S | V | N);
        self.sreg = keep | (z << 1) | c;
    }

    fn set_product(&mut self, product: u16) {
        self.put(0, product as u8);
        self.put(1, (product >> 8) as u8);
        self.mul_flags(product);
    }

    fn mul_flags_shifted(&mut self, unshifted: u16) {
        let shifted = unshifted << 1;
        self.put(0, shifted as u8);
        self.put(1, (shifted >> 8) as u8);
        let c = u8::from((unshifted >> 15) & 1 == 1);
        let z = u8::from(shifted == 0);
        let keep = self.sreg & (I | T | H | S | V | N);
        self.sreg = keep | (z << 1) | c;
    }
}

fn signed_pair(rd: u8, rr: u8) -> u16 {
    (i16::from(rd as i8) * i16::from(rr as i8)) as u16
}

fn signed_unsigned(rd: u8, rr: u8) -> u16 {
    (i16::from(rd as i8) * i16::from(rr)) as u16
}
