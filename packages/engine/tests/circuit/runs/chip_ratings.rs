//! An `atmega328p`'s `ratings` on the documented example of `docs/engine/netlist.md`: each port
//! pin's current, the sums through `VCC` and `GND_8`, and each pin's voltage window.

use std::fs;

use crate::runs::common::repo;
use orbe_engine::Session;
use orbe_engine::diagnostic::{Diagnostic, Excursion, VoltageExcursion};
use orbe_engine::netlist::{self, Average, Basis};
use orbe_engine::timestamp::Timestamp;
use orbe_model::Time;

/// The documented `leakage_resistance`, to each rail.
const LEAKAGE: f64 = 5.5e6;
/// The documented `output.*_resistance`.
const STAGE: f64 = 25.0;

fn section() -> String {
    let doc = fs::read_to_string(repo("docs/engine/netlist.md")).unwrap();
    let start = doc.find("### ATmega328P").unwrap();
    let end = start + doc[start..].find("\n### SPI replay").unwrap();
    doc[start..end].to_string()
}

/// The section's code block that starts with `start`.
fn block(start: &str) -> String {
    let section = section();
    let from = section.find(&format!("```json\n{start}")).unwrap() + "```json\n".len();
    let to = from + section[from..].find("\n```").unwrap();
    section[from..to].to_string()
}

/// The documented example, its chip rated with the documented `ratings` object.
fn rated() -> serde_json::Value {
    let mut netlist: serde_json::Value = serde_json::from_str(&block("{")).unwrap();
    let ratings: serde_json::Value =
        serde_json::from_str(&format!("{{{}}}", block("\"ratings\""))).unwrap();
    chip(&mut netlist)["ratings"] = ratings["ratings"].clone();
    netlist
}

fn chip(netlist: &mut serde_json::Value) -> &mut serde_json::Value {
    part(netlist, "u1")
}

fn part<'n>(netlist: &'n mut serde_json::Value, name: &str) -> &'n mut serde_json::Value {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == name)
        .unwrap()
}

fn add(netlist: &mut serde_json::Value, nets: &[&str], parts: serde_json::Value) {
    for net in nets {
        netlist["nets"].as_array_mut().unwrap().push((*net).into());
    }
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .extend(parts.as_array().unwrap().iter().cloned());
}

fn source(name: &str, p: &str, n: &str, volts: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "voltage_source", "pins": { "p": p, "n": n },
        "voltage": { "value": volts, "unit": "V" }
    })
}

fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "resistor", "pins": { "a": a, "b": b },
        "resistance": { "value": ohm, "unit": "ohm" },
        "ratings": { "power": null }
    })
}

fn run(netlist: &serde_json::Value) -> Vec<Diagnostic> {
    orbe_engine::run(
        netlist.to_string().as_bytes(),
        Timestamp::from_unix_seconds(0),
    )
    .results
    .diagnostics()
    .to_vec()
}

fn currents(found: &[Diagnostic]) -> Vec<&Excursion> {
    found
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingCurrent(excursion) => Some(excursion.as_ref()),
            _ => None,
        })
        .collect()
}

fn windows(found: &[Diagnostic]) -> Vec<&VoltageExcursion> {
    found
        .iter()
        .filter_map(|d| match d {
            Diagnostic::RatingVoltage(excursion) => Some(excursion),
            _ => None,
        })
        .collect()
}

fn refs(found: &[Diagnostic]) -> Vec<String> {
    found
        .iter()
        .map(|d| format!("{} {}", d.code(), d.refs()[0]))
        .collect()
}

/// The time constant a DC current rating's RMS filter takes, 10.2 ms.
const TAU_PS: u64 = 10_200_000_000;

/// `ldi r16,0x20`, `out DDRB,r16`, `rjmp .-2`: PB5 an output, low, from cycle 2 on.
const PB5_LOW: &str = ":0600000000E204B9FFCF8D\n:00000001FF\n";

/// `netlist` run to `stop_ps` on its own step.
fn lasting(netlist: &mut serde_json::Value, stop_ps: u64) {
    netlist["analysis"]["transient"]["stop_ps"] = stop_ps.into();
}

