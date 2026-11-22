#!/usr/bin/env bash
# Runs the asset step's tests, rebuilds every part that has model.py, and fails if that
# changes any file under parts/. Needs Node 24 and the packages/cad environment.
# check.sh does not run this.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $# -ne 0 ]]; then
  echo "usage: scripts/check-assets.sh" >&2
  exit 2
fi
scripts/need-node-deps.sh scripts/check-assets.sh
node --test packages/asset-step/asset-step.test.mjs
scripts/model.sh --all
changed=$(git status --porcelain -- parts)
if [ -n "$changed" ]; then
  echo "the asset step changed parts/:"
  echo "$changed"
  exit 1
fi
echo "parts/ is unchanged"
