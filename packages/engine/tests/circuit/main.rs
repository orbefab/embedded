mod common;

mod atmega328p;
mod chip;
mod contact;
mod divider;
mod inductor;
mod junction;
mod potentiometer;
mod regulator;
mod relay;
mod runs;
mod session;
mod solver;
mod sources;
mod step;
mod steps;
mod transient;
mod zener;

/// Cargo compiles a directory test target only through `mod` declarations. A scan of this
/// directory misses a file added under `runs/`.
#[test]
fn every_test_file_is_a_module() {
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/circuit"));
    walk(&root, &root.join("main.rs"));
}

fn walk(dir: &std::path::Path, parent: &std::path::Path) {
    let source = std::fs::read_to_string(parent).unwrap();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if path.is_dir() {
            let nested = path.join("mod.rs");
            let sibling = dir.join(format!("{name}.rs"));
            let child = if nested.is_file() {
                nested
            } else if sibling.is_file() {
                sibling
            } else {
                assert!(
                    !contains_rust(&path),
                    "{} holds a rust file outside a module",
                    path.display()
                );
                continue;
            };
            assert!(
                declares(&source, name),
                "{} is not declared in {}",
                path.display(),
                parent.display()
            );
            walk(&path, &child);
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "rs") {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            if stem == "main" || stem == "mod" {
                continue;
            }
            assert!(
                declares(&source, stem),
                "{} is not declared in {}",
                path.display(),
                parent.display()
            );
        }
    }
}

fn declares(source: &str, name: &str) -> bool {
    source.contains(&format!("mod {name};"))
}

fn contains_rust(dir: &std::path::Path) -> bool {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if contains_rust(&path) {
                return true;
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            return true;
        }
    }
    false
}
