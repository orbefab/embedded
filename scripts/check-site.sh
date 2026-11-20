#!/usr/bin/env bash
# Checks the site. It runs the type check, the circuit unit tests and the brand check, then a
# build with ORBE_SITE_INSTALL=1. scripts/site-install.sh checks that build: every part and
# Block page shows its orbe add line, and Get orbe shows the install command, the repo link
# and install.sh. The site workflow runs the same script on the build it uploads. Playwright
# drives that build once, on `vite preview`, for the checks that need the install line. Then
# it runs the build as it deploys, whose own files must show no install line.
# The footer links the repo either way. The build must serve the AVR runtime source every
# Block's NOTICE names.
# Playwright then drives that build in headless Chromium twice: on `vite preview`, and on
# `wrangler dev --local`.
#
# The flag gates the site's own text and links: the pages, the build's scripts and styles, the
# brand files and the part views. The registry under r/, orbe-wasm and the runtime source are data
# the site serves unchanged, so a Block's README or block.jsonc still says `orbe add`.
#
# Needs what scripts/site.sh needs, and a Chromium found as check-viewer-e2e.sh finds it. It
# installs nothing. scripts/check.sh web runs this. `--previous <dir>` builds the registry on
# the one the live site serves. With no `--previous`, the build is local and passes
# `--first-deploy`, when there is no registry to download. `--renders <dir>` saves screenshots there. The
# arguments can be in any order. An unknown argument is an error.
set -euo pipefail
cd "$(dirname "$0")/.."
previous=()
first=0
renders=""
saw_renders=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --first-deploy)
      first=1
      shift
      ;;
    --previous)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "check-site: --previous needs a directory" >&2
        exit 2
      fi
      previous=(--previous "$2")
      shift 2
      ;;
    --renders)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "check-site: --renders needs a directory" >&2
        exit 2
      fi
      renders=$2
      saw_renders=1
      shift 2
      ;;
    *)
      echo "check-site: unknown argument $1" >&2
      exit 2
      ;;
  esac
done
if [[ $first -eq 1 && ${#previous[@]} -gt 0 ]]; then
  echo "check-site: pass --previous or --first-deploy, not both" >&2
  exit 2
fi
if [[ ${#previous[@]} -gt 0 ]]; then
  site=("${previous[@]}")
else
  site=(--first-deploy)
fi
if [[ $saw_renders -eq 1 ]]; then
  export ORBE_RENDERS=$renders
fi
scripts/need-node-deps.sh scripts/check-site.sh
fail() {
  echo "check-site: $1" >&2
  exit 1
}
pnpm --filter @orbe/site typecheck
pnpm --filter @orbe/site test
scripts/brand.sh check
dist=apps/site/dist/client
own() { grep -rlE "$1" "$dist" --exclude-dir=r --exclude-dir=orbe-wasm --exclude-dir=sources || true; }
pages() { find "$dist/$1" -mindepth 2 -maxdepth 2 -name index.html | wc -l; }
listed() {
  # console.log colors a number when FORCE_COLOR is set, and [[ ]] then rejects the count.
  node -e 'const index = require(process.argv[1]); process.stdout.write(String(new Set(index.items.filter((row) => row.type === process.argv[2]).map((row) => row.name)).size) + "\n")' \
    "$PWD/$dist/r/index.json" "$1"
}

ORBE_SITE_INSTALL=1 scripts/site.sh "${site[@]}" >/dev/null
parts=$(pages parts)
blocks=$(pages blocks)
embeds=$(find "$dist/blocks" -mindepth 3 -maxdepth 3 -name index.html | wc -l)
[[ $parts -eq $(listed part) && $blocks -eq $(listed block) && $embeds -eq $blocks ]] ||
  fail "the build has $parts part, $blocks Block and $embeds embed pages, and the registry lists $(listed part) and $(listed block)"
scripts/site-install.sh

# 8787 is wrangler's default and may already be taken. Ask the OS for a free port.
free_port() {
  node -e 'const s = require("node:net").createServer().listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port) + "\n"); s.close(); })'
}
install_port=$(free_port)
export ORBE_E2E=site
export ORBE_SITE_INSTALL_E2E=1
export ORBE_SITE_SERVERS=vite
export ORBE_SITE_VITE_PORT=$install_port
echo "check-site: install-on browser checks on 127.0.0.1:$install_port"
pnpm exec playwright test apps/site/e2e/install.spec.ts
unset ORBE_SITE_INSTALL_E2E ORBE_SITE_SERVERS

scripts/site.sh "${site[@]}"
[[ ! -e $dist/install.sh ]] || fail "install.sh is served with ORBE_SITE_INSTALL unset"
shown=$(own 'orbe add|cargo install|orbefab\.com/embedded/install\.sh')
[[ -z $shown ]] || fail "these files of the site's own show an install line with ORBE_SITE_INSTALL unset: $shown"
grep -q 'href="https://github.com/orbefab/embedded"' "$dist/index.html" ||
  fail "the footer does not link the repo"
grep -q 'name="robots" content="noindex"' "$dist/blocks/nano-button-led/embed/index.html" ||
  fail "the embed page is not noindex"
echo "check-site: no file of the site's own shows an install line with ORBE_SITE_INSTALL unset, and the footer links the repo"

pin() { sed -n "s/^$1=//p" packages/engine/scripts/sketches.sh; }
archive=$(pin runtime_url)
archive=${archive##*/}
[[ $(sha256sum "$dist/sources/$archive" | cut -d' ' -f1) == "$(pin runtime_sha256)" ]] ||
  fail "$dist/sources/$archive is not the runtime source sketches.sh pins"
# shellcheck disable=SC2016 # javascript
node -e '
const [r, want] = process.argv.slice(1);
const fs = require("node:fs");
const index = JSON.parse(fs.readFileSync(`${r}/index.json`, "utf8"));
for (const row of index.items.filter((row) => row.type === "block")) {
  const record = JSON.parse(fs.readFileSync(`${r}/${row.url}`, "utf8"));
  const notice = record.files.find((file) => file.path === "firmware/NOTICE");
  if (notice === undefined || !fs.readFileSync(`${r}/files/${notice.sha256}`, "utf8").includes(want)) {
    console.error(`check-site: ${row.name}@${row.version} has no firmware/NOTICE that names ${want}`);
    process.exit(1);
  }
}' "$dist/r" "https://orbefab.com/embedded/sources/$archive"
echo "check-site: the build serves sources/$archive, the runtime source each Block NOTICE names"

target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
export ORBE=$target/debug/orbe
export ORBE_E2E=site
export WRANGLER_SEND_METRICS=false
vite_port=$(free_port)
wrangler_port=$(free_port)
export ORBE_SITE_VITE_PORT=$vite_port
export ORBE_SITE_WRANGLER_PORT=$wrangler_port
pnpm exec playwright test
