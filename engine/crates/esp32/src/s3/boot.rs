//! The chip as the board's second-stage bootloader leaves it at the app's entry,
//! `call_start_cpu0`.
//!
//! The boot ROM and the bootloader are not run. Their result is: the app's RAM segments in
//! SRAM and RTC memory, its flash segments mapped by the MMU, and the rest of the state as the
//! board showed it at that point after a software restart, which `boot/state.txt` holds. The
//! MMU entries come from the captured table, and the app's own DROM and IROM entries are mapped
//! over it as the bootloader maps them, so another app image gets its own.

use sim_xtensa::{Core, Esp32s3, Sr};

use super::clock::Source;
use super::flash;
use super::soc::Soc;

/// The captured state: the core registers, the peripheral registers, and words of RAM.
const STATE: &str = include_str!("boot/state.txt");

/// One value of the captured state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Value<'a> {
    /// A core register by its lowercase name, such as `a1` or `windowbase`.
    Core(&'a str, u32),
    Register(u32, u32),
    Word(u32, u32),
}

/// Every value of the captured state, in file order.
pub(crate) fn state() -> impl Iterator<Item = Value<'static>> {
    STATE.lines().flat_map(|line| {
        let mut fields = line.split_whitespace();
        let kind = fields.next().filter(|kind| !kind.starts_with('#'));
        let hex = |text: Option<&str>| {
            u32::from_str_radix(text.expect("a value"), 16).expect("a hex value")
        };
        let values: Vec<Value<'static>> = match kind {
            None => Vec::new(),
            Some("core") => {
                let name = fields.next().expect("a register name");
                vec![Value::Core(name, hex(fields.next()))]
            }
            Some("reg") => {
                let addr = hex(fields.next());
                vec![Value::Register(addr, hex(fields.next()))]
            }
            Some("mem") => {
                let start = hex(fields.next());
                fields
                    .enumerate()
                    .map(|(k, word)| Value::Word(start + 4 * k as u32, hex(Some(word))))
                    .collect()
            }
            Some(other) => panic!("unknown state line kind {other}"),
        };
        values
    })
}

/// The CPU clock the captured `SYSTEM_SYSCLK_CONF_REG` and `SYSTEM_CPU_PER_CONF_REG` select.
pub(crate) fn clock_source(state: &[Value]) -> Option<Source> {
    let register = |at| {
        state.iter().find_map(|value| match *value {
            Value::Register(addr, word) if addr == at => Some(word),
            _ => None,
        })
    };
    Source::of(register(0x600c_0060)?, register(0x600c_0010)?)
}

/// The special register a core slot of the state names. The break registers have no model,
/// and the state holds them at 0.
fn special(name: &str) -> Option<Sr> {
    Some(match name {
        "ccount" => Sr::Ccount,
        "ps" => Sr::Ps,
        "windowbase" => Sr::WindowBase,
        "windowstart" => Sr::WindowStart,
        "sar" => Sr::Sar,
        "lbeg" => Sr::Lbeg,
        "lend" => Sr::Lend,
        "lcount" => Sr::Lcount,
        "br" => Sr::Br,
        "scompare1" => Sr::Scompare1,
        "acclo" => Sr::Acclo,
        "acchi" => Sr::Acchi,
        "m0" => Sr::M0,
        "m1" => Sr::M1,
        "m2" => Sr::M2,
        "m3" => Sr::M3,
        "atomctl" => Sr::AtomCtl,
        "epc1" => Sr::Epc1,
        "epc2" => Sr::Epc2,
        "epc3" => Sr::Epc3,
        "epc4" => Sr::Epc4,
        "epc5" => Sr::Epc5,
        "epc6" => Sr::Epc6,
        "epc7" => Sr::Epc7,
        "depc" => Sr::Depc,
        "eps2" => Sr::Eps2,
        "eps3" => Sr::Eps3,
        "eps4" => Sr::Eps4,
        "eps5" => Sr::Eps5,
        "eps6" => Sr::Eps6,
        "eps7" => Sr::Eps7,
        "excsave1" => Sr::Excsave1,
        "excsave2" => Sr::Excsave2,
        "excsave3" => Sr::Excsave3,
        "excsave4" => Sr::Excsave4,
        "excsave5" => Sr::Excsave5,
        "excsave6" => Sr::Excsave6,
        "excsave7" => Sr::Excsave7,
        "cpenable" => Sr::CpEnable,
        "interrupt" => Sr::Interrupt,
        "intenable" => Sr::IntEnable,
        "vecbase" => Sr::Vecbase,
        "exccause" => Sr::ExcCause,
        "debugcause" => Sr::DebugCause,
        "icount" => Sr::Icount,
        "icountlevel" => Sr::IcountLevel,
        "excvaddr" => Sr::ExcVaddr,
        "ccompare0" => Sr::Ccompare0,
        "ccompare1" => Sr::Ccompare1,
        "ccompare2" => Sr::Ccompare2,
        "misc0" => Sr::Misc0,
        "misc1" => Sr::Misc1,
        "misc2" => Sr::Misc2,
        "misc3" => Sr::Misc3,
        "threadptr" => Sr::Threadptr,
        _ => return None,
    })
}

