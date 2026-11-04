//! The core keeps each instruction it decodes, by its PC and the bytes the fetch read. Each test
//! changes what a cached PC holds, or what runs around it, and checks that the core runs what
//! memory holds now.

use std::ops::Range;

use sim_xtensa::{Bus, Cause, Config, Core, Cycle, Esp32, Esp32s3, Exit, Sr, Stop, Width};

use crate::core::{CODE, core_at};

const WOE: u32 = 1 << 18;
const LIMIT: Cycle = Cycle(10_000);

/// Code and data in one array from `CODE`, which the core fetches, loads and stores. `alias`
/// maps a second window onto the same bytes, the way the data-bus window aliases the
/// instruction-bus window. `banks` swaps which array the fetch reads, as a remap of the MMU
/// would.
struct Memory {
    banks: Vec<Vec<u8>>,
    bank: usize,
    alias: Option<u32>,
    fetch_fault: Option<u32>,
}

impl Memory {
    fn new(hex: &str) -> Memory {
        Memory {
            banks: vec![bytes(hex)],
            bank: 0,
            alias: None,
            fetch_fault: None,
        }
    }

    fn offset(&self, addr: u32, len: u32) -> Option<Range<usize>> {
        let base = match self.alias {
            Some(alias) if addr >= alias => alias,
            _ => CODE,
        };
        let at = addr.checked_sub(base)? as usize;
        let end = at + len as usize;
        (end <= self.banks[self.bank].len()).then_some(at..end)
    }

    fn poke(&mut self, addr: u32, hex: &str) {
        let at = (addr - CODE) as usize;
        let new = bytes(hex);
        self.banks[self.bank][at..at + new.len()].copy_from_slice(&new);
    }
}

fn bytes(hex: &str) -> Vec<u8> {
    let mut out: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    out.resize(out.len().next_multiple_of(4), 0);
    out
}

impl Bus for Memory {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        if self.fetch_fault == Some(addr) {
            return Err(Exit::Fault(Cause::InstructionFetchError));
        }
        let range = self.offset(addr, 4).ok_or(Exit::Stop)?;
        let word = &self.banks[self.bank][range];
        Ok(u32::from_le_bytes(word.try_into().unwrap()))
    }

    fn load(&mut self, _now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        let range = self
            .offset(addr, width.bytes())
            .ok_or(Exit::Fault(Cause::LoadProhibited))?;
        let mut word = [0u8; 4];
        word[..range.len()].copy_from_slice(&self.banks[self.bank][range]);
        Ok(u32::from_le_bytes(word))
    }

    fn store(&mut self, _now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        let range = self
            .offset(addr, width.bytes())
            .ok_or(Exit::Fault(Cause::StoreProhibited))?;
        let len = range.len();
        self.banks[self.bank][range].copy_from_slice(&value.to_le_bytes()[..len]);
        Ok(())
    }

    fn interrupts(&mut self, _now: Cycle) -> u32 {
        0
    }

    fn next_event(&mut self, _now: Cycle) -> Option<Cycle> {
        None
    }
}

fn says(hex: &str, pc: u32, text: &str) {
    let decoded = Esp32s3::decode(&bytes(hex)[..hex.len() / 2], pc).unwrap();
    assert_eq!(Esp32s3::render(&decoded), text, "{hex}");
}

fn again(core: &mut Core<Esp32s3>, bus: &mut Memory) -> Stop {
    core.set_pc(CODE);
    core.run_until(bus, LIMIT)
}

/// `addi.n a3, a3, 1`, then `s32i.n a5, a6, 0` over the first word. The second pass runs the
/// `addi.n a3, a3, 2` that the core's own store wrote there.
#[test]
fn a_store_over_a_cached_op_runs_the_new_op() {
    says("1b33", CODE, "addi.n\ta3, a3, 1");
    says("5906", CODE, "s32i.n\ta5, a6, 0");
    let mut bus = Memory::new("1b335906");
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_a(5, u32::from_le_bytes([0x2b, 0x33, 0x59, 0x06]));
    core.set_a(6, CODE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.a(3), 1);
    assert_eq!(again(&mut core, &mut bus), Stop::Bus);
    assert_eq!(core.a(3), 3, "the stored addi.n a3, a3, 2 ran");
}

