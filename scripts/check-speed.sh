#!/usr/bin/env bash
# Checks that every Block of this repo's registry plays live in a browser. It runs
# packages/wasm/tests/speed.mjs, which plays each Block in orbe-wasm's worker.js in headless
# Chromium, 5 times interleaved, and fails when a median misses its bar: 1.5x real time in a
# dedicated Worker, and 1.0x on the page's main thread under Chrome's 4x CPU throttle.
# docs/wasm.md#speed says how it measures.
#
# Usage: scripts/check-speed.sh [--profile BLOCK] [PKG_DIR REGISTRY_DIR]
#
# With no directories it builds the package with scripts/wasm.sh and the registry with
# `orbe registry build` under .scratch/speed, and removes them after. scripts/check.sh full passes
# the ones scripts/check-wasm.sh built. `--profile BLOCK` instead records a CPU profile of one run
# of BLOCK on the main thread at 4x and prints the functions with the most self time.
#
# Needs Node 24, the repo's Node dependencies and a Chromium, found as check-viewer-e2e.sh finds
# it, and with no directories what scripts/wasm.sh needs. It installs nothing.
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v node >/dev/null || ! node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 24 ? 0 : 1)'; then
  echo "scripts/check-speed.sh: needs node 24 or later. Install it and re-run." >&2
  exit 2
fi
scripts/need-node-deps.sh scripts/check-speed.sh
profile=()
if [[ ${1:-} == --profile ]]; then
  [[ $# -ge 2 ]] || {
    echo "usage: scripts/check-speed.sh [--profile BLOCK] [PKG_DIR REGISTRY_DIR]" >&2
    exit 2
  }
  profile=(--profile "$2")
  shift 2
fi
case $# in
  2)
    pkg=$1
    registry=$2
    ;;
  0)
    scratch=.scratch/speed
    rm -rf "$scratch"
    trap 'rm -rf "$scratch"' EXIT
    scripts/wasm.sh "$scratch/pkg" >/dev/null
    cargo build -q -p orbe --locked
    target=$(cargo metadata --format-version 1 --no-deps |
      sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
    "$target/debug/orbe" registry build . "$scratch/registry" >/dev/null
    pkg=$scratch/pkg
    registry=$scratch/registry
    ;;
  *)
    echo "usage: scripts/check-speed.sh [--profile BLOCK] [PKG_DIR REGISTRY_DIR]" >&2
    exit 2
    ;;
esac
node packages/wasm/tests/speed.mjs "$pkg" "$registry" "${profile[@]}"
