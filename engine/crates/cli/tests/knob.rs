//! `examples/atmega-board-knob`: a detent knob as two contacts on the board fragment. `sw_a`
//! pulls `d2` low and `sw_b` pulls `d3` low, each against 10 kohm to `v5`, and
//! `firmware/sketch/knob.hex` prints both counts after each fall.

mod common;

use std::ops::RangeInclusive;

use common::{records, repo};
use sim_circuit::{Input, Session, Setting, Status};
use sim_core::name::Name;
use sim_core::netlist::Contact;
use sim_core::run_folder::RunFolder;
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;

// The shortest to the longest seen over the board's two knob captures, 13 A and 18 B detents,
// rounded outward to 0.1 ms: how long one detent held each line low, and the idle time from a
// release to the next press.
const A_LOW_PS: RangeInclusive<u64> = 51_200_000_000..=130_400_000_000;
const B_LOW_PS: RangeInclusive<u64> = 29_400_000_000..=331_700_000_000;
const GAP_PS: RangeInclusive<u64> = 41_600_000_000..=9_185_600_000_000;

const LEAD_PS: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    A,
    B,
}

impl Line {
    fn switch(self) -> &'static str {
        match self {
            Line::A => "sw_a",
            Line::B => "sw_b",
        }
    }

    fn low_ps(self) -> RangeInclusive<u64> {
        match self {
            Line::A => A_LOW_PS,
            Line::B => B_LOW_PS,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Detent {
    line: Line,
    at_ps: u64,
    low_ps: u64,
}

#[test]
fn the_script_stays_inside_what_the_board_showed() {
    let netlist = netlist();
    let detents = detents(&netlist);
    for line in [Line::A, Line::B] {
        let switch = part(&netlist, line.switch());
        assert_eq!(switch["initial"], "open", "{line:?}");
        assert!(switch["bounce"].is_null(), "{line:?}");
        let lows: Vec<u64> = detents
            .iter()
            .filter(|detent| detent.line == line)
            .map(|detent| detent.low_ps)
            .collect();
        let range = line.low_ps();
        assert!(
            lows.iter().all(|low| range.contains(low)),
            "{line:?} {lows:?}"
        );
        assert!(lows.contains(range.start()), "{line:?} holds its shortest");
        assert!(lows.contains(range.end()), "{line:?} holds its longest");
    }
    for pair in detents.windows(2) {
        let release = pair[0].at_ps + pair[0].low_ps;
        assert!(release < pair[1].at_ps, "both lines low at once: {pair:?}");
        assert!(GAP_PS.contains(&(pair[1].at_ps - release)), "{pair:?}");
    }
    let last = detents.last().unwrap();
    let stop = netlist["analysis"]["transient"]["stop_ps"]
        .as_u64()
        .unwrap();
    assert!(last.at_ps + last.low_ps < stop);
}

#[test]
fn the_sketch_prints_the_counts_the_script_gives() {
    let netlist = netlist();
    assert_eq!(
        records(part(&netlist, "mcu")["firmware"].as_str().unwrap()),
        records(&std::fs::read_to_string(repo("firmware/sketch/knob.hex")).unwrap())
    );
    let done = sim_cli::run(&example_bytes(), stamp());
    assert!(
        done.results.diagnostics().is_empty(),
        "{}",
        serde_json::to_string(done.results.diagnostics()).unwrap()
    );
    assert_eq!(serial_text(&done.folder), counts(&detents(&netlist)));
}

#[test]
fn detents_applied_live_finish_as_sim_run_of_the_export() {
    let mut netlist = netlist();
    let detents = detents(&netlist);
    for line in [Line::A, Line::B] {
        part_mut(&mut netlist, line.switch())["events"] = serde_json::json!([]);
    }
    let mut session = Session::record_document(&serde_json::to_vec(&netlist).unwrap()).unwrap();
    for detent in &detents {
        session.advance_to(Time::from_ps(detent.at_ps - LEAD_PS));
        let part = Name::new(detent.line.switch()).unwrap();
        for (at_ps, contact) in [
            (detent.at_ps, Contact::Closed),
            (detent.at_ps + detent.low_ps, Contact::Open),
        ] {
            let applied = session
                .apply(&Input {
                    part: part.clone(),
                    at: Time::from_ps(at_ps),
                    setting: Setting::Contact(contact),
                })
                .unwrap_or_else(|rejected| panic!("{detent:?} {contact:?}: {rejected:?}"));
            assert_eq!(applied.at, Time::from_ps(at_ps), "{detent:?} {contact:?}");
        }
    }
    assert_eq!(session.advance_to(Time::from_ps(u64::MAX)), Status::Stopped);
    let export = session.export().unwrap();
    let batch = sim_cli::run(&export, stamp()).folder;
    let folder = session.finish(stamp()).unwrap();
    assert_eq!(paths(&folder), paths(&batch));
    for ((path, got), (_, want)) in folder.files().zip(batch.files()) {
        assert!(got == want, "{path} differs from sim run of the export");
    }
    let committed = sim_cli::run(&example_bytes(), stamp()).folder;
    assert_eq!(paths(&folder), paths(&committed));
    for ((path, got), (_, want)) in folder.files().zip(committed.files()) {
        if path != "netlist.json" && path != "manifest.json" {
            assert!(
                got == want,
                "{path} differs from the committed script's run"
            );
        }
    }
}

fn example_bytes() -> Vec<u8> {
    std::fs::read(repo("examples/atmega-board-knob/netlist.json")).unwrap()
}

fn netlist() -> serde_json::Value {
    serde_json::from_slice(&example_bytes()).unwrap()
}

fn part<'a>(netlist: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn part_mut<'a>(netlist: &'a mut serde_json::Value, name: &str) -> &'a mut serde_json::Value {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn detents(netlist: &serde_json::Value) -> Vec<Detent> {
    let mut out = Vec::new();
    for line in [Line::A, Line::B] {
        let events = part(netlist, line.switch())["events"].as_array().unwrap();
        assert_eq!(events.len() % 2, 0, "{line:?} ends open");
        for pair in events.chunks(2) {
            assert_eq!(pair[0]["state"], "closed", "{line:?}");
            assert_eq!(pair[1]["state"], "open", "{line:?}");
            let at_ps = pair[0]["t_ps"].as_u64().unwrap();
            let low_ps = pair[1]["t_ps"].as_u64().unwrap() - at_ps;
            out.push(Detent {
                line,
                at_ps,
                low_ps,
            });
        }
    }
    out.sort_by_key(|detent| detent.at_ps);
    assert!(out.iter().any(|detent| detent.line == Line::A));
    assert!(out.iter().any(|detent| detent.line == Line::B));
    out
}

fn counts(detents: &[Detent]) -> String {
    let (mut a, mut b) = (0, 0);
    let mut text = String::new();
    for detent in detents {
        match detent.line {
            Line::A => a += 1,
            Line::B => b += 1,
        }
        text += &format!("{a} {b}\r\n");
    }
    text
}

fn serial_text(folder: &RunFolder) -> String {
    let csv = std::str::from_utf8(folder.file("traces/serial.csv").unwrap()).unwrap();
    csv.lines()
        .skip(1)
        .map(|line| char::from(line.split(',').nth(3).unwrap().parse::<u8>().unwrap()))
        .collect()
}

fn paths(folder: &RunFolder) -> Vec<String> {
    folder.files().map(|(path, _)| path.to_string()).collect()
}

fn stamp() -> Timestamp {
    Timestamp::from_unix_seconds(1_700_000_000)
}
