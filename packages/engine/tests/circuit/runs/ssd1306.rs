//! An SSD1306 behind the module's regulator. The module's VCC source is 0 V until 40 ms, so
//! the regulator output is below the controller's 1.65 V logic minimum. The bus is pulled up to
//! the MCU's 5 V, so a transmission can finish and be refused. After VCC steps to 5 V the
//! regulator is over the minimum and Adafruit SSD1306's `begin` is acknowledged. The same
//! controller on 5 V, over its 4.0 V absolute maximum, still answers and draws. The module with
//! a switch in its VCC wire loses the picture when the switch opens: the lit panel pulls its
//! rail under the logic minimum, and the bus alone holds it up only while the panel sleeps.

use std::collections::BTreeMap;

use crate::runs::common::repo;

/// After the sketch's first `begin`, which talks at about 22 ms, and before its `delay(40)` ends.
const POWER_AT_PS: u64 = 40_000_000_000;
const STOP_PS: u64 = 160_000_000_000;
/// The Block's sketch has its frame whole by about 170 ms.
const HELLO_STOP_PS: u64 = 250_000_000_000;

/// The module's switch opens once the Block's frame is whole.
const OPEN_PS: u64 = 200_000_000_000;
const OPEN_STOP_PS: u64 = 260_000_000_000;

/// Runs `circuit(vdd, stop_ps)` with the image at `hex`, a path from the repo, and returns its
/// run folder. The run has no diagnostics.
fn run(hex: &str, vdd: &str, stop_ps: u64) -> orbe_engine::run_folder::RunFolder {
    let (folder, _) = run_netlist(hex, &circuit(vdd, stop_ps));
    folder
}

/// Runs `netlist` with the image at `hex`, a path from the repo, and returns its run folder and
/// its results. The run has no diagnostics.
fn run_netlist(
    hex: &str,
    netlist: &str,
) -> (
    orbe_engine::run_folder::RunFolder,
    orbe_engine::run_folder::Results,
) {
    let hex = std::fs::read_to_string(repo(hex)).unwrap();
    let mut netlist: serde_json::Value = serde_json::from_str(netlist).unwrap();
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "mcu")
        .unwrap()["firmware"] = serde_json::Value::String(hex);
    let bytes = serde_json::to_vec(&netlist).unwrap();
    let parsed = orbe_engine::netlist::parse(&bytes).unwrap();
    let results = orbe_engine::simulate(&parsed);
    assert_eq!(
        serde_json::to_value(results.diagnostics()).unwrap(),
        serde_json::json!([]),
    );
    let folder = orbe_engine::run_folder::render(
        &bytes,
        Some(&parsed),
        &results,
        orbe_engine::timestamp::Timestamp::from_unix_seconds(0),
    );
    (folder, results)
}

#[test]
fn an_open_vcc_fails_begin_and_power_later_lets_it_succeed() {
    let folder = run(
        "packages/engine/tests/firmware/sketch/ssd1306_power.hex",
        "vdd",
        STOP_PS,
    );
    let serial = serial_text(folder.file("traces/serial.csv").unwrap());
    let text = serial.values().next().unwrap();
    assert_eq!(text, "TX 2\r\nTX 0\r\nBEGIN 1\r\n");
    let observations: serde_json::Value =
        serde_json::from_slice(folder.file("observations.json").unwrap()).unwrap();
    let display = &observations["displays"]["oled"];
    assert_eq!(display["flags"]["display_on"], true);
    assert_eq!(display["charge_pump"], 0x14);
    let frame = folder.file("traces/display-oled.mono-pages").unwrap();
    assert!(
        frame.iter().all(|byte| *byte == 0),
        "the glass was never drawn"
    );
    let acks = address_acks(folder.file("traces/i2c.csv").unwrap());
    assert_eq!(acks.first().copied(), Some(0), "the open rail NACKs 0x3C");
    assert!(
        acks.contains(&1),
        "a later address to 0x3C is acknowledged, {acks:?}"
    );
}

