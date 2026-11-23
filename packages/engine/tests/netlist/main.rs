mod common;

mod atmega328p;
mod cst816;
mod diagnostic;
mod inductor;
mod ne555;
mod op_amp;
mod parse;
mod parts;
mod potentiometer;
mod regulator;
mod relay;
mod sources;
mod spi_replay;
mod ssd1306;
mod switch;
mod transient;
mod transistor;
mod values;
mod world;
mod ws2812;
mod zener;

#[path = "../support/declare.rs"]
mod declare;

/// Cargo compiles a directory test target only through `mod` declarations. A scan of this
/// directory misses a file added under a nested module.
#[test]
fn every_test_file_is_a_module() {
    declare::assert_declared(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/netlist"));
}
