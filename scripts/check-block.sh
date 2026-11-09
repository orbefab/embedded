#!/usr/bin/env bash
# Builds the viewer page, orbe-wasm's package and this repo's registry, then drives the page in
# `?block=` mode in headless Chromium: nano-button-led runs in the Worker, lights on a press and
# goes dark on release. The package and the registry go to .scratch/wasm/pkg and
# .scratch/wasm/registry, where the page's dev and preview servers serve them. Needs what
# scripts/wasm.sh needs, Node 24, pnpm, cargo and a Chromium, found as check-viewer-e2e.sh finds
# it. It installs nothing. check.sh does not run this. Pass `--renders <dir>` to also save
# screenshots there.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/need-node-deps.sh scripts/check-block.sh
pnpm --filter @orbe/viewer-page build >/dev/null
scripts/wasm.sh .scratch/wasm/pkg
cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
rm -rf .scratch/wasm/registry
"$target/debug/orbe" registry build . .scratch/wasm/registry >/dev/null
while [[ $# -gt 0 ]]; do
  if [[ $1 == --renders ]]; then
    export ORBE_RENDERS=${2:-}
    shift 2
  else
    shift
  fi
done
export ORBE_E2E=block
pnpm exec playwright test
