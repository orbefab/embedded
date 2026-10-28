//! The core: 64 address registers behind the window, the modeled special registers, the PC and
//! the cycle count. It owns no memory. Every fetch, load and store goes through a [`Bus`].

use std::fmt;
use std::marker::PhantomData;

use crate::bus::{Bus, Cause, Cycle, Exit, Width};
use crate::config::Config;
use crate::decode::{Decoded, Insn, Op};
use crate::special::{Access, EPC, EPS, Sr, register_op};

const PS_INTLEVEL: u32 = 0xf;
const PS_EXCM: u32 = 1 << 4;
const PS_UM: u32 = 1 << 5;
const PS_CALLINC_SHIFT: u32 = 16;
const PS_CALLINC: u32 = 3 << PS_CALLINC_SHIFT;
const PS_WOE: u32 = 1 << 18;

const RESET_VECTOR: u32 = 0x4000_0400;
const RESET_VECBASE: u32 = 0x4000_0000;
const RESET_PS: u32 = PS_EXCM | PS_INTLEVEL;

const KERNEL_VECTOR: u32 = 0x300;
const USER_VECTOR: u32 = 0x340;
const DOUBLE_VECTOR: u32 = 0x3c0;

const DEBUGCAUSE_ICOUNT: u32 = 1 << 0;
const DEBUGCAUSE_BREAK: u32 = 1 << 3;
const DEBUGCAUSE_BREAK_N: u32 = 1 << 4;

/// Cycles a taken branch, a jump, a call or a return adds to its own one. 32 taken `j` on the
/// classic board net 144 cycles: four each, and one more for each of the 16 whose target runs
/// into the next word. That one sequential chain is the only pattern measured.
const REDIRECT_CYCLES: u64 = 3;
/// Cycles of `quos`, `quou`, `rems` and `remu`, a placeholder until the S3 board timing (S5b). A
/// divide also refetches the next instruction, a cycle more when that instruction runs into the
/// next word, and that cycle is a placeholder too. With it, 32 `quos` net 208, the low edge of
/// the classic bounds of 208 to 216, where the board measured 212. Without it no whole divide
/// cost lands inside them. The model is off outside the divide too: 32 `memw` net 32 here and 35
/// on the board.
const DIVIDE_CYCLES: u64 = 6;

/// Why `run_until` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The core's cycle reached the limit.
    Limit,
    /// The bus answered an access with [`Exit::Stop`]. The core stopped before the instruction at
    /// its PC.
    Bus,
    /// The next instruction needs behavior this core does not model. The core stopped before it.
    Unimplemented(Unimplemented),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unimplemented {
    pub pc: u32,
    pub missing: Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    Op(Op),
    /// This op at an address that is not a multiple of its size: `l32ai` or `s32ri` at any such
    /// address, and any other load or store when one of its parts faults. The ISA lets the chip
    /// raise `LoadStoreAlignmentCause` there and does not say what `EXCVADDR` holds for a later
    /// part. A store's parts before the faulting one stay written.
    Unaligned(Op),
    /// `l32r` with `LITBASE` enabled.
    Litbase,
    /// `entry` would rotate into a frame `WindowStart` still holds, or the next instruction could
    /// reference one, where the chip would spill it first. A run that resumes stops here again
    /// until the frame is out of the window or overflow is off.
    WindowOverflow,
    /// `retw` would return to a frame `WindowStart` no longer holds.
    WindowUnderflow,
}

impl fmt::Display for Unimplemented {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.missing {
            Missing::Op(op) => write!(f, "`{}` at {:#010x} is not modeled", op.name(), self.pc),
            Missing::Unaligned(op) => {
                write!(
                    f,
                    "unaligned `{}` at {:#010x} is not modeled",
                    op.name(),
                    self.pc
                )
            }
            Missing::Litbase => {
                write!(f, "`l32r` with LITBASE at {:#010x} is not modeled", self.pc)
            }
            Missing::WindowOverflow => {
                write!(f, "window overflow at {:#010x} is not modeled", self.pc)
            }
            Missing::WindowUnderflow => {
                write!(f, "window underflow at {:#010x} is not modeled", self.pc)
            }
        }
    }
}

/// Why an instruction did not complete.
enum Trap {
    Exception(Cause, Option<u32>),
    Debug(u32),
    Stop(Stop),
}

impl Trap {
    fn missing(pc: u32, missing: Missing) -> Trap {
        Trap::Stop(Stop::Unimplemented(Unimplemented { pc, missing }))
    }
}

