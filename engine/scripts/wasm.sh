#!/usr/bin/env bash
# Builds crates/wasm into one wasm-bindgen package that loads in a browser module Worker and in a
# node worker thread. The folder holds sim_wasm.js, sim_wasm_bg.wasm, their two .d.ts files, and
# package.json. docs/wasm.md says how to load it.
#
# Usage: scripts/wasm.sh [OUT_DIR]
#
# OUT_DIR defaults to .scratch/wasm/pkg beside the repo. The wasm-bindgen CLI must be the version
# Cargo.lock holds for the crate. The script uses WASM_TOOLS_DIR/wasm-bindgen-<version> when it is
# there, then a wasm-bindgen on PATH at that version, and otherwise installs it into that
# directory from source, which takes about a minute. WASM_TOOLS_DIR defaults to
# .scratch/wasm/tools beside the repo, and checkouts that set one WASM_TOOLS_DIR share one
# install. CARGO_TARGET_DIR is honored, and CARGO_BUILD_JOBS defaults to 4.
#
# package.json's simEngine.commit is SIM_COMMIT when that is set. Otherwise, in this git checkout,
# it is git rev-parse HEAD, plus -dirty when git status --porcelain lists a change. An untracked
# file counts. SIM_COMMIT must be 40 lowercase hex digits with an optional -dirty suffix, or the
# script exits 1. With no SIM_COMMIT outside a checkout, simEngine.commit is null, the script
# prints one warning, and the build continues. An archive of the repo is that case.
#
# Sourced, this file defines sim_wasm_workspace_version and sim_wasm_git_root. Executed, it builds.
# sim_wasm_git_root prints the git toplevel of its argument after pwd -P, or nothing when git has
# no toplevel.

sim_wasm_workspace_version() {
  awk '
    $0 == "[workspace.package]" { section = 1; next }
    section && /^\[/ { exit }
    section && /^version[[:space:]]*=/ {
      sub(/^version[[:space:]]*=[[:space:]]*"/, "")
      sub(/".*$/, "")
      print
      exit
    }
  ' "$1"
}

sim_wasm_git_root() {
  local root
  root=$(git -C "$1" rev-parse --show-toplevel 2>/dev/null || true)
  if [[ -n "$root" ]]; then
    (cd "$root" && pwd -P)
  fi
}

if [[ ${BASH_SOURCE[0]} != "$0" ]]; then
  return 0
fi

set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "usage: $0 [OUT_DIR]" >&2
  exit 2
fi
if ! command -v rustup >/dev/null; then
  echo "rustup not found" >&2
  exit 2
fi
repo=$(cd "$(dirname "$0")/.." && pwd -P)
out=${1:-$repo/../.scratch/wasm/pkg}
tools_dir=${WASM_TOOLS_DIR:-$repo/../.scratch/wasm/tools}
[[ $out == /* ]] || out=$PWD/$out
[[ $tools_dir == /* ]] || tools_dir=$PWD/$tools_dir

# The workspace version and the source commit are fixed before the build, so a second run on the
# same tree writes the same package.json. An archive extracted inside some other repo is not this
# checkout, and must not inherit that repo's HEAD.
workspace_version=$(sim_wasm_workspace_version "$repo/Cargo.toml")
if [[ ! $workspace_version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]]; then
  echo "sim-wasm: Cargo.toml [workspace.package] version must be X.Y.Z or X.Y.Z with a semver pre-release" >&2
  exit 1
fi
source_commit=
if [[ -n ${SIM_COMMIT:-} ]]; then
  source_commit=$SIM_COMMIT
else
  git_root=$(sim_wasm_git_root "$repo")
  if [[ $git_root == "$repo" ]] && source_commit=$(git -C "$repo" rev-parse HEAD 2>/dev/null); then
    if [[ -n $(git -C "$repo" status --porcelain --untracked-files=normal) ]]; then
      source_commit=${source_commit}-dirty
    fi
  else
    source_commit=
    echo "sim-wasm: no source commit, so simEngine.commit is null" >&2
  fi
fi
if [[ -n $source_commit && ! $source_commit =~ ^[0-9a-f]{40}(-dirty)?$ ]]; then
  echo "sim-wasm: commit must be 40 lowercase hex digits, with an optional -dirty suffix" >&2
  exit 1
fi

cd "$repo"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

if ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
  rustup target add wasm32-unknown-unknown
fi

version=$(awk '$0 == "name = \"wasm-bindgen\"" { getline; gsub(/version = |"/, ""); print }' \
  Cargo.lock)
tools=$tools_dir/wasm-bindgen-$version
bindgen=$tools/bin/wasm-bindgen
if [[ ! -x $bindgen ]] && found=$(command -v wasm-bindgen); then
  have=$("$found" --version | awk '{ print $2 }')
  if [[ $have == "$version" ]]; then
    bindgen=$found
  else
    echo "ignoring $found, which is wasm-bindgen $have, because Cargo.lock holds $version" >&2
  fi
fi
if [[ ! -x $bindgen ]]; then
  mkdir -p "$tools_dir"
  # Each install builds in its own directory and renames its result into place, so two at once
  # do not share files, and the first to finish wins.
  build=$(mktemp -d "$tools_dir/install.XXXXXX")
  trap 'rm -rf "$build"' EXIT
  install=(cargo install wasm-bindgen-cli --version "=$version" --locked --root)
  echo "installing wasm-bindgen $version into $tools" >&2
  if ! CARGO_TARGET_DIR=$build/target "${install[@]}" "$build/root"; then
    echo "could not install wasm-bindgen $version. Install it with:" >&2
    echo "  ${install[*]} $tools" >&2
    exit 1
  fi
  if ! moved=$(mv -T "$build/root" "$tools" 2>&1) && [[ ! -x "$bindgen" ]]; then
    echo "could not move wasm-bindgen $version into $tools: $moved" >&2
    exit 1
  fi
fi

mkdir -p "$out"
cargo build -p sim-wasm --release --locked --target wasm32-unknown-unknown
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
"$bindgen" --target web --out-dir "$out" "$target/wasm32-unknown-unknown/release/sim_wasm.wasm"
# init stores the wasm instance on the module, so the manifest omits sideEffects.
# sim_wasm_bg.wasm.d.ts types the wasm module's exports. It does not type a URL, so the wasm
# export stays a path.
if [[ -n $source_commit ]]; then
  commit_json="\"$source_commit\""
else
  commit_json=null
fi
cat >"$out/package.json" <<EOF
{
  "name": "sim-wasm",
  "version": "$workspace_version",
  "private": true,
  "type": "module",
  "main": "./sim_wasm.js",
  "module": "./sim_wasm.js",
  "types": "./sim_wasm.d.ts",
  "exports": {
    ".": {
      "types": "./sim_wasm.d.ts",
      "default": "./sim_wasm.js"
    },
    "./sim_wasm_bg.wasm": "./sim_wasm_bg.wasm"
  },
  "files": [
    "package.json",
    "sim_wasm.d.ts",
    "sim_wasm.js",
    "sim_wasm_bg.wasm",
    "sim_wasm_bg.wasm.d.ts"
  ],
  "simEngine": {
    "commit": $commit_json
  }
}
EOF
echo "$out"
