//! examples/atmega-i2c-scan and the `i2c-` fixtures that run firmware: Wire sketches on the
//! ATmega328P's TWI at 16 MHz, on 4.7 kohm pull-ups with no bus capacitance. `i2c.csv` is the
//! master's view, and `logic.vcd` holds the bus levels each chip read.
//!
//! The bit table of `docs/avr.md` gives the lengths. At `TWBR` 72, `T` is 160 cycles, `S` 72
//! and `L` 88. At `TWBR` 12, `T` is 40, `S` 18 and `L` 22.

mod common;

use std::collections::BTreeMap;

use common::atmega::{
    chip_matches_fixture, folder_file, i2c_csv, logic_edges, run_beside_steps, serial_csv,
};
use common::{read_vcd, repo};

const CYCLE_PS: u64 = 62_500;

/// A row of `i2c.csv`: its time, part, phase, and its address, read, byte and ack cells.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    t_ps: u64,
    part: String,
    phase: String,
    cells: [Option<u8>; 4],
}

fn rows(csv: &[u8]) -> Vec<Row> {
    let text = std::str::from_utf8(csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("t_ps,part,phase,address,read,byte,ack"));
    lines
        .map(|line| {
            let cell: Vec<&str> = line.split(',').collect();
            assert_eq!(cell.len(), 7, "{line}");
            let number = |k: usize| (!cell[k].is_empty()).then(|| cell[k].parse().unwrap());
            Row {
                t_ps: cell[0].parse().unwrap(),
                part: cell[1].to_string(),
                phase: cell[2].to_string(),
                cells: [number(3), number(4), number(5), number(6)],
            }
        })
        .collect()
}

/// Each part's text on its USART, from `serial.csv`.
fn serial_text(csv: &[u8]) -> BTreeMap<String, String> {
    let mut text = BTreeMap::new();
    for line in std::str::from_utf8(csv).unwrap().lines().skip(1) {
        let cell: Vec<&str> = line.split(',').collect();
        let byte: u8 = cell[3].parse().unwrap();
        text.entry(cell[1].to_string())
            .or_insert_with(String::new)
            .push(char::from(byte));
    }
    text
}

/// The phases a scan of addresses `from` to 126 writes when nothing answers.
fn nacked_from(from: u8, rows: &[Row]) {
    let phases: Vec<(&str, [Option<u8>; 4])> = rows
        .iter()
        .map(|row| (row.phase.as_str(), row.cells))
        .collect();
    let mut want = Vec::new();
    for address in from..=126u8 {
        want.push(("start", [None; 4]));
        want.push(("address", [Some(address), Some(0), None, Some(0)]));
        want.push(("stop", [None; 4]));
    }
    assert_eq!(phases, want);
}

/// The run folder of the netlist in the repo directory `dir`, rendered in memory.
fn rendered(dir: &str) -> sim_core::run_folder::RunFolder {
    let path = repo(&format!("{dir}/netlist.json"));
    let bytes = std::fs::read(path).unwrap();
    let parsed = sim_core::netlist::parse(&bytes).unwrap();
    let results = sim_circuit::simulate(&parsed);
    assert_eq!(
        serde_json::to_value(results.diagnostics()).unwrap(),
        serde_json::json!([]),
        "{dir}"
    );
    sim_core::run_folder::render(
        &bytes,
        Some(&parsed),
        &results,
        sim_core::timestamp::Timestamp::from_unix_seconds(0),
    )
}

/// The run folder of the fixture `name`, rendered in memory.
fn fixture(name: &str) -> sim_core::run_folder::RunFolder {
    rendered(&format!("crates/cli/tests/fixtures/{name}"))
}

/// `(t_ps, high)` of `part`'s `pin` in a `logic.vcd`.
fn pin(vcd: &[u8], part: &str, pin: &str) -> Vec<(u64, bool)> {
    let vcd = read_vcd(std::str::from_utf8(vcd).unwrap());
    logic_edges(&vcd, &format!("#{part}.logic.{pin}"))
}

