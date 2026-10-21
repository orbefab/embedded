//! Times one parsed netlist, and the four committed chip examples.
//!
//! The caller supplies the clock. `std::time::Instant` is not available on
//! `wasm32-unknown-unknown`.

use std::borrow::Cow;
use std::hint::black_box;
use std::path::Path;

use serde::Serialize;
use sim_avr::{Channel, Cpu};
use sim_core::diagnostic::Severity;
use sim_core::netlist::{Analysis, Chip, Model, Netlist};

/// Committed examples the benchmark reads at run time.
pub const EXAMPLES: [&str; 4] = [
    "atmega-blink",
    "atmega-button",
    "atmega-fade",
    "atmega-analog-serial",
];

const SERIAL_EXAMPLE: &str = "atmega-analog-serial";

const SERIAL_SECS: [u64; 3] = [4, 8, 16];

/// One millisecond, in picoseconds.
const CORE_SLICE_PS: u64 = 1_000_000_000;

const ADC_VOLTS: f32 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Committed,
    Gridless,
    Serial(u64),
}

impl Variant {
    fn label(self) -> Cow<'static, str> {
        match self {
            Variant::Committed => Cow::Borrowed("committed"),
            Variant::Gridless => Cow::Borrowed("gridless"),
            Variant::Serial(secs) => Cow::Owned(format!("serial-{secs}s")),
        }
    }
}

impl Serialize for Variant {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.label())
    }
}

#[derive(Clone, Debug)]
pub struct Example {
    pub name: String,
    pub json: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Spread {
    pub median: u64,
    pub min: u64,
    pub max: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub example: String,
    pub variant: Variant,
    pub simulated_ps: u64,
    pub end_ps: Option<u64>,
    pub errors: usize,
    pub landings: u64,
    pub stops: u64,
    pub wall_ns: Spread,
    pub bare_core_ns: Option<Spread>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub target: String,
    pub runs: usize,
    pub workloads: Vec<Row>,
}

struct Job {
    example: String,
    variant: Variant,
    netlist: Netlist,
}

struct Sample {
    wall_ns: u64,
    bare_core_ns: Option<u64>,
    landings: u64,
    stops: u64,
    errors: usize,
    end_ps: Option<u64>,
    simulated_ps: u64,
}

pub fn target_label() -> &'static str {
    if cfg!(target_os = "wasi") {
        "wasi"
    } else if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
        "worker"
    } else {
        "native"
    }
}

pub fn load_examples(dir: &Path) -> Result<Vec<Example>, String> {
    let mut examples = Vec::with_capacity(EXAMPLES.len());
    for name in EXAMPLES {
        let path = dir.join(name).join("netlist.json");
        let json = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        examples.push(Example {
            name: name.to_string(),
            json,
        });
    }
    Ok(examples)
}

pub fn bench(
    examples: &[Example],
    runs: usize,
    mut now_ns: impl FnMut() -> u64,
) -> Result<Report, String> {
    if runs == 0 {
        return Err("runs must be at least 1".to_string());
    }
    let jobs = jobs(examples)?;
    let mut workloads = Vec::with_capacity(jobs.len());
    for job in &jobs {
        workloads.push(time_job(job, runs, &mut now_ns)?);
    }
    Ok(Report {
        target: target_label().to_string(),
        runs,
        workloads,
    })
}

pub fn to_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).expect("report serializes")
}

pub fn markdown(report: &Report) -> String {
    let target = &report.target;
    let mut lines = vec![
        format!(
            "| example | variant | simulated | {target} | {target} x | bare core | landings | stops |"
        ),
        "| --- | --- | --- | --- | --- | --- | --- | --- |".to_string(),
    ];
    for row in &report.workloads {
        lines.push(markdown_row(row));
    }
    for row in &report.workloads {
        let Variant::Serial(secs) = row.variant else {
            continue;
        };
        let rate = wall_per_sim(row.wall_ns.median, row.simulated_ps);
        lines.push(format!(
            "{example} {secs} s takes {rate:.3} wall seconds per simulated second on {target}.",
            example = row.example,
        ));
    }
    lines.join("\n")
}

