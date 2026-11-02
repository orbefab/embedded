// Each test crate uses its own part of `common`.
#[allow(dead_code)]
mod common;

use std::fs;

use common::{held_capacitor, resistor, source_on, trace, transient};
use sim_circuit::{EdgeCursor, Input, Refused, SerialCursor, Session, Setting, Status};
use sim_core::blob::Blobs;
use sim_core::diagnostic::{Diagnostic, Severity};
use sim_core::finite::Finite;
use sim_core::name::{Name, Ref};
use sim_core::netlist::{self, Analysis, Contact, Model, Netlist};
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;

/// A worker thread runs a `Session`, so this stops compiling if `Session` is not `Send`.
#[test]
fn session_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Session>();
}

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
/// at `now`. Each probe has one integral per ref. The changes read since the previous call are
/// the ones `logic.vcd` holds in `(previous now, now]`, and the whole log is `logic.changes()`.
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
    let mut edge_cursor = EdgeCursor::default();
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
        assert_eq!(probe.integrals().len(), probe.refs().len(), "{name}");
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
        let (changes, next_cursor) = session.edges_since(edge_cursor).expect("cursor still held");
        let expect: Vec<_> = logic
            .changes()
            .iter()
            .copied()
            .filter(|change| change.at > now && change.at <= probe.at())
            .collect();
        assert_eq!(
            changes,
            &expect[..],
            "{name} edges in ({now:?}, {:?}]",
            probe.at()
        );
        edge_cursor = next_cursor;
        if !running {
            break;
        }
    }
    assert_eq!(session.status(), Status::Stopped);
    assert_eq!(session.fault(), None);
    assert_eq!(session.now(), *rows.last().unwrap());
    assert!(changes.next().is_none(), "{name}: a change after stop");
    let (edges, end) = session
        .edges_since(EdgeCursor::default())
        .expect("cursor still held");
    assert_eq!(edges, logic.changes(), "{name}: the edge log");
    assert_eq!(end, edge_cursor);
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

/// `record_document` refuses what `open_document` refuses, and a grid with more rows than a
/// trace holds, with the diagnostic `simulate` reports.
#[test]
fn a_recorded_session_refuses_an_op_an_invalid_document_and_a_grid_past_the_row_limit() {
    let divider = std::fs::read(example_path("divider")).unwrap();
    assert!(matches!(
        Session::record_document(&divider),
        Err(Refused::Op)
    ));
    assert!(matches!(
        Session::record_document(b"{}"),
        Err(Refused::Invalid(_))
    ));
    let json = transient(
        1_000_000_000_000,
        1_000,
        r#""gnd","vin""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "gnd", 1000.0),
        ],
    );
    let Err(Refused::Failed(diagnostic)) = Session::record_document(json.as_bytes()) else {
        panic!("a session opens");
    };
    assert_eq!(diagnostic.code(), "trace.too_large");
    let netlist = netlist::parse(json.as_bytes()).unwrap();
    assert_eq!(
        sim_circuit::simulate(&netlist).diagnostics(),
        std::slice::from_ref(&diagnostic)
    );
}

#[test]
fn a_session_that_records_nothing_finishes_to_no_folder() {
    let json = std::fs::read(example_path("v-steps")).unwrap();
    let at = Timestamp::from_unix_seconds(0);
    assert!(Session::open_document(&json).unwrap().finish(at).is_none());
    let netlist = netlist::parse(&json).unwrap();
    assert!(Session::open(netlist).unwrap().finish(at).is_none());
}

/// A cursor is the count of bytes read, so a count read back resumes the log there.
#[test]
fn a_serial_cursor_is_the_count_of_bytes_read() {
    assert_eq!(SerialCursor::from(0), SerialCursor::default());
    let mut session = Session::open(example("atmega-analog-serial", u64::MAX)).unwrap();
    assert_eq!(session.advance_to(Time::from_ps(u64::MAX)), Status::Stopped);
    let (all, end) = session
        .serial_since(SerialCursor::default())
        .expect("cursor still held");
    let all = all.to_vec();
    assert!(!all.is_empty());
    assert_eq!(u64::from(end), all.len() as u64);
    for read in 0..=all.len() as u64 {
        let (bytes, cursor) = session
            .serial_since(SerialCursor::from(read))
            .expect("cursor still held");
        assert_eq!(bytes, &all[read as usize..]);
        assert_eq!(cursor, end);
    }
    let past = SerialCursor::from(all.len() as u64 + 1);
    assert_eq!(session.serial_since(past), Some((&[][..], past)));
    let kept = SerialCursor::from(4);
    session.forget_serial(kept);
    assert!(session.serial_since(SerialCursor::default()).is_none());
    assert!(session.serial_since(SerialCursor::from(3)).is_none());
    let (tail, cursor) = session.serial_since(kept).expect("cursor still held");
    assert_eq!(tail, &all[4..]);
    assert_eq!(cursor, end);
    assert_eq!(session.serial_since(past), Some((&[][..], past)));
    session.forget_serial(SerialCursor::default());
    assert!(session.serial_since(SerialCursor::from(3)).is_none());
    let (tail, _) = session.serial_since(kept).expect("an earlier forget stays");
    assert_eq!(tail, &all[4..]);
}

