#!/usr/bin/env bash
# Builds the site into apps/site/dist from this commit. scripts/registry.sh writes the registry
# from release tags v0.2.0 and later, scripts/wasm.sh builds orbe-wasm, and the AVR runtime's
# source archive comes from the blob cache. All three go to .scratch/site, and `vite build` in
# apps/site renders every page from them. Needs what scripts/wasm.sh needs, Node 24, pnpm and
# cargo. It installs nothing. ORBE_SITE_INSTALL=1 shows install lines, once the repo and the
# binaries are public, and copies scripts/install.sh to /orbe/install.sh. The footer links the
# repo either way. The build writes apps/site/.wrangler/deploy/config.json, which points
# wrangler and `vite preview` at dist/orbefab_embedded/wrangler.json. That built config has
# the assets directory; the source wrangler.jsonc does not, so neither server starts without
# the pointer. The pointer is copied to dist/wrangler-deploy.json so a job that has only the
# build can put it back.
#
# The Block firmware links the AVR runtime under the LGPL, so the site serves the runtime's source
# beside it, at /orbe/sources/. The archive is the one packages/engine/scripts/sketches.sh
# pins by URL and SHA-256. It is fetched once into the blob cache, $ORBE_BLOBS, else
# $XDG_CACHE_HOME/orbe/blobs, else ~/.cache/orbe/blobs, as a file named by its SHA-256, and a
# download with another hash is refused.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -gt 0 ]]; then
  echo "scripts/site.sh: unknown argument $1" >&2
  exit 2
fi

scripts/need-node-deps.sh scripts/site.sh
# shellcheck source=scripts/check-lock.sh
source scripts/check-lock.sh
check_lock "scripts/site.sh" "scripts/site.sh"
scratch=$PWD/.scratch/site

pin() { sed -n "s/^$1=//p" packages/engine/scripts/sketches.sh; }
runtime_url=$(pin runtime_url)
runtime_sha256=$(pin runtime_sha256)
cache=${ORBE_BLOBS:-${XDG_CACHE_HOME:-$HOME/.cache}/orbe/blobs}
blob=$cache/$runtime_sha256
sha() { sha256sum "$1" | cut -d' ' -f1; }
if [[ ! -f $blob || $(sha "$blob") != "$runtime_sha256" ]]; then
  mkdir -p "$cache"
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$blob.part" "$runtime_url"; then
    rm -f "$blob.part"
    echo "scripts/site.sh: could not fetch $runtime_url, the AVR runtime source the site serves beside the Block firmware." >&2
    echo "Connect and run it again, or put that archive, SHA-256 $runtime_sha256, at $blob." >&2
    exit 1
  fi
  got=$(sha "$blob.part")
  if [[ $got != "$runtime_sha256" ]]; then
    rm -f "$blob.part"
    echo "scripts/site.sh: $runtime_url has SHA-256 $got, and sketches.sh pins $runtime_sha256. Refusing it." >&2
    exit 1
  fi
  mv "$blob.part" "$blob"
fi

cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
rm -rf "$scratch/registry" "$scratch/sources"
mkdir -p "$scratch/sources"
cp "$blob" "$scratch/sources/${runtime_url##*/}"
scripts/registry.sh "$target/debug/orbe" "$scratch/registry"
scripts/wasm.sh "$scratch/wasm"
pnpm --filter @orbe/site build
pointer=apps/site/.wrangler/deploy/config.json
if [[ ! -f $pointer ]]; then
  echo "scripts/site.sh: the build wrote no $pointer, so wrangler has no assets directory." >&2
  exit 1
fi
cp "$pointer" apps/site/dist/wrangler-deploy.json
