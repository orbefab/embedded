//! The stock `analogWrite(18, 128)` sketch on the DevKitC-1: GPIO18 through 220 ohm into an
//! LED, at the core's default 1 kHz and 8 bits. The fixture's app and partition table are the
//! images named in `packages/engine/tests/firmware/esp32s3/arduino/stock/NOTICE`.

mod common;

/// The run, past the boot and some 290 PWM edges.
const STOP_PS: u64 = 300_000_000_000;

/// Accepted steps a GPIO18 edge, over the whole run, boot included. The board's auto-reset
/// transistors sit idle beside the pin with junctions of about 11 pF that ripple a few
/// microvolts. Held to 1 nV, they kept each edge to about 78 steps, and rejected 5 tries an
/// edge.
const MOST_STEPS_AN_EDGE: u64 = 25;

#[test]
fn a_devkit_pwm_edge_takes_a_few_steps_and_none_is_rejected() {
    let project = common::project("esp32s3-stock-analogwrite");
    let netlist = orbe_scene::read(
        &orbe_scene::Files::disk(),
        &project.path().join("scene.jsonc"),
    )
    .unwrap_or_else(|found| panic!("{found:#?}"));
    let mut json: serde_json::Value = serde_json::from_str(&netlist.json).unwrap();
    json["analysis"]["transient"]["stop_ps"] = STOP_PS.into();
    let parsed =
        orbe_engine::netlist::parse_with(json.to_string().as_bytes(), &netlist.blobs).unwrap();
    let (results, stats) = orbe_engine::simulate_stats(&parsed);
    assert!(
        results.diagnostics().is_empty(),
        "{:?}",
        results.diagnostics()
    );
    let logic = results.trace().unwrap().logic().unwrap();
    let pin = logic
        .signals()
        .iter()
        .position(|signal| signal.to_string().ends_with("GPIO18"))
        .expect("GPIO18 has a logic signal");
    let edges = logic
        .changes()
        .iter()
        .filter(|change| change.signal == pin)
        .count() as u64;
    eprintln!(
        "{} steps, {} rejected, {edges} GPIO18 edges",
        stats.landings, stats.rejections
    );
    assert!(edges > 500, "GPIO18 changed {edges} times");
    assert!(
        stats.landings <= MOST_STEPS_AN_EDGE * edges,
        "{} steps over {edges} edges",
        stats.landings
    );
    assert_eq!(stats.rejections, 0, "rejected tries");
}
