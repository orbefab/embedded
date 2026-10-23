# AGENTS.md

sim-engine is the electronics simulation engine: a resolved netlist in, a deterministic run folder out.

Notes and plans: ~/notes/orbefab/lines/sim-engine/ (repo Orbefab/notes)

Private notes live in `~/notes/orbefab` (Orbefab/notes). Follow the `notes` skill for what goes there vs here, and `notes-sync` to pull/push them.

## Crates

- `crates/core`, package `sim-core`, holds the netlist, the diagnostics, and the run folder.
- `crates/circuit`, package `sim-circuit`, solves the DC operating point.
- `crates/cli`, package `sim-cli`, is the `sim` binary.
- `crates/avr`, package `sim-avr`, models the ATmega328P core. `sim-circuit` runs it for each `atmega328p` part.
- `crates/bench`, package `sim-bench`, times native, WASI and browser Worker runs. It is not published.
- `crates/wasm`, package `sim-wasm`, is the session and `sim run` for JavaScript. `scripts/wasm.sh` builds it, and `docs/wasm.md` is its guide.

## Before every commit

Run `scripts/check.sh` from the repo root. It runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p sim-avr --all-targets --locked --features port-skip-check -- -D warnings
cargo test --workspace --locked --exclude sim-avr
cargo test -p sim-avr --locked --features port-skip-check
uv run scripts/ngspice_refs.py --self-test
```

The `sim-avr` lines turn on `port-skip-check` so the skip check runs, and the other crates leave the feature off.

Also run `scripts/wasm-check.sh` before a commit that touches `crates/wasm`, `crates/circuit/src/session*`, `scripts/wasm.sh`, or the math in `crates/circuit/src/element/junction.rs`, `crates/circuit/src/step.rs` and `crates/core/src/netlist.rs` that goes through `libm`. It needs node 22.7 or later and rustup, and checks the wasm package against native `sim run` in node Workers.
