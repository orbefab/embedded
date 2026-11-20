#!/usr/bin/env bash
# Checks the repo. `lint` is what CI runs: shellcheck on every tracked shell script, ruff on every
# tracked Python file, `scripts/core-deps.sh`, rustfmt, clippy, and clippy on orbe-model,
# orbe-scene, orbe-session and orbe-wasm for wasm32.
# `fast` runs `lint`, then the Rust tests through cargo-nextest, except the engine tests marked
# `full tier`. `orbe-avr` and `orbe-microchip` are in that workspace run. `full` runs the
# microchip tests once, in their `port-skip-check` build, instead of in the workspace run.
# `web` runs the viewer unit tests and the viewer and site checks, each once: `check-viewer.sh`,
# `check-viewer-e2e.sh`, `check-block.sh`, `check-headed.sh`, `site-previous.test.sh`,
# `check-site.sh` and `check-engines.sh`.
# `check-assets.sh` stays separate, because it needs a models checkout.
# `full` runs `fast`, then the chip models' skip checks, the engine's Python checks,
# `ngspice_refs.py --check`, `orbe-run` for WASI against native, the wasm package check, the
# browser speed check on the package that check built, and `web`. The wasm package check and
# the speed check run once, not again in `web`.
# `lint` needs uv and shellcheck, and `fast` and `full` need cargo-nextest too.
# `web` needs Node 24, pnpm, the repo's Node dependencies, a Chromium as
# scripts/check-viewer-e2e.sh finds it, the Firefox and WebKit scripts/check-engines.sh
# finds, and what scripts/wasm.sh and scripts/site.sh need. Install the browsers with
# `pnpm exec playwright install chromium firefox webkit`. On Ubuntu, Firefox and WebKit
# also need `sudo pnpm exec playwright install-deps firefox webkit`.
# `full` also needs ngspice-42, rustup with the wasm32-wasip1 target, and the wasm-bindgen CLI
# that scripts/wasm.sh names.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  echo "usage: scripts/check.sh lint|fast|full|web" >&2
  exit 2
}

need_nextest() {
  if command -v cargo-nextest >/dev/null 2>&1; then
    return 0
  fi
  local version
  version=$(awk -F '"' '/^nextest-version[[:space:]]*=/ { print $2; exit }' .config/nextest.toml)
  echo "scripts/check.sh: cargo-nextest is not installed. Install it (cargo install cargo-nextest --version ${version} --locked) and re-run." >&2
  exit 1
}

rust_tests() {
  cargo nextest run --profile ci --locked "$@"
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
  # A download cut short through `curl | sh` must run nothing, so the installer does all its
  # work in main and calls it on its last line.
  [[ $(grep -v '^[[:space:]]*$' scripts/install.sh | tail -n 1) == 'main "$@"' ]] || {
    echo "scripts/check.sh: scripts/install.sh must end with the line: main \"\$@\"" >&2
    exit 1
  }
  mapfile -t py < <(git ls-files '*.py')
  if [[ ${#py[@]} -eq 0 ]]; then
    echo "scripts/check.sh: no tracked Python file" >&2
    exit 1
  fi
  uv run --project packages/cad --frozen ruff check "${py[@]}"
  uv run --project packages/cad --frozen ruff format --check "${py[@]}"
}

lint() {
  lint_tracked
  scripts/core-deps.sh
  cargo fmt --all --check
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo clippy -p orbe-engine --all-targets --locked --features runner -- -D warnings
  cargo clippy -p orbe-microchip --all-targets --locked --features port-skip-check -- -D warnings
  cargo clippy -p orbe-model -p orbe-scene -p orbe-session -p orbe-wasm \
    --target wasm32-unknown-unknown --locked -- -D warnings
}

fast() {
  need_nextest
  lint
  blobs=$(packages/engine/scripts/blobs.sh)
  rust_tests --workspace "$@"
}

run_check() {
  local circuit=$1 checker=$2 run
  shift 2
  run="${CARGO_TARGET_DIR:-target}/$circuit-check"
  rm -rf "$run"
  cargo run -q -p orbe-engine --features runner --bin orbe-run --locked -- \
    run "packages/engine/tests/circuits/$circuit/netlist.json" --out "$run" --blobs "$blobs" >/dev/null
  uv run "packages/engine/scripts/$checker" "$@" --check "$run"
  rm -rf "$run"
}

full() {
  # The microchip tests run once, with the port skip check built in.
  fast --exclude orbe-microchip
  rust_tests -p orbe-microchip --features port-skip-check
  rust_tests -p orbe-engine --test circuit --run-ignored only
  cargo clippy -p orbe-xtensa -p orbe-espressif -p orbe-engine --all-targets --locked \
    --features orbe-xtensa/poll-skip-check -- -D warnings
  rust_tests -p orbe-xtensa -p orbe-espressif --features orbe-xtensa/poll-skip-check
  rust_tests -p orbe-engine --test circuit -E 'test(/^runs::/)' --features orbe-xtensa/poll-skip-check
  local name knob_dir
  uv run --project packages/engine/scripts --frozen pytest --rootdir=packages/engine/scripts packages/engine/scripts/tests
  uv run packages/engine/scripts/st77916_example.py --check
  uv run packages/engine/scripts/pattern_header.py --check
  uv run packages/engine/scripts/ngspice_refs.py --check
  run_check st77916-replay render_st77916.py packages/engine/tests/circuits/st77916-replay/events.json
  run_check esp32s3-blink esp32s3_blink.py
  run_check esp32s3-pattern render_st77916.py --pattern packages/engine/tests/firmware/esp32s3/pattern.json
  for name in hello screen encoder touch dualcore echo ring uart pulls adc timers; do
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
  local wasm=.scratch/wasm/full
  (
    trap 'rm -rf "$wasm"' EXIT
    scripts/check-wasm.sh "$wasm"
    scripts/check-speed.sh "$wasm/pkg" "$wasm/registry"
  )
  web
}

web() {
  scripts/check-viewer.sh
  scripts/check-viewer-e2e.sh
  scripts/check-block.sh
  scripts/check-headed.sh
  scripts/site-previous.test.sh
  scripts/check-site.sh --first-deploy
  scripts/check-engines.sh
}

[[ $# -eq 1 ]] || usage
case "$1" in
  lint) lint ;;
  fast) fast ;;
  full) full ;;
  web) web ;;
  *) usage ;;
esac
