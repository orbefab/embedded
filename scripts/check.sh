#!/usr/bin/env bash
# Checks the repo. `fast` is what CI runs: rustfmt, clippy and the Rust tests, except the
# `orbe-avr` tests and five engine targets marked `full tier`. `full` runs those too, then the
# chip models' skip checks, the engine's Python checks and the wasm package check. Both need uv.
# `full` also needs ngspice-42, node 22.7 or later and rustup.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  echo "usage: scripts/check.sh fast|full" >&2
  exit 2
}

fast() {
  cargo fmt --all --check
  blobs=$(packages/engine/scripts/blobs.sh)
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo clippy -p orbe-avr --all-targets --locked --features port-skip-check -- -D warnings
  cargo test --workspace --locked --exclude orbe-avr
}

run_check() {
  local circuit=$1 checker=$2 run
  shift 2
  run="${CARGO_TARGET_DIR:-target}/$circuit-check"
  rm -rf "$run"
  cargo run -q -p orbe-engine-cli --locked -- run "packages/engine/tests/circuits/$circuit/netlist.json" \
    --out "$run" --blobs "$blobs" >/dev/null
  uv run "packages/engine/scripts/$checker" "$@" --check "$run"
  rm -rf "$run"
}

full() {
  fast
  cargo test -p orbe-avr --locked --features port-skip-check
  cargo test -p orbe-circuit --locked --test transient --test session -- --ignored
  cargo test -p orbe-engine-cli --locked --test run --test session --test ws2812 -- --ignored
  cargo clippy -p orbe-xtensa -p orbe-espressif -p orbe-engine-cli --all-targets --locked \
    --features orbe-xtensa/poll-skip-check -- -D warnings
  cargo test -p orbe-xtensa -p orbe-espressif -p orbe-engine-cli --locked \
    --features orbe-xtensa/poll-skip-check
  local script name knob_dir
  for script in ngspice_refs render_st77916 esp32s3_blink esp32s3_hello esp32s3_screen \
    esp32s3_encoder i2c_decode esp32s3_touch esp32s3_dualcore esp32s3_echo esp32s3_ring \
    esp32s3_arduino_console esp32s3_knob; do
    uv run "packages/engine/scripts/$script.py" --self-test
  done
  uv run packages/engine/scripts/st77916_example.py --check
  uv run packages/engine/scripts/pattern_header.py --check
  run_check st77916-replay render_st77916.py packages/engine/tests/circuits/st77916-replay/events.json
  run_check esp32s3-blink esp32s3_blink.py
  run_check esp32s3-pattern render_st77916.py --pattern packages/engine/tests/firmware/esp32s3/pattern.json
  for name in hello screen encoder touch dualcore echo ring; do
    run_check "esp32s3-$name" "esp32s3_$name.py"
  done
  knob_dir=$(mktemp -d "${TMPDIR:-/tmp}/knob-script.XXXXXX")
  (
    trap 'rm -rf "$knob_dir"' EXIT
    cp packages/engine/tests/firmware/esp32s3/arduino/checks/knob/netlist.json "$knob_dir/netlist.json"
    uv run packages/engine/scripts/esp32s3_knob.py script \
      packages/engine/tests/firmware/esp32s3/arduino/checks/knob/refs.json \
      "$knob_dir/netlist.json"
    cmp packages/engine/tests/firmware/esp32s3/arduino/checks/knob/netlist.json "$knob_dir/netlist.json"
  )
  packages/engine/scripts/wasm-check.sh
}

[[ $# -eq 1 ]] || usage
case "$1" in
  fast) fast ;;
  full) full ;;
  *) usage ;;
esac
