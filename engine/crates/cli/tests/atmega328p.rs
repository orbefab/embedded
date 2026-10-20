use std::fs;
use std::path::PathBuf;

use sim_core::name::Ref;
use sim_core::timestamp::Timestamp;

/// A cycle of the document's 16 MHz crystal.
const CYCLE_PS: u64 = 62_500;

fn repo(path: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap()
}

fn documented() -> String {
    let doc = repo("docs/netlist.md");
    let section = &doc[doc.find("### ATmega328P").unwrap()..];
    let start = section.find("```json\n").unwrap() + "```json\n".len();
    let end = start + section[start..].find("\n```").unwrap();
    section[start..end].to_string()
}

#[test]
fn the_documented_example_runs_with_no_diagnostics() {
    let done = sim_cli::run(documented().as_bytes(), Timestamp::from_unix_seconds(0));
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
    assert_eq!(done.results.trace().unwrap().time().len(), 21);
}

fn chip(netlist: &mut serde_json::Value) -> &mut serde_json::Value {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["kind"] == "atmega328p")
        .unwrap()
}

fn leakage() -> f64 {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    chip(&mut netlist)["leakage_resistance"]["value"]
        .as_f64()
        .unwrap()
}

fn value(results: &sim_core::run_folder::Results, reference: &str) -> f64 {
    let op = results.observations().op().unwrap();
    op.values[&reference.parse::<Ref>().unwrap()].get()
}

/// The documented example at its operating point, with `parts` added on `nets`.
fn probe(
    nets: &[&str],
    parts: serde_json::Value,
    edit: impl FnOnce(&mut serde_json::Value),
) -> sim_cli::Completed {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    netlist["analysis"] = serde_json::json!({ "op": {} });
    for net in nets {
        netlist["nets"].as_array_mut().unwrap().push((*net).into());
    }
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .extend(parts.as_array().unwrap().iter().cloned());
    edit(chip(&mut netlist));
    sim_cli::run(
        netlist.to_string().as_bytes(),
        Timestamp::from_unix_seconds(0),
    )
}

#[test]
fn the_documented_part_is_the_tests_fixture() {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(&repo("crates/core/tests/fixtures/atmega328p.json")).unwrap();
    assert_eq!(*chip(&mut netlist), fixture);
}

#[test]
fn the_documented_firmware_is_toggle_hex() {
    let mut netlist: serde_json::Value = serde_json::from_str(&documented()).unwrap();
    let chip = chip(&mut netlist);
    let records = |text: &str| text.lines().map(str::to_string).collect::<Vec<_>>();
    assert_eq!(
        records(chip["firmware"].as_str().unwrap()),
        records(&repo("firmware/toggle.hex"))
    );
}

/// `toggle.S` drives PB5 low at cycle 2, onto a pin its load already holds at 0 V, then toggles
/// it at cycle 4 and every 100 cycles. Each edge is a row on the old level at its cycle's
/// picosecond, and the restart's 1 ns step after it holds the new one.
#[test]
fn pb5_switches_at_the_picosecond_of_each_cycle_the_firmware_implies() {
    let netlist = sim_core::netlist::parse(documented().as_bytes()).unwrap();
    let results = sim_circuit::simulate_steps(&netlist);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let trace = results.trace().unwrap();
    let pb5: Ref = "#net.pb5".parse().unwrap();
    let column = trace.columns().iter().position(|r| *r == pb5).unwrap();
    let rows: Vec<(u64, f64)> = trace
        .time()
        .iter()
        .map(|at| at.as_ps())
        .zip(trace.column(column).iter().copied())
        .collect();
    let edges: Vec<(u64, bool)> = rows
        .windows(2)
        .filter(|pair| (pair[0].1 > 2.5) != (pair[1].1 > 2.5))
        .map(|pair| {
            assert_eq!(
                pair[1].0,
                pair[0].0 + 1_000,
                "a restart at {} ps",
                pair[0].0
            );
            (pair[0].0, pair[1].1 > 2.5)
        })
        .collect();
    let leak = 1.0 / leakage();
    let share = |up: f64, down: f64| 5.0 * up / (up + down);
    let levels = [
        share(leak, leak + 1e-3),
        share(leak, leak + 1e-3 + 1.0 / 25.0),
        share(leak + 1.0 / 25.0, leak + 1e-3),
    ];
    assert!(
        rows.iter()
            .all(|&(_, v)| levels.iter().any(|level| (v - level).abs() < 1e-9)),
        "off, low, or high on 1 kohm, each with the pin's leakage"
    );
    assert_eq!(
        edges,
        [
            (4 * CYCLE_PS, true),
            (104 * CYCLE_PS, false),
            (204 * CYCLE_PS, true),
            (304 * CYCLE_PS, false),
        ]
    );
}

/// 1 uA, the leakage bound of Table 30-1, into an off pin through 1 kohm. The pin's leakage to
/// each rail holds it, so the net solves at 2.5 V plus 1 uA through half the leakage
/// resistance and the 1 kohm.
#[test]
fn a_net_held_only_by_an_off_pin_solves_on_its_leakage() {
    let parts = serde_json::json!([
        { "name": "i1", "kind": "current_source", "pins": { "p": "x", "n": "gnd" },
          "current": { "value": 1e-6, "unit": "A", "source": "user" } },
        { "name": "rx", "kind": "resistor", "pins": { "a": "x", "b": "pd3" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null } }
    ]);
    let done = probe(&["x"], parts, |_| {});
    assert!(
        done.results.diagnostics().is_empty(),
        "{:?}",
        done.results.diagnostics()
    );
    let expected = 2.5 + 1e-6 * (leakage() / 2.0 + 1000.0);
    let x = value(&done.results, "#net.x");
    assert!(
        (x / expected - 1.0).abs() < 1e-5,
        "{x} within gmin's share of {expected}"
    );
}

#[test]
fn a_ground_pin_on_a_net_of_its_own_is_an_error() {
    let parts = serde_json::json!([
        { "name": "ra", "kind": "resistor", "pins": { "a": "vcc", "b": "agnd" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null } }
    ]);
    let done = probe(&["agnd"], parts, |chip| {
        chip["pins"]["GND_22"] = "agnd".into()
    });
    let codes: Vec<&str> = done
        .results
        .diagnostics()
        .iter()
        .map(|d| d.code())
        .collect();
    assert_eq!(codes, ["netlist.same_net"]);
}
