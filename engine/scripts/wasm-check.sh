#!/usr/bin/env bash
# Builds the wasm package with scripts/wasm.sh and runs crates/wasm/tests/worker.mjs, which drives
# it in node worker threads and checks its folders and serial rows against native `sim run` on
# every example, byte for byte. It needs node 22.7 or later and rustup, and fails without them.
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

"$repo/scripts/wasm.sh" "$scratch/pkg"
cd "$repo"
# shellcheck source=scripts/wasm.sh
source "$repo/scripts/wasm.sh"
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
