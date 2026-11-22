#!/usr/bin/env bash
# Drives a smoke in headless Firefox and headless WebKit. The viewer's fixture plays and
# the time reading changes. WebKit draws the board. Headless Firefox on Linux cannot create
# a WebGL context, so the viewer fixture shows that error instead of the board, and those
# console lines are the expected ones. The site's Get orbe page, parts page and Blocks page
# load on vite preview. A Block page draws its Block in WebKit. In Firefox it shows one
# sentence in place of the view and starts no renderer, so it logs none of those lines. Any
# other page error or console error fails the check.
# Needs the built viewer page, the built site, Node 24, pnpm, and the Firefox and WebKit
# Playwright 1.64.1 expects under ~/.cache/ms-playwright. It installs nothing. A missing
# browser or a missing system library exits 2. Install the browsers with
# `pnpm exec playwright install firefox webkit`. On Ubuntu, install the libraries with
# `sudo pnpm exec playwright install-deps firefox webkit`.
# scripts/check.sh web runs this, after the site build. An unknown argument is an error.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $# -gt 0 ]]; then
  echo "check-engines: unknown argument $1" >&2
  exit 2
fi
scripts/need-node-deps.sh scripts/check-engines.sh
if [[ ! -f apps/viewer/dist/index.html ]]; then
  echo "check-engines: no viewer build at apps/viewer/dist. Run \`pnpm --filter @orbe/viewer-page build\` first." >&2
  exit 2
fi
if [[ ! -f apps/site/dist/client/index.html ]]; then
  echo "check-engines: no site build at apps/site/dist/client. Run \`scripts/site.sh\` first, or \`scripts/check-site.sh\`." >&2
  exit 2
fi
free_port() {
  node -e 'const s = require("node:net").createServer().listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port) + "\n"); s.close(); })'
}
viewer_port=$(free_port)
site_port=$(free_port)
export ORBE_VIEWER_PORT=$viewer_port
export ORBE_SITE_PORT=$site_port
export ORBE_E2E=engines
started=$(date +%s)
pnpm exec playwright test
echo "check-engines: firefox and webkit smoke finished in $(($(date +%s) - started)) s"
