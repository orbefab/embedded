//! A SparkFun BOB-12009 style BSS138 module between a 5 V Nano and a 3.3 V bus.
//!
//! `level_shift.hex` is the Nano's Wire at 100 kHz reading a `cst816` on the low side.
//! A second circuit drives one channel from 5 V and another from 3.3 V, push-pull, and
//! checks the translated levels and the rise across the module's 10 kohm pull-ups.

mod common;

use std::fs;

use orbe_engine::diagnostic::Severity;
use orbe_engine::logic::Level;
use orbe_engine::run_folder::Results;
use orbe_engine::timestamp::Timestamp;
use orbe_engine::trace::Trace;
use orbe_scene::Files;

const TOUCH: &str = "1 291 69\r\n1 16 32\r\n";

/// I2C standard mode, 30% to 70%, from UM10204.
const RISE_LIMIT_PS: u64 = 1_000_000;

fn expand(scene: &str) -> serde_json::Value {
    let project = common::project("nano-button");
    let hex = common::repo().join("packages/engine/tests/firmware/sketch/level_shift.hex");
    fs::copy(hex, project.path().join("build/level_shift.hex")).unwrap();
    let path = project.path().join("scene.jsonc");
    fs::write(&path, scene).unwrap();
    let netlist = orbe_scene::read(&Files::disk(), &path).expect("the scene checks");
    serde_json::from_str(&netlist.json).unwrap()
}

fn run(json: &serde_json::Value) -> (Results, String) {
    let bytes = serde_json::to_vec(json).unwrap();
    let parsed = orbe_engine::netlist::parse(&bytes).expect("the netlist parses");
    let results = orbe_engine::simulate(&parsed);
    let folder = orbe_engine::run_folder::render(
        &bytes,
        Some(&parsed),
        &results,
        Timestamp::from_unix_seconds(0),
    );
    let serial = folder
        .file("traces/serial.csv")
        .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
        .unwrap_or_default();
    (results, serial)
}

fn index(trace: &Trace, key: &str) -> usize {
    trace
        .columns()
        .iter()
        .position(|column| column.to_string() == key)
        .unwrap_or_else(|| panic!("no column {key}"))
}

fn at(trace: &Trace, key: &str, t_ps: u64) -> f64 {
    let column = trace.column(index(trace, key));
    let row = trace
        .time()
        .iter()
        .position(|time| time.as_ps() >= t_ps)
        .unwrap_or_else(|| panic!("{key} has no row at {t_ps}"));
    column[row]
}

fn peak(trace: &Trace, key: &str) -> f64 {
    let index = index(trace, key);
    let rows = trace.column(index).iter().copied().fold(0.0, f64::max);
    trace.max_column(index).iter().copied().fold(rows, f64::max)
}

/// Time from the last sample below `low` to the first sample at or above `high` after `from`.
/// A step that jumps the window in one sample counts as that sample's gap.
fn rise_upper_ps(trace: &Trace, key: &str, from: u64, low: f64, high: f64) -> u64 {
    let volts = trace.column(index(trace, key));
    let time: Vec<u64> = trace.time().iter().map(|time| time.as_ps()).collect();
    let start = time.iter().position(|time| *time >= from).unwrap();
    let mut last_below = time[start.saturating_sub(1)];
    for i in start..volts.len() {
        if volts[i] < low {
            last_below = time[i];
        }
        if volts[i] >= high {
            return time[i] - last_below;
        }
    }
    panic!("{key} never reached {high} V after {from}");
}

fn serial_text(csv: &str) -> String {
    csv.lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let byte: u8 = line.split(',').nth(3).unwrap().parse().unwrap();
            char::from(byte)
        })
        .collect()
}

fn scl_period_ps(trace: &Trace) -> u64 {
    let logic = trace.logic().expect("the Nano has logic");
    let signal = logic
        .signals()
        .iter()
        .position(|signal| signal.to_string().ends_with("logic.PC5"))
        .expect("SCL is PC5");
    let rises: Vec<u64> = logic
        .changes()
        .iter()
        .filter(|change| change.signal == signal && change.level == Level::High)
        .map(|change| change.at.as_ps())
        .collect();
    let mut periods: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(periods.len() > 8, "SCL rose {} times", rises.len());
    periods.sort_unstable();
    periods[periods.len() / 2]
}

