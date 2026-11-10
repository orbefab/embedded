fn main() {
    let path = std::env::args().nth(1).expect("netlist path");
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("read {path}: {err}"));
    let netlist =
        orbe_engine::netlist::parse(&bytes).unwrap_or_else(|err| panic!("parse {path}: {err:?}"));
    let (results, stats) = orbe_engine::simulate_stats(&netlist);
    let errors = results.count(orbe_engine::diagnostic::Severity::Error);
    println!("steps\t{}\terrors\t{}", stats.landings, errors);
}
