#!/usr/bin/env bash
# Rewrites the scene tests' parts.lock fixture from parts/, and the viewer's protocol.ts from the
# viewer messages in packages/session. A Block has no parts.lock to bless, since it reads its parts
# from the catalog built into orbe.
# Build a part's model with scripts/model.sh first. This script does not.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
usage: scripts/bless.sh

Rewrite the scene tests' parts.lock fixture from parts/, and
packages/viewer/src/protocol.ts from the viewer messages in packages/session.
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

# Runs the one test `$1` of `cargo test` on the target the other arguments name, with ORBE_BLESS
# set, which rewrites what it checks.
bless() {
  local name=$1
  shift
  ORBE_BLESS=1 cargo test --locked --color never "$@" "$name" -- --exact 2>&1 | tee "$log"
  if ! grep -q 'test result: ok\. 1 passed;' "$log"; then
    echo "scripts/bless.sh: $name did not pass once" >&2
    exit 1
  fi
}

bless parts_lock_matches_the_catalog -p orbe-scene --test read
bless typescript::protocol_ts_is_the_generated_one -p orbe-session --lib