/// The SCL rise-to-rise periods inside each address byte, its eight data bits and its ninth.
fn periods_inside_bytes(rows: &[Row], scl: &[(u64, bool)]) -> Vec<u64> {
    let mut periods = Vec::new();
    for row in rows.iter().filter(|row| row.phase == "address") {
        let rises: Vec<u64> = scl
            .iter()
            .filter(|&&(at, high)| high && at >= row.t_ps)
            .map(|&(at, _)| at)
            .take(9)
            .collect();
        periods.extend(rises.windows(2).map(|pair| pair[1] - pair[0]));
    }
    periods
}

/// Every row's time is a bus edge the master read.
fn rows_sit_on_edges(rows: &[Row], scl: &[(u64, bool)], sda: &[(u64, bool)]) {
    for row in rows {
        assert!(
            scl.iter().chain(sda).any(|&(at, _)| at == row.t_ps),
            "{row:?} is on no bus edge"
        );
    }
}

#[test]
fn the_chip_is_the_tests_fixture_running_i2c_scan_hex() {
    chip_matches_fixture("atmega-i2c-scan", "firmware/sketch/i2c_scan.hex");
}

#[test]
fn the_fixtures_run_the_committed_hex() {
    let cases = [
        ("i2c-scan-400k", "u1", "firmware/sketch/i2c_scan_400k.hex"),
        ("i2c-stretch", "u1", "firmware/sketch/i2c_scan.hex"),
        ("i2c-arbitration", "u1", "firmware/sketch/i2c_scan.hex"),
        ("i2c-contend", "u1", "firmware/sketch/i2c_contend.hex"),
        ("i2c-contend", "u2", "firmware/sketch/i2c_contend.hex"),
        ("i2c-cst816-fast", "u1", "firmware/sketch/cst816_fast.hex"),
    ];
    for (name, part, hex) in cases {
        let path = repo(&format!("crates/cli/tests/fixtures/{name}/netlist.json"));
        let netlist: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let chip = netlist["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == part)
            .unwrap_or_else(|| panic!("{name} has no {part}"));
        assert_eq!(
            common::records(chip["firmware"].as_str().unwrap()),
            common::records(&std::fs::read_to_string(repo(hex)).unwrap()),
            "{name} {part}"
        );
    }
}

fn decode(dir: &str) -> std::process::Output {
    std::process::Command::new("uv")
        .args(["run", "scripts/i2c_decode.py"])
        .arg(repo(dir))
        .current_dir(repo("."))
        .output()
        .expect("uv run scripts/i2c_decode.py: uv is not installed")
}

/// An SDA fall during bit 0's high. The same edge is status 0x00 in
/// `a_start_inside_a_byte_is_a_bus_error`.
#[test]
fn a_bit0_sda_change_decodes_as_a_bus_error() {
    let output = decode("crates/cli/tests/decode/i2c-bit0");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("2 of 2 rows match"), "{text}");
}

/// An `arbitration_lost` row at a START's time is rejected. The row has to match a byte.
#[test]
fn an_arbitration_row_must_match_a_byte() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-kind");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("matches a start, not a byte"), "{text}");
}

/// Address 64 or more, so bit 7 is 1. The loss row is that byte's first bus change,
/// before the first SCL rise. A later rise reads SDA low, and the byte is a bus error.
#[test]
fn an_arbitration_loss_on_a_leading_one_matches() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-high");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(text.contains("2 of 2 rows match"), "{text}");
}

/// A bus error whose bits 7 to 0 all read SDA high. A loss needs a low read on one of them.
#[test]
fn a_bus_error_with_no_low_read_rejects_the_arbitration_row() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-nolow");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("no low read on bits 7 to 0 of an address, on bits 7 to 0 of a write data byte, or on the ACK bit of a read data byte"),
        "{text}"
    );
}

