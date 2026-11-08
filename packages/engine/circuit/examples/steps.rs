fn main() {
    let path = std::env::args().nth(1).expect("netlist path");
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("read {path}: {err}"));
    let netlist = orbe_engine_core::netlist::parse(&bytes)
        .unwrap_or_else(|err| panic!("parse {path}: {err:?}"));
    let (results, stats) = orbe_circuit::simulate_stats(&netlist);
    let errors = results.count(orbe_engine_core::diagnostic::Severity::Error);
    println!("steps\t{}\terrors\t{}", stats.landings, errors);
}
