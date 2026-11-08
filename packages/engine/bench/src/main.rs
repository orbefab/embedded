use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

struct Args {
    runs: usize,
    examples: PathBuf,
    out: Option<PathBuf>,
    list: bool,
    live_netlists: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = match parse(env::args().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };
    if args.list {
        println!(
            "{}",
            serde_json::to_string(&orbe_engine_bench::EXAMPLES).expect("example names serialize")
        );
        return ExitCode::SUCCESS;
    }
    let examples = match orbe_engine_bench::load_examples(&args.examples) {
        Ok(examples) => examples,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if let Some(dir) = &args.live_netlists {
        if let Err(err) = orbe_engine_bench::write_live_netlists(&examples, dir) {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }
    let report =
        match orbe_engine_bench::bench(&examples, args.runs, orbe_engine_bench::host_now_ns) {
            Ok(report) => report,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::from(1);
            }
        };
    let json = orbe_engine_bench::to_json(&report);
    if let Some(out) = &args.out {
        if let Err(err) = std::fs::write(out, &json) {
            eprintln!("{}: {err}", out.display());
            return ExitCode::from(1);
        }
    } else {
        println!("{json}");
    }
    println!("{}", orbe_engine_bench::markdown(&report));
    if report.workloads.iter().any(|row| row.errors > 0) {
        eprintln!("a workload reported an error diagnostic");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn parse(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut runs = 3usize;
    let mut examples = PathBuf::from("packages/engine/tests/circuits");
    let mut out = None;
    let mut list = false;
    let mut live_netlists = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runs" => runs = number(&mut args, "--runs")?,
            "--examples" => examples = PathBuf::from(value(&mut args, "--examples")?),
            "--out" => out = Some(PathBuf::from(value(&mut args, "--out")?)),
            "--list" => list = true,
            "--live-netlists" => {
                live_netlists = Some(PathBuf::from(value(&mut args, "--live-netlists")?))
            }
            "--help" => return Err(HELP.to_string()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if runs == 0 {
        return Err("--runs must be at least 1".to_string());
    }
    Ok(Args {
        runs,
        examples,
        out,
        list,
        live_netlists,
    })
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn number(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<usize, String> {
    let text = value(args, flag)?;
    text.parse()
        .map_err(|_| format!("{flag} needs a positive integer"))
}

const HELP: &str = "\
orbe-engine-bench [--runs N] [--examples DIR] [--out FILE] [--list] [--live-netlists DIR]

--live-netlists DIR writes each live netlist and exits. --out and --runs do nothing with it.";
