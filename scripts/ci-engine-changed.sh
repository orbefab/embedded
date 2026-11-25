#!/usr/bin/env bash
# Prints whether a diff should run the engine CI jobs.
# Usage:
#   scripts/ci-engine-changed.sh <base> <head>
#   scripts/ci-engine-changed.sh --worktree <base>
# Prints `run=true` or `run=false`. With two commits, the diff is the changes
# <head> made since the merge base of the two commits; an empty diff prints
# `run=false`. With --worktree, it compares committed, staged, unstaged and
# untracked files against the merge base of <base> and HEAD; an empty diff
# prints `run=true` so a clean checkout runs everything. The jobs run unless
# every changed file is README.md, AGENTS.md, under apps/site/, under docs/
# except docs/engine/diagnostics.md, docs/engine/netlist.md and
# docs/engine/run-folder.md (read by packages/engine tests), or under .github/
# except .github/workflows/ci.yml and .github/actions/. Skills, Blocks, parts,
# packages/ and the rest of what the engine, the wasm build or the viewer build
# reads are not in that list. An unknown argument, or a commit git cannot
# resolve, exits 2.
set -euo pipefail
cd "$(dirname "$0")/.."

mode=commits
if [[ $# -eq 2 && $1 == --worktree && -n ${2:-} ]]; then
  mode=worktree
  if ! base=$(git merge-base "$2" HEAD 2>/dev/null); then
    echo "ci-engine-changed: cannot resolve merge base of $2 and HEAD" >&2
    exit 2
  fi
  changed_paths() {
    git diff --name-only --no-renames "$base"
    git diff --cached --name-only --no-renames "$base"
    git ls-files --others --exclude-standard
  }
elif [[ $# -eq 2 && -n ${1:-} && -n ${2:-} && $1 != -* ]]; then
  if ! git rev-parse --verify --quiet "$1^{commit}" >/dev/null || ! git rev-parse --verify --quiet "$2^{commit}" >/dev/null; then
    echo "ci-engine-changed: cannot resolve $1 or $2" >&2
    exit 2
  fi
  base=$1
  head=$2
  changed_paths() {
    git diff --name-only --no-renames "$base...$head"
  }
else
  echo "usage: scripts/ci-engine-changed.sh <base> <head> | --worktree <base>" >&2
  exit 2
fi

under() {
  local path=$1 prefix=$2
  [[ $path == "$prefix" || ${path#"$prefix"/} != "$path" ]]
}

# A file the engine jobs and fast Rust checks can skip. ci.yml and
# .github/actions/ change how CI runs, and three files under docs/engine/ are
# read by packages/engine tests.
skippable() {
  local path=$1
  case "$path" in
    README.md | AGENTS.md) return 0 ;;
    .github/workflows/ci.yml) return 1 ;;
    docs/engine/diagnostics.md | docs/engine/netlist.md | docs/engine/run-folder.md) return 1 ;;
  esac
  under "$path" .github/actions && return 1
  under "$path" apps/site && return 0
  under "$path" docs && return 0
  under "$path" .github && return 0
  return 1
}

run=false
saw_path=0
while IFS= read -r path; do
  [[ -n $path ]] || continue
  saw_path=1
  if ! skippable "$path"; then
    run=true
    break
  fi
done < <(changed_paths)
if [[ $mode == worktree && $saw_path -eq 0 ]]; then
  run=true
fi
echo "run=$run"
