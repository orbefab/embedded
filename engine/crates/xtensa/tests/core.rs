//! The core on a flat test bus: code words from a list, RAM in a `PagedRam`, and a stop at the
//! first fetch past the code.

use std::ops::Range;

use sim_xtensa::{
    Bus, Cause, Config, Core, Cycle, Esp32, Esp32s3, Exit, Missing, Op, PagedRam, Sr, Stop,
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

/// `Core` is copied at every landing, so its size is pinned.
#[test]
fn core_size_is_pinned() {
    assert_eq!(size_of::<Core<Esp32>>(), 848);
    assert_eq!(size_of::<Core<Esp32s3>>(), 848);
}
