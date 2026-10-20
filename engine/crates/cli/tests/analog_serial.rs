//! examples/atmega-analog-serial, `firmware/analog_serial.S` on an ATmega328P at 16 MHz.
//! The wiper of a 10 kohm potentiometer is ADC0. The firmware prints that code on TXD.
//! The trace holds the divider's closed form, and `logic.vcd` holds the 8N1 frames.

mod common;

use common::atmega::{
    assert_holds, chip_matches_fixture, cycle_ps, logic_edges, logic_vcd, run_beside_steps,
    wiper_volts,
};

/// `analog_serial.S` leaves `U2X0` clear and writes `UBRR0` = 103. Table 20-1: a bit is
/// 16 baud ticks, and a tick is `UBRR0 + 1` cycles (§20.3.1).
const UBRR: u64 = 103;
const TXCLK_DIV: u64 = 16;
const LINES: usize = 3;

#[test]
fn the_chip_is_the_tests_fixture_running_analog_serial_hex() {
    chip_matches_fixture("atmega-analog-serial", "firmware/analog_serial.hex");
}

#[test]
fn sim_run_prints_the_adc_code_of_the_wiper_on_txd() {
    let beside = run_beside_steps("atmega-analog-serial");
    let (vin, vcc) = wiper_volts(&beside.netlist);
    assert_holds(&beside, "#net.pc0", vin);

    let code = adc_code(vin, vcc);
    let bit_ps = TXCLK_DIV * (UBRR + 1) * cycle_ps(&beside.netlist);
    let vcd = logic_vcd(&beside, &["#u1.logic.PD1"]);
    assert_eq!(vcd["#u1.logic.PD1"]["initial"], "low");
    let frames = decode_txd(&logic_edges(&vcd, "#u1.logic.PD1"), bit_ps);
    let line = format!("{code}\r\n");
    let line_bytes = line.len();
    assert_eq!(
        frames.len(),
        LINES * line_bytes,
        "{} frames, want {LINES} lines of {line_bytes} bytes",
        frames.len()
    );
    let text: String = frames.iter().map(|&(_, byte)| byte as char).collect();
    assert_eq!(text, line.repeat(LINES));
    for (n, &(start, _)) in frames.iter().enumerate() {
        let Some(&(next, _)) = frames.get(n + 1) else {
            break;
        };
        let gap = next - start;
        if (n + 1) % line_bytes == 0 {
            assert!(
                gap > 10 * bit_ps + 1_000_000_000 && gap < 10 * bit_ps + 2_000_000_000,
                "the gap after line {} is {gap} ps",
                n / line_bytes + 1
            );
        } else {
            assert_eq!(gap, 10 * bit_ps, "frame at {start} ps");
        }
    }
}

/// §24.7. `floor(Vin * 1024 / Vref)`.
fn adc_code(vin: f64, vref: f64) -> u16 {
    let code = (vin * 1024.0 / vref).floor();
    assert!(
        (0.0..1024.0).contains(&code),
        "ADC code {code} is outside 0..1024"
    );
    code as u16
}

/// 8N1 frames. Each later fall is a start bit. Data bits are LSB first.
fn decode_txd(edges: &[(u64, bool)], bit_ps: u64) -> Vec<(u64, u8)> {
    let (at, level) = edges[0];
    assert!(level, "TXD rises to idle at {at} ps");
    let mut i = 1;
    let mut high = true;
    let mut frames = Vec::new();
    while i < edges.len() {
        let (start, level) = edges[i];
        assert!(!level && high, "start bit at {start} ps");
        i += 1;
        high = false;
        let mut bits = [false; 9];
        for (k, bit) in bits.iter_mut().enumerate() {
            let boundary = start + (k as u64 + 1) * bit_ps;
            if i < edges.len() && edges[i].0 == boundary {
                high = edges[i].1;
                i += 1;
            } else if i < edges.len() && edges[i].0 < boundary {
                panic!(
                    "edge at {} ps is off the bit grid of the frame at {start} ps",
                    edges[i].0
                );
            }
            *bit = high;
        }
        assert!(bits[8], "stop bit of the frame at {start} ps");
        let mut byte = 0u8;
        for (bit, &level) in bits[..8].iter().enumerate() {
            if level {
                byte |= 1 << bit;
            }
        }
        frames.push((start, byte));
        high = true;
    }
    frames
}
