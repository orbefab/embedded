#!/usr/bin/env bash
# Builds crates/wasm into one wasm-bindgen package that loads in a browser module Worker and in a
# node worker thread. The folder holds sim_wasm.js, sim_wasm_bg.wasm, their two .d.ts files,
# package.json, LICENSE-MIT, and LICENSE-APACHE. docs/wasm.md says how to load it.
#
# Usage: scripts/wasm.sh [OUT_DIR]
#
# The script needs bash 4.2 or later and rustup.
#
# OUT_DIR defaults to .scratch/wasm/pkg beside the repo. The wasm-bindgen CLI must be the version
# Cargo.lock holds for the crate. The script uses WASM_TOOLS_DIR/wasm-bindgen-<version> when it is
# there, then a wasm-bindgen on PATH at that version, and otherwise installs it into that
# directory from source, which takes about a minute. WASM_TOOLS_DIR defaults to
# .scratch/wasm/tools beside the repo, and checkouts that set one WASM_TOOLS_DIR share one
# install. CARGO_TARGET_DIR is honored, and CARGO_BUILD_JOBS defaults to 4.
#
# The sim-wasm cargo build remaps the checkout, CARGO_HOME, the sysroot, and a
# target directory that sits outside the checkout. The shortest prefix is first.
# The package is built reproducibly and takes no extra rustflags. The child
# cargo receives only those remap flags in CARGO_ENCODED_RUSTFLAGS. The script
# exits 1 when RUSTFLAGS, CARGO_ENCODED_RUSTFLAGS, CARGO_BUILD_RUSTFLAGS, or
# CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS is set and not empty, and when
# a cargo config file contains the text rustflags. That check runs before
# wasm-bindgen is installed.
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

sim_wasm_script=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)

# Later rows replace an equal path. Callers sort the shortest prefix first,
# because rustc keeps the last match.
sim_wasm_repo_path() {
  (cd "$sim_wasm_script/.." && pwd "$1")
}

