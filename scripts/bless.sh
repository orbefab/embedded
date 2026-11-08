#!/usr/bin/env bash
# Rewrites the parts.lock fixture from parts/ and refreshes each example from
# the catalog ids its scene names. A copy the scene no longer names is removed.
# Build a part's model with scripts/model.sh first. This script does not.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
usage: scripts/bless.sh

Rewrite the parts.lock fixture from parts/, then refresh each example from the catalog ids its scene names.
A copy the scene no longer names is removed.
Build a part's model with scripts/model.sh first. This script does not.
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

log=$(mktemp)
trap 'rm -f "$log"' EXIT
ORBE_BLESS=1 cargo test --locked --color never -p orbe-scene --test read parts_lock_matches_the_catalog -- --exact 2>&1 | tee "$log"
if ! grep -q 'test result: ok\. 1 passed;' "$log"; then
  echo "scripts/bless.sh: parts_lock_matches_the_catalog did not pass once" >&2
  exit 1
fi
