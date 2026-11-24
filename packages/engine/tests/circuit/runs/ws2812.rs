//! An ATmega328P bit-bangs a 13-LED chain, and a pulse replay of the same colours
//! reports the same chain. `packages/engine/scripts/ws2812_decode.py` reads the bit-bang `logic.vcd`
//! apart from the engine. Variants of the replay check what the records do between landings.

use std::path::PathBuf;
use std::process::Command;

use crate::runs::common;
use common::repo;
use orbe_engine::netlist;
use orbe_engine::run_folder;
use orbe_engine::simulate_stats;
use orbe_engine::timestamp::Timestamp;

fn palette() -> Vec<Vec<u64>> {
    (0..13).map(|i| vec![i, i + 16, i + 32]).collect()
}

fn render(bytes: &[u8]) -> (run_folder::RunFolder, orbe_engine::RunStats) {
    let parsed = netlist::parse(bytes).unwrap();
    let (results, stats) = simulate_stats(&parsed);
    assert_eq!(
        results.count(orbe_engine::diagnostic::Severity::Error),
        0,
        "{}",
        serde_json::to_string(results.diagnostics()).unwrap()
    );
    let folder = run_folder::render(
        bytes,
        Some(&parsed),
        &results,
        Timestamp::from_unix_seconds(0),
    );
    (folder, stats)
}

fn run(name: &str) -> serde_json::Value {
    let path = repo(&format!(
        "packages/engine/tests/circuits/{name}/netlist.json"
    ));
    let bytes = std::fs::read(&path).unwrap();
    let (folder, stats) = render(&bytes);
    let observations: serde_json::Value =
        serde_json::from_slice(folder.file("observations.json").unwrap()).unwrap();
    let pixels = observations["leds"]["ring"]["pixels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|px| {
            px.as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(pixels, palette(), "{name}");
    assert_eq!(observations["leds"]["ring"]["order"], "grb");
    assert_eq!(observations["leds"]["ring"]["count"], 13);
    let csv = std::str::from_utf8(folder.file("traces/leds.csv").unwrap()).unwrap();
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("t_ps,part,count,grb"));
    let row = lines.next().unwrap();
    assert!(lines.next().is_none(), "{name} latches once");
    assert!(row.contains(",ring,13,"), "{row}");
    if name == "ws2812-replay" {
        assert_eq!(
            row,
            "1669062501,ring,13,0010200111210212220313230414240515250616260717270818280919290a1a2a0b1b2b0c1c2c"
        );
    }
    println!("{name} landings {} stops {}", stats.landings, stats.stops);
    if name == "atmega-ws2812" {
        let dir = scratch().join(name);
        folder.write(&dir).unwrap();
        let output = Command::new("uv")
            .args([
                "run",
                "packages/engine/scripts/ws2812_decode.py",
                "--part",
                "ring",
            ])
            .arg(&dir)
            .current_dir(repo(""))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    observations
}

fn scratch() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("ws2812-check")
}

#[ignore = "full tier"]
#[test]
fn the_sketch_hex_is_the_one_the_example_runs() {
    let path = repo("packages/engine/tests/circuits/atmega-ws2812/netlist.json");
    let netlist: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let chip = netlist["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == "u1")
        .unwrap();
    assert_eq!(chip["clock_hz"]["value"], 16_000_000);
    assert_eq!(
        common::records(chip["firmware"].as_str().unwrap()),
        common::records(
            &std::fs::read_to_string(repo("packages/engine/tests/firmware/sketch/ws2812.hex"))
                .unwrap()
        )
    );
}

#[ignore = "full tier"]
#[test]
fn the_bit_bang_and_the_replay_report_the_same_colours() {
    let bang = run("atmega-ws2812");
    let replay = run("ws2812-replay");
    assert_eq!(
        bang["leds"]["ring"]["pixels"],
        replay["leds"]["ring"]["pixels"]
    );
}

/// `packages/engine/tests/circuits/ws2812-replay` with its replay edited: its `leds.csv`, its `observations.json`
/// `leds` and its `diagnostics.json`.
fn replay(edit: impl FnOnce(&mut serde_json::Value)) -> (String, serde_json::Value, String) {
    let path = repo("packages/engine/tests/circuits/ws2812-replay/netlist.json");
    let mut netlist: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let part = netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["kind"] == "pulses_replay")
        .unwrap();
    edit(part);
    let (folder, _) = render(&serde_json::to_vec(&netlist).unwrap());
    let text = |name| String::from_utf8(folder.file(name).unwrap().to_vec()).unwrap();
    let observations: serde_json::Value =
        serde_json::from_slice(folder.file("observations.json").unwrap()).unwrap();
    (
        text("traces/leds.csv"),
        observations["leds"].clone(),
        text("diagnostics.json"),
    )
}