const FRAME_PS: u64 = 16_666_666_667;
// The step after an edge is 1 ns, so the dense calls run from 2 ns before it to 4 ns after it.
const EDGE_BEFORE_PS: u64 = 2_000;
const EDGE_AFTER_PS: u64 = 4_000;

fn frame_times(stop: u64) -> Vec<u64> {
    let mut times = Vec::new();
    let mut at = FRAME_PS;
    while at < stop {
        times.push(at);
        at += FRAME_PS;
    }
    times.push(stop);
    times
}

fn first_edges(netlist: &Netlist, stop: u64, n: usize) -> Vec<u64> {
    let mut edges = Vec::new();
    if let Some(logic) = sim_circuit::simulate(netlist)
        .trace()
        .and_then(|trace| trace.logic())
    {
        for change in logic.changes() {
            let at = change.at.as_ps();
            if at == 0 || at >= stop {
                continue;
            }
            if edges.last() == Some(&at) {
                continue;
            }
            edges.push(at);
            if edges.len() == n {
                return edges;
            }
        }
    }
    if !edges.is_empty() {
        return edges;
    }
    for (_, part) in netlist.parts() {
        let Model::Switch(switch) = &part.model else {
            continue;
        };
        for event in switch.schedule.events() {
            let at = event.at.as_ps();
            if at > 0 && at < stop {
                edges.push(at);
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges.truncate(n);
    assert!(!edges.is_empty(), "no edge before {stop}");
    edges
}

fn window(times: &mut Vec<u64>, at: u64, stop: u64) {
    let from = at.saturating_sub(EDGE_BEFORE_PS).max(1);
    let to = at.saturating_add(EDGE_AFTER_PS).min(stop);
    let mut cursor = from;
    while cursor <= to {
        times.push(cursor);
        cursor += 1;
    }
}

fn dense_times(stop: u64, edges: &[u64], held: u64) -> Vec<u64> {
    let mut times = frame_times(stop);
    for &edge in edges {
        window(&mut times, edge, stop);
    }
    window(&mut times, held, stop);
    times.sort_unstable();
    times.dedup();
    times
}

/// Accepted-step times of `name` with `stop_ps` set to `stop`. `max_step_ps` is cut to `stop`
/// when the file's step is longer, which does not move a first step shorter than that cut.
fn accepted_times(name: &str, stop: u64) -> Vec<u64> {
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(example_path(name)).unwrap()).unwrap();
    let transient = &mut json["analysis"]["transient"];
    transient["stop_ps"] = stop.into();
    let max_step = transient["max_step_ps"].as_u64().unwrap().min(stop);
    transient["max_step_ps"] = max_step.into();
    let grid = transient["grid_ps"].as_u64().unwrap().clamp(1_000, stop);
    transient["grid_ps"] = grid.into();
    let netlist = netlist::parse(&serde_json::to_vec(&json).unwrap()).unwrap();
    let results = sim_circuit::simulate_steps(&netlist);
    let times: Vec<u64> = results
        .trace()
        .unwrap()
        .time()
        .iter()
        .map(|at| at.as_ps())
        .collect();
    assert!(times.len() >= 2, "{name} stopped at {stop}");
    times
}

/// The end of the step `open` holds. A short run whose stop clips that step lands on the stop,
/// so the search steps the stop out until the landing stays put.
fn first_held_end(name: &str, stop: u64) -> u64 {
    let mut hi = 1_000u64;
    loop {
        let landed = accepted_times(name, hi)[1];
        if landed < hi {
            assert!(landed > 1, "{name}: held step {landed}");
            return landed;
        }
        let next = hi.saturating_mul(2);
        assert!(
            next > hi && next <= stop,
            "{name}: no held step before {stop}"
        );
        hi = next;
    }
}

fn integral_bits(session: &Session) -> Vec<u64> {
    let probe = session.probe();
    assert_eq!(probe.integrals().len(), probe.refs().len());
    probe
        .integrals()
        .iter()
        .map(|value| value.to_bits())
        .collect()
}

fn integrals_match_across_splits(name: &str, stop: u64) {
    let netlist = example(name, stop);
    let frames = frame_times(stop);
    let edges = first_edges(&netlist, stop, 4);
    let held = first_held_end(name, stop);
    let apart = EDGE_BEFORE_PS + EDGE_AFTER_PS;
    assert!(
        edges.iter().all(|&edge| held.abs_diff(edge) > apart),
        "{name}: held step {held} sits in an edge window"
    );
    let dense = dense_times(stop, &edges, held);
    assert!(
        dense.contains(&held),
        "{name}: held step {held} not sampled"
    );
    assert!(
        dense.contains(&(held - 1)),
        "{name}: no sample before held step {held}"
    );
    assert!(dense.len() > frames.len(), "{name}: no 1 ps calls");

    let mut whole = Session::open(netlist.clone()).unwrap();
    assert_eq!(whole.advance_to(Time::from_ps(stop)), Status::Stopped);
    let at_stop = integral_bits(&whole);

    let mut framed = Session::open(netlist.clone()).unwrap();
    let mut framed_reads = Vec::new();
    for &at in &frames {
        let status = framed.advance_to(Time::from_ps(at));
        assert_eq!(framed.now().as_ps(), at, "{name}");
        assert_eq!(
            status,
            if at == stop {
                Status::Stopped
            } else {
                Status::Running
            },
            "{name} at {at}"
        );
        framed_reads.push((at, integral_bits(&framed)));
    }

    let mut split = Session::open(netlist.clone()).unwrap();
    let mut split_at = std::collections::BTreeMap::new();
    for &at in &dense {
        split.advance_to(Time::from_ps(at));
        assert_eq!(split.now().as_ps(), at, "{name}");
        split_at.insert(at, integral_bits(&split));
    }

    for (at, bits) in &framed_reads {
        assert_eq!(split_at.get(at), Some(bits), "{name} integrals at {at}");
    }
    assert_eq!(framed_reads.last().map(|(_, bits)| bits), Some(&at_stop));
    assert_eq!(split_at.get(&stop), Some(&at_stop), "{name} at stop");
    for at in [held - 1, held, held + 1] {
        assert!(dense.contains(&at), "{name}: no sample at {at}");
        let mut fresh = Session::open(netlist.clone()).unwrap();
        fresh.advance_to(Time::from_ps(at));
        assert_eq!(fresh.now().as_ps(), at, "{name}");
        assert_eq!(
            split_at.get(&at),
            Some(&integral_bits(&fresh)),
            "{name} integrals at {at}"
        );
    }
    let whole_edges = edge_list(&whole);
    assert_eq!(edge_list(&framed), whole_edges, "{name} edges");
    assert_eq!(edge_list(&split), whole_edges, "{name} edges");
}

fn edge_list(session: &Session) -> Vec<sim_core::logic::Change> {
    session
        .edges_since(EdgeCursor::default())
        .expect("cursor still held")
        .0
        .to_vec()
}

#[test]
fn integrals_match_across_splits_of_atmega_fade() {
    integrals_match_across_splits("atmega-fade", 50_000_000_000);
}

#[test]
fn integrals_match_across_splits_of_rc_switch() {
    integrals_match_across_splits("rc-switch", 50_000_000_000);
}

fn advance_at(session: &mut Session, times: &[u64]) {
    for &ps in times {
        session.advance_to(Time::from_ps(ps));
        assert_eq!(session.now().as_ps(), ps);
    }
}

/// `open` and `record_document` on one chip netlist, each advanced in its own splits after the
/// same position input, return the same `edges_since(0)`. The recorded list is what `logic.vcd`
/// is written from.
#[test]
fn an_open_edge_log_matches_a_recorded_one() {
    let bytes = std::fs::read(example_path("atmega-analog-serial")).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let stop = document["analysis"]["transient"]["stop_ps"]
        .as_u64()
        .unwrap();
    let input_at = 1_000_000_000;
    let mut opened = Session::open(netlist::parse(&bytes).unwrap()).unwrap();
    let mut recorded = Session::record_document(&bytes).unwrap();
    advance_at(&mut opened, &[400_000_000, input_at]);
    advance_at(&mut recorded, &[input_at]);
    let input = Input {
        part: Name::new("rv1").unwrap(),
        at: Time::from_ps(input_at),
        setting: Setting::Level(Finite::new(0.2).unwrap()),
    };
    let open_landed = opened.apply(&input).unwrap().at;
    let recorded_landed = recorded.apply(&input).unwrap().at;
    assert_eq!(open_landed, recorded_landed);
    advance_at(&mut opened, &[3_000_000_000, 9_000_000_000, stop]);
    advance_at(&mut recorded, &[stop]);
    assert_eq!(opened.status(), Status::Stopped);
    assert_eq!(recorded.status(), Status::Stopped);
    let opened_edges = edge_list(&opened);
    let recorded_edges = edge_list(&recorded);
    assert_eq!(opened_edges, recorded_edges);
    assert!(opened_edges.len() >= 3);
    assert!(
        opened_edges.iter().any(|change| change.at >= open_landed),
        "an edge after the input"
    );
}

fn edges_floor(session: &mut Session, which: &str) {
    let all = edge_list(session);
    let end = all.len() as u64;
    assert!(all.len() >= 3, "{which}");
    let floor = 1u64;
    session.forget_edges(EdgeCursor::from(floor));
    let (tail, cursor) = session
        .edges_since(EdgeCursor::from(floor))
        .expect("floor held");
    assert_eq!(tail, &all[floor as usize..], "{which} edges from {floor}");
    assert_eq!(u64::from(cursor), end, "{which}");
    let between = floor + 1;
    let (tail, cursor) = session
        .edges_since(EdgeCursor::from(between))
        .expect("cursor between the floor and the end");
    assert_eq!(
        tail,
        &all[between as usize..],
        "{which} edges from {between}"
    );
    assert_eq!(u64::from(cursor), end, "{which}");
    let past = end + 1;
    session.forget_edges(EdgeCursor::from(past));
    let (tail, cursor) = session
        .edges_since(EdgeCursor::from(past))
        .expect("cursor past the end");
    assert!(tail.is_empty(), "{which}");
    assert_eq!(u64::from(cursor), past, "{which}");
    let (tail, cursor) = session
        .edges_since(EdgeCursor::from(end))
        .expect("cursor at the end");
    assert!(tail.is_empty(), "{which}");
    assert_eq!(u64::from(cursor), end, "{which}");
    assert_eq!(u64::from(session.edge_cursor()), end, "{which}");
}

fn serial_floor(session: &mut Session, which: &str) {
    let (all, end_cursor) = session
        .serial_since(SerialCursor::default())
        .expect("cursor still held");
    let all = all.to_vec();
    let end = u64::from(end_cursor);
    assert!(all.len() >= 3, "{which}");
    assert_eq!(end, all.len() as u64, "{which}");
    let floor = 1u64;
    session.forget_serial(SerialCursor::from(floor));
    let (tail, cursor) = session
        .serial_since(SerialCursor::from(floor))
        .expect("floor held");
    assert_eq!(tail, &all[floor as usize..], "{which} serial from {floor}");
    assert_eq!(u64::from(cursor), end, "{which}");
    let between = floor + 1;
    let (tail, cursor) = session
        .serial_since(SerialCursor::from(between))
        .expect("cursor between the floor and the end");
    assert_eq!(
        tail,
        &all[between as usize..],
        "{which} serial from {between}"
    );
    assert_eq!(u64::from(cursor), end, "{which}");
    let past = end + 1;
    session.forget_serial(SerialCursor::from(past));
    let (tail, cursor) = session
        .serial_since(SerialCursor::from(past))
        .expect("cursor past the end");
    assert!(tail.is_empty(), "{which}");
    assert_eq!(u64::from(cursor), past, "{which}");
    let (tail, cursor) = session
        .serial_since(SerialCursor::from(end))
        .expect("cursor at the end");
    assert!(tail.is_empty(), "{which}");
    assert_eq!(u64::from(cursor), end, "{which}");
}

/// On a recorded session and on an unrecorded one, `forget_edges(f)` and `forget_serial(f)`
/// leave `[f..]` readable, a cursor between `f` and the end reads that tail, and a forget past
/// the end reads nothing and keeps the cursor.
#[test]
fn a_forget_reads_the_tail_through_the_floor() {
    let bytes = std::fs::read(example_path("atmega-analog-serial")).unwrap();
    for recorded in [true, false] {
        let mut session = if recorded {
            Session::record_document(&bytes).unwrap()
        } else {
            Session::open(netlist::parse(&bytes).unwrap()).unwrap()
        };
        assert_eq!(session.advance_to(Time::from_ps(u64::MAX)), Status::Stopped);
        let which = if recorded { "recorded" } else { "open" };
        edges_floor(&mut session, which);
        serial_floor(&mut session, which);
    }
}

/// Two readers, a forget, and `levels` as the fold of the changes before `edge_cursor`.
#[test]
fn an_edge_cursor_is_the_count_of_changes_read() {
    let mut session = Session::open(example("atmega-blink", 1_000_000_000_000)).unwrap();
    let initial = session.levels().to_vec();
    assert_eq!(session.edge_cursor(), EdgeCursor::default());
    assert_eq!(
        session.edges_since(EdgeCursor::default()),
        Some((&[][..], EdgeCursor::default()))
    );
    session.advance_to(Time::from_ps(300_000_000_000));
    let (early, mid) = session
        .edges_since(EdgeCursor::default())
        .expect("cursor still held");
    let early = early.to_vec();
    assert!(!early.is_empty());
    assert_eq!(u64::from(mid), early.len() as u64);
    let slow = EdgeCursor::default();
    session.forget_edges(mid);
    assert!(session.edges_since(slow).is_none());
    assert_eq!(session.edges_since(mid), Some((&[][..], mid)));
    session.advance_to(Time::from_ps(1_000_000_000_000));
    assert!(session.edges_since(slow).is_none());
    let (later, end) = session.edges_since(mid).expect("cursor still held");
    let later = later.to_vec();
    assert!(!later.is_empty());
    assert!(
        session
            .edges_since(EdgeCursor::from(u64::from(mid) - 1))
            .is_none()
    );
    let (kept, again) = session.edges_since(mid).expect("cursor still held");
    assert_eq!(kept, &later[..]);
    assert_eq!(again, end);
    assert_eq!(session.edge_cursor(), end);
    let mut folded = initial;
    for change in early.iter().chain(&later) {
        folded[change.signal] = change.level;
    }
    assert_eq!(
        session.levels(),
        folded,
        "levels are the fold through edge_cursor"
    );
    let past = EdgeCursor::from(u64::from(end) + 1);
    assert_eq!(session.edges_since(past), Some((&[][..], past)));
    session.forget_edges(past);
    assert!(
        session
            .edges_since(EdgeCursor::from(u64::from(end) - 1))
            .is_none()
    );
    assert_eq!(session.edges_since(end), Some((&[][..], end)));
    assert_eq!(session.edges_since(past), Some((&[][..], past)));
    session.forget_edges(mid);
    assert!(session.edges_since(slow).is_none());
    assert_eq!(session.edge_cursor(), end);
}

/// `forget_serial` and `forget_edges` leave the folder `finish` writes unchanged.
#[test]
fn a_recorded_finish_keeps_what_a_forget_dropped() {
    let stop = 5_000_000_000u64;
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(example_path("atmega-analog-serial")).unwrap())
            .unwrap();
    let transient = &mut json["analysis"]["transient"];
    for key in ["stop_ps", "max_step_ps", "grid_ps"] {
        transient[key] = transient[key].as_u64().unwrap().min(stop).into();
    }
    let bytes = serde_json::to_vec(&json).unwrap();
    let at = Timestamp::from_unix_seconds(0);
    let mut plain = Session::record_document(&bytes).unwrap();
    assert_eq!(plain.advance_to(Time::from_ps(stop)), Status::Stopped);
    let plain = plain.finish(at).unwrap();
    let mut forgotten = Session::record_document(&bytes).unwrap();
    assert_eq!(forgotten.advance_to(Time::from_ps(stop)), Status::Stopped);
    let (serial, _) = forgotten
        .serial_since(SerialCursor::default())
        .expect("cursor still held");
    assert!(serial.len() >= 2);
    let (edges, _) = forgotten
        .edges_since(EdgeCursor::default())
        .expect("cursor still held");
    assert!(!edges.is_empty());
    forgotten.forget_serial(SerialCursor::from(1));
    forgotten.forget_edges(EdgeCursor::from(1));
    assert!(forgotten.serial_since(SerialCursor::default()).is_none());
    assert!(forgotten.edges_since(EdgeCursor::default()).is_none());
    let folder = forgotten.finish(at).unwrap();
    let plain_paths: Vec<String> = plain.files().map(|(path, _)| path.to_string()).collect();
    let folder_paths: Vec<String> = folder.files().map(|(path, _)| path.to_string()).collect();
    assert_eq!(folder_paths, plain_paths);
    for path in &plain_paths {
        assert_eq!(folder.file(path), plain.file(path), "{path}");
    }
}

fn column_integral(session: &Session, reference: &str) -> f64 {
    let probe = session.probe();
    let column = reference.parse().unwrap();
    let index = probe
        .refs()
        .iter()
        .position(|r| *r == column)
        .unwrap_or_else(|| panic!("{reference} is not a column"));
    probe.integrals()[index]
}

fn relative(got: f64, expect: f64) -> f64 {
    (got - expect).abs() / expect.abs()
}

#[test]
fn a_divider_integrates_its_voltage() {
    let stop = 1_000_000_000u64;
    let json = transient(
        stop,
        stop,
        r#""gnd","vin","mid""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "mid", 1000.0),
            &resistor("r2", "mid", "gnd", 1000.0),
        ],
    );
    let mut session = opened(&json);
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    let vin = column_integral(&session, "#net.vin");
    let vin_expect = 5.0 * stop as f64;
    let vin_error = relative(vin, vin_expect);
    assert!(
        vin_error <= 1e-12,
        "{vin} vs {vin_expect}, relative {vin_error}"
    );
    let mid_v = session
        .probe()
        .get(&"#net.mid".parse().unwrap())
        .unwrap()
        .get();
    let mid = column_integral(&session, "#net.mid");
    let mid_expect = mid_v * stop as f64;
    let mid_error = relative(mid, mid_expect);
    assert!(
        mid_error <= 1e-12,
        "{mid} vs {mid_expect}, relative {mid_error}"
    );
}

