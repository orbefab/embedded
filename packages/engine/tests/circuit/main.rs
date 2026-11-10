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

#[path = "../support/declare.rs"]
mod declare;

/// Cargo compiles a directory test target only through `mod` declarations. A scan of this
/// directory misses a file added under `runs/`.
#[test]
fn every_test_file_is_a_module() {
    declare::assert_declared(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/circuit"));
}
