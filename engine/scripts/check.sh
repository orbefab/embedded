#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
blobs=$(scripts/blobs.sh)
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
uv run scripts/pattern_header.py --check
pattern_run="${CARGO_TARGET_DIR:-target}/esp32s3-pattern-check"
rm -rf "$pattern_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-pattern/netlist.json --out "$pattern_run" >/dev/null
uv run scripts/render_st77916.py --pattern firmware/esp32s3/pattern.json --check "$pattern_run"
rm -rf "$pattern_run"
uv run scripts/esp32s3_hello.py --self-test
hello_run="${CARGO_TARGET_DIR:-target}/esp32s3-hello-check"
rm -rf "$hello_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-hello/netlist.json --out "$hello_run" --blobs "$blobs" >/dev/null
uv run scripts/esp32s3_hello.py --check "$hello_run"
rm -rf "$hello_run"
uv run scripts/esp32s3_screen.py --self-test
screen_run="${CARGO_TARGET_DIR:-target}/esp32s3-screen-check"
rm -rf "$screen_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-screen/netlist.json --out "$screen_run" --blobs "$blobs" >/dev/null
uv run scripts/esp32s3_screen.py --check "$screen_run"
rm -rf "$screen_run"
