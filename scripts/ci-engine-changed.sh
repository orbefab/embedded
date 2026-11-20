#!/usr/bin/env bash
# Prints whether a diff should run the engine CI jobs.
# Usage: scripts/ci-engine-changed.sh <base> <head>
# Prints `run=true` or `run=false`. An unknown argument, or a commit git cannot
# resolve, exits 2.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -ne 2 || -z ${1:-} || -z ${2:-} ]]; then
  echo "usage: scripts/ci-engine-changed.sh <base> <head>" >&2
  exit 2
fi
if ! git rev-parse --verify --quiet "$1^{commit}" >/dev/null || ! git rev-parse --verify --quiet "$2^{commit}" >/dev/null; then
  echo "ci-engine-changed: cannot resolve $1 or $2" >&2
  exit 2
fi

under() {
  local path=$1 prefix=$2
  [[ $path == "$prefix" || ${path#"$prefix"/} != "$path" ]]
}

run=false
while IFS= read -r path; do
  [[ -n $path ]] || continue
  if under "$path" packages/engine || under "$path" packages/cores || under "$path" packages/socs; then
    run=true
    break
  fi
  case $path in
    Cargo.toml | Cargo.lock | scripts/check.sh | scripts/check-wasm.sh | scripts/wasm.sh | scripts/install-wasm-bindgen.sh | scripts/ci-engine-changed.sh | .github/workflows/ci.yml)
      run=true
      break
      ;;
  esac
done < <(git diff --name-only "$1" "$2")
echo "run=$run"
