//! Circuit-level tests for the MAX7219 LED matrix driver: single chip, 4-chip daisy chain,
//! ISET resistor scaling, intensity/scan-limit/shutdown, missing LOAD, reversed DIN/DOUT,
//! unpowered behavior, and datasheet ratings warnings.

use orbe_engine::Session;
use orbe_engine::blob::Blobs;
use orbe_engine::device::max7219::dot_brightness;
use orbe_engine::diagnostic::{Diagnostic, ProtocolRule};
use orbe_engine::display::{art, lit_count};
use orbe_engine::name::Name;
use orbe_model::Time;

fn val(value: f64, unit: &str) -> serde_json::Value {
    serde_json::json!({ "value": value, "unit": unit })
}

fn source(name: &str, net: &str, volts: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "gnd" },
        "voltage": val(volts, "V"),
    })
}

fn steps(name: &str, net: &str, initial: f64, events: &[(u64, f64)]) -> serde_json::Value {
    let events: Vec<_> = events
        .iter()
        .map(|(t, volts)| serde_json::json!({ "t_ps": t, "level": val(*volts, "V") }))
        .collect();
    serde_json::json!({
        "name": name,
        "kind": "voltage_source",
        "pins": { "p": net, "n": "gnd" },
        "voltage": {
            "kind": "steps",
            "initial": val(initial, "V"),
            "events": events,
        },
    })
}

fn resistor(name: &str, a: &str, b: &str, ohm: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": "resistor",
        "pins": { "a": a, "b": b },
        "resistance": val(ohm, "ohm"),
        "ratings": { "power": null },
    })
}

fn max7219(
    name: &str,
    vcc: &str,
    din: &str,
    clk: &str,
    load: &str,
    dout: &str,
    iset: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": "max7219",
        "pins": {
            "VCC": vcc, "GND": "gnd", "DIN": din, "CLK": clk,
            "LOAD": load, "DOUT": dout, "ISET": iset,
        },
        "input_threshold": { "low": val(0.8, "V"), "high": val(3.5, "V") },
        "dout": {
            "high_drop": val(0.0, "V"), "high_resistance": val(1000.0, "ohm"),
            "low_drop": val(0.0, "V"), "low_resistance": val(250.0, "ohm"),
        },
        "led_forward_voltage": val(1.85, "V"),
        "draw": {
            "shutdown_current": val(0.00015, "A"),
            "logic_current": val(0.008, "A"),
            "holds_above": val(4.0, "V"),
        },
        "ratings": {
            "rail_margin": val(0.3, "V"),
            "supply_voltage": val(6.0, "V"),
            "input_voltage": val(6.0, "V"),
            "supply_current": { "value": 0.5, "unit": "A", "basis": "peak" },
        },
    })
}

struct SpiWave {
    t: u64,
    v_high: f64,
    v_low: f64,
    din: Vec<(u64, f64)>,
    clk: Vec<(u64, f64)>,
    load: Vec<(u64, f64)>,
    din_v: f64,
    clk_v: f64,
    load_v: f64,
}

impl SpiWave {
    fn new() -> Self {
        Self::with_levels(0.0, 5.0)
    }

    fn with_levels(v_low: f64, v_high: f64) -> Self {
        Self {
            t: 1_000_000, // 1 us
            v_high,
            v_low,
            din: Vec::new(),
            clk: Vec::new(),
            load: Vec::new(),
            din_v: v_low,
            clk_v: v_low,
            load_v: v_high,
        }
    }

    fn set_din(&mut self, v: f64) {
        if (self.din_v - v).abs() > 1e-6 {
            self.din.push((self.t, v));
            self.din_v = v;
        }
    }

    fn set_clk(&mut self, v: f64) {
        if (self.clk_v - v).abs() > 1e-6 {
            self.clk.push((self.t, v));
            self.clk_v = v;
        }
    }

    fn set_load(&mut self, v: f64) {
        if (self.load_v - v).abs() > 1e-6 {
            self.load.push((self.t, v));
            self.load_v = v;
        }
    }