#[ignore = "full tier"]
#[test]
fn a_frame_split_across_two_events_writes_the_same_leds() {
    let whole = replay(|_| {});
    for at in [288, 289] {
        let split = replay(|part| {
            let event = &part["events"][0];
            let t = event["t_ps"].as_u64().unwrap();
            let pulses = event["pulses"].as_array().unwrap().clone();
            let (first, second) = pulses.split_at(at);
            let end = t + first.iter().map(|p| p["ps"].as_u64().unwrap()).sum::<u64>();
            part["events"] = serde_json::json!([
                { "t_ps": t, "pulses": first },
                { "t_ps": end, "pulses": second },
            ]);
        });
        assert_eq!(split, whole, "split at pulse {at}");
    }
}

#[ignore = "full tier"]
#[test]
fn an_idle_high_replay_writes_the_same_leds() {
    let low = replay(|_| {});
    let high = replay(|part| part["idle"] = serde_json::json!(true));
    assert_eq!(high, low);
}

/// Levels and their lengths in picoseconds, as an event lists them.
type Train = Vec<(bool, u64)>;

/// The sketch's bits as pulses, high then low, the last low lengthened past the reset.
fn train(bytes: &[u8]) -> Train {
    let mut out = Vec::new();
    for byte in bytes {
        for k in (0..8).rev() {
            let (high, low) = if byte & (1 << k) != 0 {
                (625_000, 625_000)
            } else {
                (312_500, 937_500)
            };
            out.extend([(true, high), (false, low)]);
        }
    }
    out.last_mut().unwrap().1 += 300_000_000;
    out
}

/// `packages/engine/tests/circuits/ws2812-replay` cut to two LEDs, with `events` on an `idle` line, run to `stop_ps`:
/// its `leds.csv`, its `observations.json` and its `diagnostics.json`.
fn two_leds(idle: bool, events: &[(u64, &[(bool, u64)])], stop_ps: u64) -> [String; 3] {
    let path = repo("packages/engine/tests/circuits/ws2812-replay/netlist.json");
    let mut netlist: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let transient = &mut netlist["analysis"]["transient"];
    transient["stop_ps"] = stop_ps.into();
    transient["max_step_ps"] = 100_000_000.into();
    transient["grid_ps"] = 100_000_000.into();
    for part in netlist["parts"].as_array_mut().unwrap() {
        if part["kind"] == "ws2812" {
            part["leds"] = 2.into();
        }
        if part["kind"] == "pulses_replay" {
            part["idle"] = idle.into();
            part["events"] = events
                .iter()
                .map(|(t_ps, pulses)| {
                    let pulses: Vec<_> = pulses
                        .iter()
                        .map(|&(level, ps)| serde_json::json!({ "level": level, "ps": ps }))
                        .collect();
                    serde_json::json!({ "t_ps": t_ps, "pulses": pulses })
                })
                .collect();
        }
    }
    let (folder, _) = render(&serde_json::to_vec(&netlist).unwrap());
    let text = |name| String::from_utf8(folder.file(name).unwrap().to_vec()).unwrap();
    [
        text("traces/leds.csv"),
        text("observations.json"),
        text("diagnostics.json"),
    ]
}

