//! packages/engine/tests/circuits/npn-blinker. Two small-signal NPNs blink two LEDs.
//! The supply steps from 0 V to 5 V at 1 ns.

use std::sync::OnceLock;

use crate::common;
use orbe_engine_core::diagnostic::Diagnostic;
use orbe_engine_core::netlist::{self, Analysis, Netlist};
use orbe_engine_core::run_folder::Results;
use orbe_engine_core::timestamp::Timestamp;

/// An LED is on above this current. The on current is about 6.5 mA and the off current is microamps.
const LED_ON_A: f64 = 1e-3;

/// The first collector crosses this on the way down. It swings from about 0.08 V to about 3.9 V.
const CROSS_V: f64 = 2.5;

/// Exclusive runs shorter than this are the startup, a sub-microsecond spike at the supply step
/// and then about 13 ms of the second LED alone. The steady on times are 125 ms and 271 ms.
const STEADY_ON_S: f64 = 0.05;

/// Steady on time of the LED on the first collector. The 10 uF capacitor sets it.
const LED1_ON_S: f64 = 0.271;

/// Steady on time of the LED on the second collector. The 4.7 uF capacitor sets it.
const LED2_ON_S: f64 = 0.125;

/// Both LEDs conduct together for about this long after the supply steps up.
const BOTH_ON_S: f64 = 0.009;

/// Falling edges of `v(c1)` through 2.5 V, from [`NGSPICE_DECK`].
/// Write that deck to a file, run `ngspice -b`, and take each falling crossing linear between
/// ngspice's points. The mean of the intervals after the first is [`NGSPICE_STEADY_PERIOD_S`].
const NGSPICE_EDGES_S: [f64; 5] = [
    0.021429111241055218,
    0.40573671089198204,
    0.8158691953753351,
    1.226022588025449,
    1.6361759653102375,
];

/// Mean spacing of [`NGSPICE_EDGES_S`] after the first interval.
const NGSPICE_STEADY_PERIOD_S: f64 = 0.4101464181394185;

/// The ngspice-42 run that produced [`NGSPICE_EDGES_S`]. 11958 points over 2 s.
/// The script's deck adds 0 V ammeters and sets `reltol` to 1e-9 and `vntol` to 1e-12, and
/// ngspice aborts that deck at the supply step.
const NGSPICE_DECK: &str = r#"* npn-blinker at reltol=1e-6. The supply steps from 0 V to 5 V at 1 ns. The script deck adds 0 V ammeters, sets reltol to 1e-9 and vntol to 1e-12, and ngspice aborts that deck.
.options temp=25 tnom=25 reltol=1e-6 abstol=1e-15 vntol=1e-9 gmin=1e-12
cca c1 b2 1e-05
ccb c2 b1 4.7e-06
dled1 a1 c1 dmod_led1
.model dmod_led1 d (is=6.711e-10 n=4.427 rs=2.115)
dled2 a2 c2 dmod_led2
.model dmod_led2 d (is=6.711e-10 n=4.427 rs=2.115)
qq1 c1 b1 0 qmod_q1
.model qmod_q1 npn (is=8.095e-15 bf=173.2 br=0.2704 nf=1.0 nr=1.0 vaf=100.0 rb=5.707 rc=1.531 re=0.867 cje=8e-12 cjc=4e-12 mje=0 mjc=0)
qq2 c2 b2 0 qmod_q2
.model qmod_q2 npn (is=8.095e-15 bf=173.2 br=0.2704 nf=1.0 nr=1.0 vaf=100.0 rb=5.707 rc=1.531 re=0.867 cje=8e-12 cjc=4e-12 mje=0 mjc=0)
rrb1 vcc b1 47000.0
rrb2 vcc b2 47000.0
rrc1 vcc a1 470.0
rrc2 vcc a2 470.0
vv1 vcc 0 pwl(0 0 1n 0 1.001n 5)
* The engine's gmin on every node, beside the gmin ngspice puts across each junction.
rx_gmin_a1 a1 0 1000000000000.0
rx_gmin_a2 a2 0 1000000000000.0
rx_gmin_b1 b1 0 1000000000000.0
rx_gmin_b2 b2 0 1000000000000.0
rx_gmin_c1 c1 0 1000000000000.0
rx_gmin_c2 c2 0 1000000000000.0
rx_gmin_vcc vcc 0 1000000000000.0
.control
save v(c1) v(c2)
tran 2000000000p 2000000000000p 0 200000000p
set filetype=ascii
write tran.raw
.endc
.end
"#;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Led {
    One,
    Two,
    Both,
    Off,
}

struct Solved {
    diagnostics: Vec<Diagnostic>,
    times: Vec<u64>,
    c1: Vec<f64>,
    led1: Vec<f64>,
    led2: Vec<f64>,
}