    /// Shifts one or more 16-bit words (chain order: farthest chip first) and optionally pulses LOAD.
    fn send(&mut self, words: &[u16], pulse_load: bool) {
        const STEP: u64 = 1_000_000; // 1 us per half-clock
        let (lo, hi) = (self.v_low, self.v_high);
        if pulse_load {
            self.set_load(lo);
            self.t += STEP;
        }
        for &word in words {
            for bit in (0..16).rev() {
                let v = if (word >> bit) & 1 != 0 { hi } else { lo };
                self.set_din(v);
                self.t += STEP;
                self.set_clk(hi);
                self.t += STEP;
                self.set_clk(lo);
                self.t += STEP;
            }
        }
        if pulse_load {
            self.set_load(hi);
            self.t += STEP * 2;
        }
    }
}

fn run_session(doc: serde_json::Value) -> Session {
    let json = serde_json::to_vec(&doc).unwrap();
    let netlist = orbe_engine::netlist::parse_with(&json, &Blobs::new()).expect("valid netlist");
    let mut session = Session::open(netlist).expect("open session");
    let stop = doc["analysis"]["transient"]["stop_ps"].as_u64().unwrap();
    session.advance_to(Time::from_ps(stop));
    session
}

#[test]
fn single_chip_rset_intensity_scan_and_shutdown_work_end_to_end() {
    let run_with = |rset: f64, intensity: u16, scan: u16, shutdown: bool| {
        let mut wave = SpiWave::new();
        wave.send(&[0x0C01], true); // wake
        wave.send(&[0x0B00 | scan], true);
        wave.send(&[0x0A00 | intensity], true);
        wave.send(&[0x01FF], true); // row 0 all lit
        wave.send(&[0x08AA], true); // row 7 alternating
        if shutdown {
            wave.send(&[0x0C00], true);
        }
        let stop = wave.t + 10_000_000;
        let doc = serde_json::json!({
            "version": 1,
            "seed": 0,
            "analysis": { "transient": { "stop_ps": stop, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
            "ground": "gnd",
            "nets": ["gnd", "vcc", "din", "clk", "load", "dout", "iset"],
            "parts": [
                source("v_vcc", "vcc", 5.0),
                steps("v_din", "din", 0.0, &wave.din),
                steps("v_clk", "clk", 0.0, &wave.clk),
                steps("v_load", "load", 5.0, &wave.load),
                resistor("rset", "vcc", "iset", rset),
                max7219("u1", "vcc", "din", "clk", "load", "dout", "iset"),
            ],
        });
        let session = run_session(doc);
        assert!(
            session.diagnostics().is_empty(),
            "{:?}",
            session.diagnostics()
        );
        let disp = session.display(&Name::new("u1").unwrap()).unwrap();
        let panel = disp.panel();
        let frame = disp.image();
        let px = disp.pixels();
        let grid = dot_brightness(&panel, &frame);
        (lit_count(&px), art(panel.width, &px), grid[0][0])
    };

    let (lit_hi, rows, bright_hi) = run_with(10_000.0, 15, 7, false);
    assert_eq!(lit_hi, 12);
    assert!(rows.starts_with("########\n"), "{rows}");
    assert!(rows.ends_with("#.#.#.#.\n"), "{rows}");

    // Lower intensity lowers brightness by 31/1
    let (lit_lo, _, bright_lo) = run_with(10_000.0, 0, 7, false);
    assert_eq!(lit_lo, 12);
    assert!(
        (bright_hi / bright_lo - 31.0).abs() < 0.2,
        "{bright_hi} vs {bright_lo}"
    );

    // Larger RSET (28k vs 10k) reduces current
    let (_, _, bright_28k) = run_with(28_000.0, 15, 7, false);
    assert!(bright_28k < bright_hi * 0.6, "{bright_28k} vs {bright_hi}");

    // Scan limit 3 (4 digits) blanks row 7 and doubles row 0 brightness
    let (lit_scan, _, bright_scan) = run_with(10_000.0, 15, 3, false);
    assert_eq!(lit_scan, 8);
    assert!(
        (bright_scan / bright_hi - 2.0).abs() < 0.05,
        "{bright_scan} vs {bright_hi}"
    );

    // Shutdown blanks everything
    let (lit_off, _, bright_off) = run_with(10_000.0, 15, 7, true);
    assert_eq!(lit_off, 0);
    assert_eq!(bright_off, 0.0);
}

#[test]
fn four_chained_chips_shift_distinct_rows_via_dout() {
    let mut wave = SpiWave::new();
    // Broadcast wake, scan=7, intensity=8 to all 4 chips
    wave.send(&[0x0C01, 0x0C01, 0x0C01, 0x0C01], true);
    wave.send(&[0x0B07, 0x0B07, 0x0B07, 0x0B07], true);
    wave.send(&[0x0A08, 0x0A08, 0x0A08, 0x0A08], true);
    // Distinct row 0 per chip: shifted farthest-first -> u4=0x81, u3=0x42, u2=0x24, u1=0x18
    wave.send(&[0x0181, 0x0142, 0x0124, 0x0118], true);

    let stop = wave.t + 10_000_000;
    let doc = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": stop, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "din", "clk", "load", "d01", "d12", "d23", "dout", "i1", "i2", "i3", "i4"],
        "parts": [
            source("v_vcc", "vcc", 5.0),
            steps("v_din", "din", 0.0, &wave.din),
            steps("v_clk", "clk", 0.0, &wave.clk),
            steps("v_load", "load", 5.0, &wave.load),
            resistor("r1", "vcc", "i1", 10_000.0),
            resistor("r2", "vcc", "i2", 10_000.0),
            resistor("r3", "vcc", "i3", 10_000.0),
            resistor("r4", "vcc", "i4", 10_000.0),
            max7219("u1", "vcc", "din", "clk", "load", "d01", "i1"),
            max7219("u2", "vcc", "d01", "clk", "load", "d12", "i2"),
            max7219("u3", "vcc", "d12", "clk", "load", "d23", "i3"),
            max7219("u4", "vcc", "d23", "clk", "load", "dout", "i4"),
        ],
    });
    let session = run_session(doc);
    assert!(
        session.diagnostics().is_empty(),
        "{:?}",
        session.diagnostics()
    );
    for (name, expected_row0) in [
        ("u1", "...##..."),
        ("u2", "..#..#.."),
        ("u3", ".#....#."),
        ("u4", "#......#"),
    ] {
        let disp = session.display(&Name::new(name).unwrap()).unwrap();
        let rows = art(disp.panel().width, &disp.pixels());
        assert_eq!(
            rows.lines().next().unwrap(),
            expected_row0,
            "{name}: {rows}"
        );
    }
}

