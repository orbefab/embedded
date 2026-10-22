// Each test crate uses its own part of `common`.
#[allow(dead_code)]
mod common;

use common::{resistor, source_on, transient};
use sim_circuit::{Input, Refused, Session, Setting, Status};
use sim_core::diagnostic::Diagnostic;
use sim_core::finite::Finite;
use sim_core::name::Name;
use sim_core::netlist::{self, Contact, Model, Netlist};
use sim_core::time::Time;

/// A seeded xorshift, so a failing run of calls replays.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

fn example_path(name: &str) -> String {
    format!(
        "{}/../../examples/{name}/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// `examples/<name>`, its stop, step and grid cut to `stop_ps` at most.
fn example(name: &str, stop_ps: u64) -> Netlist {
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(example_path(name)).unwrap()).unwrap();
    let transient = &mut json["analysis"]["transient"];
    for key in ["stop_ps", "max_step_ps", "grid_ps"] {
        transient[key] = transient[key].as_u64().unwrap().min(stop_ps).into();
    }
    netlist::parse(&serde_json::to_vec(&json).unwrap()).unwrap()
}

/// `examples/<name>` cut to `stop_ps`, opened with no trace and advanced to `stop` in seeded
/// calls. A call ends on one of the next four row times, or at a random time before it. At a
/// row time the probe is that row of `simulate`'s trace, bit for bit. The session lists the chip
/// signals `logic.vcd` holds, and after every call each one's level is the one `logic.vcd` holds
/// at `now`.
fn reads_are_the_batch(name: &str, stop_ps: u64, seed: u64) {
    let netlist = example(name, stop_ps);
    let results = sim_circuit::simulate(&netlist);
    let trace = results.trace().unwrap();
    let logic = trace.logic().unwrap();
    assert!(!logic.changes().is_empty(), "{name}: no signal changes");
    let interpolated: Vec<Time> = results
        .diagnostics()
        .iter()
        .filter(|d| d.code() == "trace.row_interpolated")
        .filter_map(Diagnostic::at)
        .collect();
    let rows = trace.time();
    let mut levels = logic.initial().to_vec();
    let mut changes = logic.changes().iter().peekable();
    let mut session = Session::open(netlist.clone()).unwrap();
    let signals = session.signals().to_vec();
    assert_eq!(signals, logic.signals());
    let mut rng = Rng(seed);
    let mut on_rows = 0;
    loop {
        let now = session.now();
        let next = rows.partition_point(|&row| row <= now);
        let k = (next + rng.below(4) as usize).min(rows.len() - 1);
        let after = rows[k - 1].max(now).as_ps();
        let to = match rows[k].as_ps() - after {
            gap if gap > 1 && rng.below(2) == 0 => Time::from_ps(after + 1 + rng.below(gap - 1)),
            _ => rows[k],
        };
        let running = session.advance_to(to) == Status::Running;
        let probe = session.probe();
        assert_eq!(probe.at(), session.now());
        if let Ok(row) = rows.binary_search(&probe.at()) {
            assert_eq!(probe.refs(), trace.columns());
            for (j, column) in trace.columns().iter().enumerate() {
                assert_eq!(
                    probe.get(column).unwrap().get().to_bits(),
                    trace.column(j)[row].to_bits(),
                    "{name} {column} at {:?}",
                    probe.at()
                );
            }
            assert_eq!(probe.interpolated(), interpolated.contains(&probe.at()));
            on_rows += 1;
        }
        while let Some(change) = changes.next_if(|change| change.at <= probe.at()) {
            levels[change.signal] = change.level;
        }
        assert_eq!(session.levels(), levels, "{name} at {:?}", probe.at());
        for (signal, level) in signals.iter().zip(&levels) {
            assert_eq!(
                session.level(signal),
                Some(*level),
                "{name} {signal} at {:?}",
                probe.at()
            );
        }
        if !running {
            break;
        }
    }
    assert_eq!(session.status(), Status::Stopped);
    assert_eq!(session.fault(), None);
    assert_eq!(session.now(), *rows.last().unwrap());
    assert!(changes.next().is_none(), "{name}: a change after stop");
    assert!(
        on_rows > rows.len() / 4,
        "{name}: {on_rows} of {} rows",
        rows.len()
    );
    assert_eq!(session.level(&trace.columns()[0]), None);
}

#[test]
fn a_live_potentiometer_sets_the_divider_voltage() {
    let json = r#"{
      "version": 0,
      "seed": 0,
      "analysis": { "transient": { "stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000 } },
      "ground": "gnd",
      "nets": ["gnd", "vin", "mid"],
      "parts": [
        {
          "name": "v1",
          "kind": "voltage_source",
          "pins": { "p": "vin", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" }
        },
        {
          "name": "rv1",
          "kind": "potentiometer",
          "pins": { "a": "vin", "wiper": "mid", "b": "gnd" },
          "resistance": { "value": 10000.0, "unit": "ohm", "source": "user" },
          "position": { "value": 0.25, "unit": "1", "source": "user" }
        }
      ]
    }"#;
    let mut session = opened(json);
    let mid = "#net.mid".parse().unwrap();
    let close = |got: f64, position: f64| {
        let expect = 5.0 * (1.0 - position);
        assert!(
            (got - expect).abs() <= 1e-8 * expect.abs(),
            "{got} vs {expect}"
        );
    };
    close(session.probe().get(&mid).unwrap().get(), 0.25);
    let applied = session
        .apply(&Input {
            part: Name::new("rv1").unwrap(),
            at: Time::ZERO,
            setting: Setting::Level(Finite::new(0.75).unwrap()),
        })
        .unwrap();
    assert_eq!(applied.at, Time::from_ps(1));
    assert_eq!(session.advance_to(applied.at), Status::Running);
    close(session.probe().get(&mid).unwrap().get(), 0.25);
    assert_eq!(session.advance_to(Time::from_ps(2)), Status::Running);
    close(session.probe().get(&mid).unwrap().get(), 0.75);
}

/// The session leaves the scope that parsed its netlist, so a session that borrowed it would
/// not compile.
fn opened(json: &str) -> Session {
    Session::open(netlist::parse(json.as_bytes()).unwrap()).unwrap()
}

#[test]
fn the_session_outlives_the_callers_netlist() {
    let json = transient(
        1_000,
        1_000,
        r#""gnd","vin""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "gnd", 1000.0),
        ],
    );
    let mut session = opened(&json);
    assert_eq!(session.advance_to(Time::from_ps(1_000)), Status::Stopped);
    assert_eq!(session.now(), Time::from_ps(1_000));
    let vin = "#net.vin".parse().unwrap();
    assert_eq!(session.probe().get(&vin).unwrap().get(), 5.0);
}

#[test]
fn atmega_analog_serial_reads_are_the_batch() {
    reads_are_the_batch("atmega-analog-serial", 10_000_000_000, 0x5eed_1001);
}

#[test]
fn ne555_led_reads_are_the_batch() {
    reads_are_the_batch("ne555-led", u64::MAX, 0x5eed_1002);
}

/// The diagnostic `open` refuses `json` with, which is the one `simulate` reports.
fn refused(json: &str) -> Diagnostic {
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let Err(Refused::Failed(diagnostic)) = Session::open(netlist.clone()) else {
        panic!("a session opens");
    };
    assert_eq!(
        sim_circuit::simulate(&netlist).diagnostics(),
        std::slice::from_ref(&diagnostic)
    );
    diagnostic
}

#[test]
fn an_op_analysis_opens_no_session() {
    let netlist = netlist::parse(&std::fs::read(example_path("divider")).unwrap()).unwrap();
    assert!(matches!(Session::open(netlist), Err(Refused::Op)));
}

#[test]
fn a_chain_one_past_the_dense_limit_opens_no_session() {
    let nets: Vec<String> = (0..4097).map(|k| format!(r#""n{k}""#)).collect();
    let parts: Vec<String> = (0..4097)
        .map(|k| {
            let from = if k == 0 {
                "gnd".to_string()
            } else {
                format!("n{}", k - 1)
            };
            resistor(&format!("r{k}"), &from, &format!("n{k}"), 1000.0)
        })
        .collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let json = transient(
        1_000,
        1_000,
        &format!(r#""gnd",{}"#, nets.join(",")),
        &parts,
    );
    assert_eq!(refused(&json).code(), "solver.too_large");
}

/// 1e200 V across `ohm`, from t = 0 to 1 ns.
fn huge_voltage_across(ohm: f64) -> String {
    transient(
        1_000,
        1_000,
        r#""gnd","vin""#,
        &[
            &source_on("v1", "vin", 1e200),
            &resistor("r1", "vin", "gnd", ohm),
        ],
    )
}

#[test]
fn an_operating_point_that_does_not_settle_opens_no_session() {
    let diagnostic = refused(&huge_voltage_across(1e-300));
    assert_eq!(diagnostic.code(), "solver.nonfinite");
    assert_eq!(diagnostic.refs(), ["#net.vin".parse().unwrap()]);
}

#[test]
fn an_operating_point_with_a_quantity_that_is_not_finite_opens_no_session() {
    let diagnostic = refused(&huge_voltage_across(1.0));
    assert_eq!(diagnostic.code(), "solver.nonfinite");
    assert_eq!(diagnostic.refs(), ["#r1".parse().unwrap()]);
}

/// A grid with more rows than a trace holds refuses `simulate`, and a session keeps no trace.
#[test]
fn a_grid_past_the_row_limit_still_opens_a_session() {
    let json = transient(
        1_000_000_000_000,
        1_000,
        r#""gnd","vin""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "gnd", 1000.0),
        ],
    );
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    let batch = sim_circuit::simulate(&netlist);
    assert_eq!(batch.diagnostics()[0].code(), "trace.too_large");
    let mut session = Session::open(netlist.clone()).unwrap();
    let at = Time::from_ps(10_500);
    assert_eq!(session.advance_to(at), Status::Running);
    let probe = session.probe();
    assert_eq!(probe.at(), at);
    assert_eq!(probe.get(&"#net.vin".parse().unwrap()).unwrap().get(), 5.0);
}

/// A session opened on a document exports it with each input in its switch's events. One
/// opened on a netlist has no document to export.
#[test]
fn a_session_opened_on_a_document_exports_its_inputs() {
    let json = std::fs::read(example_path("rc-switch")).unwrap();
    let mut session = Session::open_document(&json).unwrap();
    session.advance_to(Time::from_ps(40_000_000_000));
    let input = Input {
        part: Name::new("sw1").unwrap(),
        at: Time::ZERO,
        setting: Setting::Contact(Contact::Closed),
    };
    let landed = session.apply(&input).unwrap().at;
    assert_eq!(session.advance_to(Time::from_ps(u64::MAX)), Status::Stopped);
    let exported = netlist::parse(&session.export().unwrap()).unwrap();
    let id = exported.part_id(&input.part).unwrap();
    let Model::Switch(switch) = &exported.part(id).model else {
        panic!("sw1 is a switch");
    };
    let last = switch.events.events().last().unwrap();
    assert_eq!((last.at, last.to), (landed, Contact::Closed));
    assert_eq!(Session::open(exported).unwrap().export(), None);
    assert!(matches!(
        Session::open_document(b"{}"),
        Err(Refused::Invalid(_))
    ));
}
