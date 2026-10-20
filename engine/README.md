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
cargo test --workspace --locked
uv run scripts/ngspice_refs.py --self-test
```

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

The process exits with one of these codes.

- 0, `ok`. The folder is written. A warning still exits 0.
- 1, `failed`. The folder is written and contains an error diagnostic.
- 2, `usage_error`. The command line is wrong. Nothing is written.
- 3, `io_error`. The netlist cannot be read, or the folder cannot be written.

`--help` and `--version` print plain text and exit 0.

Stdout for `run` repeats `diagnostics` in the same order as `diagnostics.json`.

The schema is [docs/netlist.md](docs/netlist.md). How the engine steps, and how accurate it is, is [docs/engine.md](docs/engine.md). The folder is [docs/run-folder.md](docs/run-folder.md). The codes are [docs/diagnostics.md](docs/diagnostics.md). Which changes keep or bump each version is [docs/versions.md](docs/versions.md). The ngspice references are [docs/reference.md](docs/reference.md). The ATmega328P core behind the `atmega328p` kind is [docs/avr.md](docs/avr.md).
