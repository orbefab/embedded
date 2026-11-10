//! One instruction run from a fixed register file and data space, and what it changed.

use std::sync::Arc;

use super::decode::{Op, decode};
use super::{Activity, Core};
use crate::IO_BASE;
use crate::bus::Bus;

/// Program-counter mask for this sweep, 16 Ki words.
pub(super) const MASK: u16 = 16 * 1024 - 1;

/// Stack pointer the sweep starts from. It sits inside the flat data space.
pub(super) const INITIAL_SP: u16 = 0x08FF;

const SRAM_BASE: u16 = 0x0100;
const SRAM_END: u16 = 0x08FF;
/// I/O address `0x3F`, the address the instruction set assigns to `SREG`.
const SREG_ADDR: u16 = 0x005F;
/// The two bytes under `SREG`. The sweep's stack pointer lives here.
const SPL_ADDR: u16 = 0x005D;
const SPH_ADDR: u16 = 0x005E;

/// Flat data space. The register file, `SREG` and the stack pointer stay in the core.
#[derive(Clone)]
pub(super) struct Mem {
    data: Vec<u8>,
    sleep_enabled: bool,
    sleep_armed: bool,
    halt: u8,
    io_writes: Vec<(u16, u8)>,
    pub(super) wdrs: u32,
}

impl Mem {
    pub(super) fn new() -> Self {
        Self {
            data: vec![0; 1 << 16],
            sleep_enabled: false,
            sleep_armed: false,
            halt: 0,
            io_writes: Vec::new(),
            wdrs: 0,
        }
    }

    pub(super) fn sram(&self) -> &[u8] {
        &self.data[usize::from(SRAM_BASE)..=usize::from(SRAM_END)]
    }

    pub(super) fn restore_sram(&mut self, bytes: &[u8]) {
        self.data[usize::from(SRAM_BASE)..=usize::from(SRAM_END)].copy_from_slice(bytes);
    }

    /// A host write. It does not log, and it adds no stall.
    pub(super) fn poke(&mut self, core: &mut Core, addr: u16, value: u8) {
        self.write(core, addr, value, false);
    }

    pub(super) fn cpu_halt(&self) -> u8 {
        self.halt
    }

    pub(super) fn set_halt(&mut self, halt: u8) {
        self.halt = halt;
    }

    pub(super) fn sleep_armed(&self) -> bool {
        self.sleep_armed
    }

    pub(super) fn set_sleep_enabled(&mut self, enabled: bool) {
        self.sleep_enabled = enabled;
    }

    pub(super) fn io_writes(&self) -> &[(u16, u8)] {
        &self.io_writes
    }

    fn write(&mut self, core: &mut Core, addr: u16, value: u8, log: bool) {
        if addr <= 0x001F {
            core.set_reg(addr as u8, value);
            return;
        }
        if addr == SREG_ADDR {
            core.set_sreg(value);
            return;
        }
        if addr == SPL_ADDR {
            core.set_sp_lo(value);
            return;
        }
        if addr == SPH_ADDR {
            core.set_sp_hi(value);
            return;
        }
        self.data[usize::from(addr)] = value;
        if log && (IO_BASE..=0x00FF).contains(&addr) {
            self.io_writes.push((addr, value));
        }
    }

    fn read(&self, core: &Core, addr: u16) -> u8 {
        if addr <= 0x001F {
            return core.reg(addr as u8);
        }
        if addr == SREG_ADDR {
            return core.sreg();
        }
        if addr == SPL_ADDR {
            return core.sp() as u8;
        }
        if addr == SPH_ADDR {
            return (core.sp() >> 8) as u8;
        }
        self.data[usize::from(addr)]
    }
}

impl Bus for Mem {
    #[inline(always)]
    fn load(&mut self, core: &Core, addr: u16) -> u8 {
        self.read(core, addr)
    }

    #[inline(always)]
    fn store(&mut self, core: &mut Core, addr: u16, value: u8, _cycles: u64) {
        self.write(core, addr, value, true);
    }

    #[inline(always)]
    fn peek(&self, core: &Core, addr: u16) -> u8 {
        self.read(core, addr)
    }

    #[inline(always)]
    fn steady(&self, _addr: u16) -> bool {
        true
    }

    #[inline(always)]
    fn store_idle(&self, core: &Core, addr: u16, value: u8) -> bool {
        self.read(core, addr) == value
    }