/// One Xtensa core of chip `C`.
pub struct Core<C: Config> {
    /// The physical address registers. `a(n)` is `ar[(WindowBase * 4 + n) % 64]`.
    ar: [u32; 64],
    /// Indexed by [`Sr`]. `Ccount` holds `CCOUNT` minus the low 32 bits of `cycle`.
    sr: [u32; Sr::COUNT],
    pc: u32,
    cycle: Cycle,
    /// The last instruction moved the fetch to a new address, so the next fetch costs a cycle
    /// more when its instruction runs into the next word.
    refetch: bool,
    /// The last instruction may have exposed a live frame, so [`Core::run_until`] checks the
    /// window before the next one. It stays set until the check passes.
    window_check: bool,
    chip: PhantomData<C>,
}

impl<C: Config> Clone for Core<C> {
    fn clone(&self) -> Self {
        Core {
            ar: self.ar,
            sr: self.sr,
            pc: self.pc,
            cycle: self.cycle,
            refetch: self.refetch,
            window_check: self.window_check,
            chip: PhantomData,
        }
    }
}

impl<C: Config> fmt::Debug for Core<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Core")
            .field("chip", &C::NAME)
            .field("pc", &format_args!("{:#010x}", self.pc))
            .field("cycle", &self.cycle.0)
            .finish_non_exhaustive()
    }
}

impl<C: Config> Default for Core<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Config> Core<C> {
    /// The reset state: PC at the reset vector, `PS` with `EXCM` set and `INTLEVEL` 15,
    /// `WindowStart` 1, `VECBASE` at its reset value, and cycle 0.
    pub fn new() -> Self {
        let mut core = Core {
            ar: [0; 64],
            sr: [0; Sr::COUNT],
            pc: RESET_VECTOR,
            cycle: Cycle(0),
            refetch: false,
            window_check: false,
            chip: PhantomData,
        };
        core.sr[Sr::Ps as usize] = RESET_PS;
        core.sr[Sr::WindowStart as usize] = 1;
        core.sr[Sr::Vecbase as usize] = RESET_VECBASE;
        core
    }

    pub fn pc(&self) -> u32 {
        self.pc
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
    }

    pub fn cycle(&self) -> Cycle {
        self.cycle
    }

    /// Address register `n` of the current window, `n < 16`.
    pub fn a(&self, n: u32) -> u32 {
        self.ar[self.ar_index(n)]
    }

    pub fn set_a(&mut self, n: u32, value: u32) {
        let index = self.ar_index(n);
        self.ar[index] = value;
    }

    pub fn special(&self, sr: Sr) -> u32 {
        match sr {
            Sr::Ccount => self.sr[sr as usize].wrapping_add(self.cycle.0 as u32),
            _ => self.sr[sr as usize],
        }
    }

    /// Writes `value` with the register's unwritable bits cleared.
    pub fn set_special(&mut self, sr: Sr, value: u32) {
        let value = value & sr.mask();
        self.sr[sr as usize] = match sr {
            Sr::Ccount => value.wrapping_sub(self.cycle.0 as u32),
            _ => value,
        };
    }

    /// Runs until the cycle reaches `limit`, the bus stops the core, or the next instruction is
    /// not modeled. An instruction that starts before `limit` runs whole, so the cycle may pass
    /// it.
    pub fn run_until<B: Bus>(&mut self, bus: &mut B, limit: Cycle) -> Stop {
        loop {
            if self.window_check {
                if self.window_exposed() {
                    return Stop::Unimplemented(Unimplemented {
                        pc: self.pc,
                        missing: Missing::WindowOverflow,
                    });
                }
                self.window_check = false;
            }
            if self.cycle >= limit {
                return Stop::Limit;
            }
            if let Err(stop) = self.step(bus) {
                return stop;
            }
        }
    }

    fn step<B: Bus>(&mut self, bus: &mut B) -> Result<(), Stop> {
        let counting = self.cintlevel() < self.sr[Sr::IcountLevel as usize];
        let icount = self.sr[Sr::Icount as usize];
        let done = if counting && icount == u32::MAX && self.debug_allowed() {
            Err(Trap::Debug(DEBUGCAUSE_ICOUNT))
        } else {
            self.execute(bus)
        };
        match done {
            Ok(op) => {
                if counting && !matches!(op, Op::WSR_ICOUNT | Op::XSR_ICOUNT) {
                    self.sr[Sr::Icount as usize] = icount.wrapping_add(1);
                }
                self.window_check = moves_window(op);
            }
            Err(Trap::Exception(cause, vaddr)) => self.exception(cause, vaddr),
            Err(Trap::Debug(cause)) => self.debug_exception(cause),
            Err(Trap::Stop(stop)) => return Err(stop),
        }
        Ok(())
    }

