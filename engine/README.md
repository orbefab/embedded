# sim-engine

sim-engine reads a resolved netlist and writes a deterministic run folder.

The input is JSON. The folder holds `manifest.json`, `netlist.json`, `diagnostics.json`, and `observations.json`, and a transient run adds `traces/`. This tree solves resistors, voltage sources, current sources, diodes, LEDs, capacitors, and timed switches at one operating point or over time, and reports parts run past their ratings. A voltage source or a current source holds a DC value, a step, or a pulse.

## Build

`rust-toolchain.toml` pins rustc 1.98.1, rustfmt, and clippy.

```sh
cargo build -p sim-cli
```

The binary is `sim`.

## Check

Run `scripts/check.sh` before every commit. From the repo root it runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p sim-avr --all-targets --locked --features port-skip-check -- -D warnings
cargo test --workspace --locked --exclude sim-avr
cargo test -p sim-avr --locked --features port-skip-check
cargo clippy -p sim-xtensa -p sim-esp32 -p sim-cli --all-targets --locked --features sim-xtensa/poll-skip-check -- -D warnings
cargo test -p sim-xtensa -p sim-esp32 -p sim-cli --locked --features sim-xtensa/poll-skip-check
uv run scripts/ngspice_refs.py --self-test
```

The `sim-avr` lines turn on `port-skip-check` so the skip check runs, and the other crates leave the feature off. The `sim-xtensa` lines turn on `poll-skip-check`. On a step whose poll is skipped, that build reads the interrupt lines and panics if they moved. The read is the one a normal build makes when it does poll, so it settles and stamps on that path. It does not run the stamp a skipped poll uses.

## Run the divider

`examples/divider/netlist.json` is a 5 V source, a 1000 ohm resistor, and a 2000 ohm resistor. `#net.vout` is within 1e-8 relative of 10/3 V.

```sh
cargo run -q -p sim-cli -- run examples/divider/netlist.json --out /tmp/sim-divider
```

A clean run prints one JSON document and exits 0.

```json
{
  "status": "ok",
  "out": "/tmp/sim-divider",
  "errors": 0,
  "warnings": 0,
  "diagnostics": []
}
```

`sim doctor` runs that divider and checks that two renders of it are byte-identical. It exits 0 when both checks pass.

`sim view <run> --out <dir>` writes `display-<part>.png` into `<dir>` for each display part of a finished run folder, what the panel's glass showed at the end. [docs/run-folder.md](docs/run-folder.md#what-the-glass-shows) says how it draws them.

The process exits with one of these codes.

- 0, `ok`. The folder is written. A warning still exits 0.
- 1, `failed`. The folder is written and contains an error diagnostic.
- 2, `usage_error`. The command line is wrong. Nothing is written.
- 3, `io_error`. The netlist cannot be read, or the folder cannot be written. For `view`, the run folder cannot be read, or an image cannot be written.

`--help` and `--version` print plain text and exit 0.

Stdout for `run` repeats `diagnostics` in the same order as `diagnostics.json`.

The schema is [docs/netlist.md](docs/netlist.md). How the engine steps, and how accurate it is, is [docs/engine.md](docs/engine.md). The folder is [docs/run-folder.md](docs/run-folder.md). The codes are [docs/diagnostics.md](docs/diagnostics.md). Which changes keep or bump each version is [docs/versions.md](docs/versions.md). The ngspice references are [docs/reference.md](docs/reference.md). The ATmega328P core behind the `atmega328p` kind is [docs/avr.md](docs/avr.md). The ESP32-S3 behind the `esp32s3` kind is [docs/esp32s3.md](docs/esp32s3.md). The development board fragment is [docs/board.md](docs/board.md). How to time a run is [docs/bench.md](docs/bench.md). The JavaScript package for a Worker is [docs/wasm.md](docs/wasm.md).

## License

The code is under either of the MIT license ([LICENSE-MIT](LICENSE-MIT)) or the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option. A contribution you submit for inclusion is under the same two licenses, with no other terms.

The compiled sketch images in `firmware/sketch/` are test fixtures that link third-party code under its own licenses. [firmware/sketch/NOTICE](firmware/sketch/NOTICE) names them. The engine does not link that code.