/// The time `pulses` from `start` last fall, the return to an `idle` low included.
fn last_fall(start: u64, pulses: &[(bool, u64)], idle: bool) -> Option<u64> {
    let (mut t, mut high, mut fall) = (start, idle, None);
    for &(level, ps) in pulses.iter().chain([(idle, 0)].iter()) {
        if high && !level {
            fall = Some(t);
        }
        high = level;
        t += ps;
    }
    fall
}

/// A 2-LED frame split into two events, at each pulse boundary and, on an idle-low line, inside
/// the reset low. On an idle-high line, an event also opens on a 500 ns low while the line is
/// held: before the frame from power-on, and after a first event that ends on a low. The second
/// event starts 0 ps or 1 ps after the first. At each stop, before, inside and after each event
/// and around each latch, the folder is that of one event holding the frame with the gap at the
/// split. The gap is at the idle level, except that a hold between two lows is no bit, so the
/// line stays low across it. No folder has a `chip.protocol` warning.
fn a_split_frame_writes_the_folder_of_the_whole_frame(idle: bool) {
    const START: u64 = 1_000_000_000;
    const RES: u64 = 280_000_000;
    const LEAD: u64 = 500_000;
    let pulses = train(&[0x12, 0x34, 0x56, 0xA5, 0x5A, 0xC3]);
    let mut splits: Vec<(String, Train, Train)> = (1..pulses.len())
        .map(|k| {
            (
                format!("pulse {k}"),
                pulses[..k].to_vec(),
                pulses[k..].to_vec(),
            )
        })
        .collect();
    if !idle {
        let (level, ps) = *pulses.last().unwrap();
        for into in [1, RES / 2, ps - 1] {
            let mut first = pulses.clone();
            first.last_mut().unwrap().1 = into;
            splits.push((
                format!("{into} ps into the reset"),
                first,
                vec![(level, ps - into)],
            ));
        }
    } else {
        splits.push((
            "a low before the frame".into(),
            vec![(false, LEAD)],
            pulses.clone(),
        ));
        for k in (2..pulses.len()).step_by(2) {
            let mut second = vec![(false, LEAD)];
            second.extend(&pulses[k..]);
            splits.push((
                format!("pulse {k}, the second opening low"),
                pulses[..k].to_vec(),
                second,
            ));
        }
    }
    let sum = |pulses: &[(bool, u64)]| pulses.iter().map(|&(_, ps)| ps).sum::<u64>();
    let mut wholes = std::collections::BTreeMap::new();
    for (name, first, second) in &splits {
        for gap in [0, 1] {
            let start = START + sum(first) + gap;
            let mut joined = Train::new();
            let hold = idle && (first.last().unwrap().0 || second[0].0);
            let gap_pulse = (gap > 0).then_some((hold, gap));
            for &(level, ps) in first.iter().chain(&gap_pulse).chain(second) {
                match joined.last_mut() {
                    Some(last) if last.0 == level => last.1 += ps,
                    _ => joined.push((level, ps)),
                }
            }
            let latch = last_fall(START, &joined, idle).unwrap() + RES + 1;
            let mut stops = vec![
                START + sum(first) / 2,
                start - gap,
                start - gap + 1,
                latch - 1,
                latch,
                start + sum(second) - 1,
                start + sum(second),
                start + sum(second) + 1,
            ];
            if let Some(fall) = last_fall(START, first, idle) {
                let early = fall + RES + 1;
                stops.extend([early - 1, early, early + 1]);
            }
            stops.sort_unstable();
            stops.dedup();
            for stop in stops {
                let whole = wholes
                    .entry((joined.clone(), stop))
                    .or_insert_with(|| two_leds(idle, &[(START, &joined)], stop));
                assert!(!whole[2].contains("chip.protocol"), "{}", whole[2]);
                let split = two_leds(idle, &[(START, first), (start, second)], stop);
                assert_eq!(
                    &split, whole,
                    "idle {idle}, split at {name}, gap {gap}, stop {stop}"
                );
            }
        }
    }
}

#[ignore = "full tier"]
#[test]
fn a_split_frame_on_an_idle_low_line_writes_the_folder_of_the_whole_frame() {
    a_split_frame_writes_the_folder_of_the_whole_frame(false);
}