    fn write_bit(&mut self, core: &mut Core, addr: u16, bit: u8, set: bool) {
        let current = self.read(core, addr);
        let mask = 1u8 << (bit & 7);
        let written = if set { current | mask } else { current & !mask };
        self.write(core, addr, written, true);
    }

    fn sleep_enabled(&self) -> bool {
        self.sleep_enabled
    }

    fn arm_sleep(&mut self) {
        self.sleep_armed = true;
    }

    fn watchdog_reset_timer(&mut self) {
        self.wdrs = self.wdrs.wrapping_add(1);
    }

    fn cpu_halt(&self) -> u8 {
        self.halt
    }

    fn clear_cpu_halt(&mut self) {
        self.halt = 0;
    }
}

const SITES: [Site; 6] = [
    Site::Before(0x0000),
    Site::Before(0x9100),
    Site::Before(0x940E),
    Site::Before(0xC123),
    Site::Last,
    Site::Top,
];

/// `LDS r17`, `STS r17`, `JMP` and `CALL`.
const FIRST_WORDS: [u16; 4] = [0x9110, 0x9310, 0x940C, 0x940E];

/// `CPSE r0, r0`, which always skips.
const SKIP: u16 = 0x1000;

/// Where a word sits in flash.
#[derive(Clone, Copy)]
enum Site {
    /// At 0, before `following`.
    Before(u16),
    /// At 1, the last word of the image.
    Last,
    /// At the last word of a full flash.
    Top,
}

/// How a swept word meets the core.
#[derive(Clone, Copy)]
enum At {
    /// It runs at a site.
    Site(Site),
    /// It is the second word of the instruction given.
    After(u16),
    /// A taken skip passes over it.
    Skipped,
}

/// One run of a swept word: the image, the program counter, and the op it runs.
pub(super) struct Case {
    swept: u16,
    at: At,
    flash: Arc<[u16]>,
    pub(super) pc: u16,
    pub(super) op: Op,
    lane: Lane,
}

impl std::fmt::Display for Case {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "word {:#06x} ", self.swept)?;
        match self.at {
            At::Site(Site::Before(following)) => write!(f, "before {following:#06x}")?,
            At::Site(Site::Last) => write!(f, "as the last word")?,
            At::Site(Site::Top) => write!(f, "at the top of flash")?,
            At::After(first) => write!(f, "after {first:#06x}")?,
            At::Skipped => write!(f, "after a taken skip")?,
        }
        write!(f, ", {:?}", self.op)
    }
}

/// What a run reads and writes, from fixed starting states. The two seeds hold complementary `SREG`
/// bytes and write complementary bytes to the I/O space below the stack pointer. So each flag
/// takes both values, as does each I/O byte, and one seed has sleep enabled.
pub(super) struct Rig {
    seeds: [Seed; 2],
    sram: Vec<u8>,
    top: Arc<[u16]>,
}

/// A starting core and bus, and a copy of the bus that [`Lane::Quiet`] runs share, so they skip
/// the copy.
struct Seed {
    core: Core,
    bus: Mem,
    quiet: Mem,
}

impl Rig {
    pub(super) fn new() -> Self {
        let mut bus = Mem::new();
        let painted: Vec<u8> = (0..bus.sram().len())
            .map(|i| (i as u8).wrapping_add((i >> 8) as u8).wrapping_mul(17))
            .collect();
        bus.restore_sram(&painted);
        let first = seeded(1, [0x0200, 0x0400, 0x0600]);
        let mut second = seeded(7, [0x0020, 0x0700, 0x0300]);
        second.sreg = !first.sreg;
        let seeds = [(first, 0x00), (second, 0xFF)].map(|(mut core, flip)| {
            let mut bus = bus.clone();
            for addr in IO_BASE..SPL_ADDR {
                bus.poke(
                    &mut core,
                    addr,
                    (addr as u8).wrapping_mul(0x5B) ^ 0xA6 ^ flip,
                );
            }
            // The second seed may sleep. The first may not.
            bus.set_sleep_enabled(flip == 0xFF);
            Seed {
                core,
                quiet: bus.clone(),
                bus,
            }
        });
        Self {
            seeds,
            sram: painted,
            top: Arc::from(vec![0u16; usize::from(MASK) + 1]),
        }
    }

