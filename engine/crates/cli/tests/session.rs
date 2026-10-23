//! A recorded session, advanced in seeded chunks and finished, writes the run folder `sim run`
//! writes. It uses only the public items a binding calls: `sim_cli::run` and `Session`.

mod common;

use std::fs;
use std::path::Path;

use common::examples::examples;
use sim_circuit::{Input, Refused, Rejected, SerialCursor, Session, Setting, Status};
use sim_core::diagnostic::ValueRule;
use sim_core::finite::Finite;
use sim_core::name::Name;
use sim_core::netlist::{self, Analysis, Contact};
use sim_core::run_folder::RunFolder;
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;

/// One 60 Hz frame of a run paced at 1x.
const FRAME_PS: u64 = 16_666_666_667;

/// Past every run's stop.
const END: Time = Time::from_ps(u64::MAX);

/// A seeded xorshift, so a failing chunking replays.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// Not the epoch, so a `finish` that drops its timestamp writes another `manifest.json`.
fn created_at() -> Timestamp {
    Timestamp::from_unix_seconds(1_700_000_000)
}

/// `created_at` as `manifest.json` writes it.
const CREATED_AT: &str = "2023-11-14T22:13:20Z";

fn stop_ps(json: &[u8]) -> u64 {
    match netlist::parse(json).unwrap().analysis() {
        Analysis::Transient(transient) => transient.stop.as_ps(),
        Analysis::Op => panic!("an op analysis has no stop"),
    }
}

/// Advances `session` on `json` in seeded calls until it ends or stands at `until`. A call moves
/// 1 ps, a 60 Hz frame at 1x, a 60th of the run, or up to an eighth of it, and stops at `until`.
/// After each call it reads the probe, the levels and the new serial bytes, and those reads change
/// nothing. Then `input` may apply an input at `now`. Returns the serial bytes read, as rows of
/// `serial.csv`.
fn advance_in_chunks(
    session: &mut Session,
    json: &[u8],
    until: Time,
    rng: &mut Rng,
    mut input: impl FnMut(&mut Session, &mut Rng),
) -> Vec<String> {
    let stop = stop_ps(json);
    let mut cursor = SerialCursor::default();
    let mut rows = Vec::new();
    loop {
        let stride = match rng.below(8) {
            0 | 1 => 1,
            2 => FRAME_PS,
            3..=5 => (stop / 60).max(1),
            _ => 1 + rng.below((stop / 8).max(1)),
        };
        let to = Time::from_ps(session.now().as_ps().saturating_add(stride)).min(until);
        let status = session.advance_to(to);
        let now = session.now();
        assert_eq!(session.probe().at(), now);
        assert_eq!(session.levels().len(), session.signals().len());
        let (bytes, next) = session.serial_since(cursor);
        let usarts = session.usarts();
        rows.extend(bytes.iter().map(|byte| {
            let (part, usart) = &usarts[usize::from(byte.usart)];
            format!(
                "{},{},{usart},{}",
                byte.at.as_ps(),
                part.as_str(),
                byte.byte
            )
        }));
        cursor = next;
        assert_eq!(session.now(), now, "a read moved the session");
        if status != Status::Running || now >= until {
            return rows;
        }
        input(session, rng);
    }
}

/// The two folders hold the same files, byte for byte, and the session's is stamped `stamp`.
fn assert_same_folder(name: &str, session: &RunFolder, batch: &RunFolder, stamp: &str) {
    let manifest: serde_json::Value =
        serde_json::from_slice(session.file("manifest.json").unwrap()).unwrap();
    assert_eq!(manifest["created_at"], stamp, "{name}");
    let paths = |folder: &RunFolder| -> Vec<String> {
        folder.files().map(|(path, _)| path.to_string()).collect()
    };
    assert_eq!(paths(session), paths(batch), "{name}");
    for ((path, got), (_, want)) in session.files().zip(batch.files()) {
        assert!(got == want, "{name}: {path} differs");
    }
}

/// `rows` are the rows of the folder's `serial.csv`, and none without one.
fn assert_serial_rows(name: &str, folder: &RunFolder, rows: &[String]) {
    let csv = folder
        .file("traces/serial.csv")
        .unwrap_or(b"t_ps,part,usart,byte\n");
    let csv = std::str::from_utf8(csv).unwrap();
    let want: Vec<&str> = csv.lines().skip(1).collect();
    assert_eq!(rows, want, "{name}: serial rows");
}

