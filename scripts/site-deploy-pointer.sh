#!/usr/bin/env bash
# Restores apps/site/.wrangler/deploy/config.json from apps/site/dist/wrangler-deploy.json
# and checks that the built config it points to names an assets directory. Both `vite preview`
# (through @cloudflare/vite-plugin) and `wrangler dev` follow that pointer. A shared site
# build carries only apps/site/dist, so scripts/check-site.sh and scripts/check-engines.sh
# call this before starting either server.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -gt 1 ]]; then
  echo "usage: scripts/site-deploy-pointer.sh [caller]" >&2
  exit 2
fi

caller=${1:-site-deploy-pointer}
src=apps/site/dist/wrangler-deploy.json
pointer=apps/site/.wrangler/deploy/config.json

if [[ ! -f $src ]]; then
  echo "$caller: the build has no wrangler deploy pointer at $src. Run \`scripts/site.sh\` first." >&2
  exit 1
fi

mkdir -p "$(dirname "$pointer")"
cp "$src" "$pointer"
# shellcheck disable=SC2016 # javascript
node -e '
const [caller, pointer] = process.argv.slice(1);
const fs = require("node:fs");
const path = require("node:path");
const cfg = JSON.parse(fs.readFileSync(pointer, "utf8"));
const configPath = path.resolve(path.dirname(pointer), cfg.configPath);
const built = JSON.parse(fs.readFileSync(configPath, "utf8"));
if (typeof built.assets?.directory !== "string" || built.assets.directory === "") {
  console.error(`${caller}: ${configPath} has no assets directory`);
  process.exit(1);
}
' "$caller" "$pointer"