    fn ar_index(&self, n: u32) -> usize {
        debug_assert!(n < 16, "a{n} is not an address register");
        ((self.sr[Sr::WindowBase as usize] * 4 + n) & 63) as usize
    }

    fn ps(&self) -> u32 {
        self.sr[Sr::Ps as usize]
    }

    fn cintlevel(&self) -> u32 {
        let ps = self.ps();
        let level = ps & PS_INTLEVEL;
        if ps & PS_EXCM != 0 {
            level.max(C::EXCM_LEVEL)
        } else {
            level
        }
    }

    fn debug_allowed(&self) -> bool {
        self.cintlevel() < C::DEBUG_LEVEL
    }

    /// Window overflow is on and a frame `WindowStart` holds lies in the current window's upper
    /// three quarters. The chip spills that frame on the first reference to one of its registers.
    /// This core checks no reference, so it stops instead. `entry` keeps this false, and only a
    /// write to `WindowBase`, `WindowStart` or `PS` or a return can make it true.
    fn window_exposed(&self) -> bool {
        let ps = self.ps();
        ps & PS_WOE != 0
            && ps & PS_EXCM == 0
            && live_frame_above(
                self.sr[Sr::WindowStart as usize],
                self.sr[Sr::WindowBase as usize],
                3,
            )
    }

    /// A vector costs a redirect's cycles, a placeholder until the S3 board timing (S5b). No board
    /// row measures it.
    fn take_vector(&mut self, target: u32) {
        self.pc = target;
        self.cycle.0 += 1 + REDIRECT_CYCLES;
        self.refetch = true;
    }

    fn exception(&mut self, cause: Cause, vaddr: Option<u32>) {
        self.sr[Sr::ExcCause as usize] = cause as u32;
        if let Some(vaddr) = vaddr {
            self.sr[Sr::ExcVaddr as usize] = vaddr;
        }
        let ps = self.ps();
        let vecbase = self.sr[Sr::Vecbase as usize];
        if ps & PS_EXCM != 0 {
            self.sr[Sr::Depc as usize] = self.pc;
            self.take_vector(vecbase + DOUBLE_VECTOR);
        } else {
            self.sr[Sr::Epc1 as usize] = self.pc;
            self.sr[Sr::Ps as usize] = ps | PS_EXCM;
            let offset = if ps & PS_UM != 0 {
                USER_VECTOR
            } else {
                KERNEL_VECTOR
            };
            self.take_vector(vecbase + offset);
        }
    }

    fn debug_exception(&mut self, cause: u32) {
        let level = C::DEBUG_LEVEL;
        let ps = self.ps();
        self.sr[Sr::DebugCause as usize] = cause;
        self.sr[EPC[level as usize] as usize] = self.pc;
        self.sr[EPS[level as usize] as usize] = ps;
        self.sr[Sr::Ps as usize] = (ps & !PS_INTLEVEL) | PS_EXCM | level;
        let vecbase = self.sr[Sr::Vecbase as usize];
        self.take_vector(vecbase + level_vector(level));
    }

    fn fetch<B: Bus>(&mut self, bus: &mut B) -> Result<Insn, Trap> {
        let pc = self.pc;
        let word = pc & !3;
        let mut bytes = [0u8; 8];
        let low = bus
            .fetch(self.cycle, word)
            .map_err(|exit| exit_trap(exit, pc))?;
        bytes[..4].copy_from_slice(&low.to_le_bytes());
        let offset = (pc & 3) as usize;
        let Some(len) = C::insn_len(bytes[offset]) else {
            return Err(Trap::Exception(Cause::IllegalInstruction, None));
        };
        let end = offset + len as usize;
        if end > 4 {
            let next = word.wrapping_add(4);
            let high = bus
                .fetch(self.cycle, next)
                .map_err(|exit| exit_trap(exit, next))?;
            bytes[4..].copy_from_slice(&high.to_le_bytes());
        }
        match C::decode(&bytes[offset..end], pc) {
            Some(Decoded::Insn(insn)) => {
                if self.refetch && end > 4 {
                    self.cycle.0 += 1;
                }
                self.refetch = false;
                Ok(insn)
            }
            _ => Err(Trap::Exception(Cause::IllegalInstruction, None)),
        }
    }

