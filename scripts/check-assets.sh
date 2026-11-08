#!/usr/bin/env bash
# Runs the asset step's tests, rebuilds every part that has model.py, then reruns the step
# from the models commit each other part's asset.json records. Fails if that changes any file
# under parts/. Needs Node 24, the packages/cad environment, and a checkout of orbefab/models,
# which it only reads. check.sh does not run this.
set -euo pipefail
cd "$(dirname "$0")/.."
models=${1:?usage: scripts/check-assets.sh <path to an orbefab/models checkout>}
scripts/need-node-deps.sh scripts/check-assets.sh
node --test packages/asset-step/asset-step.test.mjs
scripts/model.sh --all
node packages/asset-step/asset-step.mjs --models "$models"
changed=$(git status --porcelain -- parts)
if [ -n "$changed" ]; then
  echo "the asset step changed parts/:"
  echo "$changed"
  exit 1
fi
echo "parts/ is unchanged"