/// PB5 high into `load` to ground: the stage and the two leakages against the load.
fn high_into(load: f64) -> f64 {
    let up = 1.0 / STAGE + 1.0 / LEAKAGE;
    let down = 1.0 / LEAKAGE + 1.0 / load;
    5.0 * up / (up + down) / load
}

#[test]
fn the_documented_ratings_are_the_part_files() {
    let ratings = block("\"ratings\"");
    for part in ["atmega328p-dip28", "atmega328p-tqfp32"] {
        let file = fs::read_to_string(repo(&format!("parts/{part}/part.jsonc"))).unwrap();
        for line in ratings
            .lines()
            .skip(1)
            .filter(|line| line.contains("\"value\""))
        {
            // The part file also says where each value came from, which the netlist does not.
            let value = line
                .trim()
                .trim_end_matches(',')
                .trim_end_matches('}')
                .trim_end();
            assert!(file.contains(value), "{part} lacks {value}");
        }
    }
}

#[test]
fn a_chip_with_no_ratings_object_is_unrated() {
    let mut netlist = rated();
    chip(&mut netlist)
        .as_object_mut()
        .unwrap()
        .remove("ratings");
    part(&mut netlist, "r1")["resistance"]["value"] = 47.0.into();
    assert_eq!(run(&netlist), []);
}

#[test]
fn the_documented_example_is_inside_every_rating() {
    assert_eq!(run(&rated()), []);
}

/// `toggle.S` drives PB5 high from 250 ns to 6.5 us and from 12.75 us to 19 us of the 20 us run.
/// Into 47 ohm that is 69 mA, the stage's closed form, for 12.5 us in all: well under 40 mA
/// through a 10.2 ms time constant, as a capacitor's charging pulse is.
#[test]
fn an_overload_shorter_than_the_window_stays_inside_the_pin_rating() {
    let mut netlist = rated();
    part(&mut netlist, "r1")["resistance"]["value"] = 47.0.into();
    assert_eq!(run(&netlist), []);
}

/// The RMS filter's reading after `ps` of a steady `amps` from cold.
fn filled(amps: f64, ps: u64) -> f64 {
    amps * (1.0 - libm::exp(-(ps as f64) / TAU_PS as f64)).sqrt()
}

/// Run on to three time constants, `toggle.S` keeps PB5 high half of each 12.5 us. Into
/// 47 ohm that is 69 mA, the stage's closed form, so its RMS settles toward 69 mA / sqrt(2),
/// 48.8 mA, and first rounds above 40 mA after `tau * ln(48.8^2 / (48.8^2 - 40.05^2))`, 11.2 ms.
#[test]
fn a_port_pin_over_its_current_for_its_window_is_named_with_the_stage_s_rms() {
    let mut netlist = rated();
    part(&mut netlist, "r1")["resistance"]["value"] = 47.0.into();
    lasting(&mut netlist, 3 * TAU_PS);
    let found = run(&netlist);
    assert_eq!(refs(&found), ["rating.current #u1.pin.PB5"]);
    let [pin] = currents(&found)[..] else {
        unreachable!()
    };
    assert_eq!(
        (pin.limit, pin.basis),
        (
            0.04,
            Basis::Sustained {
                tau_ps: TAU_PS,
                average: Average::Rms
            }
        )
    );
    let rms = high_into(47.0) / 2f64.sqrt();
    let reading = filled(rms, 3 * TAU_PS);
    assert!(
        (pin.peak / reading - 1.0).abs() < 0.01,
        "{} against {reading}",
        pin.peak
    );
    let rounds_over = 0.04005;
    let first =
        (TAU_PS as f64 * libm::log(rms * rms / (rms * rms - rounds_over * rounds_over))) as u64;
    assert!(
        pin.first.as_ps().abs_diff(first) < 20_000_000,
        "{:?} against {first}",
        pin.first
    );
    let over = pin.over_ps.unwrap();
    assert!(over.abs_diff(3 * TAU_PS - first) < 20_000_000, "{over}");
    assert_eq!(pin.held_ps, None);
}

