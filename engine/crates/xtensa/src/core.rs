//! The core: 64 address registers behind the window, the modeled special registers, the PC and
//! the cycle count. It owns no memory. Every fetch, load and store goes through a [`Bus`].

use std::fmt;
use std::marker::PhantomData;

use crate::bus::{Bus, Cause, Cycle, Exit, Width};
use crate::config::Config;
use crate::decode::{Decoded, Insn, Op};
use crate::special::{Access, CCOMPARE, EPC, EPS, PRID, Sr};

mod options;

const PS_INTLEVEL: u32 = 0xf;
const PS_EXCM: u32 = 1 << 4;
const PS_UM: u32 = 1 << 5;
const PS_OWB_SHIFT: u32 = 8;
const PS_OWB: u32 = 0xf << PS_OWB_SHIFT;
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

/// Low two bits of [`Core::pace`]: 0 when no loop-back is pending, 1 when the next one is the
/// first after a loop op, 2 when it is the second, 3 after that.
const PACE_LOOP: u8 = 0b11;
/// Set after a store, so the next `memw` can cost [`Config::memw_after_store`].
const PACE_STORE: u8 = 1 << 2;
/// Set after a divide, so the next divide can cost the follower rate.
const PACE_DIVIDE: u8 = 1 << 3;

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
    /// This op at an address that is not a multiple of its size: `l32ai`, `s32ri`, `s32c1i`,
    /// `ld.qr` or `st.qr` at any such address, and any other load or store when one of its parts
    /// faults. The ISA lets the chip raise `LoadStoreAlignmentCause` there and does not say what
    /// `EXCVADDR` holds for a later part. A store's parts before the faulting one stay written.
    Unaligned(Op),
    /// `l32r` with `LITBASE` enabled.
    Litbase,
    /// This op as the last of a loop when it loops back. The ISA says a loop's last op must not
    /// be a call, `isync`, `waiti` or `rsr.lcount` (4.3.2.2, p 86), and its own pseudo-code and
    /// prose disagree on where a call there returns (8.3.57, p 373).
    LoopEnd(Op),
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
            Missing::LoopEnd(op) => write!(
                f,
                "`{}` as the last op of a loop at {:#010x} is not modeled",
                op.name(),
                self.pc
            ),
        }
    }
}

/// The latest coprocessor-disabled exception: the coprocessor, the op that needed it and the op's
/// PC. A handler that enables the coprocessor and returns runs the op again. When that op is one
/// this core does not model, such as an S3 vector op, the run stops there, and this names the
/// op that first found the coprocessor off.
///
/// The record stays until the next such exception. An op whose coprocessor is already on raises
/// nothing and leaves an older record in place, so a reader that takes the record for a stop
/// checks that its `pc` is the stop's PC. Keeping it, rather than clearing it at each op, keeps
/// that check off the path every instruction takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoprocessorDisabled {
    pub coprocessor: u8,
    pub op: Op,
    pub pc: u32,
}

/// Why an instruction did not complete.
enum Trap {
    Exception(Cause, Option<u32>),
    /// A window overflow or underflow. Its handler runs with `WindowBase` at `base`, from
    /// `VECBASE` plus `vector`.
    Window {
        base: u32,
        vector: u32,
    },
    Debug(u32),
    Stop(Stop),
}

impl Trap {
    fn missing(pc: u32, missing: Missing) -> Trap {
        Trap::Stop(Stop::Unimplemented(Unimplemented { pc, missing }))
    }
}