sim_wasm_cargo_home() {
  local cargo_raw
  cargo_raw=${CARGO_HOME:-${HOME:-}/.cargo}
  cargo_raw=${cargo_raw%/}
  if [[ $cargo_raw != /* ]]; then
    cargo_raw=$(sim_wasm_repo_path -P)/$cargo_raw
  fi
  printf '%s\n' "$cargo_raw"
}

sim_wasm_remap_emit() {
  local from=${1%/}
  [[ $from == /* && $from != / ]] || return 0
  printf '%s\t%s\n' "$from" "$2"
}

sim_wasm_path_inside() {
  [[ $1 == "$2" || $1 == "$2"/* ]]
}

sim_wasm_remap_rows() {
  local repo_raw repo_phys cargo_raw cargo_phys sys_raw sys_phys
  local target_raw target_phys rustc_bin
  repo_raw=$(sim_wasm_repo_path -L)
  repo_phys=$(sim_wasm_repo_path -P)
  cargo_raw=$(sim_wasm_cargo_home)
  if [[ -d $cargo_raw ]]; then
    cargo_phys=$(cd "$cargo_raw" && pwd -P)
  else
    cargo_phys=$cargo_raw
  fi
  rustc_bin=${RUSTC:-rustc}
  sys_raw=$("$rustc_bin" --print sysroot)
  sys_raw=${sys_raw%/}
  sys_phys=$(cd "$sys_raw" && pwd -P)
  if ! target_raw=$(
    cd "$repo_phys" &&
      cargo metadata --format-version 1 --no-deps --color never |
      sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
  ); then
    echo "sim-wasm: cargo metadata failed" >&2
    return 1
  fi
  if [[ -z $target_raw ]]; then
    echo "sim-wasm: cargo metadata did not report a target directory" >&2
    return 1
  fi
  mkdir -p "$target_raw"
  target_phys=$(cd "$target_raw" && pwd -P)
  sim_wasm_remap_emit "$cargo_raw" /cargo
  sim_wasm_remap_emit "$cargo_phys" /cargo
  sim_wasm_remap_emit "$sys_raw" /rustc
  sim_wasm_remap_emit "$sys_phys" /rustc
  if ! sim_wasm_path_inside "$target_raw" "$repo_raw" &&
    ! sim_wasm_path_inside "$target_raw" "$repo_phys"; then
    sim_wasm_remap_emit "$target_raw" /target
  fi
  if ! sim_wasm_path_inside "$target_phys" "$repo_raw" &&
    ! sim_wasm_path_inside "$target_phys" "$repo_phys"; then
    sim_wasm_remap_emit "$target_phys" /target
  fi
  sim_wasm_remap_emit "$repo_raw" /sim-engine
  sim_wasm_remap_emit "$repo_phys" /sim-engine
}

sim_wasm_join_unit() {
  local first=1 arg
  for arg in "$@"; do
    if ((first)); then
      printf '%s' "$arg"
      first=0
    else
      printf '\x1f%s' "$arg"
    fi
  done
}

# Any occurrence of the text rustflags is refused. A comment or a quoted key counts.
sim_wasm_file_has_rustflags() {
  grep -F -q -- 'rustflags' "$1"
}

sim_wasm_rustflags_above() {
  local dir=$1 file
  while true; do
    for file in "$dir/.cargo/config" "$dir/.cargo/config.toml"; do
      if [[ -f $file ]] && sim_wasm_file_has_rustflags "$file"; then
        printf '%s\n' "$file"
        return 0
      fi
    done
    [[ $dir == / ]] && return 1
    dir=${dir%/*}
    [[ -n $dir ]] || dir=/
  done
}

sim_wasm_refuse_other_rustflags() {
  local name cargo_home file raw phys found=
  for name in RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS \
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS; do
    if [[ -v $name && -n ${!name} ]]; then
      found=$name
      break
    fi
  done
  if [[ -z $found ]]; then
    cargo_home=$(sim_wasm_cargo_home)
    for file in "$cargo_home/config" "$cargo_home/config.toml"; do
      if [[ -f $file ]] && sim_wasm_file_has_rustflags "$file"; then
        found=$file
        break
      fi
    done
  fi
  if [[ -z $found ]]; then
    raw=$(sim_wasm_repo_path -L)
    phys=$(sim_wasm_repo_path -P)
    if found=$(sim_wasm_rustflags_above "$phys"); then
      :
    elif [[ $raw != "$phys" ]] && found=$(sim_wasm_rustflags_above "$raw"); then
      :
    else
      found=
    fi
  fi
  if [[ -n $found ]]; then
    echo "sim-wasm: the package is built reproducibly and takes no extra rustflags. Found ${found}." >&2
    return 1
  fi
  return 0
}

sim_wasm_remap_flags() {
  local rows from to bytes
  local -a order=() sortable=()
  local -A dest=()
  rows=$(sim_wasm_remap_rows) || return 1
  while IFS=$'\t' read -r from to || [[ -n ${from:-} ]]; do
    [[ -n ${from:-} ]] || continue
    if [[ -z ${dest[$from]+x} ]]; then
      order+=("$from")
    fi
    dest[$from]=$to
  done <<<"$rows"
  for from in "${order[@]+"${order[@]}"}"; do
    bytes=$(printf '%s' "$from" | wc -c)
    bytes=${bytes//[[:space:]]/}
    sortable+=("${bytes}"$'\t'"${from}"$'\t'"--remap-path-prefix=${from}=${dest[$from]}")
  done
  if ((${#sortable[@]} == 0)); then
    return 0
  fi
  (
    set -o pipefail
    printf '%s\n' "${sortable[@]}" | LC_ALL=C sort -t $'\t' -k1,1n -k2,2 | cut -f3-
  )
}

sim_wasm_encoded_rustflags() {
  local line remap_text
  local -a remaps=()
  remap_text=$(sim_wasm_remap_flags) || return 1
  while IFS= read -r line || [[ -n ${line:-} ]]; do
    [[ -n ${line:-} ]] && remaps+=("$line")
  done <<<"$remap_text"
  if ((${#remaps[@]})); then
    sim_wasm_join_unit "${remaps[@]}"
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
sim_wasm_refuse_other_rustflags || exit 1
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
# The encoded value is on this cargo invocation only, so a native build does not
# inherit the flags. An empty parent value is not a source on the child.
encoded=$(sim_wasm_encoded_rustflags) || exit 1
if [[ -z $encoded ]]; then
  echo "sim-wasm: no remap flags" >&2
  exit 1
fi
env -u RUSTFLAGS \
  -u CARGO_ENCODED_RUSTFLAGS \
  -u CARGO_BUILD_RUSTFLAGS \
  -u CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS \
  CARGO_ENCODED_RUSTFLAGS="$encoded" \
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
  "license": "MIT OR Apache-2.0",
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
    "LICENSE-APACHE",
    "LICENSE-MIT",
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
cp "$repo/LICENSE-APACHE" "$repo/LICENSE-MIT" "$out/"
echo "$out"