/// PB5 held low on the supply's net sinks 5 V across 25 ohm, the 200 mA rating of the ground
/// pin, so only PB5 warns: the ground pin carries its rating and the leakages of the other
/// pins, and its filter reads under that. PB5's RMS is the steady current's filtered from cold.
#[test]
fn a_pin_driving_low_on_the_supply_is_over_and_its_ground_at_its_rating_is_not() {
    let mut netlist = rated();
    chip(&mut netlist)["firmware"] = PB5_LOW.into();
    lasting(&mut netlist, 2 * TAU_PS);
    chip(&mut netlist)["pins"]["PB5"] = "vcc".into();
    part(&mut netlist, "r1")["pins"]["a"] = "vcc".into();
    netlist["nets"]
        .as_array_mut()
        .unwrap()
        .retain(|net| net != "pb5");
    let found = run(&netlist);
    assert_eq!(refs(&found), ["rating.current #u1.pin.PB5"]);
    let [pin] = currents(&found)[..] else {
        unreachable!()
    };
    let reading = filled(5.0 / STAGE + 5.0 / LEAKAGE, 2 * TAU_PS);
    assert!(
        (pin.peak / reading - 1.0).abs() < 1e-3,
        "{} against {reading}",
        pin.peak
    );
}

/// `ports.S` drives every port pin high. Six of them into 120 ohm each carry 34.5 mA, inside
/// the pin rating. With PB5's 4.9 mA into the example's 1 kohm, `VCC` carries 212 mA, plus at
/// most 5 V across the leakage of each of the other pins. Over two time constants its filter
/// reads 0.93 of that, 197 mA, so the run lists it over now at its stop.
#[test]
fn six_pins_inside_their_rating_put_vcc_over_its_own() {
    let mut netlist = rated();
    chip(&mut netlist)["firmware"] =
        fs::read_to_string(repo("packages/engine/tests/firmware/ports.hex"))
            .unwrap()
            .into();
    let loads: Vec<_> = (2..8)
        .map(|k| resistor(&format!("rd{k}"), &format!("pd{k}"), "gnd", 120.0))
        .collect();
    add(&mut netlist, &[], serde_json::Value::Array(loads));
    lasting(&mut netlist, 2 * TAU_PS);
    let found = run(&netlist);
    assert_eq!(refs(&found), ["rating.current #u1.pin.VCC"]);
    let [vcc] = currents(&found)[..] else {
        unreachable!()
    };
    let sum = filled(6.0 * high_into(120.0) + high_into(1000.0), 2 * TAU_PS);
    let most = filled(23.0 * 5.0 / LEAKAGE, 2 * TAU_PS);
    assert!(
        vcc.peak > 0.999 * sum && vcc.peak < sum + most,
        "{} against {sum}",
        vcc.peak
    );
    assert!(high_into(120.0) < 0.04);
}

/// 9 V through 1 kohm into PD2, and -9 V into PD3: an off pin holds what the circuit drives,
/// with no clamp, so each is outside -0.5 V to VCC + 0.5 V from the operating point on. The
/// closed form leaves out the solver's gmin, about 9 nV here.
#[test]
fn a_pin_past_either_rail_is_outside_its_window() {
    let mut netlist = rated();
    netlist["analysis"] = serde_json::json!({ "op": {} });
    add(
        &mut netlist,
        &["v9", "vm9"],
        serde_json::json!([
            source("vhi", "v9", "gnd", 9.0),
            resistor("rhi", "v9", "pd2", 1000.0),
            source("vlo", "vm9", "gnd", -9.0),
            resistor("rlo", "vm9", "pd3", 1000.0),
        ]),
    );
    let found = run(&netlist);
    assert_eq!(
        refs(&found),
        ["rating.voltage #u1.pin.PD2", "rating.voltage #u1.pin.PD3"]
    );
    let [high, low] = windows(&found)[..] else {
        unreachable!()
    };
    let held = |volts: f64| (volts / 1000.0 + 5.0 / LEAKAGE) / (1.0 / 1000.0 + 2.0 / LEAKAGE);
    for (window, volts) in [(high, 9.0), (low, -9.0)] {
        assert_eq!((window.low, window.high), (Some(-0.5), Some(5.5)));
        assert!((window.peak - held(volts)).abs() < 1e-7, "{}", window.peak);
        assert_eq!((window.first, window.over_ps), (Time::ZERO, None));
    }
    let message = serde_json::to_value(&found[0]).unwrap()["message"].clone();
    assert_eq!(
        message,
        "#u1.pin.PD2 reaches 9 V over the chip's ground, outside its -0.5 V to 5.5 V rating"
    );
}