/// Accepted steps only. A reading between steps can be larger than the integral at the next one.
#[test]
fn a_non_negative_divider_integral_never_falls_on_accepted_steps() {
    let stop = 10_000_000u64;
    let json = transient(
        stop,
        1_000_000,
        r#""gnd","vin","mid""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "mid", 1000.0),
            &resistor("r2", "mid", "gnd", 1000.0),
        ],
    );
    let steps = common::simulate_steps(&json);
    let times = trace(&steps).time();
    assert!(times.len() >= 3, "{} accepted steps", times.len());
    let mut session = opened(&json);
    let vin_ref = "#net.vin".parse().unwrap();
    let mut previous = 0.0;
    for &at in times {
        let status = session.advance_to(at);
        assert_eq!(session.now(), at);
        assert_eq!(
            status,
            if at.as_ps() == stop {
                Status::Stopped
            } else {
                Status::Running
            }
        );
        let vin = session.probe().get(&vin_ref).unwrap().get();
        assert!(vin >= 0.0, "{vin} at {}", at.as_ps());
        let integral = column_integral(&session, "#net.vin");
        assert!(
            integral >= previous,
            "{integral} < {previous} at {}",
            at.as_ps()
        );
        previous = integral;
    }
    assert!(previous > 0.0);
}

fn closing_switch(edge: u64, ohm: f64) -> String {
    format!(
        r#"{{ "name": "sw1", "kind": "switch", "pins": {{ "a": "vin", "b": "vout" }},
          "on_resistance": {{ "value": {ohm:?}, "unit": "ohm", "source": "user" }},
          "off_resistance": {{ "value": 1e9, "unit": "ohm", "source": "user" }},
          "initial": "open", "events": [{{ "t_ps": {edge}, "state": "closed" }}], "bounce": null }}"#
    )
}