    /// Loads the bytes from `addr`, little-endian, one bus access per aligned part.
    fn load<B: Bus>(&self, bus: &mut B, op: Op, addr: u32, width: Width) -> Result<u32, Trap> {
        let mut value = 0;
        for (at, part, shift) in parts(addr, width) {
            let bytes = bus
                .load(self.cycle, at, part)
                .map_err(|exit| self.access_trap(exit, op, addr, width))?;
            value |= bytes << shift;
        }
        Ok(value)
    }

    fn store<B: Bus>(
        &self,
        bus: &mut B,
        op: Op,
        addr: u32,
        width: Width,
        value: u32,
    ) -> Result<(), Trap> {
        for (at, part, shift) in parts(addr, width) {
            bus.store(self.cycle, at, part, value >> shift)
                .map_err(|exit| self.access_trap(exit, op, addr, width))?;
        }
        Ok(())
    }

    /// An aligned access that faults raises the exception with its address in `EXCVADDR`. An
    /// unaligned one stops, since the ISA leaves its cause and `EXCVADDR` open.
    fn access_trap(&self, exit: Exit, op: Op, addr: u32, width: Width) -> Trap {
        match exit {
            Exit::Fault(_) if !addr.is_multiple_of(width.bytes()) => {
                Trap::missing(self.pc, Missing::Unaligned(op))
            }
            exit => exit_trap(exit, addr),
        }
    }

