mod adc;
mod alu_flags;
mod clock_select;
mod common;
mod counted_runs;
mod cycles;
mod free_runs;
mod gaps;
mod gpio_in;
mod idle_loops;
mod instructions;
mod mspim;
mod oc_pin;
mod portb;
mod sent;
mod sketches;
mod spi;
mod steady_runs;
mod system;
mod timeline;
mod timers;
mod timing;
mod twi;
mod usart;

/// With `autotests = false`, cargo skips a test file that is not declared above.
#[test]
fn every_test_file_is_a_module() {
    let main = include_str!("main.rs");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().is_some_and(|e| e == "rs") && stem != "main" {
            assert!(
                main.contains(&format!("mod {stem};\n")),
                "tests/{stem}.rs is not declared in tests/main.rs"
            );
        }
    }
}