#[test]
fn a_switched_resistor_integrates_its_on_current() {
    let edge = 1_000_000u64;
    let stop = 1_000_000_000u64;
    let on_ohm = 0.1;
    let load = 1000.0;
    let json = transient(
        stop,
        stop,
        r#""gnd","vin","vout""#,
        &[
            &source_on("v1", "vin", 5.0),
            &closing_switch(edge, on_ohm),
            &resistor("r1", "vout", "gnd", load),
        ],
    );
    let mut session = opened(&json);
    let i_on = 5.0 / (on_ohm + load);
    let i_off = 5.0 / (1.0e9 + load);
    // 500 ps sits in the 1 ns step after the edge, so the reading includes the partial trapezoid.
    let inside = 500u64;
    let at = edge + inside;
    assert_eq!(session.advance_to(Time::from_ps(at)), Status::Running);
    assert_eq!(session.now().as_ps(), at);
    let early = i_off * edge as f64 + i_on * inside as f64;
    let early_got = column_integral(&session, "#r1.i");
    let early_error = relative(early_got, early);
    assert!(
        early_error <= 1e-6,
        "inside the step after the edge, {early_got} vs {early}, relative {early_error}"
    );
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    let expect = i_off * edge as f64 + i_on * (stop - edge) as f64;
    let got = column_integral(&session, "#r1.i");
    let error = relative(got, expect);
    assert!(error <= 1e-9, "{got} vs {expect}, relative {error}");
}

