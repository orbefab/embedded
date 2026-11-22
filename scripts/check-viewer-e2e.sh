#!/usr/bin/env bash
# Builds the viewer page, orbe-wasm's package and this repo's registry, then drives the page
# with no session token in headless Chromium. That page runs nano-button-led: presses and
# releases, the top view, hover and the glow. The package and the registry go to
# .scratch/wasm/pkg and .scratch/wasm/registry, where the page's dev and preview servers
# serve them. Needs what scripts/wasm.sh needs, Node 24, pnpm, cargo and a Chromium. It
# installs no browser. It uses the Chromium Playwright expects under ~/.cache/ms-playwright,
# or ORBE_CHROMIUM. Pass `--renders <dir>` to also save screenshots there. `--shard N/M`
# runs that Playwright shard. An unknown argument is an error. scripts/check.sh viewer
# runs this with no shard. check-viewer.sh does not run it, so that script needs no browser.
set -euo pipefail
cd "$(dirname "$0")/.."
shard=""
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
    --shard)
      if [[ $# -lt 2 || ! ${2:-} =~ ^[1-9][0-9]*/[1-9][0-9]*$ ]]; then
        echo "check-viewer-e2e: --shard needs N/M, such as 1/2" >&2
        exit 2
      fi
      shard_n=${2%/*}
      shard_m=${2#*/}
      if ((shard_n > shard_m)); then
        echo "check-viewer-e2e: --shard $2 is out of range" >&2
        exit 2
      fi
      shard=$2
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
scripts/wasm.sh .scratch/wasm/pkg
cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
rm -rf .scratch/wasm/registry
"$target/debug/orbe" registry build . .scratch/wasm/registry >/dev/null
export ORBE_E2E=viewer
if [[ -n $shard ]]; then
  pnpm exec playwright test --shard="$shard"
else
  pnpm exec playwright test
fi