    /// Every run of `swept`: as the word at each site, as the second word of each two-word
    /// instruction in [`FIRST_WORDS`], and as the word a taken skip passes over.
    pub(super) fn cases(&mut self, swept: u16) -> Vec<Case> {
        let mut cases = Vec::with_capacity(SITES.len() + FIRST_WORDS.len() + 1);
        for site in SITES {
            let (flash, pc, following) = self.place(site, swept);
            let op = decode(swept, following, pc, MASK);
            cases.push(Case {
                swept,
                at: At::Site(site),
                flash,
                pc,
                op,
                lane: if reaches_bus(op) {
                    Lane::Copy
                } else {
                    Lane::Quiet
                },
            });
        }
        for first in FIRST_WORDS {
            cases.push(Case {
                swept,
                at: At::After(first),
                flash: Arc::from([first, swept]),
                pc: 0,
                op: decode(first, swept, 0, MASK),
                lane: if (0x0020..=0x00FF).contains(&swept) {
                    Lane::Copy
                } else {
                    Lane::Quiet
                },
            });
        }
        cases.push(Case {
            swept,
            at: At::Skipped,
            flash: Arc::from([SKIP, swept]),
            pc: 0,
            op: decode(SKIP, swept, 0, MASK),
            lane: Lane::Quiet,
        });
        cases
    }

    /// The image, the program counter, and the word after `word` that a fetch returns.
    fn place(&mut self, site: Site, word: u16) -> (Arc<[u16]>, u16, u16) {
        match site {
            Site::Before(following) => (Arc::from([word, following]), 0, following),
            Site::Last => (Arc::from([0x0000, word]), 1, 0),
            Site::Top => {
                Arc::get_mut(&mut self.top).expect("no core holds the image")[usize::from(MASK)] =
                    word;
                (Arc::clone(&self.top), MASK, 0)
            }
        }
    }

    /// Runs `exec` on `case` once from each seed. One that reaches the bus runs on a copy of the
    /// seed's bus, and its effect includes the I/O space.
    pub(super) fn effects(
        &mut self,
        case: &Case,
        mut exec: impl FnMut(&mut Core, &mut Mem),
    ) -> [Effect; 2] {
        let sram = &self.sram;
        self.seeds.each_mut().map(|seed| {
            let mut core = seed.core.clone();
            core.flash = Arc::clone(&case.flash);
            core.pc = case.pc;
            if case.lane == Lane::Copy {
                let mut bus = seed.bus.clone();
                exec(&mut core, &mut bus);
                return Effect::of(&core, &bus, sram, true);
            }
            exec(&mut core, &mut seed.quiet);
            let effect = Effect::of(&core, &seed.quiet, sram, false);
            if effect.halt != 0 || !effect.io_writes.is_empty() {
                seed.quiet = seed.bus.clone();
            } else if !effect.sram.is_empty() {
                seed.quiet.restore_sram(sram);
            }
            effect
        })
    }
}

/// The bus a run uses. `Quiet` is shared between runs, for one that writes nothing past SRAM.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Copy,
    Quiet,
}

fn reaches_bus(op: Op) -> bool {
    !matches!(
        op,
        Op::Nop
            | Op::Add(..)
            | Op::Adc(..)
            | Op::Sub(..)
            | Op::Sbc(..)
            | Op::And(..)
            | Op::Or(..)
            | Op::Eor(..)
            | Op::Mov(..)
            | Op::Cp(..)
            | Op::Cpc(..)
            | Op::Cpse { .. }
            | Op::Subi(..)
            | Op::Sbci(..)
            | Op::Andi(..)
            | Op::Ori(..)
            | Op::Ldi(..)
            | Op::Cpi(..)
            | Op::Adiw(..)
            | Op::Sbiw(..)
            | Op::Movw(..)
            | Op::Mul(..)
            | Op::Muls(..)
            | Op::Mulsu(..)
            | Op::Fmul(..)
            | Op::Fmuls(..)
            | Op::Fmulsu(..)
            | Op::Lpm { .. }
            | Op::Rjmp(_)
            | Op::Brbs { .. }
            | Op::Brbc { .. }
            | Op::Countdown(_)
            | Op::Jmp(_)
            | Op::Ijmp
            | Op::Bst { .. }
            | Op::Bld { .. }
            | Op::Sbrc { .. }
            | Op::Sbrs { .. }
            | Op::Bset(_)
            | Op::Sei
            | Op::Bclr(_)
            | Op::Com(_)
            | Op::Neg(_)
            | Op::Swap(_)
            | Op::Inc(_)
            | Op::Asr(_)
            | Op::Lsr(_)
            | Op::Ror(_)
            | Op::Dec(_)
            | Op::Spm
            | Op::Break
            | Op::Unknown(_)
    )
}

