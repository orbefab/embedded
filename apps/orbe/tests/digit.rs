//! What a single 7-segment display reads on a Nano: a new digit within a millisecond and never
//! a mix of two, a dim segment as part of the digit, PWM and multiplexed drive as a steady digit,
//! and an open common as blank.

mod live;
#[path = "registry_env.rs"]
mod registry_env;

use std::fs;

use live::{common, firmware, orbe, project, replies_in};
use tempfile::TempDir;

const LETTERS: [&str; 7] = ["a", "b", "c", "d", "e", "f", "g"];

/// A Nano running `hex`, with segment a to g of `display` through `resistance` each to D2 to D8.
/// `g` overrides segment g's resistor, `extra` adds wires, and `com` is where the common goes.
fn display(
    hex: &std::path::Path,
    display: &str,
    resistance: &str,
    g: &str,
    com: Option<&str>,
    extra: &str,
    test: &str,
) -> TempDir {
    let dir = project(test);
    fs::copy(hex, dir.path().join("build/button.ino.hex")).unwrap();
    let mut parts = String::new();
    let mut wires = String::new();
    for (k, letter) in LETTERS.iter().enumerate() {
        let ohms = if *letter == "g" { g } else { resistance };
        parts.push_str(&format!(
            r#""r{letter}": {{ "type": "resistor-axial", "resistance": "{ohms}" }}, "#
        ));
        wires.push_str(&format!(
            r#"["disp.{letter}", "r{letter}.a"], ["r{letter}.b", "nano.D{}"], "#,
            k + 2
        ));
    }
    if let Some(com) = com {
        wires.push_str(&format!(r#"["disp.com", "{com}"], "#));
    }
    wires.push_str(extra);
    fs::write(
        dir.path().join("scene.jsonc"),
        format!(
            r#"{{ "format": 1, "parts": {{ "nano": {{ "type": "nano-board", "firmware": "build/button.ino.hex" }}, "disp": {{ "type": "{display}" }}, {parts} }}, "wires": [{wires}] }}"#
        ),
    )
    .unwrap();
    dir
}

/// The counter Block's firmware: 0 to 9, each digit for 200 ms, from D2 to D8.
fn count() -> std::path::PathBuf {
    common::repo().join("blocks/nano-7seg-count/build/count.ino.hex")
}

/// `open`, then `step <first>ms`, then `samples` times `step 1ms` and `get @disp.digit`.
fn every_millisecond(first: u64, samples: usize) -> String {
    let mut test = format!("open\nstep {first}ms\n");
    for _ in 0..samples {
        test.push_str("step 1ms\nget @disp.digit\n");
    }
    test
}

/// Each `get @disp.digit` reply as its time in ms and the digit it read.
fn readings(dir: &TempDir) -> Vec<(u64, String)> {
    replies_in(dir)
        .iter()
        .filter(|reply| reply["data"]["kind"] == "shown")
        .map(|reply| {
            (
                reply["t_ps"].as_u64().unwrap() / 1_000_000_000,
                reply["data"]["value"]["shown"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect()
}

fn passes(dir: &TempDir) {
    let run = orbe(dir.path(), &["test", "-v"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
}

/// The counter turns 1 into 2 at 400.08 ms. The next millisecond reads 2, and no reading
/// shows segments of both.
#[test]
fn a_new_digit_reads_within_a_millisecond_and_never_mixes_with_the_old_one() {
    let dir = display(
        &count(),
        "display-7seg-cc",
        "330",
        "330",
        Some("nano.GND"),
        "",
        &every_millisecond(395, 25),
    );
    let read = readings(&dir);
    assert_eq!(read.len(), 25);
    for (ms, digit) in &read {
        let want = if *ms <= 400 { "1" } else { "2" };
        assert_eq!(digit, want, "at {ms} ms: {read:?}");
    }
}

/// The common cathode on D10 is low 55 of each 255 steps of its 490 Hz PWM, about 1.25 mA
/// average per segment through 560 ohm. Every segment glows dim and the digit reads 8 at each
/// millisecond.
#[test]
fn a_pwm_dimmed_digit_reads_steadily() {
    let mut test = every_millisecond(100, 40);
    test.push_str("expect @disp.seg_a dim\nexpect @disp.seg_g dim\nexpect no diagnostics\n");
    let dir = display(
        &firmware("seg_dim"),
        "display-7seg-cc",
        "560",
        "560",
        Some("nano.D10"),
        "",
        &test,
    );
    let read = readings(&dir);
    assert_eq!(read.len(), 40);
    assert!(read.iter().all(|(_, digit)| digit == "8"), "{read:?}");
    passes(&dir);
}

/// Segment g through 4.7k carries about 0.68 mA while the counter shows 8. It glows dim and
/// still makes the 8.
#[test]
fn a_dim_segment_counts_toward_the_digit() {
    let dir = display(
        &count(),
        "display-7seg-cc",
        "330",
        "4.7k",
        Some("nano.GND"),
        "",
        "open\nstep 1650ms\nexpect @disp.seg_g dim\nexpect @disp.digit 8\n",
    );
    passes(&dir);
}

/// The cathode is low for 2.5 ms of each 10 ms frame, so the display is dark three quarters
/// of the time, and the digit changes every 200 ms. Every millisecond reads a digit, never
/// blank or a mix, and the digits only count up.
#[test]
fn a_digit_multiplexed_at_100_hz_reads_steadily_through_each_change() {
    let dir = display(
        &firmware("seg_mux"),
        "display-7seg-cc",
        "330",
        "330",
        Some("nano.D10"),
        "",
        &every_millisecond(30, 600),
    );
    let read = readings(&dir);
    assert_eq!(read.len(), 600);
    let digits: Vec<u32> = read
        .iter()
        .map(|(ms, digit)| {
            digit
                .parse()
                .unwrap_or_else(|_| panic!("at {ms} ms the display read {digit}"))
        })
        .collect();
    assert!(digits.windows(2).all(|w| w[0] <= w[1]), "{read:?}");
    assert_eq!(digits.first(), Some(&0));
    assert_eq!(digits.last(), Some(&3));
}

/// Each digit 0 to 9 on either display, with the decimal point off and then on, reads that
/// digit, and the decimal point alone reads blank. The common-anode display has its common on
/// 5V and D11 on GND, so the sketch drives each segment low to light it.
#[test]
fn every_digit_reads_on_both_displays_with_the_decimal_point_off_and_on() {
    let mut test = String::from("open\nstep 50ms\n");
    for n in 0..10 {
        test.push_str(&format!(
            "expect @disp.digit {n}\nexpect @disp.seg_dp off\nstep 100ms\n\
             expect @disp.digit {n}\nexpect @disp.seg_dp lit\nstep 100ms\n"
        ));
    }
    test.push_str("expect @disp.seg_dp lit\nexpect @disp.digit blank\nexpect no diagnostics\n");
    for (part, com, extra) in [
        ("display-7seg-cc", "nano.GND", ""),
        (
            "display-7seg-ca",
            "nano.5V",
            r#"["nano.D11", "nano.GND"], "#,
        ),
    ] {
        let dir = display(
            &firmware("seg_count"),
            part,
            "330",
            "330",
            Some(com),
            &format!(r#"["disp.dp", "rp.a"], ["rp.b", "nano.D9"], {extra}"#),
            &test,
        );
        live::edit_scene(
            &dir,
            r#""ra":"#,
            r#""rp": { "type": "resistor-axial", "resistance": "330" }, "ra":"#,
        );
        passes(&dir);
    }
}

/// A common-anode display with its common on no net, each segment and the decimal point through
/// its own 330 ohm to D2 to D9. No segment has a supply, so the session opens and reads blank.
#[test]
fn an_unwired_common_anode_opens_and_reads_blank() {
    let dir = display(
        &count(),
        "display-7seg-ca",
        "330",
        "330",
        None,
        r#"["disp.dp", "rp.a"], ["rp.b", "nano.D9"]"#,
        "open\nstep 300ms\nexpect @disp.digit blank\nexpect no diagnostics\n",
    );
    live::edit_scene(
        &dir,
        r#""ra":"#,
        r#""rp": { "type": "resistor-axial", "resistance": "330" }, "ra":"#,
    );
    passes(&dir);
}
