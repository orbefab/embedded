//! `chip.floating_input` on `packages/engine/tests/circuits/atmega-button`: PD2 reads a button
//! to ground. `button.S` turns PD2's pull-up on; with that `SBI` made a `CBI`, the pull-up stays
//! off and nothing but the chip's own leakage holds PD2's net while the button is open.

use std::fs;

use crate::runs::common::repo;
use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::timestamp::Timestamp;

/// `button.hex` with `SBI PORTD,2` (`0x9A5A`) made `CBI PORTD,2` (`0x985A`), its checksum 2 up.
const NO_PULL_UP: &str =
    ":1000000052985A98259A09B1009502FB15B115F935\n:0400100015B9F9CF56\n:00000001FF\n";

/// `ldi r24,0`, `ldi r25,0`, then `adiw r24,1` and `brne` 65,536 times, 4 cycles each, then
/// `in r16,PIND` on a loop: the first read of PD2 is at cycle 262,148, 16.38 ms in, and no
/// drive ever changes.
const LATE_READ: &str = ":0C00000080E090E00196F1F709B1FECF1E\n:00000001FF\n";

/// The fixture cut to 1 ms with the button held `initial` until `release_ps`, if ever.
fn netlist(pull_up: bool, initial: &str, release_ps: Option<u64>) -> serde_json::Value {
    let text = fs::read_to_string(repo(
        "packages/engine/tests/circuits/atmega-button/netlist.json",
    ))
    .unwrap();
    let mut netlist: serde_json::Value = serde_json::from_str(&text).unwrap();
    netlist["analysis"]["transient"] = serde_json::json!({
        "stop_ps": 1_000_000_000u64, "max_step_ps": 100_000_000u64, "grid_ps": 100_000_000u64
    });
    let events: Vec<_> = release_ps
        .map(|t_ps| serde_json::json!({ "t_ps": t_ps, "state": "open" }))
        .into_iter()
        .collect();
    let switch = part(&mut netlist, "sw1");
    switch["initial"] = initial.into();
    switch["events"] = events.into();
    switch["bounce"] = serde_json::Value::Null;
    if !pull_up {
        part(&mut netlist, "u1")["firmware"] = NO_PULL_UP.into();
    }
    netlist
}

fn part<'n>(netlist: &'n mut serde_json::Value, name: &str) -> &'n mut serde_json::Value {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["name"] == name)
        .unwrap()
}

fn resistor(netlist: &mut serde_json::Value, a: &str, b: &str) {
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "r2", "kind": "resistor", "pins": { "a": a, "b": b },
            "resistance": { "value": 10_000.0, "unit": "ohm" },
            "ratings": { "power": null }
        }));
}

fn floating(netlist: &serde_json::Value) -> Vec<Diagnostic> {
    orbe_engine::run(
        netlist.to_string().as_bytes(),
        Timestamp::from_unix_seconds(0),
    )
    .results
    .diagnostics()
    .iter()
    .filter(|d| d.code() == "chip.floating_input")
    .cloned()
    .collect()
}

#[test]
fn a_read_input_with_its_pull_up_off_and_its_button_open_warns_once_naming_its_pin() {
    let found = floating(&netlist(false, "open", None));
    assert_eq!(found.len(), 1, "{found:?}");
    let refs: Vec<String> = found[0].refs().iter().map(ToString::to_string).collect();
    assert_eq!(refs, ["#u1.pin.PD2", "#net.pd2"]);
    // The loop's first `IN PIND` is at cycle 6 at 16 MHz; the warning is at the step after it.
    let at = found[0].at().unwrap().as_ps();
    assert!((375_000..100_000_000).contains(&at), "at {at} ps");
    let json = serde_json::to_value(&found[0]).unwrap();
    assert_eq!(json["severity"], "warning");
    let message = json["message"].as_str().unwrap();
    assert!(
        message.starts_with(&format!(
            "#u1.pin.PD2 floats on #net.pd2 at {at} ps, and the firmware has taken in its level since the pin last became an input with its pull-up off"
        )),
        "{message}"
    );
    assert!(
        message.ends_with("add a pull-up or a pull-down resistor, or turn the pin's pull-up on")
    );
}