/// `pointers` are the bases of X, Y and Z. Each pointer is its base plus a byte from the seed.
fn seeded(seed: u32, pointers: [u16; 3]) -> Core {
    let mut core = Core::new(INITIAL_SP, MASK);
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    for slot in &mut core.r {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *slot = x as u8;
    }
    for (lo, base) in [26, 28, 30].into_iter().zip(pointers) {
        let ptr = base + (x as u16 & 0xFF);
        core.r[lo] = ptr as u8;
        core.r[lo + 1] = (ptr >> 8) as u8;
        x = x.rotate_left(8);
    }
    core.sreg = (x >> 8) as u8;
    core.sp = 0x0480 + (x as u16 & 0xFF);
    core.cycles = 1000;
    core
}

/// The state a single instruction can change. `sram` lists the bytes that differ from the
/// painted SRAM, and `io` is the I/O space as `peek` reads it, for an op that reaches the bus.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Effect {
    r: [u8; 32],
    sreg: u8,
    pc: u16,
    sp: u16,
    cycles: u64,
    unknown: Option<u16>,
    irq_delay: bool,
    spm: Vec<(u16, u16)>,
    stopped: bool,
    activity: Activity,
    wdr_executions: u32,
    halt: u8,
    sleep_armed: bool,
    io_writes: Vec<(u16, u8)>,
    sram: Vec<(usize, u8)>,
    io: Vec<u8>,
}

impl Effect {
    fn of(core: &Core, bus: &Mem, painted: &[u8], io: bool) -> Self {
        let mut sram = Vec::new();
        for (block, (now, was)) in bus.sram().chunks(64).zip(painted.chunks(64)).enumerate() {
            if now != was {
                sram.extend(
                    (now.iter().zip(was).enumerate())
                        .filter(|(_, (now, was))| now != was)
                        .map(|(i, (now, _))| (block * 64 + i, *now)),
                );
            }
        }
        Self {
            r: core.r,
            sreg: core.sreg,
            pc: core.pc,
            sp: core.sp,
            cycles: core.cycles,
            unknown: core.unknown,
            irq_delay: core.irq_delay,
            spm: core.spm.iter().map(|note| (note.z, note.word)).collect(),
            stopped: core.stopped,
            activity: core.activity,
            wdr_executions: core.wdr_executions,
            halt: bus.cpu_halt(),
            sleep_armed: bus.sleep_armed(),
            io_writes: bus.io_writes().to_vec(),
            sram,
            io: if io {
                (0x20..=0xFF).map(|addr| bus.peek(core, addr)).collect()
            } else {
                Vec::new()
            },
        }
    }

    /// FNV-1a over every field, in a byte order that does not depend on the platform.
    pub(super) fn digest(&self, hash: &mut u64) {
        let mut eat = |bytes: &[u8]| {
            for byte in bytes {
                *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01B3);
            }
        };
        eat(&self.r);
        eat(&[self.sreg]);
        eat(&self.pc.to_le_bytes());
        eat(&self.sp.to_le_bytes());
        eat(&self.cycles.to_le_bytes());
        eat(&self.unknown.map_or([0xFF; 3], |w| {
            let [lo, hi] = w.to_le_bytes();
            [0, lo, hi]
        }));
        eat(&[u8::from(self.irq_delay), u8::from(self.stopped)]);
        eat(&(self.spm.len() as u32).to_le_bytes());
        for (z, word) in &self.spm {
            eat(&z.to_le_bytes());
            eat(&word.to_le_bytes());
        }
        match self.activity {
            Activity::Fetch => eat(&[0]),
            Activity::Sleep => eat(&[1]),
            Activity::Startup { until, from_reset } => {
                eat(&[2, u8::from(from_reset)]);
                eat(&until.to_le_bytes());
            }
            Activity::Held => eat(&[3]),
        }
        eat(&self.wdr_executions.to_le_bytes());
        eat(&[self.halt, u8::from(self.sleep_armed)]);
        eat(&(self.io_writes.len() as u32).to_le_bytes());
        for (addr, value) in &self.io_writes {
            eat(&addr.to_le_bytes());
            eat(&[*value]);
        }
        eat(&(self.sram.len() as u32).to_le_bytes());
        for (i, value) in &self.sram {
            eat(&(*i as u16).to_le_bytes());
            eat(&[*value]);
        }
        eat(&self.io);
    }
}

pub(super) const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
