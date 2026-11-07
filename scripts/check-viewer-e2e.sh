#!/usr/bin/env bash
# Builds the viewer's fixture page and drives it in Chromium: presses and releases, the top
# view, hover and the glow. Needs Node 24, npm and a Chromium. It installs no browser: it uses
# the one playwright-core expects under ~/.cache/ms-playwright, or ORBE_CHROMIUM. Pass
# `--renders <dir>` to also save screenshots there. check-viewer.sh does not run this.
set -euo pipefail
cd "$(dirname "$0")/.."
npm ci --no-audit --no-fund --silent
npm run build --workspace apps/viewer >/dev/null
node apps/viewer/e2e/e2e.mjs "$@"