#[test]
fn missing_load_reversed_din_dout_and_unpowered_leave_matrix_blank() {
    // 1) Missing LOAD: words shifted in without LOAD rising edge never latch
    let mut wave = SpiWave::new();
    wave.send(&[0x0C01], false);
    wave.send(&[0x01FF], false);
    let stop = wave.t + 10_000_000;
    let missing_load = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": stop, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "din", "clk", "load", "dout", "iset"],
        "parts": [
            source("v_vcc", "vcc", 5.0),
            steps("v_din", "din", 0.0, &wave.din),
            steps("v_clk", "clk", 0.0, &wave.clk),
            source("v_load", "load", 0.0), // held low forever
            resistor("rset", "vcc", "iset", 10_000.0),
            max7219("u1", "vcc", "din", "clk", "load", "dout", "iset"),
        ],
    });
    let s = run_session(missing_load);
    assert_eq!(
        lit_count(&s.display(&Name::new("u1").unwrap()).unwrap().pixels()),
        0
    );

    // 2) Reversed DIN/DOUT: MCU drives DOUT while DIN sits at GND -> matrix stays blank
    let mut wave2 = SpiWave::new();
    wave2.send(&[0x0C01], true);
    wave2.send(&[0x01FF], true);
    let stop2 = wave2.t + 10_000_000;
    let reversed = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": stop2, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "sig", "clk", "load", "iset"],
        "parts": [
            source("v_vcc", "vcc", 5.0),
            steps("v_sig", "sig", 0.0, &wave2.din),
            steps("v_clk", "clk", 0.0, &wave2.clk),
            steps("v_load", "load", 5.0, &wave2.load),
            resistor("rset", "vcc", "iset", 10_000.0),
            // DIN wired to gnd, DOUT wired to sig!
            max7219("u1", "vcc", "gnd", "clk", "load", "sig", "iset"),
        ],
    });
    let s2 = run_session(reversed);
    assert_eq!(
        lit_count(&s2.display(&Name::new("u1").unwrap()).unwrap().pixels()),
        0
    );

    // 3) Unpowered (0 V): stays blank and draws nothing
    let unpowered = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": stop2, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "din", "clk", "load", "dout", "iset"],
        "parts": [
            source("v_vcc", "vcc", 0.0),
            steps("v_din", "din", 0.0, &wave2.din),
            steps("v_clk", "clk", 0.0, &wave2.clk),
            steps("v_load", "load", 5.0, &wave2.load),
            resistor("rset", "vcc", "iset", 10_000.0),
            max7219("u1", "vcc", "din", "clk", "load", "dout", "iset"),
        ],
    });
    let s3 = run_session(unpowered);
    assert_eq!(
        lit_count(&s3.display(&Name::new("u1").unwrap()).unwrap().pixels()),
        0
    );
}

