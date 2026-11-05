//! What decides whether the APP CPU runs: `SYSTEM_CORE_1_CONTROL_0_REG` and `_1_REG`, and the
//! APP CPU's stall and reset fields in `RTC_CNTL`, and the steer a store to them gives the run.
//!
//! The APP CPU runs while `CLKGATE_EN` is set, `RESETING` and `RUNSTALL` are clear, and the RTC
//! stall is off. The RTC stall is on while `RTC_CNTL_SW_STALL_APPCPU_C0` is 2 and
//! `RTC_CNTL_SW_STALL_APPCPU_C1` is 0x21. A clear of `RESETING` while it was set, or a write of
//! `RTC_CNTL_SW_APPCPU_RST`, resets it. `CONTROL_1` is a word the ROM reads as the boot address.
//! IDF v5.5.5, `components/soc/esp32s3/register/soc/system_reg.h` and `rtc_cntl_reg.h`, and
//! `components/hal/esp32s3/include/hal/cpu_utility_ll.h`.

use sim_xtensa::Cycle;

/// `SYSTEM_CORE_1_CONTROL_0_REG` and `_1_REG`, in the SYSTEM block.
pub(crate) const CONTROL_0: u32 = 0x0;
pub(crate) const CONTROL_1: u32 = 0x4;
const RUNSTALL: u32 = 1 << 0;
const CLKGATE_EN: u32 = 1 << 1;
const RESETING: u32 = 1 << 2;
/// `RTC_CNTL_SW_CPU_STALL_REG`, in the RTC_CNTL block.
pub(crate) const SW_CPU_STALL: u32 = 0xbc;
/// `RTC_CNTL_SW_APPCPU_RST`, a bit of `RTC_CNTL_OPTIONS0_REG` that reads 0.
pub(crate) const APPCPU_RST: u32 = 1 << 4;

/// Whether a core's RTC stall is on: its `C0` field, at bit `c0` of `RTC_CNTL_OPTIONS0_REG`, is
/// 2, and its `C1` field, at bit `c1` of `RTC_CNTL_SW_CPU_STALL_REG`, is 0x21.
fn stalled(options0: u32, sw_cpu_stall: u32, c0: u32, c1: u32) -> bool {
    (options0 >> c0) & 0x3 == 2 && (sw_cpu_stall >> c1) & 0x3f == 0x21
}

/// Whether `RTC_CNTL_SW_STALL_PROCPU_C0` and `_C1` stall the PRO CPU.
pub(crate) fn pro_stalled(options0: u32, sw_cpu_stall: u32) -> bool {
    stalled(options0, sw_cpu_stall, 2, 26)
}

/// The two SYSTEM words, at their power-on values until the boot path restores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AppControl {
    pub(crate) control: u32,
    pub(crate) boot: u32,
}

impl Default for AppControl {
    fn default() -> AppControl {
        AppControl {
            control: RESETING,
            boot: 0,
        }
    }
}

/// What holds the APP CPU at one moment: `CONTROL_0` and whether the RTC stall is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hold {
    control: u32,
    stalled: bool,
}

impl Hold {
    /// From `CONTROL_0`, `RTC_CNTL_OPTIONS0_REG` and `RTC_CNTL_SW_CPU_STALL_REG`.
    pub(crate) fn new(control: u32, options0: u32, sw_cpu_stall: u32) -> Hold {
        Hold {
            control,
            stalled: stalled(options0, sw_cpu_stall, 0, 20),
        }
    }

    /// Whether the APP CPU runs.
    pub(crate) fn runs(self) -> bool {
        self.clocked() && self.control & (RESETING | RUNSTALL) == 0 && !self.stalled
    }

    /// Whether its clock runs, so `CCOUNT` counts.
    fn clocked(self) -> bool {
        self.control & CLKGATE_EN != 0
    }

    /// Whether `RESETING` is set.
    pub(crate) fn resetting(self) -> bool {
        self.control & RESETING != 0
    }
}

/// A store that changed what a core may do, at the start of the storing step. The storing core
/// returns before its next step, and the scheduler applies it before any core runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Steer {
    /// Reset the APP CPU.
    Reset(Cycle),
    /// Whether the APP CPU runs or its clock runs changed. `clocked` is whether its clock ran
    /// up to `at`.
    Run { at: Cycle, clocked: bool },
    /// A store outside SRAM and RTC memory while the other core is halted or parked.
    Wake(Cycle),
}

impl Steer {
    /// What a store at `at` that took the hold from `before` to `after` does, with `reset` a
    /// write of `RTC_CNTL_SW_APPCPU_RST`.
    pub(crate) fn of_store(before: Hold, after: Hold, reset: bool, at: Cycle) -> Option<Steer> {
        let released = before.control & RESETING != 0 && after.control & RESETING == 0;
        if reset || released {
            Some(Steer::Reset(at))
        } else if before.runs() != after.runs() || before.clocked() != after.clocked() {
            Some(Steer::Run {
                at,
                clocked: before.clocked(),
            })
        } else {
            None
        }
    }

    /// How strong the steer is, so a step keeps the strongest of its steers.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Steer::Reset(_) => 2,
            Steer::Run { .. } => 1,
            Steer::Wake(_) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_cpu_runs_with_its_clock_on_and_nothing_holding_it() {
        let runs = |control, options0, stall| Hold::new(control, options0, stall).runs();
        assert!(runs(CLKGATE_EN, 0, 0));
        assert!(!runs(CLKGATE_EN | RUNSTALL, 0, 0));
        assert!(!runs(CLKGATE_EN | RESETING, 0, 0));
        assert!(!runs(0, 0, 0));
        assert!(!runs(CLKGATE_EN, 2, 0x21 << 20), "the RTC stall");
        assert!(runs(CLKGATE_EN, 2, 0x20 << 20));
        assert!(runs(CLKGATE_EN, 1, 0x21 << 20));
        assert!(
            runs(CLKGATE_EN, 2 << 2, 0x21 << 26),
            "the PRO CPU's stall fields"
        );
    }

    #[test]
    fn the_pro_cpu_stalls_on_its_own_fields() {
        assert!(pro_stalled(2 << 2, 0x21 << 26));
        assert!(!pro_stalled(2, 0x21 << 20), "the APP CPU's stall fields");
        assert!(!pro_stalled(2 << 2, 0x20 << 26));
        assert!(!pro_stalled(1 << 2, 0x21 << 26));
    }

    #[test]
    fn a_release_resets_and_a_gate_edge_steers_even_while_stalled() {
        let at = Cycle(7);
        let hold = |control| Hold::new(control, 0, 0);
        let steer = |before, after| Steer::of_store(hold(before), hold(after), false, at);
        assert_eq!(
            steer(CLKGATE_EN | RESETING, CLKGATE_EN),
            Some(Steer::Reset(at))
        );
        assert_eq!(
            steer(RESETING, 0),
            Some(Steer::Reset(at)),
            "a release while gated"
        );
        assert_eq!(
            steer(CLKGATE_EN | RUNSTALL, CLKGATE_EN),
            Some(Steer::Run { at, clocked: true })
        );
        assert_eq!(
            steer(RUNSTALL, CLKGATE_EN | RUNSTALL),
            Some(Steer::Run { at, clocked: false }),
            "a gate edge under RUNSTALL"
        );
        assert_eq!(steer(RUNSTALL, RUNSTALL), None);
        assert_eq!(steer(CLKGATE_EN, CLKGATE_EN), None);
        assert_eq!(
            Steer::of_store(hold(0), hold(0), true, at),
            Some(Steer::Reset(at))
        );
    }
}