fn markdown_row(row: &Row) -> String {
    let simulated = seconds_ps(row.simulated_ps);
    let ratio = format!("{:.2}", times_real(row.simulated_ps, row.wall_ns.median));
    let spread = format!(
        "{:.3} ({:.3} to {:.3})",
        seconds_ns(row.wall_ns.median),
        seconds_ns(row.wall_ns.min),
        seconds_ns(row.wall_ns.max)
    );
    let bare = row
        .bare_core_ns
        .map(|spread| format!("{:.3}", seconds_ns(spread.median)))
        .unwrap_or_else(|| "none".to_string());
    format!(
        "| {} | {} | {simulated:.3} | {spread} | {ratio} | {bare} | {} | {} |",
        row.example,
        row.variant.label(),
        row.landings,
        row.stops,
    )
}

fn jobs(examples: &[Example]) -> Result<Vec<Job>, String> {
    let mut jobs = Vec::new();
    for example in examples {
        let value: serde_json::Value = serde_json::from_slice(&example.json)
            .map_err(|err| format!("{}: {err}", example.name))?;
        let stop = json_u64(&value, "stop_ps")?;
        jobs.push(job(&example.name, Variant::Committed, &value)?);
        let mut gridless = value.clone();
        gridless["analysis"]["transient"]["grid_ps"] = serde_json::json!(stop);
        jobs.push(job(&example.name, Variant::Gridless, &gridless)?);
        if example.name == SERIAL_EXAMPLE {
            for secs in SERIAL_SECS {
                let mut serial = value.clone();
                serial["analysis"]["transient"]["stop_ps"] =
                    serde_json::json!(secs * 1_000_000_000_000);
                jobs.push(job(&example.name, Variant::Serial(secs), &serial)?);
            }
        }
    }
    Ok(jobs)
}

fn job(example: &str, variant: Variant, value: &serde_json::Value) -> Result<Job, String> {
    Ok(Job {
        example: example.to_string(),
        variant,
        netlist: parse_value(example, value)?,
    })
}