#[ignore = "full tier"]
#[test]
fn a_split_frame_on_an_idle_high_line_writes_the_folder_of_the_whole_frame() {
    a_split_frame_writes_the_folder_of_the_whole_frame(true);
}

/// A held idle-high line under an event that opens on a low: from power-on, and 1 ms after a
/// frame. The line falls at the event's start with no bit, so the folder is the idle-low one.
#[ignore = "full tier"]
#[test]
fn a_held_line_under_an_event_that_opens_low_writes_the_idle_low_folder() {
    const START: u64 = 1_000_000_000;
    let lead = |ps: u64| {
        let mut pulses = vec![(false, ps)];
        pulses.extend(train(&[0x80, 0x00, 0x00]));
        pulses
    };
    let from_power_on = lead(500_000);
    let first = train(&[0x11, 0x22, 0x33]);
    let next = START + first.iter().map(|&(_, ps)| ps).sum::<u64>() + 1_000_000_000;
    let after_a_frame = lead(400_000);
    for events in [
        vec![(START, &from_power_on[..])],
        vec![(START, &first[..]), (next, &after_a_frame[..])],
    ] {
        let stop = next + 2_000_000_000;
        let high = two_leds(true, &events, stop);
        assert!(high[0].contains(",ring,2,800000000000\n"), "{}", high[0]);
        assert!(!high[2].contains("chip.protocol"), "{}", high[2]);
        let low = two_leds(false, &events, stop);
        assert_eq!((&high[0], &high[2]), (&low[0], &low[2]));
    }
}

/// A high in neither window right before a split with no gap. A run that stops at the split
/// warns at the split, whether the frame is one event or two, as the pin would at that stop.
#[ignore = "full tier"]
#[test]
fn a_bad_high_before_a_split_warns_the_same_when_the_run_stops_at_the_split() {
    const START: u64 = 1_000_000_000;
    let mut pulses = train(&[0x12, 0x34, 0x56, 0xA5, 0x5A, 0xC3]);
    pulses[4].1 = 450_000;
    let (first, second) = pulses.split_at(5);
    let split = START + first.iter().map(|&(_, ps)| ps).sum::<u64>();
    for idle in [false, true] {
        for stop in [split - 1, split, split + 1] {
            let whole = two_leds(idle, &[(START, &pulses)], stop);
            let parts = two_leds(idle, &[(START, first), (split, second)], stop);
            assert_eq!(parts, whole, "idle {idle}, stop {stop}");
            assert_eq!(
                whole[2].contains(&format!("\"t_ps\": {split}")),
                stop >= split,
                "idle {idle}, stop {stop}: {}",
                whole[2]
            );
        }
    }
}

/// Idle high, one frame, the reset low split across two events. The return to idle ends the
/// first low. The second low starts the reset again, so the frame never latches when what
/// remains is within the reset, and it latches later when the remainder runs past it.
#[ignore = "full tier"]
#[test]
fn an_idle_high_split_inside_the_reset_low_never_latches_or_latches_late() {
    const START: u64 = 1_000_000_000;
    const RES: u64 = 280_000_000;
    let pulses = train(&[0x12, 0x34, 0x56, 0xA5, 0x5A, 0xC3]);
    let fall = last_fall(START, &pulses, true).unwrap();
    let whole_at = fall + RES + 1;
    let last = pulses.last().unwrap().1;
    for (into, gap) in [
        (150_000_000, 0),
        (150_000_000, 1),
        (1_000_000, 0),
        (1_000_000, 1),
    ] {
        let mut first = pulses.clone();
        first.last_mut().unwrap().1 = into;
        let second = vec![(false, last - into)];
        let start = fall + into + gap;
        let stop = start + (last - into) + 1_000_000_000;
        let whole = two_leds(true, &[(START, &pulses)], stop);
        let split = two_leds(true, &[(START, &first), (start, &second)], stop);
        assert!(
            whole[0].contains(&format!("{whole_at},ring,2,123456a55ac3")),
            "{}",
            whole[0]
        );
        assert!(
            !split[2].contains("chip.protocol"),
            "into {into} gap {gap}: {}",
            split[2]
        );
        let remainder = last - into;
        if remainder < RES + 1 {
            assert_eq!(
                split[0].lines().count(),
                1,
                "into {into} gap {gap}: {}",
                split[0]
            );
        } else {
            let late = start + RES + 1;
            assert_eq!(
                split[0].lines().count(),
                2,
                "into {into} gap {gap}: {}",
                split[0]
            );
            assert!(
                !split[0].contains(&format!("{whole_at},ring,2,123456a55ac3")),
                "into {into} gap {gap}: {}",
                split[0]
            );
            assert!(
                split[0].contains(&format!("{late},ring,2,123456a55ac3")),
                "into {into} gap {gap}: {}",
                split[0]
            );
        }
    }
}

