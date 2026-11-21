#!/usr/bin/env bash
# Prints whether a diff should run the engine CI jobs.
# Usage: scripts/ci-engine-changed.sh <base> <head>
# Prints `run=true` or `run=false`. The diff is the changes <head> made since
# the merge base of the two commits. The jobs run unless every changed file is
# markdown, under apps/site/, under docs/, or under .github/ except
# .github/workflows/ci.yml and .github/actions/. packages/ and the rest of what
# the engine, the wasm build or the viewer build reads are not in that list. An unknown argument, or
# a commit git cannot resolve, exits 2.
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

# A file the engine jobs can skip. ci.yml and the setup actions change how those jobs run.
skippable() {
  local path=$1
  [[ $path == *.md ]] && return 0
  [[ $path == .github/workflows/ci.yml ]] && return 1
  under "$path" .github/actions && return 1
  under "$path" apps/site && return 0
  under "$path" docs && return 0
  under "$path" .github && return 0
  return 1
}

run=false
while IFS= read -r path; do
  [[ -n $path ]] || continue
  if ! skippable "$path"; then
    run=true
    break
  fi
done < <(git diff --name-only --no-renames "$1...$2")
echo "run=$run"