/// `json` recorded, advanced in chunks with no input, and finished, is `sim run`'s folder. An
/// `op` analysis opens no session, and `sim run` still writes its folder.
fn finishes_as_sim_run(name: &str, json: &[u8], seed: u64) {
    let batch = sim_cli::run(json, created_at()).folder;
    let mut session = match Session::record_document(json) {
        Ok(session) => session,
        Err(Refused::Op) => {
            assert!(batch.file("observations.json").is_some(), "{name}");
            return;
        }
        Err(refused) => panic!("{name}: {refused:?}"),
    };
    let rows = advance_in_chunks(&mut session, json, END, &mut Rng(seed), |_, _| {});
    let folder = session.finish(created_at()).unwrap();
    assert_same_folder(name, &folder, &batch, CREATED_AT);
    assert_serial_rows(name, &folder, &rows);
}

fn example(path: &Path) -> (String, Vec<u8>) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    (name, fs::read(path.join("netlist.json")).unwrap())
}

#[test]
fn every_example_finishes_to_the_folder_sim_run_writes() {
    let examples: Vec<_> = examples().iter().map(|path| example(path)).collect();
    assert!(examples.len() >= 14);
    std::thread::scope(|scope| {
        for (k, (name, json)) in examples.iter().enumerate() {
            scope.spawn(move || finishes_as_sim_run(name, json, 0x5eed_8000 + k as u64));
        }
    });
}

/// `v-steps` as a JSON value.
fn v_steps() -> serde_json::Value {
    serde_json::from_slice(&fs::read(common::repo("examples/v-steps/netlist.json")).unwrap())
        .unwrap()
}

/// `v-steps` with a floating pair of nets and a power rating `r1` exceeds.
fn floating() -> serde_json::Value {
    let mut json = v_steps();
    json["nets"]
        .as_array_mut()
        .unwrap()
        .extend(["x".into(), "y".into()]);
    json["parts"][1]["ratings"]["power"] =
        serde_json::json!({ "value": 0.001, "unit": "W", "source": "user" });
    json["parts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "r2",
            "kind": "resistor",
            "pins": { "a": "x", "b": "y" },
            "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
            "ratings": { "power": null }
        }));
    json
}

#[test]
fn floating_nets_and_ratings_finish_as_sim_run_reports_them() {
    let json = serde_json::to_vec(&floating()).unwrap();
    let results = sim_cli::run(&json, created_at()).results;
    let codes: Vec<_> = results.diagnostics().iter().map(|d| d.code()).collect();
    assert_eq!(codes, ["net.floating", "net.floating", "rating.power"]);
    finishes_as_sim_run("floating", &json, 0x5eed_8100);
}

/// `json`, the floating `v-steps` changed so it cannot open, is refused by `record_document` with
/// `code`, and the folder `sim run` writes lists the floating nets, then `code`.
fn refused_after_its_floating_nets(json: serde_json::Value, code: &str) {
    let json = serde_json::to_vec(&json).unwrap();
    let Err(Refused::Failed(refused)) = Session::record_document(&json) else {
        panic!("a session opens");
    };
    assert_eq!(refused.code(), code);
    let folder = sim_cli::run(&json, created_at()).folder;
    let diagnostics: serde_json::Value =
        serde_json::from_slice(folder.file("diagnostics.json").unwrap()).unwrap();
    let codes: Vec<_> = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["net.floating", "net.floating", code]);
}

/// The floating `v-steps` on a grid past the row limit, refused before any solve.
#[test]
fn a_grid_past_the_row_limit_reports_its_floating_nets() {
    let mut json = floating();
    json["analysis"]["transient"]["stop_ps"] = 2_000_000_000.into();
    json["analysis"]["transient"]["grid_ps"] = 1_000.into();
    refused_after_its_floating_nets(json, "trace.too_large");
}

/// The floating `v-steps` starting at 1e200 V, which faults the operating point at t = 0.
#[test]
fn a_fault_at_t_0_reports_its_floating_nets() {
    let mut json = floating();
    json["parts"][0]["voltage"]["initial"]["value"] = 1e200.into();
    refused_after_its_floating_nets(json, "solver.nonfinite");
}

/// `v-steps` whose second step is 1e200 V, which faults the run at that step.
#[test]
fn a_faulted_session_finishes_to_the_folder_sim_run_writes() {
    let mut json = v_steps();
    json["parts"][0]["voltage"]["events"][1]["level"]["value"] = 1e200.into();
    let json = serde_json::to_vec(&json).unwrap();
    let mut session = Session::record_document(&json).unwrap();
    assert_eq!(session.advance_to(END), Status::Faulted);
    assert_eq!(session.fault().unwrap().code(), "solver.nonfinite");
    finishes_as_sim_run("fault", &json, 0x5eed_8200);
}