    /// Runs one instruction. On `Ok` the PC, the registers and the cycle have moved. On `Err`
    /// none of them has, except what fetching and decoding charged, but an unaligned store that
    /// stops keeps the parts it wrote before the faulting one.
    fn execute<B: Bus>(&mut self, bus: &mut B) -> Result<Op, Trap> {
        let insn = self.fetch(bus)?;
        let pc = self.pc;
        let next = pc.wrapping_add(u32::from(insn.len));
        let f = insn.fields;
        let mut cycles = 1;
        let mut target = None;
        let sar = self.sr[Sr::Sar as usize];
        match insn.op {
            Op::NOP
            | Op::NOP_N
            | Op::MEMW
            | Op::EXTW
            | Op::ISYNC
            | Op::RSYNC
            | Op::ESYNC
            | Op::DSYNC
            | Op::EXCW => {}

            Op::ADD | Op::ADD_N => self.set_a(f[0], self.a(f[1]).wrapping_add(self.a(f[2]))),
            Op::ADDX2 => self.set_a(f[0], (self.a(f[1]) << 1).wrapping_add(self.a(f[2]))),
            Op::ADDX4 => self.set_a(f[0], (self.a(f[1]) << 2).wrapping_add(self.a(f[2]))),
            Op::ADDX8 => self.set_a(f[0], (self.a(f[1]) << 3).wrapping_add(self.a(f[2]))),
            Op::SUB => self.set_a(f[0], self.a(f[1]).wrapping_sub(self.a(f[2]))),
            Op::SUBX2 => self.set_a(f[0], (self.a(f[1]) << 1).wrapping_sub(self.a(f[2]))),
            Op::SUBX4 => self.set_a(f[0], (self.a(f[1]) << 2).wrapping_sub(self.a(f[2]))),
            Op::SUBX8 => self.set_a(f[0], (self.a(f[1]) << 3).wrapping_sub(self.a(f[2]))),
            Op::ADDI | Op::ADDI_N | Op::ADDMI => self.set_a(f[0], self.a(f[1]).wrapping_add(f[2])),
            Op::AND => self.set_a(f[0], self.a(f[1]) & self.a(f[2])),
            Op::OR => self.set_a(f[0], self.a(f[1]) | self.a(f[2])),
            Op::XOR => self.set_a(f[0], self.a(f[1]) ^ self.a(f[2])),
            Op::NEG => self.set_a(f[0], self.a(f[1]).wrapping_neg()),
            Op::ABS => self.set_a(f[0], (self.a(f[1]) as i32).wrapping_abs() as u32),
            Op::MOVI | Op::MOVI_N => self.set_a(f[0], f[1]),
            Op::MOV_N => self.set_a(f[0], self.a(f[1])),
            Op::MOVEQZ => self.move_if(f, |t| t == 0),
            Op::MOVNEZ => self.move_if(f, |t| t != 0),
            Op::MOVLTZ => self.move_if(f, |t| (t as i32) < 0),
            Op::MOVGEZ => self.move_if(f, |t| (t as i32) >= 0),
            Op::MIN => self.set_a(f[0], (self.a(f[1]) as i32).min(self.a(f[2]) as i32) as u32),
            Op::MAX => self.set_a(f[0], (self.a(f[1]) as i32).max(self.a(f[2]) as i32) as u32),
            Op::MINU => self.set_a(f[0], self.a(f[1]).min(self.a(f[2]))),
            Op::MAXU => self.set_a(f[0], self.a(f[1]).max(self.a(f[2]))),
            Op::CLAMPS => {
                let high = (1i32 << f[2]) - 1;
                self.set_a(f[0], (self.a(f[1]) as i32).clamp(-high - 1, high) as u32);
            }
            Op::SEXT => {
                let shift = 31 - f[2];
                self.set_a(f[0], (((self.a(f[1]) << shift) as i32) >> shift) as u32);
            }
            Op::NSA => {
                let value = self.a(f[1]);
                let sign = ((value as i32) >> 31) as u32;
                self.set_a(f[0], (value ^ sign).leading_zeros() - 1);
            }
            Op::NSAU => self.set_a(f[0], self.a(f[1]).leading_zeros()),

            Op::EXTUI => {
                let mask = (1u32 << f[3]) - 1;
                self.set_a(f[0], (self.a(f[1]) >> f[2]) & mask);
            }
            Op::SLLI => self.set_a(f[0], self.a(f[1]).checked_shl(f[2]).unwrap_or(0)),
            Op::SRLI => self.set_a(f[0], self.a(f[1]) >> f[2]),
            Op::SRAI => self.set_a(f[0], ((self.a(f[1]) as i32) >> f[2]) as u32),
            Op::SLL => self.set_a(f[0], ((u64::from(self.a(f[1])) << 32) >> sar) as u32),
            Op::SRL => self.set_a(f[0], (u64::from(self.a(f[1])) >> sar) as u32),
            Op::SRA => self.set_a(f[0], (i64::from(self.a(f[1]) as i32) >> sar) as u32),
            Op::SRC => {
                let pair = (u64::from(self.a(f[1])) << 32) | u64::from(self.a(f[2]));
                self.set_a(f[0], (pair >> sar) as u32);
            }
            Op::SSR => self.sr[Sr::Sar as usize] = self.a(f[0]) & 31,
            Op::SSL => self.sr[Sr::Sar as usize] = 32 - (self.a(f[0]) & 31),
            Op::SSA8L => self.sr[Sr::Sar as usize] = (self.a(f[0]) & 3) * 8,
            Op::SSA8B => self.sr[Sr::Sar as usize] = 32 - (self.a(f[0]) & 3) * 8,
            Op::SSAI => self.sr[Sr::Sar as usize] = f[0],

            Op::MUL16U => {
                let product = (self.a(f[1]) & 0xffff) * (self.a(f[2]) & 0xffff);
                self.set_a(f[0], product);
            }
            Op::MUL16S => {
                let product = i32::from(self.a(f[1]) as i16) * i32::from(self.a(f[2]) as i16);
                self.set_a(f[0], product as u32);
            }
            Op::MULL => self.set_a(f[0], self.a(f[1]).wrapping_mul(self.a(f[2]))),
            Op::MULUH => {
                let product = u64::from(self.a(f[1])) * u64::from(self.a(f[2]));
                self.set_a(f[0], (product >> 32) as u32);
            }
            Op::MULSH => {
                let product = i64::from(self.a(f[1]) as i32) * i64::from(self.a(f[2]) as i32);
                self.set_a(f[0], (product >> 32) as u32);
            }
            Op::QUOU | Op::QUOS | Op::REMU | Op::REMS => {
                let (s, t) = (self.a(f[1]), self.a(f[2]));
                if t == 0 {
                    return Err(Trap::Exception(Cause::IntegerDivideByZero, None));
                }
                let value = match insn.op {
                    Op::QUOU => s / t,
                    Op::REMU => s % t,
                    Op::QUOS => (s as i32).wrapping_div(t as i32) as u32,
                    _ => (s as i32).wrapping_rem(t as i32) as u32,
                };
                self.set_a(f[0], value);
                cycles = DIVIDE_CYCLES;
                self.refetch = true;
            }

            Op::L8UI => {
                let value =
                    self.load(bus, insn.op, self.a(f[1]).wrapping_add(f[2]), Width::Byte)?;
                self.set_a(f[0], value);
            }
            Op::L16UI => {
                let value =
                    self.load(bus, insn.op, self.a(f[1]).wrapping_add(f[2]), Width::Half)?;
                self.set_a(f[0], value);
            }
            Op::L16SI => {
                let value =
                    self.load(bus, insn.op, self.a(f[1]).wrapping_add(f[2]), Width::Half)?;
                self.set_a(f[0], value as i16 as u32);
            }
            Op::L32I | Op::L32I_N => {
                let value =
                    self.load(bus, insn.op, self.a(f[1]).wrapping_add(f[2]), Width::Word)?;
                self.set_a(f[0], value);
            }
            Op::L32AI => {
                let addr = sync_addr(pc, insn.op, self.a(f[1]).wrapping_add(f[2]))?;
                let value = self.load(bus, insn.op, addr, Width::Word)?;
                self.set_a(f[0], value);
            }
            Op::L32R => {
                if self.sr[Sr::Litbase as usize] & 1 != 0 {
                    return Err(Trap::missing(pc, Missing::Litbase));
                }
                let value = self.load(bus, insn.op, f[1], Width::Word)?;
                self.set_a(f[0], value);
            }
            Op::S8I => self.store(
                bus,
                insn.op,
                self.a(f[1]).wrapping_add(f[2]),
                Width::Byte,
                self.a(f[0]),
            )?,
            Op::S16I => self.store(
                bus,
                insn.op,
                self.a(f[1]).wrapping_add(f[2]),
                Width::Half,
                self.a(f[0]),
            )?,
            Op::S32I | Op::S32I_N | Op::S32NB => {
                self.store(
                    bus,
                    insn.op,
                    self.a(f[1]).wrapping_add(f[2]),
                    Width::Word,
                    self.a(f[0]),
                )?;
            }
            Op::S32RI => {
                let addr = sync_addr(pc, insn.op, self.a(f[1]).wrapping_add(f[2]))?;
                self.store(bus, insn.op, addr, Width::Word, self.a(f[0]))?;
            }

            Op::BEQZ | Op::BEQZ_N => target = taken(self.a(f[0]) == 0, f[1]),
            Op::BNEZ | Op::BNEZ_N => target = taken(self.a(f[0]) != 0, f[1]),
            Op::BGEZ => target = taken((self.a(f[0]) as i32) >= 0, f[1]),
            Op::BLTZ => target = taken((self.a(f[0]) as i32) < 0, f[1]),
            Op::BEQ => target = taken(self.a(f[0]) == self.a(f[1]), f[2]),
            Op::BNE => target = taken(self.a(f[0]) != self.a(f[1]), f[2]),
            Op::BGE => target = taken(self.a(f[0]) as i32 >= self.a(f[1]) as i32, f[2]),
            Op::BLT => target = taken((self.a(f[0]) as i32) < self.a(f[1]) as i32, f[2]),
            Op::BGEU => target = taken(self.a(f[0]) >= self.a(f[1]), f[2]),
            Op::BLTU => target = taken(self.a(f[0]) < self.a(f[1]), f[2]),
            Op::BANY => target = taken(self.a(f[0]) & self.a(f[1]) != 0, f[2]),
            Op::BNONE => target = taken(self.a(f[0]) & self.a(f[1]) == 0, f[2]),
            Op::BALL => target = taken(!self.a(f[0]) & self.a(f[1]) == 0, f[2]),
            Op::BNALL => target = taken(!self.a(f[0]) & self.a(f[1]) != 0, f[2]),
            Op::BBC => target = taken(self.a(f[0]) >> (self.a(f[1]) & 31) & 1 == 0, f[2]),
            Op::BBS => target = taken(self.a(f[0]) >> (self.a(f[1]) & 31) & 1 != 0, f[2]),
            Op::BBCI => target = taken(self.a(f[0]) >> f[1] & 1 == 0, f[2]),
            Op::BBSI => target = taken(self.a(f[0]) >> f[1] & 1 != 0, f[2]),
            Op::BEQI => target = taken(self.a(f[0]) == f[1], f[2]),
            Op::BNEI => target = taken(self.a(f[0]) != f[1], f[2]),
            Op::BGEI => target = taken(self.a(f[0]) as i32 >= f[1] as i32, f[2]),
            Op::BLTI => target = taken((self.a(f[0]) as i32) < f[1] as i32, f[2]),
            Op::BGEUI => target = taken(self.a(f[0]) >= f[1], f[2]),
            Op::BLTUI => target = taken(self.a(f[0]) < f[1], f[2]),

            Op::J => target = Some(f[0]),
            Op::JX => target = Some(self.a(f[0])),
            Op::CALL0 => {
                self.set_a(0, next);
                target = Some(f[0]);
            }
            Op::CALLX0 => {
                let to = self.a(f[0]);
                self.set_a(0, next);
                target = Some(to);
            }
            Op::RET | Op::RET_N => target = Some(self.a(0)),
            Op::CALL4 => target = Some(self.window_call(1, f[0], next)),
            Op::CALL8 => target = Some(self.window_call(2, f[0], next)),
            Op::CALL12 => target = Some(self.window_call(3, f[0], next)),
            Op::CALLX4 => target = Some(self.window_call(1, self.a(f[0]), next)),
            Op::CALLX8 => target = Some(self.window_call(2, self.a(f[0]), next)),
            Op::CALLX12 => target = Some(self.window_call(3, self.a(f[0]), next)),
            Op::ENTRY => self.entry(pc, f[0], f[1])?,
            Op::RETW | Op::RETW_N => target = Some(self.retw(pc, insn.op)?),

            Op::RSIL => {
                let ps = self.ps();
                self.set_a(f[0], ps);
                self.sr[Sr::Ps as usize] = (ps & !PS_INTLEVEL) | f[1];
            }
            Op::RFE => {
                self.sr[Sr::Ps as usize] &= !PS_EXCM;
                target = Some(self.sr[Sr::Epc1 as usize]);
            }
            Op::RFDE => {
                self.sr[Sr::Ps as usize] &= !PS_EXCM;
                target = Some(self.sr[Sr::Depc as usize]);
            }
            Op::RFI => {
                let level = f[0] as usize;
                if !(2..=7).contains(&level) {
                    return Err(Trap::missing(pc, Missing::Op(insn.op)));
                }
                self.sr[Sr::Ps as usize] = self.sr[EPS[level] as usize];
                target = Some(self.sr[EPC[level] as usize]);
            }
            Op::ILL | Op::ILL_N => return Err(Trap::Exception(Cause::IllegalInstruction, None)),
            Op::SYSCALL => return Err(Trap::Exception(Cause::Syscall, None)),
            Op::BREAK if self.debug_allowed() => return Err(Trap::Debug(DEBUGCAUSE_BREAK)),
            Op::BREAK_N if self.debug_allowed() => return Err(Trap::Debug(DEBUGCAUSE_BREAK_N)),
            Op::BREAK | Op::BREAK_N => {}

            op => match register_op(op) {
                Some((access, sr)) => self.register_op(access, sr, f[0]),
                None => return Err(Trap::missing(pc, Missing::Op(op))),
            },
        }
        self.cycle.0 += cycles;
        match target {
            Some(to) => {
                self.cycle.0 += REDIRECT_CYCLES;
                self.refetch = true;
                self.pc = to;
            }
            None => self.pc = next,
        }
        Ok(insn.op)
    }