fn parse_value(example: &str, value: &serde_json::Value) -> Result<Netlist, String> {
    let bytes = serde_json::to_vec(value).map_err(|err| format!("{example}: {err}"))?;
    sim_core::netlist::parse(&bytes).map_err(|diagnostics| {
        format!(
            "{example}: {}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

fn json_u64(value: &serde_json::Value, field: &str) -> Result<u64, String> {
    value["analysis"]["transient"][field]
        .as_u64()
        .ok_or_else(|| format!("analysis.transient.{field} is not an integer"))
}

fn time_job(job: &Job, runs: usize, now_ns: &mut impl FnMut() -> u64) -> Result<Row, String> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        samples.push(
            sample(job, now_ns)
                .map_err(|err| format!("{} {} {err}", job.example, job.variant.label()))?,
        );
    }
    let first = &samples[0];
    for sample in &samples[1..] {
        if sample.landings != first.landings || sample.stops != first.stops {
            return Err(format!(
                "{} {} counts changed across runs",
                job.example,
                job.variant.label()
            ));
        }
    }
    let bare_core_ns = first
        .bare_core_ns
        .map(|_| spread(samples.iter().filter_map(|sample| sample.bare_core_ns)));
    Ok(Row {
        example: job.example.clone(),
        variant: job.variant,
        simulated_ps: first.simulated_ps,
        end_ps: first.end_ps,
        errors: first.errors,
        landings: first.landings,
        stops: first.stops,
        wall_ns: spread(samples.iter().map(|sample| sample.wall_ns)),
        bare_core_ns,
    })
}

fn sample(job: &Job, now_ns: &mut impl FnMut() -> u64) -> Result<Sample, String> {
    let t0 = now_ns();
    let (results, stats) = sim_circuit::simulate_stats(&job.netlist);
    black_box(&results);
    let t1 = now_ns();
    let errors = results.count(Severity::Error);
    let end_ps = results.observations().end().map(|time| time.as_ps());
    drop(results);
    let bare_core_ns = bare_core(&job.netlist, now_ns)?;
    let simulated_ps = match job.netlist.analysis() {
        Analysis::Transient(transient) => transient.stop.as_ps(),
        Analysis::Op => 0,
    };
    Ok(Sample {
        wall_ns: t1.saturating_sub(t0),
        bare_core_ns,
        landings: stats.landings,
        stops: stats.stops,
        errors,
        end_ps,
        simulated_ps,
    })
}

fn bare_core(netlist: &Netlist, now_ns: &mut impl FnMut() -> u64) -> Result<Option<u64>, String> {
    let mut found = None;
    for (_, part) in netlist.parts() {
        if let Model::Chip(Chip::Atmega328p(mcu)) = &part.model {
            if found.is_some() {
                return Ok(None);
            }
            found = Some(mcu);
        }
    }
    let mcu = match found {
        Some(mcu) => mcu,
        None => return Ok(None),
    };
    let Analysis::Transient(transient) = netlist.analysis() else {
        return Ok(None);
    };
    let mut cpu = Cpu::new();
    let clock = u32::try_from(mcu.clock_hz.value).expect("parse keeps the clock in u32");
    cpu.set_clock_hz(clock);
    cpu.set_fuses(mcu.fuses.low(), mcu.fuses.high(), mcu.fuses.extended());
    cpu.load_flash(mcu.firmware.flash());
    cpu.set_vcc(5.0);
    cpu.set_avcc(5.0);
    cpu.set_aref(5.0);
    cpu.set_channel_voltage(0, ADC_VOLTS);
    let stop_ps = transient.stop.as_ps();
    let hz = cpu.cpu_hz();
    let t0 = now_ns();
    let mut covered = 0u64;
    while covered < stop_ps {
        let end = covered.saturating_add(CORE_SLICE_PS).min(stop_ps);
        cpu.run_until(cycles_at(end, hz));
        while cpu.take_pin().is_some() {}
        while cpu.take(Channel::Usart0).is_some() {}
        covered = end;
    }
    black_box(&cpu);
    let t1 = now_ns();
    let overflow = cpu.pin_overflow();
    if overflow != 0 {
        return Err(format!("pin_overflow {overflow}"));
    }
    Ok(Some(t1.saturating_sub(t0)))
}

fn cycles_at(ps: u64, hz: u32) -> u64 {
    let cycles = u128::from(ps) * u128::from(hz) / 1_000_000_000_000;
    u64::try_from(cycles).expect("cycle count fits in u64")
}

fn spread(values: impl Iterator<Item = u64>) -> Spread {
    let mut values: Vec<u64> = values.collect();
    assert!(!values.is_empty(), "a spread needs a run");
    let min = values.iter().copied().min().unwrap();
    let max = values.iter().copied().max().unwrap();
    values.sort_unstable();
    Spread {
        median: values[values.len() / 2],
        min,
        max,
    }
}

fn seconds_ns(ns: u64) -> f64 {
    ns as f64 / 1e9
}

fn seconds_ps(ps: u64) -> f64 {
    ps as f64 / 1e12
}

fn times_real(simulated_ps: u64, wall_ns: u64) -> f64 {
    if wall_ns == 0 {
        return f64::INFINITY;
    }
    (simulated_ps as f64) / (wall_ns as f64) * 1e-3
}

fn wall_per_sim(wall_ns: u64, simulated_ps: u64) -> f64 {
    if simulated_ps == 0 {
        return f64::INFINITY;
    }
    (wall_ns as f64) / (simulated_ps as f64) * 1e3
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn host_now_ns() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    #[allow(clippy::disallowed_methods)]
    let start = START.get_or_init(Instant::now);
    #[allow(clippy::disallowed_methods)]
    let elapsed = start.elapsed();
    elapsed.as_nanos() as u64
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(unsafe_code)]
mod wasm_abi {
    use std::cell::RefCell;

    use super::{Example, bench, to_json};

    struct State {
        runs: usize,
        examples: Vec<Example>,
        input: Vec<u8>,
        output: Vec<u8>,
    }

    thread_local! {
        static STATE: RefCell<State> = const {
            RefCell::new(State {
                runs: 3,
                examples: Vec::new(),
                input: Vec::new(),
                output: Vec::new(),
            })
        };
    }

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn now_ms() -> f64;
    }

    fn host_now_ns() -> u64 {
        let ms = unsafe { now_ms() };
        (ms * 1_000_000.0) as u64
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn bench_alloc(len: u32) -> *mut u8 {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.input.clear();
            state.input.resize(len as usize, 0);
            state.input.as_mut_ptr()
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn bench_begin(runs: u32) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.runs = runs.max(1) as usize;
            state.examples.clear();
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn bench_add(name_len: u32) -> u32 {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let name_len = name_len as usize;
            if name_len > state.input.len() {
                return 0;
            }
            let name = state.input[..name_len].to_vec();
            let json = state.input[name_len..].to_vec();
            let Ok(name) = String::from_utf8(name) else {
                return 0;
            };
            state.examples.push(Example { name, json });
            1
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn bench_finish() -> u32 {
        let (examples, runs) = STATE.with(|state| {
            let mut state = state.borrow_mut();
            (std::mem::take(&mut state.examples), state.runs)
        });
        let report = bench(&examples, runs, host_now_ns);
        let text = match report {
            Ok(report) => to_json(&report),
            Err(err) => serde_json::json!({ "error": err }).to_string(),
        };
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.output = text.into_bytes();
            state.output.len() as u32
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn bench_result() -> *const u8 {
        STATE.with(|state| {
            let state = state.borrow();
            state.output.as_ptr()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RC: &str = r#"{
      "version": 0,
      "seed": 0,
      "analysis": { "transient": { "stop_ps": 1000000, "max_step_ps": 100000, "grid_ps": 100000 } },
      "ground": "gnd",
      "nets": ["gnd", "vcc", "mid"],
      "parts": [
        { "name": "v1", "kind": "voltage_source", "pins": { "p": "vcc", "n": "gnd" },
          "voltage": { "value": 5.0, "unit": "V", "source": "user" } },
        { "name": "r1", "kind": "resistor", "pins": { "a": "vcc", "b": "mid" },
          "resistance": { "value": 1000.0, "unit": "ohm", "source": "user" },
          "ratings": { "power": null } },
        { "name": "c1", "kind": "capacitor", "pins": { "a": "mid", "b": "gnd" },
          "capacitance": { "value": 1e-9, "unit": "F", "source": "user" },
          "initial": { "value": 0.0, "unit": "V", "source": "user" } }
      ]
    }"#;

    #[test]
    fn a_short_rc_reaches_its_stop_with_no_chip() {
        let mut ticks = 0u64;
        let report = bench(
            &[Example {
                name: "rc".to_string(),
                json: RC.as_bytes().to_vec(),
            }],
            1,
            || {
                ticks += 1_000_000;
                ticks
            },
        )
        .unwrap();
        let row = report
            .workloads
            .iter()
            .find(|row| row.variant == Variant::Committed)
            .unwrap();
        assert_eq!(row.simulated_ps, 1_000_000);
        assert_eq!(row.end_ps, Some(1_000_000));
        assert_eq!(row.errors, 0);
        assert!(row.landings >= 1);
        assert_eq!(row.stops, 0);
        assert!(row.bare_core_ns.is_none());
        assert!(row.wall_ns.median >= 1_000_000);
    }
}
