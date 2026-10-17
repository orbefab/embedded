# AGENTS.md

sim-engine is the electronics simulation engine: a resolved netlist in, a deterministic run folder out.

Notes and plans: ~/notes/orbefab/lines/sim-engine/ (repo Orbefab/notes)

Private notes live in `~/notes/orbefab` (Orbefab/notes). Follow the `notes` skill for what goes there vs here, and `notes-sync` to pull/push them.

## Crates

- `crates/core`, package `sim-core`, holds the netlist, the diagnostics, and the run folder.
- `crates/circuit`, package `sim-circuit`, solves the DC operating point.
- `crates/cli`, package `sim-cli`, is the `sim` binary.

## Before every commit

Run `scripts/check.sh` from the repo root. It runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, and `uv run scripts/ngspice_refs.py --self-test`.
