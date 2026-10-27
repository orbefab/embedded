fn main() {
    let path = std::env::args().nth(1).expect("netlist path");
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("read {path}: {err}"));
    let netlist =
        sim_core::netlist::parse(&bytes).unwrap_or_else(|err| panic!("parse {path}: {err:?}"));
    let (results, stats) = sim_circuit::simulate_stats(&netlist);
    let errors = results.count(sim_core::diagnostic::Severity::Error);
    println!("steps\t{}\terrors\t{}", stats.landings, errors);
}
