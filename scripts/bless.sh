#!/usr/bin/env bash
# Rewrites the parts.lock fixture from parts/ and refreshes each example with
# `orbe add --overwrite`. The asset step is left out. It needs a checkout of
# orbefab/models. Run scripts/check-assets.sh for that.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
usage: scripts/bless.sh

Rewrite the parts.lock fixture from parts/, then refresh each example with orbe add --overwrite.
The asset step is left out. It needs a checkout of orbefab/models.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi
if [[ $# -ne 0 ]]; then
  usage >&2
  exit 2
fi

ORBE_BLESS=1 cargo test --locked -p orbe-scene --test read parts_lock_matches_the_catalog -- --exact

cargo build --locked -p orbe
orbe="$PWD/target/debug/orbe"

for scene in examples/*/scene.jsonc; do
  dir=$(dirname "$scene")
  mapfile -t ids < <(
    {
      grep -oE '"type"[[:space:]]*:[[:space:]]*"[^"./][^"]*"' "$scene" \
        | sed -E 's/.*"([^"]*)"$/\1/'
      if [[ -f "$dir/parts.lock" ]]; then
        awk 'NF { print $1 }' "$dir/parts.lock"
      fi
    } | sort -u
  )
  "$orbe" add -C "$dir" --overwrite "${ids[@]}"
done
