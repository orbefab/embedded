//! packages/engine/tests/circuits/atmega-serial-echo, `packages/engine/tests/firmware/serial_echo.S` on an ATmega328P at 16 MHz.
//! `vrx` draws 8N1 frames through 1 kohm onto PD0. The RX vector echoes each byte
//! on TXD and drives the LED on PB5 from `'1'` and `'0'`.

use orbe_model::Time;

use orbe_engine::netlist::{Baud, Frame};

use crate::runs::common;
use common::atmega::{
    assert_led_follows, chip_matches_fixture, cycle_ps, logic_edges, logic_vcd, run_beside_steps,
    serial_csv,
};
use common::read_vcd;
use orbe_engine::name::Name;

use orbe_engine::timestamp::Timestamp;
use orbe_engine::{Input, Serial, Session, Setting, Status};

/// `serial_echo.S` leaves `U2X0` clear and writes `UBRR0` = 103. Table 20-1: a bit is
/// 16 baud ticks, and a tick is `UBRR0 + 1` cycles (§20.3.1).
const UBRR: u64 = 103;
const HOST_BAUD: u64 = 9600;
const FAST_BAUD: u64 = 115200;

/// Cycles from reset until the `STS` of `UBRR0L` retires. `RJMP` is 2, and each
/// `LDI`/`STS` pair is 3 (DS40002198B, AVRe). Four pairs write `UCSR0A`, `UCSR0C`,
/// `UBRR0H` and `UBRR0L`.
const UBRR_STORE: u64 = 2 + 4 * 3;

/// `SBI DDRB,5` retires here. `LDI` of `UCSR0B`, its `STS`, then the `SBI`.
/// PB5 was already low, so the watch does not record an edge.
const DRIVEN: u64 = UBRR_STORE + 1 + 2 + 2;

/// The RX vector through `STS UDR0` on the long path, `'0'` or `'1'`.
/// Entry is 4 (§7.7.1). Then `RJMP` 2, `LDS` 2, `CPI` 1, the `'1'` or `'0'`
/// arm (6 either way), `LDS` 2, `SBRS` 2 and `STS` 2. `'h'` and `'i'` take
/// the short path and wait for the previous echo's stop bit, not for a tick.
/// They fit the same window, one bit past this count, because a bit at
/// `UBRR0` = 103 is shorter than the host's 9600 bit.
const VECTOR_TO_UDR: u64 = 4 + 2 + 2 + 1 + 6 + 2 + 2 + 2;

/// `"1"` at 1 ms, then `"0hi"` at 5 ms.
const EVENTS: &[(u64, &[u8])] = &[(1_000_000_000, b"1"), (5_000_000_000, b"0hi")];

#[test]
fn the_chip_is_the_tests_fixture_running_serial_echo_hex() {
    chip_matches_fixture(
        "atmega-serial-echo",
        "packages/engine/tests/firmware/serial_echo.hex",
    );
}

/// Sample 1 is the first baud tick at or after the start edge (§20.8.1).
/// The `UBRR0L` store reloads the counter on its cycle (§20.3.1, §20.11.5).
/// A tick on that cycle counts, and later tick `k` is `stored + k * (UBRR + 1)`.
fn sample1(edge: u64) -> u64 {
    let period = UBRR + 1;
    let steps = edge.saturating_sub(UBRR_STORE).div_ceil(period).max(1);
    UBRR_STORE + steps * period
}

/// `RXC0` on the stop bit's last vote. 153 counts 8 data bits and no parity.
/// Nine groups of 16 cover the start bit and the data, and sample 10 of the
/// stop bit adds 9 (§20.8.2, Figure 20-7).
fn rxc_cycle(edge: u64) -> u64 {
    sample1(edge) + 153 * (UBRR + 1)
}

/// Bit boundary `index` of a host frame. The same floor the waveform uses.
fn boundary_at(event_ps: u64, index: u64, baud: u64) -> u64 {
    let ticks = u128::from(index) * 1_000_000_000_000 / u128::from(baud);
    event_ps + u64::try_from(ticks).unwrap()
}

fn boundary_ps(event_ps: u64, index: u64) -> u64 {
    boundary_at(event_ps, index, HOST_BAUD)
}

/// Mark is high. Bit 0 of an 8N1 frame is the start bit and bit 9 is the stop bit.
fn frame_mark(byte: u8, bit: u64) -> bool {
    if bit == 0 {
        return false;
    }
    if bit == 9 {
        return true;
    }
    byte & (1u8 << u32::try_from(bit - 1).unwrap()) != 0
}