struct Blink {
    shipped: Solved,
    fine: Solved,
    folder_rows: u64,
    folder_expected: u64,
    folder_diagnostics: Vec<Diagnostic>,
}

fn example() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(common::repo(
            "packages/engine/tests/circuits/npn-blinker/netlist.json",
        ))
        .unwrap(),
    )
    .unwrap()
}

fn netlist_of(json: &serde_json::Value) -> Netlist {
    netlist::parse(&serde_json::to_vec(json).unwrap()).unwrap()
}

fn with_step(json: &serde_json::Value, max_step_ps: u64) -> Netlist {
    let mut json = json.clone();
    json["analysis"]["transient"]["max_step_ps"] = max_step_ps.into();
    netlist_of(&json)
}

fn column(results: &Results, key: &str) -> Vec<f64> {
    let trace = results.trace().unwrap();
    let index = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == key)
        .unwrap_or_else(|| panic!("no column {key}"));
    trace.column(index).to_vec()
}

fn solve(netlist: &Netlist) -> Solved {
    let results = orbe_circuit::simulate_steps(netlist);
    let diagnostics = results.diagnostics().to_vec();
    let Some(trace) = results.trace() else {
        return Solved {
            diagnostics,
            times: Vec::new(),
            c1: Vec::new(),
            led1: Vec::new(),
            led2: Vec::new(),
        };
    };
    let times = trace.time().iter().map(|time| time.as_ps()).collect();
    Solved {
        diagnostics,
        times,
        c1: column(&results, "#net.c1"),
        led1: column(&results, "#led1.i"),
        led2: column(&results, "#led2.i"),
    }
}

fn blink() -> &'static Blink {
    static CELL: OnceLock<Blink> = OnceLock::new();
    CELL.get_or_init(|| {
        let json = example();
        let netlist = netlist_of(&json);
        let &Analysis::Transient(transient) = netlist.analysis() else {
            panic!("npn-blinker is a transient");
        };
        let bytes = serde_json::to_vec(&json).unwrap();
        let fine_netlist = with_step(&json, transient.max_step / 10);
        let (shipped, fine, folder) = std::thread::scope(|scope| {
            let shipped = scope.spawn(|| solve(&netlist));
            let fine = scope.spawn(|| solve(&fine_netlist));
            let folder = scope.spawn(|| {
                let done = orbe_circuit::run(&bytes, Timestamp::from_unix_seconds(0));
                let rows = done
                    .results
                    .trace()
                    .map(|trace| trace.time().len() as u64)
                    .unwrap_or(0);
                (rows, done.results.diagnostics().to_vec())
            });
            (
                shipped.join().unwrap(),
                fine.join().unwrap(),
                folder.join().unwrap(),
            )
        });
        Blink {
            shipped,
            fine,
            folder_rows: folder.0,
            folder_expected: transient.rows(),
            folder_diagnostics: folder.1,
        }
    })
}

fn seconds(picoseconds: u64) -> f64 {
    picoseconds as f64 * 1e-12
}

fn falling_edges(times: &[u64], samples: &[f64]) -> Vec<f64> {
    let mut out = Vec::new();
    for index in 1..times.len() {
        let (before, after) = (samples[index - 1], samples[index]);
        if before >= CROSS_V && after < CROSS_V {
            let frac = (CROSS_V - before) / (after - before);
            let dt = (times[index] - times[index - 1]) as f64;
            out.push(seconds(times[index - 1]) + frac * dt * 1e-12);
        }
    }
    out
}

/// Completed spans of one LED state. A span still open at the last sample is left out.
fn segments(times: &[u64], led1: &[f64], led2: &[f64]) -> Vec<(Led, f64)> {
    let mut events = Vec::new();
    for (led, samples) in [(0usize, led1), (1, led2)] {
        for index in 1..times.len() {
            let before = samples[index - 1];
            let after = samples[index];
            if (before > LED_ON_A) == (after > LED_ON_A) {
                continue;
            }
            let frac = (LED_ON_A - before) / (after - before);
            let when =
                seconds(times[index - 1]) + frac * (times[index] - times[index - 1]) as f64 * 1e-12;
            events.push((when, led, after > LED_ON_A));
        }
    }
    events.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut state = [led1[0] > LED_ON_A, led2[0] > LED_ON_A];
    let mut at = seconds(times[0]);
    let mut out = Vec::new();
    for (when, led, high) in events {
        if when > at {
            out.push((side(state), when - at));
        }
        state[led] = high;
        at = when;
    }
    out
}

fn side(state: [bool; 2]) -> Led {
    match state {
        [true, false] => Led::One,
        [false, true] => Led::Two,
        [true, true] => Led::Both,
        [false, false] => Led::Off,
    }
}

