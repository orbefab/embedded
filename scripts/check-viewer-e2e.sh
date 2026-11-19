#!/usr/bin/env bash
# Builds the viewer's fixture page and drives it in headless Chromium: presses and releases,
# the top view, hover and the glow. Needs Node 24, pnpm and a Chromium. It installs no browser.
# It uses the one Playwright expects under ~/.cache/ms-playwright, or ORBE_CHROMIUM. Pass
# `--renders <dir>` to also save screenshots there. An unknown argument is an error.
# scripts/check.sh web runs this. check-viewer.sh does not, so that script needs no browser.
set -euo pipefail
cd "$(dirname "$0")/.."
while [[ $# -gt 0 ]]; do
  case "$1" in
    --renders)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "check-viewer-e2e: --renders needs a directory" >&2
        exit 2
      fi
      export ORBE_RENDERS=$2
      shift 2
      ;;
    *)
      echo "check-viewer-e2e: unknown argument $1" >&2
      exit 2
      ;;
  esac
done
scripts/need-node-deps.sh scripts/check-viewer-e2e.sh
pnpm --filter @orbe/viewer-page build >/dev/null
export ORBE_E2E=viewer
pnpm exec playwright test
