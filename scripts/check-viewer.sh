#!/usr/bin/env bash
# Checks the viewer: the type check, the unit tests and the page build. Needs Node 24 and npm.
# check.sh does not run this, so it needs neither.
set -euo pipefail
cd "$(dirname "$0")/.."
npm ci --no-audit --no-fund --silent
npm run typecheck --workspaces
npm test --workspace packages/viewer
npm run build --workspace apps/viewer