#[test]
fn a_capacitor_integrates_its_charge() {
    let stop = 1_000_000_000u64;
    let farad = 1e-6;
    let json = transient(
        stop,
        stop,
        r#""gnd","vin","vout""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "vout", 1000.0),
            &held_capacitor("c1", "vout", "gnd", farad, 0.0),
        ],
    );
    let mut session = opened(&json);
    let vout = "#net.vout".parse().unwrap();
    let v0 = session.probe().get(&vout).unwrap().get();
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    let v1 = session.probe().get(&vout).unwrap().get();
    let expect = farad * (v1 - v0) * 1.0e12;
    let got = column_integral(&session, "#c1.i");
    let error = relative(got, expect);
    assert!(error <= 1e-4, "{got} vs {expect}, relative {error}");
}

const LED: &str = r#"{
  "name": "led1",
  "kind": "led",
  "pins": { "anode": "vled", "cathode": "gnd" },
  "saturation_current": { "value": 6.711e-10, "unit": "A", "source": "user" },
  "emission_coefficient": { "value": 4.427, "unit": "1", "source": "user" },
  "series_resistance": { "value": 2.115, "unit": "ohm", "source": "user" },
  "ratings": {
    "current": { "value": 0.03, "unit": "A", "source": "user" },
    "power": { "value": 0.075, "unit": "W", "source": "user" }
  }
}"#;