/// `examples/<name>` recorded, advanced in chunks to half its run, and finished while it runs,
/// is `sim run`'s folder for its bytes. At each chunk end `reject` may apply inputs that are
/// refused, and those leave the bytes as they were.
fn finishes_partway_as_sim_run(name: &str, seed: u64, mut reject: impl FnMut(&mut Session)) {
    let json = fs::read(common::repo(&format!("examples/{name}/netlist.json"))).unwrap();
    let batch = sim_cli::run(&json, created_at()).folder;
    let mut session = Session::record_document(&json).unwrap();
    let until = Time::from_ps(stop_ps(&json) / 2);
    advance_in_chunks(&mut session, &json, until, &mut Rng(seed), |session, _| {
        reject(session)
    });
    assert_eq!(session.status(), Status::Running, "{name}");
    let folder = session.finish(created_at()).unwrap();
    assert_eq!(folder.file("netlist.json"), Some(json.as_slice()), "{name}");
    assert_same_folder(name, &folder, &batch, CREATED_AT);
}

#[test]
fn a_running_session_with_no_input_finishes_to_the_folder_sim_run_writes() {
    std::thread::scope(|scope| {
        for (k, &(name, _, _)) in INPUTS.iter().enumerate() {
            scope.spawn(move || finishes_partway_as_sim_run(name, 0x5eed_8500 + k as u64, |_| {}));
        }
    });
}

/// Applies each input, at `now` unless it names a time, and asserts it is refused.
fn refuse_each(session: &mut Session, refusals: &[(&str, Option<Time>, Setting, Rejected)]) {
    for &(part, at, setting, why) in refusals {
        let input = Input {
            part: Name::new(part).unwrap(),
            at: at.unwrap_or(session.now()),
            setting,
        };
        assert_eq!(session.apply(&input), Err(why), "{part}");
    }
}

/// At each chunk end `v-steps` refuses its own last level on `v1`, a level on a part it does not
/// have, a new level on `v1` at `stop`, a contact on `v1`, and a level on `r1`.
/// `atmega-analog-serial` refuses a position of 2 on `rv1`. That is every kind of refusal a
/// running session gives.
#[test]
fn refused_inputs_finish_to_the_folder_sim_run_writes_for_the_opened_bytes() {
    let level = |value| Setting::Level(Finite::new(value).unwrap());
    let json = fs::read(common::repo("examples/v-steps/netlist.json")).unwrap();
    let stop = Some(Time::from_ps(stop_ps(&json)));
    let closed = Setting::Contact(Contact::Closed);
    let v_steps = [
        ("v1", None, level(1.0), Rejected::NoChange),
        ("v9", None, level(1.0), Rejected::UnknownPart),
        ("v1", stop, level(2.0), Rejected::PastStop),
        ("v1", None, closed, Rejected::WrongSetting),
        ("r1", None, level(1.0), Rejected::NotSettable),
    ];
    let pot = [(
        "rv1",
        None,
        level(2.0),
        Rejected::Value(ValueRule::Fraction),
    )];
    std::thread::scope(|scope| {
        scope.spawn(|| {
            finishes_partway_as_sim_run("v-steps", 0x5eed_8600, |session| {
                refuse_each(session, &v_steps)
            })
        });
        scope.spawn(|| {
            finishes_partway_as_sim_run("atmega-analog-serial", 0x5eed_8601, |session| {
                refuse_each(session, &pot)
            })
        });
    });
}

/// `v-steps` advanced to its stop refuses a new level on `v1` with `Ended`, and finished a day
/// after `created_at`, is `sim run`'s folder for the bytes read at that time.
#[test]
fn an_input_after_the_end_finishes_to_the_folder_sim_run_writes_for_the_opened_bytes() {
    let json = fs::read(common::repo("examples/v-steps/netlist.json")).unwrap();
    let a_day_later = Timestamp::from_unix_seconds(1_700_086_400);
    let batch = sim_cli::run(&json, a_day_later).folder;
    let mut session = Session::record_document(&json).unwrap();
    assert_eq!(session.advance_to(END), Status::Stopped);
    let level = Setting::Level(Finite::new(2.0).unwrap());
    refuse_each(&mut session, &[("v1", None, level, Rejected::Ended)]);
    let folder = session.finish(a_day_later).unwrap();
    assert_eq!(folder.file("netlist.json"), Some(json.as_slice()));
    assert_same_folder("v-steps", &folder, &batch, "2023-11-15T22:13:20Z");
}

/// The part each example takes inputs on, and whether they are contacts or volts.
const INPUTS: &[(&str, &str, bool)] = &[
    ("atmega-button", "sw1", true),
    ("button-led", "sw1", true),
    ("rc-switch", "sw1", true),
    ("v-steps", "v1", false),
];

