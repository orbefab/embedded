//! Whole-chip tests for the pulse counter (`PCNT`, `0x6001_7000`): GPIO matrix routing,
//! quadrature counting, glitch filtering, limit interrupts waking `WAITI`, split-invariance,
//! and refused settings.

use super::super::soc::PCNT;
use super::*;

const INTERRUPT_CORE0: u32 = 0x600c_2000;
const PCNT_INTR_SOURCE: u32 = 41;
const PCNT_CLK_EN: u32 = 0xf9c1_e06f | (1 << 10);
const PCNT_RST: u32 = 1 << 10;

/// GPIO matrix input signal indices for unit 0: ch0 sig=33, ch1 sig=34, ch0 ctrl=35, ch1 ctrl=36.
const SIG_CH0_IN0: u32 = 33;
const SIG_CH1_IN0: u32 = 34;
const CTRL_CH0_IN0: u32 = 35;
const CTRL_CH1_IN0: u32 = 36;

/// `GPIO_FUNC_IN_SEL_CFG_REG` offset for signal `sig`: `0x154 + 4 * sig`.
fn in_sel(sig: u32) -> u32 {
    GPIO + 0x154 + 4 * sig
}

/// `IO_MUX` register address for pad `gpio`: `0x6000_9000 + 4 + 4 * gpio`.
fn io_mux(pad: u32) -> u32 {
    IO_MUX + 4 + 4 * pad
}

/// Sets up PCNT unit 0 on GPIO4 (A) and GPIO5 (B) for x4 quadrature decoding with high limit
/// `h_lim`, low limit `l_lim` and glitch filter `filter_cycles` APB cycles, enables unit 0's
/// interrupt on CPU interrupt 1, then sleeps in `WAITI`.
fn pcnt_waiti_program(h_lim: i16, l_lim: i16, filter_cycles: u32) -> Program {
    let filter = if filter_cycles > 0 {
        (filter_cycles & 0x3ff) | (1 << 10)
    } else {
        0
    };
    // Ch0: neg=inc(1), pos=dec(2), hctrl=keep(0), lctrl=inv(1)
    // Ch1: neg=dec(2), pos=inc(1), hctrl=keep(0), lctrl=inv(1)
    let conf0 = filter
        | (1 << 12) // THR_H_LIM_EN
        | (1 << 13) // THR_L_LIM_EN
        | (1 << 16)
        | (2 << 18)
        | (1 << 22)
        | (2 << 24)
        | (1 << 26)
        | (1 << 30);
    let conf2 = u32::from(h_lim as u16) | (u32::from(l_lim as u16) << 16);
    let pairs = [
        (TIMG0 + 0x48, 0),
        (RTC_CNTL + 0x98, 0),
        (SYSTEM + 0x18, PCNT_CLK_EN),
        (SYSTEM + 0x20, PCNT_RST),
        (SYSTEM + 0x20, 0),
        (io_mux(4), (1 << 12) | (1 << 9)),
        (io_mux(5), (1 << 12) | (1 << 9)),
        (in_sel(SIG_CH0_IN0), (1 << 7) | 4),
        (in_sel(CTRL_CH0_IN0), (1 << 7) | 5),
        (in_sel(SIG_CH1_IN0), (1 << 7) | 5),
        (in_sel(CTRL_CH1_IN0), (1 << 7) | 4),
        (PCNT, conf0),
        (PCNT + 0x08, conf2),
        (PCNT + 0x4c, 0xf),
        (PCNT + 0x48, 1),
        (INTERRUPT_CORE0 + 4 * PCNT_INTR_SOURCE, 1),
        // Clear unit 0 reset and pause, leaving units 1..3 in reset.
        (PCNT + 0x60, 0x54),
    ];
    let literals: Vec<u32> = pairs.iter().flat_map(|&(a, v)| [a, v]).collect();
    let mut program = Program::new(&literals);
    for k in 0..pairs.len() {
        program.l32r(2, 2 * k).l32r(3, 2 * k + 1).rri8(6, 3, 2, 0);
    }
    program
        .movi(7, 2) // bit 1 = CPU interrupt 1
        .wsr(7, INTENABLE)
        .movi(7, 0)
        .wsr(7, PS)
        .raw(&WAITI)
        .raw(&LOOP);
    program
}