    fn move_if(&mut self, f: [u32; 8], test: impl Fn(u32) -> bool) {
        if test(self.a(f[2])) {
            self.set_a(f[0], self.a(f[1]));
        }
    }

    fn register_op(&mut self, access: Access, sr: Sr, t: u32) {
        let old = self.special(sr);
        match access {
            Access::Read => self.set_a(t, old),
            Access::Write => self.set_special(sr, self.a(t)),
            Access::Exchange => {
                self.set_special(sr, self.a(t));
                self.set_a(t, old);
            }
        }
    }

    /// `callN`: the return address in `a(4N)` with `N` in its top two bits, and `N` in
    /// `PS.CALLINC` for the callee's `entry`.
    fn window_call(&mut self, n: u32, to: u32, next: u32) -> u32 {
        self.set_a(n * 4, (n << 30) | (next & 0x3fff_ffff));
        let ps = self.ps();
        self.sr[Sr::Ps as usize] = (ps & !PS_CALLINC) | (n << PS_CALLINC_SHIFT);
        to
    }

    /// Rotates the window by `PS.CALLINC` and moves the new frame's stack pointer down by
    /// `frame`. The ISA checks the frames `WindowBase + 1` to `WindowBase + CALLINC` for the
    /// register `entry` writes, and none when `CALLINC` is 0. The chip checks the callee's other
    /// registers on each reference and this core does not, so it also stops when a frame lies in
    /// the new window's upper three quarters. Together that is `WindowBase + 1` to
    /// `WindowBase + CALLINC + 3`, checked before any write.
    fn entry(&mut self, pc: u32, s: u32, frame: u32) -> Result<(), Trap> {
        let ps = self.ps();
        if s > 3 || ps & PS_WOE == 0 || ps & PS_EXCM != 0 {
            return Err(Trap::missing(pc, Missing::Op(Op::ENTRY)));
        }
        let inc = (ps & PS_CALLINC) >> PS_CALLINC_SHIFT;
        let base = self.sr[Sr::WindowBase as usize];
        let start = self.sr[Sr::WindowStart as usize];
        if live_frame_above(start, base, inc + 3) {
            return Err(Trap::missing(pc, Missing::WindowOverflow));
        }
        let sp = self.a(s).wrapping_sub(frame);
        let base = (base + inc) & 15;
        self.sr[Sr::WindowBase as usize] = base;
        self.sr[Sr::WindowStart as usize] = start | (1 << base);
        self.set_a(s, sp);
        Ok(())
    }

