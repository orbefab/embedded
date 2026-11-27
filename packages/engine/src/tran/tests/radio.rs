//! An ESP32-S3 in a netlist with a network is built with its radio. The stock Arduino station
//! sketch then gets through `esp_wifi_init` and `esp_wifi_start`, which `WiFi.mode(WIFI_STA)`
//! runs, with no diagnostic. Without the network the same app stops at the radio, as before.

use std::sync::Arc;

use orbe_mcu::Setting;

use super::active::run_active;
use crate::blob::Blobs;
use crate::diagnostic::Diagnostic;
use crate::netlist::Netlist;

const MS: u64 = 1_000_000_000;

/// The DevKitC-1 module's supply and the stock station app, run for 100 ms, with `network`
/// after the parts.
fn station(network: &str) -> Netlist {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rom = std::fs::read(root.join("parts/esp32s3-qfn56/esp32s3_rev0_rom.elf")).unwrap();
    let fixture = root.join("packages/scene/tests/fixtures/esp32s3-stock-station");
    let partitions = std::fs::read(fixture.join("partitions.bin")).unwrap();
    let app = std::fs::read(fixture.join("app.bin")).unwrap();

    let mut blobs = Blobs::new();
    let rom_bytes = rom.len();
    let rom_sha = blobs.add(Arc::from(rom));
    let part_bytes = partitions.len();
    let part_sha = blobs.add(Arc::from(partitions));
    let app_bytes = app.len();
    let app_sha = blobs.add(Arc::from(app));

    let ohm = |value: f64| format!(r#"{{ "value": {value:?}, "unit": "ohm" }}"#);
    let json = format!(
        r#"{{
          "version": 1,
          "seed": 0,
          "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {MS}, "grid_ps": {MS} }} }},
          "ground": "gnd",
          "nets": ["gnd", "vdd"],
          "parts": [
            {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vdd", "n": "gnd" }},
              "voltage": {{ "value": 3.3, "unit": "V" }} }},
            {{ "name": "u1", "kind": "esp32s3",
              "pins": {{ "VCC": "vdd", "GND": "gnd" }},
              "rom": {{ "sha256": "{rom_sha}", "bytes": {rom_bytes} }},
              "firmware": {{
                "flash": [
                  {{ "offset": 32768, "sha256": "{part_sha}", "bytes": {part_bytes} }},
                  {{ "offset": 65536, "sha256": "{app_sha}", "bytes": {app_bytes} }}
                ]
              }},
              "output": {{ "high_drop": {{ "value": 0.0, "unit": "V" }}, "high_resistance": {r},
                          "low_drop": {{ "value": 0.0, "unit": "V" }}, "low_resistance": {r} }},
              "pull_up_resistance": {pull},
              "leakage_resistance": {leak},
              "input_threshold": {{ "low": {{ "value": 0.25, "unit": "1" }},
                                   "high": {{ "value": 0.75, "unit": "1" }} }} }}
          ]{network}
        }}"#,
        stop = 100 * MS,
        r = ohm(17.0),
        pull = ohm(45e3),
        leak = ohm(66e6),
    );
    crate::netlist::parse_with(json.as_bytes(), &blobs).unwrap()
}

/// What the chip sent on UART0.
fn uart0(netlist: &Netlist) -> (String, Vec<Diagnostic>) {
    let (trace, diagnostics) = run_active(netlist, Vec::new());
    let files = trace.files();
    let serial = files
        .iter()
        .find(|(name, _)| *name == "serial.csv")
        .map(|(_, bytes)| String::from_utf8(bytes.clone()).unwrap())
        .expect("serial.csv");
    let text = serial
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split(',').collect();
            (cols.get(2) == Some(&"UART0")).then(|| char::from(cols[3].parse::<u8>().unwrap()))
        })
        .collect();
    (text, diagnostics)
}

#[test]
fn with_the_network_the_stock_station_starts_its_radio_with_no_diagnostic() {
    let network = r#", "network": { "ssid": "orbe", "bssid": "02:6f:72:62:65:01", "channel": 6,
        "passphrase": "orbe-password", "epoch_s": 1773480360 }"#;
    let (text, diagnostics) = uart0(&station(network));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    // `WiFi.mode(WIFI_STA)` returns true only when `esp_wifi_init` and `esp_wifi_start` do.
    assert!(text.contains("ready\r\nmode 1\n"), "UART0 output:\n{text}");
}

#[test]
fn without_the_network_the_stock_station_stops_at_the_radio() {
    let (text, diagnostics) = uart0(&station(""));
    assert!(text.contains("ready"), "UART0 output:\n{text}");
    assert!(!text.contains("mode"), "UART0 output:\n{text}");
    assert!(
        matches!(
            diagnostics.as_slice(),
            [Diagnostic::ChipUnsupportedSetting {
                peripheral: "RADIO",
                setting: Setting::Unsimulated(_),
                ..
            }]
        ),
        "{diagnostics:?}"
    );
}