/// Quadrature inputs on GPIO4 and GPIO5 count up and down, filter out short contact bounce,
/// and fire the high-limit interrupt after the glitch filter delay, waking `WAITI` at the
/// exact same tick regardless of how `run_until` splits the run.
#[test]
fn quadrature_counting_glitch_filter_and_waiti_wakeup_are_split_invariant() {
    // 10 APB cycles at 40 MHz crystal = 10 crystal cycles.
    let filter_cycles = 10u32;
    let program = pcnt_waiti_program(4, -4, filter_cycles);

    let drive = |step: u64| {
        let mut chip = program.chip_on(&[4, 5]);
        let schedule: &[(u64, &[(u8, bool)])] = &[
            (0, &[(4, true), (5, true)]),
            // After setup finishes (~60 crystal cycles), inject a 5-cycle bounce on A (ignored).
            (100, &[(4, false)]),
            (105, &[(4, true)]),
            // One full forward quadrature cycle (4 counts -> hits h_lim = 4 on the 4th edge!).
            (150, &[(4, false)]),
            (200, &[(5, false)]),
            (250, &[(4, true)]),
            (300, &[(5, true)]),
        ];
        let mut idx = 0;
        let mut limit = 0u64;
        let mut halts = Vec::new();
        while limit < 400 {
            let next_input = schedule.get(idx).map(|&(t, _)| t).unwrap_or(u64::MAX);
            let next = (limit + step).min(400).min(next_input);
            while let Some(halt) = chip.run_until(c(next)) {
                halts.push(halt);
            }
            limit = next;
            while idx < schedule.len() && schedule[idx].0 == limit {
                chip.input(c(limit), schedule[idx].1.iter().copied());
                idx += 1;
            }
        }
        (halts, chip)
    };

    let (whole, mut chip) = drive(400);
    // The 4th edge at cycle 300 commits at cycle 310 (filter = 10), raising PCNT interrupt 1,
    // which wakes WAITI and jumps to the unmapped level-1 vector at 0x4000_0300.
    assert!(
        matches!(
            whole[..],
            [Halt::Fault(at, Fault::UnmappedFetch(pc))]
                if pc == pro(0x4000_0300) && at >= c(310) && at <= c(320)
        ),
        "{whole:?}"
    );
    // Counter reset to 0 on h_lim, and H_LIM_LAT (bit 5) + INT_RAW (bit 0) are set.
    assert_eq!(word(&mut chip, PCNT + 0x30), 0);
    assert_eq!(word(&mut chip, PCNT + 0x50) & (1 << 5), 1 << 5);
    assert_eq!(word(&mut chip, PCNT + 0x40) & 1, 1);

    for step in [1, 3, 7, 17] {
        let (split, _) = drive(step);
        assert_eq!(split, whole, "step {step}");
    }
}

/// Unmodeled PCNT settings (gated clock, held in reset, negative h_lim, positive l_lim,
/// reserved CTRL bits, reserved INT_ENA bits, clock off while filter timer is pending) end
/// the run at the store without writing.
#[test]
fn pcnt_refusals_end_the_run_at_their_store() {
    // Accessing PCNT while SYSTEM_PCNT_CLK_EN is clear.
    let program = stores(&[(PCNT + 0x30, 0)]);
    let mut chip = program.chip();
    assert_eq!(
        chip.run_until(c(100)),
        Some(Halt::Fault(
            c(2),
            Fault::Refused(
                Refused::Register("PCNT", "clock_off"),
                store_pc(&program, 0)
            )
        ))
    );

    // Accessing PCNT while SYSTEM_PCNT_RST is set.
    let program = stores(&[
        (SYSTEM + 0x18, PCNT_CLK_EN),
        (SYSTEM + 0x20, PCNT_RST),
        (PCNT + 0x30, 0),
    ]);
    let mut chip = program.chip();
    assert_eq!(
        chip.run_until(c(100)),
        Some(Halt::Fault(
            c(8),
            Fault::Refused(
                Refused::Register("PCNT", "held_in_reset"),
                store_pc(&program, 2)
            )
        ))
    );

    // Invalid h_lim, l_lim, ctrl, and INT_ENA bits.
    for (offset, value, expected) in [
        (0x08, 0xffff, Refused::Register("PCNT", "h_lim")),
        (0x08, 1 << 16, Refused::Register("PCNT", "l_lim")),
        (0x60, 1 << 17, Refused::Register("PCNT", "ctrl")),
        (0x48, 1 << 4, Refused::Interrupt("PCNT")),
    ] {
        let program = stores(&[(SYSTEM + 0x18, PCNT_CLK_EN), (PCNT + offset, value)]);
        let mut chip = program.chip();
        assert_eq!(
            chip.run_until(c(100)),
            Some(Halt::Fault(
                c(5),
                Fault::Refused(expected, store_pc(&program, 1))
            )),
            "{offset:#x} = {value:#x}"
        );
    }
}
