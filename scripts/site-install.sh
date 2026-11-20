#!/usr/bin/env bash
# The install-on assertions for a site build already in apps/site/dist/client.
# scripts/check-site.sh runs this after it builds with ORBE_SITE_INSTALL=1. The site
# workflow runs it after rebuilding with that flag, which is the build that is uploaded.
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
  echo "scripts/site-install.sh: $1" >&2
  exit 1
}

dist=apps/site/dist/client
[[ -f $dist/index.html ]] || fail "apps/site/dist/client is not a site build"
pages() { find "$dist/$1" -mindepth 2 -maxdepth 2 -name index.html | wc -l; }
parts=$(pages parts)
blocks=$(pages blocks)
[[ $(grep -rlE 'orbe add [a-z]' "$dist/parts" "$dist/blocks" --include=index.html | wc -l) -eq $((parts + blocks)) ]] ||
  fail "not every part and Block page shows its orbe add line"
grep -q 'curl -fsSL https://orbefab.com/embedded/install.sh | sh' "$dist/index.html" ||
  fail "Get orbe shows no install command"
grep -q 'href="https://github.com/orbefab/embedded"' "$dist/index.html" ||
  fail "Get orbe links no repo"
[[ -f $dist/install.sh ]] || fail "the build does not serve install.sh"
cmp -s scripts/install.sh "$dist/install.sh" ||
  fail "install.sh is not scripts/install.sh"
echo "scripts/site-install.sh: orbe add is on $parts part and $blocks Block pages, and Get orbe shows the install command and the repo"