    /// Rotates the window back by the increment in `a0`'s top two bits and returns the target.
    /// The ISA leaves it undefined when the nearest frame below, `m` quarters down, is not the
    /// caller's.
    fn retw(&mut self, pc: u32, op: Op) -> Result<u32, Trap> {
        let ps = self.ps();
        let ret = self.a(0);
        let n = ret >> 30;
        let base = self.sr[Sr::WindowBase as usize];
        let start = self.sr[Sr::WindowStart as usize];
        let m = (1..=3)
            .find(|k| start >> (base.wrapping_sub(*k) & 15) & 1 != 0)
            .unwrap_or(0);
        if n == 0 || (m != 0 && m != n) || ps & PS_WOE == 0 || ps & PS_EXCM != 0 {
            return Err(Trap::missing(pc, Missing::Op(op)));
        }
        let caller = base.wrapping_sub(n) & 15;
        if start >> caller & 1 == 0 {
            return Err(Trap::missing(pc, Missing::WindowUnderflow));
        }
        self.sr[Sr::WindowStart as usize] = start & !(1 << base);
        self.sr[Sr::WindowBase as usize] = caller;
        Ok((pc & 0xc000_0000) | (ret & 0x3fff_ffff))
    }
}

fn taken(condition: bool, to: u32) -> Option<u32> {
    condition.then_some(to)
}