fn session_warnings(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity() == Severity::Warning
                && diagnostic.code() != "trace.row_interpolated"
        })
        .cloned()
        .collect()
}

fn kept(diagnostic: &Diagnostic) -> (String, Vec<Ref>, Option<u64>) {
    (
        diagnostic.code().to_string(),
        diagnostic.refs(),
        diagnostic.at().map(Time::as_ps),
    )
}

fn rating_at(diagnostics: &[Diagnostic], code: &str) -> Option<u64> {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == code)
        .and_then(Diagnostic::at)
        .map(Time::as_ps)
}

fn peak(diagnostics: &[Diagnostic], code: &str) -> f64 {
    let found = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == code)
        .unwrap_or_else(|| panic!("{code} is absent"));
    serde_json::to_value(found).unwrap()["data"]["peak"]
        .as_f64()
        .unwrap()
}

/// Warnings in `diagnostics.json` other than `trace.row_interpolated` and the fault.
fn folder_warnings(which: &str, bytes: &[u8], blobs: &Blobs, stop: Time) -> Vec<Diagnostic> {
    let netlist = netlist::parse_with(bytes, blobs).unwrap();
    let results = sim_circuit::simulate(&netlist);
    let from_run = session_warnings(results.diagnostics());
    drop(results);
    let mut opened = Session::open_document_with(bytes, blobs).expect(which);
    opened.advance_to(stop);
    let open_found = opened.diagnostics();
    drop(opened);
    let mut recorded = Session::record_document_with(bytes, blobs).expect(which);
    recorded.advance_to(stop);
    let recorded_found = recorded.diagnostics();
    let folder = recorded
        .finish(Timestamp::from_unix_seconds(0))
        .expect(which);
    let parsed: serde_json::Value =
        serde_json::from_slice(folder.file("diagnostics.json").unwrap()).unwrap();
    let from_file: Vec<serde_json::Value> = parsed
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["severity"] == "warning" && item["code"] != "trace.row_interpolated")
        .cloned()
        .collect();
    assert_eq!(open_found, from_run, "{which} open");
    assert_eq!(recorded_found, from_run, "{which} recorded");
    assert_eq!(
        serde_json::to_value(&from_run).unwrap(),
        serde_json::Value::Array(from_file),
        "{which} diagnostics.json"
    );
    from_run
}

