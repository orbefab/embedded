//! End-to-end `orbe` CLI tests for `rating.supply_low`: normal power cycles vs sags,
//! clock-dependent ATmega328P minimum, ATmega328P reset current at t=0, SK6812 3.7 V minimum,
//! and composite part `accepted_warnings`.

mod live;
#[path = "registry_env.rs"]
mod registry_env;

use std::fs;
use std::path::Path;

use live::{common, orbe};
use serde_json::{Value, json};

const BENCH_PART: &str = r#"{
  "format": 4,
  "part": "bench",
  "version": "0.0.1",
  "title": "Ideal bench supply",
  "summary": "Ideal voltage source with a scene-set voltage.",
  "tags": ["test"],
  "license": "MIT OR Apache-2.0",
  "docs": {},
  "real": { "generic": true, "maker": null, "number": null, "datasheet": "test" },
  "pins": { "pos": [1], "neg": [2] },
  "params": {
    "voltage": { "unit": "V", "required": true, "example": 5, "sets": "/voltage" }
  },
  "model": { "kind": "voltage_source", "pins": { "p": "pos", "n": "neg" } }
}"#;

const PLUG_PART: &str = r#"{
  "format": 1,
  "part": "plug",
  "version": "0.0.1",
  "title": "Plug contact, starts closed",
  "summary": "A contact that starts closed and opens on release.",
  "tags": ["test"],
  "license": "MIT OR Apache-2.0",
  "docs": {},
  "real": { "generic": true, "maker": null, "number": null, "datasheet": "test" },
  "pins": { "a": [1], "b": [2] },
  "controls": { "press": { "input": "contact" } },
  "model": {
    "kind": "switch",
    "on_resistance": { "value": 0.05, "unit": "ohm", "source": { "closed_form": { "note": "Closed contact." } } },
    "off_resistance": { "value": 1e8, "unit": "ohm", "source": { "closed_form": { "note": "Open contact." } } },
    "initial": "closed",
    "events": [],
    "bounce": null
  }
}"#;

fn ps(ms: u64, us: u64) -> u64 {
    ms * 1_000_000_000 + us * 1_000_000
}

fn volts(value: f64) -> Value {
    json!({ "value": value, "unit": "V", "source": "user" })
}

fn write_local_part(root: &Path, name: &str, body: &str) {
    let dir = root.join("_parts").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("part.jsonc"), body).unwrap();
}

fn write_wave_part(root: &Path, name: &str, voltage: Value) -> String {
    let doc = json!({
        "format": 4,
        "part": name,
        "version": "0.0.1",
        "title": name,
        "summary": "Ideal source with a fixed waveform.",
        "tags": ["test"],
        "license": "MIT OR Apache-2.0",
        "docs": {},
        "real": { "generic": true, "maker": null, "number": null, "datasheet": "test" },
        "pins": { "pos": [1], "neg": [2] },
        "model": {
            "kind": "voltage_source",
            "pins": { "p": "pos", "n": "neg" },
            "voltage": voltage
        }
    });
    write_local_part(root, name, &serde_json::to_string_pretty(&doc).unwrap());
    format!("./_parts/{name}")
}

fn write_open_switch(root: &Path, name: &str, open_at_ms: u64) -> String {
    let doc = json!({
        "format": 1,
        "part": name,
        "version": "0.0.1",
        "title": name,
        "summary": "A contact that starts closed and opens once.",
        "tags": ["test"],
        "license": "MIT OR Apache-2.0",
        "docs": {},
        "real": { "generic": true, "maker": null, "number": null, "datasheet": "test" },
        "pins": { "a": [1], "b": [2] },
        "model": {
            "kind": "switch",
            "on_resistance": { "value": 0.05, "unit": "ohm", "source": { "closed_form": { "note": "Closed." } } },
            "off_resistance": { "value": 1e8, "unit": "ohm", "source": { "closed_form": { "note": "Open." } } },
            "initial": "closed",
            "events": [{ "t_ps": ps(open_at_ms, 0), "state": "open" }],
            "bounce": null
        }
    });
    write_local_part(root, name, &serde_json::to_string_pretty(&doc).unwrap());
    format!("./_parts/{name}")
}

fn setup_project(scene: Value, test: &str, avr_firmware: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(common::repo().join("parts"), dir.path().join("parts")).unwrap();
    write_local_part(dir.path(), "bench", BENCH_PART);
    write_local_part(dir.path(), "plug", PLUG_PART);
    if avr_firmware {
        let fw = dir.path().join("fw");
        fs::create_dir_all(&fw).unwrap();
        fs::copy(
            common::repo().join("blocks/nano-button-led/build/button.ino.hex"),
            fw.join("button.ino.hex"),
        )
        .unwrap();
    }
    fs::write(
        dir.path().join("scene.jsonc"),
        serde_json::to_string_pretty(&scene).unwrap(),
    )
    .unwrap();
    let tests = dir.path().join("tests");
    fs::create_dir_all(&tests).unwrap();
    fs::write(tests.join("run.test"), test).unwrap();
    dir
}

fn pin_volts(stdout: &str, label: &str) -> f64 {
    let prefix = format!("{label} ");
    stdout
        .lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix(prefix.as_str())?;
            rest.split_whitespace().find_map(|word| word.parse().ok())
        })
        .unwrap_or_else(|| panic!("no {label} in:\n{stdout}"))
}