#[test]
fn outside_datasheet_supply_and_current_keep_working_and_warn() {
    // 6.5 V supply (> 6 V abs max) + tiny 1 kohm RSET (> 500 mA display test) -> warns and keeps working
    let mut wave = SpiWave::new();
    wave.send(&[0x0F01], true); // display test: all 64 LEDs at 31/32
    let stop = wave.t + 20_000_000;
    let doc = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": stop, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "din", "clk", "load", "dout", "iset"],
        "parts": [
            source("v_vcc", "vcc", 6.5),
            steps("v_din", "din", 0.0, &wave.din),
            steps("v_clk", "clk", 0.0, &wave.clk),
            steps("v_load", "load", 5.0, &wave.load),
            resistor("rset", "vcc", "iset", 1_000.0),
            max7219("u1", "vcc", "din", "clk", "load", "dout", "iset"),
        ],
    });
    let s = run_session(doc);
    // Keeps working: all 64 LEDs lit, and dot current readback does not clip at 65.535 mA!
    let disp = s.display(&Name::new("u1").unwrap()).unwrap();
    let panel = disp.panel();
    let frame = disp.image();
    assert_eq!(lit_count(&disp.pixels()), 64);
    let dot_a = dot_brightness(&panel, &frame)[0][0];
    assert!(
        dot_a > 0.0656 && (dot_a - 0.06592).abs() < 0.0005,
        "expected unclipped ~65.9 mA/dot, got {dot_a}"
    );
    let has_v = s
        .diagnostics()
        .iter()
        .any(|d| matches!(d, Diagnostic::RatingVoltage(_)));
    let has_i = s
        .diagnostics()
        .iter()
        .any(|d| matches!(d, Diagnostic::RatingCurrent(_)));
    assert!(
        has_v,
        "expected supply voltage rating warning: {:?}",
        s.diagnostics()
    );
    assert!(
        has_i,
        "expected supply current rating warning: {:?}",
        s.diagnostics()
    );
}

