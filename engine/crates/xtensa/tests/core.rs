//! The core on a flat test bus: code words from a list, RAM in a `PagedRam`, and a stop at the
//! first fetch past the code.

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

    fn store(&mut self, _now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        if self.ram.contains(addr) {
            self.ram.write(addr, width, value);
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
    // add.s f1, f2, f3
    let mut bus = Flat::new("30120a");
    let mut core = core_at::<C>(CODE);
    let stop = core.run_until(&mut bus, Cycle(100));
    let want = Unimplemented {
        pc: CODE,
        missing: Missing::Op(Op::ADD_S),
    };
    assert_eq!(stop, Stop::Unimplemented(want), "{}", C::NAME);
    assert_eq!(core.pc(), CODE);
    assert_eq!(core.cycle(), Cycle(0));
    assert_eq!(want.to_string(), "`add.s` at 0x40370000 is not modeled");
}

#[test]
fn unmodeled_op_stops_before_it_on_both_chips() {
    unmodeled_op_stops_before_it::<Esp32>();
    unmodeled_op_stops_before_it::<Esp32s3>();
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
    assert_eq!(size_of::<Core<Esp32>>(), 504);
    assert_eq!(size_of::<Core<Esp32s3>>(), 504);
}