/// Normal power-up ramps (RC soft/fast, OLED soft ramp, staircase up) and power-down decays
/// (staircase down, USB unplug by live release or scheduled event, capacitor decay, regulator
/// decay) cross the low band cleanly and stay silent.
#[test]
fn normal_power_up_and_power_down_circuits_stay_silent() {
    // 1. RC ramps on DHT22 (soft 100 ohm, fast 10 ohm, fast with 100 us max_step).
    //    Steps that span the whole ramp stay silent on stdout as well as in final diagnostics.
    for (tag, resistance, max_step, steps) in [
        ("soft", "100", None, "step 5ms\nstep 82ms\n"),
        ("fast", "10", None, "step 5ms\nstep 2ms\nstep 80ms\n"),
        (
            "fast-fine",
            "10",
            Some("100us"),
            "step 5ms\nstep 2ms\nstep 80ms\n",
        ),
    ] {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "step-0-to-5",
            json!({ "kind": "step", "initial": volts(0.0), "final": volts(5.0), "at_ps": ps(5, 0) }),
        );
        let mut scene = json!({
            "format": 1,
            "parts": {
                "psu": { "type": src },
                "rs": { "type": "resistor-axial", "resistance": resistance },
                "c": { "type": "capacitor-electrolytic-radial", "capacitance": "100u", "initial": "0V" },
                "u": { "type": "dht22" }
            },
            "wires": [
                ["psu.neg", "c.neg"],
                ["psu.pos", "rs.a"],
                ["rs.b", "c.pos"],
                ["c.pos", "u.VDD"],
                ["c.neg", "u.GND"]
            ],
            "ground": "u.GND"
        });
        if let Some(step) = max_step {
            scene["sim"] = json!({ "max_step": step });
        }
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            format!("open\n{steps}expect @u.VDD > 4.9V\nexpect no diagnostics\n"),
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "dht-ramp-{tag}: {}{}", run.stdout, run.stderr);
        assert!(
            !run.stdout.contains("rating.supply_low"),
            "dht-ramp-{tag}:\n{}",
            run.stdout
        );
    }

    // 2. Soft RC ramp on SSD1306 OLED module (1 kohm + 47 uF).
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "step-0-to-5",
            json!({ "kind": "step", "initial": volts(0.0), "final": volts(5.0), "at_ps": ps(5, 0) }),
        );
        let scene = json!({
            "format": 1,
            "parts": {
                "psu": { "type": src },
                "rs": { "type": "resistor-axial", "resistance": "1k" },
                "c": { "type": "capacitor-electrolytic-radial", "capacitance": "47u", "initial": "0V" },
                "u": { "type": "oled-ssd1306-128x64" }
            },
            "wires": [
                ["psu.neg", "c.neg"],
                ["psu.pos", "rs.a"],
                ["rs.b", "c.pos"],
                ["c.pos", "u.VCC"],
                ["c.neg", "u.GND"]
            ],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 5ms\nstep 82ms\nexpect @u.display.VDD > 3.2V\nexpect no diagnostics\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "oled-ramp-soft: {}{}", run.stdout, run.stderr);
        assert!(
            !run.stdout.contains("rating.supply_low"),
            "oled-ramp-soft:\n{}",
            run.stdout
        );
    }

    // 3. Staircase up and staircase down on DHT22.
    for (tag, initial, events, test) in [
        (
            "stair-up",
            0.0,
            (1..=10)
                .map(|i| json!({ "t_ps": ps(2 * i, 0), "level": volts(0.5 * i as f64) }))
                .collect::<Vec<_>>(),
            "open\nstep 24ms\nget @u.VDD\nexpect no diagnostics\n",
        ),
        (
            "stair-down",
            5.0,
            (1..=10)
                .map(|i| json!({ "t_ps": ps(2 * i, 0), "level": volts(5.0 - 0.5 * i as f64) }))
                .collect::<Vec<_>>(),
            "open\nstep 4ms\nget @u.VDD\nstep 20ms\nget @u.VDD\nexpect no diagnostics\n",
        ),
    ] {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            tag,
            json!({ "kind": "steps", "initial": volts(initial), "events": events }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(dir.path().join("tests/run.test"), test).unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "{tag}: {}{}", run.stdout, run.stderr);
        assert!(
            !run.stdout.contains("rating.supply_low"),
            "{tag}:\n{}",
            run.stdout
        );
    }

    // 4. USB unplug via live `release` and via scheduled switch open.
    for (tag, scheduled) in [("usb-unplug-release", false), ("usb-unplug-event", true)] {
        let dir = setup_project(json!({}), "", false);
        let sw_type = if scheduled {
            write_open_switch(dir.path(), "open-at-5ms", 5)
        } else {
            "./_parts/plug".to_string()
        };
        let scene = json!({
            "format": 1,
            "parts": {
                "psu": { "type": "./_parts/bench", "voltage": "5V" },
                "sw": { "type": sw_type },
                "d": { "type": "diode-sod123" },
                "bulk": { "type": "capacitor-electrolytic-radial", "capacitance": "10u", "initial": "4.7V" },
                "decap": { "type": "capacitor-0603", "capacitance": "100n", "initial": "4.7V" },
                "load": { "type": "resistor-axial", "resistance": "470" },
                "u": { "type": "dht22" }
            },
            "wires": [
                ["psu.neg", "u.GND"],
                ["psu.pos", "sw.a"],
                ["sw.b", "d.anode"],
                ["d.cathode", "bulk.pos"],
                ["bulk.neg", "u.GND"],
                ["bulk.pos", "decap.a"],
                ["decap.b", "u.GND"],
                ["bulk.pos", "load.a"],
                ["load.b", "u.GND"],
                ["bulk.pos", "u.VDD"]
            ],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        let test = if scheduled {
            "open\nstep 5ms\nstep 50ms\nexpect @u.VDD < 0.1V\nexpect no diagnostics\n"
        } else {
            "open\nstep 5ms\nrelease @sw\nstep 50ms\nexpect @u.VDD < 0.1V\nexpect no diagnostics\n"
        };
        fs::write(dir.path().join("tests/run.test"), test).unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "{tag}: {}{}", run.stdout, run.stderr);
        assert!(
            !run.stdout.contains("rating.supply_low"),
            "{tag}:\n{}",
            run.stdout
        );
    }

    // 5. Capacitor decay (100 uF into 1 kohm) and regulator decay (AMS1117 input falls to 0 V).
    {
        let dir = setup_project(json!({}), "", false);
        let sw = write_open_switch(dir.path(), "open-at-5ms", 5);
        let scene = json!({
            "format": 1,
            "parts": {
                "psu": { "type": "./_parts/bench", "voltage": "5V" },
                "sw": { "type": sw },
                "c": { "type": "capacitor-electrolytic-radial", "capacitance": "100u", "initial": "5V" },
                "load": { "type": "resistor-axial", "resistance": "1k" },
                "u": { "type": "dht22" }
            },
            "wires": [
                ["psu.neg", "u.GND"],
                ["psu.pos", "sw.a"],
                ["sw.b", "c.pos"],
                ["c.neg", "u.GND"],
                ["c.pos", "load.a"],
                ["load.b", "u.GND"],
                ["c.pos", "u.VDD"]
            ],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 5ms\nstep 30ms\nstep 370ms\nexpect @u.VDD < 0.1V\nexpect no diagnostics\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "cap-decay: {}{}", run.stdout, run.stderr);
        assert!(
            !run.stdout.contains("rating.supply_low"),
            "cap-decay:\n{}",
            run.stdout
        );
    }
    {
        let dir = setup_project(json!({}), "", false);
        let fall9 = write_wave_part(
            dir.path(),
            "step-9-to-0",
            json!({
                "kind": "steps",
                "initial": volts(9.0),
                "events": [{ "t_ps": ps(5, 0), "level": volts(0.0) }]
            }),
        );
        let scene = json!({
            "format": 1,
            "parts": {
                "psu": { "type": fall9 },
                "reg": { "type": "ams1117-sot223" },
                "c": { "type": "capacitor-electrolytic-radial", "capacitance": "100u", "initial": "5V" },
                "load": { "type": "resistor-axial", "resistance": "1k" },
                "u": { "type": "dht22" }
            },
            "wires": [
                ["psu.neg", "reg.GND"],
                ["psu.pos", "reg.IN"],
                ["reg.OUT", "c.pos"],
                ["reg.GND", "c.neg"],
                ["reg.OUT", "load.a"],
                ["reg.GND", "load.b"],
                ["reg.OUT", "u.VDD"],
                ["reg.GND", "u.GND"]
            ],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 5ms\nstep 400ms\nexpect @u.VDD < 0.1V\nexpect no diagnostics\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "reg-decay: {}{}", run.stdout, run.stderr);
        assert!(
            !run.stdout.contains("rating.supply_low"),
            "reg-decay:\n{}",
            run.stdout
        );
    }
}

/// Sags (five dips merging into one warning with lowest voltage), incomplete turn-ons, and a run
/// ending while the rail sits in the low band all warn `rating.supply_low`.
#[test]
fn sags_incomplete_turn_on_and_ending_in_low_band_warn() {
    // Five dips on a DHT22 merge to one warning reporting the lowest 0.5 V and 5 ms total.
    {
        let dir = setup_project(json!({}), "", false);
        let mut events = Vec::new();
        let mut t = 2;
        for level in [2.5, 2.2, 1.5, 2.8, 0.5] {
            events.push(json!({ "t_ps": ps(t, 0), "level": volts(level) }));
            events.push(json!({ "t_ps": ps(t + 1, 0), "level": volts(5.0) }));
            t += 2;
        }
        let src = write_wave_part(
            dir.path(),
            "dip-five",
            json!({ "kind": "steps", "initial": volts(5.0), "events": events }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 30ms\nsnapshot\nexpect diagnostics contains \"rating.supply_low\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let snapshot_warnings: Vec<_> = run
            .stdout
            .lines()
            .filter(|line| line.trim_start().starts_with("warning[rating.supply_low]"))
            .collect();
        assert_eq!(snapshot_warnings.len(), 1, "{}", run.stdout);
        assert!(
            snapshot_warnings[0].contains("drops to 0.5 V")
                && snapshot_warnings[0].contains("below its 3.3 V minimum for 5ms"),
            "{}",
            snapshot_warnings[0]
        );
    }

    // Incomplete turn-on (0 -> 2.0 V -> 0 V) warns once.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "incomplete",
            json!({
                "kind": "steps",
                "initial": volts(0.0),
                "events": [
                    { "t_ps": ps(2, 0), "level": volts(2.0) },
                    { "t_ps": ps(6, 0), "level": volts(0.0) }
                ]
            }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 10ms\nsnapshot\nexpect diagnostics contains \"rating.supply_low\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        assert!(run.stdout.contains("drops to 2 V"), "{}", run.stdout);
    }

    // Ending mid-ramp while sitting in the low band warns in `snapshot` and `expect diagnostics`.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "stuck-low",
            json!({
                "kind": "steps",
                "initial": volts(0.0),
                "events": [{ "t_ps": ps(2, 0), "level": volts(2.0) }]
            }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 10ms\nsnapshot\nexpect diagnostics contains \"rating.supply_low\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        assert!(run.stdout.contains("drops to 2 V"), "{}", run.stdout);
    }
}

/// ATmega328P at 16 MHz warns below 3.78 V (naming the frequency) on both the bare chip and a
/// Nano clamped to 3.3 V, while 3.78 V, the stock USB Nano, and the stock Uno stay silent.
#[test]
fn atmega328p_minimum_follows_clock_frequency_and_stock_boards_stay_silent() {
    // Bare ATmega328P at 3.3 V warns with 3.78 V minimum at 16 MHz.
    let avr_3v3 = setup_project(
        json!({
            "format": 1,
            "parts": {
                "psu": { "type": "./_parts/bench", "voltage": "3.3V" },
                "u": { "type": "atmega328p-dip28", "firmware": "fw/button.ino.hex" }
            },
            "wires": [["psu.pos", "u.VCC"], ["psu.pos", "u.AVCC"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        }),
        "open\nstep 2ms\nsnapshot\nexpect diagnostics contains \"below its 3.78 V minimum at 16 MHz\"\n",
        true,
    );
    let run = orbe(avr_3v3.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    // Bare ATmega328P at 3.78 V stays silent on rating.supply_low.
    let avr_3v78 = setup_project(
        json!({
            "format": 1,
            "parts": {
                "psu": { "type": "./_parts/bench", "voltage": "3.78V" },
                "u": { "type": "atmega328p-dip28", "firmware": "fw/button.ino.hex" }
            },
            "wires": [["psu.pos", "u.VCC"], ["psu.pos", "u.AVCC"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        }),
        "open\nstep 2ms\nsnapshot\nexpect @u.VCC > 3.7V\n",
        true,
    );
    let run = orbe(avr_3v78.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("rating.supply_low"), "{}", run.stdout);

    // Nano clamped to 3.3 V warns rating.supply_low at 16 MHz.
    let nano_3v3 = setup_project(
        json!({
            "format": 1,
            "parts": {
                "nano": { "type": "nano-board", "firmware": "fw/button.ino.hex" },
                "psu": { "type": "./_parts/bench", "voltage": "3.3V" }
            },
            "wires": [["psu.neg", "nano.GND"], ["psu.pos", "nano.5V"]],
            "ground": "nano.GND"
        }),
        "open\nstep 2ms\nsnapshot\nexpect diagnostics contains \"below its 3.78 V minimum at 16 MHz\"\n",
        true,
    );
    let run = orbe(nano_3v3.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("warning[rating.supply_low]"),
        "{}",
        run.stdout
    );

    // Stock USB Nano (~4.76 V) and stock Uno stay silent.
    for board in ["nano-board", "uno-board"] {
        let dir = setup_project(
            json!({
                "format": 1,
                "parts": {
                    "b": { "type": board, "firmware": "fw/button.ino.hex" }
                },
                "wires": [],
                "ground": "b.GND"
            }),
            "open\nstep 2ms\nget @b.5V\nexpect @b.5V > 4.5V\nexpect no diagnostics\n",
            true,
        );
        let run = orbe(dir.path(), &["test", "-v"]);
        assert_eq!(run.code, 0, "{board}: {}{}", run.stdout, run.stderr);
    }
}

/// An ATmega328P fed through 1 kohm from 5 V sits at ~3.57 V (its 2 mA reset-current divider)
/// throughout the run, with brief 62.5 ns active-current dips to ~1.52 V whenever it leaves reset.
/// Both `open` and `snapshot` after stepping report the held ~3.57 V level, not the nanosecond dip.
#[test]
fn atmega328p_operating_point_uses_reset_current() {
    let dir = setup_project(
        json!({
            "format": 1,
            "parts": {
                "psu": { "type": "./_parts/bench", "voltage": "5V" },
                "rs": { "type": "resistor-axial", "resistance": "1k" },
                "u": { "type": "atmega328p-dip28", "firmware": "fw/button.ino.hex" }
            },
            "wires": [
                ["psu.neg", "u.GND"],
                ["psu.pos", "rs.a"],
                ["rs.b", "u.VCC"],
                ["rs.b", "u.AVCC"]
            ],
            "ground": "u.GND"
        }),
        "open\nget @u.VCC\nexpect @u.VCC > 3.5V\nexpect @u.VCC < 3.65V\nstep 1us\nget @u.VCC\nexpect @u.VCC > 3.5V\nsnapshot\nstep 200ms\nget @u.VCC\nexpect @u.VCC > 3.5V\nsnapshot\nexpect diagnostics contains \"drops to 3.57 V\"\n",
        true,
    );
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let v0 = pin_volts(&run.stdout, "u.VCC");
    assert!((v0 - 3.566).abs() < 0.05, "t=0 u.VCC={v0}\n{}", run.stdout);
    let supply_warnings: Vec<_> = run
        .stdout
        .lines()
        .filter(|line| line.trim_start().starts_with("warning[rating.supply_low]"))
        .collect();
    assert_eq!(
        supply_warnings.len(),
        3,
        "expected open + 1us snapshot + 200ms snapshot warnings:\n{}",
        run.stdout
    );
    for line in &supply_warnings {
        assert!(
            line.contains("drops to 3.57 V")
                && line.contains("below its 3.78 V minimum at 16 MHz")
                && !line.contains("1.52 V"),
            "unexpected warning line: {line}\n{}",
            run.stdout
        );
    }

    // Verify JSON output agrees between `open` and `snapshot` too.
    let json_run = orbe(dir.path(), &["test", "-v", "--json"]);
    assert_eq!(json_run.code, 0, "{}{}", json_run.stdout, json_run.stderr);
    let doc: Value = serde_json::from_str(&json_run.stdout).unwrap();
    let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
    let open_v = replies[0]["reply"]["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "rating.supply_low")
        .unwrap()["data"]["voltage"]
        .as_f64()
        .unwrap();
    let snap_diag = replies
        .iter()
        .rev()
        .find(|r| r["command"] == "snapshot")
        .unwrap()["reply"]["data"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "rating.supply_low")
        .unwrap();
    let snap_v = snap_diag["data"]["voltage"].as_f64().unwrap();
    let snap_over = snap_diag["data"]["over_ps"].as_u64().unwrap();
    assert!((open_v - 3.566).abs() < 0.05, "open_v={open_v}");
    assert!((snap_v - 3.566).abs() < 0.05, "snap_v={snap_v}");
    assert!(
        (open_v - snap_v).abs() < 1e-3,
        "open_v={open_v} vs snap_v={snap_v}"
    );
    assert_eq!(snap_over, ps(200, 1), "snap_over={snap_over}");
}

/// SK6812 warns below its 3.7 V datasheet minimum when wired alone or externally on the DevKitC-1's
/// 3V3 pin, while the DevKitC-1's own onboard LED on 3.3 V is accepted by board design and listed
/// in `orbe part esp32s3-devkitc-1`.
#[test]
fn sk6812_warns_below_3v7_and_devkitc_accepts_only_its_onboard_led() {
    // SK6812 alone at 3.5 V warns; at 3.7 V stays silent.
    let sk_3v5 = setup_project(
        json!({
            "format": 1,
            "parts": {
                "psu": { "type": "./_parts/bench", "voltage": "3.5V" },
                "led": { "type": "sk68xxmini-hs" }
            },
            "wires": [["psu.pos", "led.VDD"], ["psu.neg", "led.GND"], ["led.DIN", "led.GND"]],
            "ground": "led.GND"
        }),
        "open\nstep 20ms\nexpect diagnostics contains \"below its 3.7 V minimum\"\n",
        false,
    );
    let run = orbe(sk_3v5.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    let sk_3v7 = setup_project(
        json!({
            "format": 1,
            "parts": {
                "psu": { "type": "./_parts/bench", "voltage": "3.7V" },
                "led": { "type": "sk68xxmini-hs" }
            },
            "wires": [["psu.pos", "led.VDD"], ["psu.neg", "led.GND"], ["led.DIN", "led.GND"]],
            "ground": "led.GND"
        }),
        "open\nstep 20ms\nexpect no diagnostics\n",
        false,
    );
    let run = orbe(sk_3v7.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    // User-wired external SK6812 on the DevKitC-1's 3V3 pin warns, while the board's own LED does not.
    let devkit = tempfile::tempdir().unwrap();
    common::copy_dir(
        &common::repo().join("blocks/devkitc-button-led"),
        devkit.path(),
    );
    fs::remove_file(devkit.path().join("block.jsonc")).unwrap();
    std::os::unix::fs::symlink(common::repo().join("parts"), devkit.path().join("parts")).unwrap();
    fs::write(
        devkit.path().join("scene.jsonc"),
        serde_json::to_string_pretty(&json!({
            "format": 1,
            "parts": {
                "board": {
                    "type": "esp32s3-devkitc-1",
                    "app": "build/button.ino.bin",
                    "partitions": "build/button.ino.partitions.bin"
                },
                "ext": { "type": "sk68xxmini-hs" }
            },
            "nets": { "GND": "board.GND" },
            "wires": [
                ["board.3V3", "ext.VDD"],
                ["board.GND", "ext.GND"],
                ["ext.DIN", "board.GND"]
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        devkit.path().join("tests/button.test"),
        "open\nstep 20ms\nsnapshot\nexpect diagnostics contains \"#ext.pin.VCC\"\n",
    )
    .unwrap();
    let run = orbe(devkit.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("below its 3.7 V minimum"),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("#board__led.pin.VCC"),
        "onboard LED should stay quiet:\n{}",
        run.stdout
    );

    // `orbe part esp32s3-devkitc-1` lists the accepted deviation where `not simulated` lives.
    let card = orbe(devkit.path(), &["part", "esp32s3-devkitc-1"]);
    assert_eq!(card.code, 0, "{}{}", card.stdout, card.stderr);
    assert!(
        card.stdout
            .contains("accepted rating.supply_low on led.VDD"),
        "{}",
        card.stdout
    );
}

/// Pins the four PR 309 meter-parity behaviors:
/// 1. A slow rise that never reaches minimum tracks the rising rail instead of where it started.
/// 2. A staircase reports each active plateau on snapshot and the dip's depth after recovery.
/// 3. An oscillating rail prints identical numbers on the step line and the snapshot.
/// 4. A flat low rail reports the exact elapsed duration rather than stopping at the last step.
#[test]
fn supply_low_tracks_most_recent_held_level_exact_duration_and_step_snapshot_parity() {
    let supply_diag = |reply: &Value| -> Value {
        reply["reply"]["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["code"] == "rating.supply_low")
            .unwrap()
            .clone()
    };

    // 1. Slow RC rise (2.2 V through 10 kohm into 10 uF) never reaches DHT22's 3.3 V minimum.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(dir.path(), "rc-rise-src", volts(2.2));
        let scene = json!({
            "format": 1,
            "parts": {
                "psu": { "type": src },
                "u": { "type": "dht22" },
                "rs": { "type": "resistor-axial", "resistance": "10k" },
                "c": { "type": "capacitor-electrolytic-radial", "capacitance": "10u", "initial": "0V" }
            },
            "wires": [
                ["psu.neg", "c.neg"],
                ["psu.pos", "rs.a"],
                ["rs.b", "c.pos"],
                ["c.pos", "u.VDD"],
                ["c.neg", "u.GND"]
            ],
            "ground": "u.GND",
            "sim": { "max_step": "100us" }
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 5ms\nget @u.VDD\nsnapshot\nstep 45ms\nget @u.VDD\nsnapshot\nstep 250ms\nget @u.VDD\nsnapshot\nexpect diagnostics contains \"drops to 2.09 V\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        let mut snaps = replies.iter().filter(|r| r["command"] == "snapshot");
        let mut gets = replies.iter().filter(|r| r["command"] == "get @u.VDD");
        for expected_text in ["drops to 0.113 V", "drops to 0.867 V", "drops to 2.09 V"] {
            let get_v = gets.next().unwrap()["reply"]["data"]["value"]["v"]
                .as_f64()
                .unwrap();
            let diag = supply_diag(snaps.next().unwrap());
            let warn_v = diag["data"]["voltage"].as_f64().unwrap();
            assert!(
                (warn_v - get_v).abs() < 1e-3,
                "warn_v={warn_v} vs get_v={get_v} ({diag})"
            );
            assert!(
                diag["message"].as_str().unwrap().contains(expected_text),
                "{diag}"
            );
        }
    }

    // 2. Staircase (5 -> 3.0 -> 2.5 -> 2.0 -> 5.0 V): each mid-plateau snapshot matches `get`,
    //    and the recovered snapshot names 2 V for 15 ms.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "stair",
            json!({
                "kind": "steps",
                "initial": volts(5.0),
                "events": [
                    { "t_ps": ps(5, 0), "level": volts(3.0) },
                    { "t_ps": ps(10, 0), "level": volts(2.5) },
                    { "t_ps": ps(15, 0), "level": volts(2.0) },
                    { "t_ps": ps(20, 0), "level": volts(5.0) }
                ]
            }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 7ms\nsnapshot\nstep 5ms\nsnapshot\nstep 5ms\nsnapshot\nstep 5ms\nsnapshot\nexpect diagnostics contains \"drops to 2 V\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        let snaps: Vec<_> = replies
            .iter()
            .filter(|r| r["command"] == "snapshot")
            .map(supply_diag)
            .collect();
        let expected = [
            (3.0, ps(2, 0)),
            (2.5, ps(7, 0)),
            (2.0, ps(12, 0)),
            (2.0, ps(15, 0)),
        ];
        assert_eq!(snaps.len(), expected.len());
        for (diag, (want_v, want_over)) in snaps.iter().zip(expected) {
            let v = diag["data"]["voltage"].as_f64().unwrap();
            let over = diag["data"]["over_ps"].as_u64().unwrap();
            assert!((v - want_v).abs() < 1e-6, "got {v}, want {want_v}: {diag}");
            assert_eq!(over, want_over, "{diag}");
        }
    }

    // 3. Oscillating rails: step line and snapshot print the exact same voltage and total duration.
    for (tag, high, low, width_ns, period_ns, want_over_ps) in [
        ("osc-fast", 3.5, 3.1, 400u64, 800u64, ps(1, 0)),
        ("osc-900ns", 5.0, 2.0, 900u64, 4_000u64, ps(0, 450)),
    ] {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            tag,
            json!({
                "kind": "pulse",
                "initial": volts(high),
                "pulsed": volts(low),
                "delay_ps": ps(1, 0),
                "width_ps": width_ns * 1_000,
                "period_ps": period_ns * 1_000
            }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND",
            "sim": { "max_step": "50us" }
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 500us\nstep 2500us\nsnapshot\nexpect diagnostics contains \"rating.supply_low\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{tag}: {}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        let step_diag = replies
            .iter()
            .find(|r| r["command"] == "step 2500us")
            .unwrap()["reply"]["data"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "diagnostic" && c["diagnostic"]["code"] == "rating.supply_low")
            .unwrap()["diagnostic"]
            .clone();
        let snap_diag = supply_diag(replies.iter().find(|r| r["command"] == "snapshot").unwrap());
        assert_eq!(step_diag, snap_diag, "{tag}: step vs snapshot mismatch");
        assert_eq!(
            snap_diag["data"]["over_ps"].as_u64(),
            Some(want_over_ps),
            "{tag}: {snap_diag}"
        );
        assert!(
            (snap_diag["data"]["voltage"].as_f64().unwrap() - low).abs() < 1e-6,
            "{tag}: {snap_diag}"
        );
    }

    // 4. Flat 2.0 V rail reports exact elapsed duration (0 at open, 4 ms at 4 ms, 10 ms at 10 ms).
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(dir.path(), "dc2", volts(2.0));
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nsnapshot\nstep 4ms\nsnapshot\nstep 6ms\nsnapshot\nexpect diagnostics contains \"for 10ms\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        let snaps: Vec<_> = replies
            .iter()
            .filter(|r| r["command"] == "snapshot")
            .map(supply_diag)
            .collect();
        assert_eq!(snaps[0]["data"]["over_ps"].as_u64(), Some(0));
        assert_eq!(snaps[1]["data"]["over_ps"].as_u64(), Some(ps(4, 0)));
        assert_eq!(snaps[2]["data"]["over_ps"].as_u64(), Some(ps(10, 0)));
    }
}

/// Pins open-episode `rating.supply_low` reporting on `step` and `pause` following the same
/// two-phase key convention as `rating.power`'s `over_now`:
/// - `stair`: `step 7ms` prints `3 V` for `2ms` matching the snapshot; intermediate open steps at
///   12 ms and 17 ms do not repeat; `step 22ms` after recovery prints the settled `2 V` for `15ms`
///   matching the snapshot; a later step does not repeat.
/// - `mid-sag`: `step 5ms` prints `2 V` for `3ms` matching the snapshot; `step 10ms` prints the
///   settled `2 V` for `6ms` matching the snapshot.
/// - `dc-2v`: `open` prints `2 V` (`over_ps: 0`); `step 4ms` and `step 6ms` do not repeat while
///   snapshots report `4ms` and `10ms`.
/// - Mid-ramp stop then clean power-up: stopping mid-ramp prints `rating.supply_low` matching the
///   snapshot, and finishing the ramp leaves `no diagnostics`.
/// - Live `play`/`pause`: pausing mid-sag prints the open warning matching the snapshot; pausing
///   after recovery prints one deduplicated settled warning matching the snapshot; pausing after a
///   clean power-up spanning multiple ticks prints no `rating.supply_low`.
#[test]
fn open_supply_low_episodes_report_on_step_and_pause() {
    use orbe_scene::Files;
    use orbe_session::{Command, Live};

    let supply_in_changes = |reply: &Value| -> Vec<Value> {
        reply["data"]["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["kind"] == "diagnostic" && c["diagnostic"]["code"] == "rating.supply_low")
            .map(|c| c["diagnostic"].clone())
            .collect()
    };
    let supply_in_diags = |reply: &Value| -> Vec<Value> {
        reply["data"]["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["code"] == "rating.supply_low")
            .cloned()
            .collect()
    };

    // 1. Staircase: open step at 7 ms warns matching snapshot, intermediate open steps stay quiet,
    //    recovery step at 22 ms warns settled matching snapshot, later step stays quiet.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "stair",
            json!({
                "kind": "steps",
                "initial": volts(5.0),
                "events": [
                    { "t_ps": ps(5, 0), "level": volts(3.0) },
                    { "t_ps": ps(10, 0), "level": volts(2.5) },
                    { "t_ps": ps(15, 0), "level": volts(2.0) },
                    { "t_ps": ps(20, 0), "level": volts(5.0) }
                ]
            }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 7ms\nsnapshot\nstep 5ms\nsnapshot\nstep 5ms\nsnapshot\nstep 5ms\nsnapshot\nstep 5ms\nsnapshot\nexpect diagnostics contains \"drops to 2 V\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        let steps: Vec<_> = replies
            .iter()
            .filter(|r| r["command"].as_str().unwrap().starts_with("step "))
            .map(|r| supply_in_changes(&r["reply"]))
            .collect();
        let snaps: Vec<_> = replies
            .iter()
            .filter(|r| r["command"] == "snapshot")
            .map(|r| supply_in_diags(&r["reply"]))
            .collect();
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0], snaps[0], "7 ms open step should match snapshot");
        assert!(
            steps[1].is_empty(),
            "12 ms still-open step should not repeat: {:?}",
            steps[1]
        );
        assert!(
            steps[2].is_empty(),
            "17 ms still-open step should not repeat: {:?}",
            steps[2]
        );
        assert_eq!(
            steps[3], snaps[3],
            "22 ms recovery step should match snapshot"
        );
        assert!(
            steps[4].is_empty(),
            "27 ms post-recovery step should not repeat: {:?}",
            steps[4]
        );
    }

    // 2. Mid-sag: open step at 5 ms and closed step at 10 ms both match their snapshots.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "mid-sag",
            json!({
                "kind": "steps",
                "initial": volts(5.0),
                "events": [
                    { "t_ps": ps(2, 0), "level": volts(2.0) },
                    { "t_ps": ps(8, 0), "level": volts(5.0) }
                ]
            }),
        );
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 5ms\nsnapshot\nstep 5ms\nsnapshot\nexpect diagnostics contains \"for 6ms\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        let steps: Vec<_> = replies
            .iter()
            .filter(|r| r["command"].as_str().unwrap().starts_with("step "))
            .map(|r| supply_in_changes(&r["reply"]))
            .collect();
        let snaps: Vec<_> = replies
            .iter()
            .filter(|r| r["command"] == "snapshot")
            .map(|r| supply_in_diags(&r["reply"]))
            .collect();
        assert_eq!(
            steps[0], snaps[0],
            "5 ms mid-sag step should match snapshot"
        );
        assert_eq!(steps[0][0]["data"]["over_ps"].as_u64(), Some(ps(3, 0)));
        assert_eq!(
            steps[1], snaps[1],
            "10 ms recovery step should match snapshot"
        );
        assert_eq!(steps[1][0]["data"]["over_ps"].as_u64(), Some(ps(6, 0)));
    }

    // 3. Flat 2.0 V rail (`dc-2v`): `open` reports it once; later steps stay quiet while
    //    snapshots track exact elapsed duration.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(dir.path(), "dc2", volts(2.0));
        let scene = json!({
            "format": 1,
            "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
            "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 4ms\nsnapshot\nstep 6ms\nsnapshot\nexpect diagnostics contains \"for 10ms\"\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        assert_eq!(supply_in_diags(&replies[0]["reply"]).len(), 1);
        let steps: Vec<_> = replies
            .iter()
            .filter(|r| r["command"].as_str().unwrap().starts_with("step "))
            .map(|r| supply_in_changes(&r["reply"]))
            .collect();
        assert!(
            steps[0].is_empty(),
            "dc-2v step 4ms should not repeat open warning"
        );
        assert!(
            steps[1].is_empty(),
            "dc-2v step 6ms should not repeat open warning"
        );
    }

    // 4. Stopping mid-ramp reports the open episode on that step (matching snapshot), then
    //    finishing the clean power-up leaves `no diagnostics`.
    {
        let dir = setup_project(json!({}), "", false);
        let src = write_wave_part(
            dir.path(),
            "step-0-to-5",
            json!({ "kind": "step", "initial": volts(0.0), "final": volts(5.0), "at_ps": ps(5, 0) }),
        );
        let scene = json!({
            "format": 1,
            "parts": {
                "psu": { "type": src },
                "rs": { "type": "resistor-axial", "resistance": "100" },
                "c": { "type": "capacitor-electrolytic-radial", "capacitance": "100u", "initial": "0V" },
                "u": { "type": "dht22" }
            },
            "wires": [
                ["psu.neg", "c.neg"],
                ["psu.pos", "rs.a"],
                ["rs.b", "c.pos"],
                ["c.pos", "u.VDD"],
                ["c.neg", "u.GND"]
            ],
            "ground": "u.GND"
        });
        fs::write(
            dir.path().join("scene.jsonc"),
            serde_json::to_string_pretty(&scene).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/run.test"),
            "open\nstep 5ms\nstep 2ms\nsnapshot\nstep 80ms\nexpect @u.VDD > 4.9V\nexpect no diagnostics\n",
        )
        .unwrap();
        let run = orbe(dir.path(), &["test", "-v", "--json"]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        let doc: Value = serde_json::from_str(&run.stdout).unwrap();
        let replies = doc["data"]["tests"][0]["replies"].as_array().unwrap();
        let mid_step = supply_in_changes(&replies[2]["reply"]);
        let mid_snap = supply_in_diags(&replies[3]["reply"]);
        assert_eq!(mid_step.len(), 1);
        assert_eq!(
            mid_step, mid_snap,
            "mid-ramp step should match snapshot at 7 ms"
        );
    }

    // 5. Live `play`/`pause`: pausing mid-sag, pausing after multi-tick recovery, and pausing
    //    after a multi-tick clean power-up.
    let run_play_pause =
        |name: &str, voltage: Value, ticks_ms: &[u64]| -> (Vec<Value>, Vec<Value>) {
            let dir = setup_project(json!({}), "", false);
            let src = write_wave_part(dir.path(), name, voltage);
            let scene = json!({
                "format": 1,
                "parts": { "psu": { "type": src }, "u": { "type": "dht22" } },
                "wires": [["psu.pos", "u.VDD"], ["psu.neg", "u.GND"]],
                "ground": "u.GND"
            });
            fs::write(
                dir.path().join("scene.jsonc"),
                serde_json::to_string_pretty(&scene).unwrap(),
            )
            .unwrap();
            let mut live = Live::new(name, Files::disk());
            assert!(
                live.run(&Command::Open(dir.path().join("scene.jsonc")))
                    .ok()
            );
            assert!(live.run(&Command::Play).ok());
            for &ms in ticks_ms {
                assert!(live.tick(std::time::Duration::from_millis(ms)).is_some());
            }
            let pause_json = live.run(&Command::Pause).json();
            let snap_json = live.run(&Command::Snapshot).json();
            (supply_in_changes(&pause_json), supply_in_diags(&snap_json))
        };

    // Pause mid-sag (1 ms -> 2 V, still 2 V at 16 ms across two 8 ms ticks).
    let (pause_mid, snap_mid) = run_play_pause(
        "play-mid",
        json!({
            "kind": "steps",
            "initial": volts(5.0),
            "events": [
                { "t_ps": ps(1, 0), "level": volts(2.0) },
                { "t_ps": ps(80, 0), "level": volts(5.0) }
            ]
        }),
        &[8, 8],
    );
    assert_eq!(pause_mid.len(), 1);
    assert_eq!(pause_mid, snap_mid, "pause mid-sag should match snapshot");
    assert_eq!(pause_mid[0]["data"]["over_ps"].as_u64(), Some(ps(15, 0)));

    // Pause after recovery spanning multiple ticks: one deduplicated entry matching snapshot.
    let (pause_stair, snap_stair) = run_play_pause(
        "play-stair",
        json!({
            "kind": "steps",
            "initial": volts(5.0),
            "events": [
                { "t_ps": ps(5, 0), "level": volts(3.0) },
                { "t_ps": ps(10, 0), "level": volts(2.5) },
                { "t_ps": ps(15, 0), "level": volts(2.0) },
                { "t_ps": ps(20, 0), "level": volts(5.0) }
            ]
        }),
        &[8, 8, 8, 8],
    );
    assert_eq!(pause_stair.len(), 1);
    assert_eq!(
        pause_stair, snap_stair,
        "pause after recovery should match snapshot"
    );
    assert_eq!(pause_stair[0]["data"]["over_ps"].as_u64(), Some(ps(15, 0)));

    // Clean power-up spanning two ticks (tick 1 stops mid-staircase, tick 2 finishes at 5 V):
    // pause drops the transient open-episode warning and matches the clean snapshot.
    let (pause_up, snap_up) = run_play_pause(
        "play-power-up",
        json!({
            "kind": "steps",
            "initial": volts(0.0),
            "events": [
                { "t_ps": ps(2, 0), "level": volts(1.5) },
                { "t_ps": ps(10, 0), "level": volts(5.0) }
            ]
        }),
        &[5, 10],
    );
    assert!(
        pause_up.is_empty(),
        "clean power-up during play should stay quiet on pause: {pause_up:?}"
    );
    assert!(snap_up.is_empty());
}
