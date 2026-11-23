//! Writes the netlist v1 of one scene to stdout.
//!
//! ```text
//! netlist [--blobs <dir>] <scene.jsonc>
//! ```
//!
//! With `--blobs`, each blob the scene read is written into that directory, the file named
//! by its SHA-256.

use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use orbe_engine::blob::Blobs;
use orbe_scene::Files;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let mut scene = None;
    let mut blobs = None;
    while let Some(arg) = args.next() {
        if arg == "--blobs" {
            let Some(dir) = args.next() else {
                usage();
                return ExitCode::from(2);
            };
            if blobs.is_some() {
                usage();
                return ExitCode::from(2);
            }
            blobs = Some(dir);
        } else if scene.is_some() {
            usage();
            return ExitCode::from(2);
        } else {
            scene = Some(arg);
        }
    }
    let Some(path) = scene else {
        usage();
        return ExitCode::from(2);
    };
    match orbe_scene::read(&Files::disk(), Path::new(&path)) {
        Ok(netlist) => {
            if let Some(dir) = &blobs
                && let Err(err) = write_blobs(Path::new(dir), &netlist.blobs)
            {
                eprintln!("netlist: {err}");
                return ExitCode::from(1);
            }
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

fn usage() {
    eprintln!("usage: netlist [--blobs <dir>] <scene.jsonc>");
}

fn write_blobs(dir: &Path, blobs: &Blobs) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    for (name, bytes) in blobs.iter() {
        fs::write(dir.join(name.to_string()), bytes)?;
    }
    Ok(())
}
