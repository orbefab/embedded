#!/usr/bin/env bash
# Checks the orbe-wasm package. It builds the package with scripts/wasm.sh, builds it again with a
# fresh target directory and requires the same bytes, checks package.json and the .d.ts surface,
# and rejects a package that holds a host path. Then it builds this repo's registry with
# `orbe registry build` and runs packages/wasm/tests/worker.mjs, which opens each Block in the
# package's worker.js in a node worker thread, plays it and presses it, and times one simulated
# second of nano-button-led.
#
# The host path check fails when a regular file contains $HOME/ and HOME is not / and not equal
# to /orbe, /cargo, /rustc, or /target, or a remapped host prefix with a trailing slash. A prefix
# equal to one of those four is not a needle, because the build writes that replacement into the
# package. A match is a leak only when it starts a path. The check walks back from the match
# over / bytes. The match is a leak when that reaches the start of the file, or the byte there is
# outside [A-Za-z0-9._/-]. A host path glued on after other path characters, such as
# abc/home/u/x, is a known miss. Before the build, a fixture that only names replacements passes
# under HOME /cargo, /rustc, /orbe, and /target, and a fixture that holds /probe-home/secret
# fails under HOME /probe-home.
#
# Usage: scripts/check-wasm.sh [SCRATCH_DIR]
#
# SCRATCH_DIR defaults to .scratch/wasm/check in the repo, and the script removes it when it
# passes. A SCRATCH_DIR the caller names is kept, with the package in pkg and the registry in
# registry, so scripts/check.sh full hands both to scripts/check-speed.sh. It needs what scripts/wasm.sh needs, and node 24 or later, which runs the viewer's
# TypeScript registry client as it is. It installs nothing.
#
# Sourced, this file only defines orbe_wasm_check_manifest and orbe_wasm_reject_host_paths.
# Source scripts/wasm.sh first. With ORBE_COMMIT unset, the manifest check requires a commit
# stamp when this directory is the git checkout. An export may leave the commit null.

orbe_wasm_check_manifest() {
  local pkg_dir=$1
  local repo=$2
  local expect_version require_commit repo_physical git_root
  expect_version=$(orbe_wasm_workspace_version "$repo/Cargo.toml")
  require_commit=0
  if [[ -z ${ORBE_COMMIT:-} ]]; then
    repo_physical=$(cd "$repo" && pwd -P)
    git_root=$(orbe_wasm_git_root "$repo")
    if [[ "$git_root" == "$repo_physical" ]]; then
      require_commit=1
    fi
  fi
  node - "$pkg_dir" "$expect_version" "$require_commit" <<'JS'
const fs = require('fs');
const path = require('path');
const pkgDir = process.argv[2];
const expectVersion = process.argv[3];
const requireCommit = process.argv[4] === '1';
let manifest;
try {
  manifest = JSON.parse(fs.readFileSync(path.join(pkgDir, 'package.json'), 'utf8'));
} catch (error) {
  console.error(`package.json is not valid JSON: ${error.message}`);
  process.exit(1);
}
if (manifest.name !== 'orbe-wasm') {
  console.error(`package.json name is ${manifest.name}`);
  process.exit(1);
}
if (manifest.version !== expectVersion) {
  console.error(`package.json version ${manifest.version} != workspace ${expectVersion}`);
  process.exit(1);
}
if (manifest.private !== true) {
  console.error('package.json private is not true');
  process.exit(1);
}
if (manifest.license !== 'MIT OR Apache-2.0') {
  console.error(`package.json license is ${manifest.license}`);
  process.exit(1);
}
const files = [
  'LICENSE-APACHE',
  'LICENSE-MIT',
  'package.json',
  'orbe_wasm.d.ts',
  'orbe_wasm.js',
  'orbe_wasm_bg.wasm',
  'orbe_wasm_bg.wasm.d.ts',
  'worker.js',
];
const got = manifest.files;
const sameFiles =
  Array.isArray(got) && got.length === files.length && got.every((name, i) => name === files[i]);
if (!sameFiles) {
  console.error(`package.json files is ${JSON.stringify(got)}`);
  process.exit(1);
}
for (const name of files) {
  const file = path.join(pkgDir, name);
  if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
    console.error(`package.json file is missing: ${name}`);
    process.exit(1);
  }
}
const commit = manifest.orbe && manifest.orbe.commit;
const stamped = typeof commit === 'string' && /^[0-9a-f]{40}(-dirty)?$/.test(commit);
const commitOk = stamped || (commit === null && !requireCommit);
if (!manifest.orbe || !commitOk) {
  const allowed = requireCommit
    ? '40 lowercase hex digits with an optional -dirty suffix'
    : 'null or 40 lowercase hex digits with an optional -dirty suffix';
  console.error(`package.json commit ${commit} is not ${allowed}`);
  process.exit(1);
}
const listed = [
  ['.', manifest.exports && manifest.exports['.']],
  ['./orbe_wasm_bg.wasm', manifest.exports && manifest.exports['./orbe_wasm_bg.wasm']],
  ['./worker.js', manifest.exports && manifest.exports['./worker.js']],
];
for (const [key, value] of listed) {
  const targets = typeof value === 'string' ? [['default', value]] : value && Object.entries(value);
  if (!targets) {
    console.error(`package.json exports lacks ${key}`);
    process.exit(1);
  }
  for (const [condition, target] of targets) {
    if (typeof target !== 'string' || !target.startsWith('./') || target.includes('..')) {
      console.error(`package.json export ${key} ${condition} is ${target}`);
      process.exit(1);
    }
    const file = path.join(pkgDir, target.slice(2));
    if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
      console.error(`package.json export ${key} does not resolve: ${target}`);
      process.exit(1);
    }
  }
}
console.log(`package.json ok ${manifest.name}@${manifest.version} ${manifest.orbe.commit}`);
JS
}

