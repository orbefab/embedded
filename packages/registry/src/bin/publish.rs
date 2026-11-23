//! Merges registries built from release tags. `scripts/registry.sh` runs this.
//!
//! The manifest is a JSON array. Each entry names one release:
//! `{"version":"0.2.0","formats":[1,2,3],"dir":"/path/to/registry"}`.
//! `dir` is the folder `orbe registry build` wrote for that tag.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbe_registry::{Registry, Release, publish, read_built};
use serde::Deserialize;

#[derive(Deserialize)]
struct Entry {
    version: String,
    formats: Vec<u32>,
    dir: PathBuf,
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let (Some(manifest), Some(out)) = (args.next(), args.next()) else {
        eprintln!("publish: pass the manifest and the output folder");
        return ExitCode::from(2);
    };
    if args.next().is_some() {
        eprintln!("publish: pass the manifest and the output folder");
        return ExitCode::from(2);
    }
    let text = match fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("publish: {manifest}: {error}");
            return ExitCode::from(1);
        }
    };
    let entries: Vec<Entry> = match serde_json::from_str(&text) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("publish: {manifest} is not a release manifest: {error}");
            return ExitCode::from(1);
        }
    };
    let mut releases = Vec::new();
    for entry in &entries {
        let registry = match read_built(&entry.dir) {
            Ok(registry) => registry,
            Err(error) => {
                eprintln!("publish: {}: {error}", entry.dir.display());
                return ExitCode::from(1);
            }
        };
        releases.push(Release {
            version: entry.version.clone(),
            formats: entry.formats.clone(),
            registry,
        });
    }
    let merged = match publish(&releases) {
        Ok(merged) => merged,
        Err(error) => {
            eprintln!("publish: {error}");
            return ExitCode::from(1);
        }
    };
    let registry = Registry {
        items: Vec::new(),
        output: merged,
        report: releases
            .first()
            .map(|release| release.registry.report.clone())
            .expect("publish refused an empty manifest"),
    };
    if let Err(error) = registry.write(PathBuf::from(&out).as_path()) {
        eprintln!("publish: {error}");
        return ExitCode::from(1);
    }
    println!("wrote {out}");
    ExitCode::SUCCESS
}
