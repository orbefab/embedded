//! Merges registries built from release tags. `scripts/registry.sh` runs this.
//!
//! The manifest is a JSON array. Each entry names one release:
//! `{"version":"0.2.0","formats":[1,2,3],"dir":"/path/to/registry"}`.
//! `dir` is the folder `orbe registry build` wrote for that tag. One entry may add
//! `"tree": true`: the registry of the working tree, named as the next release. It fails the
//! run when it changes an item a tag published without raising that item's version. It is
//! merged as a release only with `--with-tree`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use orbe_registry::{Registry, Release, publish, read_built, unbumped};
use serde::Deserialize;

#[derive(Deserialize)]
struct Entry {
    version: String,
    formats: Vec<u32>,
    dir: PathBuf,
    #[serde(default)]
    tree: bool,
}

const USAGE: &str = "publish: pass [--with-tree] then the manifest and the output folder";

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let with_tree = args.first().is_some_and(|arg| arg == "--with-tree");
    if with_tree {
        args.remove(0);
    }
    let Ok([manifest, out]) = <[String; 2]>::try_from(args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
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
    if entries.iter().filter(|entry| entry.tree).count() > 1 {
        eprintln!("publish: {manifest} names more than one working tree");
        return ExitCode::from(1);
    }
    let mut releases = Vec::new();
    let mut tree = None;
    for entry in &entries {
        let registry = match read_built(&entry.dir) {
            Ok(registry) => registry,
            Err(error) => {
                eprintln!("publish: {}: {error}", entry.dir.display());
                return ExitCode::from(1);
            }
        };
        let release = Release {
            version: entry.version.clone(),
            formats: entry.formats.clone(),
            registry,
        };
        if entry.tree {
            tree = Some(release);
        } else {
            releases.push(release);
        }
    }
    if let Some(tree) = tree {
        let stale = unbumped(&releases, &tree.registry);
        if !stale.is_empty() {
            for line in &stale {
                eprintln!("publish: {line}");
            }
            eprintln!(
                "publish: {} items changed without a new version, so `orbe add` would keep installing the published bytes",
                stale.len()
            );
            return ExitCode::from(1);
        }
        if with_tree {
            releases.push(tree);
        }
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
