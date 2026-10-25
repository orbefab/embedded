//! The binding's methods, called natively. `tests/worker.mjs` runs the same methods in a Worker.

use std::fs;

use sim_circuit::{Input, Serial as SerialSend, Setting};
use sim_core::finite::Finite;
use sim_core::logic::Level;
use sim_core::name::Name;
use sim_core::netlist::{Baud, Contact, Frame};
use sim_core::time::Time;
use sim_wasm::{Code, Folder, Session, SimError, Status, run};

fn example(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../../examples/{name}/netlist.json",
        env!("CARGO_MANIFEST_DIR")
    );
    fs::read(path).unwrap()
}

fn refused(result: Result<Session, SimError>) -> SimError {
    result.err().expect("the document is refused")
}

fn rejected<T>(result: Result<T, SimError>) -> SimError {
    result.err().expect("the call is rejected")
}

fn manifest(folder: &Folder) -> serde_json::Value {
    serde_json::from_slice(&folder.file("manifest.json").unwrap().unwrap()).unwrap()
}

/// The binding's `Status` for the session's, matched here rather than through the binding.
fn status(status: sim_circuit::Status) -> Status {
    match status {
        sim_circuit::Status::Running => Status::Running,
        sim_circuit::Status::Stopped => Status::Stopped,
        sim_circuit::Status::Faulted => Status::Faulted,
    }
}

#[test]
fn a_session_reports_running_then_stopped() {
    let json = example("v-steps");
    let mut session = Session::open(&json).unwrap();
    assert_eq!(session.status(), Status::Running);
    assert_eq!(session.advance_to(1).unwrap(), Status::Running);
    assert_eq!(session.now(), 1);
    assert_eq!(session.advance_to(u64::MAX).unwrap(), Status::Stopped);
    assert_eq!(session.now(), 20_000_000);
    assert_eq!(session.fault(), None);
}