/// The same store, aimed at the data-bus alias of the instruction window. The fetch address
/// and the store address differ, and they name one byte.
#[test]
fn a_store_through_the_dram_alias_reaches_the_fetch() {
    says("1b33", CODE, "addi.n\ta3, a3, 1");
    says("2b33", CODE, "addi.n\ta3, a3, 2");
    let alias = CODE + 0x200;
    let mut bus = Memory::new("1b335906");
    bus.alias = Some(alias);
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_a(5, u32::from_le_bytes([0x2b, 0x33, 0x59, 0x06]));
    core.set_a(6, alias);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.a(3), 1);
    assert_eq!(again(&mut core, &mut bus), Stop::Bus);
    assert_eq!(
        core.a(3),
        3,
        "the store through the alias replaced the cached addi.n"
    );
}

/// The other core stores over an op this core has cached. Each core keeps its own table, and
/// shares the memory. The PRO CPU's `addi.n` is followed by a jump out of memory, and the APP
/// CPU's store at `CODE + 0x108` runs through the alias.
#[test]
fn a_store_by_the_other_core_reaches_this_one() {
    says("861e00", CODE + 2, "j\t0x40370080");
    let mut bus = Memory::new("1b33861e00");
    bus.alias = Some(CODE + 0x100);
    bus.banks[0].extend(bytes("5906"));
    let mut pro = core_at::<Esp32s3>(CODE);
    assert_eq!(pro.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!((pro.a(3), pro.pc()), (1, CODE + 0x80));
    let mut app = core_at::<Esp32s3>(CODE + 0x108);
    app.set_a(5, u32::from_le_bytes([0x2b, 0x33, 0x86, 0x1e]));
    app.set_a(6, CODE);
    assert_eq!(app.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(again(&mut pro, &mut bus), Stop::Bus);
    assert_eq!((pro.a(3), pro.pc()), (3, CODE + 0x80));
}

/// The fetch reads other bytes at the same PC, as after an MMU remap, here a three-byte op
/// where a two-byte one was.
#[test]
fn a_remap_under_a_cached_pc_runs_what_the_fetch_reads() {
    says("32c305", CODE, "addi\ta3, a3, 5");
    let mut bus = Memory::new("1b330000");
    bus.banks.push(bytes("32c30500"));
    let mut core = core_at::<Esp32s3>(CODE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.a(3), 1);
    bus.bank = 1;
    assert_eq!(again(&mut core, &mut bus), Stop::Bus);
    assert_eq!(core.a(3), 6);
}

/// An op that runs into the next word is keyed by the bytes of both words.
#[test]
fn a_change_in_the_second_word_of_an_op_misses() {
    says("f02000", CODE, "nop");
    let mut bus = Memory::new("3df032c305f02000");
    let mut core = core_at::<Esp32s3>(CODE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.a(3), 5);
    bus.poke(CODE + 4, "07");
    core.set_a(3, 0);
    assert_eq!(again(&mut core, &mut bus), Stop::Bus);
    assert_eq!(core.a(3), 7);
}

/// The same bytes at two PCs decode to two targets, since a PC-relative operand is the target
/// address.
#[test]
fn the_same_jump_at_two_pcs_goes_to_two_targets() {
    says("060000", CODE, "j\t0x40370004");
    let mut bus = Memory::new("060000");
    bus.alias = Some(CODE + 0x100);
    let mut core = core_at::<Esp32s3>(CODE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.pc(), CODE + 4);
    core.set_pc(CODE + 0x100);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.pc(), CODE + 0x104);
}

/// A cached op whose bytes become `ill` raises the exception on the next pass.
#[test]
fn a_cached_op_overwritten_with_ill_raises_it() {
    let mut bus = Memory::new("1b330000");
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_special(Sr::Ps, WOE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    bus.poke(CODE, "000000");
    core.set_pc(CODE);
    core.run_until(&mut bus, Cycle(core.cycle().0 + 1));
    assert_eq!(core.special(Sr::ExcCause), Cause::IllegalInstruction as u32);
    assert_eq!(core.special(Sr::Epc1), CODE);
}

/// `000100` is not an instruction. Both fetches raise, and a table that kept the word would run it.
#[test]
fn an_undefined_word_stays_illegal() {
    let mut bus = Memory::new("000100");
    let mut core = core_at::<Esp32s3>(CODE);
    for _ in 0..2 {
        core.set_special(Sr::Ps, WOE);
        core.set_special(Sr::ExcCause, 5);
        core.set_pc(CODE);
        core.run_until(&mut bus, Cycle(core.cycle().0 + 1));
        assert_eq!(core.special(Sr::ExcCause), Cause::IllegalInstruction as u32);
        assert_eq!(core.special(Sr::Epc1), CODE);
    }
}

/// `LBEG`, `LEND` and `LCOUNT` are not part of a decode, so a cached loop body runs each
/// count the loop op sets.
#[test]
fn a_cached_loop_body_runs_each_new_count() {
    let mut bus = Memory::new("7682011b333df0");
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_special(Sr::Ps, WOE);
    for count in [5, 2, 7] {
        core.set_a(2, count);
        core.set_a(3, 0);
        assert_eq!(again(&mut core, &mut bus), Stop::Bus);
        assert_eq!((core.a(3), core.special(Sr::Lcount)), (count, 0));
    }
}

/// The window check reads `WindowStart` when the op runs, so a cached op that reaches `a8`
/// spills once a frame above is live.
#[test]
fn a_cached_op_still_checks_the_window() {
    says("82c801", CODE, "addi\ta8, a8, 1");
    let mut bus = Memory::new("82c801");
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_special(Sr::Ps, WOE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.a(8), 1);
    core.set_special(Sr::WindowStart, 0b11);
    core.set_pc(CODE);
    core.run_until(&mut bus, Cycle(core.cycle().0 + 1));
    assert_eq!(core.special(Sr::Epc1), CODE, "the overflow was taken");
    assert_eq!(core.special(Sr::WindowBase), 1, "at the spilled frame");
}

/// A clone starts with an empty table. Equality of two cores does not read the table, so this
/// checks the bytes each copy runs after a store.
#[test]
fn a_clone_runs_what_memory_holds_now() {
    let mut bus = Memory::new("1b330000");
    let mut core = core_at::<Esp32s3>(CODE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    let mut copy = core.clone();
    bus.poke(CODE, "2b33");
    assert_eq!(again(&mut copy, &mut bus), Stop::Bus);
    assert_eq!(copy.a(3), 3);
    bus.poke(CODE, "3b33");
    assert_eq!(again(&mut core, &mut bus), Stop::Bus);
    assert_eq!(core.a(3), 4);
}

/// A fetch that faults at a PC the core has already decoded raises that fault and does not
/// run the kept instruction. `PS.EXCM` is clear, so the fault is recorded in `EPC1`.
#[test]
fn a_fetch_fault_at_a_cached_pc_does_not_run_it() {
    says("1b33", CODE, "addi.n\ta3, a3, 1");
    let mut bus = Memory::new("1b330000");
    let mut core = core_at::<Esp32s3>(CODE);
    core.set_special(Sr::Ps, WOE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.a(3), 1);
    bus.fetch_fault = Some(CODE);
    core.set_a(3, 0);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::ExcCause, 5);
    core.set_pc(CODE);
    assert_eq!(core.run_until(&mut bus, LIMIT), Stop::Bus);
    assert_eq!(core.a(3), 0, "the kept addi.n did not run");
    assert_eq!(
        core.special(Sr::ExcCause),
        Cause::InstructionFetchError as u32
    );
    assert_eq!(core.special(Sr::Epc1), CODE);
    assert_eq!(core.special(Sr::ExcVaddr), CODE);
}

/// One `u32` is the fetched-byte key. A length of 0 is the empty slot, and a length above 4
/// would drop the rest of an instruction out of that key.
#[test]
fn every_instruction_length_is_from_one_to_four() {
    for first in 0..=255u8 {
        if let Some(len) = Esp32::insn_len(first) {
            assert!(
                (1..=4).contains(&len),
                "esp32 first byte {first:#04x} names {len} bytes"
            );
        }
        if let Some(len) = Esp32s3::insn_len(first) {
            assert!(
                (1..=4).contains(&len),
                "esp32s3 first byte {first:#04x} names {len} bytes"
            );
        }
    }
}
