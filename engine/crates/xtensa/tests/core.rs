//! The core on a flat test bus: code words from a list, RAM in a `PagedRam`, and a stop at the
//! first fetch past the code.

use std::ops::Range;

use sim_xtensa::{
    Bus, Cause, Config, Core, CoreId, Cycle, Esp32, Esp32s3, Exit, Missing, Op, PagedRam, Sr, Stop,
    Unimplemented, Width,
};

pub const CODE: u32 = 0x4037_0000;
pub const RAM: u32 = 0x3fc8_8000;

#[derive(Clone)]
pub struct Flat {
    code: Vec<u8>,
    pub ram: PagedRam,
    /// RAM that loads and refuses stores.
    pub read_only: Range<u32>,
    /// Interrupt lines that are high from the start.
    pub lines: u32,
    /// Interrupt lines that rise at a cycle and stay high.
    pub rise: Option<(Cycle, u32)>,
    /// How many times the core read the lines.
    pub polls: u64,
    /// Asks the core to return before each step.
    pub yields: bool,
}

impl Flat {
    /// `hex` is the code from `CODE`, padded with zero bytes to whole words. RAM is the S3's
    /// 512 KB of internal SRAM.
    pub fn new(hex: &str) -> Flat {
        let mut code: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        code.resize(code.len().next_multiple_of(4), 0);
        Flat {
            code,
            ram: PagedRam::new(RAM, 512 * 1024),
            read_only: 0..0,
            lines: 0,
            rise: None,
            polls: 0,
            yields: false,
        }
    }
}

impl Bus for Flat {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        let at = addr.wrapping_sub(CODE) as usize;
        match self.code.get(at..at + 4) {
            Some(word) => Ok(u32::from_le_bytes(word.try_into().unwrap())),
            None => Err(Exit::Stop),
        }
    }

    fn load(&mut self, _now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        if self.ram.contains(addr) {
            Ok(self.ram.read(addr, width))
        } else {
            Err(Exit::Fault(Cause::LoadProhibited))
        }
    }

    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        self.check_store(now, addr)?;
        self.ram.write(addr, width, value);
        Ok(())
    }

    fn check_store(&mut self, _now: Cycle, addr: u32) -> Result<(), Exit> {
        if self.ram.contains(addr) && !self.read_only.contains(&addr) {
            Ok(())
        } else {
            Err(Exit::Fault(Cause::StoreProhibited))
        }
    }

    fn interrupts(&mut self, now: Cycle) -> u32 {
        self.polls += 1;
        match self.rise {
            Some((at, lines)) if now >= at => self.lines | lines,
            _ => self.lines,
        }
    }

    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        self.rise.map(|(at, _)| at).filter(|at| now < *at)
    }

    fn yields(&self) -> bool {
        self.yields
    }
}

pub fn core_at<C: Config>(pc: u32) -> Core<C> {
    let mut core = Core::new();
    core.set_pc(pc);
    core
}

fn unmodeled_op_stops_before_it<C: Config>() {
    let rfdo = [0x00u8, 0xe0, 0xf1];
    assert_eq!(
        C::render(&C::decode(&rfdo, CODE).unwrap()),
        "rfdo\t0",
        "{}",
        C::NAME
    );
    let mut bus = Flat::new("00e0f1");
    let mut core = core_at::<C>(CODE);
    let stop = core.run_until(&mut bus, Cycle(100));
    let want = Unimplemented {
        pc: CODE,
        missing: Missing::Op(Op::RFDO),
    };
    assert_eq!(stop, Stop::Unimplemented(want), "{}", C::NAME);
    assert_eq!(core.pc(), CODE);
    assert_eq!(core.cycle(), Cycle(0));
    assert_eq!(want.to_string(), "`rfdo` at 0x40370000 is not modeled");
}

#[test]
fn unmodeled_op_stops_before_it_on_both_chips() {
    unmodeled_op_stops_before_it::<Esp32>();
    unmodeled_op_stops_before_it::<Esp32s3>();
}

/// `witlb` and `wdtlb` set a region's attribute, which `ritlb1` and `rdtlb1` read back beside the
/// region's address bits, and a region never written reads 2. `PRID` is core 0's, and `rer`
/// reads 0.
fn region_prid_and_external_registers<C: Config>() {
    // witlb a2, a3; wdtlb a2, a3; ritlb1 a4, a3; rdtlb1 a7, a8; ritlb0 a9, a3; rsr.prid a5;
    // rer a6, a3; wer a6, a3; rfdo 0.
    let mut bus = Flat::new("20635020e35040735070f85090335050eb0360634060734000e0f1");
    let mut core = core_at::<C>(CODE);
    core.set_a(2, 0xf);
    core.set_a(3, 0x4123_4567);
    core.set_a(6, 0x5a5a);
    core.set_a(8, 0x6000_0010);
    let stop = core.run_until(&mut bus, Cycle(100));
    let want = Unimplemented {
        pc: CODE + 0x18,
        missing: Missing::Op(Op::RFDO),
    };
    assert_eq!(stop, Stop::Unimplemented(want), "{}", C::NAME);
    assert_eq!(core.a(4), 0x4000_000f, "{}", C::NAME);
    assert_eq!(core.a(7), 0x6000_0002, "{}", C::NAME);
    assert_eq!(core.a(9), 0x4000_0000, "{}", C::NAME);
    assert_eq!(core.a(5), 0xcdcd, "{}", C::NAME);
    assert_eq!(core.a(6), 0, "{}", C::NAME);
}

