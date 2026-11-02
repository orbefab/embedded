//! The core keeps the lines it read until the bus's next event, an access the bus says can move
//! them, or the next run. A device register here raises a level-2 line when it is read or
//! written, as a read of I2C0 settles the master and can raise `TRANS_COMPLETE`, and as a store
//! to a peripheral's interrupt enable, with its raw bit already latched, raises a line.

use sim_xtensa::{Bus, Cause, Config, Core, Cycle, Esp32, Exit, Sr, Width};

use crate::core::{CODE, core_at};

const WOE: u32 = 1 << 18;
const LINE2: u32 = 1 << 19;
const RAM: u32 = 0x3ffb_0000;
const DEVICE: u32 = 0x6001_3000;

struct Device {
    code: Vec<u8>,
    /// Whether the line is high. A load or a store of `DEVICE` raises it.
    high: bool,
    polls: u64,
}

impl Device {
    fn new(hex: &str) -> Device {
        let mut code: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        code.resize(code.len().next_multiple_of(4), 0);
        Device {
            code,
            high: false,
            polls: 0,
        }
    }
}

impl Bus for Device {
    fn fetch(&mut self, _now: Cycle, addr: u32) -> Result<u32, Exit> {
        let at = addr.wrapping_sub(CODE) as usize;
        let word = self.code.get(at..at + 4).ok_or(Exit::Stop)?;
        Ok(u32::from_le_bytes(word.try_into().unwrap()))
    }

    fn load(&mut self, _now: Cycle, addr: u32, _width: Width) -> Result<u32, Exit> {
        match addr {
            DEVICE => {
                self.high = true;
                Ok(0)
            }
            RAM => Ok(0),
            _ => Err(Exit::Fault(Cause::LoadProhibited)),
        }
    }

    fn store(&mut self, _now: Cycle, addr: u32, _width: Width, _value: u32) -> Result<(), Exit> {
        if addr == DEVICE {
            self.high = true;
            Ok(())
        } else {
            Err(Exit::Fault(Cause::StoreProhibited))
        }
    }

    fn interrupts(&mut self, _now: Cycle) -> u32 {
        self.polls += 1;
        if self.high { LINE2 } else { 0 }
    }

    fn next_event(&mut self, _now: Cycle) -> Option<Cycle> {
        None
    }

    fn access_moves_lines(&self, addr: u32) -> bool {
        addr != RAM
    }
}

fn says(hex: &str, text: &str) {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let decoded = Esp32::decode(&bytes, CODE).unwrap();
    assert_eq!(Esp32::render(&decoded), text, "{hex}");
}

fn setup(a3: u32) -> Core<Esp32> {
    let mut core = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::IntEnable, LINE2);
    core.set_a(3, a3);
    core
}

/// `l32i a2, a3, 0` and two `nop`s. The load raises the line, and the next step takes it.
#[test]
fn a_load_that_raises_a_line_is_seen_at_the_next_step() {
    says("222300", "l32i\ta2, a3, 0");
    let mut bus = Device::new("222300f02000f02000");
    let mut core = setup(DEVICE);
    core.run_until(&mut bus, Cycle(4));
    assert_eq!(core.special(Sr::Epc2), CODE + 3, "taken after the load");
    assert_eq!(core.pc(), 0x4000_0100 + 0x40 * 2);
    // A check build reads the lines on the steps this one skips.
    #[cfg(not(feature = "poll-skip-check"))]
    assert_eq!(
        bus.polls, 2,
        "the run's first step and the step after the load"
    );
}

/// A load the bus says moves no line keeps the lines, so only the run's first step reads them.
#[test]
fn a_load_that_moves_no_line_keeps_the_lines() {
    let mut bus = Device::new("222300f02000f02000");
    let mut core = setup(RAM);
    core.run_until(&mut bus, Cycle(3));
    assert_eq!(core.pc(), CODE + 9);
    #[cfg(not(feature = "poll-skip-check"))]
    assert_eq!(bus.polls, 1);
}

/// `s32i a2, a3, 0` and two `nop`s. The store raises the line, and the next step takes it.
#[test]
fn a_store_that_raises_a_line_is_seen_at_the_next_step() {
    says("226300", "s32i\ta2, a3, 0");
    let mut bus = Device::new("226300f02000f02000");
    let mut core = setup(DEVICE);
    core.run_until(&mut bus, Cycle(4));
    assert_eq!(core.special(Sr::Epc2), CODE + 3, "taken after the store");
    assert_eq!(core.pc(), 0x4000_0100 + 0x40 * 2);
}

/// Each run reads the lines at its first step, so a change between runs is seen.
#[test]
fn a_change_between_runs_is_seen_at_the_next_run() {
    let mut bus = Device::new(&"f02000".repeat(8));
    let mut core = setup(RAM);
    core.run_until(&mut bus, Cycle(2));
    assert_eq!(core.pc(), CODE + 6);
    bus.high = true;
    core.run_until(&mut bus, Cycle(3));
    assert_eq!(core.special(Sr::Epc2), CODE + 6);
}