fn steady_period(edges: &[f64]) -> f64 {
    let intervals: Vec<f64> = edges.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        intervals.len() >= 3,
        "need three intervals, got {intervals:?}"
    );
    let steady = &intervals[1..];
    steady.iter().sum::<f64>() / steady.len() as f64
}

fn median(values: &[f64]) -> f64 {
    assert!(!values.is_empty(), "no on times");
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn assert_finished(solved: &Solved) {
    assert!(solved.diagnostics.is_empty(), "{:?}", solved.diagnostics);
    assert!(!solved.times.is_empty(), "the run wrote no trace");
}

fn assert_edges_close(left: &[f64], right: &[f64], scale: f64) {
    assert_eq!(left.len(), right.len(), "{left:?} vs {right:?}");
    for (earlier, later) in left.iter().zip(right) {
        let error = (earlier - later).abs() / scale;
        assert!(
            error <= 1e-4,
            "{earlier} s and {later} s differ by {error:e} of the {scale} s period"
        );
    }
}

fn assert_on_time(got: f64, want: f64, led: &str) {
    let relative = got / want - 1.0;
    assert!(
        relative.abs() <= 1e-2,
        "{led} on time {got} s is {relative:e} from {want} s"
    );
}

#[test]
fn the_run_has_no_diagnostics() {
    let blink = blink();
    assert!(
        blink.folder_diagnostics.is_empty(),
        "{:?}",
        blink.folder_diagnostics
    );
    assert_eq!(blink.folder_rows, blink.folder_expected);
    assert_finished(&blink.shipped);
}

#[test]
fn the_leds_alternate() {
    let solved = &blink().shipped;
    assert_finished(solved);
    assert!(
        solved.led1[0] < LED_ON_A && solved.led2[0] < LED_ON_A,
        "the supply starts at 0 V, led1 {} A and led2 {} A",
        solved.led1[0],
        solved.led2[0]
    );
    let spans = segments(&solved.times, &solved.led1, &solved.led2);
    let flash = spans
        .iter()
        .find(|(led, dt)| *led == Led::Both && *dt > 1e-3);
    let Some((Led::Both, dt)) = flash else {
        panic!("no shared flash, {spans:?}");
    };
    assert!((dt - BOTH_ON_S).abs() <= 1e-3, "both LEDs on for {dt} s");
    let long: Vec<(Led, f64)> = spans
        .into_iter()
        .filter(|(led, dt)| matches!(led, Led::One | Led::Two) && *dt >= STEADY_ON_S)
        .collect();
    assert!(long.len() >= 6, "{long:?}");
    assert!(
        long.windows(2).all(|pair| pair[0].0 != pair[1].0),
        "{long:?}"
    );
    assert_eq!(
        long[0].0,
        Led::One,
        "the 10 uF capacitor makes LED1 lead, {long:?}"
    );
    let led1: Vec<f64> = long
        .iter()
        .filter(|(led, _)| *led == Led::One)
        .map(|(_, dt)| *dt)
        .collect();
    let led2: Vec<f64> = long
        .iter()
        .filter(|(led, _)| *led == Led::Two)
        .map(|(_, dt)| *dt)
        .collect();
    assert!(led1.len() >= 3 && led2.len() >= 3, "{long:?}");
    assert_on_time(median(&led1), LED1_ON_S, "led1");
    assert_on_time(median(&led2), LED2_ON_S, "led2");
}

#[test]
fn each_falling_edge_stays_within_1e_4_at_a_tenth_of_the_step() {
    let blink = blink();
    assert_finished(&blink.shipped);
    assert_finished(&blink.fine);
    let coarse = falling_edges(&blink.shipped.times, &blink.shipped.c1);
    let fine = falling_edges(&blink.fine.times, &blink.fine.c1);
    assert_edges_close(&coarse, &fine, steady_period(&fine));
}

#[test]
fn each_falling_edge_matches_the_recorded_ngspice_run() {
    assert!(NGSPICE_DECK.contains("vv1 vcc 0 pwl(0 0 1n 0 1.001n 5)"));
    assert!(NGSPICE_DECK.contains("reltol=1e-6"));
    assert!(NGSPICE_DECK.contains("vntol=1e-9"));
    assert!(!NGSPICE_DECK.contains("vx_q"));
    let solved = &blink().shipped;
    assert_finished(solved);
    let edges = falling_edges(&solved.times, &solved.c1);
    assert_edges_close(&edges, &NGSPICE_EDGES_S, NGSPICE_STEADY_PERIOD_S);
}

#[test]
fn the_period_matches_the_recorded_ngspice_run() {
    let solved = &blink().shipped;
    assert_finished(solved);
    let period = steady_period(&falling_edges(&solved.times, &solved.c1));
    let relative = period / NGSPICE_STEADY_PERIOD_S - 1.0;
    assert!(
        relative.abs() <= 1e-3,
        "{period} s is {relative:e} from the ngspice period of the recorded deck"
    );
}