/// A landing commits only a core that stops there, so the copy that answers when the
/// firmware first read a pin is the one run ahead.
#[test]
fn a_read_long_after_the_last_drive_change_warns_at_that_read() {
    let mut netlist = netlist(true, "open", None);
    part(&mut netlist, "u1")["firmware"] = LATE_READ.into();
    netlist["analysis"]["transient"]["stop_ps"] = 30_000_000_000u64.into();
    netlist["analysis"]["transient"]["max_step_ps"] = 1_000_000_000u64.into();
    let found = floating(&netlist);
    assert_eq!(found.len(), 1, "{found:?}");
    let first_read = 262_148 * 62_500;
    let at = found[0].at().unwrap().as_ps();
    assert!(
        (first_read..first_read + 1_000_000_000).contains(&at),
        "at {at} ps"
    );
}

#[test]
fn the_pins_own_pull_up_keeps_it_quiet() {
    assert!(floating(&netlist(true, "open", None)).is_empty());
}

#[test]
fn a_resistor_to_ground_or_to_the_supply_keeps_it_quiet() {
    for rail in ["gnd", "vcc"] {
        let mut netlist = netlist(false, "open", None);
        resistor(&mut netlist, "pd2", rail);
        assert!(floating(&netlist).is_empty(), "a resistor to {rail}");
    }
}

/// `button.S` drives PB5 low from cycle 6, so a resistor from PD2 to PB5 pulls PD2 down.
#[test]
fn a_resistor_to_a_pin_the_chip_drives_keeps_it_quiet() {
    let mut netlist = netlist(false, "open", None);
    resistor(&mut netlist, "pd2", "pb5");
    assert!(floating(&netlist).is_empty());
}

/// A closed button joins PD2 to ground. Once it opens the next accepted step warns.
#[test]
fn a_closed_button_holds_the_input_until_it_opens() {
    assert!(floating(&netlist(false, "closed", None)).is_empty());
    let found = floating(&netlist(false, "closed", Some(500_000_000)));
    assert_eq!(found.len(), 1, "{found:?}");
    let at = found[0].at().unwrap().as_ps();
    assert!((500_000_000..600_000_000).contains(&at), "at {at} ps");
}

/// `sbi PORTD,2`, `in r16,PIND`, `cbi PORTD,2`, `rjmp .`: the only read of PD2 is while its
/// pull-up is on, and the pull-up then turns off for good.
const READ_UNDER_PULL_UP: &str = ":080000005A9A09B15A98FFCF8A\n:00000001FF\n";

/// `in r16,PIND`, `sbi PORTD,2`, `rjmp .`: PD2 is read while its pull-up is still off, then
/// the pull-up turns on.
const READ_THEN_PULL_UP: &str = ":0600000009B15A9AFFCF7E\n:00000001FF\n";

/// `in r16,PIND`, `sbi PORTD,2`, `cbi PORTD,2`, `rjmp .`: PD2 is read with its pull-up off,
/// then the pull-up turns on and off again, and PD2 is never read after.
const READ_THEN_PULL_UP_ON_AND_OFF: &str = ":0800000009B15A9A5A98FFCF8A\n:00000001FF\n";

/// A read under the pull-up reads a held level. Once the pull-up turns off nothing reads PD2,
/// so the open button warns of nothing.
#[test]
fn a_read_under_the_pull_up_is_no_read_once_the_pull_up_turns_off() {
    let mut netlist = netlist(false, "open", None);
    part(&mut netlist, "u1")["firmware"] = READ_UNDER_PULL_UP.into();
    assert_eq!(floating(&netlist), []);
}

/// The read is while PD2 floats, so the step where the pull-up turns on still warns.
#[test]
fn a_read_before_the_pull_up_turns_on_warns() {
    let mut netlist = netlist(false, "open", None);
    part(&mut netlist, "u1")["firmware"] = READ_THEN_PULL_UP.into();
    let found = floating(&netlist);
    assert_eq!(found.len(), 1, "{found:?}");
    let at = found[0].at().unwrap().as_ps();
    assert!(at <= 1_000_000, "at {at} ps");
}

/// The closed button holds PD2 at its only read. The pull-up turns on and off after it, so
/// once the button opens at 0.5 ms PD2 floats with no read since its pull-up went off.
#[test]
fn a_read_before_the_pull_up_turned_on_and_off_is_no_read_of_the_later_float() {
    let mut netlist = netlist(false, "closed", Some(500_000_000));
    part(&mut netlist, "u1")["firmware"] = READ_THEN_PULL_UP_ON_AND_OFF.into();
    assert_eq!(floating(&netlist), []);
}
