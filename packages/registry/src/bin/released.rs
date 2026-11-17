//! Writes `released.lock` for a registry source. `scripts/released.sh` runs this.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(src) = args.next() else {
        eprintln!("released: pass the registry source, such as the repo root");
        return ExitCode::from(2);
    };
    if args.next().is_some() {
        eprintln!("released: pass one directory");
        return ExitCode::from(2);
    }
    let src = PathBuf::from(src);
    let text = match orbe_registry::released_lock(&src) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("released: {error}");
            return ExitCode::from(1);
        }
    };
    let count = text.lines().filter(|line| !line.is_empty()).count();
    let path = src.join(orbe_registry::RELEASED);
    if let Err(error) = fs::write(&path, text) {
        eprintln!("released: {}: {error}", path.display());
        return ExitCode::from(1);
    }
    println!("wrote {}, {count} items", path.display());
    ExitCode::SUCCESS
}
