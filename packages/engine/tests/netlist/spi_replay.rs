use orbe_engine::device::SpiReplay;
use orbe_engine::netlist::{self, Chip, Model, Netlist};

use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::link::{Cs, Lanes};

use orbe_engine::time::Time;

const EXAMPLE: &str = include_str!("../circuits/spi-replay/netlist.json");

/// `packages/engine/tests/circuits/spi-replay` with `edit` applied to its replay part.
fn replay(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Netlist, Vec<Diagnostic>> {
    let mut document: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
    edit(&mut document["parts"][0]);
    netlist::parse(&serde_json::to_vec(&document).unwrap())
}

fn values(netlist: &Netlist) -> SpiReplay {
    let (_, part) = netlist.parts().next().unwrap();
    match &part.model {
        Model::Chip(Chip::SpiReplay(replay)) => (**replay).clone(),
        other => panic!("{other:?}"),
    }
}

/// Each diagnostic's code and its message.
fn errors(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<(String, String)> {
    replay(edit)
        .unwrap_err()
        .iter()
        .map(|d| serde_json::to_value(d).unwrap())
        .map(|d| {
            let text = |key: &str| d[key].as_str().unwrap().to_string();
            (text("code"), text("message"))
        })
        .collect()
}

fn code(edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let errors = errors(edit);
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors[0].0.clone()
}

#[test]
fn each_event_ends_after_its_cycles_at_the_clock() {
    let replay = values(&replay(|_| {}).unwrap());
    let ends: Vec<_> = replay.events.iter().map(|event| event.end).collect();
    // 32 cycles a register write and 8 more a data byte, at 25 ns a cycle. The pixel write is
    // 32 cycles and 2 bytes on 4 lanes, and its continuation 4 cycles.
    let want = [10_800, 21_000, 31_600, 41_600, 50_900, 52_100, 60_800];
    assert_eq!(ends, want.map(|ns| Time::from_ps(ns * 1000)));
    assert_eq!(replay.events[4].lanes, Lanes::Four);
    assert_eq!(replay.events[4].cs, Cs::Held);
    assert_eq!(&replay.events[4].data[..], [248, 0]);
    assert_eq!(replay.events[0].phases[1].value, 0x1100);
}

#[test]
fn a_phase_or_lane_count_outside_the_rules_is_syntax() {
    let phase = |key: &'static str, value: serde_json::Value| {
        move |part: &mut serde_json::Value| part["events"][0]["phases"][0][key] = value
    };
    assert_eq!(code(phase("bits", 0.into())), "netlist.syntax");
    assert_eq!(code(phase("bits", 33.into())), "netlist.syntax");
    assert_eq!(code(phase("lanes", 3.into())), "netlist.syntax");
    assert_eq!(code(phase("value", 256.into())), "netlist.syntax");
    assert_eq!(
        code(|part| part["events"][0]["lanes"] = 8.into()),
        "netlist.syntax"
    );
    assert_eq!(
        code(|part| part["events"][0]["cs"] = "low".into()),
        "netlist.syntax"
    );
    let wide = |part: &mut serde_json::Value| {
        part["events"][0]["phases"][0] =
            serde_json::json!({ "bits": 32, "lanes": 1, "value": u32::MAX })
    };
    assert!(replay(wide).is_ok());
}

#[test]
fn events_keep_the_serial_schedule_rules() {
    let overlap = errors(|part| part["events"][1]["t_ps"] = 10_799_999.into());
    assert_eq!(
        overlap,
        [(
            "netlist.schedule".to_string(),
            "#rp1 events[1] starts before the previous event ends at 10800000 ps".to_string()
        )]
    );
    assert!(replay(|part| part["events"][1]["t_ps"] = 10_800_000.into()).is_ok());
    let rule = |edit: fn(&mut serde_json::Value)| {
        let doc = serde_json::to_value(&replay(edit).unwrap_err()[0]).unwrap();
        doc["data"]["rule"].as_str().unwrap().to_string()
    };
    assert_eq!(
        rule(|part| part["events"][1]["t_ps"] = 10_000_000.into()),
        "not_increasing"
    );
    assert_eq!(rule(|part| part["events"][0]["t_ps"] = 0.into()), "at_zero");
    assert_eq!(
        rule(|part| {
            part["events"][0]["phases"] = serde_json::json!([]);
            part["events"][0]["bytes"] = serde_json::json!([]);
        }),
        "same_state"
    );
    assert_eq!(
        rule(|part| part["events"][6]["t_ps"] = u64::MAX.into()),
        "overflow"
    );
}

#[test]
fn a_clock_is_a_whole_number_of_hertz_and_every_pin_is_required() {
    assert_eq!(
        code(|part| part["clock_hz"]["value"] = 0.into()),
        "netlist.value"
    );
    assert_eq!(
        code(|part| part["clock_hz"]["unit"] = "kHz".into()),
        "netlist.unit"
    );
    assert_eq!(
        code(|part| {
            part["pins"].as_object_mut().unwrap().remove("D3");
        }),
        "netlist.pins"
    );
}
