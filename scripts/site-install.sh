#!/usr/bin/env bash
# The install-on assertions for a site build already in apps/site/dist/client. It also checks
# that the build has a page for every part and Block the registry lists.
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
listed() {
  # console.log colors a number when FORCE_COLOR is set, and [[ ]] then rejects the count.
  node -e 'const index = require(process.argv[1]); process.stdout.write(String(new Set(index.items.filter((row) => row.type === process.argv[2]).map((row) => row.name)).size) + "\n")' \
    "$PWD/$dist/r/index.json" "$1"
}
parts=$(pages parts)
blocks=$(pages blocks)
embeds=$(find "$dist/blocks" -mindepth 3 -maxdepth 3 -name index.html | wc -l)
[[ $parts -gt 0 && $blocks -gt 0 && $parts -eq $(listed part) && $blocks -eq $(listed block) && $embeds -eq $blocks ]] ||
  fail "the build has $parts part, $blocks Block and $embeds embed pages, and the registry lists $(listed part) and $(listed block)"
[[ $(grep -rlE 'orbe add [a-z]' "$dist/parts" "$dist/blocks" --include=index.html | wc -l) -eq $((parts + blocks)) ]] ||
  fail "not every part and Block page shows its orbe add line"
grep -q 'curl -fsSL https://orbefab.com/embedded/install.sh | sh' "$dist/index.html" ||
  fail "Get orbe shows no install command"
grep -q 'href="https://github.com/orbefab/orbe"' "$dist/index.html" ||
  fail "Get orbe links no repo"
[[ -f $dist/install.sh ]] || fail "the build does not serve install.sh"
cmp -s scripts/install.sh "$dist/install.sh" ||
  fail "install.sh is not scripts/install.sh"
echo "scripts/site-install.sh: orbe add is on $parts part and $blocks Block pages, and Get orbe shows the install command and the repo"
