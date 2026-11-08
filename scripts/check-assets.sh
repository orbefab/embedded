#!/usr/bin/env bash
# Runs the asset step's tests, then reruns the step from the models commit each part's asset.json
# records, and fails if that changes any file under parts/. A part with model.py is left as
# scripts/model.sh wrote it. Needs Node 24 and a checkout of orbefab/models, which it only reads.
# check.sh does not run this, so it needs neither.
set -euo pipefail
cd "$(dirname "$0")/.."
models=${1:?usage: scripts/check-assets.sh <path to an orbefab/models checkout>}
pnpm install --frozen-lockfile
node --test packages/asset-step/asset-step.test.mjs
node packages/asset-step/asset-step.mjs --models "$models"
changed=$(git status --porcelain -- parts)
if [ -n "$changed" ]; then
  echo "the asset step changed parts/:"
  echo "$changed"
  exit 1
fi
echo "parts/ is unchanged"