/// `RESET` may reach 13 V and `VCC` 6 V. Every other pin's ceiling follows `VCC`, so a 6.5 V
/// supply puts only `VCC` outside.
#[test]
fn reset_and_vcc_take_their_own_ceilings() {
    let at_reset = |volts: f64| {
        let mut netlist = rated();
        netlist["analysis"] = serde_json::json!({ "op": {} });
        add(
            &mut netlist,
            &[],
            serde_json::json!([source("vr", "pc6", "gnd", volts)]),
        );
        refs(&run(&netlist))
    };
    assert_eq!(at_reset(12.0), Vec::<String>::new());
    assert_eq!(at_reset(14.0), ["rating.voltage #u1.pin.PC6"]);

    let mut netlist = rated();
    netlist["analysis"] = serde_json::json!({ "op": {} });
    part(&mut netlist, "v1")["voltage"]["value"] = 6.5.into();
    let found = run(&netlist);
    assert_eq!(refs(&found), ["rating.voltage #u1.pin.VCC"]);
    assert_eq!(windows(&found)[0].high, Some(6.0));
}

/// Between accepted steps a session checks the chip's pins at the probe, its filter running
/// to the probe. PB5 carries 69 mA into 47 ohm half of each 12.5 us, so it is over now while
/// its filter has not crossed 40 mA. That filter, toward 49 mA RMS, crosses at about 11 ms.
#[test]
fn a_session_checks_the_pins_at_its_probe() {
    let mut netlist = rated();
    part(&mut netlist, "r1")["resistance"]["value"] = 47.0.into();
    lasting(&mut netlist, 2 * TAU_PS);
    let mut session =
        Session::open(netlist::parse(netlist.to_string().as_bytes()).unwrap()).unwrap();
    let over_at = |session: &mut Session, ps: u64| {
        session.advance_to(Time::from_ps(ps));
        let probe = session.probe();
        assert_eq!(probe.at(), Time::from_ps(ps));
        let over = session.over_ratings(&probe);
        let now: Vec<bool> = over.iter().map(Diagnostic::over_now).collect();
        (refs(&over), now)
    };
    assert_eq!(
        over_at(&mut session, 5_100_333_333),
        (vec!["rating.current #u1.pin.PB5".to_string()], vec![true])
    );
    assert_eq!(
        over_at(&mut session, 15_300_333_333),
        (vec!["rating.current #u1.pin.PB5".to_string()], vec![false])
    );
}

#[test]
fn esp32s3_pin_and_supply_voltage_windows_warn_past_datasheet_limits() {
    let mut netlist: serde_json::Value = serde_json::from_slice(
        &fs::read(repo(
            "packages/engine/tests/circuits/esp32s3-blink/netlist.json",
        ))
        .unwrap(),
    )
    .unwrap();
    netlist["analysis"] = serde_json::json!({ "op": {} });
    let s3 = part(&mut netlist, "u1");
    s3["ratings"] = serde_json::json!({
        "rail_margin": { "value": 0.3, "unit": "V" },
        "supply_voltage": { "value": 3.6, "unit": "V" },
    });
    s3["pins"]["GPIO4"] = "gpio4".into();
    s3["pins"]["GPIO5"] = "gpio5".into();
    add(
        &mut netlist,
        &["gpio4", "gpio5", "v5", "vm1"],
        serde_json::json!([
            source("vhi", "v5", "gnd", 5.0),
            resistor("rhi", "v5", "gpio4", 1000.0),
            source("vlo", "vm1", "gnd", -0.6),
            resistor("rlo", "vm1", "gpio5", 1000.0),
        ]),
    );
    let mut blobs = orbe_engine::blob::Blobs::new();
    for entry in fs::read_dir(repo("packages/engine/tests/circuits/esp32s3-blink/blobs")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            blobs.add(fs::read(path).unwrap().into());
        }
    }
    let parsed = netlist::parse_with(netlist.to_string().as_bytes(), &blobs).unwrap();
    let found = orbe_engine::simulate(&parsed).diagnostics().to_vec();
    assert_eq!(
        refs(&found),
        [
            "rating.voltage #u1.pin.GPIO4",
            "rating.voltage #u1.pin.GPIO5"
        ]
    );
    let [high, low] = windows(&found)[..] else {
        unreachable!()
    };
    assert_eq!(high.low, Some(-0.3));
    assert!((high.high.unwrap() - 3.6).abs() < 1e-12, "{:?}", high.high);
    assert!((high.peak - 5.0).abs() < 1e-3, "{}", high.peak);
    assert_eq!(low.low, Some(-0.3));
    assert!((low.high.unwrap() - 3.6).abs() < 1e-12, "{:?}", low.high);
    assert!((low.peak + 0.6).abs() < 1e-3, "{}", low.peak);

    // Supply above 3.6 V warns on VCC only when GPIOs sit within VCC + 0.3 V.
    part(&mut netlist, "v1")["voltage"]["value"] = 3.8.into();
    part(&mut netlist, "vhi")["voltage"]["value"] = 3.3.into();
    part(&mut netlist, "vlo")["voltage"]["value"] = 0.0.into();
    let parsed = netlist::parse_with(netlist.to_string().as_bytes(), &blobs).unwrap();
    let found = orbe_engine::simulate(&parsed).diagnostics().to_vec();
    assert_eq!(refs(&found), ["rating.voltage #u1.pin.VCC"]);
    assert_eq!(windows(&found)[0].high, Some(3.6));
}

