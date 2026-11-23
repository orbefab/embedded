use orbe_engine::diagnostic::Severity;

fn read(rel: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read(&path).unwrap()
}

fn landings(bytes: &[u8]) -> u64 {
    let netlist = orbe_engine::netlist::parse(bytes).unwrap();
    let (results, stats) = orbe_engine::simulate_stats(&netlist);
    let errors: Vec<_> = results
        .diagnostics()
        .iter()
        .filter(|d| d.severity() == Severity::Error)
        .map(|d| serde_json::to_string(d).unwrap())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    stats.landings
}

fn with_resistance(bytes: &[u8], scale: f64) -> Vec<u8> {
    let mut doc: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for part in doc["parts"].as_array_mut().unwrap() {
        if part["kind"] == "linear_regulator" {
            let value = part["output_resistance"]["value"].as_f64().unwrap();
            part["output_resistance"]["value"] = serde_json::json!(value * scale);
        }
    }
    serde_json::to_vec(&doc).unwrap()
}

/// One run's accepted steps, the 1 ns steps, and the largest reversal of `#net.out`
/// between the charge and the first load edge.
fn shape(bytes: &[u8]) -> (u64, u64, u64, u64, u64, f64) {
    let netlist = orbe_engine::netlist::parse(bytes).unwrap();
    let results = orbe_engine::simulate_steps(&netlist);
    let trace = results.trace().unwrap();
    let times: Vec<u64> = trace.time().iter().map(|t| t.as_ps()).collect();
    let column = trace
        .columns()
        .iter()
        .position(|reference| reference.to_string() == "#net.vout")
        .unwrap();
    let volts = trace.column(column);
    let in_window = |from, to| times.iter().filter(|&&t| from < t && t <= to).count() as u64;
    let nanos = times.windows(2).filter(|w| w[1] - w[0] == 1000).count() as u64;
    let mut zigzag = 0.0_f64;
    for index in 1..times.len() - 1 {
        if times[index] <= 40_000_000 || times[index + 1] >= 1_000_000_000 {
            continue;
        }
        let left = volts[index] - volts[index - 1];
        let right = volts[index + 1] - volts[index];
        if left * right < 0.0 {
            zigzag = zigzag.max(right.abs());
        }
    }
    (
        (times.len() as u64).saturating_sub(1),
        nanos,
        in_window(0, 40_000_000),
        in_window(28_000_000, 40_000_000),
        in_window(1_000_000_000, 1_100_000_000) + in_window(3_000_000_000, 3_100_000_000),
        zigzag,
    )
}

/// A count at most ten percent above `measured`, in integer math.
fn at_most(got: u64, measured: u64, label: &str) {
    let bound = measured + measured / 10;
    assert!(got <= bound, "{label}: {got}, bound {bound}");
}

/// Measured 226 accepted steps on the load step and 68 on the ideal source. Each bound is
/// ten percent above that count. The ratio is under ten.
#[test]
fn load_step_stays_within_ten_times_the_ideal_source() {
    let example_bytes = read("tests/circuits/regulator-load-step/netlist.json");
    let baseline_bytes = read("tests/circuit/regulator-baseline.json");
    let example = landings(&example_bytes);
    let baseline = landings(&baseline_bytes);
    eprintln!("load-step landings {example}");
    eprintln!("baseline landings {baseline}");
    at_most(example, 226, "load-step landings");
    at_most(baseline, 68, "baseline landings");
    assert!(example <= baseline * 10);
}

/// The charge, the load edges, and the same circuit with the output resistance at
/// ten times and a tenth. Each count is a ceiling ten percent above the measured run.
/// The 1 ns count is the segment-start step.
#[test]
fn load_step_shape_stays_at_the_measured_counts() {
    let bytes = read("tests/circuits/regulator-load-step/netlist.json");
    let fitted = shape(&bytes);
    let stiff = shape(&with_resistance(&bytes, 10.0));
    let soft = shape(&with_resistance(&bytes, 0.1));
    eprintln!("fitted steps nanos charge charge_28_40 edges zigzag: {fitted:?}");
    eprintln!("ro_x10: {stiff:?}");
    eprintln!("ro_div10: {soft:?}");
    let cases = [
        ("fitted", fitted, [226, 2, 89, 5, 84]),
        ("ro_x10", stiff, [310, 4, 123, 21, 134]),
        ("ro_div10", soft, [189, 2, 76, 5, 60]),
    ];
    for (label, got, measured) in cases {
        for (index, (count, measured)) in [got.0, got.1, got.2, got.3, got.4]
            .into_iter()
            .zip(measured)
            .enumerate()
        {
            at_most(count, measured, &format!("{label} field {index}"));
        }
        assert!(got.5 < 1e-6, "{label} zigzag {}", got.5);
    }
}
