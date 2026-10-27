#!/usr/bin/env bash
# Builds the wasm package with scripts/wasm.sh and runs crates/wasm/tests/worker.mjs, which drives
# it in node worker threads and checks its folders and serial rows against native `sim run` on
# every example, byte for byte. scripts/wasm-check.sh needs bash 4.2 or later, node 22.7 or later, and rustup. scripts/wasm.sh needs bash 4.2 or later and rustup.
# It also fails when a regular file contains $HOME/ and HOME is not / and not
# equal to /sim-engine, /cargo, /rustc, or /target, or a remapped host prefix
# with a trailing slash. A prefix equal to one of those four is not a needle,
# because the build writes that replacement into the package. A match is
# a leak only when it starts a path. The check walks back from the match over / bytes.
# The match is a leak when that reaches the start of the file, or the byte there is
# outside [A-Za-z0-9._/-]. A host path glued on after other path characters, such as
# abc/home/u/x, is a known miss. When HOME equals a replacement, such as /cargo,
# a true host path under it passes, because it reads the same as the remapped
# path. Before the build, a fixture that only names
# replacements passes under HOME /cargo, /rustc, /sim-engine, and /target. A fixture
# that holds /probe-home/secret fails under HOME /probe-home.
#
# Usage: scripts/wasm-check.sh [SCRATCH_DIR]
#
# SCRATCH_DIR defaults to .scratch/wasm/check beside the repo. The package goes in SCRATCH_DIR/pkg,
# the native folders in SCRATCH_DIR/native, and the input cases' files in SCRATCH_DIR/work.
#
# Sourced, this file only defines sim_wasm_check_manifest. Source scripts/wasm.sh first. Executed,
# this file builds the package and runs the Worker test. With SIM_COMMIT unset, that check requires
# a commit stamp when this directory is the git checkout. An export may leave the commit null.

