#!/usr/bin/env bash
# Checks the viewer: the type check, the unit tests and the page build. Needs Node 24 and pnpm.
# check.sh does not run this, so it needs neither.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/need-node-deps.sh scripts/check-viewer.sh
pnpm --filter @orbe/viewer --filter @orbe/viewer-page typecheck
pnpm --filter @orbe/viewer test
pnpm --filter @orbe/viewer-page build
