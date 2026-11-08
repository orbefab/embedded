#!/usr/bin/env bash
# Exits 2 with the command to run when the repo's Node dependencies are not installed. Scripts call
# it instead of installing them.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ ! -f node_modules/.modules.yaml ]]; then
  echo "${1:-$0}: Node dependencies are missing. Run: pnpm install --frozen-lockfile" >&2
  exit 2
fi