sim_wasm_check_manifest() {
  local pkg_dir=$1
  local repo=$2
  local expect_version require_commit repo_physical git_root
  expect_version=$(sim_wasm_workspace_version "$repo/Cargo.toml")
  require_commit=0
  if [[ -z ${SIM_COMMIT:-} ]]; then
    repo_physical=$(cd "$repo" && pwd -P)
    git_root=$(sim_wasm_git_root "$repo")
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
if (manifest.name !== 'sim-wasm') {
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
  'sim_wasm.d.ts',
  'sim_wasm.js',
  'sim_wasm_bg.wasm',
  'sim_wasm_bg.wasm.d.ts',
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
const commit = manifest.simEngine && manifest.simEngine.commit;
const stamped = typeof commit === 'string' && /^[0-9a-f]{40}(-dirty)?$/.test(commit);
const commitOk = stamped || (commit === null && !requireCommit);
if (!manifest.simEngine || !commitOk) {
  const allowed = requireCommit
    ? '40 lowercase hex digits with an optional -dirty suffix'
    : 'null or 40 lowercase hex digits with an optional -dirty suffix';
  console.error(`package.json commit ${commit} is not ${allowed}`);
  process.exit(1);
}
const listed = [
  ['.', manifest.exports && manifest.exports['.']],
  ['./sim_wasm_bg.wasm', manifest.exports && manifest.exports['./sim_wasm_bg.wasm']],
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
console.log(`package.json ok ${manifest.name}@${manifest.version} ${manifest.simEngine.commit}`);
JS
}

# Source scripts/wasm.sh first. Every regular file under the package is scanned.
# A home of /, or a home equal to a replacement, is not a needle. A prefix equal
# to a replacement is not a needle, because the build writes that replacement
# into the package.
sim_wasm_skip_host_prefix() {
  local prefix=$1 repl
  for repl in /sim-engine /cargo /rustc /target; do
    [[ $prefix == "$repl" ]] && return 0
  done
  return 1
}

sim_wasm_match_is_leak() {
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

sim_wasm_file_has_needle() {
  local file=$1 needle=$2 hit off
  while IFS= read -r hit; do
    off=${hit%%:*}
    if sim_wasm_match_is_leak "$file" "$off"; then
      return 0
    fi
  done < <(grep -a -F -b -o -- "$needle" "$file" || true)
  return 1
}

sim_wasm_reject_host_paths() {
  local pkg=${1%/} file needle home prefix
  local -a needles=()
  local -A seen=()
  local rows
  if [[ ! -d $pkg ]]; then
    echo "wasm-check: package directory is missing" >&2
    return 1
  fi
  rows=$(sim_wasm_remap_rows) || return 1
  while IFS=$'\t' read -r prefix _ || [[ -n ${prefix:-} ]]; do
    [[ -n ${prefix:-} ]] || continue
    prefix=${prefix%/}
    sim_wasm_skip_host_prefix "$prefix" && continue
    needle="${prefix}/"
    if [[ -z ${seen[$needle]+x} ]]; then
      seen[$needle]=1
      needles+=("$needle")
    fi
  done <<<"$rows"
  home=${HOME:-}
  home=${home%/}
  if [[ -n $home && $home != / ]] && ! sim_wasm_skip_host_prefix "$home"; then
    needle="${home}/"
    if [[ -z ${seen[$needle]+x} ]]; then
      needles+=("$needle")
    fi
  fi
  while IFS= read -r -d '' file; do
    for needle in "${needles[@]+"${needles[@]}"}"; do
      if sim_wasm_file_has_needle "$file" "$needle"; then
        echo "wasm-check: ${file#"$pkg"/} contains a host path" >&2
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
for tool in node rustup; do
  if ! command -v "$tool" >/dev/null; then
    echo "$tool not found" >&2
    exit 2
  fi
done
# The Worker test runs on node 22.7 or later. The generated package.json has "type": "module",
# which is what node before 22.7 needs in order to read sim_wasm.js as a module.
if ! node -e 'const [a, b] = process.versions.node.split(".").map(Number);
  process.exit(a > 22 || (a === 22 && b >= 7) ? 0 : 1)'; then
  echo "node 22.7 or later not found, $(command -v node) is $(node --version)" >&2
  exit 2
fi
repo=$(cd "$(dirname "$0")/.." && pwd)
scratch=${1:-$repo/../.scratch/wasm/check}
mkdir -p "$scratch"
scratch=$(cd "$scratch" && pwd)
rm -rf "$scratch/pkg" "$scratch/native" "$scratch/work"

cd "$repo"
# shellcheck source=scripts/wasm.sh
source "$repo/scripts/wasm.sh"
home_case=$scratch/home-needle
rm -rf "$home_case"
mkdir -p "$home_case"
printf '%s\n' '/cargo/registry' '/rustc/lib' '/sim-engine/crates/x' '/target/release/x' >"$home_case/replaced.bin"
# Cargo and rustup keep the caller's homes. Only the needle sees the trial HOME.
caller_cargo=${CARGO_HOME:-$HOME/.cargo}
caller_rustup=${RUSTUP_HOME:-$HOME/.rustup}
for home in /cargo /rustc /sim-engine /target; do
  if ! (
    export CARGO_HOME=$caller_cargo RUSTUP_HOME=$caller_rustup HOME=$home
    sim_wasm_reject_host_paths "$home_case"
  ); then
    echo "wasm-check: HOME=$home rejected a package that only names replacements" >&2
    exit 1
  fi
done
printf '%s\n' '/probe-home/secret' >"$home_case/replaced.bin"
if msg=$(
  export CARGO_HOME=$caller_cargo RUSTUP_HOME=$caller_rustup HOME=/probe-home
  sim_wasm_reject_host_paths "$home_case" 2>&1
); then
  echo "wasm-check: HOME=/probe-home missed a host path" >&2
  exit 1
fi
if [[ $msg != *"contains a host path"* ]]; then
  echo "wasm-check: HOME=/probe-home failed for another reason: $msg" >&2
  exit 1
fi
rm -rf "$home_case"
echo "wasm-check: a replacement home is not a leak"

"$repo/scripts/wasm.sh" "$scratch/pkg"
sim_wasm_reject_host_paths "$scratch/pkg"
sim_wasm_check_manifest "$scratch/pkg" "$repo"
# The .d.ts types each argument as docs/wasm.md does, and its Code union is the docs' codes.
codes=$(sed -n '/^| `code` |/,/^$/ s/^| `\([a-z_]*\)` | .*/"\1"/p' docs/wasm.md | sed 1d |
  paste -sd '|' | sed 's/|/ | /g')
for line in \
  "export type Code = $codes;" \
  'readonly integrals: Float64Array;' \
  'static open(netlist: Uint8Array): Session;' \
  'static record(netlist: Uint8Array): Session;' \
  'advanceTo(ps: bigint): Status;' \
  'diagnostics(): string;' \
  'serialSince(cursor: bigint): Serial;' \
  'edgesSince(cursor: bigint): Edges;' \
  'edgeCursor(): bigint;' \
  'forgetEdges(cursor: bigint): void;' \
  'forgetSerial(cursor: bigint): void;' \
  'applyContact(part: string, ps: bigint, closed: boolean): Applied;' \
  'applyLevel(part: string, ps: bigint, si: number): Applied;' \
  'applySerial(part: string, ps: bigint, baud: number, bytes: Uint8Array): Applied;' \
  'finish(createdAtSeconds: bigint): Folder;' \
  'export function run(netlist: Uint8Array, createdAtSeconds: bigint): Folder;' \
  'file(path: string): Uint8Array | undefined;'; do
  if ! grep -qF "$line" "$scratch/pkg/sim_wasm.d.ts"; then
    echo "sim_wasm.d.ts lacks $line" >&2
    exit 1
  fi
done
edges=$(sed -n '/^export class Edges {$/,/^}$/p' "$scratch/pkg/sim_wasm.d.ts")
if [[ -z "$edges" ]]; then
  echo "sim_wasm.d.ts has no Edges class" >&2
  exit 1
fi
for line in \
  'readonly at: BigUint64Array;' \
  'readonly signal: Uint32Array;' \
  'readonly level: Uint8Array;' \
  'readonly cursor: bigint;'; do
  if ! grep -qF "$line" <<<"$edges"; then
    echo "Edges in sim_wasm.d.ts lacks $line" >&2
    exit 1
  fi
done
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
cargo build -p sim-cli --release --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
sim=$target/release/sim
for netlist in examples/*/netlist.json; do
  name=$(basename "$(dirname "$netlist")")
  "$sim" run "$netlist" --out "$scratch/native/$name" >/dev/null
done
node crates/wasm/tests/worker.mjs "$scratch/pkg" examples "$scratch/native" "$sim" "$scratch/work"