# Source scripts/wasm.sh first. Every regular file under the package is scanned.
# A home of /, or a home equal to a replacement, is not a needle. A prefix equal
# to a replacement is not a needle, because the build writes that replacement
# into the package.
orbe_wasm_skip_host_prefix() {
  local prefix=$1 repl
  for repl in /orbe /cargo /rustc /target; do
    [[ $prefix == "$repl" ]] && return 0
  done
  return 1
}

orbe_wasm_match_is_leak() {
  local file=$1 pos=$2 b
  local -a bytes
  while ((pos > 0)); do
    read -ra bytes < <(od -v -An -t u1 -j $((pos - 1)) -N 1 -- "$file")
    b=${bytes[0]}
    if ((b != 47)); then
      break
    fi
    pos=$((pos - 1))
  done
  if ((pos == 0)); then
    return 0
  fi
  if ((b == 45 || b == 46 || b == 47 || b == 95)); then
    return 1
  fi
  if ((b >= 48 && b <= 57)); then
    return 1
  fi
  if ((b >= 65 && b <= 90)); then
    return 1
  fi
  if ((b >= 97 && b <= 122)); then
    return 1
  fi
  return 0
}

orbe_wasm_file_has_needle() {
  local file=$1 needle=$2 hit off
  while IFS= read -r hit; do
    off=${hit%%:*}
    if orbe_wasm_match_is_leak "$file" "$off"; then
      return 0
    fi
  done < <(grep -a -F -b -o -- "$needle" "$file" || true)
  return 1
}

orbe_wasm_reject_host_paths() {
  local pkg=${1%/} file needle home prefix
  local -a needles=()
  local -A seen=()
  local rows
  if [[ ! -d $pkg ]]; then
    echo "check-wasm: package directory is missing" >&2
    return 1
  fi
  rows=$(orbe_wasm_remap_rows) || return 1
  while IFS=$'\t' read -r prefix _ || [[ -n ${prefix:-} ]]; do
    [[ -n ${prefix:-} ]] || continue
    prefix=${prefix%/}
    orbe_wasm_skip_host_prefix "$prefix" && continue
    needle="${prefix}/"
    if [[ -z ${seen[$needle]+x} ]]; then
      seen[$needle]=1
      needles+=("$needle")
    fi
  done <<<"$rows"
  home=${HOME:-}
  home=${home%/}
  if [[ -n $home && $home != / ]] && ! orbe_wasm_skip_host_prefix "$home"; then
    needle="${home}/"
    if [[ -z ${seen[$needle]+x} ]]; then
      needles+=("$needle")
    fi
  fi
  while IFS= read -r -d '' file; do
    for needle in "${needles[@]+"${needles[@]}"}"; do
      if orbe_wasm_file_has_needle "$file" "$needle"; then
        echo "check-wasm: ${file#"$pkg"/} contains a host path" >&2
        return 1
      fi
    done
  done < <(find "$pkg" -type f -print0)
}