#[test]
fn region_prid_and_external_registers_on_both_chips() {
    region_prid_and_external_registers::<Esp32>();
    region_prid_and_external_registers::<Esp32s3>();
}

#[test]
fn undefined_word_raises_illegal_instruction() {
    let mut bus = Flat::new("000100");
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_special(Sr::Ps, 0);
    core.set_special(Sr::Vecbase, 0x4000_0000);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Bus);
    assert_eq!(core.special(Sr::ExcCause), Cause::IllegalInstruction as u32);
    assert_eq!(core.special(Sr::Epc1), CODE);
    assert_eq!(core.special(Sr::Ps), 1 << 4, "PS.EXCM is set");
    assert_eq!(core.pc(), 0x4000_0300);
}

/// `nop.n` in the last two bytes of the code runs without a fetch of the next word, so the core
/// stops at the next instruction.
#[test]
fn two_byte_op_before_an_unmapped_word_runs() {
    let mut bus = Flat::new("00003df0");
    let mut core = core_at::<Esp32>(CODE + 2);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Bus);
    assert_eq!(core.pc(), CODE + 4);
    assert_eq!(core.cycle(), Cycle(1));
}

/// A core reset at a cycle reads its own `PRID` and counts `CCOUNT` from 0 there. `skip_to`
/// never moves the cycle back, and `CCOUNT` counts the gap it skips.
#[test]
fn a_reset_app_cpu_reads_its_prid_and_counts_from_its_reset() {
    // rsr.prid a5; rsr.ccount a6.
    let mut bus = Flat::new("50eb0360ea03");
    let mut core = Core::<Esp32s3>::reset(CoreId::App, Cycle(1000));
    core.set_pc(CODE);
    assert_eq!(core.id(), CoreId::App);
    assert_eq!(core.special(Sr::Ccount), 0);
    assert_eq!(core.run_until(&mut bus, Cycle(2000)), Stop::Bus);
    assert_eq!((core.a(5), core.a(6)), (0xabab, 1));
    core.skip_to(Cycle(500));
    assert_eq!(core.cycle(), Cycle(1002));
    core.skip_to(Cycle(3000));
    assert_eq!(core.special(Sr::Ccount), 2000);
}

/// A bus that asks the core to yield stops it before its next step, after the limit check.
#[test]
fn a_yielding_bus_stops_the_core_before_its_next_step() {
    let mut bus = Flat::new("50eb03");
    bus.yields = true;
    let mut core = core_at::<Esp32s3>(CODE);
    assert_eq!(core.run_until(&mut bus, Cycle(0)), Stop::Limit);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Yield);
    assert_eq!((core.pc(), core.cycle(), bus.polls), (CODE, Cycle(0), 0));
}

/// `Core` is copied at every landing, so its size is pinned.
#[test]
fn core_size_is_pinned() {
    assert_eq!(size_of::<Core<Esp32>>(), 936);
    assert_eq!(size_of::<Core<Esp32s3>>(), 936);
}

/// `DBREAKAn` and `DBREAKCn` read back through `wsr` and `rsr`.
#[test]
fn the_data_breakpoint_registers_read_back() {
    let regs = [
        (144u8, 0x3fc8_8000),
        (145, 0x3fc8_8020),
        (160, 0x8000_0020),
        (161, 0x3f),
    ];
    for (sr, value) in regs {
        let hex = format!("20{sr:02x}1330{sr:02x}03");
        let mut bus = Flat::new(&hex);
        let mut core = core_at::<Esp32s3>(CODE);
        core.set_a(2, value);
        assert_eq!(core.run_until(&mut bus, Cycle(10)), Stop::Bus);
        assert_eq!(core.a(3), value, "sr {sr}");
    }
}

