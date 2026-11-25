#!/usr/bin/env bash
# Builds the viewer page, builds orbe with the page in it, then drives three live headed sessions
# in headless Chromium. The first is the first target: play, hold @btn1, see @led1 lit as
# `orbe get` reads it, close. The second is the light divider: move the light slider, read the
# new light on the next frame and the divider voltage with `orbe get`. The third is a Nano
# reading a potentiometer: turn it with its slider and with `orbe set`, and see the knob turn
# and the LED follow. Needs Node 24, pnpm, cargo and a Chromium, found as check-viewer-e2e.sh
# finds it. It never opens a desktop browser. Pass
# `--renders <dir>` to also save screenshots there. An unknown argument is an error.
# It runs the orbe cargo just built. cargo metadata names that target directory, so
# CARGO_TARGET_DIR is the binary under test. scripts/check.sh web runs this.
set -euo pipefail
cd "$(dirname "$0")/.."
while [[ $# -gt 0 ]]; do
  case "$1" in
    --renders)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "check-headed: --renders needs a directory" >&2
        exit 2
      fi
      export ORBE_RENDERS=$2
      shift 2
      ;;
    *)
      echo "check-headed: unknown argument $1" >&2
      exit 2
      ;;
  esac
done
scripts/need-node-deps.sh scripts/check-headed.sh
# shellcheck source=scripts/check-lock.sh
source scripts/check-lock.sh
check_lock "scripts/check-headed.sh" "scripts/check-headed.sh $*"
pnpm --filter @orbe/viewer-page build >/dev/null
cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
orbe=$target/debug/orbe
if [[ ! -e $orbe ]]; then
  echo "No orbe at $orbe. Usage: scripts/check-headed.sh [--renders <dir>]" >&2
  exit 2
fi
export ORBE=$orbe
export ORBE_E2E=headed
pnpm exec playwright test 9>&-
export ORBE_E2E=world
pnpm exec playwright test 9>&-
export ORBE_E2E=knob
pnpm exec playwright test 9>&-
