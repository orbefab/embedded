#!/usr/bin/env bash
# Checks the site. It runs the type check and the brand check, then a build with
# ORBE_SITE_INSTALL=1 that must give every part and Block a page with its install line. Then it
# runs the build as it deploys, whose own files must show no install line and no repo link, and
# which must serve the AVR runtime source every Block's NOTICE names. apps/site/e2e/site.mjs then
# drives that build in headless Chromium twice: on `vite preview`, and on `wrangler dev --local`.
#
# The flag gates the site's own text and links: the pages, the build's scripts and styles, the
# brand files and the part views. The registry under r/, orbe-wasm and the runtime source are data
# the site serves unchanged, so a Block's README or block.jsonc still says `orbe add`.
#
# Needs what scripts/site.sh needs, and a Chromium found as check-viewer-e2e.sh finds it. It
# installs nothing. check.sh does not run this. Pass `--previous <dir>` first to build the
# registry on the one the live site serves, as scripts/site.sh does, and `--renders <dir>` to
# save screenshots there.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/need-node-deps.sh scripts/check-site.sh
previous=()
if [[ ${1:-} == --previous ]]; then
  previous=(--previous "$2")
  shift 2
fi
fail() {
  echo "check-site: $1" >&2
  exit 1
}
pnpm --filter @orbe/site typecheck
scripts/brand.sh check
dist=apps/site/dist/client
own() { grep -rlE "$1" "$dist" --exclude-dir=r --exclude-dir=orbe-wasm --exclude-dir=sources || true; }
pages() { find "$dist/$1" -mindepth 2 -name index.html | wc -l; }
listed() {
  node -e 'const index = require(process.argv[1]); console.log(new Set(index.items.filter((row) => row.type === process.argv[2]).map((row) => row.name)).size)' \
    "$PWD/$dist/r/index.json" "$1"
}

ORBE_SITE_INSTALL=1 scripts/site.sh "${previous[@]}" >/dev/null
parts=$(pages parts)
blocks=$(pages blocks)
[[ $parts -eq $(listed part) && $blocks -eq $(listed block) ]] ||
  fail "the build has $parts part and $blocks Block pages, and the registry lists $(listed part) and $(listed block)"
[[ $(grep -rlE 'orbe add [a-z]' "$dist/parts" "$dist/blocks" --include=index.html | wc -l) -eq $((parts + blocks)) ]] ||
  fail "with ORBE_SITE_INSTALL=1, not every part and Block page shows its orbe add line"
grep -q 'cargo install --locked --git https://github.com/orbefab/embedded orbe' "$dist/index.html" ||
  fail "with ORBE_SITE_INSTALL=1, the landing page shows no install line"
grep -q 'href="https://github.com/orbefab/embedded"' "$dist/index.html" ||
  fail "with ORBE_SITE_INSTALL=1, the landing page links no repo"
echo "check-site: ORBE_SITE_INSTALL=1 shows orbe add on $parts part and $blocks Block pages, and cargo install and the repo on the landing page"

scripts/site.sh "${previous[@]}"
shown=$(own 'orbe add|cargo install|github\.com/orbefab/embedded')
[[ -z $shown ]] || fail "these files of the site's own show an install line or a repo link with ORBE_SITE_INSTALL unset: $shown"
echo "check-site: no file of the site's own shows an install line or a repo link with ORBE_SITE_INSTALL unset"

pin() { sed -n "s/^$1=//p" packages/engine/scripts/sketches.sh; }
archive=$(pin runtime_url)
archive=${archive##*/}
[[ $(sha256sum "$dist/sources/$archive" | cut -d' ' -f1) == "$(pin runtime_sha256)" ]] ||
  fail "$dist/sources/$archive is not the runtime source sketches.sh pins"
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
server=
stop() {
  [[ -n $server ]] && kill -- "-$server" 2>/dev/null || true
  server=
}
trap stop EXIT
# Starts a server on a free port in its own process group, waits for the landing page, and runs
# the e2e against it.
drive() {
  local name=$1 port log
  shift
  port=$(node -e 'const s = require("node:net").createServer().listen(0, "127.0.0.1", () => { console.log(s.address().port); s.close(); })')
  log=$(mktemp)
  (cd apps/site && WRANGLER_SEND_METRICS=false exec setsid "$@" --port "$port" >"$log" 2>&1) &
  server=$!
  for _ in $(seq 120); do
    curl -fs -o /dev/null "http://127.0.0.1:$port/embedded/" && break
    sleep 0.5
  done
  curl -fs -o /dev/null "http://127.0.0.1:$port/embedded/" || { cat "$log" >&2; fail "$name did not serve /embedded/"; }
  echo "check-site: driving the build on $name"
  node apps/site/e2e/site.mjs --orbe "$target/debug/orbe" --base "http://127.0.0.1:$port" "${renders[@]}"
  stop
  rm -f "$log"
}
renders=("$@")
drive "vite preview" node_modules/.bin/vite preview --strictPort --host 127.0.0.1
drive "wrangler dev --local" node_modules/.bin/wrangler dev --local --ip 127.0.0.1