/// A 32-byte store-only window stops a byte inside it and not the accesses outside it. The
/// window starts 32 bytes into RAM, so the word at `base - 4` lies in the RAM. Mask `0x3f` with
/// the store bit set compares every low bit, so a store at `base + 1` misses. Mask `0x3f` alone
/// arms nothing. With no watch armed, nothing stops.
#[test]
fn a_store_watch_stops_on_a_byte_inside_its_window() {
    let base = RAM + 32;
    let inside = access(base, "22431f", base, 0x8000_0020);
    assert!(
        matches!(inside.stop, Stop::Watchpoint { pc, addr } if pc == CODE && addr == base + 31),
        "{:?}",
        inside.stop
    );
    assert_eq!(inside.bus.ram.read(base + 31, Width::Byte), 0);

    let past = access(base, "224300", base + 32, 0x8000_0020);
    assert_eq!(past.stop, Stop::Bus);
    assert_eq!(past.bus.ram.read(base + 32, Width::Byte), 0x11);

    let before = access(base, "226300", base - 4, 0x8000_0020);
    assert_eq!(before.stop, Stop::Bus);
    assert_eq!(before.bus.ram.read(base - 4, Width::Word), 0x11);

    let load = access(base, "222300", base, 0x8000_0020);
    assert_eq!(load.stop, Stop::Bus);

    let polarity = access(base, "224300", base + 1, 0x8000_003f);
    assert_eq!(polarity.stop, Stop::Bus);
    assert_eq!(polarity.bus.ram.read(base + 1, Width::Byte), 0x11);

    let mask = access(base, "224300", base + 1, 0x3f);
    assert_eq!(mask.stop, Stop::Bus);
    assert_eq!(mask.bus.ram.read(base + 1, Width::Byte), 0x11);

    let open = access(base, "224300", base, 0);
    assert_eq!(open.stop, Stop::Bus);
}

/// Watch 1 stops a byte store inside a 32-byte window. `DBREAKC1` is `0x80000020`.
#[test]
fn a_store_on_watch_1_stops_inside_its_window() {
    let base = RAM + 32;
    let inside = access_on(1, base, "22431f", base, 0x8000_0020);
    assert!(
        matches!(inside.stop, Stop::Watchpoint { pc, addr } if pc == CODE && addr == base + 31),
        "{:?}",
        inside.stop
    );
    assert_eq!(inside.bus.ram.read(base + 31, Width::Byte), 0);
    assert_eq!(inside.a3, base);
}

/// A load watch stops `l32i a3, a3, 0` before the load, so `a3` stays the address. `DBREAKC0` is
/// `0x40000020`.
#[test]
fn a_load_watch_stops_before_the_load() {
    let base = RAM + 32;
    let load = access_on(0, base, "322300", base, 0x4000_0020);
    assert!(
        matches!(load.stop, Stop::Watchpoint { pc, addr } if pc == CODE && addr == base),
        "{:?}",
        load.stop
    );
    assert_eq!(load.a3, base);
    assert_eq!(load.bus.ram.read(base, Width::Word), 0);
}

struct Watched {
    stop: Stop,
    bus: Flat,
    a3: u32,
}

/// Runs the one instruction in `hex` at `addr` in `a3`, with `DBREAKA0` at `base` and `DBREAKC0`
/// `control`. `a2` holds `0x11`.
fn access(base: u32, hex: &str, addr: u32, control: u32) -> Watched {
    access_on(0, base, hex, addr, control)
}

/// As [`access`], on watch `which`. Watch 1 uses `DBREAKA1` and `DBREAKC1`.
fn access_on(which: u8, base: u32, hex: &str, addr: u32, control: u32) -> Watched {
    let mut bus = Flat::new(hex);
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_a(2, 0x11);
    core.set_a(3, addr);
    let (addr_sr, control_sr) = if which == 0 {
        (Sr::Dbreaka0, Sr::Dbreakc0)
    } else {
        (Sr::Dbreaka1, Sr::Dbreakc1)
    };
    core.set_special(addr_sr, base);
    core.set_special(control_sr, control);
    let stop = core.run_until(&mut bus, Cycle(10));
    let a3 = core.a(3);
    Watched { stop, bus, a3 }
}

/// A `j` to itself while every interrupt is masked stops the core before it once, and the core
/// spins on after. With an interrupt enabled above `CINTLEVEL` it does not stop, and with one
/// enabled at `CINTLEVEL` it does. A `j` to another address does not stop.
#[test]
fn a_jump_to_itself_with_every_interrupt_masked_stops_once() {
    let mut bus = Flat::new("06ffff");
    let mut core = core_at::<Esp32s3>(CODE);
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Spin);
    assert_eq!((core.pc(), core.cycle()), (CODE, Cycle(0)));
    assert_eq!(core.run_until(&mut bus, Cycle(100)), Stop::Limit);
    assert_eq!(core.pc(), CODE);
    let mut open = core_at::<Esp32s3>(CODE);
    open.set_special(Sr::Ps, 0);
    open.set_special(Sr::IntEnable, 1);
    assert_eq!(open.run_until(&mut bus, Cycle(100)), Stop::Limit);
    let mut level_1 = core_at::<Esp32s3>(CODE);
    level_1.set_special(Sr::Ps, 1);
    level_1.set_special(Sr::IntEnable, 1);
    assert_eq!(level_1.run_until(&mut bus, Cycle(100)), Stop::Spin);
    let mut bus = Flat::new("0600000006ffff");
    let mut forward = core_at::<Esp32s3>(CODE);
    assert_eq!(forward.run_until(&mut bus, Cycle(100)), Stop::Spin);
    assert_eq!(forward.pc(), CODE + 4, "a `j` elsewhere runs");
}
