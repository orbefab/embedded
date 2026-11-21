//! Writes the netlist v1 of one scene to stdout.

use std::env;
use std::path::Path;
use std::process::ExitCode;

use orbe_scene::Files;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: netlist <scene.jsonc>");
        return ExitCode::from(2);
    };
    if args.next().is_some() {
        eprintln!("usage: netlist <scene.jsonc>");
        return ExitCode::from(2);
    }
    match orbe_scene::read(&Files::disk(), Path::new(&path)) {
        Ok(netlist) => {
            print!("{}", netlist.json);
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in errors {
                eprintln!("{error}");
            }
            ExitCode::from(1)
        }
    }
}
