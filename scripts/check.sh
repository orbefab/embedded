#!/usr/bin/env bash
# Checks the repo. `fast` is what CI runs: shellcheck on every tracked shell script, ruff on every
# tracked Python file, rustfmt, clippy, clippy on orbe-scene, orbe-session and orbe-wasm for wasm32,
# and the Rust tests, except the `orbe-avr` tests and five engine targets marked `full tier`.
# `full` runs those too, then the chip models' skip checks, the engine's Python checks, `sim` for
# WASI against native, and the wasm package check. Both need uv and shellcheck. `full` also needs
# ngspice-42, node 24 or later, rustup with the wasm32-wasip1 target, and the wasm-bindgen CLI
# that scripts/wasm.sh names.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  echo "usage: scripts/check.sh fast|full" >&2
  exit 2
}

lint_tracked() {
  if ! command -v shellcheck >/dev/null 2>&1; then
    echo "scripts/check.sh: shellcheck is not installed. Install it (apt install shellcheck) and re-run." >&2
    exit 1
  fi
  local -a scripts py
  mapfile -t scripts < <(git ls-files '*.sh')
  if [[ ${#scripts[@]} -eq 0 ]]; then
    echo "scripts/check.sh: no tracked shell script" >&2
    exit 1
  fi
  shellcheck -x "${scripts[@]}"
  mapfile -t py < <(git ls-files '*.py')
  if [[ ${#py[@]} -eq 0 ]]; then
    echo "scripts/check.sh: no tracked Python file" >&2
    exit 1
  fi
  uv run --project packages/cad --frozen ruff check "${py[@]}"
  uv run --project packages/cad --frozen ruff format --check "${py[@]}"
}

fast() {
  lint_tracked
  cargo fmt --all --check
  blobs=$(packages/engine/scripts/blobs.sh)
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo clippy -p orbe-avr --all-targets --locked --features port-skip-check -- -D warnings
  cargo clippy -p orbe-scene -p orbe-session -p orbe-wasm --target wasm32-unknown-unknown --locked \
    -- -D warnings
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
  packages/engine/scripts/wasi-check.sh
  scripts/check-wasm.sh
}

[[ $# -eq 1 ]] || usage
case "$1" in
  fast) fast ;;
  full) full ;;
  *) usage ;;
esac
