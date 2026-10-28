#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p sim-avr --all-targets --locked --features port-skip-check -- -D warnings
cargo test --workspace --locked --exclude sim-avr
cargo test -p sim-avr --locked --features port-skip-check
uv run scripts/ngspice_refs.py --self-test
uv run scripts/st77916_example.py --check
uv run scripts/render_st77916.py --self-test
display_run="${CARGO_TARGET_DIR:-target}/st77916-check"
rm -rf "$display_run"
cargo run -q -p sim-cli --locked -- run examples/st77916-replay/netlist.json --out "$display_run" >/dev/null
uv run scripts/render_st77916.py examples/st77916-replay/events.json --check "$display_run"
rm -rf "$display_run"
uv run scripts/esp32s3_blink.py --self-test
blink_run="${CARGO_TARGET_DIR:-target}/esp32s3-blink-check"
rm -rf "$blink_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-blink/netlist.json --out "$blink_run" >/dev/null
uv run scripts/esp32s3_blink.py --check "$blink_run"
rm -rf "$blink_run"