/// Idle high, and the event opens on a low the line was already holding. The fall at the
/// record's start starts the reset, so a lead longer than the reset latches before the bits.
#[ignore = "full tier"]
#[test]
fn a_held_fall_starts_the_reset_count() {
    const START: u64 = 1_000_000_000;
    const RES: u64 = 280_000_000;
    const LEAD: u64 = 300_000_000;
    let mut pulses = vec![(false, LEAD)];
    pulses.extend(train(&[0x80, 0x00, 0x00, 0x00, 0x00, 0x00]));
    let stop = START + pulses.iter().map(|&(_, ps)| ps).sum::<u64>() + 1_000_000_000;
    let folder = two_leds(true, &[(START, &pulses)], stop);
    let early = START + RES + 1;
    assert!(
        folder[0].contains(&format!("{early},ring,2,000000000000")),
        "{}",
        folder[0]
    );
    let fall = last_fall(START, &pulses, true).unwrap();
    assert!(
        folder[0].contains(&format!("{},ring,2,800000000000", fall + RES + 1)),
        "{}",
        folder[0]
    );
    assert!(!folder[2].contains("chip.protocol"), "{}", folder[2]);
}

/// The `ws2812` part `ring` of circuit `name` cut into one part per count, each part's `DOUT` on
/// the next part's `DIN`. The last part has no `DOUT`. Gives each part's pixels by name, `led0`
/// first, and the run's `diagnostics.json`.
fn chained(name: &str, counts: &[u16]) -> (Vec<serde_json::Value>, serde_json::Value) {
    chained_with(name, counts, |_| {})
}

/// [`chained`], with `edit` made to the netlist first.
fn chained_with(
    name: &str,
    counts: &[u16],
    edit: impl FnOnce(&mut serde_json::Value),
) -> (Vec<serde_json::Value>, serde_json::Value) {
    let path = repo(&format!(
        "packages/engine/tests/circuits/{name}/netlist.json"
    ));
    let mut netlist: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    edit(&mut netlist);
    let parts = netlist["parts"].as_array_mut().unwrap();
    let k = parts
        .iter()
        .position(|part| part["name"] == "ring")
        .unwrap();
    let ring = parts.remove(k);
    let mut din = ring["pins"]["DIN"].as_str().unwrap().to_string();
    let mut nets = Vec::new();
    for (k, &count) in counts.iter().enumerate() {
        let mut part = ring.clone();
        part["name"] = format!("led{k}").into();
        part["leds"] = count.into();
        part["pins"]["DIN"] = din.clone().into();
        if k + 1 < counts.len() {
            din = format!("dout{k}");
            nets.push(din.clone());
            part["pins"]["DOUT"] = din.clone().into();
            part["dout"] = serde_json::json!({
                "output": {
                    "high_drop": { "value": 0.0, "unit": "V" },
                    "high_resistance": { "value": 50.0, "unit": "ohm" },
                    "low_drop": { "value": 0.0, "unit": "V" },
                    "low_resistance": { "value": 50.0, "unit": "ohm" }
                },
                "delay": { "value": 500000, "unit": "ps" }
            });
        }
        parts.push(part);
    }
    netlist["nets"]
        .as_array_mut()
        .unwrap()
        .extend(nets.into_iter().map(Into::into));
    let (folder, _) = render(&serde_json::to_vec(&netlist).unwrap());
    let observations: serde_json::Value =
        serde_json::from_slice(folder.file("observations.json").unwrap()).unwrap();
    let diagnostics: serde_json::Value =
        serde_json::from_slice(folder.file("diagnostics.json").unwrap()).unwrap();
    let pixels = (0..counts.len())
        .map(|k| observations["leds"][format!("led{k}")]["pixels"].clone())
        .collect();
    (pixels, diagnostics)
}