/// Falling edges of the 115200 host, in cycles. The line idles at mark.
fn fast_falls(cycle_ps: u64) -> Vec<u64> {
    let mut falls = Vec::new();
    let mut mark = true;
    for &(event_ps, bytes) in EVENTS {
        let n = 10 * bytes.len() as u64;
        for i in 0..=n {
            let next = if i == n {
                true
            } else {
                frame_mark(bytes[(i / 10) as usize], i % 10)
            };
            if mark && !next {
                falls.push(boundary_at(event_ps, i, FAST_BAUD) / cycle_ps);
            }
            mark = next;
        }
    }
    falls
}

/// The 115200 line at `cycle`. A boundary on that cycle has already taken effect.
fn fast_mark(cycle: u64, cycle_ps: u64) -> bool {
    let ps = cycle * cycle_ps;
    let mut mark = true;
    for &(event_ps, bytes) in EVENTS {
        let n = 10 * bytes.len() as u64;
        let end = boundary_at(event_ps, n, FAST_BAUD);
        if ps < event_ps || ps >= end {
            continue;
        }
        let mut index = 0;
        for i in 0..=n {
            if boundary_at(event_ps, i, FAST_BAUD) <= ps {
                index = i;
            } else {
                break;
            }
        }
        mark = if index == n {
            true
        } else {
            frame_mark(bytes[(index / 10) as usize], index % 10)
        };
    }
    mark
}

/// Majority of samples 8, 9 and 10 of bit `bit` (§20.8.2). `origin` is sample 1.
fn vote_mark(origin: u64, bit: u64, cycle_ps: u64) -> bool {
    let period = UBRR + 1;
    let mut ones = 0;
    for k in 0..3 {
        if fast_mark(origin + (bit * 16 + 7 + k) * period, cycle_ps) {
            ones += 1;
        }
    }
    ones >= 2
}

/// The first 115200 edge whose sample 1 and start-bit vote are still low, and
/// the byte the data votes build. A firmware bit is about twelve host bits, so
/// those votes land on the idle mark and the byte is 255.
fn garbage_frame(cycle_ps: u64) -> (u64, u8) {
    let falls = fast_falls(cycle_ps);
    let period = UBRR + 1;
    let mut i = 0;
    while i < falls.len() {
        let edge = falls[i];
        let origin = sample1(edge);
        let rejected_at = if fast_mark(origin, cycle_ps) {
            origin
        } else if vote_mark(origin, 0, cycle_ps) {
            origin + 9 * period
        } else {
            let mut byte = 0u8;
            for bit in 1..=8u8 {
                if vote_mark(origin, u64::from(bit), cycle_ps) {
                    byte |= 1u8 << u32::from(bit - 1);
                }
            }
            return (edge, byte);
        };
        i = falls.partition_point(|fall| *fall <= rejected_at);
    }
    panic!("the 115200 host produced no frame");
}

/// One row of `serial.csv`.
fn sent(csv: &[u8]) -> Vec<(u64, u8)> {
    let text = std::str::from_utf8(csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("t_ps,part,usart,byte"));
    lines
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut cols = line.split(',');
            let at: u64 = cols.next().unwrap().parse().unwrap();
            assert_eq!(cols.next(), Some("u1"));
            assert_eq!(cols.next(), Some("USART0"));
            let byte: u8 = cols.next().unwrap().parse().unwrap();
            assert!(cols.next().is_none(), "{line}");
            (at, byte)
        })
        .collect()
}