#[test]
fn a_floating_net_is_listed_when_the_session_opens() {
    let json = transient(
        1_000_000,
        1_000_000,
        r#""gnd","vin","lonely""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "gnd", 1000.0),
            r#"{ "name": "c1", "kind": "capacitor", "pins": { "a": "lonely", "b": "gnd" },
              "capacitance": { "value": 1e-9, "unit": "F", "source": "user" }, "initial": null }"#,
        ],
    );
    let bytes = json.as_bytes();
    let mut session = opened(&json);
    let at_open = session.diagnostics();
    assert_eq!(at_open.len(), 1, "{at_open:?}");
    assert_eq!(at_open[0].code(), "net.floating");
    assert_eq!(at_open[0].at(), None);
    assert_eq!(at_open[0].refs(), vec!["#net.lonely".parse().unwrap()]);
    assert_eq!(
        session.advance_to(Time::from_ps(1_000_000)),
        Status::Stopped
    );
    assert_eq!(session.diagnostics(), at_open);
    assert_eq!(
        folder_warnings("floating", bytes, &Blobs::new(), Time::from_ps(1_000_000)),
        at_open
    );
}

/// 5 V pulses, 0.5 ms wide every 20 ms, through 22 ohm into a 30 mA LED.
/// Three 60 Hz frames. The pulses are over the rating and the mean of a frame is under it.
fn pulsed_led() -> String {
    let stop = 3 * FRAME_PS;
    let step = 1_000_000_000u64;
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": {stop}, "max_step_ps": {step}, "grid_ps": {step} }} }},
      "ground": "gnd",
      "nets": ["gnd", "vin", "vled"],
      "parts": [
        {{
          "name": "v1",
          "kind": "voltage_source",
          "pins": {{ "p": "vin", "n": "gnd" }},
          "voltage": {{
            "kind": "pulse",
            "initial": {{ "value": 0.0, "unit": "V", "source": "user" }},
            "pulsed": {{ "value": 5.0, "unit": "V", "source": "user" }},
            "delay_ps": 2000000000,
            "width_ps": 500000000,
            "period_ps": 20000000000
          }}
        }},
        {{
          "name": "r1",
          "kind": "resistor",
          "pins": {{ "a": "vin", "b": "vled" }},
          "resistance": {{ "value": 22.0, "unit": "ohm", "source": "user" }},
          "ratings": {{ "power": null }}
        }},
        {LED}
      ]
    }}"#
    )
}

#[test]
fn a_pulsed_over_current_is_the_same_for_one_call_frames_and_1_ps_calls_around_the_crossing() {
    let json = pulsed_led();
    let bytes = json.as_bytes();
    let stop = Time::from_ps(3 * FRAME_PS);
    let mut once = opened(&json);
    assert!(
        once.diagnostics()
            .iter()
            .all(|d| d.code() != "rating.current")
    );
    assert_eq!(once.advance_to(stop), Status::Stopped);
    let found = once.diagnostics();
    assert_eq!(once.advance_to(stop), Status::Stopped);
    assert_eq!(once.diagnostics(), found);
    let crossing = rating_at(&found, "rating.current").expect("rating.current");
    assert!(
        (2_000_000_000..3_000_000_000).contains(&crossing),
        "first over at {crossing} ps"
    );
    assert!(peak(&found, "rating.current") > 0.03);

    let mut frames = opened(&json);
    assert_eq!(frames.advance_to(Time::from_ps(FRAME_PS)), Status::Running);
    let mean = column_integral(&frames, "#led1.i") / FRAME_PS as f64;
    assert!(mean < 0.03, "mean over the first frame is {mean} A");
    assert_eq!(
        rating_at(&frames.diagnostics(), "rating.current"),
        Some(crossing)
    );
    assert_eq!(
        frames.advance_to(Time::from_ps(2 * FRAME_PS)),
        Status::Running
    );
    assert_eq!(frames.advance_to(stop), Status::Stopped);
    assert_eq!(frames.diagnostics(), found);

    let mut fine = opened(&json);
    assert_eq!(
        fine.advance_to(Time::from_ps(crossing - 1)),
        Status::Running
    );
    assert!(
        fine.diagnostics()
            .iter()
            .all(|d| d.code() != "rating.current"),
        "{:?}",
        fine.diagnostics()
    );
    for ps in crossing..=crossing + 4 {
        fine.advance_to(Time::from_ps(ps));
    }
    assert_eq!(
        rating_at(&fine.diagnostics(), "rating.current"),
        Some(crossing)
    );
    assert_eq!(fine.advance_to(stop), Status::Stopped);
    assert_eq!(fine.diagnostics(), found);
    assert_eq!(
        folder_warnings("pulsed-led", bytes, &Blobs::new(), stop),
        found
    );
}

#[test]
fn an_input_keeps_a_rating_and_its_first_time() {
    let stop = 5_000_000u64;
    let json = transient(
        stop,
        1_000_000,
        r#""gnd","vin","vled""#,
        &[
            &source_on("v1", "vin", 5.0),
            &resistor("r1", "vin", "vled", 22.0),
            LED,
        ],
    );
    let mut session = opened(&json);
    let before = session.diagnostics();
    let crossing = rating_at(&before, "rating.current").expect("over at t = 0");
    assert_eq!(crossing, 0);
    assert!(peak(&before, "rating.current") > 0.03);
    assert_eq!(
        session.advance_to(Time::from_ps(1_000_000)),
        Status::Running
    );
    let running = session.diagnostics();
    for stamp in before.iter().map(kept) {
        assert!(running.iter().map(kept).any(|later| later == stamp));
    }
    session
        .apply(&Input {
            part: Name::new("v1").unwrap(),
            at: session.now(),
            setting: Setting::Level(Finite::new(0.0).unwrap()),
        })
        .unwrap();
    let landed = session.diagnostics();
    for stamp in running.iter().map(kept) {
        assert!(
            landed.iter().map(kept).any(|later| later == stamp),
            "lost {stamp:?} after the input"
        );
    }
    assert_eq!(session.advance_to(Time::from_ps(stop)), Status::Stopped);
    let ended = session.diagnostics();
    for stamp in landed.iter().map(kept) {
        assert!(ended.iter().map(kept).any(|later| later == stamp));
    }
    assert_eq!(rating_at(&ended, "rating.current"), Some(0));
}