#[ignore = "full tier"]
#[test]
fn four_chained_parts_on_the_sketch_s_pin_each_show_their_own_colour() {
    let (pixels, diagnostics) = chained("atmega-ws2812", &[1, 1, 1, 1]);
    let palette = palette();
    for (k, pixels) in pixels.iter().enumerate() {
        assert_eq!(pixels, &serde_json::json!([palette[k]]), "led{k}");
    }
    assert_eq!(diagnostics, serde_json::json!([]));
}

#[ignore = "full tier"]
#[test]
fn a_ring_cut_in_three_by_dout_shows_the_whole_ring_s_colours() {
    let (pixels, diagnostics) = chained("atmega-ws2812", &[1, 4, 8]);
    let palette = palette();
    assert_eq!(pixels[0], serde_json::json!(palette[..1]));
    assert_eq!(pixels[1], serde_json::json!(palette[1..5]));
    assert_eq!(pixels[2], serde_json::json!(palette[5..]));
    assert_eq!(diagnostics, serde_json::json!([]));
}

#[ignore = "full tier"]
#[test]
fn a_record_goes_on_through_dout_to_the_next_chain() {
    let (pixels, diagnostics) = chained("ws2812-replay", &[1, 12]);
    let palette = palette();
    assert_eq!(pixels[0], serde_json::json!(palette[..1]));
    assert_eq!(pixels[1], serde_json::json!(palette[1..]));
    assert_eq!(diagnostics, serde_json::json!([]));
}

#[ignore = "full tier"]
#[test]
fn thirteen_parts_chained_on_a_record_each_show_their_own_colour() {
    let (pixels, diagnostics) = chained("ws2812-replay", &[1; 13]);
    let palette = palette();
    for (k, pixels) in pixels.iter().enumerate() {
        assert_eq!(pixels, &serde_json::json!([palette[k]]), "led{k}");
    }
    assert_eq!(diagnostics, serde_json::json!([]));
}

/// A high of 450 ns in the second part's bits is in neither window. The first part warns at its
/// fall, as it does for a bad high on the pin, and the second part warns at the fall on its own
/// `DIN`, 500 ns later. Both warn the same whether the run ends after the record or inside it,
/// after that fall.
#[ignore = "full tier"]
#[test]
fn a_bad_high_past_the_first_part_warns_on_the_part_it_reached() {
    const START: u64 = 1_000_000_000;
    const DELAY: u64 = 500_000;
    let mut pulses = train(&[0x12, 0x34, 0x56, 0xA5, 0x5A, 0xC3]);
    pulses[50].1 = 450_000;
    let fall = START + pulses[..51].iter().map(|&(_, ps)| ps).sum::<u64>() + DELAY;
    let frame: Vec<_> = pulses
        .iter()
        .map(|&(level, ps)| serde_json::json!({ "level": level, "ps": ps }))
        .collect();
    for stop in [2_000_000_000u64, fall + 1_000_000] {
        let (_, diagnostics) = chained_with("ws2812-replay", &[1, 1], |netlist| {
            netlist["analysis"]["transient"]["stop_ps"] = stop.into();
            let parts = netlist["parts"].as_array_mut().unwrap();
            let replay = parts
                .iter_mut()
                .find(|part| part["kind"] == "pulses_replay")
                .unwrap();
            replay["events"] = serde_json::json!([{ "t_ps": START, "pulses": frame }]);
        });
        let rules: Vec<_> = diagnostics
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["code"].clone(),
                    d["data"]["rule"].clone(),
                    d["message"].clone(),
                )
            })
            .collect();
        assert_eq!(
            rules,
            [("led0", fall - DELAY), ("led1", fall)]
                .map(|(part, at)| (
                    "chip.protocol".into(),
                    "bit".into(),
                    format!("#{part} dropped a bit that broke rule bit at {at} ps, the first time")
                        .into()
                ))
                .to_vec(),
            "stop {stop}"
        );
    }
}

