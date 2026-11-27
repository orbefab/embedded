//! An OC pin edge has to reach pin-change listeners on the cycle it is driven.
//!
//! DS40002061B §14.3.3: a pin change is the toggling of any pin in the group,
//! including a pin the timer drives as an alternate function (§13, Table 14-3).
//! `OC1B` is `PB2`. The PCINT0 handler toggles `PB5`.
//!
//! `ICR1` is `0x9F`. Mode 12 clears on the next timer clock, clock 160. The
//! prescaler runs from reset, and the `TCCR1B` store retires before cycle 64,
//! so timer clock `n` is cycle `n × 64`. `OCR1B` is 0, so `OC1B` toggles and
//! `OCF1B` rises together as `TCNT1` leaves 0 (Figure 16-10): on clock 1,
//! cycle 64, and on clock 161 after the clear, cycle 10304. `OCF1B` wakes the
//! idle core there. Idle wake entry is 8 cycles (§7.7) and the `TIMER1_COMPB`
//! vector is a 4-cycle `RETI`, after which the core runs one main instruction,
//! the 1-cycle `LDI`. `PCIF0`, raised at 10306 through the model's `PINx`
//! synchronizer (§14.2.4), is served next: entry 4, the vector `JMP` 3, and
//! `SBI` 2, so `10304 + 8 + 4 + 1 + 4 + 3 + 2 = 10326`.

use orbe_microchip::{Cpu, Level, parse_hex};

fn edges(hex: &str, until: u64) -> Vec<(u32, Level, u64)> {
    let mut cpu = Cpu::new();
    cpu.load_flash(&parse_hex(hex).unwrap());
    cpu.run_until(until);
    let mut out = Vec::new();
    while let Some(report) = cpu.take_pin() {
        out.push((report.pin, report.level, report.cycle));
    }
    out
}

/// `EEARL` does not touch a pin. The compare fire still has to hand `OC1B`
/// to pin change in that period. The handler's `SBI` retires at 10326.
#[test]
fn eear_store_lets_pcint_see_oc1b() {
    let got = edges(include_str!("fixtures/oc1b_pcint_eear.hex"), 12_000);
    assert!(
        got.contains(&(5, Level::Low, 10_326)),
        "PCINT0 should toggle L at 10326, got {got:?}"
    );
}

/// `OCF1B` is a Table 24-6 source, and the same fire toggles `OC1B`.
/// Pin change has to see that edge in the same period. A `GPIOR0` store
/// does not flush anything, so the fire itself has to dispatch the pad.
#[test]
fn compare_notice_still_dispatches_the_oc_pin() {
    let got = edges(include_str!("fixtures/oc1b_pcint_gpior.hex"), 12_000);
    let led: Vec<_> = got.iter().filter(|edge| edge.0 == 5).copied().collect();
    assert!(
        led.contains(&(5, Level::Low, 10_326)),
        "PCINT0 should toggle L one entry after the OC edge, got {got:?}"
    );
}