#[test]
fn a_nano_reads_a_3v3_touch_controller_through_the_module() {
    let mut json = expand(
        r#"{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/level_shift.hex" },
    "shifter": { "type": "bss138-level-shifter" },
    "lv_supply": { "type": "barrel-jack-dc21", "voltage": "3.3" },
    "rint": { "type": "resistor-axial", "resistance": "10k" }
  },
  "wires": [
    ["shifter.GND", "nano.GND"],
    ["lv_supply.sleeve", "nano.GND"],
    ["shifter.HV", "nano.5V"],
    ["shifter.LV", "lv_supply.center"],
    ["shifter.HV1", "nano.A4"],
    ["shifter.HV2", "nano.A5"],
    ["rint.a", "shifter.LV"]
  ],
  "nets": {
    "GND": "nano.GND",
    "lv": "shifter.LV",
    "lv_sda": "shifter.LV1",
    "lv_scl": "shifter.LV2",
    "int": "rint.b"
  }
}"#,
    );
    json["analysis"]["transient"]["stop_ps"] = serde_json::json!(40_000_000_000u64);
    json["analysis"]["transient"]["grid_ps"] = serde_json::json!(1_000_000_000u64);
    json["analysis"]["transient"]["max_step_ps"] = serde_json::json!(1_000_000_000u64);
    json["parts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "touch",
            "kind": "cst816",
            "pins": {
                "VCC": "lv",
                "GND": "GND",
                "SCL": "lv_scl",
                "SDA": "lv_sda",
                "INT": "int",
                "RST": "lv"
            },
            "low_drop": { "value": 0.0, "unit": "V" },
            "low_resistance": { "value": 133.3, "unit": "ohm" },
            "input_threshold": {
                "low": { "value": 0.3, "unit": "1" },
                "high": { "value": 0.7, "unit": "1" }
            },
            "touches": [
                { "t_ps": 1_000_000_000u64, "x": 291, "y": 69, "down": true },
                { "t_ps": 8_000_000_000u64, "x": 16, "y": 32, "down": true }
            ]
        }));
    let (results, serial) = run(&json);
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    assert_eq!(serial_text(&serial), TOUCH, "{serial}");
    let period = scl_period_ps(results.trace().unwrap());
    assert!(
        (9_500_000..=10_500_000).contains(&period),
        "SCL period {period} ps, Wire.setClock(100000) is 10 µs"
    );
    let trace = results.trace().unwrap();
    for net in ["#net.lv_sda", "#net.lv_scl"] {
        let high = peak(trace, net);
        assert!(high <= 3.6, "{net} reached {high} V on the 3.3 V side");
    }
}

#[test]
fn a_push_pull_translates_down_and_up() {
    let mut json = expand(
        r#"{
  "format": 1,
  "ground": "hv_supply.sleeve",
  "parts": {
    "hv_supply": { "type": "barrel-jack-dc21", "voltage": "5" },
    "lv_supply": { "type": "barrel-jack-dc21", "voltage": "3.3" },
    "shifter": { "type": "bss138-level-shifter" }
  },
  "wires": [
    ["lv_supply.sleeve", "hv_supply.sleeve"],
    ["shifter.GND", "hv_supply.sleeve"],
    ["shifter.HV", "hv_supply.center"],
    ["shifter.LV", "lv_supply.center"]
  ],
  "nets": {
    "hv1": "shifter.HV1",
    "lv1": "shifter.LV1",
    "hv2": "shifter.HV2",
    "lv2": "shifter.LV2"
  }
}"#,
    );
    json["analysis"]["transient"]["stop_ps"] = serde_json::json!(40_000_000u64);
    json["analysis"]["transient"]["grid_ps"] = serde_json::json!(20_000u64);
    json["analysis"]["transient"]["max_step_ps"] = serde_json::json!(200_000u64);
    let parts = json["parts"].as_array_mut().unwrap();
    parts.push(steps("down", "hv1", 5_000_000, 5.0));
    parts.push(steps("up", "lv2", 20_000_000, 3.3));
    let (results, _) = run(&json);
    assert_eq!(
        results.count(Severity::Error),
        0,
        "{:?}",
        results.diagnostics()
    );
    let trace = results.trace().unwrap();
    assert!(
        at(trace, "#net.lv1", 4_000_000) < 0.4,
        "a low on HV1 stays low on LV1"
    );
    assert!(
        at(trace, "#net.hv2", 4_000_000) < 0.4,
        "a low on LV2 stays low on HV2"
    );
    let lv_high = at(trace, "#net.lv1", 15_000_000);
    assert!(
        (3.0..=3.5).contains(&lv_high),
        "5 V on HV1 landed at {lv_high} V on LV1"
    );
    let hv_high = at(trace, "#net.hv2", 35_000_000);
    assert!(hv_high > 4.5, "3.3 V on LV2 landed at {hv_high} V on HV2");
    assert!(peak(trace, "#net.lv1") <= 3.6, "LV1 climbed past 3.3 V");
    let down = rise_upper_ps(trace, "#net.lv1", 5_000_000, 0.99, 2.31);
    let up = rise_upper_ps(trace, "#net.hv2", 20_000_000, 1.5, 3.5);
    assert!(down <= RISE_LIMIT_PS, "LV1 rose in {down} ps");
    assert!(up <= RISE_LIMIT_PS, "HV2 rose in {up} ps");
}

fn steps(name: &str, net: &str, at_ps: u64, high: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "GND" },
        "voltage": {
            "kind": "steps",
            "initial": { "value": 0.0, "unit": "V" },
            "events": [{ "t_ps": at_ps, "level": { "value": high, "unit": "V" } }]
        }
    })
}