#[test]
fn exact_vih_3v5_and_vil_0v8_work_while_3v3_logic_warns_input_level() {
    let run_levels = |v_low: f64, v_high: f64| {
        let mut wave = SpiWave::with_levels(v_low, v_high);
        for word in [0x0C01, 0x0B07, 0x0A0F, 0x01FF] {
            wave.send(&[word], true);
        }
        let stop = wave.t + 10_000_000;
        let doc = serde_json::json!({
            "version": 1,
            "seed": 0,
            "analysis": { "transient": { "stop_ps": stop, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
            "ground": "gnd",
            "nets": ["gnd", "vcc", "din", "clk", "load", "dout", "iset"],
            "parts": [
                source("v_vcc", "vcc", 5.0),
                steps("v_din", "din", v_low, &wave.din),
                steps("v_clk", "clk", v_low, &wave.clk),
                steps("v_load", "load", v_high, &wave.load),
                resistor("rset", "vcc", "iset", 10_000.0),
                max7219("u1", "vcc", "din", "clk", "load", "dout", "iset"),
            ],
        });
        let s = run_session(doc);
        let lit = lit_count(&s.display(&Name::new("u1").unwrap()).unwrap().pixels());
        (lit, s.diagnostics())
    };

    // Exact datasheet VIH = 3.50 V and VIL = 0.80 V work cleanly with no warnings.
    let (lit_exact, diags_exact) = run_levels(0.80, 3.50);
    assert_eq!(lit_exact, 8);
    assert!(diags_exact.is_empty(), "{diags_exact:?}");

    // 3.3 V logic (ESP32-S3 driving a 5 V MAX7219) sits in (0.8 V, 3.5 V), leaves matrix dark,
    // and warns chip.protocol rule input_level.
    let (lit_3v3, diags_3v3) = run_levels(0.0, 3.30);
    assert_eq!(lit_3v3, 0);
    assert!(
        diags_3v3.iter().any(|d| matches!(
            d,
            Diagnostic::ChipProtocol {
                rule: ProtocolRule::InputLevel,
                ..
            }
        )),
        "expected chip.protocol input_level warning at 3.3 V: {diags_3v3:?}"
    );

    // A normal RC edge (10 kohm + 20 pF, ~200 ns across 0.8 V..3.5 V, like a BSS138 level
    // shifter) does not false-alarm on input_level.
    let mut rc_wave = SpiWave::new();
    for word in [0x0C01, 0x0B07, 0x0A0F, 0x01FF] {
        rc_wave.send(&[word], true);
    }
    let rc_stop = rc_wave.t + 10_000_000;
    let rc_doc = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "transient": { "stop_ps": rc_stop, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64 } },
        "ground": "gnd",
        "nets": ["gnd", "vcc", "din_src", "din", "clk", "load", "dout", "iset"],
        "parts": [
            source("v_vcc", "vcc", 5.0),
            steps("v_din", "din_src", 0.0, &rc_wave.din),
            resistor("r_din", "din_src", "din", 10_000.0),
            {
                "name": "c_din",
                "kind": "capacitor",
                "pins": { "a": "din", "b": "gnd" },
                "capacitance": val(20e-12, "F"),
                "initial": null,
            },
            steps("v_clk", "clk", 0.0, &rc_wave.clk),
            steps("v_load", "load", 5.0, &rc_wave.load),
            resistor("rset", "vcc", "iset", 10_000.0),
            max7219("u1", "vcc", "din", "clk", "load", "dout", "iset"),
        ],
    });
    let s_rc = run_session(rc_doc);
    assert_eq!(
        lit_count(&s_rc.display(&Name::new("u1").unwrap()).unwrap().pixels()),
        8
    );
    assert!(s_rc.diagnostics().is_empty(), "{:?}", s_rc.diagnostics());
}

#[test]
fn supply_dip_retains_registers_down_to_2v_and_resets_below_2v() {
    let run_dip = |dip_v: f64| {
        let mut wave = SpiWave::new();
        for word in [0x0C01, 0x0B07, 0x0A0F, 0x01FF] {
            wave.send(&[word], true);
        }
        let dip_start = wave.t + 5_000_000;
        let dip_end = dip_start + 10_000_000;
        let stop = dip_end + 10_000_000;
        let doc = serde_json::json!({
            "version": 1,
            "seed": 0,
            "analysis": { "transient": { "stop_ps": stop, "max_step_ps": 1_000_000u64, "grid_ps": 5_000_000u64 } },
            "ground": "gnd",
            "nets": ["gnd", "vcc", "din", "clk", "load", "dout", "iset"],
            "parts": [
                steps("v_vcc", "vcc", 5.0, &[(dip_start, dip_v), (dip_end, 5.0)]),
                steps("v_din", "din", 0.0, &wave.din),
                steps("v_clk", "clk", 0.0, &wave.clk),
                steps("v_load", "load", 5.0, &wave.load),
                resistor("rset", "vcc", "iset", 10_000.0),
                max7219("u1", "vcc", "din", "clk", "load", "dout", "iset"),
            ],
        });
        let s = run_session(doc);
        assert!(
            s.diagnostics().is_empty(),
            "dip={dip_v}V: {:?}",
            s.diagnostics()
        );
        lit_count(&s.display(&Name::new("u1").unwrap()).unwrap().pixels())
    };

    for dip in [3.999, 3.3, 2.5, 2.0] {
        assert_eq!(run_dip(dip), 8, "dip={dip}V should retain registers");
    }
    assert_eq!(run_dip(1.9), 0, "dip=1.9V below 2 V floor should reset");
}
