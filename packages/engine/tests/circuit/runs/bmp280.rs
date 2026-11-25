//! A BMP280 on a 3.3 V ATmega328P over I2C (address 0x76, SDO grounded, CSB pulled up).
//!
//! 1. The stock Adafruit_BMP280 sketch (`bmp280.hex`) calls `begin()` (which enters normal mode)
//!    and then `setSampling(MODE_NORMAL, SAMPLING_X2, SAMPLING_X16, FILTER_X16, STANDBY_MS_500)`.
//!    `setSampling()` writes `config` (0xF5) before `ctrl_meas` (0xF4), while the chip is in
//!    normal mode. Per BST-BMP280-DS001 §4.3.5 the write is ignored and raises `chip.protocol`
//!    rule `config_in_normal` once, so `filter` stays off and standby stays 0.5 ms. Stepping
//!    temperature from 20 °C to 85 °C at 100 ms shows the full 85.00 °C on the next print.
//! 2. `bmp280_filter.hex` writes sleep mode to `ctrl_meas` (0xF4 = 0x00), then `config`
//!    (0xF5 = 0x90: `t_sb = 100` -> 500 ms standby per Table 11, `filter = 100` -> coefficient 16
//!    per Table 6), then normal mode (`0xF4 = 0x57`: `osrs_t = ×2`, `osrs_p = ×16`, Table 13
//!    `t_measure = 37.5 ms`, cycle period `537.5 ms`). No diagnostic is raised, a read during
//!    the 500 ms standby still returns 20.00 °C, and after the 20 °C -> 85 °C step at 100 ms
//!    samples 1..4 lag per §3.3.3 `data_filtered = (old * 15 + adc) / 16`:
//!    24.09 °C, 27.92 °C, 31.51 °C, 34.87 °C (ideal linear 24.06, 27.87, 31.44, 34.79 °C).

use std::collections::BTreeMap;

use orbe_engine::diagnostic::{Diagnostic, ProtocolRule};

use crate::runs::common::repo;

const STEP_AT_PS: u64 = 100_000_000_000;
const STOP_PS: u64 = 2_350_000_000_000;

fn run(hex: &str, stop_ps: u64) -> (orbe_engine::run_folder::RunFolder, Vec<Diagnostic>) {
    let hex = std::fs::read_to_string(repo(hex)).unwrap();
    let mut netlist: serde_json::Value = serde_json::from_str(&circuit(stop_ps)).unwrap();
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "mcu")
        .unwrap()["firmware"] = serde_json::Value::String(hex);
    let bytes = serde_json::to_vec(&netlist).unwrap();
    let parsed = orbe_engine::netlist::parse(&bytes).unwrap();
    let results = orbe_engine::simulate(&parsed);
    let diagnostics = results.diagnostics().to_vec();
    let folder = orbe_engine::run_folder::render(
        &bytes,
        Some(&parsed),
        &results,
        orbe_engine::timestamp::Timestamp::from_unix_seconds(0),
    );
    (folder, diagnostics)
}

#[test]
fn sleep_then_config_then_normal_lags_20_to_85_step_per_table_6_and_11() {
    let (folder, diagnostics) = run(
        "packages/engine/tests/firmware/sketch/bmp280_filter.hex",
        STOP_PS,
    );
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {}",
        serde_json::to_string(&diagnostics).unwrap()
    );
    let serial = serial_text(folder.file("traces/serial.csv").unwrap());
    let text = serial.values().next().unwrap();
    assert_eq!(
        text,
        "s0=20.00\r\nsb=20.00\r\ns1=24.09\r\ns2=27.92\r\ns3=31.51\r\ns4=34.87\r\n"
    );
}

#[test]
fn stock_sketch_warns_config_in_normal_once_and_steps_immediately() {
    let (folder, diagnostics) = run("packages/engine/tests/firmware/sketch/bmp280.hex", STOP_PS);
    assert_eq!(
        diagnostics.len(),
        1,
        "{}",
        serde_json::to_string(&diagnostics).unwrap()
    );
    assert!(matches!(
        &diagnostics[0],
        Diagnostic::ChipProtocol {
            rule: ProtocolRule::ConfigInNormal,
            ..
        }
    ));
    let message = serde_json::to_value(&diagnostics[0]).unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        message.contains("config (0xF5)")
            && message.contains("§4.3.5")
            && message.contains("ctrl_meas 0xF4"),
        "{message}"
    );
    let serial = serial_text(folder.file("traces/serial.csv").unwrap());
    let text = serial.values().next().unwrap();
    assert!(
        text.contains("Temperature = 20.00 *C") && text.contains("Temperature = 85.00 *C"),
        "{text:?}"
    );
}

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