/// `VDD` on the 5 V rail, over Table 11-1's 4.0 V absolute maximum. The controller keeps
/// running: every address is acknowledged and the Block's sketch draws its frame.
#[test]
fn a_vdd_over_four_volts_still_answers_and_draws() {
    let folder = run("blocks/nano-oled/build/hello.ino.hex", "vcc", HELLO_STOP_PS);
    let acks = address_acks(folder.file("traces/i2c.csv").unwrap());
    assert!(!acks.is_empty(), "the sketch addresses 0x3C");
    assert!(acks.iter().all(|ack| *ack == 1), "{acks:?}");
    let observations: serde_json::Value =
        serde_json::from_slice(folder.file("observations.json").unwrap()).unwrap();
    let display = &observations["displays"]["oled"];
    assert_eq!(display["flags"]["display_on"], true);
    assert_eq!(display["charge_pump"], 0x14);
    let frame = folder.file("traces/display-oled.mono-pages").unwrap();
    assert!(frame.iter().any(|byte| *byte != 0), "the frame is drawn");
}

/// The Block's sketch draws its frame on the module, with the panel's draw, and the VCC wire's
/// switch opens at 200 ms. The regulator holds 3.3 V while the switch is closed. Once it
/// opens, the lit panel drains the module's capacitors under the 1.65 V logic minimum within a
/// millisecond, which loses the GDDRAM, and the glass stays dark. The Nano's pull-ups then
/// charge the rail through the module's 4.7 kohm with the controller asleep.
#[test]
fn opening_the_vcc_wire_under_a_lit_panel_drops_its_rail_and_darkens_the_glass() {
    let (folder, results) = run_netlist(
        "blocks/nano-oled/build/hello.ino.hex",
        &module(OPEN_PS, OPEN_STOP_PS),
    );
    let trace = results.trace().unwrap();
    let column = trace
        .columns()
        .iter()
        .position(|column| column.to_string() == "#net.vdd")
        .unwrap();
    let time: Vec<u64> = trace.time().iter().map(|time| time.as_ps()).collect();
    let row = |t_ps: u64| time.iter().position(|&time| time >= t_ps).unwrap();
    let before = trace.column(column)[row(OPEN_PS - 1_000_000_000)];
    assert!(
        (3.2..=3.31).contains(&before),
        "the regulator holds {before} V"
    );
    let low = trace.min_column(column)[row(OPEN_PS)..row(OPEN_PS + 2_000_000_000)]
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    assert!(low < 1.65, "the rail falls to {low} V under the panel");
    let observations: serde_json::Value =
        serde_json::from_slice(folder.file("observations.json").unwrap()).unwrap();
    let display = &observations["displays"]["oled"];
    assert_eq!(display["flags"]["display_on"], false);
    let frame = folder.file("traces/display-oled.mono-pages").unwrap();
    assert!(frame.iter().all(|byte| *byte == 0), "the GDDRAM is lost");
    let end = trace.column(column)[time.len() - 1];
    assert!(
        end > 1.65,
        "the bus holds the sleeping controller up at {end} V"
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

fn address_acks(csv: &[u8]) -> Vec<u8> {
    std::str::from_utf8(csv)
        .unwrap()
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cell: Vec<&str> = line.split(',').collect();
            (cell[2] == "address" && cell[3] == "60").then(|| cell[6].parse().unwrap())
        })
        .collect()
}