/// Address 0x7F read, so the eight bits are 1. The ACK reads 0 and SDA rises in that high.
/// The only low read is the ninth bit, and that is not a loss.
#[test]
fn an_ack_low_does_not_mark_an_arbitration_loss() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-ack");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("no low read on bits 7 to 0 of an address, on bits 7 to 0 of a write data byte, or on the ACK bit of a read data byte"),
        "{text}"
    );
}

/// Address 0x50 read, acked, then a data byte whose first bit reads 0 and whose next high
/// changes SDA. The master released those bits, so the low is not a loss.
#[test]
fn a_read_data_low_does_not_mark_an_arbitration_loss() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-read");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("no low read on bits 7 to 0 of an address, on bits 7 to 0 of a write data byte, or on the ACK bit of a read data byte"),
        "{text}"
    );
}

/// Address 0x50 read, acked, then eight high data bits. The ninth rise reads low and SDA
/// rises in that high. The master sends that ACK bit, so the loss matches.
#[test]
fn a_read_nack_low_marks_an_arbitration_loss() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-nack");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(text.contains("3 of 3 rows match"), "{text}");
}

/// Address 0x50 write, acked, then a data bit that reads low. SDA rises on the next high.
/// The master sends bits 7 to 0 of that write, so the loss matches.
#[test]
fn a_write_data_low_marks_an_arbitration_loss() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-write");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(text.contains("3 of 3 rows match"), "{text}");
}

/// The START before the loss has no row. The decoder names that skipped phase.
#[test]
fn an_arbitration_row_does_not_skip_an_earlier_phase() {
    let output = decode("crates/cli/tests/decode/i2c-arbitration-skip");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("skips start"), "{text}");
}

