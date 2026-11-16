//! `packages/engine/tests/circuits/atmega-dht22`: a Nano reads a `dht22` on D2 every 2 s and
//! prints the read's number, the temperature and the humidity. The world steps from 23.4 C and
//! 61.7 %RH to -10.1 C and 40 %RH at 3 s. A read sends what the sensor measured as the read
//! before it ended, so the step shows on the third read. AREF sits on 5V, so no net floats.

use crate::runs::common;
use common::atmega::logic_edges;
use common::{read_vcd, records, repo};

const NETLIST: &str = "packages/engine/tests/circuits/atmega-dht22/netlist.json";

const TEXT: &str = "n=1 t=23.4 h=61.7\r\nn=2 t=23.4 h=61.7\r\nn=3 t=-10.1 h=40.0\r\n";

/// Each read's five bytes: 617 and 234 tenths twice, then 400 and 101 tenths with the sign bit.
const FRAMES: [[u8; 5]; 3] = [
    [0x02, 0x69, 0x00, 0xEA, 0x55],
    [0x02, 0x69, 0x00, 0xEA, 0x55],
    [0x01, 0x90, 0x80, 0x65, 0x76],
];

const US: u64 = 1_000_000;

#[test]
fn the_nano_runs_dht22_hex() {
    let netlist: serde_json::Value =
        serde_json::from_slice(&std::fs::read(repo(NETLIST)).unwrap()).unwrap();
    let mcu = netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == "nano__mcu")
        .unwrap();
    assert_eq!(
        records(mcu["firmware"].as_str().unwrap()),
        records(
            &std::fs::read_to_string(repo("packages/engine/tests/firmware/sketch/dht22.hex"))
                .unwrap()
        )
    );
}

/// The sketch prints the world a read before, and the line the sensor sees carries each frame:
/// a host low of at least 0.8 ms, the 80 us response, 40 bits whose high is a 1 over 48 us,
/// and the last low.
#[test]
fn the_sketch_prints_the_world_the_read_before_measured() {
    let bytes = std::fs::read(repo(NETLIST)).unwrap();
    let parsed = orbe_engine::netlist::parse(&bytes).unwrap();
    let results = orbe_engine::simulate(&parsed);
    assert!(
        results.diagnostics().is_empty(),
        "{}",
        serde_json::to_string(results.diagnostics()).unwrap()
    );
    let folder = orbe_engine::run_folder::render(
        &bytes,
        Some(&parsed),
        &results,
        orbe_engine::timestamp::Timestamp::from_unix_seconds(0),
    );
    assert_eq!(serial_text(folder.file("traces/serial.csv").unwrap()), TEXT);

    let vcd = read_vcd(std::str::from_utf8(folder.file("traces/logic.vcd").unwrap()).unwrap());
    let edges = logic_edges(&vcd, "#dht__sensor.logic.DATA");
    let reads: Vec<&[(u64, bool)]> = edges.chunks(2 + 84).collect();
    assert_eq!(reads.len(), 3, "{edges:?}");
    for (read, frame) in reads.iter().zip(FRAMES) {
        assert_eq!(decode(read), frame);
    }
}

/// The frame of one read's edges: the host's fall and release, then the sensor's 84 edges.
fn decode(read: &[(u64, bool)]) -> [u8; 5] {
    assert_eq!(read.len(), 86, "{read:?}");
    assert!(read.iter().step_by(2).all(|&(_, high)| !high), "{read:?}");
    assert!(
        read.iter().skip(1).step_by(2).all(|&(_, high)| high),
        "{read:?}"
    );
    let spans: Vec<u64> = read.windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(spans[0] >= 800 * US, "the host's low {}", spans[0]);
    let mut bytes = [0u8; 5];
    for bit in 0..40 {
        let high = spans[5 + 2 * bit];
        bytes[bit / 8] |= u8::from(high > 48 * US) << (7 - bit % 8);
    }
    bytes
}

fn serial_text(csv: &[u8]) -> String {
    let mut text = String::new();
    for line in std::str::from_utf8(csv).unwrap().lines().skip(1) {
        let byte: u8 = line.split(',').nth(3).unwrap().parse().unwrap();
        text.push(char::from(byte));
    }
    text
}
