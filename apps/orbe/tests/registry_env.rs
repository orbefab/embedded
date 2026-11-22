//! A registry built from this commit, as a file URL. Tests set `ORBE_REGISTRY` to it so
//! `orbe add`, `orbe part`, `orbe search` and `orbe blocks` never call orbefab.com.
//! Not every test binary calls every helper.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("registry-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_orbe"))
            .args([
                "registry",
                "build",
                repo.to_str().unwrap(),
                dir.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "registry build: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        dir
    })
}

pub fn template() -> String {
    format!("file://{}/{{name}}.json", dir().display())
}

pub fn apply(command: &mut Command) {
    command.env("ORBE_REGISTRY", template());
}

/// Sets `ORBE_REGISTRY` when `args` is a command that fetches.
pub fn apply_if(command: &mut Command, args: &[&str]) {
    let fetches = args
        .iter()
        .any(|arg| matches!(*arg, "add" | "part" | "search" | "blocks"));
    if fetches {
        apply(command);
    }
}