/// One Xtensa core of chip `C`.
#[derive(Clone, PartialEq, Eq)]
pub struct Core<C: Config> {
    /// The physical address registers. `a(n)` is `ar[(WindowBase * 4 + n) % 64]`.
    ar: [u32; 64],
    /// Indexed by [`Sr`]. `Ccount` holds `CCOUNT` minus the low 32 bits of `cycle`, and
    /// `Interrupt` the pending bits that latch: software, edge and timer.
    sr: [u32; Sr::COUNT],
    pc: u32,
    cycle: Cycle,
    /// The last instruction moved the fetch to a new address, so the next fetch costs a cycle
    /// more when its instruction runs into the next word.
    refetch: bool,
    /// `WAITI` suspended the core until it takes an interrupt.
    waiting: bool,
    /// Loop-back arm, and whether the previous completed op was a store or a divide.
    pace: u8,
    /// The external interrupt lines as the last poll read them.
    lines: u32,
    /// The cycle at which each `CCOMPAREn` next equals `CCOUNT`.
    timer_due: [Cycle; 3],
    /// The FPU registers `f0` to `f15`.
    fr: [u32; 16],
    /// The S3's `q0` to `q7`, each as four words, lowest first.
    q: [[u32; 4]; 8],
    disabled: Option<CoprocessorDisabled>,
    /// The access rights of each 512 MB region, for instruction fetches and then for loads and
    /// stores, as `witlb` and `wdtlb` leave them.
    regions: [[u8; 8]; 2],
    chip: PhantomData<C>,
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
    /// `WindowStart` 1, `VECBASE` at its reset value, and cycle 0. The registers the ISA leaves
    /// undefined at reset are 0.
    pub fn new() -> Self {
        let mut core = Core {
            ar: [0; 64],
            sr: [0; Sr::COUNT],
            pc: RESET_VECTOR,
            cycle: Cycle(0),
            refetch: false,
            waiting: false,
            pace: 0,
            lines: 0,
            timer_due: [Cycle(0); 3],
            fr: [0; 16],
            q: [[0; 4]; 8],
            disabled: None,
            regions: [[REGION_RESET; 8]; 2],
            chip: PhantomData,
        };
        core.sr[Sr::Ps as usize] = RESET_PS;
        core.sr[Sr::WindowStart as usize] = 1;
        core.sr[Sr::Vecbase as usize] = RESET_VECBASE;
        for i in 0..3 {
            core.arm_timer(i);
        }
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

    /// FPU register `n`, `n < 16`.
    pub fn f(&self, n: u32) -> u32 {
        self.fr[n as usize]
    }

    pub fn set_f(&mut self, n: u32, value: u32) {
        self.fr[n as usize] = value;
    }

    /// The S3's `q` register `n`, `n < 8`. No classic op reads or writes it.
    pub fn q(&self, n: u32) -> u128 {
        let words = self.q[n as usize];
        words
            .iter()
            .rev()
            .fold(0, |value, word| (value << 32) | u128::from(*word))
    }

    pub fn set_q(&mut self, n: u32, value: u128) {
        for (i, word) in self.q[n as usize].iter_mut().enumerate() {
            *word = (value >> (32 * i)) as u32;
        }
    }

    /// The latest coprocessor-disabled exception this core raised.
    pub fn coprocessor_disabled(&self) -> Option<CoprocessorDisabled> {
        self.disabled
    }

    /// `Interrupt` is the pending bits that latch, with the external level lines as the last
    /// poll read them. `ACCHI` reads sign-extended from its 8 bits, and `PRID` is fixed.
    pub fn special(&self, sr: Sr) -> u32 {
        match sr {
            Sr::Prid => PRID,
            Sr::Ccount => self.sr[sr as usize].wrapping_add(self.cycle.0 as u32),
            Sr::Interrupt => self.sr[sr as usize] | (self.lines & C::INTERRUPTS.extern_level),
            Sr::Acchi => self.sr[sr as usize] as u8 as i8 as u32,
            _ => self.sr[sr as usize],
        }
    }

    /// Writes `value` with the register's unwritable bits cleared. Writing `CCOMPAREn` clears
    /// its timer interrupt, and writing `CCOUNT` or `CCOMPAREn` moves the next timer match.
    /// Writing `Interrupt` sets the bits that latch.
    pub fn set_special(&mut self, sr: Sr, value: u32) {
        let value = value & sr.mask();
        match sr {
            Sr::Ccount => {
                self.sr[sr as usize] = value.wrapping_sub(self.cycle.0 as u32);
                for i in 0..3 {
                    self.arm_timer(i);
                }
            }
            Sr::Ccompare0 | Sr::Ccompare1 | Sr::Ccompare2 => {
                let i = sr as usize - Sr::Ccompare0 as usize;
                self.sr[sr as usize] = value;
                self.sr[Sr::Interrupt as usize] &= !C::INTERRUPTS.timer_mask(i);
                self.arm_timer(i);
            }
            Sr::Interrupt => self.sr[sr as usize] = value & latched::<C>(),
            _ => self.sr[sr as usize] = value,
        }
    }

    /// Runs until the cycle reaches `limit`, the bus stops the core, or the next instruction is
    /// not modeled. An instruction that starts before `limit` runs whole, so the cycle may pass
    /// it. A core in `WAITI` stops at `limit` exactly.
    pub fn run_until<B: Bus>(&mut self, bus: &mut B, limit: Cycle) -> Stop {
        loop {
            if self.cycle >= limit {
                return Stop::Limit;
            }
            if let Err(stop) = self.step(bus, limit) {
                return stop;
            }
        }
    }

    /// Takes a pending interrupt, idles in `WAITI`, or runs one instruction. Interrupts come
    /// first, as the ISA's priority list puts them ahead of the `ICOUNT` debug exception (4.4.1.11,
    /// p 145).
    fn step<B: Bus>(&mut self, bus: &mut B, limit: Cycle) -> Result<(), Stop> {
        let nmi = self.poll(bus);
        let level = if nmi {
            Some(C::INTERRUPTS.nmi_level)
        } else {
            self.interrupt_level()
        };
        if let Some(level) = level {
            self.waiting = false;
            if level == 1 {
                self.exception(Cause::Level1Interrupt, None);
            } else {
                self.take_level(level);
            }
            return Ok(());
        }
        if self.waiting {
            self.idle(bus, limit);
            return Ok(());
        }
        let counting = self.cintlevel() < self.sr[Sr::IcountLevel as usize];
        let icount = self.sr[Sr::Icount as usize];
        // Below debug level the usual rule applies. The S3 also re-takes at debug level.
        // A higher level does not, so the entry cannot lower `PS.INTLEVEL`.
        let recount = C::icount_past_debug_level() && self.cintlevel() <= C::DEBUG_LEVEL;
        let done = if counting && icount == u32::MAX && (self.debug_allowed() || recount) {
            Err(Trap::Debug(DEBUGCAUSE_ICOUNT))
        } else {
            self.execute(bus)
        };
        match done {
            Ok(op) => {
                if counting && !matches!(op, Op::WSR_ICOUNT | Op::XSR_ICOUNT) {
                    self.sr[Sr::Icount as usize] = icount.wrapping_add(1);
                }
            }
            Err(Trap::Exception(cause, vaddr)) => self.exception(cause, vaddr),
            Err(Trap::Window { base, vector }) => self.window_exception(base, vector),
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

    /// Reads the lines and the timers before an instruction, and returns whether the NMI line
    /// rose.
    fn poll<B: Bus>(&mut self, bus: &mut B) -> bool {
        let interrupts = C::INTERRUPTS;
        let lines = bus.interrupts(self.cycle) & interrupts.external();
        let rose = lines & !self.lines;
        self.lines = lines;
        let mut latched = self.sr[Sr::Interrupt as usize] | (rose & interrupts.extern_edge);
        for i in 0..3 {
            let due = self.timer_due[i];
            if self.cycle >= due {
                latched |= interrupts.timer_mask(i);
                let periods = ((self.cycle.0 - due.0) >> 32) + 1;
                self.timer_due[i] = Cycle(due.0 + (periods << 32));
            }
        }
        self.sr[Sr::Interrupt as usize] = latched;
        rose & interrupts.nmi != 0
    }

    /// The highest level above `CINTLEVEL` with an enabled pending interrupt, as the ISA's
    /// `checkInterrupts` picks it (4.4.6.4, p 160).
    fn interrupt_level(&self) -> Option<u32> {
        let pending = self.special(Sr::Interrupt) & self.sr[Sr::IntEnable as usize];
        if pending == 0 {
            return None;
        }
        let by_level = const { C::INTERRUPTS.by_level() };
        (self.cintlevel() + 1..C::INTERRUPTS.nmi_level)
            .rev()
            .find(|&level| pending & by_level[level as usize] != 0)
    }

    /// `CCOMPAREn` next equals `CCOUNT` at the first cycle after this one where they agree. A
    /// value equal to `CCOUNT` now matches a whole count later, 2^32 cycles on. The ISA says only
    /// that a match raises the interrupt (4.4.7.2, p 162).
    fn arm_timer(&mut self, i: usize) {
        let compare = self.sr[CCOMPARE[i] as usize];
        let ahead = match compare.wrapping_sub(self.special(Sr::Ccount)) {
            0 => 1 << 32,
            ahead => u64::from(ahead),
        };
        self.timer_due[i] = Cycle(self.cycle.0 + ahead);
    }

    /// `WAITI` with no interrupt to take moves the cycle to the bus's next event, a timer match
    /// or `limit`, whichever comes first, and at least one cycle on.
    fn idle<B: Bus>(&mut self, bus: &mut B, limit: Cycle) {
        let wake = bus
            .next_event(self.cycle)
            .into_iter()
            .chain(self.timer_due)
            .fold(limit, Cycle::min);
        self.cycle = wake.max(Cycle(self.cycle.0 + 1));
    }

    /// The ISA's `WindowCheck` for an op whose address registers reach quarter `reach`: how many
    /// quarters up the nearest live frame within it lies (6.1.3, p 246). `CWOE` is
    /// `PS.WOE` with `PS.EXCM` clear (4.4.1.4, p 135).
    fn overflow(&self, reach: u32) -> Option<u32> {
        if reach == 0 || self.ps() & (PS_WOE | PS_EXCM) != PS_WOE {
            return None;
        }
        let hit = self.frames_above() & ((1 << reach) - 1);
        (hit != 0).then(|| hit.trailing_zeros() + 1)
    }

    /// Bit `k - 1` is `WindowStart` bit `WindowBase + k`, for `k` from 1 to 15.
    fn frames_above(&self) -> u32 {
        let start = self.sr[Sr::WindowStart as usize];
        let base = self.sr[Sr::WindowBase as usize];
        ((start | start << 16) >> (base + 1)) & 0x7fff
    }

    /// The overflow that spills the frame `n` quarters up. The next live frame above that one
    /// bounds its size, and the size picks the vector.
    fn overflow_trap(&self, n: u32) -> Trap {
        let base = (self.sr[Sr::WindowBase as usize] + n) & 15;
        let next = self.frames_above() >> n;
        let quarters = if next & 1 != 0 {
            1
        } else if next & 2 != 0 {
            2
        } else {
            3
        };
        Trap::Window {
            base,
            vector: window_vector(quarters, false),
        }
    }

    /// Enters a window handler as `WindowCheck` and `retw` do: `PS.OWB` keeps the current
    /// `WindowBase`, `EPC1` the PC, and `PS.UM` stays.
    fn window_exception(&mut self, base: u32, vector: u32) {
        let ps = self.ps();
        let owb = self.sr[Sr::WindowBase as usize];
        self.sr[Sr::Ps as usize] = (ps & !PS_OWB) | (owb << PS_OWB_SHIFT) | PS_EXCM;
        self.sr[Sr::Epc1 as usize] = self.pc;
        self.sr[Sr::WindowBase as usize] = base;
        let vecbase = self.sr[Sr::Vecbase as usize];
        self.take_vector(vecbase + vector);
    }

    /// 4 cycles on both chips. The syscall rows mix the entry with `rsr` and `wsr` of `epc1`
    /// and with `rfe`, so no capture isolates it. The entry clears the store and divide
    /// follower bits. The loop arm stays.
    fn take_vector(&mut self, target: u32) {
        self.pc = target;
        self.cycle.0 += 4;
        self.refetch = true;
        self.pace &= PACE_LOOP;
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
        self.sr[Sr::DebugCause as usize] = cause;
        self.take_level(C::DEBUG_LEVEL);
    }

    /// The ISA's `takeinterrupt` for a level from 2 up (4.4.6.4, pp 160-161).
    fn take_level(&mut self, level: u32) {
        let ps = self.ps();
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
        self.guard_access(op, addr, width)?;
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
        self.guard_access(op, addr, width)?;
        for (at, part, shift) in parts(addr, width) {
            bus.store(self.cycle, at, part, value >> shift)
                .map_err(|exit| self.access_trap(exit, op, addr, width))?;
        }
        Ok(())
    }

    /// Instruction-bus and unaligned-sync faults are raised before any bus access, so a store
    /// writes nothing. Anything else is split, and a faulting part of that split stops.
    fn guard_access(&self, op: Op, addr: u32, width: Width) -> Result<(), Trap> {
        let natural = addr.is_multiple_of(width.bytes());
        let sync = matches!(op, Op::L32AI | Op::S32RI | Op::S32C1I);
        if C::instruction_bus(addr) {
            if !natural {
                return Err(Trap::Exception(Cause::LoadStoreAlignment, Some(addr)));
            }
            if width != Width::Word || sync {
                return Err(Trap::Exception(Cause::LoadStoreError, Some(addr)));
            }
            return Ok(());
        }
        if sync && !addr.is_multiple_of(4) {
            return if C::sync_unaligned_excepts() {
                Err(Trap::Exception(Cause::LoadStoreAlignment, Some(addr)))
            } else {
                Err(Trap::missing(self.pc, Missing::Unaligned(op)))
            };
        }
        Ok(())
    }

    /// An aligned access that faults raises the exception with its address in `EXCVADDR`. An
    /// unaligned one whose part faults stops. The S3 instruction bus and its unaligned `l32ai`,
    /// `s32ri` and `s32c1i` are decided in [`Self::guard_access`] before this.
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
    ///
    /// The loop-back follows the ISA's fetch pseudo-code (3.5.4.1, p 53): it sets the next PC
    /// before the op runs, so a taken branch as a loop's last op still counts `LCOUNT` down, which
    /// the ISA says is undefined on some implementations (4.3.2.2, p 86).
    ///
    /// An op that raises an exception as a loop's last op has two readings. The literal fetch
    /// pseudo-code (3.5.4.1, p 53, and 4.3.2.4, p 86) counts `LCOUNT` down before `Inst()`, and
    /// neither `Exception` (4.4.1.11, p 145) nor `WindowCheck` (6.1.3, p 246) puts it back, so a
    /// handler that reruns the op loses a pass. The precise reading has the op not complete, so
    /// its loop-back waits for the rerun. This core takes the precise reading and restores
    /// `LCOUNT`. The S3 board confirms it: an exception on the last op of a 3-pass loop's first
    /// pass leaves `LCOUNT` at 2, and the rerun finishes all 3 passes. The classic chip is
    /// unprobed.
    ///
    /// The coprocessor-disabled exception comes after the window check, as the ISA's priority list
    /// orders them (4.4.1.11, p 146).
    fn execute<B: Bus>(&mut self, bus: &mut B) -> Result<Op, Trap> {
        let insn = self.fetch(bus)?;
        if let Some(n) = self.overflow(u32::from(insn.reach)) {
            return Err(self.overflow_trap(n));
        }
        let pc = self.pc;
        if let Some(n) = C::coprocessor(insn.op)
            && self.sr[Sr::CpEnable as usize] >> n & 1 == 0
        {
            let Some(cause) = Cause::coprocessor_disabled(n) else {
                return Err(Trap::missing(pc, Missing::Op(insn.op)));
            };
            self.disabled = Some(CoprocessorDisabled {
                coprocessor: n,
                op: insn.op,
                pc,
            });
            return Err(Trap::Exception(cause, None));
        }
        let lcount = self.sr[Sr::Lcount as usize];
        let seq = pc.wrapping_add(u32::from(insn.len));
        let loops = seq == self.sr[Sr::Lend as usize] && lcount != 0 && self.ps() & PS_EXCM == 0;
        let next = if loops {
            if no_loop_end(insn.op) {
                return Err(Trap::missing(pc, Missing::LoopEnd(insn.op)));
            }
            self.sr[Sr::Lcount as usize] = lcount - 1;
            self.sr[Sr::Lbeg as usize]
        } else {
            seq
        };
        let (cycles, target) = match self.operate(bus, &insn, next) {
            Ok(done) => done,
            Err(trap) => {
                self.sr[Sr::Lcount as usize] = lcount;
                return Err(trap);
            }
        };
        self.note(&insn);
        self.cycle.0 += cycles;
        match target {
            Some(to) => {
                self.cycle.0 += C::redirect_cycles();
                self.refetch = true;
                self.pc = to;
            }
            None => {
                if loops {
                    let arm = self.pace & PACE_LOOP;
                    if arm > 0 {
                        self.cycle.0 += C::loopback_extra(u32::from(arm - 1));
                        let next_arm = (arm + 1).min(3);
                        self.pace = (self.pace & !PACE_LOOP) | next_arm;
                    }
                }
                self.pc = next;
            }
        }
        Ok(insn.op)
    }

    /// The store and divide bits describe the op that just completed. The loop arm is left as the
    /// loop op set it, including across a taken branch, so a later loop-back still pays the first
    /// extra. The FPU and `q` stores, and a `s32c1i` that stored, count as stores, though no board
    /// row times a `memw` after one. A `s32c1i` stored when the old word it left in `at` equals
    /// `SCOMPARE1`.
    fn note(&mut self, insn: &Insn) {
        let op = insn.op;
        self.pace &= PACE_LOOP;
        let stored = match op {
            Op::S8I | Op::S16I | Op::S32I | Op::S32I_N | Op::S32NB | Op::S32E | Op::S32RI => true,
            Op::SSI | Op::SSX | Op::SSIP | Op::SSXP | Op::st_QR => true,
            Op::S32C1I => self.a(insn.fields[0]) == self.sr[Sr::Scompare1 as usize],
            _ => false,
        };
        if stored {
            self.pace |= PACE_STORE;
        }
        if matches!(op, Op::QUOU | Op::QUOS | Op::REMU | Op::REMS) {
            self.pace |= PACE_DIVIDE;
        }
    }

    /// The op's own operation, with `next` the PC after it. Returns its cycles and the target of
    /// a taken branch, jump, call or return.
    fn operate<B: Bus>(
        &mut self,
        bus: &mut B,
        insn: &Insn,
        next: u32,
    ) -> Result<(u64, Option<u32>), Trap> {
        let pc = self.pc;
        let f = insn.fields;
        let mut cycles = 1;
        let mut target = None;
        let sar = self.sr[Sr::Sar as usize];
        match insn.op {
            Op::MEMW => {
                if (self.pace & PACE_STORE) != 0 {
                    cycles = C::memw_after_store();
                }
            }
            Op::NOP
            | Op::NOP_N
            | Op::EXTW
            | Op::ISYNC
            | Op::RSYNC
            | Op::ESYNC
            | Op::DSYNC
            | Op::EXCW => {}

            Op::WITLB | Op::WDTLB => {
                let side = usize::from(insn.op == Op::WDTLB);
                self.regions[side][region(self.a(f[1]))] = (self.a(f[0]) & 0xf) as u8;
            }
            Op::RITLB0 | Op::RDTLB0 => self.set_a(f[0], self.a(f[1]) & REGION_MASK),
            Op::RITLB1 | Op::RDTLB1 => {
                let side = usize::from(insn.op == Op::RDTLB1);
                let at = self.a(f[1]);
                self.set_a(
                    f[0],
                    (at & REGION_MASK) | u32::from(self.regions[side][region(at)]),
                );
            }
            Op::IITLB | Op::IDTLB => {}
            Op::RER => self.set_a(f[0], 0),
            Op::WER => {}

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
            Op::SALT => self.set_a(
                f[0],
                u32::from((self.a(f[1]) as i32) < (self.a(f[2]) as i32)),
            ),
            Op::SALTU => self.set_a(f[0], u32::from(self.a(f[1]) < self.a(f[2]))),
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
                let magnitude = match insn.op {
                    Op::QUOS | Op::REMS => (s as i32).unsigned_abs(),
                    _ => s,
                };
                let remainder = matches!(insn.op, Op::REMU | Op::REMS);
                cycles = C::divide_cycles((self.pace & PACE_DIVIDE) != 0, remainder, magnitude);
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
            // Both chips' `core-isa.h` set `XCHAL_UNALIGNED_LOAD_HW` and `XCHAL_UNALIGNED_STORE_HW`
            // (classic lines 173-174, S3 lines 204-205). That Hardware Alignment Option builds on
            // the Unaligned Exception Option and does not list `l32e` or `s32e` among the ops that
            // may still raise `LoadStoreAlignmentCause` (4.6, p 168), so they split as `l32i` and
            // `s32i` do.
            Op::L32I | Op::L32I_N | Op::L32E => {
                let value =
                    self.load(bus, insn.op, self.a(f[1]).wrapping_add(f[2]), Width::Word)?;
                self.set_a(f[0], value);
            }
            Op::L32AI => {
                let value =
                    self.load(bus, insn.op, self.a(f[1]).wrapping_add(f[2]), Width::Word)?;
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
            Op::S32I | Op::S32I_N | Op::S32NB | Op::S32E => {
                self.store(
                    bus,
                    insn.op,
                    self.a(f[1]).wrapping_add(f[2]),
                    Width::Word,
                    self.a(f[0]),
                )?;
            }
            Op::S32RI => {
                self.store(
                    bus,
                    insn.op,
                    self.a(f[1]).wrapping_add(f[2]),
                    Width::Word,
                    self.a(f[0]),
                )?;
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
            Op::MOVSP => {
                if self.frames_below() == 0 {
                    return Err(Trap::Exception(Cause::Alloca, None));
                }
                self.set_a(f[0], self.a(f[1]));
            }
            Op::ROTW => {
                let base = self.sr[Sr::WindowBase as usize].wrapping_add(f[0]) & 15;
                self.sr[Sr::WindowBase as usize] = base;
            }
            Op::RFWO | Op::RFWU => {
                let ps = self.ps();
                let base = self.sr[Sr::WindowBase as usize];
                let bit = 1 << base;
                let start = self.sr[Sr::WindowStart as usize];
                self.sr[Sr::WindowStart as usize] = if insn.op == Op::RFWO {
                    start & !bit
                } else {
                    start | bit
                };
                self.sr[Sr::WindowBase as usize] = (ps & PS_OWB) >> PS_OWB_SHIFT;
                self.sr[Sr::Ps as usize] = ps & !PS_EXCM;
                target = Some(self.sr[Sr::Epc1 as usize]);
            }

            Op::LOOP | Op::LOOPNEZ | Op::LOOPGTZ => {
                let count = self.a(f[0]);
                self.sr[Sr::Lcount as usize] = count.wrapping_sub(1);
                self.sr[Sr::Lbeg as usize] = next;
                self.sr[Sr::Lend as usize] = f[1];
                self.pace = (self.pace & !PACE_LOOP) | 1;
                let skip = match insn.op {
                    Op::LOOPNEZ => count == 0,
                    Op::LOOPGTZ => count as i32 <= 0,
                    _ => false,
                };
                target = taken(skip, f[1]);
            }

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
            Op::WAITI => {
                self.sr[Sr::Ps as usize] = (self.ps() & !PS_INTLEVEL) | f[0];
                self.waiting = true;
            }
            Op::WSR_INTSET => {
                self.sr[Sr::Interrupt as usize] |= self.a(f[0]) & C::INTERRUPTS.software;
            }
            Op::WSR_INTCLEAR => {
                let clears = C::INTERRUPTS.software | C::INTERRUPTS.extern_edge;
                self.sr[Sr::Interrupt as usize] &= !(self.a(f[0]) & clears);
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

            _ => target = self.option_op(bus, insn)?,
        }
        Ok((cycles, target))
    }

    fn move_if(&mut self, f: [u32; 8], test: impl Fn(u32) -> bool) {
        if test(self.a(f[2])) {
            self.set_a(f[0], self.a(f[1]));
        }
    }

    fn register_op(&mut self, access: Access, sr: Sr, t: u32) {
        let mut old = self.special(sr);
        if sr == Sr::Icount
            && matches!(access, Access::Read | Access::Exchange)
            && C::icount_read_lag()
            && self.cintlevel() < self.sr[Sr::IcountLevel as usize]
        {
            old = old.wrapping_sub(1);
        }
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
    /// `frame`. Its `WindowCheck (00, PS.CALLINC, 00)` tests the quarters up to `CALLINC`, before
    /// the rotation (8.3.106, p 427). The callee's own references check the rest of its window.
    fn entry(&mut self, pc: u32, s: u32, frame: u32) -> Result<(), Trap> {
        let ps = self.ps();
        let inc = (ps & PS_CALLINC) >> PS_CALLINC_SHIFT;
        if let Some(n) = self.overflow(inc) {
            return Err(self.overflow_trap(n));
        }
        if s > 3 || ps & PS_WOE == 0 || ps & PS_EXCM != 0 {
            return Err(Trap::missing(pc, Missing::Op(Op::ENTRY)));
        }
        let sp = self.a(s).wrapping_sub(frame);
        let base = (self.sr[Sr::WindowBase as usize] + inc) & 15;
        self.sr[Sr::WindowBase as usize] = base;
        self.sr[Sr::WindowStart as usize] |= 1 << base;
        self.set_a(s, sp);
        Ok(())
    }

    /// Rotates the window back by the increment in `a0`'s top two bits and returns the target,
    /// or raises the underflow when the caller's frame was spilled (6.1.4, p 247). The ISA leaves
    /// it undefined when the nearest frame below, `m` quarters down, is not the caller's.
    fn retw(&mut self, pc: u32, op: Op) -> Result<u32, Trap> {
        let ps = self.ps();
        let ret = self.a(0);
        let n = ret >> 30;
        let below = self.frames_below();
        let m = if below == 0 {
            0
        } else {
            below.trailing_zeros() + 1
        };
        if n == 0 || (m != 0 && m != n) || ps & PS_WOE == 0 || ps & PS_EXCM != 0 {
            return Err(Trap::missing(pc, Missing::Op(op)));
        }
        let base = self.sr[Sr::WindowBase as usize];
        let caller = base.wrapping_sub(n) & 15;
        let start = self.sr[Sr::WindowStart as usize];
        if start >> caller & 1 == 0 {
            return Err(Trap::Window {
                base: caller,
                vector: window_vector(n, true),
            });
        }
        self.sr[Sr::WindowStart as usize] = start & !(1 << base);
        self.sr[Sr::WindowBase as usize] = caller;
        Ok((pc & 0xc000_0000) | (ret & 0x3fff_ffff))
    }

    /// Bit `k - 1` is `WindowStart` bit `WindowBase - k`, for `k` from 1 to 3.
    fn frames_below(&self) -> u32 {
        let start = self.sr[Sr::WindowStart as usize];
        let base = self.sr[Sr::WindowBase as usize];
        (1..=3).fold(0, |bits, k| {
            bits | (((start >> (base.wrapping_sub(k) & 15)) & 1) << (k - 1))
        })
    }
}

fn taken(condition: bool, to: u32) -> Option<u32> {
    condition.then_some(to)
}

/// The `INTERRUPT` bits that hold until cleared: software, edge and timer.
const fn latched<C: Config>() -> u32 {
    let interrupts = C::INTERRUPTS;
    interrupts.software
        | interrupts.extern_edge
        | interrupts.timer_mask(0)
        | interrupts.timer_mask(1)
        | interrupts.timer_mask(2)
}

/// The access rights each region has at reset: 2, bypass the cache, read, write and execute.
const REGION_RESET: u8 = 2;
/// The address bits that pick a region.
const REGION_MASK: u32 = 0xe000_0000;

/// The region `addr` lies in.
fn region(addr: u32) -> usize {
    (addr >> 29) as usize
}

/// The ops the ISA says must not end a loop (4.3.2.2, p 86).
fn no_loop_end(op: Op) -> bool {
    matches!(
        op,
        Op::CALL0
            | Op::CALL4
            | Op::CALL8
            | Op::CALL12
            | Op::CALLX0
            | Op::CALLX4
            | Op::CALLX8
            | Op::CALLX12
            | Op::ISYNC
            | Op::WAITI
            | Op::RSR_LCOUNT
    )
}

/// The offset from `VECBASE` of the overflow or underflow vector for a frame of `quarters`
/// quarters, 1 to 3. Both chips' `core-isa.h` place them 0x80 apart, underflow 0x40 after.
const fn window_vector(quarters: u32, underflow: bool) -> u32 {
    (quarters - 1) * 0x80 + if underflow { 0x40 } else { 0 }
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