/// 3.3 V ATmega328P + BMP280 on 4.7 kohm I2C pull-ups, SDO grounded (0x76), CSB pulled up.
/// World temperature starts at 20 °C and steps to 85 °C at 100 ms; pressure holds 101325 Pa.
fn circuit(stop_ps: u64) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": 10000000000, "grid_ps": 10000000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "csb", "aref", "xtal1", "xtal2",
        "pb0", "pb1", "pb2", "pb3", "pb4", "pb5",
        "pc0", "pc1", "pc2", "pc3", "pc4", "pc5", "pc6",
        "pd0", "pd1", "pd2", "pd3", "pd4", "pd5", "pd6", "pd7"],
      "world": {{
        "temperature": {{
          "kind": "steps",
          "initial": {{ "value": 20.0, "unit": "degC" }},
          "events": [{{ "t_ps": {STEP_AT_PS}, "level": {{ "value": 85.0, "unit": "degC" }} }}]
        }},
        "pressure": {{ "value": 101325.0, "unit": "Pa" }}
      }},
      "parts": [
        {{ "name": "v3", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 3.3, "unit": "V" }} }},
        {{ "name": "rscl", "kind": "resistor", "pins": {{ "a": "vcc", "b": "pc5" }},
          "resistance": {{ "value": 4700.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
        {{ "name": "rsda", "kind": "resistor", "pins": {{ "a": "vcc", "b": "pc4" }},
          "resistance": {{ "value": 4700.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
        {{ "name": "rcsb", "kind": "resistor", "pins": {{ "a": "vcc", "b": "csb" }},
          "resistance": {{ "value": 10000.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
        {{ "name": "cx1", "kind": "capacitor", "pins": {{ "a": "xtal1", "b": "gnd" }},
          "capacitance": {{ "value": 2.2e-11, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "cx2", "kind": "capacitor", "pins": {{ "a": "xtal2", "b": "gnd" }},
          "capacitance": {{ "value": 2.2e-11, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "cref", "kind": "capacitor", "pins": {{ "a": "aref", "b": "gnd" }},
          "capacitance": {{ "value": 1e-7, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "bmp", "kind": "bmp280",
          "pins": {{ "VDD": "vcc", "VDDIO": "vcc", "GND": "gnd", "CSB": "csb", "SDI": "pc4", "SCK": "pc5", "SDO": "gnd" }},
          "low_resistance": {{ "value": 108.0, "unit": "ohm" }},
          "input_threshold": {{
            "low": {{ "value": 0.2, "unit": "1" }},
            "high": {{ "value": 0.8, "unit": "1" }} }},
          "ratings": {{
            "supply_voltage": {{ "value": 4.25, "unit": "V" }},
            "pin_margin": {{ "value": 0.3, "unit": "V" }} }} }},
        {MCU}
      ]
    }}"#
    )
}

const MCU: &str = r#"{
  "name": "mcu",
  "kind": "atmega328p",
  "pins": {
    "PC6": "pc6", "PD0": "pd0", "PD1": "pd1", "PD2": "pd2", "PD3": "pd3", "PD4": "pd4",
    "VCC": "vcc", "GND_8": "gnd", "PB6": "xtal1", "PB7": "xtal2",
    "PD5": "pd5", "PD6": "pd6", "PD7": "pd7",
    "PB0": "pb0", "PB1": "pb1", "PB2": "pb2", "PB3": "pb3", "PB4": "pb4", "PB5": "pb5",
    "AVCC": "vcc", "AREF": "aref", "GND_22": "gnd",
    "PC0": "pc0", "PC1": "pc1", "PC2": "pc2", "PC3": "pc3", "PC4": "pc4", "PC5": "pc5"
  },
  "firmware": "",
  "clock_hz": { "value": 16000000, "unit": "Hz" },
  "fuses": { "low": 255, "high": 217, "extended": 253 },
  "output": {
    "high_drop": { "value": 0.0, "unit": "V" },
    "high_resistance": { "value": 25.0, "unit": "ohm" },
    "low_drop": { "value": 0.0, "unit": "V" },
    "low_resistance": { "value": 25.0, "unit": "ohm" }
  },
  "pull_up_resistance": { "value": 35714.0, "unit": "ohm" },
  "reset_pull_up_resistance": { "value": 46296.0, "unit": "ohm" },
  "leakage_resistance": { "value": 5500000.0, "unit": "ohm" },
  "input_threshold": {
    "low": { "value": 0.42, "unit": "1" },
    "high": { "value": 0.52, "unit": "1" }
  },
  "reset_threshold": {
    "low": { "value": 0.42, "unit": "1" },
    "high": { "value": 0.44, "unit": "1" }
  }
}"#;
