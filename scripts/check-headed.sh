#!/usr/bin/env bash
# Builds the viewer page, builds orbe with the page in it, then drives a live headed session of
# the first target in Chromium: play, hold @btn1, see @led1 lit as `orbe get` reads it, close.
# Needs Node 24, npm, cargo and a Chromium, found as check-viewer-e2e.sh finds it. It never
# opens a desktop browser. Pass `--renders <dir>` to also save screenshots there.
set -euo pipefail
cd "$(dirname "$0")/.."
npm ci --no-audit --no-fund --silent
npm run build --workspace apps/viewer >/dev/null
cargo build -p orbe --locked
node apps/viewer/e2e/headed.mjs target/debug/orbe "$@"
