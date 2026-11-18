#!/usr/bin/env bash
# Builds the viewer's fixture page and drives it in headless Chromium: presses and releases,
# the top view, hover and the glow. Needs Node 24, pnpm and a Chromium. It installs no browser.
# It uses the one Playwright expects under ~/.cache/ms-playwright, or ORBE_CHROMIUM. Pass
# `--renders <dir>` to also save screenshots there. scripts/check.sh web runs this.
# check-viewer.sh does not, so that script needs no browser.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/need-node-deps.sh scripts/check-viewer-e2e.sh
pnpm --filter @orbe/viewer-page build >/dev/null
while [[ $# -gt 0 ]]; do
  if [[ $1 == --renders ]]; then
    export ORBE_RENDERS=${2:-}
    shift 2
  else
    shift
  fi
done
export ORBE_E2E=viewer
pnpm exec playwright test
