#!/usr/bin/env bash
# Builds the viewer page, orbe-wasm's package and this repo's registry, then drives the page in
# `?block=` mode in headless Chromium. Every Block under blocks/ opens in the Worker, plays, and
# shows what it does: a button lights @led1 while it is held, the fade's LED changes brightness,
# the light comparator's slider lights @led1 in the dark and turns it off in the light, and a
# 7-segment count lights a segment.
# The package and the registry go to .scratch/wasm/pkg and .scratch/wasm/registry, where the
# page's dev and preview servers serve them. Needs what scripts/wasm.sh needs, Node 24, pnpm,
# cargo and a Chromium, found as check-viewer-e2e.sh finds it. It installs nothing.
# scripts/check.sh web runs this. Pass `--renders <dir>` to also save screenshots there.
# An unknown argument is an error.
set -euo pipefail
cd "$(dirname "$0")/.."
while [[ $# -gt 0 ]]; do
  case "$1" in
    --renders)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "check-block: --renders needs a directory" >&2
        exit 2
      fi
      export ORBE_RENDERS=$2
      shift 2
      ;;
    *)
      echo "check-block: unknown argument $1" >&2
      exit 2
      ;;
  esac
done
scripts/need-node-deps.sh scripts/check-block.sh
# shellcheck source=scripts/check-lock.sh
source scripts/check-lock.sh
check_lock "scripts/check-block.sh" "scripts/check-block.sh $*"
pnpm --filter @orbe/viewer-page build >/dev/null
scripts/wasm.sh .scratch/wasm/pkg
cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
rm -rf .scratch/wasm/registry
"$target/debug/orbe" registry build . .scratch/wasm/registry >/dev/null
export ORBE_E2E=block
pnpm exec playwright test 9>&-