if [[ ${BASH_SOURCE[0]} != "$0" ]]; then
  return 0
fi

set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "usage: $0 [SCRATCH_DIR]" >&2
  exit 2
fi
if ! command -v node >/dev/null; then
  echo "node not found" >&2
  exit 2
fi
if ! node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 24 ? 0 : 1)'; then
  echo "node 24 or later not found, $(command -v node) is $(node --version)" >&2
  exit 2
fi
repo=$(cd "$(dirname "$0")/.." && pwd)
scratch=${1:-$repo/.scratch/wasm/check}
keep=${1:+1}
mkdir -p "$scratch"
scratch=$(cd "$scratch" && pwd)
rm -rf "$scratch/pkg" "$scratch/again" "$scratch/target" "$scratch/registry"

cd "$repo"
# shellcheck source=scripts/wasm.sh
source "$repo/scripts/wasm.sh"
home_case=$scratch/home-needle
rm -rf "$home_case"
mkdir -p "$home_case"
printf '%s\n' '/cargo/registry' '/rustc/lib' '/orbe/packages/x' '/target/release/x' >"$home_case/replaced.bin"
# Cargo and rustup keep the caller's homes. Only the needle sees the trial HOME.
caller_cargo=${CARGO_HOME:-$HOME/.cargo}
caller_rustup=${RUSTUP_HOME:-$HOME/.rustup}
for home in /cargo /rustc /orbe /target; do
  if ! CARGO_HOME=$caller_cargo RUSTUP_HOME=$caller_rustup HOME=$home \
    orbe_wasm_reject_host_paths "$home_case"; then
    echo "check-wasm: HOME=$home rejected a package that only names replacements" >&2
    exit 1
  fi
done
printf '%s\n' '/probe-home/secret' >"$home_case/replaced.bin"
if msg=$(
  CARGO_HOME=$caller_cargo RUSTUP_HOME=$caller_rustup HOME=/probe-home \
    orbe_wasm_reject_host_paths "$home_case" 2>&1
); then
  echo "check-wasm: HOME=/probe-home missed a host path" >&2
  exit 1
fi
if [[ $msg != *"contains a host path"* ]]; then
  echo "check-wasm: HOME=/probe-home failed for another reason: $msg" >&2
  exit 1
fi
rm -rf "$home_case"
echo "check-wasm: a replacement home is not a leak"

"$repo/scripts/wasm.sh" "$scratch/pkg"
CARGO_TARGET_DIR=$scratch/target "$repo/scripts/wasm.sh" "$scratch/again"
if ! diff -r "$scratch/pkg" "$scratch/again" >/dev/null; then
  echo "check-wasm: a build with a fresh target directory wrote other bytes" >&2
  diff -rq "$scratch/pkg" "$scratch/again" >&2 || true
  exit 1
fi
rm -rf "$scratch/again" "$scratch/target"
echo "check-wasm: two builds wrote the same bytes"
orbe_wasm_reject_host_paths "$scratch/pkg"
orbe_wasm_check_manifest "$scratch/pkg" "$repo"
for line in \
  'add(path: string, bytes: Uint8Array): void;' \
  'static open(name: string, block: Block): Session;' \
  'command(text: string): void;' \
  'tick(wall_ms: number): void;' \
  'playing(): boolean;' \
  'messages(): string[];' \
  'export function tickMs(): number;' \
  'export function sceneParts(block: Block): string;'; do
  if ! grep -qF "$line" "$scratch/pkg/orbe_wasm.d.ts"; then
    echo "check-wasm: orbe_wasm.d.ts lacks $line" >&2
    exit 1
  fi
done

export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
"$target/debug/orbe" registry build . "$scratch/registry" >/dev/null
node packages/wasm/tests/worker.mjs "$scratch/pkg" "$scratch/registry"
[[ -n $keep ]] || rm -rf "$scratch"
