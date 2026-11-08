#!/usr/bin/env bash
# Builds parts/<id>/model.py into parts/<id>/<id>.glb, then runs the asset step
# on those parts. Prints the bless command when it finishes. Never installs anything.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
usage: scripts/model.sh <id>...
       scripts/model.sh --all

Build each part's model.py into parts/<id>/<id>.glb, then run the asset step on those parts.
Rebuild only the parts you changed. The same bytes on every run are proven on Linux x86_64.
--all builds every part that has model.py.

Needs uv, Node 24, and the installed environments. This script does not install them.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi
if [[ $# -eq 0 ]]; then
  usage >&2
  exit 2
fi

if ! command -v uv >/dev/null 2>&1; then
  echo "scripts/model.sh: uv is not installed. Install it from https://docs.astral.sh/uv/getting-started/installation/" >&2
  exit 1
fi
if ! command -v node >/dev/null 2>&1; then
  echo "scripts/model.sh: Node is not installed. Install Node 24." >&2
  exit 1
fi
node_major=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || true)
node_major=${node_major//[^0-9]/}
if [[ -z "$node_major" || "$node_major" -lt 24 ]]; then
  echo "scripts/model.sh: Node 24 is required. Install Node 24." >&2
  exit 1
fi
if [[ ! -e packages/asset-step/node_modules/@gltf-transform/core && ! -e node_modules/@gltf-transform/core ]]; then
  echo "scripts/model.sh: Node dependencies are missing. Run: pnpm install --frozen-lockfile" >&2
  exit 1
fi
py=packages/cad/.venv/bin/python
if [[ ! -x "$py" ]] || ! UV_PYTHON_DOWNLOADS=never "$py" -c 'import build123d, cad' >/dev/null 2>&1; then
  echo "scripts/model.sh: the model environment is not installed. Run: uv sync --project packages/cad --frozen" >&2
  exit 1
fi

if [[ "$1" == "--all" ]]; then
  if [[ $# -ne 1 ]]; then
    usage >&2
    exit 2
  fi
  ids=()
  while IFS= read -r script; do
    ids+=("$(basename "$(dirname "$script")")")
  done < <(find parts -mindepth 2 -maxdepth 2 -name model.py | sort)
  if [[ ${#ids[@]} -eq 0 ]]; then
    echo "scripts/model.sh: no part has model.py" >&2
    exit 1
  fi
else
  ids=("$@")
fi

for id in "${ids[@]}"; do
  if [[ "$id" == -* || "$id" == */* || ! -f "parts/$id/model.py" ]]; then
    echo "scripts/model.sh: parts/$id/model.py does not exist" >&2
    exit 1
  fi
done

raw=$(mktemp -d)
trap 'rm -rf "$raw"' EXIT
for id in "${ids[@]}"; do
  mkdir -p "$raw/$id"
  UV_PYTHON_DOWNLOADS=never "$py" "parts/$id/model.py" "$raw/$id"
done

node packages/asset-step/asset-step.mjs --local --raw "$raw" "${ids[@]}"
echo "Run scripts/bless.sh."