/// Applies the state to the core and the SoC. `WindowBase` goes first, so a0 to a15 land in
/// its window, and `INTERRUPT` last, since writing a `CCOMPAREn` clears its pending bit.
///
/// `WindowStart` keeps only the current frame. The frames below it are the bootloader's, whose
/// registers outside the window the state lacks, and the app's entry never returns to them.
pub(crate) fn apply(state: &[Value], core: &mut Core<Esp32s3>, soc: &mut Soc) {
    let windowbase = state.iter().find_map(|value| match *value {
        Value::Core("windowbase", word) => Some(word),
        _ => None,
    });
    let windowbase = windowbase.expect("WINDOWBASE in the state");
    core.set_special(Sr::WindowBase, windowbase);
    let mut interrupt = 0;
    for value in state {
        match *value {
            Value::Core("interrupt", word) => interrupt = word,
            Value::Core("windowstart", word) => {
                assert_ne!(word & 1 << windowbase, 0, "the current frame is live");
                core.set_special(Sr::WindowStart, 1 << windowbase);
            }
            Value::Core(name, word) => {
                if let Some(n) = name.strip_prefix('a').and_then(|n| n.parse().ok()) {
                    core.set_a(n, word);
                } else if let Some(sr) = special(name) {
                    core.set_special(sr, word);
                } else {
                    assert_eq!(word, 0, "{name} has no model and must be 0");
                }
            }
            Value::Register(addr, word) => soc.restore(addr, word),
            Value::Word(addr, word) => soc.poke(addr, &word.to_le_bytes()),
        }
    }
    core.set_special(Sr::Interrupt, interrupt);
}

/// A segment of the app image: `len` bytes at bus address `addr`, from flash offset `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub addr: u32,
    pub offset: u32,
    pub len: u32,
}

impl Segment {
    /// Whether the cache maps it from flash rather than the bootloader copying it to RAM.
    pub fn mapped(&self) -> bool {
        flash::IROM.contains(&self.addr) || flash::DROM.contains(&self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_names_the_entry_point_and_parses_whole() {
        let values: Vec<Value> = state().collect();
        let core = values
            .iter()
            .filter(|v| matches!(v, Value::Core(..)))
            .count();
        let registers = values
            .iter()
            .filter(|v| matches!(v, Value::Register(..)))
            .count();
        let words = values
            .iter()
            .filter(|v| matches!(v, Value::Word(..)))
            .count();
        assert_eq!((core, registers, words), (77, 612, 7234));
        assert!(values.contains(&Value::Core("a1", 0x3fce_b340)));
        assert!(values.contains(&Value::Register(0x600c_0060, 0x000a_8400)));
    }
}