/// Whether `start` holds a frame in the `quarters` quarters of the window above `base`.
fn live_frame_above(start: u32, base: u32, quarters: u32) -> bool {
    (1..=quarters).any(|k| start >> ((base + k) & 15) & 1 != 0)
}

/// The ops after which [`Core::window_exposed`] can turn true.
fn moves_window(op: Op) -> bool {
    matches!(
        op,
        Op::WSR_WINDOWBASE
            | Op::XSR_WINDOWBASE
            | Op::WSR_WINDOWSTART
            | Op::XSR_WINDOWSTART
            | Op::WSR_PS
            | Op::XSR_PS
            | Op::RFE
            | Op::RFDE
            | Op::RFI
            | Op::RETW
            | Op::RETW_N
    )
}

/// The aligned parts that cover `width` bytes from `addr`, lowest first, each with its shift in
/// the value. An aligned access is one part. An unaligned one is split at its natural
/// alignments, so no part crosses a word and none touches a byte outside the access.
fn parts(addr: u32, width: Width) -> impl Iterator<Item = (u32, Width, u32)> {
    let len = width.bytes();
    let mut done = 0;
    std::iter::from_fn(move || {
        (done < len).then(|| {
            let at = addr.wrapping_add(done);
            let part = [Width::Word, Width::Half]
                .into_iter()
                .find(|part| at.is_multiple_of(part.bytes()) && part.bytes() <= len - done)
                .unwrap_or(Width::Byte);
            let item = (at, part, done * 8);
            done += part.bytes();
            item
        })
    })
}

/// `l32ai` and `s32ri` order memory for other cores, and what the chip does with one that is not
/// word-aligned is unmeasured.
fn sync_addr(pc: u32, op: Op, addr: u32) -> Result<u32, Trap> {
    if addr.is_multiple_of(4) {
        Ok(addr)
    } else {
        Err(Trap::missing(pc, Missing::Unaligned(op)))
    }
}

/// The offset from `VECBASE` of the vector for interrupt level `level`, 2 to 7. Level 6 is the
/// debug vector and level 7 the NMI vector on both chips.
const fn level_vector(level: u32) -> u32 {
    0x100 + 0x40 * level
}

fn exit_trap(exit: Exit, addr: u32) -> Trap {
    match exit {
        Exit::Stop => Trap::Stop(Stop::Bus),
        Exit::Fault(cause) => Trap::Exception(cause, Some(addr)),
    }
}
