#!/usr/bin/env bash
# Builds crates/wasm into one wasm-bindgen package that loads in a browser module Worker and in a
# node worker thread: sim_wasm.js, sim_wasm_bg.wasm and their two .d.ts files. docs/wasm.md says
# how to load it.
#
# Usage: scripts/wasm.sh [OUT_DIR]
#
# OUT_DIR defaults to .scratch/wasm/pkg beside the repo. The wasm-bindgen CLI must be the version
# Cargo.lock holds for the crate. The script uses WASM_TOOLS_DIR/wasm-bindgen-<version> when it is
# there, then a wasm-bindgen on PATH at that version, and otherwise installs it into that
# directory from source, which takes about a minute. WASM_TOOLS_DIR defaults to
# .scratch/wasm/tools beside the repo, and checkouts that set one WASM_TOOLS_DIR share one
# install. CARGO_TARGET_DIR is honored, and CARGO_BUILD_JOBS defaults to 4.
set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "usage: $0 [OUT_DIR]" >&2
  exit 2
fi
if ! command -v rustup >/dev/null; then
  echo "rustup not found" >&2
  exit 2
fi
repo=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$repo/../.scratch/wasm/pkg}
tools_dir=${WASM_TOOLS_DIR:-$repo/../.scratch/wasm/tools}
[[ $out == /* ]] || out=$PWD/$out
[[ $tools_dir == /* ]] || tools_dir=$PWD/$tools_dir
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
echo "$out"