/// Each in-byte SCL period on 100 pF is `T` plus the rise to 0.52 of VCC
/// through the 4.7 kohm pull-up in parallel with the pin's pull-up, within one cycle.
#[test]
fn a_100_pf_bus_adds_the_rise_to_each_scl_period() {
    let path = repo("examples/atmega-i2c-scan/netlist.json");
    let mut net: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let part = |name: &str| {
        net["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == name)
            .unwrap()
            .clone()
    };
    let r_bus = part("rscl")["resistance"]["value"].as_f64().unwrap();
    let r_pull = part("u1")["pull_up_resistance"]["value"].as_f64().unwrap();
    let high = part("u1")["input_threshold"]["high"]["value"]
        .as_f64()
        .unwrap();
    net["analysis"]["transient"]["stop_ps"] = serde_json::json!(500_000_000);
    let cap = |name: &str, net_name: &str| {
        serde_json::json!({
            "name": name,
            "kind": "capacitor",
            "pins": { "a": net_name, "b": "gnd" },
            "capacitance": { "value": 100e-12, "unit": "F", "source": "user" },
            "initial": { "value": 0.0, "unit": "V", "source": "user" }
        })
    };
    let parts = net["parts"].as_array_mut().unwrap();
    parts.push(cap("csda", "pc4"));
    parts.push(cap("cscl", "pc5"));
    let bytes = serde_json::to_vec(&net).unwrap();
    let parsed = sim_core::netlist::parse(&bytes).unwrap();
    let results = sim_circuit::simulate(&parsed);
    assert_eq!(
        serde_json::to_value(results.diagnostics()).unwrap(),
        serde_json::json!([]),
        "100 pF"
    );
    let folder = sim_core::run_folder::render(
        &bytes,
        Some(&parsed),
        &results,
        sim_core::timestamp::Timestamp::from_unix_seconds(0),
    );
    let got = rows(folder.file("traces/i2c.csv").unwrap());
    let scl = pin(folder.file("traces/logic.vcd").unwrap(), "u1", "PC5");
    let address = got.iter().find(|row| row.phase == "address").unwrap();
    let rises: Vec<u64> = scl
        .iter()
        .filter(|&&(at, high)| high && at >= address.t_ps)
        .map(|&(at, _)| at)
        .take(9)
        .collect();
    assert_eq!(rises.len(), 9, "one address byte");
    let resistance = 1.0 / (1.0 / r_bus + 1.0 / r_pull);
    let rise_ps = resistance * 100e-12 * libm::log(1.0 / (1.0 - high)) * 1e12;
    let want = 160.0 * CYCLE_PS as f64 + rise_ps;
    let cycle = CYCLE_PS as f64;
    for pair in rises.windows(2) {
        let period = (pair[1] - pair[0]) as f64;
        assert!(
            (period - want).abs() <= cycle,
            "period {period} ps, closed form {want} ps"
        );
    }
}

/// Nothing answers, so each of the 126 addresses is START, SLA+W NACKed and STOP, and the
/// sketch prints `no devices`. On a bus with no capacitor, every SCL period inside a byte is
/// `T`, 10 us, its low `L` and its high `S`.
#[test]
fn the_scanner_nacks_every_address_at_100_khz() {
    let beside = run_beside_steps("atmega-i2c-scan");
    let rows = rows(i2c_csv(&beside));
    nacked_from(1, &rows);
    assert_eq!(serial_text(serial_csv(&beside))["u1"], "no devices\r\n");
    let vcd = folder_file(&beside, "traces/logic.vcd");
    let (scl, sda) = (pin(vcd, "u1", "PC5"), pin(vcd, "u1", "PC4"));
    rows_sit_on_edges(&rows, &scl, &sda);
    let periods = periods_inside_bytes(&rows, &scl);
    assert_eq!(periods.len(), 126 * 8);
    assert!(periods.iter().all(|&period| period == 160 * CYCLE_PS));
}

#[test]
fn the_scanner_nacks_every_address_at_400_khz() {
    let folder = fixture("i2c-scan-400k");
    let rows = rows(folder.file("traces/i2c.csv").unwrap());
    nacked_from(1, &rows);
    assert_eq!(
        serial_text(folder.file("traces/serial.csv").unwrap())["u1"],
        "no devices\r\n"
    );
    let vcd = folder.file("traces/logic.vcd").unwrap();
    let (scl, sda) = (pin(vcd, "u1", "PC5"), pin(vcd, "u1", "PC4"));
    rows_sit_on_edges(&rows, &scl, &sda);
    let periods = periods_inside_bytes(&rows, &scl);
    assert_eq!(periods.len(), 126 * 8);
    assert!(periods.iter().all(|&period| period == 40 * CYCLE_PS));
}

/// `sw1` holds SCL low from 240 us to 290 us, across the TWI's release in the third bit of
/// address 1. SCL rises when the switch opens, the high is `S` from that rise, and the byte
/// ends later by the switch's open time less the release, the same run without the switch
/// being the baseline. The scan then goes on as before.
#[test]
fn a_held_scl_stretches_the_bit() {
    const OPEN_PS: u64 = 290_000_000;
    let folder = fixture("i2c-stretch");
    let rows = rows(folder.file("traces/i2c.csv").unwrap());
    nacked_from(1, &rows);
    assert_eq!(
        serial_text(folder.file("traces/serial.csv").unwrap())["u1"],
        "no devices\r\n"
    );
    let scl = pin(folder.file("traces/logic.vcd").unwrap(), "u1", "PC5");
    let base = rendered("examples/atmega-i2c-scan");
    let base_scl = pin(base.file("traces/logic.vcd").unwrap(), "u1", "PC5");
    let fall = base_scl
        .iter()
        .rfind(|&&(at, high)| !high && at < 240_000_000)
        .unwrap()
        .0;
    let release = fall + 88 * CYCLE_PS;
    let rise = scl.iter().find(|&&(at, high)| high && at > fall).unwrap().0;
    assert_eq!(rise, OPEN_PS, "SCL rises when the switch lets go");
    let next = scl.iter().find(|&&(at, _)| at > rise).unwrap();
    assert_eq!(*next, (rise + 72 * CYCLE_PS, false));
    // The ninth SCL fall of address 1 ends it. Fall 0 is the START's.
    let ninth = |edges: &[(u64, bool)]| {
        edges
            .iter()
            .filter(|&&(at, high)| !high && at > rows[1].t_ps)
            .nth(8)
            .unwrap()
            .0
    };
    assert_eq!(ninth(&scl) - ninth(&base_scl), OPEN_PS - release);
}

/// `sw1` pulls SDA low from 285 us, before the TWI releases it for bit 6 of address 1, a 1,
/// and lets go at 297 us while SCL is high, which is a STOP. The TWI reads 0 at that bit's
/// rise, loses arbitration, and releases both lines, so SCL stays high. Wire's
/// `endTransmission` returns 4 and the sketch prints it, then scans on from address 2 after
/// the switch's STOP.
#[test]
fn a_low_sda_on_a_sent_one_loses_arbitration() {
    let folder = fixture("i2c-arbitration");
    let rows = rows(folder.file("traces/i2c.csv").unwrap());
    assert_eq!(
        serial_text(folder.file("traces/serial.csv").unwrap())["u1"],
        "error 4 at 0x1\r\nno devices\r\n"
    );
    assert_eq!(rows[0].phase, "start");
    assert_eq!(rows[1].phase, "arbitration_lost");
    nacked_from(2, &rows[2..]);
    let vcd = folder.file("traces/logic.vcd").unwrap();
    let (scl, sda) = (pin(vcd, "u1", "PC5"), pin(vcd, "u1", "PC4"));
    rows_sit_on_edges(&rows, &scl, &sda);
    // Bit 6's rise is the seventh after the START. Nothing moves SCL after it until the next
    // START, and SDA rises only when the switch opens.
    let rise = scl
        .iter()
        .filter(|&&(at, high)| high && at > rows[0].t_ps)
        .nth(6)
        .unwrap()
        .0;
    let next_scl = scl.iter().find(|&&(at, _)| at > rise).unwrap().0;
    let after: Vec<_> = sda
        .iter()
        .filter(|&&(at, _)| at > rise && at < next_scl)
        .collect();
    assert_eq!(after, [&(297_000_000, true), &(rows[2].t_ps, false)]);
    assert!(
        rows[2].t_ps - 297_000_000 >= 88 * CYCLE_PS,
        "tBUF from the switch's STOP"
    );
}

/// Two chips run `i2c_contend.hex` on one bus. `u1`'s PD2 is grounded, so it sends 0x50, and
/// `u2` sends 0x51. They start together, and `u2` loses at the last address bit, where it sends
/// a 1 and reads `u1`'s 0. `u1` finishes SLA+W, which nothing ACKs, and sends the STOP.
#[test]
fn two_masters_contend_and_the_higher_address_loses() {
    let folder = fixture("i2c-contend");
    let rows = rows(folder.file("traces/i2c.csv").unwrap());
    let of = |part: &str| -> Vec<(String, [Option<u8>; 4])> {
        rows.iter()
            .filter(|row| row.part == part)
            .map(|row| (row.phase.clone(), row.cells))
            .collect()
    };
    assert_eq!(
        of("u1"),
        [
            ("start".into(), [None; 4]),
            ("address".into(), [Some(0x50), Some(0), None, Some(0)]),
            ("stop".into(), [None; 4]),
        ]
    );
    assert_eq!(
        of("u2"),
        [
            ("start".into(), [None; 4]),
            ("arbitration_lost".into(), [None; 4])
        ]
    );
    let text = serial_text(folder.file("traces/serial.csv").unwrap());
    assert_eq!(text["u1"], "0x50 returned 2\r\n");
    assert_eq!(text["u2"], "0x51 returned 4\r\n");
    let vcd = folder.file("traces/logic.vcd").unwrap();
    assert_eq!(pin(vcd, "u1", "PC5"), pin(vcd, "u2", "PC5"));
    assert_eq!(pin(vcd, "u1", "PC4"), pin(vcd, "u2", "PC4"));
    let starts: Vec<u64> = rows
        .iter()
        .filter(|row| row.phase == "start")
        .map(|row| row.t_ps)
        .collect();
    assert_eq!(starts[0], starts[1], "both masters' START is one SDA fall");
}