#[test]
fn a_run_echoes_each_byte_and_the_led_follows_one_and_zero() {
    let beside = run_beside_steps("atmega-serial-echo");
    let cycle = cycle_ps(&beside.netlist);
    let bit = 16 * (UBRR + 1) * cycle;
    let mut want = Vec::new();
    for &(event_ps, bytes) in EVENTS {
        for (j, &byte) in bytes.iter().enumerate() {
            let edge = boundary_ps(event_ps, 10 * j as u64) / cycle;
            let rxc = rxc_cycle(edge) * cycle;
            want.push((byte, rxc));
        }
    }

    let rows = sent(serial_csv(&beside));
    assert_eq!(rows.len(), want.len());
    for (&(at, byte), &(want_byte, rxc)) in rows.iter().zip(&want) {
        assert_eq!(byte, want_byte);
        let latest = rxc + (VECTOR_TO_UDR * cycle) + bit;
        assert!(
            at > rxc && at <= latest,
            "echo of {byte} at {at} ps, RXC0 at {rxc} ps, window ends at {latest} ps"
        );
    }

    let vcd = logic_vcd(
        &beside,
        &["#u1.logic.PB5", "#u1.logic.PD0", "#u1.logic.PD1"],
    );
    let pb5 = logic_edges(&vcd, "#u1.logic.PB5");
    let (one, zero) = (want[0].1, want[1].1);
    assert_eq!(pb5.len(), 2, "PB5 rises for '1' and falls for '0'");
    assert!(pb5[0].1 && pb5[0].0 > one && pb5[0].0 < rows[0].0);
    assert!(!pb5[1].1 && pb5[1].0 > zero && pb5[1].0 < rows[1].0);
    assert_led_follows(&beside, &pb5, DRIVEN * cycle);
}

/// The typed byte lands when `"0hi"` goes idle, at 8.125 ms. Its echo's stop
/// bit ends near 10.206 ms, so the 12 ms stop accepts the step that logs it.
/// A 10 ms stop does not.
#[test]
fn a_byte_typed_on_the_first_frame_is_the_fifth_echo() {
    let json = std::fs::read(common::repo(
        "packages/engine/tests/circuits/atmega-serial-echo/netlist.json",
    ))
    .unwrap();
    let mut session = Session::record_document(&json).unwrap();
    let frame = Time::from_ps(1_000_000_000);
    let mut at = frame;
    let mut status = Status::Running;
    let mut sent_byte = false;
    while status == Status::Running {
        status = session.advance_to(at);
        if !sent_byte && status == Status::Running {
            session
                .apply(&Input {
                    part: Name::new("vrx").unwrap(),
                    at: session.now(),
                    setting: Setting::Serial(Serial {
                        baud: Baud::new(9600).unwrap(),
                        frame: Frame::EightN1,
                        bytes: vec![0x31],
                    }),
                })
                .unwrap();
            sent_byte = true;
        }
        at = Time::from_ps(at.as_ps() + 1_000_000_000);
    }
    assert!(sent_byte, "the first frame was already stopped");
    let folder = session.finish(Timestamp::from_unix_seconds(0)).unwrap();
    let rows = sent(folder.file("traces/serial.csv").unwrap());
    let bytes: Vec<u8> = rows.iter().map(|(_, byte)| *byte).collect();
    assert_eq!(bytes, vec![49, 48, 104, 105, 49]);
}

/// The same netlist with the host at 115200. The firmware's `UBRR0` stays 103.
/// The receiver accepts one false frame. Its echo is that byte, and PB5 stays low.
#[test]
fn a_host_at_115200_returns_255_and_leaves_pb5_low() {
    let path = common::repo("packages/engine/tests/circuits/atmega-serial-echo/netlist.json");
    let mut netlist: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let events = &mut netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "vrx")
        .unwrap()["voltage"]["events"];
    for event in events.as_array_mut().unwrap() {
        event["baud"] = serde_json::json!(115200);
    }
    let done = orbe_engine::run(
        serde_json::to_vec(&netlist).unwrap().as_slice(),
        Timestamp::from_unix_seconds(0),
    );
    assert!(done.results.diagnostics().is_empty());
    let cycle = cycle_ps(&netlist);
    let (edge, byte) = garbage_frame(cycle);
    assert_eq!(byte, 255);
    let rxc = rxc_cycle(edge) * cycle;
    let bit = 16 * (UBRR + 1) * cycle;
    let rows = sent(done.folder.file("traces/serial.csv").unwrap());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, byte);
    let latest = rxc + (VECTOR_TO_UDR * cycle) + bit;
    assert!(
        rows[0].0 > rxc && rows[0].0 <= latest,
        "echo of {byte} at {} ps, RXC0 at {rxc} ps, window ends at {latest} ps",
        rows[0].0
    );
    let vcd = read_vcd(std::str::from_utf8(done.folder.file("traces/logic.vcd").unwrap()).unwrap());
    let pb5 = logic_edges(&vcd, "#u1.logic.PB5");
    assert!(
        pb5.iter().all(|&(_, high)| !high),
        "PB5 rose on a 115200 host: {pb5:?}"
    );
}