/// One seeded input on `part` at `now`, a contact or a level from 0 V to 5 V. A contact equal to
/// the last is moved to the other one. Whether it landed.
fn apply_seeded(session: &mut Session, rng: &mut Rng, part: &Name, contact: bool) -> bool {
    let settings = if contact {
        let closed = rng.below(2) == 0;
        [closed, !closed].map(|closed| {
            Setting::Contact(if closed {
                Contact::Closed
            } else {
                Contact::Open
            })
        })
    } else {
        let volts = Finite::new(rng.below(11) as f64 / 2.0).unwrap();
        [Setting::Level(volts); 2]
    };
    for setting in settings {
        let input = Input {
            part: part.clone(),
            at: session.now(),
            setting,
        };
        match session.apply(&input) {
            Ok(_) => return true,
            Err(Rejected::NoChange) => {}
            Err(rejected) => {
                assert_eq!(rejected, Rejected::PastStop);
                return false;
            }
        }
    }
    false
}

/// `examples/<name>` recorded and advanced in chunks to `until`, with a seeded input on `part`
/// at each chunk end until one lands and at half of them after, and the serial rows it read. One
/// seed gives the same session every time.
fn with_inputs(
    name: &str,
    part: &str,
    contact: bool,
    until: Time,
    seed: u64,
) -> (Session, Vec<String>) {
    let json = fs::read(common::repo(&format!("examples/{name}/netlist.json"))).unwrap();
    let part = Name::new(part).unwrap();
    let mut session = Session::record_document(&json).unwrap();
    let mut applied = 0;
    let rows = advance_in_chunks(
        &mut session,
        &json,
        until,
        &mut Rng(seed),
        |session, rng| {
            if (applied == 0 || rng.below(2) == 1) && apply_seeded(session, rng, &part, contact) {
                applied += 1;
            }
        },
    );
    assert!(applied > 0, "{name}: no input landed");
    assert_ne!(session.export().unwrap(), json, "{name}");
    (session, rows)
}

/// The example with inputs, advanced to its end, exported, then finished, is `sim run`'s folder
/// for that ended export, and the serial rows read are its `serial.csv`.
fn finishes_as_sim_run_on_the_export(name: &str, part: &str, contact: bool, seed: u64) {
    let (session, rows) = with_inputs(name, part, contact, END, seed);
    let batch = sim_cli::run(&session.export().unwrap(), created_at()).folder;
    let folder = session.finish(created_at()).unwrap();
    assert_same_folder(name, &folder, &batch, CREATED_AT);
    assert_serial_rows(name, &folder, &rows);
}

#[test]
fn inputs_finish_to_the_folder_sim_run_writes_for_the_export() {
    std::thread::scope(|scope| {
        for (k, &(name, part, contact)) in INPUTS.iter().enumerate() {
            scope.spawn(move || {
                finishes_as_sim_run_on_the_export(name, part, contact, 0x5eed_8300 + k as u64)
            });
        }
    });
}

/// A session with inputs, finished while it runs, writes the folder of the export it ends on. A
/// twin on the same seed, advanced to its end and exported, gives that export. The export taken
/// while the twin still runs has a shorter `stop_ps`, so `finish` cannot write that one.
fn a_running_session_finishes_as_its_ended_export(
    name: &str,
    part: &str,
    contact: bool,
    seed: u64,
) {
    let json = fs::read(common::repo(&format!("examples/{name}/netlist.json"))).unwrap();
    let until = Time::from_ps(stop_ps(&json) / 2);
    let (session, _) = with_inputs(name, part, contact, until, seed);
    let (mut twin, _) = with_inputs(name, part, contact, until, seed);
    assert_eq!(session.status(), Status::Running, "{name}");
    let running = twin.export().unwrap();
    assert_eq!(twin.advance_to(END), Status::Stopped, "{name}");
    let ended = twin.export().unwrap();
    assert_ne!(running, ended, "{name}");
    let batch = sim_cli::run(&ended, created_at()).folder;
    let folder = session.finish(created_at()).unwrap();
    assert_same_folder(name, &folder, &batch, CREATED_AT);
}

#[test]
fn a_running_session_with_inputs_finishes_to_the_folder_of_its_ended_export() {
    std::thread::scope(|scope| {
        for (k, &(name, part, contact)) in INPUTS.iter().enumerate() {
            scope.spawn(move || {
                a_running_session_finishes_as_its_ended_export(
                    name,
                    part,
                    contact,
                    0x5eed_8400 + k as u64,
                )
            });
        }
    });
}
