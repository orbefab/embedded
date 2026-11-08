#!/usr/bin/env bash
# Checks the site: the type check, the brand copy, a build with ORBE_SITE_INSTALL=1 that must show
# the install lines, then the build as it deploys, which must show none on any page, driven in
# Chromium through apps/site/e2e/site.mjs. Needs what scripts/site.sh needs, and a Chromium found
# as check-viewer-e2e.sh finds it. It installs nothing. check.sh does not run this. Pass
# `--previous <dir>` first to build the registry on the one the live site serves, as
# scripts/site.sh does, and `--renders <dir>` to save screenshots there.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/need-node-deps.sh scripts/check-site.sh
previous=()
if [[ ${1:-} == --previous ]]; then
  previous=(--previous "$2")
  shift 2
fi
pnpm --filter @orbe/site typecheck
scripts/brand.sh check
pages=apps/site/dist/embedded
shown() { grep -rlE "$1" --include='*.html' "$pages" | wc -l; }

ORBE_SITE_INSTALL=1 scripts/site.sh "${previous[@]}" >/dev/null
parts=$(find "$pages/parts" -mindepth 2 -name index.html | wc -l)
blocks=$(find "$pages/blocks" -mindepth 2 -name index.html | wc -l)
[[ $(shown 'orbe add [a-z]') -eq $((parts + blocks)) ]] ||
  { echo "check-site: with ORBE_SITE_INSTALL=1, not every part and Block page shows its orbe add line" >&2; exit 1; }
grep -q 'cargo install --locked --git https://github.com/orbefab/embedded orbe' "$pages/index.html" ||
  { echo "check-site: with ORBE_SITE_INSTALL=1, the landing page shows no install line" >&2; exit 1; }
echo "check-site: ORBE_SITE_INSTALL=1 shows orbe add on $parts part and $blocks Block pages, and cargo install on the landing page"

scripts/site.sh "${previous[@]}"
if grep -rlE 'orbe add|cargo install|github\.com/orbefab/embedded' --include='*.html' "$pages"; then
  echo "check-site: the pages above show an install line or a repo link with ORBE_SITE_INSTALL unset" >&2
  exit 1
fi
echo "check-site: no page shows an install line or a repo link with ORBE_SITE_INSTALL unset"
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
node apps/site/e2e/site.mjs --orbe "$target/debug/orbe" "$@"
