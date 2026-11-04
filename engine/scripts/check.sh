#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
blobs=$(scripts/blobs.sh)
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p sim-avr --all-targets --locked --features port-skip-check -- -D warnings
cargo test --workspace --locked --exclude sim-avr
cargo test -p sim-avr --locked --features port-skip-check
cargo clippy -p sim-xtensa -p sim-esp32 -p sim-cli --all-targets --locked --features sim-xtensa/poll-skip-check -- -D warnings
cargo test -p sim-xtensa -p sim-esp32 -p sim-cli --locked --features sim-xtensa/poll-skip-check
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
uv run scripts/esp32s3_encoder.py --self-test
encoder_run="${CARGO_TARGET_DIR:-target}/esp32s3-encoder-check"
rm -rf "$encoder_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-encoder/netlist.json --out "$encoder_run" --blobs "$blobs" >/dev/null
uv run scripts/esp32s3_encoder.py --check "$encoder_run"
rm -rf "$encoder_run"
uv run scripts/i2c_decode.py --self-test
uv run scripts/esp32s3_touch.py --self-test
touch_run="${CARGO_TARGET_DIR:-target}/esp32s3-touch-check"
rm -rf "$touch_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-touch/netlist.json --out "$touch_run" --blobs "$blobs" >/dev/null
uv run scripts/esp32s3_touch.py --check "$touch_run"
rm -rf "$touch_run"
uv run scripts/esp32s3_dualcore.py --self-test
dualcore_run="${CARGO_TARGET_DIR:-target}/esp32s3-dualcore-check"
rm -rf "$dualcore_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-dualcore/netlist.json --out "$dualcore_run" --blobs "$blobs" >/dev/null
uv run scripts/esp32s3_dualcore.py --check "$dualcore_run"
rm -rf "$dualcore_run"
uv run scripts/esp32s3_echo.py --self-test
echo_run="${CARGO_TARGET_DIR:-target}/esp32s3-echo-check"
rm -rf "$echo_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-echo/netlist.json --out "$echo_run" --blobs "$blobs" >/dev/null
uv run scripts/esp32s3_echo.py --check "$echo_run"
rm -rf "$echo_run"
uv run scripts/esp32s3_ring.py --self-test
ring_run="${CARGO_TARGET_DIR:-target}/esp32s3-ring-check"
rm -rf "$ring_run"
cargo run -q -p sim-cli --locked -- run examples/esp32s3-ring/netlist.json --out "$ring_run" --blobs "$blobs" >/dev/null
uv run scripts/esp32s3_ring.py --check "$ring_run"
rm -rf "$ring_run"
uv run scripts/esp32s3_arduino_console.py --self-test
uv run scripts/esp32s3_knob.py --self-test
knob_dir=$(mktemp -d "${TMPDIR:-/tmp}/knob-script.XXXXXX")
(
  trap 'rm -rf "$knob_dir"' EXIT
  cp firmware/esp32s3/arduino/checks/knob/netlist.json "$knob_dir/netlist.json"
  uv run scripts/esp32s3_knob.py script \
    firmware/esp32s3/arduino/checks/knob/refs.json \
    "$knob_dir/netlist.json"
  cmp firmware/esp32s3/arduino/checks/knob/netlist.json "$knob_dir/netlist.json"
)