#[test]
fn ssd1306_hd44780_and_ws2812_pin_and_supply_voltage_windows_warn() {
    let netlist = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "op": {} },
        "ground": "gnd",
        "nets": ["gnd", "v3", "v5", "vm1", "scl", "sda", "res", "din"],
        "parts": [
            source("v3", "v3", "gnd", 3.3),
            source("v5", "v5", "gnd", 5.0),
            source("vm1", "vm1", "gnd", -0.8),
            resistor("rscl", "v5", "scl", 1000.0),
            resistor("rsda", "vm1", "sda", 1000.0),
            resistor("rres", "v3", "res", 1000.0),
            resistor("rdin", "v5", "din", 1000.0),
            {
                "name": "oled",
                "kind": "ssd1306",
                "pins": { "VDD": "v3", "VSS": "gnd", "SCL": "scl", "SDA": "sda", "RES": "res" },
                "address": 60,
                "low_drop": { "value": 0.0, "unit": "V" },
                "low_resistance": { "value": 133.333, "unit": "ohm" },
                "input_threshold": {
                    "low": { "value": 0.2, "unit": "1" },
                    "high": { "value": 0.8, "unit": "1" }
                },
                "ratings": {
                    "rail_margin": { "value": 0.3, "unit": "V" },
                    "supply_voltage": { "value": 4.0, "unit": "V" }
                }
            },
            {
                "name": "lcd",
                "kind": "hd44780_i2c",
                "pins": { "VCC": "v3", "GND": "gnd", "SCL": "scl", "SDA": "sda" },
                "address": 39,
                "low_drop": { "value": 0.0, "unit": "V" },
                "low_resistance": { "value": 133.333, "unit": "ohm" },
                "input_threshold": {
                    "low": { "value": 0.3, "unit": "1" },
                    "high": { "value": 0.7, "unit": "1" }
                },
                "ratings": {
                    "rail_margin": { "value": 0.5, "unit": "V" },
                    "supply_voltage": { "value": 7.0, "unit": "V" }
                }
            },
            {
                "name": "px",
                "kind": "ws2812",
                "pins": { "VCC": "v3", "GND": "gnd", "DIN": "din" },
                "leds": 1,
                "input_threshold": {
                    "low": { "value": 0.3, "unit": "1" },
                    "high": { "value": 0.7, "unit": "1" }
                },
                "ratings": {
                    "rail_margin": { "value": 0.5, "unit": "V" },
                    "supply_voltage": { "value": 5.5, "unit": "V" }
                }
            }
        ]
    });
    let mut found = refs(&run(&netlist));
    found.sort();
    assert_eq!(
        found,
        [
            "rating.voltage #lcd.pin.SCL",
            "rating.voltage #lcd.pin.SDA",
            "rating.voltage #oled.pin.SCL",
            "rating.voltage #oled.pin.SDA",
            "rating.voltage #px.pin.DIN",
        ]
    );
}