/// `packages/engine/tests/circuits/ws2812-replay` cut to 16 LEDs that draw 12 mA a channel and
/// 1 mA idle down to 3.7 V, fed from 5 V through `ohms` on net `vled`, and sent full white.
fn behind(ohms: f64) -> Vec<u8> {
    let path = repo("packages/engine/tests/circuits/ws2812-replay/netlist.json");
    let mut netlist: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    netlist["analysis"]["transient"]["stop_ps"] = 5_000_000_000u64.into();
    netlist["nets"].as_array_mut().unwrap().push("vled".into());
    let amps = |value: f64| serde_json::json!({ "value": value, "unit": "A" });
    let frame: Vec<_> = train(&[255; 48])
        .into_iter()
        .map(|(level, ps)| serde_json::json!({ "level": level, "ps": ps }))
        .collect();
    let parts = netlist["parts"].as_array_mut().unwrap();
    for part in parts.iter_mut() {
        if part["kind"] == "ws2812" {
            part["leds"] = 16.into();
            part["pins"]["VCC"] = "vled".into();
            part["draw"] = serde_json::json!({
                "channel_current": amps(0.012),
                "idle_current": amps(0.001),
                "holds_above": { "value": 3.7, "unit": "V" },
            });
        }
        if part["kind"] == "pulses_replay" {
            part["events"] = serde_json::json!([{ "t_ps": 1_000_000_000u64, "pulses": frame }]);
        }
    }
    parts.push(serde_json::json!({
        "name": "rs",
        "kind": "resistor",
        "pins": { "a": "v5", "b": "vled" },
        "resistance": { "value": ohms, "unit": "ohm" },
        "ratings": { "power": null },
    }));
    serde_json::to_vec(&netlist).unwrap()
}

/// The supply where `ohms` from 5 V carries what the chain draws there, by bisection.
fn balance(ohms: f64, draw: orbe_model::Draw) -> f64 {
    let (mut low, mut high) = (0.0, 5.0);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if (5.0 - mid) / ohms > draw.at(mid).0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

/// Full white on 16 LEDs is 592 mA, far more than 100 ohm or 1 kohm passes from 5 V. The
/// draw fades below 3.7 V, so the chain's supply settles where the resistor carries the faded
/// draw, a few hundred millivolts up, and never goes below ground on the way.
#[test]
fn a_chain_behind_100_ohm_or_1_kohm_settles_above_ground_where_the_resistor_feeds_it() {
    use orbe_engine::Session;
    use orbe_engine::name::Ref;
    use orbe_model::Time;
    let vled: Ref = "#net.vled".parse().unwrap();
    let white = orbe_model::Draw {
        amps: 16.0 * 0.037,
        holds_above_v: 3.7,
    };
    for ohms in [100.0, 1000.0] {
        let json = behind(ohms);
        let mut session = Session::open(netlist::parse(&json).unwrap()).unwrap();
        let mut lowest = f64::INFINITY;
        for k in 1..=500 {
            session.advance_to(Time::from_ps(k * 10_000_000));
            assert!(
                session.fault().is_none(),
                "{ohms} ohm: {:?}",
                session.fault()
            );
            lowest = lowest.min(session.probe().get(&vled).unwrap().get());
        }
        let settled = session.probe().get(&vled).unwrap().get();
        let expected = balance(ohms, white);
        assert!(lowest >= 0.0, "{ohms} ohm: vled fell to {lowest} V");
        assert!(
            (settled - expected).abs() < 1e-3 && settled > 0.05,
            "{ohms} ohm: vled {settled} V against {expected} V"
        );
    }
}
