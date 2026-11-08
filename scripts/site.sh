#!/usr/bin/env bash
# Builds the site into apps/site/dist from this commit: `orbe registry build` writes the registry
# of parts/ and blocks/, scripts/wasm.sh builds orbe-wasm, and apps/site/build.mjs renders every
# page from them. Both go to .scratch/site first. Needs what scripts/wasm.sh needs, Node 24, pnpm
# and cargo. It installs nothing. Pass `--previous <dir>`, the registry the live site serves, so
# the build keeps every version it served (docs/registry.md, Versions). ORBE_SITE_INSTALL=1
# shows install lines, once the repo and the binaries are public.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/need-node-deps.sh scripts/site.sh
scratch=$PWD/.scratch/site
cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
rm -rf "$scratch/registry"
mkdir -p "$scratch"
"$target/debug/orbe" registry build . "$scratch/registry" "$@"
scripts/wasm.sh "$scratch/wasm"
node apps/site/build.mjs --registry "$scratch/registry" --wasm "$scratch/wasm"
