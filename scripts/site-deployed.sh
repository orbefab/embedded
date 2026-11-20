#!/usr/bin/env bash
# Checks the site that was just deployed. scripts/check-site.sh has already checked the build.
# This asks the live host the questions in docs/site.md, Deploy, and exits 1 when an answer
# is wrong. ORBE_SITE_INSTALL=1 expects the install line and /embedded/install.sh. Unset, it
# expects the CLI-coming line and no install script. A check is retried for a minute, because
# the new assets can take a moment to replace the previous ones.
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
  echo "scripts/site-deployed.sh: $1" >&2
  exit 1
}

dist=apps/site/dist/client
[[ -f $dist/r/index.json ]] || fail "apps/site/dist/client is not a site build. Run scripts/check-site.sh first."
base=https://orbefab.com
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

try() {
  local n=$1
  shift
  local i=1
  while [[ $i -le $n ]]; do
    if [[ $i -gt 1 ]]; then
      echo "scripts/site-deployed.sh: retry $i of $n"
      sleep 5
    fi
    if "$@"; then
      return 0
    fi
    i=$((i + 1))
  done
  return 1
}

curl_body() {
  curl -sS --max-time 30 -o "$2" -w '%{http_code}' "$1"
}

home_ok() {
  local code
  code=$(curl_body "$base/embedded/" "$work/home.html") || return 1
  [[ $code == 200 ]] || return 1
  grep -q 'Get orbe | Orbefab' "$work/home.html" || return 1
  if [[ ${ORBE_SITE_INSTALL:-} == 1 ]]; then
    grep -q 'curl -fsSL https://orbefab.com/embedded/install.sh | sh' "$work/home.html" || return 1
    grep -q 'The orbe CLI is coming' "$work/home.html" && return 1
    code=$(curl_body "$base/embedded/install.sh" "$work/install.sh") || return 1
    [[ $code == 200 ]] || return 1
    cmp -s scripts/install.sh "$work/install.sh"
  else
    grep -q 'The orbe CLI is coming' "$work/home.html" || return 1
    grep -q 'orbefab.com/embedded/install.sh' "$work/home.html" && return 1
    code=$(curl_body "$base/embedded/install.sh" "$work/install.sh") || return 1
    [[ $code == 404 ]]
  fi
}

root_ok() {
  local code
  code=$(curl_body "$base/" "$work/root.html") || return 1
  [[ $code == 200 ]] || return 1
  grep -q '<title>[^<]*Orbefab[^<]*</title>' "$work/root.html" || return 1
  ! grep -q '<title>[^<]*Get orbe' "$work/root.html"
}

elsewhere_ok() {
  local code
  code=$(curl_body "$base/embeddedfoo" "$work/foo.html") || return 1
  [[ $code == 404 ]] || return 1
  ! grep -q 'name="orbe-not-found"' "$work/foo.html" && ! grep -q 'Get orbe | Orbefab' "$work/foo.html"
}

index_ok() {
  curl -fsS --max-time 30 -o "$work/index.json" "$base/embedded/r/index.json" || return 1
  cmp -s "$dist/r/index.json" "$work/index.json"
}

part_ok() {
  local code record title sha count
  code=$(curl_body "$base/embedded/parts/led-5mm/" "$work/led.html") || return 1
  [[ $code == 200 ]] || return 1
  record=$(find "$dist/r" -maxdepth 1 -name 'led-5mm@*.json' -print -quit)
  [[ -n $record ]] || return 1
  title=$(node -e 'const record = require(require("node:path").resolve(process.argv[1])); if (typeof record.title !== "string" || !Array.isArray(record.files) || record.files.length === 0) process.exit(1); process.stdout.write(record.title)' "$record") || return 1
  grep -F -q "<title>${title} | Orbefab</title>" "$work/led.html" || return 1
  count=0
  while IFS= read -r sha; do
    [[ $sha =~ ^[0-9a-f]{64}$ ]] || return 1
    curl -fsS --max-time 60 -o "$work/led-$sha" "$base/embedded/r/files/$sha" || return 1
    cmp -s "$dist/r/files/$sha" "$work/led-$sha" || return 1
    count=$((count + 1))
  done < <(node -e 'const record = require(require("node:path").resolve(process.argv[1])); for (const file of record.files) { if (!file || typeof file.sha256 !== "string") process.exit(1); process.stdout.write(file.sha256 + "\n") }' "$record")
  [[ $count -gt 0 ]]
}

source_ok() {
  local sha archive
  sha=$(sed -n 's/^runtime_sha256=//p' packages/engine/scripts/sketches.sh)
  archive=$(sed -n 's/^runtime_url=//p' packages/engine/scripts/sketches.sh)
  archive=${archive##*/}
  curl -fsS --max-time 60 -o "$work/runtime" "$base/embedded/sources/$archive" || return 1
  echo "$sha  $work/runtime" | sha256sum -c -
}

asset_cache_ok() {
  local asset rel headers code
  asset=$(find "$dist/assets" -type f -print -quit)
  [[ -n $asset ]] || return 1
  rel=${asset#"$dist/"}
  headers=$work/asset.headers
  code=$(curl -sS --max-time 30 -D "$headers" -o /dev/null -w '%{http_code}' "$base/embedded/$rel") || return 1
  [[ $code == 200 ]] || return 1
  grep -qi '^cache-control:.*max-age=31536000' "$headers"
}

missing_ok() {
  local code
  code=$(curl_body "$base/embedded/no-such-page" "$work/missing.html") || return 1
  [[ $code == 404 ]] || return 1
  grep -q 'name="orbe-not-found"' "$work/missing.html"
}

try 12 index_ok || fail "the live index.json is not the index this build wrote"
try 6 home_ok || fail "Get orbe is not what this build serves at /embedded/"
try 6 root_ok || fail "orbefab.com/ is not the website"
try 6 elsewhere_ok || fail "orbefab.com/embeddedfoo did not answer 404 outside this site"
try 6 part_ok || fail "the led-5mm page is not that part, or it does not download every file of its record"
try 6 source_ok || fail "the runtime source is not the archive sketches.sh pins"
try 6 asset_cache_ok || fail "a hashed asset has no year-long Cache-Control"
try 6 missing_ok || fail "/embedded/no-such-page is not this site's 404"
echo "scripts/site-deployed.sh: the live site matches this build"