fn diagnostic_fixture(name: &str) -> Vec<u8> {
    fs::read(format!(
        "{}/tests/diagnostics/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// `examples/led-9v` run as a 1 us transient.
fn nine_volt_led_transient() -> Vec<u8> {
    let path = format!(
        "{}/../../examples/led-9v/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    json["analysis"] = serde_json::json!({
        "transient": { "stop_ps": 1_000_000, "max_step_ps": 1_000_000, "grid_ps": 1_000_000 }
    });
    serde_json::to_vec(&json).unwrap()
}

fn notes_reporting(name: &str, found: &[Diagnostic], reporting: &mut Vec<String>) {
    if found
        .iter()
        .any(|d| d.code() == "net.floating" || d.code().starts_with("rating."))
    {
        let codes: Vec<_> = found.iter().map(Diagnostic::code).collect();
        eprintln!("reports {name}: {}", codes.join(" "));
        reporting.push(name.to_string());
    }
}

#[test]
fn diagnostics_at_the_stop_match_the_folder() {
    let mut reporting = Vec::new();
    let mut transient_rating = false;
    let mut transient_floating = false;
    let mut mark_transient = |found: &[Diagnostic]| {
        for diagnostic in found {
            if diagnostic.code().starts_with("rating.") {
                transient_rating = true;
            }
            if diagnostic.code() == "net.floating" {
                transient_floating = true;
            }
        }
    };
    for example in common::examples::examples() {
        let name = example.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(example.join("netlist.json")).unwrap();
        let blobs = common::examples::blobs(&example);
        let netlist = netlist::parse_with(&bytes, &blobs).unwrap();
        match netlist.analysis() {
            Analysis::Op => {
                assert!(
                    matches!(Session::open_document(&bytes), Err(Refused::Op)),
                    "{name}"
                );
                assert!(
                    matches!(Session::record_document(&bytes), Err(Refused::Op)),
                    "{name}"
                );
                let results = sim_circuit::simulate(&netlist);
                notes_reporting(&name, results.diagnostics(), &mut reporting);
            }
            Analysis::Transient(transient) => {
                let stop = transient.stop;
                drop(netlist);
                let found = folder_warnings(&name, &bytes, &blobs, stop);
                mark_transient(&found);
                notes_reporting(&name, &found, &mut reporting);
            }
        }
    }
    let press = 1_000_000u64;
    let fixtures = [
        ("button-22", diagnostic_fixture("button-22")),
        ("pwm-led", diagnostic_fixture("pwm-led")),
        ("floating-net", diagnostic_fixture("floating-net")),
        ("led-9v-transient", nine_volt_led_transient()),
    ];
    for (name, bytes) in fixtures {
        let netlist = netlist::parse(&bytes).unwrap();
        let Analysis::Transient(transient) = netlist.analysis() else {
            panic!("{name} is not a transient");
        };
        let stop = transient.stop;
        drop(netlist);
        let found = folder_warnings(name, &bytes, &Blobs::new(), stop);
        mark_transient(&found);
        notes_reporting(name, &found, &mut reporting);
        match name {
            "button-22" => assert_eq!(
                rating_at(&found, "rating.current"),
                Some(press + 1_000),
                "{name} first over {found:?}"
            ),
            "pwm-led" | "led-9v-transient" => assert!(
                found.iter().any(|d| d.code().starts_with("rating.")),
                "{name} {found:?}"
            ),
            "floating-net" => assert!(
                found.iter().any(|d| d.code() == "net.floating"),
                "{name} {found:?}"
            ),
            _ => {}
        }
    }
    assert!(
        reporting.iter().any(|name| name == "led-9v"),
        "reporting examples: {reporting:?}"
    );
    assert!(
        transient_rating,
        "no transient reported a rating: {reporting:?}"
    );
    assert!(
        transient_floating,
        "no transient reported net.floating: {reporting:?}"
    );
}

#[test]
fn a_nine_volt_led_transient_matches_its_folder() {
    let bytes = nine_volt_led_transient();
    let found = folder_warnings(
        "led-9v-transient",
        &bytes,
        &Blobs::new(),
        Time::from_ps(1_000_000),
    );
    let codes: Vec<_> = found.iter().map(Diagnostic::code).collect();
    assert!(codes.contains(&"rating.current"), "{codes:?}");
    assert!(codes.contains(&"rating.power"), "{codes:?}");
    assert!(found.iter().all(|d| d.code() != "trace.row_interpolated"));
    assert!(found.iter().all(|d| d.severity() == Severity::Warning));
}
