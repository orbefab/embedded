//! Decode and execute. Data-space operands go through the bus.
//! `IN` / `OUT` / `SBI` / `CBI` add [`IO_BASE`] to the I/O address in the word.

use super::Core;
use crate::SpmNote;
use crate::bus::{Bus, IO_BASE};

const C: u8 = 1 << 0;
const N: u8 = 1 << 2;
const V: u8 = 1 << 3;
const S: u8 = 1 << 4;
const H: u8 = 1 << 5;
const T: u8 = 1 << 6;
const I: u8 = 1 << 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ptr {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Update {
    None,
    Pre,
    Post,
}

impl Core {
    pub(super) fn execute(&mut self, bus: &mut Bus, pc: u16, word: u16) {
        if word == 0x0000 {
            self.finish(pc.wrapping_add(1), 1);
            return;
        }
        if self.alu_reg(pc, word) || self.alu_imm(pc, word) || self.mul(pc, word) {
            return;
        }
        if self.transfer(bus, pc, word) || self.branch(bus, pc, word) || self.bitop(bus, pc, word) {
            return;
        }
        self.unknown = Some(word);
        self.finish(pc.wrapping_add(1), 1);
    }

    fn alu_reg(&mut self, pc: u16, word: u16) -> bool {
        let (d, r) = rd_rr(word);
        let rd = self.reg(d);
        let rr = self.reg(r);
        let next = pc.wrapping_add(1);
        let kind = word & 0xFC00;
        match kind {
            0x0C00 => {
                let result = rd.wrapping_add(rr);
                self.set_reg(d, result);
                self.add_flags(rd, rr, result);
                self.finish(next, 1);
            }
            0x1C00 => {
                let result = rd.wrapping_add(rr).wrapping_add(u8::from(self.flag(0)));
                self.set_reg(d, result);
                self.add_flags(rd, rr, result);
                self.finish(next, 1);
            }
            0x1800 => {
                let result = rd.wrapping_sub(rr);
                self.set_reg(d, result);
                self.sub_flags(rd, rr, result, false);
                self.finish(next, 1);
            }
            0x0800 => {
                let result = rd.wrapping_sub(rr).wrapping_sub(u8::from(self.flag(0)));
                self.set_reg(d, result);
                self.sub_flags(rd, rr, result, true);
                self.finish(next, 1);
            }
            0x2000 => {
                let result = rd & rr;
                self.set_reg(d, result);
                self.logic_flags(result);
                self.finish(next, 1);
            }
            0x2800 => {
                let result = rd | rr;
                self.set_reg(d, result);
                self.logic_flags(result);
                self.finish(next, 1);
            }
            0x2400 => {
                let result = rd ^ rr;
                self.set_reg(d, result);
                self.logic_flags(result);
                self.finish(next, 1);
            }
            0x2C00 => {
                self.set_reg(d, rr);
                self.finish(next, 1);
            }
            0x1400 => {
                let result = rd.wrapping_sub(rr);
                self.sub_flags(rd, rr, result, false);
                self.finish(next, 1);
            }
            0x0400 => {
                let result = rd.wrapping_sub(rr).wrapping_sub(u8::from(self.flag(0)));
                self.sub_flags(rd, rr, result, true);
                self.finish(next, 1);
            }
            0x1000 => {
                if rd == rr {
                    let extra = self.skip_len(next);
                    self.finish(next.wrapping_add(extra), u64::from(extra) + 1);
                } else {
                    self.finish(next, 1);
                }
            }
            _ => return false,
        }
        true
    }

    fn alu_imm(&mut self, pc: u16, word: u16) -> bool {
        let next = pc.wrapping_add(1);
        let top = word & 0xF000;
        let (d, k) = rd_k(word);
        let rd = self.reg(d);
        let handled = match top {
            0x5000 => {
                let result = rd.wrapping_sub(k);
                self.set_reg(d, result);
                self.sub_flags(rd, k, result, false);
                true
            }
            0x4000 => {
                let result = rd.wrapping_sub(k).wrapping_sub(u8::from(self.flag(0)));
                self.set_reg(d, result);
                self.sub_flags(rd, k, result, true);
                true
            }
            0x7000 => {
                let result = rd & k;
                self.set_reg(d, result);
                self.logic_flags(result);
                true
            }
            0x6000 => {
                let result = rd | k;
                self.set_reg(d, result);
                self.logic_flags(result);
                true
            }
            0xE000 => {
                self.set_reg(d, k);
                true
            }
            0x3000 => {
                let result = rd.wrapping_sub(k);
                self.sub_flags(rd, k, result, false);
                true
            }
            _ => false,
        };
        if handled {
            self.finish(next, 1);
            return true;
        }
        if (word & 0xFF00) == 0x9600 || (word & 0xFF00) == 0x9700 {
            let dd = ((word >> 4) & 0x3) as u8;
            let lo = 24 + dd * 2;
            let k = ((word >> 2) & 0x30) | (word & 0x0F);
            let before = self.pair(lo);
            let add = (word & 0xFF00) == 0x9600;
            let after = if add {
                before.wrapping_add(k)
            } else {
                before.wrapping_sub(k)
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
            return true;
        }
        if (word & 0xFF00) == 0x0100 {
            let d = (((word >> 4) & 0xF) as u8) * 2;
            let r = ((word & 0xF) as u8) * 2;
            self.set_reg(d, self.reg(r));
            self.set_reg(d + 1, self.reg(r + 1));
            self.finish(next, 1);
            return true;
        }
        false
    }

    fn mul(&mut self, pc: u16, word: u16) -> bool {
        let next = pc.wrapping_add(1);
        if (word & 0xFC00) == 0x9C00 {
            let (d, r) = rd_rr(word);
            let product = u16::from(self.reg(d)) * u16::from(self.reg(r));
            self.set_product(product);
            self.finish(next, 2);
            return true;
        }
        if (word & 0xFF00) == 0x0200 {
            let d = 16 + ((word >> 4) & 0xF) as u8;
            let r = 16 + (word & 0xF) as u8;
            let product = i16::from(self.reg(d) as i8) * i16::from(self.reg(r) as i8);
            self.set_product(product as u16);
            self.finish(next, 2);
            return true;
        }
        if (word & 0xFF00) == 0x0300 {
            let d = 16 + ((word >> 4) & 0x7) as u8;
            let r = 16 + (word & 0x7) as u8;
            let rd = self.reg(d);
            let rr = self.reg(r);
            let high = (word & 0x0080) != 0;
            let bit3 = (word & 0x0008) != 0;
            let wide = match (high, bit3) {
                (false, false) | (true, true) => signed_unsigned(rd, rr),
                (false, true) => u16::from(rd) * u16::from(rr),
                (true, false) => signed_pair(rd, rr),
            };
            if high || bit3 {
                self.mul_flags_shifted(wide);
            } else {
                self.set_product(wide);
            }
            self.finish(next, 2);
            return true;
        }
        false
    }

    fn transfer(&mut self, bus: &mut Bus, pc: u16, word: u16) -> bool {
        if self.displacement(bus, pc, word) {
            return true;
        }
        if (word & 0xFC00) != 0x9000 {
            return self.in_out(bus, pc, word);
        }
        let d = ((word >> 4) & 0x1F) as u8;
        let st = (word & 0x0200) != 0;
        let next = pc.wrapping_add(1);
        match word & 0x000F {
            0x0 if !st => {
                let k = self.fetch(next);
                let value = bus.load(self, k);
                self.set_reg(d, value);
                self.finish(pc.wrapping_add(2), 2);
            }
            0x0 => {
                let k = self.fetch(next);
                let value = self.reg(d);
                bus.store(self, k, value, 2);
                self.finish(pc.wrapping_add(2), 2);
            }
            0x1 if !st => self.load_ptr(bus, pc, d, Ptr::Z, Update::Post),
            0x1 => self.store_ptr(bus, pc, d, Ptr::Z, Update::Post),
            0x2 if !st => self.load_ptr(bus, pc, d, Ptr::Z, Update::Pre),
            0x2 => self.store_ptr(bus, pc, d, Ptr::Z, Update::Pre),
            0x4 if !st => {
                self.lpm_into(d, false);
                self.finish(next, 3);
            }
            0x5 if !st => {
                self.lpm_into(d, true);
                self.finish(next, 3);
            }
            0x9 if !st => self.load_ptr(bus, pc, d, Ptr::Y, Update::Post),
            0x9 => self.store_ptr(bus, pc, d, Ptr::Y, Update::Post),
            0xA if !st => self.load_ptr(bus, pc, d, Ptr::Y, Update::Pre),
            0xA => self.store_ptr(bus, pc, d, Ptr::Y, Update::Pre),
            0xC if !st => self.load_ptr(bus, pc, d, Ptr::X, Update::None),
            0xC => self.store_ptr(bus, pc, d, Ptr::X, Update::None),
            0xD if !st => self.load_ptr(bus, pc, d, Ptr::X, Update::Post),
            0xD => self.store_ptr(bus, pc, d, Ptr::X, Update::Post),
            0xE if !st => self.load_ptr(bus, pc, d, Ptr::X, Update::Pre),
            0xE => self.store_ptr(bus, pc, d, Ptr::X, Update::Pre),
            0xF if !st => {
                let value = self.pop(bus);
                self.set_reg(d, value);
                self.finish(next, 2);
            }
            0xF => {
                let value = self.reg(d);
                self.push(bus, value, 2);
                self.finish(next, 2);
            }
            _ => return false,
        }
        true
    }

    fn displacement(&mut self, bus: &mut Bus, pc: u16, word: u16) -> bool {
        let y = (word & 0xD208) == 0x8008 || (word & 0xD208) == 0x8208;
        let z = (word & 0xD208) == 0x8000 || (word & 0xD208) == 0x8200;
        if !y && !z {
            return false;
        }
        // DS40002198 LDD/STD: q is bit 13, bits 11:10, and bits 2:0.
        let q = ((word & 0x2000) >> 8) | ((word & 0x0C00) >> 7) | (word & 0x0007);
        let reg = ((word >> 4) & 0x1F) as u8;
        let base = if y { self.y() } else { self.z() };
        let addr = base.wrapping_add(q);
        if (word & 0x0200) == 0 {
            let value = bus.load(self, addr);
            self.set_reg(reg, value);
        } else {
            let value = self.reg(reg);
            bus.store(self, addr, value, 2);
        }
        self.finish(pc.wrapping_add(1), 2);
        true
    }

    fn in_out(&mut self, bus: &mut Bus, pc: u16, word: u16) -> bool {
        let kind = word & 0xF800;
        if kind != 0xB000 && kind != 0xB800 {
            return false;
        }
        let reg = ((word >> 4) & 0x1F) as u8;
        let a = (((word >> 5) & 0x30) | (word & 0x0F)).wrapping_add(IO_BASE);
        if kind == 0xB000 {
            let value = bus.load(self, a);
            self.set_reg(reg, value);
        } else {
            let value = self.reg(reg);
            bus.store(self, a, value, 1);
        }
        self.finish(pc.wrapping_add(1), 1);
        true
    }

    fn load_ptr(&mut self, bus: &mut Bus, pc: u16, d: u8, ptr: Ptr, update: Update) {
        let addr = self.ptr_addr(ptr, update);
        let value = bus.load(self, addr);
        if update != Update::None {
            self.set_ptr(ptr, addr);
        }
        self.set_reg(d, value);
        if update == Update::Post {
            self.set_ptr(ptr, addr.wrapping_add(1));
        }
        self.finish(pc.wrapping_add(1), 2);
    }

    fn store_ptr(&mut self, bus: &mut Bus, pc: u16, r: u8, ptr: Ptr, update: Update) {
        let value = self.reg(r);
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
        self.set_reg(d, byte);
        if post {
            self.set_pair(30, z.wrapping_add(1));
        }
    }

    fn branch(&mut self, bus: &mut Bus, pc: u16, word: u16) -> bool {
        let next = pc.wrapping_add(1);
        let top = word & 0xF000;
        if top == 0xC000 {
            let k = sext12(word);
            self.finish(pc.wrapping_add(1).wrapping_add(k), 2);
            return true;
        }
        if top == 0xD000 {
            let k = sext12(word);
            self.push_pc(bus, next, 3);
            self.finish(pc.wrapping_add(1).wrapping_add(k), 3);
            return true;
        }
        if (word & 0xFC00) == 0xF000 || (word & 0xFC00) == 0xF400 {
            let s = (word & 0x7) as u8;
            let k = sext7((word >> 3) & 0x7F);
            let want_set = (word & 0x0400) == 0;
            if self.flag(s) == want_set {
                self.finish(next.wrapping_add(k), 2);
            } else {
                self.finish(next, 1);
            }
            return true;
        }
        if (word & 0xFE0E) == 0x940C {
            let k = self.long_addr(self.fetch(next));
            self.finish(k, 3);
            return true;
        }
        if (word & 0xFE0E) == 0x940E {
            let k = self.long_addr(self.fetch(next));
            self.push_pc(bus, pc.wrapping_add(2), 4);
            self.finish(k, 4);
            return true;
        }
        match word {
            0x9409 => {
                self.finish(self.z(), 2);
                true
            }
            0x9509 => {
                self.push_pc(bus, next, 3);
                self.finish(self.z(), 3);
                true
            }
            0x9508 => {
                let addr = self.pop_pc(bus);
                self.finish(addr, 4);
                true
            }
            0x9518 => {
                let addr = self.pop_pc(bus);
                self.set_flag_bit(7, true);
                // The instruction after RETI runs before the next interrupt.
                self.irq_delay = true;
                self.finish(addr, 4);
                true
            }
            _ => false,
        }
    }

    fn bitop(&mut self, bus: &mut Bus, pc: u16, word: u16) -> bool {
        let next = pc.wrapping_add(1);
        if (word & 0xFE08) == 0xFA00 || (word & 0xFE08) == 0xF800 {
            let d = ((word >> 4) & 0x1F) as u8;
            let b = (word & 0x7) as u8;
            if (word & 0x0200) != 0 {
                let bit = (self.reg(d) >> b) & 1 == 1;
                self.set_flag_bit(6, bit);
            } else {
                let mut value = self.reg(d);
                if self.flag(6) {
                    value |= 1 << b;
                } else {
                    value &= !(1 << b);
                }
                self.set_reg(d, value);
            }
            self.finish(next, 1);
            return true;
        }
        if (word & 0xFE08) == 0xFC00 || (word & 0xFE08) == 0xFE00 {
            let r = ((word >> 4) & 0x1F) as u8;
            let b = (word & 0x7) as u8;
            let set = (self.reg(r) >> b) & 1 == 1;
            let want = (word & 0x0200) != 0;
            if set == want {
                let extra = self.skip_len(next);
                self.finish(next.wrapping_add(extra), u64::from(extra) + 1);
            } else {
                self.finish(next, 1);
            }
            return true;
        }
        if (word & 0xFF00) == 0x9800
            || (word & 0xFF00) == 0x9A00
            || (word & 0xFF00) == 0x9900
            || (word & 0xFF00) == 0x9B00
        {
            let a = (word >> 3) & 0x1F;
            let b = (word & 0x7) as u8;
            // SBI/CBI/SBIC/SBIS encode an I/O address in 0x00–0x1F (data
            // 0x20–0x3F) and UDR0 is data 0xC6, so the DS40002061B §20.11.1
            // note that SBIC/SBIS on UDR pops the FIFO cannot apply. `peek` is
            // the right read.
            let addr = a + IO_BASE;
            let value = bus.peek(self, addr);
            let set = value & (1 << b) != 0;
            match word & 0xFF00 {
                0x9A00 => {
                    bus.write_bit(self, addr, b, true);
                    self.finish(next, 2);
                }
                0x9800 => {
                    bus.write_bit(self, addr, b, false);
                    self.finish(next, 2);
                }
                0x9900 | 0x9B00 => {
                    let want = (word & 0x0200) != 0;
                    if set == want {
                        let extra = self.skip_len(next);
                        self.finish(next.wrapping_add(extra), u64::from(extra) + 1);
                    } else {
                        self.finish(next, 1);
                    }
                }
                _ => return false,
            }
            return true;
        }
        if (word & 0xFF0F) == 0x9408 {
            let s = ((word >> 4) & 0x7) as u8;
            let set = (word & 0x0080) == 0;
            self.set_flag_bit(s, set);
            // SEI is BSET 7. The following instruction runs before a pending interrupt.
            if s == 7 && set {
                self.irq_delay = true;
            }
            self.finish(next, 1);
            return true;
        }
        if (word & 0xFE00) == 0x9400 {
            return self.single_reg(bus, pc, word);
        }
        false
    }

    fn single_reg(&mut self, bus: &mut Bus, pc: u16, word: u16) -> bool {
        let d = ((word >> 4) & 0x1F) as u8;
        let rd = self.reg(d);
        let next = pc.wrapping_add(1);
        match word & 0x000F {
            0x0 => {
                let result = !rd;
                self.set_reg(d, result);
                let n = result >> 7;
                let z = u8::from(result == 0);
                let keep = self.sreg & (I | T | H);
                self.sreg = keep | (n << 4) | (n << 2) | (z << 1) | C;
            }
            0x1 => {
                let result = 0u8.wrapping_sub(rd);
                self.set_reg(d, result);
                let r3 = (result >> 3) & 1;
                let rd3 = (rd >> 3) & 1;
                let h = r3 | rd3;
                let n = result >> 7;
                let v = u8::from(result == 0x80);
                let z = u8::from(result == 0);
                let c = u8::from(result != 0);
                let s = n ^ v;
                self.write_arith(h, s, v, n, z, c);
            }
            0x2 => {
                self.set_reg(d, rd.rotate_left(4));
            }
            0x3 => {
                let result = rd.wrapping_add(1);
                self.set_reg(d, result);
                let n = result >> 7;
                let v = u8::from(result == 0x80);
                let z = u8::from(result == 0);
                let s = n ^ v;
                let c = u8::from(self.flag(0));
                let h = u8::from(self.flag(5));
                self.write_arith(h, s, v, n, z, c);
            }
            0x5 => {
                let carry = rd & 1;
                let result = (rd >> 1) | (rd & 0x80);
                self.set_reg(d, result);
                self.shift_flags(result, carry, result >> 7);
            }
            0x6 => {
                let carry = rd & 1;
                let result = rd >> 1;
                self.set_reg(d, result);
                self.shift_flags(result, carry, 0);
            }
            0x7 => {
                let carry = rd & 1;
                let result = (rd >> 1) | (u8::from(self.flag(0)) << 7);
                self.set_reg(d, result);
                self.shift_flags(result, carry, result >> 7);
            }
            0xA => {
                let result = rd.wrapping_sub(1);
                self.set_reg(d, result);
                let n = result >> 7;
                let v = u8::from(result == 0x7F);
                let z = u8::from(result == 0);
                let s = n ^ v;
                let c = u8::from(self.flag(0));
                let h = u8::from(self.flag(5));
                self.write_arith(h, s, v, n, z, c);
            }
            _ => return self.control(bus, pc, word),
        }
        self.finish(next, 1);
        true
    }

    fn control(&mut self, bus: &mut Bus, pc: u16, word: u16) -> bool {
        let next = pc.wrapping_add(1);
        match word {
            0x95C8 => {
                self.lpm_into(0, false);
                self.finish(next, 3);
                true
            }
            0x95E8 => {
                let z = self.z();
                let data = u16::from(self.reg(0)) | (u16::from(self.reg(1)) << 8);
                self.spm.push(SpmNote { z, word: data });
                self.finish(next, 1);
                true
            }
            0x9588 => {
                // §10.1. SE must be set.
                if bus.sleep_enabled() {
                    self.set_activity(super::Activity::Sleep);
                    bus.arm_sleep();
                }
                self.finish(next, 1);
                true
            }
            0x95A8 => {
                self.wdr_executions = self.wdr_executions.wrapping_add(1);
                bus.watchdog_reset_timer();
                self.finish(next, 1);
                true
            }
            0x9598 => {
                self.stopped = true;
                self.finish(next, 1);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn enter_interrupt(&mut self, bus: &mut Bus, word: u16, cycles: u64) {
        let return_pc = self.pc;
        self.push_pc(bus, return_pc, cycles);
        self.sreg &= !I;
        self.set_activity(super::Activity::Fetch);
        self.finish(word, cycles);
    }

    /// DS40002198 CALL / RCALL: the return address is stored with a
    /// post-decrement.
    fn push_pc(&mut self, bus: &mut Bus, addr: u16, cycles: u64) {
        self.push(bus, addr as u8, cycles);
        self.push(bus, (addr >> 8) as u8, cycles);
    }

    /// DS40002198 RET / RETI: pre-increment.
    fn pop_pc(&mut self, bus: &mut Bus) -> u16 {
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
        self.set_reg(0, product as u8);
        self.set_reg(1, (product >> 8) as u8);
        self.mul_flags(product);
    }

    fn mul_flags_shifted(&mut self, unshifted: u16) {
        let shifted = unshifted << 1;
        self.set_reg(0, shifted as u8);
        self.set_reg(1, (shifted >> 8) as u8);
        let c = u8::from((unshifted >> 15) & 1 == 1);
        let z = u8::from(shifted == 0);
        let keep = self.sreg & (I | T | H | S | V | N);
        self.sreg = keep | (z << 1) | c;
    }

    fn two_word(&self, pc: u16) -> bool {
        let word = self.fetch(pc);
        (word & 0xFE0F) == 0x9000
            || (word & 0xFE0F) == 0x9200
            || (word & 0xFE0E) == 0x940C
            || (word & 0xFE0E) == 0x940E
    }

    fn skip_len(&self, next: u16) -> u16 {
        if self.two_word(next) { 2 } else { 1 }
    }

    fn long_addr(&self, second: u16) -> u16 {
        // Bits 21:16 of a `JMP` / `CALL` sit in the first word. This part has a 14-bit PC.
        second & self.pc_mask
    }
}

fn rd_rr(word: u16) -> (u8, u8) {
    let d = ((word >> 4) & 0x1F) as u8;
    let r = (((word >> 5) & 0x10) | (word & 0x0F)) as u8;
    (d, r)
}

fn rd_k(word: u16) -> (u8, u8) {
    let d = 16 + ((word >> 4) & 0x0F) as u8;
    let k = (((word >> 4) & 0xF0) | (word & 0x0F)) as u8;
    (d, k)
}

fn sext12(word: u16) -> u16 {
    let k = word & 0x0FFF;
    if k & 0x0800 == 0 { k } else { k | 0xF000 }
}

fn sext7(k: u16) -> u16 {
    if k & 0x40 == 0 { k } else { k | 0xFF80 }
}

fn signed_pair(rd: u8, rr: u8) -> u16 {
    (i16::from(rd as i8) * i16::from(rr as i8)) as u16
}

fn signed_unsigned(rd: u8, rr: u8) -> u16 {
    (i16::from(rd as i8) * i16::from(rr)) as u16
}
