mod common;

use std::fs;

use common::repo;
use sim_core::timestamp::Timestamp;

/// `examples/npn-led` as a 10 ns transient, with the transistor's current rating at 10 mA. Its
/// collector carries about 13 mA and its base about 4 mA.
fn rated_switch() -> Vec<u8> {
    let mut netlist: serde_json::Value =
        serde_json::from_slice(&fs::read(repo("examples/npn-led/netlist.json")).unwrap()).unwrap();
    netlist["analysis"] = serde_json::json!({
        "transient": { "stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000 }
    });
    let q1 = netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "q1")
        .unwrap();
    q1["ratings"]["current"]["value"] = 0.01.into();
    serde_json::to_vec(&netlist).unwrap()
}

#[test]
fn a_transistor_writes_ib_ic_and_its_pin_power_and_its_current_rating_bounds_ic() {
    let done = sim_cli::run(&rated_switch(), Timestamp::from_unix_seconds(0));
    let index: serde_json::Value =
        serde_json::from_slice(done.folder.file("traces/index.json").unwrap()).unwrap();
    let columns: Vec<(&str, &str)> = index["analog"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| {
            (
                column["ref"].as_str().unwrap(),
                column["unit"].as_str().unwrap(),
            )
        })
        .filter(|(reference, _)| reference.starts_with("#q1."))
        .collect();
    assert_eq!(columns, [("#q1.ib", "A"), ("#q1.ic", "A"), ("#q1.p", "W")]);

    let trace = done.results.trace().unwrap();
    let column = |key: &str| {
        let k = trace
            .columns()
            .iter()
            .position(|reference| reference.to_string() == key)
            .unwrap_or_else(|| panic!("no column {key}"));
        trace.column(k)
    };
    let (c, b) = (column("#net.c"), column("#net.b"));
    let (ib, ic, p) = (column("#q1.ib"), column("#q1.ic"), column("#q1.p"));
    assert!(ic.iter().all(|&ic| ic > 0.013), "{ic:?}");
    assert!(ib.iter().all(|&ib| ib < 0.005), "{ib:?}");
    for row in 0..ic.len() {
        assert_eq!(p[row], c[row] * ic[row] + b[row] * ib[row], "row {row}");
    }

    let diagnostics = serde_json::to_value(done.results.diagnostics()).unwrap();
    let found: Vec<(&str, &str)> = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap(), d["refs"][0].as_str().unwrap()))
        .collect();
    assert_eq!(found, [("rating.current", "#q1.ic")]);
    let peak = diagnostics[0]["data"]["peak"].as_f64().unwrap();
    assert_eq!(peak, ic.iter().copied().fold(0.0, f64::max));
}

/// `examples/nmos-led` as a 10 ns transient, with the MOSFET's current rating at 10 mA.
/// The drain carries the LED current, above that rating, and the gate current is 0 at DC.
#[test]
fn an_nmos_writes_id_and_its_pin_power_and_its_current_rating_bounds_id() {
    let mut netlist: serde_json::Value =
        serde_json::from_slice(&fs::read(repo("examples/nmos-led/netlist.json")).unwrap()).unwrap();
    netlist["analysis"] = serde_json::json!({
        "transient": { "stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000 }
    });
    let m1 = netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == "m1")
        .unwrap();
    m1["ratings"]["current"]["value"] = 0.01.into();
    let done = sim_cli::run(
        &serde_json::to_vec(&netlist).unwrap(),
        Timestamp::from_unix_seconds(0),
    );
    let index: serde_json::Value =
        serde_json::from_slice(done.folder.file("traces/index.json").unwrap()).unwrap();
    let columns: Vec<(&str, &str)> = index["analog"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| {
            (
                column["ref"].as_str().unwrap(),
                column["unit"].as_str().unwrap(),
            )
        })
        .filter(|(reference, _)| reference.starts_with("#m1."))
        .collect();
    assert_eq!(columns, [("#m1.id", "A"), ("#m1.p", "W")]);

    let trace = done.results.trace().unwrap();
    let column = |key: &str| {
        let k = trace
            .columns()
            .iter()
            .position(|reference| reference.to_string() == key)
            .unwrap_or_else(|| panic!("no column {key}"));
        trace.column(k)
    };
    let (d, id, p) = (column("#net.d"), column("#m1.id"), column("#m1.p"));
    assert!(id.iter().all(|&id| id > 0.01), "{id:?}");
    for row in 0..id.len() {
        assert!(
            (p[row] - d[row] * id[row]).abs() <= 1e-15,
            "row {row}: {} vs {}",
            p[row],
            d[row] * id[row]
        );
    }

    let diagnostics = serde_json::to_value(done.results.diagnostics()).unwrap();
    let found: Vec<(&str, &str)> = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["code"].as_str().unwrap(), d["refs"][0].as_str().unwrap()))
        .collect();
    assert_eq!(found, [("rating.current", "#m1.id")]);
    let peak = diagnostics[0]["data"]["peak"].as_f64().unwrap();
    assert_eq!(peak, id.iter().copied().fold(0.0, f64::max));
}
