#!/usr/bin/env bash
# Builds the wasm package with scripts/wasm.sh and runs crates/wasm/tests/worker.mjs, which drives
# it in node worker threads and checks its folders and serial rows against native `sim run` on
# every example, byte for byte. It needs node 22.7 or later and rustup, and fails without them.
#
# Usage: scripts/wasm-check.sh [SCRATCH_DIR]
#
# SCRATCH_DIR defaults to .scratch/wasm/check beside the repo. The package goes in SCRATCH_DIR/pkg,
# the native folders in SCRATCH_DIR/native, and the input cases' files in SCRATCH_DIR/work.
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
# Before 22.7 node reads the package's sim_wasm.js as CommonJS, since no package.json says otherwise.
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
# The .d.ts types each argument as docs/wasm.md does, and its Code union is the docs' codes.
codes=$(sed -n '/^| `code` |/,/^$/ s/^| `\([a-z_]*\)` | .*/"\1"/p' docs/wasm.md | sed 1d |
  paste -sd '|' | sed 's/|/ | /g')
for line in \
  "export type Code = $codes;" \
  'static open(netlist: Uint8Array): Session;' \
  'static record(netlist: Uint8Array): Session;' \
  'advanceTo(ps: bigint): Status;' \
  'serialSince(cursor: bigint): Serial;' \
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
