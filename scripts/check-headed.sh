#!/usr/bin/env bash
# Builds the viewer page, builds orbe with the page in it, then drives a live headed session of
# the first target in headless Chromium: play, hold @btn1, see @led1 lit as `orbe get` reads it,
# close. Needs Node 24, pnpm, cargo and a Chromium, found as check-viewer-e2e.sh finds it. It
# never opens a desktop browser. Pass `--renders <dir>` to also save screenshots there.
# check.sh does not run this.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/need-node-deps.sh scripts/check-headed.sh
pnpm --filter @orbe/viewer-page build >/dev/null
cargo build -p orbe --locked
orbe=$PWD/target/debug/orbe
if [[ ! -e $orbe ]]; then
  echo "No orbe at $orbe. Usage: scripts/check-headed.sh [--renders <dir>]" >&2
  exit 2
fi
while [[ $# -gt 0 ]]; do
  if [[ $1 == --renders ]]; then
    export ORBE_RENDERS=${2:-}
    shift 2
  else
    shift
  fi
done
export ORBE=$orbe
export ORBE_E2E=headed
pnpm exec playwright test