#[test]
fn a_fault_crosses_as_its_diagnostic_json() {
    let mut json: serde_json::Value = serde_json::from_slice(&example("v-steps")).unwrap();
    json["parts"][0]["voltage"]["events"][1]["level"]["value"] = 1e200.into();
    let mut session = Session::open(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(session.advance_to(u64::MAX).unwrap(), Status::Faulted);
    let fault: serde_json::Value = serde_json::from_str(&session.fault().unwrap()).unwrap();
    assert_eq!(fault["code"], "solver.nonfinite");
}

#[test]
fn refused_documents_throw_their_code_and_diagnostics() {
    let op = refused(Session::record(&example("divider")));
    assert_eq!((op.code, op.diagnostics.as_str()), (Code::Op, "[]"));

    let invalid = refused(Session::open(b"{}"));
    assert_eq!(invalid.code, Code::Invalid);
    let diagnostics: serde_json::Value = serde_json::from_str(&invalid.diagnostics).unwrap();
    assert!(!diagnostics.as_array().unwrap().is_empty());

    let mut json: serde_json::Value = serde_json::from_slice(&example("v-steps")).unwrap();
    json["analysis"]["transient"]["stop_ps"] = 2_000_000_000.into();
    json["analysis"]["transient"]["grid_ps"] = 1_000.into();
    let failed = refused(Session::record(&serde_json::to_vec(&json).unwrap()));
    assert_eq!(failed.code, Code::Failed);
    let diagnostics: serde_json::Value = serde_json::from_str(&failed.diagnostics).unwrap();
    assert_eq!(diagnostics[0]["code"], "trace.too_large");
}

#[test]
fn rejected_inputs_throw_their_code() {
    let mut session = Session::open(&example("atmega-analog-serial")).unwrap();
    let value = rejected(session.apply_level("rv1", 0, 2.0));
    assert_eq!(value.code, Code::Value);
    assert_eq!(value.rule.as_deref(), Some("fraction"));
    let cases = [
        (
            rejected(session.apply_level("rv9", 0, 0.5)),
            Code::UnknownPart,
        ),
        (
            rejected(session.apply_level("cx1", 0, 0.5)),
            Code::NotSettable,
        ),
        (
            rejected(session.apply_contact("rv1", 0, true)),
            Code::WrongSetting,
        ),
        (
            rejected(session.apply_level("rv1", u64::MAX, 0.25)),
            Code::PastStop,
        ),
        (rejected(session.apply_level("1rv", 0, 0.5)), Code::BadName),
        (rejected(session.apply_level("rv1 ", 0, 0.5)), Code::BadName),
        (rejected(session.apply_contact("", 0, true)), Code::BadName),
        (
            rejected(session.apply_level("rv1", 0, f64::NAN)),
            Code::NotFinite,
        ),
    ];
    for (error, code) in cases {
        assert_eq!((error.code, error.rule), (code, None));
    }
    let applied = session.apply_level("rv1", 0, 0.25).unwrap();
    assert_eq!(applied.status, Status::Running);
    assert!(applied.at > 0);
    let same = rejected(session.apply_level("rv1", 0, 0.25));
    assert_eq!(same.code, Code::NoChange);
    session.advance_to(u64::MAX).unwrap();
    let ended = rejected(session.apply_level("rv1", 0, 0.75));
    assert_eq!(ended.code, Code::Ended);
}

#[test]
fn the_serial_cursor_counts_bytes_read() {
    let mut session = Session::open(&example("atmega-analog-serial")).unwrap();
    assert_eq!(session.usarts(), r#"[["u1","USART0"]]"#);
    session.advance_to(u64::MAX).unwrap();
    let all = session.serial_since(0).unwrap();
    assert_eq!(all.byte, b"512\r\n512\r\n512\r\n");
    assert_eq!(all.cursor, 15);
    assert_eq!(all.usart, [0; 15]);
    let tail = session.serial_since(13).unwrap();
    assert_eq!(
        (tail.at.as_slice(), tail.byte.as_slice()),
        (&all.at[13..], &b"\r\n"[..])
    );
    assert_eq!(tail.cursor, 15);
    let past = session.serial_since(99).unwrap();
    assert!(past.byte.is_empty());
    assert_eq!(past.cursor, 99);
}

#[test]
fn finish_on_an_unrecorded_session_throws_unrecorded() {
    let session = Session::open(&example("v-steps")).unwrap();
    assert_eq!(rejected(session.finish(0)).code, Code::Unrecorded);
}

#[test]
fn finish_and_run_stamp_the_seconds_they_are_given() {
    let json = example("v-steps");
    let session = Session::record(&json).unwrap();
    let finished = session.finish(1_700_000_000).unwrap();
    let ran = run(&json, 1_700_000_000).unwrap();
    for folder in [&finished, &ran] {
        assert_eq!(manifest(folder)["created_at"], "2023-11-14T22:13:20Z");
        assert_eq!(folder.file("netlist.json"), Ok(Some(json.clone())));
    }
    assert_eq!(finished.paths(), ran.paths());
    assert!(finished.paths().contains(&"traces/analog.f64".to_string()));
    assert_eq!(
        finished.file("traces/analog.f64"),
        ran.file("traces/analog.f64")
    );
    assert_eq!(ran.file("nothing"), Ok(None));
}

#[test]
fn run_writes_a_folder_for_a_rejected_netlist() {
    let folder = run(b"{}", 0).unwrap();
    assert_eq!(folder.file("netlist.json"), Ok(Some(b"{}".to_vec())));
    let diagnostics: serde_json::Value =
        serde_json::from_slice(&folder.file("diagnostics.json").unwrap().unwrap()).unwrap();
    assert!(!diagnostics.as_array().unwrap().is_empty());
    assert_eq!(manifest(&folder)["created_at"], "1970-01-01T00:00:00Z");
}

#[test]
fn a_probe_reads_its_columns_in_refs_order() {
    let session = Session::open(&example("v-steps")).unwrap();
    let probe = session.probe();
    assert_eq!(probe.at, 0);
    assert_eq!(probe.values.len(), session.refs().len());
    assert_eq!(probe.integrals.len(), session.refs().len());
    assert!(probe.integrals.iter().all(|value| *value == 0.0));
    assert_eq!(session.refs()[0], "#net.gnd");
    assert_eq!(probe.values[0], 0.0);
    assert!(!probe.interpolated);
    assert!(session.signals().is_empty());
    assert!(session.levels().is_empty());
}

/// The binding's reads at `now` are those of the session it wraps, opened apart on the same
/// netlist. Returns the levels and whether the probe was the line.
fn assert_same_reads(binding: &Session, inner: &sim_circuit::Session) -> (Vec<u8>, bool) {
    let at = inner.now().as_ps();
    assert_eq!(binding.now(), at);
    assert_eq!(
        binding.status(),
        status(inner.status()),
        "status at {at} ps"
    );
    let (probe, want) = (binding.probe(), inner.probe());
    assert_eq!(probe.at, want.at().as_ps());
    let bits: Vec<u64> = probe.values.iter().map(|v| v.to_bits()).collect();
    let want_bits: Vec<u64> = want.values().iter().map(|v| v.get().to_bits()).collect();
    assert_eq!(bits, want_bits, "probe values at {at} ps");
    let integrals: Vec<u64> = probe.integrals.iter().map(|v| v.to_bits()).collect();
    let want_integrals: Vec<u64> = want.integrals().iter().map(|v| v.to_bits()).collect();
    assert_eq!(integrals, want_integrals, "integrals at {at} ps");
    assert_eq!(
        probe.interpolated,
        want.interpolated(),
        "interpolated at {at} ps"
    );
    let signals: Vec<String> = inner.signals().iter().map(|r| r.to_string()).collect();
    assert_eq!(binding.signals(), signals);
    let levels: Vec<u8> = inner
        .levels()
        .iter()
        .map(|level| match level {
            Level::Low => 0,
            Level::High => 1,
        })
        .collect();
    assert_eq!(binding.levels(), levels, "levels at {at} ps");
    (levels, probe.interpolated)
}

/// On a step, off one, on a row and between pin edges, through three LED toggles.
const BLINK_PS: [u64; 10] = [
    1,
    250_000,
    250_001,
    10_000_000_000,
    123_456_789_011,
    500_000_250_000,
    500_000_250_001,
    777_777_777_777,
    1_000_000_250_000,
    1_200_000_000_000,
];

#[test]
fn the_reads_are_the_sessions_on_atmega_blink() {
    let json = example("atmega-blink");
    let mut binding = Session::open(&json).unwrap();
    let mut inner = sim_circuit::Session::open_document(&json).unwrap();
    let mut seen = vec![assert_same_reads(&binding, &inner).0];
    for ps in BLINK_PS {
        binding.advance_to(ps).unwrap();
        inner.advance_to(Time::from_ps(ps));
        seen.push(assert_same_reads(&binding, &inner).0);
    }
    assert!(seen.iter().flatten().any(|level| *level == 0));
    assert!(seen.iter().flatten().any(|level| *level == 1));
    seen.dedup();
    assert!(seen.len() > 2, "a pin moved twice: {seen:?}");
}

#[test]
fn an_input_lands_where_the_session_lands_it() {
    let json = example("atmega-button");
    let mut binding = Session::open(&json).unwrap();
    let mut inner = sim_circuit::Session::open_document(&json).unwrap();
    let mut later = 0;
    for (k, ps) in [0, 1, 40_000_000_000, 400_000_000_000, 400_000_000_001]
        .into_iter()
        .enumerate()
    {
        binding.advance_to(ps).unwrap();
        inner.advance_to(Time::from_ps(ps));
        let closed = k % 2 == 0;
        let applied = binding.apply_contact("sw1", ps, closed).unwrap();
        let contact = if closed {
            Contact::Closed
        } else {
            Contact::Open
        };
        let want = inner
            .apply(&Input {
                part: Name::new("sw1").unwrap(),
                at: Time::from_ps(ps),
                setting: Setting::Contact(contact),
            })
            .unwrap();
        assert_eq!(applied.at, want.at.as_ps(), "the input at {ps} ps");
        assert_eq!(applied.status, status(want.status));
        later += usize::from(applied.at > ps);
        assert_same_reads(&binding, &inner);
    }
    assert!(later > 0, "an input landed after the time asked for");
}

/// With Newton's row budget at 1, most probes between steps fail twice and are the line.
#[test]
fn an_interpolated_probe_says_so() {
    let json = example("v-pulse-rc");
    let open = || sim_circuit::Session::open_document_with_row_budget(&json, 1).unwrap();
    let Ok(mut binding) = Session::try_from(open()) else {
        panic!("a session opened from a document is taken");
    };
    let mut inner = open();
    let probes = 40;
    let mut interpolated = 0;
    for k in 1..=probes {
        let ps = k * 34_146_347;
        binding.advance_to(ps).unwrap();
        inner.advance_to(Time::from_ps(ps));
        interpolated += u64::from(assert_same_reads(&binding, &inner).1);
    }
    assert!(
        interpolated > 0 && interpolated < probes,
        "{interpolated} of {probes} probes interpolated"
    );
}

#[test]
fn a_session_with_no_document_is_handed_back() {
    let netlist = sim_core::netlist::parse(&example("v-steps")).unwrap();
    let inner = sim_circuit::Session::open(netlist).unwrap();
    assert!(Session::try_from(inner).is_err());
}

fn level(inner: &mut sim_circuit::Session, part: &str, at: u64, si: f64) -> sim_circuit::Applied {
    inner
        .apply(&Input {
            part: Name::new(part).unwrap(),
            at: Time::from_ps(at),
            setting: Setting::Level(Finite::new(si).unwrap()),
        })
        .unwrap()
}

/// The export holds each position as the number passed, as the session's own export does.
#[test]
fn a_level_is_stored_as_the_number_passed() {
    let json = example("atmega-analog-serial");
    let mut binding = Session::open(&json).unwrap();
    let mut inner = sim_circuit::Session::open_document(&json).unwrap();
    for (ps, si) in [(0, 0.25), (3_000_000_000, 0.1), (9_000_000_000, 0.875)] {
        binding.advance_to(ps).unwrap();
        inner.advance_to(Time::from_ps(ps));
        let applied = binding.apply_level("rv1", ps, si).unwrap();
        let want = level(&mut inner, "rv1", ps, si);
        assert_eq!(
            (applied.at, applied.status),
            (want.at.as_ps(), status(want.status))
        );
        assert_eq!(
            binding.export(),
            inner.export().unwrap(),
            "the export after {si}"
        );
        assert_same_reads(&binding, &inner);
    }
}

/// An input that makes the solve fail returns `faulted`, as the session's own `Applied` says.
#[test]
fn an_input_that_faults_says_so() {
    let json = example("v-steps");
    let mut binding = Session::open(&json).unwrap();
    let mut inner = sim_circuit::Session::open_document(&json).unwrap();
    let ps = 10_050_000;
    binding.advance_to(ps).unwrap();
    inner.advance_to(Time::from_ps(ps));
    let applied = binding.apply_level("v1", ps, 1e200).unwrap();
    let want = level(&mut inner, "v1", ps, 1e200);
    assert_eq!(
        (applied.at, applied.status),
        (want.at.as_ps(), status(want.status))
    );
    assert_eq!(applied.status, Status::Faulted);
    assert_same_reads(&binding, &inner);
    let fault = inner.fault().map(|d| serde_json::to_string(d).unwrap());
    assert_eq!(binding.fault(), fault);
}

/// `atmega-analog-serial` with a second chip, `u2`, on its own nets but the supply, ground and
/// the wiper, so both USARTs send.
fn two_usarts() -> Vec<u8> {
    let mut json: serde_json::Value =
        serde_json::from_slice(&example("atmega-analog-serial")).unwrap();
    let parts = json["parts"].as_array_mut().unwrap();
    let mut u2 = parts.iter().find(|p| p["name"] == "u1").unwrap().clone();
    u2["name"] = "u2".into();
    let mut nets = Vec::new();
    for net in u2["pins"].as_object_mut().unwrap().values_mut() {
        if !["vcc", "gnd", "pc0"].contains(&net.as_str().unwrap()) {
            *net = format!("{}_2", net.as_str().unwrap()).into();
            nets.push(net.clone());
        }
    }
    parts.push(u2);
    json["nets"].as_array_mut().unwrap().extend(nets);
    serde_json::to_vec(&json).unwrap()
}

#[test]
fn each_byte_names_its_usart() {
    let json = two_usarts();
    let mut binding = Session::open(&json).unwrap();
    let mut inner = sim_circuit::Session::open_document(&json).unwrap();
    assert_eq!(binding.usarts(), r#"[["u1","USART0"],["u2","USART0"]]"#);
    binding.advance_to(u64::MAX).unwrap();
    inner.advance_to(Time::from_ps(u64::MAX));
    let serial = binding.serial_since(0).unwrap();
    let (want, cursor) = inner.serial_since(0u64.into());
    assert_eq!(
        serial.usart,
        want.iter().map(|b| b.usart).collect::<Vec<_>>()
    );
    assert_eq!(serial.cursor, u64::from(cursor));
    for usart in [0, 1] {
        assert_eq!(serial.usart.iter().filter(|u| **u == usart).count(), 15);
    }
}

fn serial_send(bytes: &[u8], baud: u32) -> Setting {
    Setting::Serial(SerialSend {
        baud: Baud::new(baud).unwrap(),
        frame: Frame::EightN1,
        bytes: bytes.to_vec(),
    })
}

/// `applySerial` lands where the session lands the same send, and the exports match.
#[test]
fn a_serial_send_lands_where_the_session_lands_it() {
    let json = example("v-serial");
    let mut binding = Session::open(&json).unwrap();
    let mut inner = sim_circuit::Session::open_document(&json).unwrap();
    let bytes = [0x32, 0x33];
    let applied = binding.apply_serial("v1", 0, 115_200.0, &bytes).unwrap();
    let want = inner
        .apply(&Input {
            part: Name::new("v1").unwrap(),
            at: Time::from_ps(0),
            setting: serial_send(&bytes, 115_200),
        })
        .unwrap();
    assert_eq!(
        (applied.at, applied.status),
        (want.at.as_ps(), status(want.status))
    );
    assert_eq!(binding.export(), inner.export().unwrap());
    assert_same_reads(&binding, &inner);
}

#[test]
fn a_bad_baud_throws_bad_baud() {
    let mut session = Session::open(&example("v-serial")).unwrap();
    let before = session.export();
    for rate in [0.0, 0.5, 1.5, -1.0, 500_000_001.0, f64::NAN, f64::INFINITY] {
        let error = rejected(session.apply_serial("v1", 0, rate, &[0x31]));
        assert_eq!((error.code, error.rule), (Code::BadBaud, None), "{rate}");
    }
    assert_eq!(session.export(), before);
}

#[test]
fn bytes_on_a_steps_source_throw_wrong_setting() {
    let mut session = Session::open(&example("v-steps")).unwrap();
    let before = session.export();
    let error = rejected(session.apply_serial("v1", 0, 9600.0, &[0x31]));
    assert_eq!(error.code, Code::WrongSetting);
    assert_eq!(session.export(), before);
}

#[test]
fn an_empty_serial_send_throws_no_change() {
    let mut session = Session::open(&example("v-serial")).unwrap();
    let before = session.export();
    let error = rejected(session.apply_serial("v1", 0, 9600.0, &[]));
    assert_eq!(error.code, Code::NoChange);
    assert_eq!(session.export(), before);
}