/// The module's VCC, its regulator and the panel, on a 5 V ATmega328P, with the panel's `VDD`
/// on `vdd`: `"vdd"` is the regulator output and `"vcc"` the 5 V rail. 4.7 kohm to 5 V hold
/// the bus and `RES` while the regulator output is down, so a transmission finishes with
/// reset released and the controller can refuse it.
fn circuit(vdd: &str, stop_ps: u64) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": 1000000000, "grid_ps": 1000000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "vmod", "vdd", "res", "aref", "xtal1", "xtal2",
        "pb0", "pb1", "pb2", "pb3", "pb4", "pb5",
        "pc0", "pc1", "pc2", "pc3", "pc4", "pc5", "pc6",
        "pd0", "pd1", "pd2", "pd3", "pd4", "pd5", "pd6", "pd7"],
      "parts": [
        {{ "name": "v5", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V" }} }},
        {{ "name": "vmod", "kind": "voltage_source", "pins": {{ "p": "vmod", "n": "gnd" }},
          "voltage": {{ "kind": "steps", "initial": {{ "value": 0.0, "unit": "V" }},
            "events": [{{ "t_ps": {POWER_AT_PS}, "level": {{ "value": 5.0, "unit": "V" }} }}] }} }},
        {{ "name": "u1", "kind": "linear_regulator",
          "pins": {{ "in": "vmod", "out": "vdd", "gnd": "gnd" }},
          "output_voltage": {{ "value": 3.3, "unit": "V" }},
          "line_regulation": {{ "value": 0.00165, "unit": "1" }},
          "output_resistance": {{ "value": 0.6060606060606061, "unit": "ohm" }},
          "dropout_voltage": {{ "value": 0.001, "unit": "V" }},
          "dropout_resistance": {{ "value": 2.5, "unit": "ohm" }},
          "current_limit": {{ "value": 0.2, "unit": "A" }},
          "quiescent_current": {{ "value": 1e-6, "unit": "A" }},
          "ground_current_ratio": {{ "value": 0.0, "unit": "1" }},
          "ratings": {{ "current": null, "power": null }} }},
        {{ "name": "c7", "kind": "capacitor", "pins": {{ "a": "vdd", "b": "gnd" }},
          "capacitance": {{ "value": 1e-6, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "r1", "kind": "resistor", "pins": {{ "a": "vcc", "b": "pc5" }},
          "resistance": {{ "value": 4700.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
        {{ "name": "r2", "kind": "resistor", "pins": {{ "a": "vcc", "b": "pc4" }},
          "resistance": {{ "value": 4700.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
        {{ "name": "r3", "kind": "resistor", "pins": {{ "a": "vcc", "b": "res" }},
          "resistance": {{ "value": 4700.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
        {{ "name": "cx1", "kind": "capacitor", "pins": {{ "a": "xtal1", "b": "gnd" }},
          "capacitance": {{ "value": 2.2e-11, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "cx2", "kind": "capacitor", "pins": {{ "a": "xtal2", "b": "gnd" }},
          "capacitance": {{ "value": 2.2e-11, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "cref", "kind": "capacitor", "pins": {{ "a": "aref", "b": "gnd" }},
          "capacitance": {{ "value": 1e-7, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "oled", "kind": "ssd1306",
          "pins": {{ "VDD": "{vdd}", "VSS": "gnd", "SCL": "pc5", "SDA": "pc4", "RES": "res" }},
          "address": 60,
          "low_drop": {{ "value": 0.0, "unit": "V" }},
          "low_resistance": {{ "value": 133.33333333333334, "unit": "ohm" }},
          "input_threshold": {{
            "low": {{ "value": 0.2, "unit": "1" }},
            "high": {{ "value": 0.8, "unit": "1" }} }} }},
        {MCU}
      ]
    }}"#
    )
}

/// The module on a 5 V ATmega328P: 5 V through a switch that opens at `open_ps` to the module's
/// VCC, C6, the regulator, C7 and C8, the 4.7 kohm pull-ups of SCL and SDA to the 3.3 V rail,
/// R3 and C3 on `RES`, and the panel with the draw of its part file.
fn module(open_ps: u64, stop_ps: u64) -> String {
    let cap = |name: &str, a: &str, farad: f64| {
        format!(
            r#"{{ "name": "{name}", "kind": "capacitor", "pins": {{ "a": "{a}", "b": "gnd" }},
          "capacitance": {{ "value": {farad:e}, "unit": "F" }}, "initial": null }},"#
        )
    };
    let res = |name: &str, a: &str, b: &str| {
        format!(
            r#"{{ "name": "{name}", "kind": "resistor", "pins": {{ "a": "{a}", "b": "{b}" }},
          "resistance": {{ "value": 4700.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},"#
        )
    };
    let parts = [
        cap("c6", "vmod", 1e-6),
        cap("c7", "vdd", 1e-6),
        cap("c8", "vdd", 1e-6),
        cap("c3", "res", 1e-5),
        res("r1", "vdd", "pc5"),
        res("r2", "vdd", "pc4"),
        res("r3", "vdd", "res"),
    ]
    .concat();
    format!(
        r#"{{
      "version": 1,
      "seed": 1,
      "analysis": {{ "transient": {{ "stop_ps": {stop_ps}, "max_step_ps": 1000000000, "grid_ps": 1000000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "vmod", "vdd", "res", "aref", "xtal1", "xtal2",
        "pb0", "pb1", "pb2", "pb3", "pb4", "pb5",
        "pc0", "pc1", "pc2", "pc3", "pc4", "pc5", "pc6",
        "pd0", "pd1", "pd2", "pd3", "pd4", "pd5", "pd6", "pd7"],
      "parts": [
        {{ "name": "v5", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
          "voltage": {{ "value": 5.0, "unit": "V" }} }},
        {{ "name": "sw", "kind": "switch", "pins": {{ "a": "vcc", "b": "vmod" }},
          "on_resistance": {{ "value": 0.1, "unit": "ohm" }},
          "off_resistance": {{ "value": 100000000.0, "unit": "ohm" }},
          "initial": "closed", "events": [{{ "t_ps": {open_ps}, "state": "open" }}], "bounce": null }},
        {REGULATOR}
        {parts}
        {{ "name": "cx1", "kind": "capacitor", "pins": {{ "a": "xtal1", "b": "gnd" }},
          "capacitance": {{ "value": 2.2e-11, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "cx2", "kind": "capacitor", "pins": {{ "a": "xtal2", "b": "gnd" }},
          "capacitance": {{ "value": 2.2e-11, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "cref", "kind": "capacitor", "pins": {{ "a": "aref", "b": "gnd" }},
          "capacitance": {{ "value": 1e-7, "unit": "F" }},
          "initial": {{ "value": 0.0, "unit": "V" }} }},
        {{ "name": "oled", "kind": "ssd1306",
          "pins": {{ "VDD": "vdd", "VSS": "gnd", "SCL": "pc5", "SDA": "pc4", "RES": "res" }},
          "address": 60,
          "low_drop": {{ "value": 0.0, "unit": "V" }},
          "low_resistance": {{ "value": 133.33333333333334, "unit": "ohm" }},
          "input_threshold": {{
            "low": {{ "value": 0.2, "unit": "1" }},
            "high": {{ "value": 0.8, "unit": "1" }} }},
          "draw": {{
            "sleep_current": {{ "value": 1e-6, "unit": "A" }},
            "logic_current": {{ "value": 1.8e-4, "unit": "A" }},
            "half_lit_current": {{ "value": 0.0173, "unit": "A" }},
            "all_lit_current": {{ "value": 0.0231, "unit": "A" }},
            "holds_above": {{ "value": 1.65, "unit": "V" }} }} }},
        {MCU}
      ]
    }}"#
    )
}

/// The module's XC6206, from its part file.
const REGULATOR: &str = r#"{ "name": "u1", "kind": "linear_regulator",
  "pins": { "in": "vmod", "out": "vdd", "gnd": "gnd" },
  "output_voltage": { "value": 3.3, "unit": "V" },
  "line_regulation": { "value": 0.00165, "unit": "1" },
  "output_resistance": { "value": 0.6060606060606061, "unit": "ohm" },
  "dropout_voltage": { "value": 0.001, "unit": "V" },
  "dropout_resistance": { "value": 2.5, "unit": "ohm" },
  "current_limit": { "value": 0.2, "unit": "A" },
  "quiescent_current": { "value": 1e-6, "unit": "A" },
  "ground_current_ratio": { "value": 0.0, "unit": "1" },
  "ratings": { "current": null, "power": null } },"#;

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
